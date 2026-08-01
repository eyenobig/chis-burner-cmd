//! GBA · 删：擦除 flash（整片 / 逐扇区 / PPB 解锁）。
//!
//! 从参考源 `GbaFlasher.cs` 的 `EraseChip` / `EraseSector` / `UnlockAllPpb` 复刻。

use std::time::{Duration, Instant};

use crate::cartridge_link::CartridgeLink;
use crate::progress_display::{Phase, ProgressLog};

/// 全片擦除并等待完成（轮询读到 0xFFFF）。最简原语；烧录/擦除命令用带进度心跳的
/// [`erase_chip_logged`]。保留此处作为无 progress 依赖的基线实现。
#[allow(dead_code)]
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

/// 同 [`erase_chip`]，但按「已用秒/超时秒」发 progress 心跳 + 节流 log。
///
/// 整片擦除耗时约 1–2 分钟，期间无法按字节分段汇报进度；参照 NDJSON 契约
/// （`event.rs`：整片擦除心跳 done/total 为已用秒/超时秒）与 MBC 的
/// `erase_chip_logged`，每轮轮询后发一次心跳，让客户端进度条按时间线性推进，
/// 避免「擦除阶段死停 0%、进入写入后突然跳满」的前慢后快观感。
pub fn erase_chip_logged(
    link: &mut CartridgeLink,
    timeout_secs: u64,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> bool {
    let total = timeout_secs.max(1);
    if !link.rom_erase_chip() {
        return false;
    }
    log(&format!("整片擦除开始（超时 {total}s）..."));
    let start = Instant::now();
    let mut probe = [0u8; 2];
    let mut plog = ProgressLog::new(Phase::Erase);
    plog.report(0, total, progress, log);

    loop {
        if link.rom_read(0, &mut probe) && probe == [0xff, 0xff] {
            plog.report(total, total, progress, log);
            log(&format!("整片擦除完毕，耗时 {:.3}s", start.elapsed().as_secs_f64()));
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
        let elapsed = start.elapsed().as_secs().min(total.saturating_sub(1));
        plog.report(elapsed, total, progress, log);
        if start.elapsed().as_secs() > timeout_secs {
            plog.report(total, total, progress, log);
            log(&format!("整片擦除超时（{:.1}s）", start.elapsed().as_secs_f64()));
            return false;
        }
    }
}

/// 扇区擦除（byte_base 须扇区对齐）。失败仅 DTR/RTS 复位重试（不对关口 reconnect）。
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
        link.reset_mcu_buffer();
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
    let mut off = start;
    let mut done = 0u64;
    let mut plog = ProgressLog::new(Phase::Erase);
    log(&format!("扇区 {ss}B x {total}"));
    // 开局先报 0/total，避免首扇区耗时长时客户端一直停在裸「擦除」无分数。
    plog.report(0, total, progress, log);
    while off < to {
        if !erase_sector(link, off, 5) {
            log(&format!(
                "擦除失败 @0x{off:X} · {:.1}s",
                plog.elapsed_secs()
            ));
            return false;
        }
        done += 1;
        plog.report(done, total, progress, log);
        off = off.saturating_add(ss);
    }
    true
}

/// All PPB Erase：清除全部扇区的持久保护位（上半区写不进时多半因 PPB）。
///
/// 对齐 `mission_tools.cs` / `GbaFlasher.UnlockAllPpb`：先查 PPB Lock，再 All PPB Erase。
pub fn unlock_all_ppb(link: &mut CartridgeLink) {
    unlock_all_ppb_logged(link, &mut |_| {});
}

/// 同上，可把 Lock 状态等诊断写入 `log`。
pub fn unlock_all_ppb_logged(link: &mut CartridgeLink, log: &mut dyn FnMut(&str)) {
    // 退出任何命令集
    link.rom_write(0, &[0x90, 0x00]);
    link.rom_write(0, &[0x00, 0x00]);
    link.rom_write(0, &[0xf0, 0x00]);

    // Global Non-Volatile Sector Protection Freeze Command Set — 读 PPB Lock
    link.rom_write(0x555, &[0xaa, 0x00]);
    link.rom_write(0x2aa, &[0x55, 0x00]);
    link.rom_write(0x555, &[0x50, 0x00]);
    let mut lock = [0u8; 2];
    let _ = link.rom_read(0, &mut lock);
    link.rom_write(0, &[0x90, 0x00]);
    link.rom_write(0, &[0x00, 0x00]);
    link.rom_write(0, &[0xf0, 0x00]);
    let lock_u16 = u16::from_le_bytes(lock);
    log(&format!("PPB Lock Status: 0x{lock_u16:04X}"));
    if lock[0] != 1 {
        log("警告: PPB Lock 非 1，All PPB Erase 可能无效（扇区持久保护无法清除）");
    }

    // 读扇区 0 与 0x400000 的 PPB（1=未保护，0=保护）
    for &sa in &[0u32, 0x40_0000] {
        link.rom_write(0x555, &[0xaa, 0x00]);
        link.rom_write(0x2aa, &[0x55, 0x00]);
        link.rom_write(0x555, &[0xc0, 0x00]);
        let mut ppb = [0u8; 2];
        let _ = link.rom_read(sa, &mut ppb);
        link.rom_write(0, &[0x90, 0x00]);
        link.rom_write(0, &[0x00, 0x00]);
        link.rom_write(0, &[0xf0, 0x00]);
        log(&format!("PPB @0x{sa:08X}: 0x{:04X}", u16::from_le_bytes(ppb)));
    }

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

    // 再读 0x400000 PPB，确认是否清掉
    link.rom_write(0x555, &[0xaa, 0x00]);
    link.rom_write(0x2aa, &[0x55, 0x00]);
    link.rom_write(0x555, &[0xc0, 0x00]);
    let mut ppb2 = [0u8; 2];
    let _ = link.rom_read(0x40_0000, &mut ppb2);
    link.rom_write(0, &[0x90, 0x00]);
    link.rom_write(0, &[0x00, 0x00]);
    link.rom_write(0, &[0xf0, 0x00]);
    log(&format!(
        "PPB @0x00400000 after erase: 0x{:04X}",
        u16::from_le_bytes(ppb2)
    ));
}

// ============ profile 驱动的擦除（命中 profile 时走命令序列，否则用上面的硬编码）============

/// profile 命中时用 chip_erase 序列；否则回落 [`erase_chip_logged`] 时带进度心跳。
pub fn chip_erase_profile_logged(
    link: &mut CartridgeLink,
    p: &crate::profile::Profile,
    timeout_secs: u64,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> bool {
    let timeout = p.chip_erase_timeout.max(timeout_secs);
    if let Some(seq) = p.chip_erase() {
        // run_gba 含 wait_for；整片擦耗时长，失败用关口重连清总线，再试。
        for _ in 0..3 {
            if crate::profile::run_gba(link, &seq, 0) {
                return true;
            }
            let _ = link.reconnect();
        }
        false
    } else {
        erase_chip_logged(link, timeout, progress, log)
    }
}

/// profile 命中时用 sector_erase 序列擦 `byte_base` 扇区；否则回落 [`erase_sector`]。
pub fn sector_erase_profile(link: &mut CartridgeLink, p: &crate::profile::Profile, byte_base: u32, retries: u32) -> bool {
    if let Some(seq) = p.sector_erase() {
        for _ in 0..retries {
            if crate::profile::run_gba(link, &seq, byte_base) {
                return true;
            }
            link.reset_mcu_buffer();
        }
        false
    } else {
        erase_sector(link, byte_base, retries)
    }
}
