//! GBA · 存档芯片探测（`cfb save-probe`）。
//!
//! 把此前只存在于 Python 协议脚本里的四项手法收敛进 cfb（见工具链清单「待整合事项 1」）：
//!
//! 1. **接触健康**：同址多读一致性。悬空/没插到底的卡在总线上拾噪，同一地址两次读出
//!    不同值；历史上多次被误判成「坏卡」，实际重插即恢复。总线不稳时**禁止写探针**。
//! 2. **JEDEC 识别**：`0xAA@0x5555` / `0x55@0x2AAA` / `0x90@0x5555` → 读 0x0000/0x0001
//!    → `0xF0` 退出。读出值与进 ID 模式前不同即为 FLASH，查表得型号与容量。
//! 3. **SRAM/FRAM 判别**：认不出 ID 的芯片直写 16B 探针再读回；能把位从 0 置 1
//!    （写 0xFF 生效）说明是 RAM 而非 FLASH（FLASH 位只可清，置 1 必须先擦）。
//! 4. **bank 探测**：`0xF5@0x00800000` 切 bank 后写不同标记，验证高 bank 是独立区域
//!    还是低 bank 的镜像；bank 内再验 0x8000 是否折返到 0x0000（32KiB vs 64KiB）。
//!
//! ⚠️ 除 FLASH 路径外，探测**必然写卡**（JEDEC 命令字节会落进 SRAM 数据区）。
//! 所有被碰过的字节先备份、用完立即还原并**逐字节读回校验**；还原失败会显式报错，
//! 不静默放过。`allow_write=false` 时只跑第 1 步，不下任何写命令。
//!
//! 直写/bank 探针**只在确认不是 FLASH 之后**才做：对未擦除的 FLASH 直写会永久清位，
//! 不可还原。

use crate::cartridge_link::CartridgeLink;
use crate::rom::gba::data::{ProbeHealth, SaveProbe, SaveType};
use crate::rom::gba::ops::save;

/// JEDEC 命令地址（存档窗口内偏移）。
const CMD_5555: u32 = 0x5555;
const CMD_2AAA: u32 = 0x2AAA;
/// 探针窗口长度（字节）。
const PROBE_LEN: usize = 16;
/// bank 内 32KiB 折返检测点。
const HALF: u32 = 0x8000;

/// 已知 GBA 存档 FLASH 的 JEDEC ID（厂商, 器件, 型号, 容量）。
/// 取自 GBATEK「GBA Cart Backup Flash ROM」表；`C2:09` 已真机实测命中。
const FLASH_IDS: &[(u8, u8, &str, u64)] = &[
    (0xBF, 0xD4, "SST 39VF512", 64 * 1024),
    (0xC2, 0x1C, "Macronix MX29L512", 64 * 1024),
    (0x32, 0x1B, "Panasonic MN63F805MNP", 64 * 1024),
    (0x1F, 0x3D, "Atmel AT29LV512", 64 * 1024),
    (0xC2, 0x09, "Macronix MX29L010", 128 * 1024),
    (0x62, 0x13, "Sanyo LE26FV10N1TS", 128 * 1024),
];

/// 查 JEDEC 表。命中返回（型号, 容量）。
fn lookup_flash(mfr: u8, dev: u8) -> Option<(&'static str, u64)> {
    FLASH_IDS
        .iter()
        .find(|(m, d, _, _)| *m == mfr && *d == dev)
        .map(|(_, _, name, size)| (*name, *size))
}

/// 生成一段确定但不像真实存档的探针图样（避免与卡内数据撞车导致误判）。
fn pattern(seed: u8) -> [u8; PROBE_LEN] {
    let mut p = [0u8; PROBE_LEN];
    for (i, b) in p.iter_mut().enumerate() {
        *b = ((i as u8).wrapping_mul(0x6D).wrapping_add(seed)) ^ 0xA5;
    }
    p
}

/// 写一段再读回，返回是否逐字节一致。
fn write_readback(link: &mut CartridgeLink, addr: u32, data: &[u8]) -> bool {
    link.ram_write(addr, data);
    let mut got = vec![0u8; data.len()];
    link.ram_read(addr, &mut got) && got == data
}

/// 还原一段并读回校验。偶发 NACK 不该把用户存档留在被改写状态，故重试。
fn restore(link: &mut CartridgeLink, addr: u32, data: &[u8]) -> bool {
    for _ in 0..2 {
        if write_readback(link, addr, data) {
            return true;
        }
    }
    false
}

