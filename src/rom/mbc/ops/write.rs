//! MBC · 写：编程 / 烧录 GB/GBC ROM（MBC3 / MBC5 自动识别）。
//!
//! 流程化烧录：识别 MBC 代次 → CFI 查 flash 参数 → 空间校验 → 擦除目标区 →
//! 逐 bank 编程 → 读回校验。bank 切换/地址映射按 MbcKind 分发，flash 命令序列
//! （unlock 0xAAA/0x555、sector erase 0x30、program 0xfc）两代相同。
//! 参考 C# `mission_programRom_mbc5` + `mission_verifyRom_mbc5`。
#![allow(dead_code)]

use std::time::Instant;

use super::delete::erase_range_logged;
use super::read::{bus_addr, rom_get_cfi, switch_bank};
use crate::cartridge_link::CartridgeLink;
use crate::rom::gba::data::BurnResult;
use crate::rom::mbc::data::{mbc_name, MbcKind};

const PACKET: usize = 4096;

/// 烧录 GB/GBC ROM：识别 → 查 flash → 空间校验 → 擦除 → 写入 → 校验。
/// 调用前应已上 5V 电（`device::power(link, Voltage::V5)`）。
pub fn burn(
    link: &mut CartridgeLink,
    rom: &[u8],
    verify: bool,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> BurnResult {
    let length = rom.len() as u64;
    let mut res = BurnResult {
        success: false,
        bytes_written: 0,
        reconnects: 0,
        first_bad: None,
        mismatch_bytes: 0,
        seconds: 0.0,
    };
    let start = Instant::now();
    let elapsed = |s: &Instant| format!("{:.1}s", s.elapsed().as_secs_f64());

    // ---- 步骤 1：识别 MBC 代次 ----
    // ROM 头 0x147 只表示游戏 mapper；烧录器 flash 卡多数是 MBC5 总线。
    // 若按 ROM 头走 MBC3，高 bank（≥0x10000）扇区擦除会失败。优先用卡上实读类型，
    // 无效则对 flash 烧录默认 MBC5（与 C# / 多数 GB 复制卡一致）。
    let file_ct = rom.get(0x147).copied().unwrap_or(0xFF);
    let live_ct = super::read::read_cart_byte(link, 0x147).unwrap_or(0xFF);
    let kind = match live_ct {
        0x0F..=0x13 => MbcKind::Mbc3,
        0x19..=0x1E => MbcKind::Mbc5,
        _ => MbcKind::Mbc5, // 空白片 / 噪声头：按 MBC5 烧
    };
    log(&format!(
        "识别: ROM=0x{:02X} {} / cart=0x{:02X} -> {}",
        file_ct,
        mbc_name(file_ct),
        live_ct,
        kind.label()
    ));

    // ---- 步骤 2：CFI 查 flash 容量 + 写缓冲 + 扇区大小 ----
    let (device_size, buf_wr, sector_size) = rom_get_cfi(link);
    let buf_wr = if buf_wr == 0 { 32 } else { buf_wr };
    if device_size == 0 {
        log(&format!(
            "flash: size=? rom={} sector={} buf={}",
            length, sector_size, buf_wr
        ));
    } else {
        log(&format!(
            "flash: size={} buf={} sector={}",
            device_size, buf_wr, sector_size
        ));
    }

    // ---- 步骤 3：空间校验 ----
    if device_size > 0 && length > device_size {
        log(&format!("空间不足: ROM {} > flash {}", length, device_size));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    // CFI 后强制复位，避免残留查询模式导致擦除无响应
    link.gbc_write(0x00, &[0xf0]);
    link.gbc_warm_up();

    // ---- 步骤 4：按 CFI 扇区对齐擦除 ----
    log("擦除目标区 ...");
    if !erase_range_logged(link, kind, 0, length, sector_size, progress, log) {
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    // ---- 步骤 5：写入 ----
    log("开始写入 ...");
    if let Some(bad) = program_flow(link, kind, rom, 0, length, buf_wr, &mut res, length, progress) {
        log(&format!("写入失败 @0x{bad:X} | {}", elapsed(&start)));
        res.first_bad = Some(bad);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }
    link.gbc_write(0x4000, &[0x00]);

    // ---- 步骤 6：读回校验 ----
    if verify {
        log("校验中 ...");
        let mm = verify_flow(link, kind, rom, length, progress, log, &mut res);
        res.mismatch_bytes = mm;
        log(&format!("校验: {} 字节不符 | {}", mm, elapsed(&start)));
    }

    res.success = res.first_bad.is_none() && res.mismatch_bytes == 0;
    res.seconds = start.elapsed().as_secs_f64();
    res
}

/// 编程 [from,to)：每包 0xfc 必 ACK 才前进，连续失败每 5 包重连一次，60 次放弃。
/// 重连（内部走 GBA warm_up）不重置 MBC bank 寄存器，故重连后必须重新 switch_bank。
pub(crate) fn program_range(
    link: &mut CartridgeLink,
    kind: MbcKind,
    data: &[u8],
    rom_base: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<u64> {
    let mut res = BurnResult {
        success: false,
        bytes_written: 0,
        reconnects: 0,
        first_bad: None,
        mismatch_bytes: 0,
        seconds: 0.0,
    };
    let total = data.len() as u64;
    // program_flow 把 rom 索引当作绝对 ROM 偏移；这里 data 是相对映像，需映射到 rom_base。
    // 复用 program_flow：构造“假 ROM”太浪费；直接内联同样循环。
    let mut written = 0u64;
    let mut current_bank: i64 = -1;
    while written < total {
        let len = ((total - written) as usize).min(PACKET);
        let pk = &data[written as usize..written as usize + len];
        let rom_off = (rom_base + written) as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            current_bank = bank;
            switch_bank(link, bank as u32, kind);
        }
        let cartridge_addr = bus_addr(rom_off, kind);
        let mut tries = 0;
        loop {
            if link.gbc_rom_program(cartridge_addr, pk, 32) {
                break;
            }
            tries += 1;
            if tries % 5 == 0 {
                res.reconnects += 1;
                let _ = link.reconnect();
                switch_bank(link, bank as u32, kind);
            }
            if tries >= 60 {
                return Some(rom_base + written);
            }
        }
        written += len as u64;
        progress(written, total);
    }
    let _ = res;
    None
}

/// 编程 [from,to)：每包 0xfc 必 ACK 才前进，连续失败每 5 包重连一次，60 次放弃。
/// 重连（内部走 GBA warm_up）不重置 MBC bank 寄存器，故重连后必须重新 switch_bank。
fn program_flow(
    link: &mut CartridgeLink,
    kind: MbcKind,
    rom: &[u8],
    from: u64,
    to: u64,
    buf_wr: u16,
    res: &mut BurnResult,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<u64> {
    let mut written = from;
    let mut current_bank: i64 = -1;
    while written < to {
        let len = ((to - written) as usize).min(PACKET);
        let pk = &rom[written as usize..written as usize + len];
        let rom_off = written as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            current_bank = bank;
            switch_bank(link, bank as u32, kind);
        }
        let cartridge_addr = bus_addr(rom_off, kind);

        let mut tries = 0;
        loop {
            if link.gbc_rom_program(cartridge_addr, pk, buf_wr) {
                break;
            }
            tries += 1;
            if tries % 5 == 0 {
                res.reconnects += 1;
                let _ = link.reconnect();
                switch_bank(link, bank as u32, kind); // 重连后 re-arm bank
            }
            if tries >= 60 {
                return Some(written);
            }
        }

        written += len as u64;
        res.bytes_written += len as u64;
        progress(written, total);
    }
    None
}

/// 校验 [0,total) 与 ROM 数据，返回不符字节数。4096B 粒度 gbc_read，与 program_flow 同寻址。
/// 单包读失败 → 重连 + 重 switch_bank + 重试本包（不前进 read）。
fn verify_flow(
    link: &mut CartridgeLink,
    kind: MbcKind,
    rom: &[u8],
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
    res: &mut BurnResult,
) -> u32 {
    let mut mismatch = 0u32;
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut buf = vec![0u8; PACKET];
    while read < total {
        let n = ((total - read) as usize).min(PACKET);
        let rom_off = read as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            current_bank = bank;
            switch_bank(link, bank as u32, kind);
        }
        let cartridge_addr = bus_addr(rom_off, kind);
        let b = &mut buf[..n];
        if !link.gbc_read(cartridge_addr, b) {
            res.reconnects += 1;
            let _ = link.reconnect();
            switch_bank(link, bank as u32, kind);
            continue;
        }
        for i in 0..n {
            if b[i] != rom[read as usize + i] {
                mismatch += 1;
                if res.first_bad.is_none() {
                    res.first_bad = Some(read + i as u64);
                    log(&format!(
                        "0x{:08X} 校验失败: {:02X} → {:02X}",
                        read as u64 + i as u64,
                        rom[read as usize + i],
                        b[i]
                    ));
                }
            }
        }
        read += n as u64;
        progress(read, total);
    }
    mismatch
}
