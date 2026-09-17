//! 端口租约：烧录器串口的跨进程自动让渡（cfb ↔ SkyEmu DirectPlay）。
//!
//! 无常驻服务器，纯 `~/.cfb/` 下的两个文件协调：
//! - `lease-<PORT>.json` —— 持有者信息 `{v,pid,name,ts}`；持有者存活期间定期重写（心跳，ts=秒级时间戳）。
//! - `lease-<PORT>.yield` —— 让渡请求标记 `{pid,ts}`；存在即「请释放端口」（请求者建、请求者删）。
//!
//! 握手（支持让渡的持有者，如 SkyEmu 直读会话）：
//!   持有者：  开口成功 → 写租约 → 会话中心跳 + 轮询；见 `.yield` → 关口 → 删租约 →
//!            等 `.yield` 消失 → 重开口恢复。
//!   请求者：  写 `.yield` → 轮询等租约消失（超时报错）→ 删 `.yield` → 写自己的租约 → 执行 → 删租约。
//! 不让渡的持有者（任意占口进程）：租约一直不消失 → 请求者超时，报持有者信息后放弃
//!（未写租约的进程则由 OS 独占打开兜底，报「打开端口失败」）。
//! 崩溃自愈：持有者心跳断流超过 [`STALE_SECS`] 且非本进程 → 视为已死，清残留租约直接接管。
//! cfb 互斥：租约文件 `create_new` 原子独占创建；到手后心跳线程持续刷新 ts，
//! 防止长操作（burn 数分钟）被另一个 cfb 误判 stale 抢走。
//!
//! 端口名归一化（[`port_key`]）：剥 `\\.\` 前缀后整体小写 —— SkyEmu 侧实现须保持一致。

use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 持有者心跳断流超过该秒数视为已死（可抢占）。
const STALE_SECS: u64 = 20;
/// 等待持有者让渡的轮询间隔。
const POLL_MS: u64 = 250;
/// 非本人所写、且超过该秒数无人认领的 `.yield`（请求者已死）→ 接管成功后顺手清理。
const YIELD_GHOST_SECS: u64 = 30;
/// 心跳刷新间隔（须远小于 [`STALE_SECS`]）。
const HEARTBEAT_SECS: u64 = 5;
/// 默认等待让渡的超时秒数；环境变量 `CFB_LEASE_WAIT` 可覆盖。
pub const DEFAULT_WAIT_SECS: u64 = 10;

/// 我们在租约/让渡文件里登记的进程名。
const OUR_NAME: &str = "cfb";

// ==================== 状态 ====================

struct Held {
    dir: PathBuf,
    port_key: String,
    /// 本进程是否（最终）负责删除让渡标记。
    yield_owner: bool,
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);
static HEARTBEAT_ON: AtomicBool = AtomicBool::new(false);

/// 租约文件里读出的持有者信息。
#[derive(Debug, Clone)]
pub struct HolderInfo {
    pub pid: u32,
    pub name: String,
}

/// 获取失败原因。
#[derive(Debug)]
pub enum AcquireError {
    /// 超时未让渡（携带最后读到的持有者）。
    Timeout { holder: Option<HolderInfo>, waited_secs: u64 },
}

