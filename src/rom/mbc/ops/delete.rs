//! MBC · 删：擦除 GB/GBC flash（整片 / 逐扇区）。
//!
//! 从参考源 `mission_mbc5.cs` 复刻（GB 总线 flash 命令序列，unlock 写 0xAAA/0x555）。

use std::time::{Duration, Instant};

use super::read::{bus_addr, flash_phys_addr, switch_bank, switch_bank_mbcx, BANK_SIZE};
use crate::cartridge_link::CartridgeLink;
use crate::progress_display::{Phase, ProgressLog};
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
        MbcKind::Mbc1 | MbcKind::Mbc2 | MbcKind::Mbc3 => {
            let bank = phys_sector >> 14;
            (bank, bus_addr(phys_sector, kind))
        }
    }
}

/// [from,to) 线性 ROM 区间覆盖到的 flash 物理扇区起点列表（去重升序）。
pub(crate) fn phys_sectors_covering(kind: MbcKind, from: u64, to: u64, sector_size: u32) -> Vec<u64> {
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
        // 防 ss 异常导致死循环
        let next = sec.saturating_add(ss);
        if next <= sec {
            break;
        }
        sec = next;
    }
    out
}

/// 抽查若干 bank 头 16B 是否全 0xFF（须先 0xF0 退出状态机）。
fn blank_check_banks(link: &mut CartridgeLink, banks: u32) -> bool {
    link.gbc_write(0x00, &[0xf0]);
    std::thread::sleep(Duration::from_millis(20));
    let mut buf = [0u8; 16];
    for b in 0..banks.max(1) {
        switch_bank(link, b, MbcKind::Mbc5);
        if !link.gbc_read(bus_addr(b << 14, MbcKind::Mbc5), &mut buf) {
            return false;
        }
        if !buf.iter().all(|&x| x == 0xff) {
            return false;
        }
    }
    switch_bank(link, 0, MbcKind::Mbc5);
    true
}

