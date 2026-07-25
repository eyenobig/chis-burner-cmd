//! MBC · 读：GB/GBC 头解析、GB 头校验、RTC。
//!
//! 物理卡读取使用 `CartridgeLink::gbc_read`（固件命令 0xfb）。

use crate::cartridge_link::CartridgeLink;
use crate::event::RomChecksum;
use crate::rom::common;
use crate::rom::mbc::data::{mbc_name, MbcHeader, MbcKind};

/// 16KB bank。线性 ROM 偏移 → bank 号。
pub const BANK_SIZE: u32 = 0x4000;

/// 选 ROM bank。
/// - MBC3：写 0x2000；bank 0 → 1。
/// - MBC5 / 本机 Chis 复制卡（硬件实测 2026-07-24 COM13）：
///   - 寄存器 **0 与 1 读回同一物理页**（用 0x2000 或 0x2100 皆然）。
///   - 寄存器 ≥2 可正常切换。
///   - 因此线性 ROM bank `N` → 寄存器 `N+1`，地址用 **0x2100**（FlashGBX MBC5）。
///   - 用 C# 的「N→0x2000」时烧录稳定死在 `@0x4000`；改 N+1 后 bank0–2 读回校验 0 mismatch。
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
            let reg = bank.saturating_add(1);
            link.gbc_write(0x3000, &[((reg >> 8) & 0xff) as u8]);
            // 低 8 位：C# 用 0x2000；FlashGBX 用 0x2100。本机两者可读，写回用 0x2000 对齐固件/C#。
            link.gbc_write(0x2000, &[(reg & 0xff) as u8]);
        }
    }
}

/// FlashGBX `DMG_Unlicensed_MBCX.SelectBankFlash`：>8MiB 多 die。
/// **beggar_socket 无此步骤**；普通 Chis 复制卡调用会打乱总线状态，导致烧录在 0x4000 失败。
/// 仅在确认 MBCX 多 die 时使用；常规 burn/erase 不要调用。
#[allow(dead_code)]
pub fn switch_flash_bank(link: &mut CartridgeLink, flash_bank: u32) {
    link.gbc_write(0x0000, &[0x05]);
    link.gbc_write(0x4000, &[0x82]);
    link.gbc_write(0xa000, &[(flash_bank & 0xff) as u8]);
    link.gbc_write(0x0000, &[0x00]);
}

/// MBC bank 切换。常规路径只走 [`switch_bank`]（对齐 beggar_socket）。
/// `flash_bank` 参数保留以兼容旧调用点，**不再**自动 `switch_flash_bank`。
pub fn switch_bank_mbcx(link: &mut CartridgeLink, bank: u32, kind: MbcKind, _flash_bank: &mut i32) {
    switch_bank(link, bank, kind);
}

/// 线性 ROM 偏移 → GB 总线地址（按 MBC 代次分发）。
/// - MBC3：bank 0 → 0x0000-0x3FFF（固定区）；其余 → 0x4000-0x7FFF。
/// - MBC5 / ChisFlash MBCX：恒 0x4000 + 低 14 位（bank 0 也在 0x4000 窗口，见 FlashGBX start_addr）。
pub fn bus_addr(rom_off: u32, kind: MbcKind) -> u32 {
    let bank = rom_off >> 14;
    let base = match kind {
        MbcKind::Mbc3 if bank == 0 => 0x0000,
        _ => 0x4000,
    };
    base + (rom_off & 0x3fff)
}

/// 线性 ROM 偏移 → flash 芯片物理字节地址（用于扇区擦覆盖计算）。
/// MBC5 本机卡：`switch_bank` 用 N→N+1，故 phys = ((bank+1)<<14) | off14。
/// MBC3：bank0 固定区 phys=off；其余 bank 与线性一致。
pub fn flash_phys_addr(rom_off: u32, kind: MbcKind) -> u32 {
    let bank = rom_off >> 14;
    let off14 = rom_off & 0x3fff;
    match kind {
        MbcKind::Mbc5 => ((bank.saturating_add(1)) << 14) | off14,
        MbcKind::Mbc3 => rom_off,
    }
}

