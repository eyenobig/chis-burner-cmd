//! `cfb udev` —— 一键安装烧录器串口的 udev 权限规则（仅 Linux）。
//!
//! 新用户在 Ubuntu/Debian 等发行版上默认打不开 /dev/ttyACM*（root:dialout 0660），
//! 而 `usermod -aG dialout` 要**重新登录**才生效——这是上手第一坑。本命令把
//! ChisFlash 烧录器（0483:0721）专属规则写入 /etc/udev/rules.d/：
//!
//! - `TAG+="uaccess"`：桌面登录用户经 systemd-logind 的 ACL **立即**获得访问权，
//!   免重登、免拔插（安装后 `udevadm trigger` 对已插设备生效），且只授权当前会话用户；
//! - `GROUP="dialout"`：headless / SSH 等无 seat 场景仍可走传统的 dialout 组；
//! - 只匹配烧录器 VID/PID，不影响其他串口设备。
//!
//! 授权经由 pkexec（GNOME/KDE 弹系统密码框），用户无需手敲 sudo；
//! 无 pkexec 的环境打印等价的手动安装命令。幂等：规则已存在则直接返回。

use crate::event::{emit, Event};
use crate::i18n;
use std::process::ExitCode;

/// 规则安装路径。
const RULE_PATH: &str = "/etc/udev/rules.d/99-chisflash.rules";
/// 规则内容。首行的 chisflash-burner 标记用于幂等判断（识别"本工具装的规则"）。
const RULE: &str = concat!(
    "# chisflash-burner: serial permission for ChisFlash burner (0483:0721), installed by `cfb udev`\n",
    "ACTION==\"add\", SUBSYSTEM==\"tty\", ATTRS{idVendor}==\"0483\", ATTRS{idProduct}==\"0721\", ",
    "MODE=\"0660\", GROUP=\"dialout\", TAG+=\"uaccess\"\n"
);

/// 等价的手动安装命令（无 pkexec 时展示；--print 也用它）。
fn manual_cmd() -> String {
    format!(
        "sudo tee {RULE_PATH} >/dev/null <<'EOF'\n{RULE}EOF\nsudo udevadm control --reload-rules && sudo udevadm trigger"
    )
}

fn say(json: bool, phase: &str, msg: String) {
    if json {
        emit(&Event::Udev {
            phase: phase.into(),
            message: msg.clone(),
        });
        // 诊断文本走 stderr，保持 stdout 为纯 NDJSON（仓库约定，见 device/ops/mod.rs）。
        eprintln!("{msg}");
    } else if phase == "failed" || phase == "canceled" {
        eprintln!("{msg}");
    } else {
        println!("{msg}");
    }
}

fn rule_installed() -> bool {
    std::fs::read_to_string(RULE_PATH)
        .map(|s| s.contains("chisflash-burner") && s.contains("0483") && s.contains("0721"))
        .unwrap_or(false)
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join(name))
        .find(|p| p.is_file())
}

/// `cfb udev [--print]`：--print 只打印规则与手动安装命令，不做任何改动。
/// Linux：装 udev 规则；macOS：释放被占用的串口（见 cmd_udev_macos）；Windows：无需，跳过。
pub fn cmd_udev(json: bool, print_only: bool) -> ExitCode {
    if cfg!(target_os = "macos") {
        return cmd_udev_macos(json, print_only);
    }
    if cfg!(target_os = "windows") {
        return cmd_udev_windows(json);
    }
    if !cfg!(target_os = "linux") {
        // 其余 unix 无此机制；安静跳过，客户端按 phase=skipped 处理。
        say(json, "skipped", i18n::t("udev.linux_only").to_string());
        return ExitCode::SUCCESS;
    }

    if print_only {
        let msg = i18n::tf("udev.print_out", &[("cmd", &manual_cmd())]);
        say(json, "manual", msg);
        return ExitCode::SUCCESS;
    }

    if rule_installed() {
        say(
            json,
            "already",
            i18n::tf("udev.already", &[("path", RULE_PATH)]),
        );
        return ExitCode::SUCCESS;
    }

    let Some(pkexec) = which("pkexec") else {
        // 常见于极简 WM / 无 polkit 的环境：给出等价手动命令。
        say(json, "manual", i18n::tf("udev.manual", &[("cmd", &manual_cmd())]));
        return ExitCode::from(2);
    };

    // 规则写到临时文件，由 pkexec 以 root 权限 install 到位并立即生效（reload + trigger）。
    let tmp = std::env::temp_dir().join("chisflash-99-chisflash.rules");
    if let Err(e) = write_rule_tmp(&tmp) {
        say(
            json,
            "failed",
            i18n::tf("udev.write_fail", &[("err", &e.to_string())]),
        );
        return ExitCode::from(1);
    }

    say(json, "prompt", i18n::t("udev.prompt").to_string());
    let script = format!(
        "install -m 0644 '{}' {} && udevadm control --reload-rules && udevadm trigger",
        tmp.display(),
        RULE_PATH
    );
    let status = std::process::Command::new(&pkexec)
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .status();
    let _ = std::fs::remove_file(&tmp); // 临时文件清理（root 侧是 copy，删本侧即可）

    match status {
        Ok(s) if s.success() && rule_installed() => {
            say(json, "installed", i18n::t("udev.installed").to_string());
            ExitCode::SUCCESS
        }
        Ok(s) if s.code() == Some(126) => {
            // pkexec：用户在密码框点了取消 / 拒绝授权。
            say(json, "canceled", i18n::t("udev.canceled").to_string());
            ExitCode::from(1)
        }
        Ok(s) => {
            say(
                json,
                "failed",
                i18n::tf("udev.failed", &[("code", &s.code().unwrap_or(-1).to_string())]),
            );
            ExitCode::from(1)
        }
        Err(e) => {
            say(
                json,
                "failed",
                i18n::tf("udev.failed", &[("code", &e.to_string())]),
            );
            ExitCode::from(1)
        }
    }
}

