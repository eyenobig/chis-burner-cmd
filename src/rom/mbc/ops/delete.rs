//! MBC · 删：擦除 GB/GBC flash（整片 / 逐扇区）。
//!
//! 从参考源 `mission_mbc5.cs` 复刻（GB 总线 flash 命令序列，unlock 写 0xAAA/0x555）。
#![allow(dead_code)]

use std::time::{Duration, Instant};

use super::read::{bus_addr, switch_bank, BANK_SIZE};
use crate::cartridge_link::CartridgeLink;
use crate::rom::mbc::data::MbcKind;

/// 整片擦除并轮询完成（读 addr 0 == 0xFF）。
pub fn erase_chip(link: &mut CartridgeLink, timeout_secs: u64) -> bool {
    link.gbc_write(0x00, &[0xf0]);
    link.gbc_write(0xaaa, &[0xaa]);
    link.gbc_write(0x555, &[0x55]);
    link.gbc_write(0xaaa, &[0x80]);
    link.gbc_write(0xaaa, &[0xaa]);
    link.gbc_write(0x555, &[0x55]);
    link.gbc_write(0xaaa, &[0x10]); // Chip Erase

    let start = Instant::now();
    let mut probe = [0u8; 1];
    loop {
        if link.gbc_read(0, &mut probe) && probe[0] == 0xff {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1000));
        if start.elapsed().as_secs() > timeout_secs {
            return false;
        }
    }
}

fn erase_one_sector(link: &mut CartridgeLink, kind: MbcKind, off: u64) -> bool {
    let bank = (off >> 14) as u32;
    switch_bank(link, bank, kind);
    let sa = bus_addr(off as u32, kind);

    // 确保退出 CFI / 编程模式
    link.gbc_write(0x00, &[0xf0]);

    for _try in 0..3 {
        link.gbc_write(0xaaa, &[0xaa]);
        link.gbc_write(0x555, &[0x55]);
        link.gbc_write(0xaaa, &[0x80]);
        link.gbc_write(0xaaa, &[0xaa]);
        link.gbc_write(0x555, &[0x55]);
        link.gbc_write(sa, &[0x30]); // Sector Erase

        let start = Instant::now();
        let mut probe = [0u8; 1];
        loop {
            if link.gbc_read(sa, &mut probe) && probe[0] == 0xff {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
            // 大容量 NOR 单扇区偶发超过 10s；对齐 C# 无限等的上限放宽并允许重试
            if start.elapsed().as_secs() > 30 {
                link.gbc_write(0x00, &[0xf0]);
                break;
            }
        }
    }
    false
}

/// 擦除覆盖 [from,to) 的各扇区（`sector_size` 对齐；默认应按 CFI，常见 64KiB）。
/// flash 命令序列（unlock 0xAAA/0x555、0x30）MBC3/MBC5 相同，只有 bank 切换/地址按 kind 分发。
pub fn erase_range(
    link: &mut CartridgeLink,
    kind: MbcKind,
    from: u64,
    to: u64,
    sector_size: u32,
) -> bool {
    let ss = sector_size.max(BANK_SIZE) as u64;
    let mut off = from & !(ss - 1);
    while off < to {
        if !erase_one_sector(link, kind, off) {
            return false;
        }
        off += ss;
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
    let start_off = from & !(ss - 1);
    let total = ((to.saturating_sub(start_off) + ss - 1) / ss).max(1);
    let t0 = Instant::now();
    let mut off = start_off;
    let mut done = 0u64;
    log(&format!("扇区 {ss}B x {total}"));
    while off < to {
        if !erase_one_sector(link, kind, off) {
            log(&format!(
                "擦除失败 @0x{off:X} ({done}/{total}) | {:.1}s",
                t0.elapsed().as_secs_f64()
            ));
            return false;
        }
        done += 1;
        progress(done, total);
        off += ss;
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