// ==================== 路径与文件 ====================

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 租约目录 `~/.cfb`（Windows `%USERPROFILE%\.cfb`）。None = 家目录不可解析
/// （此时禁用租约协议，仅靠 OS 独占打开兜底，行为回到本特性之前）。
fn lease_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = PathBuf::from(home).join(".cfb");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 端口名 → 稳定文件名键：剥 `\\.\` / `\\?\` 前缀后整体小写。
fn port_key(port: &str) -> String {
    let p = port.trim().trim_start_matches(r"\\.\").trim_start_matches(r"\\?\");
    let mut s: String = p.chars().map(|c| c.to_ascii_lowercase()).collect();
    if s.is_empty() {
        s = "unknown".into();
    }
    s
}

fn lease_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("lease-{key}.json"))
}
fn yield_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("lease-{key}.yield"))
}

fn write_lease_file(path: &Path, pid: u32, name: &str) -> std::io::Result<()> {
    fs::write(path, format!("{{\"v\":1,\"pid\":{pid},\"name\":\"{name}\",\"ts\":{}}}", now_secs()))
}

/// 读租约：`Ok(None)` = 文件不存在；`Err(())` = 存在但不可解析（半写/损坏）。
fn read_holder(dir: &Path, key: &str) -> Result<Option<(HolderInfo, u64)>, ()> {
    let text = match fs::read_to_string(lease_path(dir, key)) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| ())?;
    let pid = v.get("pid").and_then(|p| p.as_u64()).ok_or(())? as u32;
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("?").to_string();
    let ts = v.get("ts").and_then(|t| t.as_u64()).unwrap_or(0);
    Ok(Some((HolderInfo { pid, name }, ts)))
}

fn write_yield_marker(dir: &Path, key: &str, pid: u32) {
    // 覆盖写：多个请求者并存时后写者接管标记（去删责任随之转移，见模块文档）
    let _ = fs::write(yield_path(dir, key), format!("{{\"pid\":{pid},\"ts\":{}}}", now_secs()));
}

fn read_yield_ts(dir: &Path, key: &str) -> Option<u64> {
    let text = fs::read_to_string(yield_path(dir, key)).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?.get("ts")?.as_u64()
}

// ==================== 核心获取（可测试） ====================

/// 尝试获取端口租约。成功返回 `yield_owner`（本进程是否接管了让渡标记的删除责任）。
fn acquire_in(
    dir: &Path,
    key: &str,
    our_pid: u32,
    wait_secs: u64,
) -> Result<bool, AcquireError> {
    let deadline = Instant::now() + Duration::from_secs(wait_secs.max(1));
    let lpath = lease_path(dir, key);
    let mut holder_seen: Option<HolderInfo> = None;
    let mut yield_owner = false;
    let mut unparsable_streak = 0u32;

    loop {
        // 1) 原子独占创建租约
        match OpenOptions::new().write(true).create_new(true).open(&lpath) {
            Ok(f) => {
                drop(f);
                let _ = write_lease_file(&lpath, our_pid, OUR_NAME);
                // 接管成功：自己写的标记自己删；他人的陈旧标记顺手清（请求者已死）
                if yield_owner {
                    let _ = fs::remove_file(yield_path(dir, key));
                } else if let Some(ts) = read_yield_ts(dir, key) {
                    if now_secs().saturating_sub(ts) > YIELD_GHOST_SECS {
                        let _ = fs::remove_file(yield_path(dir, key));
                    }
                }
                return Ok(yield_owner);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Ok(false), // 目录不可写等：放弃协议，OS 兜底
        }

        // 2) 读当前持有者
        match read_holder(dir, key) {
            Ok(None) => continue, // 竞态窗口：租约刚被删且未轮到我们创建 → 回去重试 create_new
            Err(()) => {
                // 半写窗口：持有者正在重写心跳。连续两轮不可解析才按残留清除。
                unparsable_streak += 1;
                if unparsable_streak >= 2 {
                    let _ = fs::remove_file(&lpath);
                    unparsable_streak = 0;
                } else {
                    std::thread::sleep(Duration::from_millis(POLL_MS));
                }
                continue;
            }
            Ok(Some((h, ts))) => {
                unparsable_streak = 0;
                if h.pid == our_pid {
                    // 已是本进程持有（重连/重开场景）：刷新心跳即成功
                    let _ = write_lease_file(&lpath, our_pid, OUR_NAME);
                    return Ok(false);
                }
                if now_secs().saturating_sub(ts) > STALE_SECS {
                    // 持有者心跳断流：视为已死，清残留后重试创建
                    let _ = fs::remove_file(&lpath);
                    continue;
                }
                // 3) 活着的持有者：首次发让渡请求
                holder_seen = Some(h);
                if !yield_owner {
                    write_yield_marker(dir, key, our_pid);
                    yield_owner = true;
                    eprintln!(
                        "{}",
                        crate::i18n::tf(
                            "lease.waiting",
                            &[
                                ("port", key),
                                ("name", &holder_seen.as_ref().map(|h| h.name.clone()).unwrap_or_default()),
                                ("pid", &holder_seen.as_ref().map(|h| h.pid.to_string()).unwrap_or_default()),
                                ("s", &wait_secs.to_string()),
                            ],
                        )
                    );
                }
                if Instant::now() >= deadline {
                    if yield_owner {
                        let _ = fs::remove_file(yield_path(dir, key));
                    }
                    return Err(AcquireError::Timeout { holder: holder_seen, waited_secs: wait_secs });
                }
                std::thread::sleep(Duration::from_millis(POLL_MS));
            }
        }
    }
}

// ==================== 进程级封装 ====================

/// 获取端口租约（幂等：同口重入直接续期）。家目录不可用时静默放行。
pub fn acquire(port: &str) -> Result<(), AcquireError> {
    let wait_secs = std::env::var("CFB_LEASE_WAIT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_WAIT_SECS);
    let Some(dir) = lease_dir() else { return Ok(()) };
    let key = port_key(port);
    let pid = std::process::id();

    {
        let mut g = HELD.lock().unwrap();
        if let Some(h) = &*g {
            if h.port_key == key {
                let _ = write_lease_file(&lease_path(&h.dir, &h.port_key), pid, OUR_NAME);
                return Ok(());
            }
            release_held(&mut g); // 极罕见：单次运行换端口
        }
    }

    match acquire_in(&dir, &key, pid, wait_secs) {
        Ok(yield_owner) => {
            if yield_owner {
                eprintln!("{}", crate::i18n::t("lease.resumed"));
            }
            *HELD.lock().unwrap() = Some(Held { dir, port_key: key, yield_owner });
            start_heartbeat();
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// 进程退出前释放（main 收尾调用；漏调也会因心跳断流自愈）。
pub fn release_all() {
    HEARTBEAT_ON.store(false, Ordering::SeqCst);
    let mut g = HELD.lock().unwrap();
    release_held(&mut g);
}

fn release_held(g: &mut Option<Held>) {
    if let Some(h) = g.take() {
        let _ = fs::remove_file(lease_path(&h.dir, &h.port_key));
        if h.yield_owner {
            let _ = fs::remove_file(yield_path(&h.dir, &h.port_key));
        }
    }
}

fn start_heartbeat() {
    if HEARTBEAT_ON.swap(true, Ordering::SeqCst) {
        return; // 已在跑
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
            if !HEARTBEAT_ON.load(Ordering::SeqCst) {
                break;
            }
            let g = HELD.lock().unwrap();
            if let Some(h) = &*g {
                let lp = lease_path(&h.dir, &h.port_key);
                // 只刷新仍存在的租约（避免在 release 后复活文件）
                if lp.exists() {
                    let _ = write_lease_file(&lp, std::process::id(), OUR_NAME);
                }
            }
        }
    });
}

// ==================== 单元测试（临时目录，无硬件） ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cfb-lease-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn foreign_lease(dir: &Path, key: &str, pid: u32, ts: u64) {
        fs::write(lease_path(dir, key), format!("{{\"v\":1,\"pid\":{pid},\"name\":\"SkyEmu\",\"ts\":{ts}}}")).unwrap();
    }

    #[test]
    fn free_port_acquires() {
        let dir = tmp_dir("free");
        assert!(acquire_in(&dir, "com7", 111, 2).is_ok());
        let (h, ts) = read_holder(&dir, "com7").unwrap().unwrap();
        assert_eq!(h.pid, 111);
        assert!(now_secs() - ts <= 2, "新租约 ts 应为当下");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_holder_is_stolen() {
        let dir = tmp_dir("stale");
        foreign_lease(&dir, "com7", 999, now_secs() - STALE_SECS - 5);
        assert!(acquire_in(&dir, "com7", 111, 2).is_ok());
        let (h, _) = read_holder(&dir, "com7").unwrap().unwrap();
        assert_eq!(h.pid, 111, "应清残留租约并接管");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_holder_times_out_and_cleans_yield() {
        let dir = tmp_dir("timeout");
        foreign_lease(&dir, "com7", 999999, now_secs());
        let r = acquire_in(&dir, "com7", 111, 1);
        match r {
            Err(AcquireError::Timeout { holder, .. }) => {
                let h = holder.expect("应携带持有者信息");
                assert_eq!(h.name, "SkyEmu");
            }
            other => panic!("期望 Timeout，得到 {other:?}"),
        }
        assert!(!yield_path(&dir, "com7").exists(), "超时应清理让渡标记");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn yield_handoff_completes() {
        let dir = tmp_dir("handoff");
        foreign_lease(&dir, "com7", 999999, now_secs());
        // 模拟持有者：等 .yield 出现 → 删租约（= 已释放）
        let holder_dir = dir.clone();
        std::thread::spawn(move || {
            for _ in 0..40 {
                if yield_path(&holder_dir, "com7").exists() {
                    std::thread::sleep(Duration::from_millis(100)); // 模拟关串口的耗时
                    let _ = fs::remove_file(lease_path(&holder_dir, "com7"));
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let r = acquire_in(&dir, "com7", 111, 5);
        assert!(r.is_ok(), "让渡握手应在超时前完成");
        let (h, _) = read_holder(&dir, "com7").unwrap().unwrap();
        assert_eq!(h.pid, 111, "接管后租约应是我们的");
        assert!(!yield_path(&dir, "com7").exists(), "接管后应删除让渡标记");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn own_pid_reacquires() {
        let dir = tmp_dir("own");
        foreign_lease(&dir, "com7", 111, now_secs() - 60); // 即使“过期”，也是自己 → 直接续期
        assert!(acquire_in(&dir, "com7", 111, 1).is_ok());
        let (h, ts) = read_holder(&dir, "com7").unwrap().unwrap();
        assert_eq!(h.pid, 111);
        assert!(now_secs() - ts <= 2, "应刷新心跳 ts");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ghost_yield_is_cleaned_on_acquire() {
        let dir = tmp_dir("ghost");
        fs::write(yield_path(&dir, "com7"), format!("{{\"pid\":1,\"ts\":{}}}", now_secs() - YIELD_GHOST_SECS - 5)).unwrap();
        assert!(acquire_in(&dir, "com7", 111, 2).is_ok());
        assert!(!yield_path(&dir, "com7").exists(), "陈旧让渡标记应被清理");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn port_key_normalizes() {
        assert_eq!(port_key(r"\\.\COM7"), "com7");
        assert_eq!(port_key("COM7"), "com7");
        assert_eq!(port_key("/dev/ttyUSB0"), "/dev/ttyusb0");
    }
}
