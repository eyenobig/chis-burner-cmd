//! GBA 数据类型（数据集）。

use crate::event::RomChecksum;

/// S29GL 系列统一扇区大小 128KB。
pub const SECTOR: u32 = 0x20000;

/// NOR flash 信息（CFI）。
pub struct FlashInfo {
    /// Autoselect ID（8 字节）；读不到为 None。
    pub id: Option<[u8; 8]>,
    pub device_size: u64,        // 字节
    pub buffer_write_bytes: u32, // 写缓冲大小(字节)，0=仅单字编程
    pub sector_size: u32,        // 统一扇区大小(字节)
    pub sector_count: u32,
}

impl FlashInfo {
    /// "01 02 ..." 形式的 ID；无则空串。
    pub fn id_hex(&self) -> String {
        match self.id {
            Some(id) => id
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
            None => String::new(),
        }
    }
}

/// GBA ROM 头解析结果。
pub struct GbaHeader {
    pub rom_title: String,
    pub game_code: String,
    pub revision: u8,
    pub checksum: RomChecksum,
    pub game_name: String,
    pub rtc: bool,
}

/// 烧录选项。
pub struct BurnOptions {
    /// true=整片擦除后连续写（--chip-erase）；默认 false=逐扇区只擦 ROM 范围（快路径）。
    pub chip_erase: bool,
    /// 开始前自动解锁 PPB。
    pub unlock_ppb: bool,
    /// 烧后校验 + 修复。
    pub verify: bool,
    /// 跳过擦除直接写入（`--no-erase`，仅用于测纯写入吞吐，要求 flash 已是擦除态）。
    pub no_erase: bool,
}

/// 烧录结果。
pub struct BurnResult {
    pub success: bool,
    pub bytes_written: u64,
    pub reconnects: u32,
    pub first_bad: Option<u64>,
    pub mismatch_bytes: u32,
    pub seconds: f64,
}

/// 存档类型（GBA：EEPROM/SRAM/FLASH/FRAM；MBC 仅 SRAM/FRAM）。
/// 与 C# `comboBox_ramType` / `comboBox_mbc5RamType` 对应。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveType {
    /// 4Kbit EEPROM（512 字节，6 位块地址）。
    Eeprom4k,
    /// 64Kbit EEPROM（8192 字节，14 位块地址）。
    Eeprom64k,
    /// SRAM（电池保持的静态 RAM，最常见）。
    Sram,
    /// FLASH 存档（写前需 JEDEC 整片擦除）。
    Flash,
    /// FRAM（铁电，需按 latency 时序访问）。
    Fram,
}

impl SaveType {
    /// 解析用户输入（eeprom4k/eeprom64k/sram/flash/fram）。无效返回 None。
    pub fn from_user(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "eeprom4k" | "eeprom512" | "eeprom512b" => Some(Self::Eeprom4k),
            "eeprom64k" | "eeprom8k" | "eeprom8192" => Some(Self::Eeprom64k),
            "sram" => Some(Self::Sram),
            "flash" => Some(Self::Flash),
            "fram" => Some(Self::Fram),
            _ => None,
        }
    }

    /// 文案标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::Eeprom4k => "4K EEPROM",
            Self::Eeprom64k => "64K EEPROM",
            Self::Sram => "SRAM",
            Self::Flash => "FLASH",
            Self::Fram => "FRAM",
        }
    }

    pub fn eeprom_size(self) -> Option<u64> {
        match self {
            Self::Eeprom4k => Some(512),
            Self::Eeprom64k => Some(8192),
            _ => None,
        }
    }
}

/// 存档读写结果（dump/write/verify 三种操作共用）。
pub struct SaveResult {
    pub success: bool,
    pub bytes: u64,
    pub mismatch_bytes: u32,
    pub seconds: f64,
}
