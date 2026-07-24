//! MBC · 读：GB/GBC 头解析、GB 头校验、RTC。
//!
//! 物理卡读取使用 `CartridgeLink::gbc_read`（固件命令 0xfb）。

use crate::cartridge_link::CartridgeLink;
use crate::event::RomChecksum;
use crate::rom::common;
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

/// CFI 查询，返回 (device_size_bytes, buffer_write_bytes, sector_size_bytes)。
/// `sector_size` 优先读 CFI 均匀扇区；读不到时默认 64KiB（多数 GB 复制卡 NOR）。
/// 失败会重试：先复位再进 CFI，避免残留 CFI 模式导致后续擦除失败。
pub fn rom_get_cfi(link: &mut CartridgeLink) -> (u64, u16, u32) {
    const DEFAULT_SECTOR: u32 = 64 * 1024;
    for attempt in 0..3 {
        link.gbc_write(0x00, &[0xf0]);
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        link.gbc_write(0xaa, &[0x98]); // CFI Query
        let mut b = [0u8; 1];

        link.gbc_read(0x4e, &mut b);
        let dev_exp = b[0];
        let device_size = if (1..64).contains(&dev_exp) {
            1u64 << dev_exp
        } else {
            0
        };

        link.gbc_read(0x54, &mut b);
        let buf_lo = b[0];
        link.gbc_read(0x56, &mut b);
        let buf_hi = b[0];
        let temp = ((buf_hi as u16) << 8) | buf_lo as u16;
        let buffer_write_bytes = if temp == 0 || buf_lo >= 16 { 0 } else { 1u16 << buf_lo };

        // 均匀扇区：CFI 0x2F/0x30（字节地址 0x5E/0x60）× 256，对齐 FlashGBX / mission_mbc5
        link.gbc_read(0x5e, &mut b);
        let sec_lo = b[0];
        link.gbc_read(0x60, &mut b);
        let sec_hi = b[0];
        let sector_raw = (((sec_hi as u32) << 8) | sec_lo as u32).saturating_mul(256);
        let sector_size = if sector_raw >= 0x1000 && sector_raw <= 0x20000 {
            sector_raw
        } else {
            DEFAULT_SECTOR
        };

        link.gbc_write(0x00, &[0xf0]); // reset
        std::thread::sleep(std::time::Duration::from_millis(5));

        if device_size > 0 {
            return (device_size, buffer_write_bytes, sector_size);
        }
    }
    (0, 0, DEFAULT_SECTOR)
}

/// CFI 查询，返回 (device_size_bytes, buffer_write_bytes)。
pub fn rom_get_size(link: &mut CartridgeLink) -> (u64, u16) {
    let (size, buf, _) = rom_get_cfi(link);
    (size, buf)
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

/// 读取物理 GB/GBC 卡头部（0x180 字节，供头校验 + db_DMG SHA1）。
/// 无有效 GB 头时返回 None。
pub fn read_live_header(link: &mut CartridgeLink) -> Option<[u8; 0x180]> {
    let mut header = [0u8; 0x180];
    for _ in 0..2 {
        if link.gbc_read(0, &mut header) {
            if is_gb_header(&header) {
                return Some(header);
            }
            return None;
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

/// GBC 标题末 4 字是否像 flashGBX 的 manufacturer/game code。
fn looks_like_gbc_game_code(code: &str) -> bool {
    let b = code.as_bytes();
    if b.len() != 4 {
        return false;
    }
    matches!(b[0], b'A' | b'B' | b'H' | b'K' | b'V')
        && matches!(
            b[3],
            b'A' | b'B' | b'D' | b'E' | b'F' | b'I' | b'J' | b'K' | b'P' | b'S' | b'U' | b'X' | b'Y'
        )
}

/// 解析 GB/GBC 头（要求 rom 至少 0x150 字节）。
/// 复刻 flashGBX `RomFileDMG.GetHeader`：GBC 且标题区满 15 字时拆末尾 4 字为 game_code。
pub fn parse_header(rom: &[u8]) -> MbcHeader {
    let cgb_flag = rom.get(0x143).copied().unwrap_or(0);
    let is_cgb = matches!(cgb_flag, 0x80 | 0xC0);

    // CGB：标题区 0x134..0x143（15B）；DMG：0x134..0x144（16B，末字节常为 0）。
    let title_end = if is_cgb { 0x143 } else { 0x144.min(rom.len()) };
    let title_end = title_end.min(rom.len()).max(0x134);
    let mut title: String = rom[0x134..title_end]
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
        .collect::<String>()
        .trim()
        .to_string();

    let mut game_code = None;
    if is_cgb {
        let raw_len = rom[0x134..0x143.min(rom.len())]
            .iter()
            .rev()
            .skip_while(|&&b| b == 0)
            .count();
        // flashGBX：rstrip(\\0) 后长度恰为 15 才尝试拆码
        if raw_len == 15 && title.len() >= 4 {
            let code = title[title.len() - 4..].to_string();
            if looks_like_gbc_game_code(&code) {
                title = title[..title.len() - 4].trim_end_matches('_').to_string();
                game_code = Some(code);
            }
        }
    }

    let cartridge_type = rom.get(0x147).copied().unwrap_or(0);
    let rom_size_bytes = (32 * 1024u64) << rom.get(0x148).copied().unwrap_or(0).min(8);
    let ram_size_bytes = ram_size(rom.get(0x149).copied().unwrap_or(0));
    let revision = rom.get(0x14C).copied().unwrap_or(0);

    MbcHeader {
        title,
        game_code,
        revision,
        cgb_flag,
        cartridge_type,
        mbc_name: mbc_name(cartridge_type),
        rom_size_bytes,
        ram_size_bytes,
        header_checksum: header_checksum(rom),
        rtc: has_rtc(cartridge_type),
    }
}

/// 判别是否有效 GB/GBC 头：非空片且头校验通过。
///
/// `info --mbc` 在 GBA 卡 / 空白 flash 上也可能 `gbc_read` 成功并解析出空标题；
/// 必须用此闸门，否则客户端会把「假 MBC 命中」当成已识别而不再回退 GBA。
pub fn is_gb_header(rom: &[u8]) -> bool {
    rom.len() >= 0x150
        && !common::ops::is_blank(&rom[..0x150])
        && header_checksum(rom).ok
}

/// 头 0x149 RAM size 编码 → 字节数（标准 GB 表）。
/// 0x00=无 0x01=2K 0x02=8K 0x03=32K 0x04=128K 0x05=64K；其余未知按 0。
pub fn ram_size(code: u8) -> u64 {
    match code {
        0x00 => 0,
        0x01 => 2 * 1024,
        0x02 => 8 * 1024,
        0x03 => 32 * 1024,
        0x04 => 128 * 1024,
        0x05 => 64 * 1024,
        _ => 0,
    }
}
