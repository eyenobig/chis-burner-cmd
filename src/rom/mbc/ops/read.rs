//! MBC · 读：GB/GBC 头解析、GB 头校验、RTC。
//!
//! **解析就绪，但 live 读取待移植**：读物理 GB 卡需 GB 总线协议（`gbcCart_read` + 分页，
//! 见参考源 `cart_adapter.cs`），尚未移植进 `cartridge_link`，故暂未接入 `cfb info`。
#![allow(dead_code)]

use crate::cartridge_link::CartridgeLink;
use crate::event::RomChecksum;
use crate::rom::mbc::data::{mbc_name, MbcHeader, MbcKind};

/// 16KB bank。线性 ROM 偏移 → bank 号。
pub const BANK_SIZE: u32 = 0x4000;

/// 选 ROM bank（按 MBC 代次分发，复刻 C# `mbc_romSwitchBank`）。
/// - MBC3：只写 0x2000，bank 0 重映射为 1（MBC3 规范：bank 0 只在固定区 0x0000-0x3FFF）。
/// - MBC5：先写 bit8 到 0x3000，再写低 8 位到 0x2000（9 位，0-511）。
pub fn switch_bank(link: &mut CartridgeLink, bank: u32, kind: MbcKind) {
    match kind {
        MbcKind::Mbc3 => {
            let mut b = (bank & 0xff) as u8;
            if b == 0 {
                b = 1;
            }
            link.gbc_write(0x2000, &[b]);
        }
        MbcKind::Mbc5 => {
            link.gbc_write(0x3000, &[((bank >> 8) & 0xff) as u8]);
            link.gbc_write(0x2000, &[(bank & 0xff) as u8]);
        }
    }
}

/// 线性 ROM 偏移 → GB 总线地址（按 MBC 代次分发，复刻 C# `mbc_BaseAddressOfBank`）。
/// - MBC3：bank 0 → 0x0000-0x3FFF（固定区）；其余 → 0x4000-0x7FFF。
/// - MBC5：恒 0x4000 + 低 14 位（MBC5 bank 0 不可经 0x4000 选中）。
pub fn bus_addr(rom_off: u32, kind: MbcKind) -> u32 {
    let bank = rom_off >> 14;
    let base = match kind {
        MbcKind::Mbc3 if bank == 0 => 0x0000,
        _ => 0x4000,
    };
    base + (rom_off & 0x3fff)
}

/// CFI 查询，返回 (device_size_bytes, buffer_write_bytes)。
pub fn rom_get_size(link: &mut CartridgeLink) -> (u64, u16) {
    link.gbc_write(0xaa, &[0x98]); // CFI Query
    let mut b = [0u8; 1];

    link.gbc_read(0x4e, &mut b);
    let dev_exp = b[0];
    let device_size = if dev_exp < 64 { 1u64 << dev_exp } else { 0 };

    link.gbc_read(0x54, &mut b);
    let buf_lo = b[0];
    link.gbc_read(0x56, &mut b);
    let buf_hi = b[0];
    let temp = ((buf_hi as u16) << 8) | buf_lo as u16;
    let buffer_write_bytes = if temp == 0 || buf_lo >= 16 { 0 } else { 1u16 << buf_lo };

    link.gbc_write(0x00, &[0xf0]); // reset
    (device_size, buffer_write_bytes)
}

/// 从卡带读 1 字节，自带 1 次重试以应对上电后第一条命令被 MCU 吞掉。
/// addr 落在 bank 0 固定区（0x0000-0x3FFF）时无需 switch_bank。
pub fn read_cart_byte(link: &mut CartridgeLink, addr: u32) -> Option<u8> {
    let mut b = [0u8; 1];
    for _ in 0..2 {
        if link.gbc_read(addr, &mut b) {
            return Some(b[0]);
        }
    }
    None
}

/// GB 头校验：stored=rom[0x14D]，computed = (Σ_{a=0x134..=0x14C} (-rom[a]-1)) & 0xFF。
pub fn header_checksum(rom: &[u8]) -> RomChecksum {
    if rom.len() <= 0x14D {
        return RomChecksum { stored: 0, computed: 0, ok: false };
    }
    let stored = rom[0x14D];
    let mut x: i32 = 0;
    for a in 0x134..=0x14C {
        x = x - rom[a] as i32 - 1;
    }
    let computed = (x & 0xFF) as u8;
    RomChecksum { stored, computed, ok: stored == computed }
}

/// MBC3+TIMER(+BATTERY) 即带 RTC。
pub fn has_rtc(cartridge_type: u8) -> bool {
    matches!(cartridge_type, 0x0F | 0x10)
}

/// 解析 GB/GBC 头（要求 rom 至少 0x150 字节）。
pub fn parse_header(rom: &[u8]) -> MbcHeader {
    let title = rom[0x134..0x143]
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
        .collect::<String>()
        .trim()
        .to_string();
    let cgb_flag = rom[0x143];
    let cartridge_type = rom[0x147];
    let rom_size_bytes = 32 * 1024u64 << rom.get(0x148).copied().unwrap_or(0).min(8);

    MbcHeader {
        title,
        cgb_flag,
        cartridge_type,
        mbc_name: mbc_name(cartridge_type),
        rom_size_bytes,
        header_checksum: header_checksum(rom),
        rtc: has_rtc(cartridge_type),
    }
}
