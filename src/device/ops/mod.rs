//! 设备操作（实现函数）：跨平台枚举、识别、detect/select 命令、端口解析。
//!
//! 数据类型在姊妹模块 `device::data`。相比 C# 版按平台分别走 WMI/ioreg//sys，
//! Rust 的 `serialport` crate 已统一提供 USB VID/PID，无需平台分支。

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use serialport::{SerialPortType, UsbPortInfo};

use super::data::{PortInfo, Voltage};
use crate::cartridge_link::{CartridgeLink, BAUD, USB_PID, USB_VID};
use crate::event::{emit, Event};
use crate::rom::common::CartridgeKind;
use crate::{config, i18n};

// ---------------- 电压：识别 + 控制 ----------------
// 底层 0xa0 发包在 cartridge_link；这里负责“识别用哪种电压”及语义化控制。

/// 识别卡带所需电压。
/// - GBA / 未知：恒 3.3V（不读存档——GBA 在 5V 下会损坏，电压偏好对它无意义）。
/// - GB/GBC(MBC)：优先用 `voltage` 命令记住的偏好（存档），否则默认 5V。
pub fn voltage_for(kind: CartridgeKind) -> Voltage {
    match kind {
        CartridgeKind::GbMbc => config::load_voltage()
            .and_then(|s| Voltage::from_user(&s))
            .unwrap_or(Voltage::V5),
        _ => Voltage::V3_3,
    }
}

/// 给卡带上指定电压。
pub fn power(link: &mut CartridgeLink, v: Voltage) {
    link.power(v.code());
}

/// 断电。
pub fn power_off(link: &mut CartridgeLink) {
    link.power(Voltage::Off.code());
}

/// `cfb disconnect` —— 断开连接：忘记记住的端口 + 尽力给卡带断电（幂等，无设备也成功）。
pub fn cmd_disconnect(json: bool, explicit: Option<String>) -> ExitCode {
    config::clear_selected(); // 忘记 select 记住的端口
    let target = explicit.or_else(first_burner);
    if let Some(p) = target {
        let mut link = CartridgeLink::new(&p);
        if link.open().is_ok() {
            power_off(&mut link);
        }
    }
    if json {
        emit(&Event::Result {
            command: "disconnect".to_string(),
            ok: true,
            bytes: 0,
            mismatch_bytes: 0,
            seconds: 0.0,
        });
    } else {
        println!("{}", i18n::t("disconnect.ok"));
    }
    ExitCode::SUCCESS
}

/// `cfb voltage [3v3|5v|off|auto] [--clear]` —— 记住/查看供电电压偏好。
pub fn cmd_voltage(json: bool, arg: Option<String>, clear: bool) -> ExitCode {
    // 清除 / auto：回到按卡型自动决定。
    if clear || arg.as_deref() == Some("auto") {
        config::clear_voltage();
        if json {
            emit(&Event::Voltage { voltage: "auto".to_string() });
        } else {
            println!("{}", i18n::t("voltage.auto"));
        }
        return ExitCode::SUCCESS;
    }

    match arg {
        // 设置。
        Some(s) => match Voltage::from_user(&s) {
            Some(v) => {
                config::save_voltage(v.label());
                if json {
                    emit(&Event::Voltage { voltage: v.label().to_string() });
                } else {
                    println!("{}", i18n::tf("voltage.saved", &[("v", v.label())]));
                }
                ExitCode::SUCCESS
            }
            None => {
                let msg = i18n::tf("voltage.invalid", &[("v", &s)]);
                if json {
                    emit(&Event::Error { command: "voltage".to_string(), message: msg });
                } else {
                    eprintln!("{msg}");
                }
                ExitCode::from(2)
            }
        },
        // 查看当前。
        None => {
            let cur = config::load_voltage().unwrap_or_else(|| "auto".to_string());
            if json {
                emit(&Event::Voltage { voltage: cur });
            } else {
                println!("{}", i18n::tf("voltage.current", &[("v", &cur)]));
            }
            ExitCode::SUCCESS
        }
    }
}

/// 列出系统所有串口（含 VID/PID），按端口名排序。
pub fn enumerate() -> Vec<PortInfo> {
    let mut result = Vec::new();
    match serialport::available_ports() {
        Ok(ports) => {
            for p in ports {
                let (vid, pid, name) = match &p.port_type {
                    SerialPortType::UsbPort(UsbPortInfo {
                        vid,
                        pid,
                        product,
                        manufacturer,
                        ..
                    }) => {
                        let nm = product
                            .clone()
                            .or_else(|| manufacturer.clone())
                            .unwrap_or_else(|| i18n::t("dev.usb"));
                        (Some(*vid), Some(*pid), nm)
                    }
                    SerialPortType::PciPort => (None, None, i18n::t("dev.pci")),
                    SerialPortType::BluetoothPort => (None, None, i18n::t("dev.bt")),
                    SerialPortType::Unknown => (None, None, i18n::t("dev.unknown")),
                };
                result.push(PortInfo {
                    port: p.port_name,
                    name,
                    vid,
                    pid,
                });
            }
        }
        Err(e) => eprintln!("scan: {e}"),
    }
    result.sort_by(|a, b| a.port.cmp(&b.port));
    result
}

/// 仅烧录器，按端口名排序。
pub fn list_burners() -> Vec<PortInfo> {
    enumerate().into_iter().filter(PortInfo::is_burner).collect()
}

/// 第一个识别到的烧录器端口；没有则返回 None。
pub fn first_burner() -> Option<String> {
    list_burners().into_iter().next().map(|p| p.port)
}

