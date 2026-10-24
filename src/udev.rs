//! `cfb udev` —— 烧录器串口一键修复（三平台）。
//!
//! 名字沿用 Linux 的 udev 规则方案；三个平台各自复刻同一套授权流程：
//!
//! **Linux**：新用户默认打不开 /dev/ttyACM*（root:dialout 0660），`usermod -aG dialout`
//! 要重新登录——上手第一坑。本命令把烧录器（0483:0721）专属规则写入 /etc/udev/rules.d/：
//! `TAG+="uaccess"` 经 systemd-logind 的 ACL **立即**授权桌面用户（免重登免拔插）；
//! `GROUP="dialout"` 兼容 headless 场景。授权经由 pkexec 弹系统密码框；幂等：已装直接返回。
//!
//! **macOS**：/dev/cu.usb* 默认全员可读写，打不开多因被占用或设备节点权限被改。
//! 流程复刻：先结束占用进程（lsof，无需提权）→ 仍打不开则经 osascript 的管理员
//! 权限（原生密码框，pkexec 等价）chmod 修复设备节点，立即生效。
//!
//! **Windows**：COM 口无用户态权限问题，打不开多因被占用或设备被禁用/状态异常。
//! 流程复刻：先结束占用进程（taskkill，无需提权）→ 仍打不开则经 UAC 提权
//! （PowerShell -Verb RunAs，原生确认框）用 PnpDevice 重新启用异常的烧录器设备实例。
//!
//! 事件 phase 约定（三平台一致，前端按此驱动 UI）：
//! checking → prompt → installed / fixed / canceled / manual / failed / none / skipped

use crate::event::{emit, Event};
use crate::i18n;
use std::process::ExitCode;

/// 规则安装路径（仅 Linux）。
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

