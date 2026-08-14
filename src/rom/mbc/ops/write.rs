//! MBC · 写：编程 / 烧录 GB/GBC ROM（MBC3 / MBC5 自动识别）。
//!
//! 流程：软件插拔 3.3V → CFI/ID → 整片擦 + ROM 范围扇区擦（16KB）→
//! 再软件插拔 → 逐 bank 0xfc 编程 → 读回校验。
//! MBC5 bank：N→N+1 @0x2000（见 `switch_bank`）。

use std::time::Instant;

use super::delete::erase_range_logged;
use super::read::{
    bus_addr, effective_erase_sector, is_js28f256, rom_get_cfi, rom_get_id, switch_bank,
    switch_window,
};
use crate::cartridge_link::CartridgeLink;
use crate::progress_display::{Phase, ProgressLog};
use crate::rom::gba::data::BurnResult;
use crate::rom::mbc::data::{mbc_name, MbcKind};

const PACKET: usize = 256;

/// 烧录 GB/GBC ROM：识别 → 查 flash → 空间校验 → 擦除 → 写入 → 校验。
/// 调用前 `open_powered` 默认 3.3V；此处 `soft_unplug_3v3` 保持 3.3V 时序做软件插拔。
/// 命令结束后由调用方 `power_idle` 确认 3.3V。
///
/// `chip_erase`：MBC 默认已走「整片 + 扇区」；该标志保留与 GBA/CLI 对齐（true 时强制整片优先）。
pub fn burn(
    link: &mut CartridgeLink,
    rom: &[u8],
    verify: bool,
    chip_erase: bool,
    no_erase: bool,
    kind_override: Option<MbcKind>,
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

    // 每次 mission：关口再开 + 3.3V 断电/上电（软件等效插拔，避免连续操作残留）
    if let Err(e) = link.soft_unplug_3v3() {
        log(&format!("软件复位失败: {e}"));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    // ---- 步骤 1：总线恒 MBC5（ChisFlash 硬件接线）----
    // ⚠ 禁止按卡头/ROM 头切 MBC3 等总线（skill 规则1）：烧完 MBC3 型 ROM（如 gb_check 0x10）
    // 后卡头即变 0x10，按头选型会切去 MBC3 → bank0 映射 0x0000 窗 → 固件 NAK，
    // 表现为「多次烧录后无法烧录」（2026-08-14 复现：MBC5 ROM 烧完正常、MBC3 ROM 烧完必挂）。
    let file_ct = rom.get(0x147).copied().unwrap_or(0xFF);
    let live_ct = super::read::read_cart_byte(link, 0x147).unwrap_or(0xFF);
    // 总线默认恒 MBC5（ChisFlash 接线；卡头是内容不是硬件）；--mbc-kind 手动兜底
    let kind = kind_override.unwrap_or(MbcKind::Mbc5);
    log(&format!(
        "识别: ROM=0x{:02X} {} / cart=0x{:02X} -> 总线 {}（默认MBC5，--mbc-kind 可覆盖）",
        file_ct,
        mbc_name(file_ct),
        live_ct,
        kind.label()
    ));

    // ---- 步骤 2：CFI + Autoselect ID ----
    let (device_size, buf_wr_cfi, cfi_sector) = rom_get_cfi(link);
    let id = rom_get_id(link);
    // 规则库（chis-burner-rule）：按 Autoselect ID 匹配卡型（GB 4B→补 0 成 8B）。
    // 命中后：扇区步进取自 profile（显式 sector_size 优先），擦除命令走 profile 序列。
    let prof = {
        let all = crate::profile::load_all();
        let id8 = [id[0], id[1], id[2], id[3], 0, 0, 0, 0];
        crate::profile::match_by_id(&all, &id8).map(|p| p.clone())
    };
    if let Some(p) = &prof {
        log(&format!("Profile: {}", p.name));
    }
    let mut buf_wr: u16 = if buf_wr_cfi == 0 { 32 } else { buf_wr_cfi };
    if is_js28f256(&id) {
        buf_wr = 256;
        log("JS28F256：buf_wr=256");
    }
    // 扇区步进：CFI 可信范围 4KB–256KB；S29GL-S/clone（01 7E 22xx）强制 128KB。
    // 2026-08-14 probe 实证：该族单次 0x30 恰清 8 个 16KB bank（=128KB）。
    // 旧版一律 16KB 步进 → 128KB 芯片上目标数 ×8，每目标 ~0.46s 空白跳查开销纯浪费。
    let sector_size = prof
        .as_ref()
        .and_then(crate::profile::uniform_sector_size)
        .unwrap_or_else(|| effective_erase_sector(&id, cfi_sector));
    log(&format!(
        "flash id={:02X}{:02X}{:02X}{:02X} size={} buf={} sector={} (cfi_sec={})",
        id[0],
        id[1],
        id[2],
        id[3],
        if device_size == 0 {
            "?".to_string()
        } else {
            device_size.to_string()
        },
        buf_wr,
        sector_size,
        cfi_sector
    ));

    // ---- 步骤 3：空间校验 ----
    if device_size > 0 && length > device_size {
        log(&format!("空间不足: ROM {} > flash {}", length, device_size));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }

    link.gbc_write(0x00, &[0xf0]);
    link.gbc_warm_up();
    switch_bank(link, 0, kind);

    // ---- 步骤 4：擦除（默认=C# 语义：空白跳过 + 仅 ROM 范围扇区擦）----
    // 对齐 C# mission_programRom_mbc5 的 isBlank → mbc5_romEraseSector(addrBegin,addrEnd)
    // 与 FlashGBX prefer_chip_erase=false。`--chip-erase` 才整片+补擦（慢路径，按需选用）。
    // `--no-erase` 跳过整个擦除段（仅用于测纯写入吞吐，要求 flash 已是擦除态）。
    if no_erase {
        log("跳过擦除，直接写入（--no-erase，flash 须已为擦除态）");
    } else {
    let banks = ((length + 0x3fff) / 0x4000) as u32;
    if !chip_erase && rom_range_blank(link, kind, banks) && boot_window_blank(link) {
        // 空白卡快路径（含开机窗检查）：无擦除发生 → 免插拔/重识别，直接进写入
        log("ROM 范围已空白，跳过擦除（对齐 C# isBlank）");
    } else {
    if chip_erase {
        log("擦除：整片 + ROM 范围扇区补擦 ...");
        // 整片失败不硬退：仍依赖后续扇区擦
        if !super::delete::erase_chip_logged(link, 180, progress, log) {
            log("整片擦失败或未净；将仅依赖后续扇区擦");
        }
    } else {
        log("擦除：ROM 范围扇区擦（C#/FlashGBX 默认）...");
    }
    if !erase_range_logged(link, kind, 0, length, sector_size, prof.as_ref(), progress, log) {
        log(&format!("扇区擦失败 | {}", elapsed(&start)));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }
    link.gbc_write(0x00, &[0xf0]);
    switch_bank(link, 0, kind);
    // 开机窗（隐藏区）条件擦除：只在读到脏时发 0x30@0x0000。
    // 安全性：若隐藏区属芯片扇区 0，扇区擦后必为空白→跳过（不会同块二次 0x30）；
    // 读到脏 ⇒ 它是独立扇区 ⇒ 单擦合法。每区域最多一次 0x30。
    if !boot_window_blank(link) {
        log("开机窗（隐藏区）非空白，单独擦除 ...");
        if !erase_boot_window(link, log) {
            log("开机窗擦除失败");
            res.first_bad = Some(0);
            res.seconds = start.elapsed().as_secs_f64();
            return res;
        }
        log("开机窗擦除完成");
    }
    // 擦后空白：警告可继续，不硬拦编程
    let mut probe = [0u8; 16];
    let mut dirty = false;
    for b in 0..banks.min(32) {
        switch_bank(link, b, kind);
        if !(link.gbc_read(bus_addr(b << 14, kind), &mut probe)
            && probe.iter().all(|&x| x == 0xff))
        {
            dirty = true;
            log(&format!("警告：擦后 bank{b} 非空，仍继续编程"));
            break;
        }
    }
    switch_bank(link, 0, kind);
    if !dirty {
        log("擦后空白抽查通过");
    }

    // 擦除后再软件插拔：对齐「擦除 mission 结束关口 → 烧录 mission 重开」；
    // 同会话硬扛易出现编程 NAK（物理插拔可好）。
    log("擦后软件插拔（关电→关串口→3.3V）...");
    if let Err(e) = link.soft_unplug_3v3() {
        log(&format!("擦后软件复位失败: {e}"));
        res.first_bad = Some(0);
        res.seconds = start.elapsed().as_secs_f64();
        return res;
    }
    let (_ds, buf_wr_re, _sec) = rom_get_cfi(link);
    let id2 = rom_get_id(link);
    log(&format!(
        "擦后重识别 id={:02X}{:02X}{:02X}{:02X} buf={}",
        id2[0], id2[1], id2[2], id2[3], buf_wr_re
    ));
    if buf_wr_re != 0 {
        buf_wr = buf_wr_re;
    }
    if is_js28f256(&id2) {
        buf_wr = 256;
    }
    switch_bank(link, 0, kind);
    std::thread::sleep(std::time::Duration::from_millis(50));
    } // end else（实际擦除路径：擦后插拔/重识别仅在有擦除时需要）
    } // end !no_erase（空白快路径免插拔直接进入写入）

    // ---- 步骤 5：写入（两段式，对齐真机布局）----
    // 段 A（开机窗）：ROM bank0 → 芯片 0x0-0x3FFF 隐藏区，经总线 0x0000 读写。
    // 段 B（主区）：ROM bank1+ → 线性偏移整体下移 0x4000（线性 0 = console bank1 = reg1）。
    // 只走单段 0x4000 窗时：校验通过但真机右移一 bank、开机窗全 FF → 白屏。
    let boot_len = length.min(0x4000);
    let main_len = length - boot_len;
    let main_rom = &rom[boot_len as usize..];
    log(&format!(
        "开始写入 ... buf_wr={buf_wr}（bank0→开机窗 {boot_len}B + 主区 {main_len}B）"
    ));
    {
        let mut write_plog = ProgressLog::new(Phase::Write);
        let mut write_progress = |d: u64, t: u64| {
            progress(d, t);
            if write_plog.should_log(d, t) {
                log(&write_plog.format(d, t));
            }
        };
        let phases: [(&[u8], bool); 2] = [(&rom[..boot_len as usize], true), (main_rom, false)];
        for &(data, is_boot) in phases.iter() {
            if data.is_empty() {
                continue;
            }
            let fail = if is_boot {
                // 段 A（开机窗）：0xFC 被固件拒，走 AMD 单字编程
                program_boot_window(link, data, &mut res, &mut write_progress)
            } else {
                program_flow(
                    link,
                    kind,
                    data,
                    0,
                    data.len() as u64,
                    buf_wr,
                    &mut res,
                    data.len() as u64,
                    &mut write_progress,
                )
            };
            if let Some(bad) = fail {
                log(&format!("写入失败 @0x{bad:X} | {}", elapsed(&start)));
                res.first_bad = Some(bad);
                res.seconds = start.elapsed().as_secs_f64();
                return res;
            }
        }
    }
    link.gbc_write(0x4000, &[0x00]);

    // ---- 步骤 6：读回校验（两段式，与写入同布局）；残留 FF 可补写第二遍（无需再擦）----
    if verify {
        log("校验中 ...");
        // 两段校验：boot 段（0x0000 窗读隐藏区）+ 主区段（0x4000 线性窗）；返回总 mismatch
        let verify_all = |link: &mut CartridgeLink,
                          res: &mut BurnResult,
                          progress: &mut dyn FnMut(u64, u64)|
         -> (u32, Option<String>) {
            let mut first_msg: Option<String> = None;
            let mut mm_total = 0u32;
            let phases: [(&[u8], bool); 2] =
                [(&rom[..boot_len as usize], true), (main_rom, false)];
            for &(data, is_boot) in phases.iter() {
                if data.is_empty() {
                    continue;
                }
                let mut verify_progress = |d: u64, t: u64| progress(d, t);
                let (mm, msg) = if is_boot {
                    verify_boot(link, data, &mut verify_progress, res)
                } else {
                    verify_flow(
                        link,
                        kind,
                        data,
                        data.len() as u64,
                        &mut verify_progress,
                        res,
                    )
                };
                mm_total += mm;
                if first_msg.is_none() {
                    first_msg = msg;
                }
            }
            (mm_total, first_msg)
        };
        let mm = {
            let mut verify_progress = |d: u64, t: u64| progress(d, t);
            let (mm, first_msg) = verify_all(link, &mut res, &mut verify_progress);
            if let Some(msg) = first_msg {
                log(&msg);
            }
            mm
        };
        res.mismatch_bytes = mm;
        if mm > 0 {
            log(&format!("校验: {mm} 字节不符，补写 ..."));
            res.first_bad = None;
            res.mismatch_bytes = 0;
            link.gbc_write(0x00, &[0xf0]);
            switch_bank(link, 0, kind);
            let rewrite_fail = {
                let mut rewrite_plog = ProgressLog::new(Phase::Write);
                let mut rewrite_progress = |d: u64, t: u64| {
                    progress(d, t);
                    if rewrite_plog.should_log(d, t) {
                        log(&rewrite_plog.format(d, t));
                    }
                };
                let phases: [(&[u8], bool); 2] =
                    [(&rom[..boot_len as usize], true), (main_rom, false)];
                let mut fail = None;
                for &(data, is_boot) in phases.iter() {
                    if data.is_empty() {
                        continue;
                    }
                    let f = if is_boot {
                        program_boot_window(link, data, &mut res, &mut rewrite_progress)
                    } else {
                        program_flow(
                            link,
                            kind,
                            data,
                            0,
                            data.len() as u64,
                            buf_wr,
                            &mut res,
                            data.len() as u64,
                            &mut rewrite_progress,
                        )
                    };
                    if f.is_some() {
                        fail = f;
                        break;
                    }
                }
                fail
            };
            if let Some(bad) = rewrite_fail {
                log(&format!("补写失败 @0x{bad:X}"));
                res.first_bad = Some(bad);
            } else {
                let mm2 = {
                    let mut verify_progress = |d: u64, t: u64| progress(d, t);
                    let (mm2, first_msg) = verify_all(link, &mut res, &mut verify_progress);
                    if let Some(msg) = first_msg {
                        log(&msg);
                    }
                    mm2
                };
                res.mismatch_bytes = mm2;
                log(&format!("补写后校验: {mm2} 字节不符 | {}", elapsed(&start)));
            }
        } else {
            log(&format!("校验: 0 字节不符 | {}", elapsed(&start)));
        }
    }

    res.success = res.first_bad.is_none() && res.mismatch_bytes == 0;
    res.seconds = start.elapsed().as_secs_f64();
    res
}

/// 编程区间；跨 bank 前 flash 复位。重连用 3.3V + gbc_warm_up。
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
    let mut written = 0u64;
    let mut current_bank: i64 = -1;
    while written < total {
        let len = ((total - written) as usize).min(PACKET);
        let pk = &data[written as usize..written as usize + len];
        let rom_off = (rom_base + written) as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            if current_bank >= 0 {
                link.gbc_write(0x00, &[0xf0]);
            }
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
                let _ = link.reconnect_as(true);
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

/// ROM 范围空白抽查（C# isBlank 的稳健版）：头 32 bank + 尾 4 bank，每 bank 抽头/中/尾三点。
/// 任一点非 FF 即视为非空白。C# 只查目标区头 512B，此处更严且仍在 1s 内。
fn rom_range_blank(link: &mut CartridgeLink, kind: MbcKind, banks: u32) -> bool {
    let mut probe = [0u8; 16];
    let head = banks.min(32);
    let tail_start = banks.saturating_sub(4).max(head);
    let mut check_bank = |link: &mut CartridgeLink, b: u32| -> bool {
        switch_bank(link, b, kind);
        let base = bus_addr(b << 14, kind);
        for off in [0u32, 0x2000, 0x3ff0] {
            if !(link.gbc_read(base + off, &mut probe) && probe.iter().all(|&x| x == 0xff)) {
                return false;
            }
        }
        true
    };
    for b in 0..head {
        if !check_bank(link, b) {
            return false;
        }
    }
    for b in tail_start..banks {
        if !check_bank(link, b) {
            return false;
        }
    }
    true
}

/// 开机窗（总线 0x0000-0x3FFF = 芯片隐藏区）编程。
/// 固件 0xFC 缓冲编程对该窗 NAK（2026-08-14 实测 @bus=0x0），改走
/// AMD 单字编程：每字节 `AA@0xAAA / 55@0x555 / A0@0xAAA / data@addr`
/// （unlock 原始写经 0x0000 窗可达 flash——擦除序列即证）。
/// 0xFF 跳过（已擦除态写 FF 是空操作）；校验由 verify_flow 兜底，失败字节可重打。
fn program_boot_window(
    link: &mut CartridgeLink,
    data: &[u8],
    res: &mut BurnResult,
    progress: &mut dyn FnMut(u64, u64),
) -> Option<u64> {
    let total = data.len() as u64;
    let mut done = 0u64;
    for (i, &b) in data.iter().enumerate() {
        if b != 0xff {
            link.gbc_write(0xaaa, &[0xaa]);
            link.gbc_write(0x555, &[0x55]);
            link.gbc_write(0xaaa, &[0xa0]);
            link.gbc_write(i as u32, &[b]);
            res.bytes_written += 1;
        }
        done += 1;
        if done % 512 == 0 {
            progress(done, total);
        }
    }
    progress(total, total);
    None
}

/// 开机窗（隐藏区，总线 0x0000-0x3FFF）是否全空白。全量读 16KB（~0.1s）。
fn boot_window_blank(link: &mut CartridgeLink) -> bool {
    let mut buf = vec![0u8; 512];
    let mut off = 0usize;
    while off < 0x4000 {
        switch_window(link, 0, MbcKind::Mbc5);
        if !link.gbc_read(0x0000 + off as u32, &mut buf) {
            return false;
        }
        if !buf.iter().all(|&x| x == 0xff) {
            return false;
        }
        off += buf.len();
    }
    true
}

/// 单独擦除开机窗（隐藏区）：unlock + 0x30@0x0000，多点 FF 判完成（20s 超时，软复位重试×1）。
/// 只在 boot_window_blank()==false 时调用（避免与扇区 0 擦除构成同块二次 0x30）。
fn erase_boot_window(link: &mut CartridgeLink, log: &mut dyn FnMut(&str)) -> bool {
    for attempt in 0..2u32 {
        if attempt > 0 {
            let _ = link.soft_unplug_3v3();
            link.gbc_write(0x00, &[0xf0]);
        }
        switch_window(link, 0, MbcKind::Mbc5);
        link.gbc_write(0x00, &[0xf0]);
        link.gbc_write(0xaaa, &[0xaa]);
        link.gbc_write(0x555, &[0x55]);
        link.gbc_write(0xaaa, &[0x80]);
        link.gbc_write(0xaaa, &[0xaa]);
        link.gbc_write(0x555, &[0x55]);
        link.gbc_write(0x0000, &[0x30]);
        let t0 = std::time::Instant::now();
        let mut probe = [0u8; 16];
        let mut ff_streak = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if t0.elapsed().as_millis() < 300 {
                continue;
            }
            switch_window(link, 0, MbcKind::Mbc5);
            if link.gbc_read(0x0000, &mut probe) && probe.iter().all(|&b| b == 0xff) {
                ff_streak += 1;
                if ff_streak >= 3 {
                    link.gbc_write(0x00, &[0xf0]);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    return boot_window_blank(link);
                }
            } else {
                ff_streak = 0;
            }
            if t0.elapsed().as_secs() > 20 {
                link.gbc_write(0x00, &[0xf0]);
                break;
            }
        }
    }
    log("开机窗擦除两轮超时");
    false
}

/// 开机窗校验：经总线 0x0000（reg0 窗口条件）读隐藏区比对。
fn verify_boot(
    link: &mut CartridgeLink,
    data: &[u8],
    progress: &mut dyn FnMut(u64, u64),
    res: &mut BurnResult,
) -> (u32, Option<String>) {
    let mut mismatch = 0u32;
    let mut first_msg = None;
    let mut buf = vec![0u8; 256];
    let total = data.len() as u64;
    let mut read = 0u64;
    while read < total {
        let n = ((total - read) as usize).min(buf.len());
        switch_window(link, 0, MbcKind::Mbc5);
        if !link.gbc_read(0x0000 + read as u32, &mut buf[..n]) {
            res.reconnects += 1;
            let _ = link.reconnect_as(true);
            continue;
        }
        for i in 0..n {
            if buf[i] != data[read as usize + i] {
                mismatch += 1;
                if first_msg.is_none() {
                    first_msg = Some(format!(
                        "开机窗 0x{:04X} 校验失败: {:02X} → {:02X}",
                        read as usize + i,
                        data[read as usize + i],
                        buf[i]
                    ));
                }
            }
        }
        read += n as u64;
        progress(read, total);
    }
    (mismatch, first_msg)
}

/// 开机窗定点补写：只重打校验不符的字节（单字编程可字节寻址）。
fn repair_boot_window(
    link: &mut CartridgeLink,
    data: &[u8],
    progress: &mut dyn FnMut(u64, u64),
) -> usize {
    let mut buf = vec![0u8; 256];
    let mut fixed = 0usize;
    let mut off = 0usize;
    while off < data.len() {
        let n = (data.len() - off).min(buf.len());
        switch_window(link, 0, MbcKind::Mbc5);
        if link.gbc_read(0x0000 + off as u32, &mut buf[..n]) {
            for i in 0..n {
                if buf[i] != data[off + i] {
                    let b = data[off + i];
                    if b != 0xff {
                        link.gbc_write(0xaaa, &[0xaa]);
                        link.gbc_write(0x555, &[0x55]);
                        link.gbc_write(0xaaa, &[0xa0]);
                        link.gbc_write((off + i) as u32, &[b]);
                        fixed += 1;
                    }
                }
            }
        }
        off += n;
        progress(off as u64, data.len() as u64);
    }
    fixed
}

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
    let align = (buf_wr as usize).max(1);
    let mut prefer_chunk = PACKET - (PACKET % align);
    let mut prefer_buf = buf_wr;
    while written < to {
        let align_off = (written as usize) % align;
        let mut chunk = if align_off != 0 {
            (align - align_off).min((to - written) as usize)
        } else {
            ((to - written) as usize).min(prefer_chunk)
        };
        if chunk == 0 {
            chunk = 1;
        }
        let rom_off = written as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            if current_bank >= 0 {
                link.gbc_write(0x00, &[0xf0]);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            current_bank = bank;
            prefer_chunk = PACKET - (PACKET % align);
            prefer_buf = buf_wr;
            if align_off == 0 {
                chunk = ((to - written) as usize).min(prefer_chunk);
            }
            switch_bank(link, bank as u32, kind);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let mut tries = 0;
        let mut use_buf = if chunk == 1 { 0 } else { prefer_buf };
        loop {
            let pk = &rom[written as usize..written as usize + chunk];
            let cartridge_addr = bus_addr(rom_off, kind);
            if link.gbc_rom_program(cartridge_addr, pk, use_buf) {
                if chunk >= 32 {
                    prefer_chunk = chunk.min(PACKET);
                }
                prefer_buf = if chunk == 1 { 0 } else { use_buf };
                break;
            }
            tries += 1;
            if tries == 1 {
                eprintln!(
                    "cfb: program nak @rom=0x{rom_off:X} bus=0x{cartridge_addr:X} bank={bank} chunk={chunk} buf={use_buf}"
                );
            }
            link.gbc_write(0x00, &[0xf0]);
            std::thread::sleep(std::time::Duration::from_millis(2));
            switch_bank(link, bank as u32, kind);
            if chunk > align {
                chunk = align;
                use_buf = buf_wr;
            } else if chunk > 1 {
                chunk = 1;
                use_buf = 0;
            }
            prefer_chunk = chunk;
            if tries % 12 == 0 {
                res.reconnects += 1;
                let _ = link.reconnect_as(true);
                switch_bank(link, bank as u32, kind);
            }
            if tries >= 25 {
                return Some(written);
            }
        }

        written += chunk as u64;
        res.bytes_written += chunk as u64;
        progress(written, total);
    }
    None
}

fn verify_flow(
    link: &mut CartridgeLink,
    kind: MbcKind,
    rom: &[u8],
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
    res: &mut BurnResult,
) -> (u32, Option<String>) {
    let mut mismatch = 0u32;
    let mut read = 0u64;
    let mut current_bank: i64 = -1;
    let mut buf = vec![0u8; PACKET];
    let mut first_msg = None;
    while read < total {
        let n = ((total - read) as usize).min(PACKET);
        let rom_off = read as u32;
        let bank = (rom_off >> 14) as i64;
        if bank != current_bank {
            if current_bank >= 0 {
                link.gbc_write(0x00, &[0xf0]);
            }
            current_bank = bank;
            switch_bank(link, bank as u32, kind);
        }
        let cartridge_addr = bus_addr(rom_off, kind);
        let b = &mut buf[..n];
        if !link.gbc_read(cartridge_addr, b) {
            res.reconnects += 1;
            let _ = link.reconnect_as(true);
            switch_bank(link, bank as u32, kind);
            continue;
        }
        for i in 0..n {
            if b[i] != rom[read as usize + i] {
                mismatch += 1;
                if res.first_bad.is_none() {
                    res.first_bad = Some(read + i as u64);
                    first_msg = Some(format!(
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
    (mismatch, first_msg)
}