/// 探测 GBA 存档芯片。`allow_write=false` 只做只读健康检查。
pub fn probe(link: &mut CartridgeLink, allow_write: bool, log: &mut dyn FnMut(&str)) -> SaveProbe {
    let mut r = SaveProbe::default();

    // ---------- 1) 接触健康：同址三读一致性 ----------
    save::sram_switch_bank(link, 0);
    let mut first = [0u8; PROBE_LEN];
    if !link.ram_read(0x0000, &mut first) {
        r.health = ProbeHealth::NoResponse;
        r.notes.push(crate::i18n::t("probe.no_response"));
        return r;
    }
    let mut stable = true;
    for _ in 0..2 {
        let mut again = [0u8; PROBE_LEN];
        if !link.ram_read(0x0000, &mut again) || again != first {
            stable = false;
        }
    }
    if !stable {
        // 噪声总线上写探针可能把垃圾写进真存档，直接停在这里。
        r.health = ProbeHealth::Unstable;
        r.notes.push(crate::i18n::t("probe.unstable"));
        log(&crate::i18n::t("probe.unstable"));
        return r;
    }
    r.health = ProbeHealth::Ok;

    if !allow_write {
        r.notes.push(crate::i18n::t("probe.readonly_skip"));
        log(&crate::i18n::t("probe.readonly_skip"));
        return r;
    }

    // ---------- 2) JEDEC 识别（碰 2 个字节，用完还原）----------
    let mut keep_2aaa = [0u8; 1];
    let mut keep_5555 = [0u8; 1];
    let backed_up =
        link.ram_read(CMD_2AAA, &mut keep_2aaa) && link.ram_read(CMD_5555, &mut keep_5555);

    link.ram_write(CMD_5555, &[0xAA]);
    link.ram_write(CMD_2AAA, &[0x55]);
    link.ram_write(CMD_5555, &[0x90]);
    let mut ident = [0u8; 2];
    let got_ident = link.ram_read(0x0000, &mut ident);
    link.ram_write(CMD_5555, &[0xF0]); // 退出 ID 模式（FLASH 回读阵列）

    // 进 ID 模式后 0x0000/0x0001 变了才是 FLASH；没变说明读到的仍是存档数据。
    let is_flash = got_ident && ident != first[..2];

    if is_flash {
        r.jedec = Some((ident[0], ident[1]));
        r.save_type = Some(SaveType::Flash);
        match lookup_flash(ident[0], ident[1]) {
            Some((name, size)) => {
                r.chip = Some(name);
                r.size_bytes = size;
                r.bank_size = save::SRAM_BANK;
                r.banks = (size / save::SRAM_BANK as u64).max(1) as u32;
                r.notes.push(crate::i18n::tf(
                    "probe.flash_id",
                    &[("id", &r.jedec_hex()), ("chip", name), ("n", &size.to_string())],
                ));
            }
            None => {
                // ID 有效但不在表里：容量未知，让用户显式给 --len，不猜。
                r.notes
                    .push(crate::i18n::tf("probe.flash_id_unknown", &[("id", &r.jedec_hex())]));
            }
        }
        // FLASH 的 ID 序列不改内容，无需还原；bank 数按容量表得出（切 bank 走 0xB0 序列）。
        r.write_probed = true;
        r.writable = true;
        for n in &r.notes {
            log(n);
        }
        return r;
    }

    // ---------- 认不出 ID：RAM 家族（SRAM / FRAM）----------
    r.notes.push(crate::i18n::t("probe.no_id"));
    // JEDEC 命令字节落进了存档数据区，立刻还原。
    if backed_up {
        let a = restore(link, CMD_2AAA, &keep_2aaa);
        let b = restore(link, CMD_5555, &keep_5555);
        r.restored = a && b;
    } else {
        r.restored = false;
    }

    // ---------- 3) 可写性 + FLASH 排除（位能否 0→1）----------
    let mut keep_b0 = [0u8; PROBE_LEN];
    let mut keep_half = [0u8; PROBE_LEN];
    let have_b0 = link.ram_read(0x0000, &mut keep_b0);
    let have_half = link.ram_read(HALF, &mut keep_half);
    r.write_probed = true;

    let pat_a = pattern(0x31);
    r.writable = write_readback(link, 0x0000, &pat_a);
    if !r.writable {
        r.notes.push(crate::i18n::t("probe.not_writable"));
        if have_b0 {
            r.restored &= restore(link, 0x0000, &keep_b0);
        }
        for n in &r.notes {
            log(n);
        }
        return r;
    }
    // FLASH 位只可清；能把整段写成 0xFF 说明是真 RAM。
    let all_ff = [0xFFu8; PROBE_LEN];
    let bits_can_set = write_readback(link, 0x0000, &all_ff);
    r.save_type = Some(if bits_can_set { SaveType::Sram } else { SaveType::Flash });
    if !bits_can_set {
        r.notes.push(crate::i18n::t("probe.bits_only_clear"));
    }

    // ---------- 4) bank 内折返：0x8000 是否别名到 0x0000 ----------
    // 此刻 0x0000 是 0xFF*16。往 0x8000 写另一张图样，若 0x0000 跟着变 = 只有 32KiB。
    let pat_b = pattern(0x77);
    link.ram_write(HALF, &pat_b);
    let mut at_zero = [0u8; PROBE_LEN];
    let wrapped = link.ram_read(0x0000, &mut at_zero) && at_zero == pat_b;
    r.bank_size = if wrapped { HALF } else { save::SRAM_BANK };
    if wrapped {
        r.notes.push(crate::i18n::t("probe.wrap32"));
    }

    // ---------- 5) bank 独立性：高 bank 是独立区域还是镜像 ----------
    // 先把 0x0000 打成已知值，再切 bank1 写另一值，回 bank0 看是否被覆写。
    let marker0 = pattern(0x11);
    link.ram_write(0x0000, &marker0);
    save::sram_switch_bank(link, 1);
    let mut keep_b1 = [0u8; PROBE_LEN];
    let have_b1 = link.ram_read(0x0000, &mut keep_b1);
    let marker1 = pattern(0x22);
    let b1_writable = write_readback(link, 0x0000, &marker1);
    save::sram_switch_bank(link, 0);
    let mut back = [0u8; PROBE_LEN];
    let read_b0 = link.ram_read(0x0000, &mut back);

    if b1_writable && read_b0 && back == marker0 {
        // bank1 的写没打到 bank0 → 两个 bank 各自独立。
        r.banks = 2;
        r.mirrored = false;
    } else {
        r.banks = 1;
        r.mirrored = true;
        r.notes.push(crate::i18n::t("probe.mirrored"));
    }
    r.size_bytes = r.bank_size as u64 * r.banks as u64;

    // ---------- 还原：先高 bank 再低 bank ----------
    if r.banks == 2 && have_b1 {
        save::sram_switch_bank(link, 1);
        r.restored &= restore(link, 0x0000, &keep_b1);
    }
    save::sram_switch_bank(link, 0);
    if have_half && !wrapped {
        r.restored &= restore(link, HALF, &keep_half);
    }
    if have_b0 {
        r.restored &= restore(link, 0x0000, &keep_b0);
    }

    r.notes.push(crate::i18n::tf(
        "probe.banks",
        &[
            ("n", &r.banks.to_string()),
            ("size", &r.bank_size.to_string()),
            ("total", &r.size_bytes.to_string()),
        ],
    ));
    if matches!(r.save_type, Some(SaveType::Sram)) {
        r.notes.push(crate::i18n::t("probe.fram_hint"));
    }
    r.notes.push(crate::i18n::t(if r.restored {
        "probe.restore_ok"
    } else {
        "probe.restore_fail"
    }));
    for n in &r.notes {
        log(n);
    }
    r
}