/// `cfb udev [--print]`：三平台一键修复烧录器串口。
/// Linux：装 udev 规则；macOS：释放占用 + 修复设备节点权限；Windows：释放占用 + 修复设备状态。
pub fn cmd_udev(json: bool, print_only: bool) -> ExitCode {
    if cfg!(target_os = "macos") {
        return cmd_udev_macos(json, print_only);
    }
    if cfg!(target_os = "windows") {
        return cmd_udev_windows(json, print_only);
    }
    if !cfg!(target_os = "linux") {
        // 其余平台无此机制；安静跳过，客户端按 phase=skipped 处理。
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
// 共享（macOS / Windows）：烧录器串口枚举与可打开探测
// ---------------------------------------------------------------------------

/// 烧录器（0483:0721）当前枚举到的串口名（/dev/cu.usb* 或 COMx）。
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn burner_port_names() -> Vec<String> {
    use serialport::SerialPortType;
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports
        .into_iter()
        .filter_map(|p| match p.port_type {
            SerialPortType::UsbPort(info) => {
                (info.vid == 0x0483 && info.pid == 0x0721).then_some(p.port_name)
            }
            _ => None,
        })
        .collect()
}

/// 探测烧录器串口当前是否可打开（短超时探测，同 device::ops::can_open）。
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn burner_openable() -> bool {
    use std::time::Duration;
    for name in burner_port_names() {
        if serialport::new(&name, 115_200)
            .timeout(Duration::from_millis(50))
            .open()
            .is_ok()
        {
            return true;
        }
    }
    // 设备未枚举（刚释放占用、驱动重载中）视为可通过——由随后的 detect 最终确认
    burner_port_names().is_empty()
}

// ---------------------------------------------------------------------------
// macOS：结束占用者 → 提权修复设备节点权限（osascript 管理员 = pkexec 等价）
// ---------------------------------------------------------------------------

/// 结束持有 /dev/cu.usb* 串口的同用户进程（lsof 定位）。返回结束的数量。
#[cfg(target_os = "macos")]
fn mac_kill_holders() -> usize {
    let Ok(out) = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("lsof -F p /dev/cu.usbmodem* /dev/cu.usbserial* 2>/dev/null")
        .output()
    else {
        return 0;
    };
    let mut pids: Vec<u32> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with('p'))
        .filter_map(|l| l[1..].trim().parse::<u32>().ok())
        .filter(|p| *p != std::process::id())
        .collect();
    pids.sort_unstable();
    pids.dedup();
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
    killed
}

#[cfg(target_os = "macos")]
fn cmd_udev_macos(json: bool, print_only: bool) -> ExitCode {
    if print_only {
        say(json, "manual", i18n::t("udev.mac_manual").to_string());
        return ExitCode::SUCCESS;
    }

    say(json, "checking", i18n::t("udev.mac_checking").to_string());

    // 第一阶段：结束占用进程（无需提权）
    let killed = mac_kill_holders();
    if killed > 0 {
        say(
            json,
            "fixed",
            i18n::tf("udev.mac_fixed", &[("n", &killed.to_string())]),
        );
        return ExitCode::SUCCESS;
    }

    // 第二阶段：无占用者仍打不开 → 设备节点权限被改（受管 Mac 等）。
    // 提权修复：osascript 管理员权限（原生密码框，pkexec 等价）chmod 设备节点。
    say(json, "prompt", i18n::t("udev.mac_stage2").to_string());
    let script = "do shell script \"chmod 666 /dev/cu.usbmodem* /dev/cu.usbserial* 2>/dev/null; true\" with administrator privileges";
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status();

    let canceled = matches!(status, Ok(s) if s.code() == Some(1)); // osascript 取消 = exit 1
    match status {
        Ok(s) if s.success() => {
            if burner_openable() {
                say(json, "installed", i18n::t("udev.mac_installed").to_string());
                return ExitCode::SUCCESS;
            }
            say(json, "failed", i18n::t("udev.mac_failed").to_string());
            ExitCode::from(1)
        }
        _ if canceled => {
            say(json, "canceled", i18n::t("udev.mac_canceled").to_string());
            ExitCode::from(1)
        }
        _ => {
            say(json, "failed", i18n::t("udev.mac_failed").to_string());
            ExitCode::from(1)
        }
    }
}

/// 非 macOS 平台的空实现：使 cmd_udev 的调用处在所有平台均可编译。
#[cfg(not(target_os = "macos"))]
fn cmd_udev_macos(json: bool, print_only: bool) -> ExitCode {
    let _ = (json, print_only);
    say(json, "skipped", i18n::t("udev.linux_only").to_string());
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Windows：结束占用者 → 提权修复设备状态（UAC + PnpDevice 重新启用）
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
        .filter_map(|line| line.split("\",\"").nth(1))
        .filter_map(|f| f.trim_matches('"').trim().parse::<u32>().ok())
        .collect()
}

/// 结束占用串口的残留进程：DirectPlay 残留的 SkyEmu、卡死未退出的 cfb 实例
/// （排除自身避免自杀）。返回结束的数量。
#[cfg(target_os = "windows")]
fn win_kill_holders() -> usize {
    let me = std::process::id();
    let mut targets: Vec<u32> = Vec::new();
    for image in ["SkyEmu.exe", "cfb.exe"] {
        for pid in pids_of_image(image) {
            if pid != me && !targets.contains(&pid) {
                targets.push(pid);
            }
        }
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
    killed
}

/// 提权修复脚本（由 UAC 提权的 PowerShell 执行）：重新启用状态异常/被禁用的
/// 烧录器设备实例（0483:0721），触发驱动重枚举，并把结果写入文件供回读。
#[cfg(target_os = "windows")]
fn win_repair_script(result_path: &std::path::Path) -> String {
    format!(
        concat!(
            "$ErrorActionPreference = 'SilentlyContinue'\n",
            "$devs = Get-PnpDevice -PresentOnly | Where-Object {{ $_.InstanceId -like 'USB\\VID_0483&PID_0721*' }}\n",
            "$fixed = 0\n",
            "foreach ($d in $devs) {{\n",
            "  if ($d.Status -ne 'OK') {{\n",
            "    Enable-PnpDevice -InstanceId $d.InstanceId -Confirm:$false\n",
            "    $fixed++\n",
            "  }}\n",
            "}}\n",
            "Set-Content -Path '{path}' -Value ('fixed=' + $fixed)\n"
        ),
        path = result_path.display().to_string().replace('\\', "\\\\")
    )
}

#[cfg(target_os = "windows")]
fn cmd_udev_windows(json: bool, print_only: bool) -> ExitCode {
    if print_only {
        say(json, "manual", i18n::t("udev.win_manual").to_string());
        return ExitCode::SUCCESS;
    }

    say(json, "checking", i18n::t("udev.win_checking").to_string());

    // 第一阶段：结束占用进程（无需提权）
    let killed = win_kill_holders();
    if killed > 0 {
        say(
            json,
            "fixed",
            i18n::tf("udev.win_fixed", &[("n", &killed.to_string())]),
        );
        return ExitCode::SUCCESS;
    }

    // 第二阶段：无占用进程仍打不开 → 设备被禁用/状态异常。
    // 提权修复：UAC（原生确认框）→ 管理员 PowerShell 用 PnpDevice 重新启用烧录器设备实例。
    say(json, "prompt", i18n::t("udev.win_stage2").to_string());
    let tmp = std::env::temp_dir();
    let repair = tmp.join("chisflash-perm-repair.ps1");
    let result = tmp.join("chisflash-perm-result.txt");
    let _ = std::fs::remove_file(&result);
    if let Err(e) = std::fs::write(&repair, win_repair_script(&result)) {
        say(
            json,
            "failed",
            i18n::tf("udev.write_fail", &[("err", &e.to_string())]),
        );
        return ExitCode::from(1);
    }

    // Start-Process -Verb RunAs 弹 UAC；-Wait 等提权子进程结束；取消 UAC 时外层报错。
    let outer = format!(
        "Start-Process powershell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{}' -Verb RunAs -Wait; $ok = $?",
        repair.display().to_string().replace('\'', "''")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &outer])
        .status();
    let _ = std::fs::remove_file(&repair);

    let result_text = std::fs::read_to_string(&result).unwrap_or_default();
    let _ = std::fs::remove_file(&result);

    let canceled = matches!(status, Ok(s) if !s.success());

    if result_text.contains("fixed=0") && !canceled {
        // 设备状态本来就正常：问题不在设备层（可能为驱动级），如实报告
        say(json, "failed", i18n::t("udev.win_failed").to_string());
        return ExitCode::from(1);
    }
    if burner_openable() {
        say(json, "installed", i18n::t("udev.win_installed").to_string());
        return ExitCode::SUCCESS;
    }
    if canceled {
        say(json, "canceled", i18n::t("udev.win_canceled").to_string());
    } else {
        say(json, "failed", i18n::t("udev.win_failed").to_string());
    }
    ExitCode::from(1)
}

/// 非 Windows 平台的空实现：使 cmd_udev 的调用处在所有平台均可编译。
#[cfg(not(target_os = "windows"))]
fn cmd_udev_windows(json: bool, print_only: bool) -> ExitCode {
    let _ = (json, print_only);
    say(json, "skipped", i18n::t("udev.linux_only").to_string());
    ExitCode::SUCCESS
}
