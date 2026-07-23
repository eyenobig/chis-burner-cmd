//! MBC · 删：擦除 GB/GBC flash（整片 / 逐扇区）。
//!
//! 从参考源 `mission_mbc5.cs` 复刻（GB 总线 flash 命令序列，unlock 写 0xAAA/0x555）。
//! ⚠️ 未经硬件测试（见根目录 TODO.md）。
#![allow(dead_code)]

use std::time::{Duration, Instant};

use super::read::{bus_addr, switch_bank, BANK_SIZE};
use crate::cartridge_link::CartridgeLink;
use crate::rom::mbc::data::MbcKind;

/// 整片擦除并轮询完成（读 addr 0 == 0xFF）。
pub fn erase_chip(link: &mut CartridgeLink, timeout_secs: u64) -> bool {
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

/// 擦除覆盖 [from,to) 的各 16KB bank 区（用 bank 基地址做 sector erase）。
/// flash 命令序列（unlock 0xAAA/0x555、0x30）MBC3/MBC5 相同，只有 bank 切换/地址按 kind 分发。
pub fn erase_range(link: &mut CartridgeLink, kind: MbcKind, from: u64, to: u64) -> bool {
    let mut off = from & !(BANK_SIZE as u64 - 1);
    while off < to {
        let bank = (off >> 14) as u32;
        switch_bank(link, bank, kind);
        let sa = bus_addr(off as u32, kind);

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
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            if start.elapsed().as_secs() > 10 {
                return false;
            }
        }
        off += BANK_SIZE as u64;
    }
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
        return erase_range(link, kind, from, to);
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
