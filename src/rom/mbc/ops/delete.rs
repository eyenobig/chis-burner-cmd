//! MBC · 删：擦除 GB/GBC flash（整片 / 逐扇区）。
//!
//! 从参考源 `mission_mbc5.cs` 复刻（GB 总线 flash 命令序列，unlock 写 0xAAA/0x555）。
#![allow(dead_code)]

use std::time::{Duration, Instant};

use super::read::{bus_addr, flash_phys_addr, switch_bank, switch_bank_mbcx, BANK_SIZE};
use crate::cartridge_link::CartridgeLink;
use crate::rom::mbc::data::MbcKind;

/// 将 flash 物理扇区起点映射为「线性 bank + 总线擦除地址」。
/// MBC5 N+1：phys 0x0000 扇区用线性 bank0 / bus 0x4000 发令（同扇区内有效地址）。
fn sector_erase_target(kind: MbcKind, phys_sector: u32) -> (u32, u32) {
    match kind {
        MbcKind::Mbc5 => {
            let erase_phys = if phys_sector < 0x4000 {
                0x4000
            } else {
                phys_sector
            };
            let reg = erase_phys >> 14;
            let linear_bank = reg.saturating_sub(1);
            let sa = 0x4000 + (erase_phys & 0x3fff);
            (linear_bank, sa)
        }
        MbcKind::Mbc3 => {
            let bank = phys_sector >> 14;
            (bank, bus_addr(phys_sector, kind))
        }
    }
}

/// [from,to) 线性 ROM 区间覆盖到的 flash 物理扇区起点列表（去重升序）。
fn phys_sectors_covering(kind: MbcKind, from: u64, to: u64, sector_size: u32) -> Vec<u64> {
    let ss = sector_size.max(BANK_SIZE) as u64;
    if to <= from {
        return Vec::new();
    }
    let phys_lo = flash_phys_addr(from as u32, kind) as u64;
    let phys_hi = flash_phys_addr((to - 1) as u32, kind) as u64;
    let mut sec = phys_lo & !(ss - 1);
    let end = phys_hi & !(ss - 1);
    let mut out = Vec::new();
    while sec <= end {
        out.push(sec);
        sec += ss;
    }
    out
}

