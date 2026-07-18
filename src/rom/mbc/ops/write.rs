//! MBC · 写：编程 / 烧录 GB/GBC ROM（MBC3 / MBC5 自动识别）。
//!
//! 流程化烧录：识别 MBC 代次 → CFI 查 flash 参数 → 空间校验 → 擦除目标区 →
//! 逐 bank 编程 → 读回校验。bank 切换/地址映射按 MbcKind 分发，flash 命令序列
//! （unlock 0xAAA/0x555、sector erase 0x30、program 0xfc）两代相同。
//! 参考 C# `mission_programRom_mbc5` + `mission_verifyRom_mbc5`。
#![allow(dead_code)]

use std::time::Instant;

use super::delete::erase_range;
use super::read::{bus_addr, rom_get_size, switch_bank};
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

    // ---- 步骤 1：识别 MBC 代次（从 ROM 文件头 0x147，不读卡带以避开上电首包被吞）----
    let cartridge_type = rom.get(0x147).copied().unwrap_or(0xFF);
    let kind = MbcKind::from_cartridge_type(cartridge_type);
    log(&format!(
        "识别: cartridge_type=0x{:02X} {} → {}",
        cartridge_type,
        mbc_name(cartridge_type),
        kind.label()
    ));

    // ---- 步骤 2：CFI 查 flash 容量 + 写缓冲（阶段 2 再用 profile 覆盖）----
    let (device_size, buf_wr) = rom_get_size(link);
    let buf_wr = if buf_wr == 0 { 32 } else { buf_wr };
    log(&format!("flash: 容量:{} BuffWr:{}", device_size, buf_wr));

    // ---- 步骤 3：空间校验 ----
    if device_size > 0 && length > device_size {
        log(&format!("空间不足: ROM {} > flash {}", length, device_size));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    // ---- 步骤 4：擦除目标区（逐 16KB bank sector erase）----
    log("擦除目标区 ...");
    if !erase_range(link, kind, 0, length) {
        log("擦除失败");
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    // ---- 步骤 5：写入（每 4096B 一包，失败重连复活）----
    log("开始写入 ...");
    if let Some(bad) = program_flow(link, kind, rom, 0, length, buf_wr, &mut res, length, progress) {
        res.first_bad = Some(bad);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }
    link.gbc_write(0x4000, &[0x00]); // settle bank reg（与 C# 一致，防震动）

    // ---- 步骤 6：读回校验 ----
    if verify {
        log("校验中 ...");
        let mm = verify_flow(link, kind, rom, length, progress, log, &mut res);
        res.mismatch_bytes = mm;
        log(&format!("校验: {} 字节不符", mm));
    }

    res.success = res.first_bad.is_none() && res.mismatch_bytes == 0;
    res.seconds = start.elapsed().as_secs_f64();
    res
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