/// 写规则临时文件并设 0644（root 侧 install -m 0644 也会带权限，这里保证内容可读）。
fn write_rule_tmp(tmp: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(tmp, RULE)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o644))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// macOS：串口被其他进程占用的一键释放
// ---------------------------------------------------------------------------

/// macOS 上 /dev/cu.usb* 默认全员可读写（无 udev 权限问题），打不开几乎都是
/// **被其他进程占用**（残留的 SkyEmu、串口监视器等）。这里用 lsof 找出持有者并
/// 结束同用户进程，等价于 Linux 侧的"一键修复"；不同用户的持有者无法杀，转手动指引。
#[cfg(target_os = "macos")]
fn cmd_udev_macos(json: bool, print_only: bool) -> ExitCode {
    if print_only {
        say(json, "manual", i18n::t("udev.mac_manual").to_string());
        return ExitCode::SUCCESS;
    }

    say(json, "checking", i18n::t("udev.mac_checking").to_string());
    // /bin/sh 负责展开 glob；不存在的匹配仅报 stderr，不影响其余设备
    let out = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("lsof -F p /dev/cu.usbmodem* /dev/cu.usbserial* 2>/dev/null")
        .output();
    let Ok(out) = out else {
        say(json, "failed", i18n::t("udev.mac_lsof_fail").to_string());
        return ExitCode::from(1);
    };

    // lsof -F p：每个持有进程一行 p<PID>；去重并排除自身
    let mut pids: Vec<u32> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with('p'))
        .filter_map(|l| l[1..].trim().parse::<u32>().ok())
        .filter(|p| *p != std::process::id())
        .collect();
    pids.sort_unstable();
    pids.dedup();

    if pids.is_empty() {
        say(json, "none", i18n::t("udev.mac_none").to_string());
        return ExitCode::SUCCESS;
    }

    let mut killed = 0usize;
    for pid in &pids {
        let ok = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            killed += 1;
        }
    }

    if killed == pids.len() {
        say(
            json,
            "fixed",
            i18n::tf("udev.mac_fixed", &[("n", &killed.to_string())]),
        );
        ExitCode::SUCCESS
    } else {
        // 有持有者属于其他用户（kill 被拒）：给出手动指引
        say(
            json,
            "manual",
            i18n::tf(
                "udev.mac_manual_holders",
                &[("n", &(pids.len() - killed).to_string())],
            ),
        );
        ExitCode::from(1)
    }
}

/// 非 macOS 平台的空实现：使 cmd_udev 的调用处在所有平台均可编译。
#[cfg(not(target_os = "macos"))]
fn cmd_udev_macos(json: bool, print_only: bool) -> ExitCode {
    let _ = print_only;
    say(json, "skipped", i18n::t("udev.linux_only").to_string());
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Windows：结束占用串口的残留进程（SkyEmu / 卡死的 cfb 实例）
// ---------------------------------------------------------------------------

/// 列出指定映像名的全部 PID（tasklist CSV：第 2 列为 PID）。
#[cfg(target_os = "windows")]
fn pids_of_image(image: &str) -> Vec<u32> {
    let Ok(out) = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split(r#",""#).nth(1))
        .filter_map(|f| f.trim_matches('"').trim().parse::<u32>().ok())
        .collect()
}

/// Windows 上 COM 口没有用户态权限问题，打不开几乎都是**被其他进程占用**：
/// DirectPlay 残留的 SkyEmu、上次操作卡死未退出的 cfb 实例。结束它们（排除自身
/// 避免自杀），客户端随后重新 detect 即可接上。不弹任何系统对话框。
#[cfg(target_os = "windows")]
fn cmd_udev_windows(json: bool) -> ExitCode {
    say(json, "checking", i18n::t("udev.win_checking").to_string());
    let me = std::process::id();
    let mut targets: Vec<u32> = Vec::new();
    for image in ["SkyEmu.exe", "cfb.exe"] {
        for pid in pids_of_image(image) {
            if pid != me && !targets.contains(&pid) {
                targets.push(pid);
            }
        }
    }

    if targets.is_empty() {
        say(json, "none", i18n::t("udev.win_none").to_string());
        return ExitCode::SUCCESS;
    }

    let mut killed = 0usize;
    for pid in &targets {
        let ok = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            killed += 1;
        }
    }

    if killed == targets.len() {
        say(
            json,
            "fixed",
            i18n::tf("udev.win_fixed", &[("n", &killed.to_string())]),
        );
        ExitCode::SUCCESS
    } else {
        say(
            json,
            "manual",
            i18n::tf(
                "udev.win_manual_holders",
                &[("n", &(targets.len() - killed).to_string())],
            ),
        );
        ExitCode::from(1)
    }
}

/// 非 Windows 平台的空实现：使 cmd_udev 的调用处在所有平台均可编译。
#[cfg(not(target_os = "windows"))]
fn cmd_udev_windows(json: bool) -> ExitCode {
    let _ = json;
    say(json, "skipped", i18n::t("udev.linux_only").to_string());
    ExitCode::SUCCESS
}