/// 整片擦除并轮询完成。
/// MBC5 窗口在 0x4000：轮询 `0x4000`（C# 轮询 0 在部分卡上会立刻读到总线空闲 0xFF，造成假完成）。
pub fn erase_chip(link: &mut CartridgeLink, timeout_secs: u64) -> bool {
    link.gbc_write(0x00, &[0xf0]);
    link.gbc_write(0xaaa, &[0xaa]);
    link.gbc_write(0x555, &[0x55]);
    link.gbc_write(0xaaa, &[0x80]);
    link.gbc_write(0xaaa, &[0xaa]);
    link.gbc_write(0x555, &[0x55]);
    link.gbc_write(0xaaa, &[0x10]); // Chip Erase

    switch_bank(link, 0, MbcKind::Mbc5);
    let probe_addr = bus_addr(0, MbcKind::Mbc5); // 0x4000
    let start = Instant::now();
    let mut probe = [0u8; 1];
    // 整片擦通常数十秒；3s 内的 0xFF 一律视为状态读假象
    const MIN_ERASE_MS: u128 = 60_000;
    std::thread::sleep(Duration::from_millis(500));
    loop {
        let elapsed_ms = start.elapsed().as_millis();
        if link.gbc_read(probe_addr, &mut probe) && probe[0] == 0xff && elapsed_ms >= MIN_ERASE_MS {
            std::thread::sleep(Duration::from_millis(100));
            if link.gbc_read(probe_addr, &mut probe) && probe[0] == 0xff {
                link.gbc_write(0x00, &[0xf0]);
                std::thread::sleep(Duration::from_millis(50));
                switch_bank(link, 0, MbcKind::Mbc5);
                let mut buf = [0u8; 256];
                let mut ok = link.gbc_read(probe_addr, &mut buf) && buf.iter().all(|&b| b == 0xff);
                if ok {
                    for b in [1u32, 2, 3, 4, 6] {
                        switch_bank(link, b, MbcKind::Mbc5);
                        ok = link.gbc_read(0x4000, &mut buf) && buf.iter().all(|&x| x == 0xff);
                        if !ok {
                            break;
                        }
                    }
                }
                switch_bank(link, 0, MbcKind::Mbc5);
                if ok {
                    // 再等一轮确认，避免擦除尚未真正结束
                    std::thread::sleep(Duration::from_millis(500));
                    link.gbc_write(0x00, &[0xf0]);
                    switch_bank(link, 0, MbcKind::Mbc5);
                    if link.gbc_read(probe_addr, &mut buf) && buf.iter().all(|&b| b == 0xff) {
                        return true;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        if start.elapsed().as_secs() > timeout_secs {
            return false;
        }
    }
}

fn erase_one_phys_sector(
    link: &mut CartridgeLink,
    kind: MbcKind,
    phys_sector: u64,
    sector_size: u32,
    flash_bank: &mut i32,
) -> bool {
    let (bank, sa) = sector_erase_target(kind, phys_sector as u32);
    switch_bank_mbcx(link, bank, kind, flash_bank);

    link.gbc_write(0x00, &[0xf0]);

    for _try in 0..3 {
        let t0 = Instant::now();
        link.gbc_write(0xaaa, &[0xaa]);
        link.gbc_write(0x555, &[0x55]);
        link.gbc_write(0xaaa, &[0x80]);
        link.gbc_write(0xaaa, &[0xaa]);
        link.gbc_write(0x555, &[0x55]);
        link.gbc_write(sa, &[0x30]); // Sector Erase

        let mut probe = [0u8; 1];
        loop {
            if link.gbc_read(sa, &mut probe) && probe[0] == 0xff {
                // 无论擦前是否空白，至少等 150ms，避免假完成
                if t0.elapsed().as_millis() >= 150 {
                    // 先退出可能的状态读，再抽查阵列数据
                    link.gbc_write(0x00, &[0xf0]);
                    std::thread::sleep(Duration::from_millis(5));
                    let (bank2, _) = sector_erase_target(kind, phys_sector as u32);
                    switch_bank_mbcx(link, bank2, kind, flash_bank);
                    if sector_blank_check(link, kind, phys_sector, sector_size, flash_bank) {
                        return true;
                    }
                    // 未空：重新发擦除
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
            if t0.elapsed().as_secs() > 30 {
                link.gbc_write(0x00, &[0xf0]);
                break;
            }
        }
    }
    false
}

/// 抽查扇区内多个 16B 窗口是否全 0xFF（覆盖扇区头/中/尾）。
fn sector_blank_check(
    link: &mut CartridgeLink,
    kind: MbcKind,
    phys_sector: u64,
    sector_size: u32,
    flash_bank: &mut i32,
) -> bool {
    let ss = sector_size.max(BANK_SIZE) as u64;
    let samples = [0u64, ss / 4, ss / 2, ss.saturating_sub(16)];
    let mut buf = [0u8; 16];
    for off in samples {
        let phys = phys_sector + off;
        let (bank, bus) = match kind {
            MbcKind::Mbc5 => {
                // phys→线性 bank：reg=phys>>14，linear=reg-1（reg0 用 bank0 读 0x0000 不适用，抬到 0x4000）
                let erase_phys = if phys < 0x4000 {
                    0x4000 + (phys & 0x3fff)
                } else {
                    phys
                } as u32;
                let reg = erase_phys >> 14;
                let linear = reg.saturating_sub(1);
                (linear, 0x4000 + (erase_phys & 0x3fff))
            }
            MbcKind::Mbc3 => {
                let p = phys as u32;
                (p >> 14, bus_addr(p, kind))
            }
        };
        switch_bank_mbcx(link, bank, kind, flash_bank);
        if !link.gbc_read(bus, &mut buf) || !buf.iter().all(|&b| b == 0xff) {
            return false;
        }
    }
    true
}

/// 擦除覆盖 [from,to) 的各扇区（按 **flash 物理地址** 对齐；MBC5 N+1 会多擦尾扇区）。
/// flash 命令序列（unlock 0xAAA/0x555、0x30）MBC3/MBC5 相同，只有 bank 切换/地址按 kind 分发。
pub fn erase_range(
    link: &mut CartridgeLink,
    kind: MbcKind,
    from: u64,
    to: u64,
    sector_size: u32,
) -> bool {
    let sectors = phys_sectors_covering(kind, from, to, sector_size);
    let mut flash_bank: i32 = -1;
    for sec in sectors {
        if !erase_one_phys_sector(link, kind, sec, sector_size, &mut flash_bank) {
            return false;
        }
    }
    true
}

/// 带进度/日志的区间擦除。`progress(done_sectors, total_sectors)`；失败返回 false。
pub fn erase_range_logged(
    link: &mut CartridgeLink,
    kind: MbcKind,
    from: u64,
    to: u64,
    sector_size: u32,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> bool {
    let ss = sector_size.max(BANK_SIZE) as u64;
    let sectors = phys_sectors_covering(kind, from, to, sector_size);
    let total = sectors.len().max(1) as u64;
    let t0 = Instant::now();
    let mut done = 0u64;
    let mut flash_bank: i32 = -1;
    log(&format!("扇区 {ss}B x {total} (phys)"));
    // 开局先报 0/total，避免首扇区耗时长时客户端一直停在裸「擦除」无分数。
    progress(0, total);
    for sec in sectors {
        if !erase_one_phys_sector(link, kind, sec, sector_size, &mut flash_bank) {
            log(&format!(
                "擦除失败 @phys=0x{sec:X} ({done}/{total}) | {:.1}s",
                t0.elapsed().as_secs_f64()
            ));
            return false;
        }
        done += 1;
        progress(done, total);
    }
    log(&format!(
        "擦除完成 {done}/{total} | {:.1}s",
        t0.elapsed().as_secs_f64()
    ));
    true
}

/// profile 驱动的 sector erase 区间（命中 profile 走命令序列，否则回落 [`erase_range`]）。
pub fn erase_range_profile(
    link: &mut CartridgeLink,
    kind: MbcKind,
    p: &crate::profile::Profile,
    from: u64,
    to: u64,
) -> bool {
    let Some(seq) = p.sector_erase() else {
        return erase_range(link, kind, from, to, BANK_SIZE);
    };
    let mut off = from & !(BANK_SIZE as u64 - 1);
    while off < to {
        let bank = (off >> 14) as u32;
        switch_bank(link, bank, kind);
        let sa = bus_addr(off as u32, kind);
        if !crate::profile::run_dmg(link, &seq, sa) {
            return false;
        }
        off += BANK_SIZE as u64;
    }
    true
}
