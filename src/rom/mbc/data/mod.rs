//! MBC 数据类型（GB/GBC 独有）：ROM 头 + **maptype**（cartridge type → MBC 名称）。
//!
//! maptype 是 GB 卡独有的信息：头部 0x147 的 cartridge type 字节决定用哪代 MBC 控制器。
use crate::event::RomChecksum;

/// GB/GBC ROM 头解析结果。
pub struct MbcHeader {
    pub title: String,    // 0x134..（CGB 可能已拆掉末尾 4 字 game code）
    /// GBC 标题区拆出的 4 字母代号（如 `AAUE`）；无则 None（可由 db_DMG 补全为 `DMG-APAE`）。
    pub game_code: Option<String>,
    /// 头 0x14C 版本号（flashGBX 的 Revision）。
    pub revision: u8,
    pub cgb_flag: u8,     // 0x143
    pub cartridge_type: u8,    // 0x147（maptype 原始字节）
    pub mbc_name: &'static str,
    pub rom_size_bytes: u64, // 0x148: 32KB << n
    /// RAM/存档大小（字节），来自头 0x149；0=无 RAM。
    #[allow(dead_code)]
    pub ram_size_bytes: u64,
    pub header_checksum: RomChecksum,
    pub rtc: bool,
}

/// MBC 控制器代次：决定 bank 切换寄存器序列与 bank 0 总线地址映射。
/// flash 命令序列（unlock/erase/program/CFI）两代完全相同，只有这两处不同。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MbcKind {
    Mbc3,
    Mbc5,
}

impl MbcKind {
    /// 由 ROM 头 cartridge_type(0x147) 推断 MBC 代次。
    /// 0x0F..=0x13 = MBC3（含 MBC3+RTC）；其余（含 0x19..=0x1E MBC5 及未知）按 MBC5 处理。
    pub fn from_cartridge_type(cartridge_type: u8) -> Self {
        match cartridge_type {
            0x0F..=0x13 => MbcKind::Mbc3,
            _ => MbcKind::Mbc5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MbcKind::Mbc3 => "MBC3",
            MbcKind::Mbc5 => "MBC5",
        }
    }
}

/// maptype：cartridge type(0x147) → MBC 名称（GB 独有）。
pub fn mbc_name(cartridge_type: u8) -> &'static str {
    match cartridge_type {
        0x00 => "ROM ONLY",
        0x01..=0x03 => "MBC1",
        0x05 | 0x06 => "MBC2",
        0x0B..=0x0D => "MMM01",
        0x0F..=0x13 => "MBC3",
        0x19..=0x1E => "MBC5",
        0x20 => "MBC6",
        0x22 => "MBC7",
        _ => "Unknown",
    }
}