/// 测试端口当前是否可打开（用于检测占用）。
fn can_open(port: &str) -> bool {
    serialport::new(port, BAUD)
        .timeout(Duration::from_millis(50))
        .open()
        .is_ok()
}

/// 端口在线（当前枚举得到）。
fn port_present(port: &str) -> bool {
    enumerate().iter().any(|p| p.port == port)
}

fn hexvid() -> String {
    format!("{USB_VID:04X}")
}
fn hexpid() -> String {
    format!("{USB_PID:04X}")
}

/// 解析要用的端口：显式 --port > `select` 记住的(仍在线) > 自动第一个烧录器。
/// 诊断信息一律走 stderr，避免污染 --json 的 stdout 事件流。返回 None 表示找不到。
pub fn resolve_port(explicit: Option<String>) -> Option<String> {
    if let Some(p) = explicit {
        return Some(p);
    }
    if let Some(saved) = config::load_selected() {
        if port_present(&saved) {
            eprintln!("{}", i18n::tf("resolve.using_saved", &[("port", &saved)]));
            return Some(saved);
        }
        eprintln!("{}", i18n::tf("resolve.saved_absent", &[("port", &saved)]));
    }
    if let Some(p) = first_burner() {
        eprintln!("{}", i18n::tf("resolve.auto", &[("port", &p)]));
        return Some(p);
    }
    eprintln!(
        "{}",
        i18n::tf("err.no_burner", &[("vid", &hexvid()), ("pid", &hexpid())])
    );
    None
}

/// `cfb detect` —— 只列出已连接的烧录器（非烧录器串口不显示）。
///
/// `json=true` 输出 NDJSON（每个烧录器一条 `port`，末尾一条 `summary`）；否则人类可读。
/// detect 总是成功退出（0）——数量由调用方读 summary/输出判断，而非退出码。
pub fn cmd_detect(json: bool) -> ExitCode {
    let burners = list_burners();

    if json {
        for p in &burners {
            emit(&Event::Port {
                port: p.port.clone(),
                vid: p.vid.map(|v| format!("{v:04X}")),
                pid: p.pid.map(|v| format!("{v:04X}")),
                burner: true,
                open: can_open(&p.port),
                name: p.name.clone(),
            });
        }
        emit(&Event::Summary {
            command: "detect".to_string(),
            burners: burners.len(),
        });
    } else if burners.is_empty() {
        println!(
            "{}",
            i18n::tf("detect.none", &[("vid", &hexvid()), ("pid", &hexpid())])
        );
    } else {
        for (i, p) in burners.iter().enumerate() {
            let busy = if can_open(&p.port) {
                String::new()
            } else {
                i18n::t("detect.busy")
            };
            println!("  [{}] {:<8} {} {}{}", i, p.port, p.vidpid(), p.name, busy);
        }
        println!(
            "{}",
            i18n::tf("detect.summary", &[("count", &burners.len().to_string())])
        );
    }

    ExitCode::SUCCESS
}

/// `cfb select` —— 选择并记住一个烧录器（持久化到 ~/.cfb.json）。
///
/// - `--clear`：清除记住的选择。
/// - `--port P`：直接记住 P（非交互；--json 模式必须走这条）。
/// - 否则：人类模式下列出烧录器并提示输入编号；只有一个时自动记住。
pub fn cmd_select(json: bool, explicit: Option<String>, clear: bool) -> ExitCode {
    if clear {
        let _ = config::clear_selected();
        if json {
            emit(&Event::Selected { port: None });
        } else {
            println!("{}", i18n::t("select.cleared"));
        }
        return ExitCode::SUCCESS;
    }

    if let Some(p) = explicit {
        let _ = config::save_selected(&p);
        if json {
            emit(&Event::Selected { port: Some(p) });
        } else {
            println!("{}", i18n::tf("select.saved", &[("port", &p)]));
        }
        return ExitCode::SUCCESS;
    }

    let burners = list_burners();
    if burners.is_empty() {
        if json {
            emit(&Event::Error {
                command: "select".to_string(),
                message: i18n::t("select.none"),
            });
        } else {
            eprintln!("{}", i18n::t("select.none"));
        }
        return ExitCode::from(1);
    }

    // --json 不能交互：要求显式 --port。
    if json {
        emit(&Event::Error {
            command: "select".to_string(),
            message: i18n::t("select.need_port_json"),
        });
        return ExitCode::from(2);
    }

    // 只有一个：自动记住。
    if burners.len() == 1 {
        let p = burners[0].port.clone();
        let _ = config::save_selected(&p);
        println!("{}", i18n::tf("select.single_auto", &[("port", &p)]));
        return ExitCode::SUCCESS;
    }

    // 交互选择。
    println!(
        "{}",
        i18n::tf("select.list_header", &[("count", &burners.len().to_string())])
    );
    for (i, p) in burners.iter().enumerate() {
        println!("  [{}] {:<8} {}", i + 1, p.port, p.vidpid());
    }
    print!(
        "{}",
        i18n::tf("select.prompt", &[("range", &format!("1-{}", burners.len()))])
    );
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    let choice: usize = line.trim().parse().unwrap_or(0);
    if choice < 1 || choice > burners.len() {
        eprintln!("{}", i18n::t("select.invalid"));
        return ExitCode::from(1);
    }

    let p = burners[choice - 1].port.clone();
    let _ = config::save_selected(&p);
    println!("{}", i18n::tf("select.saved", &[("port", &p)]));
    ExitCode::SUCCESS
}