/// CFI 查询，返回 (device_size_bytes, buffer_write_bytes, sector_size_bytes)。
/// `sector_size` 优先读 CFI 均匀扇区；读不到时默认 64KiB（多数 GB 复制卡 NOR）。
/// 失败最多再试 1 次：先复位再进 CFI，避免残留 CFI 模式导致后续擦除失败。
pub fn rom_get_cfi(link: &mut CartridgeLink) -> (u64, u16, u32) {
    const DEFAULT_SECTOR: u32 = 64 * 1024;
    for attempt in 0..2 {
        link.gbc_write(0x00, &[0xf0]);
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(8));
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
        // GB 复制卡均匀扇区多为 4~64KiB；128KiB 上限易被总线噪声误读成 0x20000
        let sector_size = if sector_raw >= 0x1000 && sector_raw <= 0x10000 {
            sector_raw
        } else {
            DEFAULT_SECTOR
        };

        link.gbc_write(0x00, &[0xf0]); // reset
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

/// Autoselect ID（C# `mbc5_romGetID`）：4 字节 `[mfr, id0, id1, id2]`。
pub fn rom_get_id(link: &mut CartridgeLink) -> [u8; 4] {
    link.gbc_write(0xaaa, &[0xaa]);
    link.gbc_write(0x555, &[0x55]);
    link.gbc_write(0xaaa, &[0x90]);
    let mut id = [0u8; 4];
    let mut b = [0u8; 1];
    link.gbc_read(0x00, &mut b);
    id[0] = b[0];
    link.gbc_read(0x02, &mut b);
    id[1] = b[0];
    link.gbc_read(0x1c, &mut b);
    id[2] = b[0];
    link.gbc_read(0x1e, &mut b);
    id[3] = b[0];
    link.gbc_write(0x00, &[0xf0]);
    id
}

/// CFI 整片擦 typical 时间（毫秒）。对齐 C# `mbc5_romCalEraseTime`（简化）。
pub fn rom_cal_erase_time_ms(link: &mut CartridgeLink) -> u64 {
    link.gbc_write(0xaa, &[0x98]);
    let mut cfi = [0u8; 1];
    link.gbc_read(0x42, &mut cfi);
    let timeout_block = 1u64 << cfi[0].min(30);
    link.gbc_read(0x44, &mut cfi);
    let timeout_chip = 1u64 << cfi[0].min(30);
    link.gbc_write(0x00, &[0xf0]);
    let mut ms = if timeout_chip <= 1 {
        timeout_block.saturating_mul(512)
    } else {
        timeout_chip
    };
    if ms < 5_000 {
        ms = 5_000;
    }
    if ms > 600_000 {
        ms = 600_000;
    }
    ms
}

/// Intel/Numonyx JS28F256 Autoselect：强制 buffer write = 256。
pub fn is_js28f256(id: &[u8; 4]) -> bool {
    *id == [0x89, 0x7e, 0x22, 0x01]
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

/// 头区探测结果：有有效游戏头，或没有（空白/损坏/读失败）。
/// `info` 在 `NoGame` 时仍可用 CFI 判断 flash 是否在位，避免把空片当成「无卡带」。
#[derive(Debug)]
pub enum HeaderProbe {
    Valid([u8; 0x180]),
    NoGame,
}

/// 按 MBC 映射读 ROM 线性偏移 0 处的 0x180 头（MBC5 在 0x4000，MBC3 在 0x0000）。
fn read_header_at(link: &mut CartridgeLink, kind: MbcKind, out: &mut [u8; 0x180]) -> bool {
    switch_bank(link, 0, kind);
    link.gbc_read(bus_addr(0, kind), out)
}

/// 探测物理 GB/GBC 卡头部（快路径优先，接触抖动只做短重试）。
///
/// 复制卡默认按 MBC5 接线：ROM 偏移 0 在 GB 总线 `0x4000`（bank 0），不是 `0x0000`。
/// 因此先试 MBC5 窗口，再试 MBC3 固定区。
///
/// - 有效头：立刻返回 `Valid`
/// - 连续稳定全 0xFF：早退 `NoGame`（真空白，交给 CFI 判在位）
/// - 读失败 / 校验失败：最多 3 轮，间隔短 sleep + flash 复位
pub fn probe_live_header(link: &mut CartridgeLink) -> HeaderProbe {
    const ATTEMPTS: u32 = 3;
    let mut header = [0u8; 0x180];
    let mut blank_streak = 0u32;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            link.gbc_write(0x00, &[0xf0]);
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        let mut got = false;
        let mut saw_blank = false;
        // MBC5 优先（烧录空片 / 多数 GB 复制卡）；再回落 MBC3 固定区。
        for kind in [MbcKind::Mbc5, MbcKind::Mbc3] {
            if !read_header_at(link, kind, &mut header) {
                continue;
            }
            got = true;
            if is_gb_header(&header) {
                return HeaderProbe::Valid(header);
            }
            if common::ops::is_blank(&header[..0x150.min(header.len())]) {
                saw_blank = true;
            }
        }
        if !got {
            blank_streak = 0;
            continue;
        }
        if saw_blank {
            blank_streak += 1;
            if blank_streak >= 2 {
                return HeaderProbe::NoGame;
            }
            continue;
        }
        blank_streak = 0;
    }
    HeaderProbe::NoGame
}

/// 读取物理 GB/GBC 卡头部（0x180 字节）。无有效 GB 头时返回 None。
pub fn read_live_header(link: &mut CartridgeLink) -> Option<[u8; 0x180]> {
    match probe_live_header(link) {
        HeaderProbe::Valid(h) => Some(h),
        HeaderProbe::NoGame => None,
    }
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
