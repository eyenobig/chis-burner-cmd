//! 设备数据类型（数据集）。
//!
//! 只放结构体/类型与其固有方法；操作逻辑在姊妹模块 `device::ops`。

use crate::cartridge_link::{USB_PID, USB_VID};

/// 一个串口设备的信息。
pub struct PortInfo {
    /// 端口名：Windows 为 COM7；macOS 为 /dev/cu.usbmodemXXXX；Linux 为 /dev/ttyACM0。
    pub port: String,
    /// 友好名（USB product/manufacturer，取不到则为占位文案）。
    pub name: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
}

impl PortInfo {
    /// 是否本烧录器（VID/PID 匹配）。
    pub fn is_burner(&self) -> bool {
        self.vid == Some(USB_VID) && self.pid == Some(USB_PID)
    }

    /// "0483:0721" 形式；非 USB 为 "-"。
    pub fn vidpid(&self) -> String {
        match (self.vid, self.pid) {
            (Some(v), Some(d)) => format!("{v:04X}:{d:04X}"),
            _ => "-".to_string(),
        }
    }
}

/// 卡带供电电压（烧录器输出）。GBA=3.3V，GB/GBC(MBC)=5V。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Voltage {
    Off,
    V3_3,
    V5,
}

impl Voltage {
    /// 协议电压码：0=断电, 1=3.3V, 2=5V（对应固件 0xa0 命令第二字节）。
    pub fn code(&self) -> u8 {
        match self {
            Voltage::Off => 0,
            Voltage::V3_3 => 1,
            Voltage::V5 => 2,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Voltage::Off => "off",
            Voltage::V3_3 => "3.3V",
            Voltage::V5 => "5V",
        }
    }

    /// 宽松解析用户输入：3v3/3.3/3.3v → 3.3V；5/5v → 5V；off → Off。
    pub fn from_user(s: &str) -> Option<Voltage> {
        match s.trim().to_lowercase().as_str() {
            "3" | "3v3" | "3.3" | "3.3v" => Some(Voltage::V3_3),
            "5" | "5v" | "5.0" | "5.0v" => Some(Voltage::V5),
            "off" | "0" => Some(Voltage::Off),
            _ => None,
        }
    }
}