/// 整片擦除 + 心跳：对齐 C# `mission_eraseChip_mbc5` + FlashGBX profile（120s 超时）。
/// - 完成判据 = **16 字节多点探针连续 FF** + 多 bank 阵列空白确认（多点探针天然免疫
///   单字节假 FF——2026-08-14 曾因单字节探针被旧数据首字节 FF 骗出 0.2s 假完成）
/// - 最短等待 15s（物理下限）；硬超时 240s（FlashGBX 120s / 实测 54-181s；
///   旧版 CFI×3 可达 26min，纯浪费）
pub fn erase_chip_logged(
    link: &mut CartridgeLink,
    timeout_secs: u64,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> bool {
    let _ = super::read::rom_cal_erase_time_ms(link); // 保留 CFI 查询副作用（复位+查询时序）
    const MIN_ACCEPT_MS: u64 = 15_000;
    let hard_timeout_ms = timeout_secs
        .saturating_mul(1000)
        .clamp(180_000, 240_000);
    let hard_timeout = Duration::from_millis(hard_timeout_ms);

    // 进度分母:实测整片擦除约 54-181s，取 190s 基数让进度条匀速。
    const ESTIMATED_ERASE_SECS: u64 = 190;
    let progress_total = ESTIMATED_ERASE_SECS;

    log(&format!(
        "整片擦除开始（预估 ~{progress_total}s, 硬超时 {:.0}s）...",
        hard_timeout.as_secs_f64()
    ));

    link.gbc_write(0x00, &[0xf0]);
    switch_bank(link, 0, MbcKind::Mbc5);

    link.gbc_write(0xaaa, &[0xaa]);
    link.gbc_write(0x555, &[0x55]);
    link.gbc_write(0xaaa, &[0x80]);
    link.gbc_write(0xaaa, &[0xaa]);
    link.gbc_write(0x555, &[0x55]);
    link.gbc_write(0xaaa, &[0x10]); // Chip Erase

    let probe_addr = bus_addr(0, MbcKind::Mbc5); // 0x4000
    let start = Instant::now();
    let mut probe = [0u8; 16];
    let mut ff_streak = 0u32;
    let mut plog = ProgressLog::new(Phase::Erase);
    plog.report(0, progress_total, progress, log);

    loop {
        std::thread::sleep(Duration::from_millis(1000));
        let elapsed_ms = start.elapsed().as_millis() as u64;
        let elapsed_secs = start.elapsed().as_secs().min(progress_total.saturating_sub(1));
        plog.report(elapsed_secs, progress_total, progress, log);

        switch_bank(link, 0, MbcKind::Mbc5);
        if !link.gbc_read(probe_addr, &mut probe) {
            ff_streak = 0;
        } else if probe.iter().all(|&b| b == 0xff) {
            ff_streak += 1;
            // 须过物理下限，且连续若干次多点 FF，再退出状态机验阵列
            if elapsed_ms >= MIN_ACCEPT_MS && ff_streak >= 5 {
                std::thread::sleep(Duration::from_millis(500));
                link.gbc_write(0x00, &[0xf0]);
                std::thread::sleep(Duration::from_millis(100));
                if blank_check_banks(link, 16) {
                    plog.report(progress_total, progress_total, progress, log);
                    log(&format!(
                        "整片擦除完毕（阵列空白确认），耗时 {:.3} s",
                        start.elapsed().as_secs_f64()
                    ));
                    return true;
                }
                log("已过最短等待但阵列未空，继续轮询...");
                ff_streak = 0;
            }
        } else {
            ff_streak = 0;
        }

        if start.elapsed() >= hard_timeout {
            link.gbc_write(0x00, &[0xf0]);
            if blank_check_banks(link, 16) {
                plog.report(progress_total, progress_total, progress, log);
                log(&format!(
                    "整片擦除完毕（超时边界），耗时 {:.3} s",
                    start.elapsed().as_secs_f64()
                ));
                return true;
            }
            log(&format!(
                "整片擦除超时（{:.1}s）且阵列未空",
                start.elapsed().as_secs_f64()
            ));
            plog.report(progress_total, progress_total, progress, log);
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
    prof: Option<&crate::profile::Profile>,
) -> bool {
    let (bank, sa) = sector_erase_target(kind, phys_sector as u32);
    switch_bank_mbcx(link, bank, kind, flash_bank);

    link.gbc_write(0x00, &[0xf0]);

    for _try in 0..3 {
        let t0 = Instant::now();
        // 命令双源：规则库 profile 优先（cmds-only，完成判定仍走本函数探针+空白抽查）；
        // 未命中回落硬编码 AMD 序列（两者等价：AA/555/80/AA/555/30@SA）。
        let emitted = match prof.and_then(|p| p.sector_erase()) {
            Some(seq) => {
                let cmds_only =
                    crate::profile::SeqFull { cmds: seq.cmds.clone(), waits: Vec::new() };
                crate::profile::run_dmg(link, &cmds_only, sa)
            }
            None => {
                link.gbc_write(0xaaa, &[0xaa]);
                link.gbc_write(0x555, &[0x55]);
                link.gbc_write(0xaaa, &[0x80]);
                link.gbc_write(0xaaa, &[0xaa]);
                link.gbc_write(0x555, &[0x55]);
                link.gbc_write(sa, &[0x30]); // Sector Erase
                true
            }
        };
        if !emitted {
            continue;
        }

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
            MbcKind::Mbc1 | MbcKind::Mbc2 | MbcKind::Mbc3 => {
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
    prof: Option<&crate::profile::Profile>,
) -> bool {
    let sectors = phys_sectors_covering(kind, from, to, sector_size);
    let mut flash_bank: i32 = -1;
    for sec in sectors {
        if !erase_one_phys_sector(link, kind, sec, sector_size, &mut flash_bank, prof) {
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
    prof: Option<&crate::profile::Profile>,
    progress: &mut dyn FnMut(u64, u64),
    log: &mut dyn FnMut(&str),
) -> bool {
    let ss = sector_size.max(BANK_SIZE) as u64;
    let sectors = phys_sectors_covering(kind, from, to, sector_size);
    let total = sectors.len().max(1) as u64;
    let mut done = 0u64;
    let mut flash_bank: i32 = -1;
    let mut plog = ProgressLog::new(Phase::Erase);
    log(&format!("扇区 {ss}B x {total} (phys)"));
    if total <= 16 {
        let list: Vec<String> = sectors.iter().map(|s| format!("0x{s:X}")).collect();
        log(&format!("扇区列表: {}", list.join(", ")));
    }
    // 自高地址向低擦（部分 NOR 习惯）。
    plog.report(0, total, progress, log);
    for sec in sectors.into_iter().rev() {
        if !erase_one_phys_sector(link, kind, sec, sector_size, &mut flash_bank, prof) {
            log(&format!(
                "擦除失败 @phys=0x{sec:X} · {:.1}s",
                plog.elapsed_secs()
            ));
            return false;
        }
        done += 1;
        plog.report(done, total, progress, log);
    }
    true
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::mbc::data::MbcKind;

    #[test]
    fn mbc5_128k_rom_covers_three_64k_phys_sectors() {
        // 128KiB ROM、CFI 64KiB、MBC5 N+1：phys 0x4000..=0x23FFF → 对齐 0 / 0x10000 / 0x20000
        let secs = phys_sectors_covering(MbcKind::Mbc5, 0, 131072, 64 * 1024);
        assert_eq!(secs, vec![0, 0x10000, 0x20000], "got {:x?}", secs);
    }

    #[test]
    fn mbc3_128k_rom_covers_two_64k_sectors() {
        let secs = phys_sectors_covering(MbcKind::Mbc3, 0, 131072, 64 * 1024);
        assert_eq!(secs, vec![0, 0x10000], "got {:x?}", secs);
    }
}
