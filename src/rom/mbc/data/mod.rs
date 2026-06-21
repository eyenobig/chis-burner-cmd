//! MBC 数据类型（GB/GBC 独有）：ROM 头 + **maptype**（cartridge type → MBC 名称）。
//!
//! maptype 是 GB 卡独有的信息：头部 0x147 的 cartridge type 字节决定用哪代 MBC 控制器。
#![allow(dead_code)] // live 读取待移植 GB 总线协议后接通

use crate::event::RomChecksum;

/// GB/GBC ROM 头解析结果。
pub struct MbcHeader {
    pub title: String,    // 0x134..0x142
    pub cgb_flag: u8,     // 0x143
    pub cartridge_type: u8,    // 0x147（maptype 原始字节）
    pub mbc_name: &'static str,
    pub rom_size_bytes: u64, // 0x148: 32KB << n
    pub header_checksum: RomChecksum,
    pub rtc: bool,
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
