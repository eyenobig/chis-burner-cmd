//! GBA · 删：擦除 flash（整片 / 逐扇区 / PPB 解锁）。
//!
//! 从参考源 `GbaFlasher.cs` 的 `EraseChip` / `EraseSector` / `UnlockAllPpb` 复刻。
//! ⚠️ 未经硬件测试（见根目录 TODO.md）。

use std::time::{Duration, Instant};

use crate::cartridge_link::CartridgeLink;

/// 全片擦除并等待完成（轮询读到 0xFFFF）。
pub fn erase_chip(link: &mut CartridgeLink, timeout_secs: u64) -> bool {
    if !link.rom_erase_chip() {
        return false;
    }
    let start = Instant::now();
    let mut probe = [0u8; 2];
    loop {
        if link.rom_read(0, &mut probe) && probe == [0xff, 0xff] {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
        if start.elapsed().as_secs() > timeout_secs {
            return false;
        }
    }
}

/// 扇区擦除（byte_base 须扇区对齐）。失败自动重连重试。
pub fn erase_sector(link: &mut CartridgeLink, byte_base: u32, retries: u32) -> bool {
    let mut probe = [0u8; 2];
    for _ in 0..retries {
        link.rom_write(0x555, &[0xaa, 0x00]);
        link.rom_write(0x2aa, &[0x55, 0x00]);
        link.rom_write(0x555, &[0x80, 0x00]);
        link.rom_write(0x555, &[0xaa, 0x00]);
        link.rom_write(0x2aa, &[0x55, 0x00]);
        link.rom_write(byte_base >> 1, &[0x30, 0x00]);

        let start = Instant::now();
        loop {
            if link.rom_read(byte_base, &mut probe) && probe == [0xff, 0xff] {
                return true;
            }
            if start.elapsed().as_secs() > 6 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = link.reconnect();
    }
    false
}

/// 带进度/日志的区间擦除。`progress(done_sectors, total_sectors)`；失败返回 false。
/// 逐扇区调用 [`erase_sector`]，用于 `cfb erase` 展示实时进度（整片 `erase_chip` 无法分段汇报）。
pub fn erase_range_logged(
    link: &mut CartridgeLink,
    from: u32,
    to: u32,
    sector_size: u32,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> bool {
    let ss = sector_size.max(0x1000);
    let start = from - (from % ss);
    let total = (((to.saturating_sub(start)) as u64 + ss as u64 - 1) / ss as u64).max(1);
    let t0 = Instant::now();
    let mut off = start;
    let mut done = 0u64;
    log(&format!("扇区 {ss}B x {total}"));
    while off < to {
        if !erase_sector(link, off, 5) {
            log(&format!(
                "擦除失败 @0x{off:X} ({done}/{total}) | {:.1}s",
                t0.elapsed().as_secs_f64()
            ));
            return false;
        }
        done += 1;
        progress(done, total);
        off = off.saturating_add(ss);
    }
    log(&format!("擦除完成 {done}/{total} | {:.1}s", t0.elapsed().as_secs_f64()));
    true
}

/// All PPB Erase：清除全部扇区的持久保护位（上半区写不进时多半因 PPB）。
pub fn unlock_all_ppb(link: &mut CartridgeLink) {
    // 退出任何命令集
    link.rom_write(0, &[0x90, 0x00]);
    link.rom_write(0, &[0x00, 0x00]);
    link.rom_write(0, &[0xf0, 0x00]);
    // 进入非易失扇区保护命令集并 All PPB Erase
    link.rom_write(0x555, &[0xaa, 0x00]);
    link.rom_write(0x2aa, &[0x55, 0x00]);
    link.rom_write(0x555, &[0xc0, 0x00]);
    link.rom_write(0, &[0x80, 0x00]);
    link.rom_write(0, &[0x30, 0x00]); // All PPB Erase
    std::thread::sleep(Duration::from_millis(2000));
    link.rom_write(0, &[0x90, 0x00]);
    link.rom_write(0, &[0x00, 0x00]);
    link.rom_write(0, &[0xf0, 0x00]);
}

// ============ profile 驱动的擦除（命中 profile 时走命令序列，否则用上面的硬编码）============

/// profile 命中时用 chip_erase 序列；否则回落 [`erase_chip`]。
pub fn chip_erase_profile(link: &mut CartridgeLink, p: &crate::profile::Profile, timeout_secs: u64) -> bool {
    if let Some(seq) = p.chip_erase() {
        let to = p.chip_erase_timeout.max(timeout_secs);
        // 命中后跑序列；run_gba 内部已含每条 cmd 的 wait_for 轮询。
        // 但 chip_erase 的超时需整体兜底，这里包一层：序列失败再重试一次。
        for _ in 0..3 {
            if crate::profile::run_gba(link, &seq, 0) {
                return true;
            }
            let _ = link.reconnect();
        }
        false
    } else {
        erase_chip(link, timeout_secs)
    }
}

/// profile 命中时用 sector_erase 序列擦 `byte_base` 扇区；否则回落 [`erase_sector`]。
pub fn sector_erase_profile(link: &mut CartridgeLink, p: &crate::profile::Profile, byte_base: u32, retries: u32) -> bool {
    if let Some(seq) = p.sector_erase() {
        for _ in 0..retries {
            if crate::profile::run_gba(link, &seq, byte_base) {
                return true;
            }
            let _ = link.reconnect();
        }
        false
    } else {
        erase_sector(link, byte_base, retries)
    }
}