/// 只读容量推断，供 `save-dump` 自动定尺寸用（**不写卡**）。
///
/// 读 bank0/bank1 同址内容比对：不同 → 高 bank 是独立区域（≥128KiB）；相同 → 无法区分
/// 「镜像」与「两个 bank 恰好内容一致」，返回 None 让调用方显式提示而不是静默截半。
pub fn readonly_bank_hint(link: &mut CartridgeLink, st: SaveType) -> Option<u32> {
    const HINT_LEN: usize = 256;
    let mut b0 = vec![0u8; HINT_LEN];
    let mut b1 = vec![0u8; HINT_LEN];
    switch(link, st, 0);
    if !link.ram_read(0x0000, &mut b0) {
        return None;
    }
    switch(link, st, 1);
    let got = link.ram_read(0x0000, &mut b1);
    switch(link, st, 0);
    if got && b0 != b1 {
        Some(2)
    } else {
        None
    }
}

fn switch(link: &mut CartridgeLink, st: SaveType, bank: u32) {
    if matches!(st, SaveType::Flash) {
        save::flash_switch_bank(link, bank);
    } else {
        save::sram_switch_bank(link, bank);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jedec_table_covers_measured_macronix_128k() {
        // 真机实测：C2:09 = MX29L010 = 128KiB（报告「卡带清单」第 2/3 张卡）。
        assert_eq!(lookup_flash(0xC2, 0x09), Some(("Macronix MX29L010", 131072)));
        assert_eq!(lookup_flash(0xC2, 0x1C), Some(("Macronix MX29L512", 65536)));
        assert_eq!(lookup_flash(0x00, 0x00), None);
        // 全 FF（无芯片/悬空）不得命中任何型号。
        assert_eq!(lookup_flash(0xFF, 0xFF), None);
    }

    #[test]
    fn probe_pattern_is_not_blank_and_varies_by_seed() {
        let a = pattern(0x31);
        let b = pattern(0x77);
        assert_ne!(a, b);
        // 探针图样不能是全 00/全 FF，否则与擦除态/空卡撞车无法判别。
        assert!(a.iter().any(|&x| x != 0x00 && x != 0xFF));
        assert!(a.iter().collect::<std::collections::HashSet<_>>().len() > 8);
    }
}
