//! MBC · 存档芯片探测（`cfb save-probe --mbc`）。
//!
//! GB/GBC 侧比 GBA 简单：卡带头 0x149 已声明存档容量，但**声明值不等于实物**
//! （改装卡、贴片换芯片、MBC 门控异常都会对不上），所以照样按 bank 逐个写标记验独立性。
//!
//! 手法（与 GBA 侧同源，走 GB 总线原语）：
//! - RAM 使能 `0x0A@0x0000` → bank 切换 `0x4000` → 窗口 `0xA000`，8KiB 一个 bank；
//! - 同址三读一致性判接触（不稳则**不写**）；
//! - 每个 bank 写入带 bank 序号的图样，写完再回读全部 bank：只要有 bank 读到别的
//!   bank 的图样即为镜像/别名，实际容量按最大独立 bank 数算；
//! - MBC2 片内 512×4bit，无 bank 概念，单独走固定路径。
//!
//! ⚠️ 探测必然写卡。所有碰过的字节先备份、用完还原并读回校验，还原失败显式报错。

use crate::cartridge_link::CartridgeLink;
use crate::rom::gba::data::{ProbeHealth, SaveProbe, SaveType};
use crate::rom::mbc::data::MbcKind;
use crate::rom::mbc::ops::save;

/// 探针窗口长度（字节）。
const PROBE_LEN: usize = 16;
/// bank 探测上限（MBC5 最多 16 个 8KiB bank = 128KiB）。
const MAX_BANKS: u32 = 16;

/// 生成带 bank 序号的探针图样（不同 bank 必然不同，便于识别别名到了哪个 bank）。
fn pattern(bank: u32) -> [u8; PROBE_LEN] {
    let mut p = [0u8; PROBE_LEN];
    for (i, b) in p.iter_mut().enumerate() {
        *b = ((i as u8).wrapping_mul(0x1D) ^ 0x5A).wrapping_add(bank as u8 * 0x11);
    }
    p
}

/// MBC2 只有低 4 位有效，比对前统一掩位。
fn mask(kind: MbcKind, data: &[u8]) -> Vec<u8> {
    if kind == MbcKind::Mbc2 {
        data.iter().map(|b| b & 0x0F).collect()
    } else {
        data.to_vec()
    }
}

fn write_readback(link: &mut CartridgeLink, kind: MbcKind, addr: u32, data: &[u8]) -> bool {
    link.gbc_write(addr, data);
    let mut got = vec![0u8; data.len()];
    link.gbc_read(addr, &mut got) && mask(kind, &got) == mask(kind, data)
}

fn restore(link: &mut CartridgeLink, kind: MbcKind, addr: u32, data: &[u8]) -> bool {
    for _ in 0..2 {
        if write_readback(link, kind, addr, data) {
            return true;
        }
    }
    false
}

/// 探测 MBC 存档。`declared` 为卡带头 0x149 声明的容量（字节，0=未知）。
pub fn probe(
    link: &mut CartridgeLink,
    kind: MbcKind,
    declared: u64,
    allow_write: bool,
    log: &mut dyn FnMut(&str),
) -> SaveProbe {
    let mut r = SaveProbe::default();
    r.bank_size = save::RAM_BANK as u32;

    save::ram_enable(link);
    save::switch_ram_bank(link, kind, 0);

    // ---------- 1) 接触健康：同址三读一致性 ----------
    let mut first = [0u8; PROBE_LEN];
    if !link.gbc_read(save::RAM_WINDOW, &mut first) {
        r.health = ProbeHealth::NoResponse;
        r.notes.push(crate::i18n::t("probe.no_response"));
        return r;
    }
    for _ in 0..2 {
        let mut again = [0u8; PROBE_LEN];
        if !link.gbc_read(save::RAM_WINDOW, &mut again) || again != first {
            r.health = ProbeHealth::Unstable;
            r.notes.push(crate::i18n::t("probe.unstable"));
            log(&crate::i18n::t("probe.unstable"));
            return r;
        }
    }
    r.health = ProbeHealth::Ok;
    if declared > 0 {
        r.notes.push(crate::i18n::tf("probe.declared", &[("n", &declared.to_string())]));
    }

    if !allow_write {
        // 只读模式下只能相信卡带头。
        r.size_bytes = declared;
        r.banks = if declared > 0 {
            (declared / save::RAM_BANK).max(1) as u32
        } else {
            0
        };
        r.notes.push(crate::i18n::t("probe.readonly_skip"));
        for n in &r.notes {
            log(n);
        }
        return r;
    }

    r.write_probed = true;

    // ---------- MBC2：片内 512×4bit，无 bank ----------
    if kind == MbcKind::Mbc2 {
        let mut keep = [0u8; PROBE_LEN];
        let have = link.gbc_read(save::RAM_WINDOW, &mut keep);
        r.writable = write_readback(link, kind, save::RAM_WINDOW, &pattern(0));
        if have {
            r.restored = restore(link, kind, save::RAM_WINDOW, &keep);
        }
        r.save_type = Some(SaveType::Sram);
        r.banks = 1;
        r.bank_size = 512;
        r.size_bytes = 512;
        r.notes.push(crate::i18n::t("probe.mbc2"));
        finish_notes(&mut r, log);
        return r;
    }

    // ---------- 2) 可写性 ----------
    let mut keep0 = [0u8; PROBE_LEN];
    let have0 = link.gbc_read(save::RAM_WINDOW, &mut keep0);
    r.writable = write_readback(link, kind, save::RAM_WINDOW, &pattern(0));
    if !r.writable {
        r.notes.push(crate::i18n::t("probe.not_writable"));
        if have0 {
            r.restored = restore(link, kind, save::RAM_WINDOW, &keep0);
        }
        r.size_bytes = declared;
        finish_notes(&mut r, log);
        return r;
    }
    r.save_type = Some(SaveType::Sram);

    // ---------- 3) bank 独立性：先全备份，再逐 bank 打标，最后统一回读 ----------
    // 声明容量给出探测上限；声明不可信（0 或超范围）时按 MBC5 上限扫。
    let limit = if declared > 0 {
        ((declared / save::RAM_BANK).max(1) as u32).min(MAX_BANKS)
    } else {
        MAX_BANKS
    };
    let mut backup: Vec<Option<[u8; PROBE_LEN]>> = Vec::with_capacity(limit as usize);
    for bank in 0..limit {
        save::switch_ram_bank(link, kind, bank);
        let mut keep = [0u8; PROBE_LEN];
        backup.push(if link.gbc_read(save::RAM_WINDOW, &mut keep) { Some(keep) } else { None });
    }
    for bank in 0..limit {
        save::switch_ram_bank(link, kind, bank);
        link.gbc_write(save::RAM_WINDOW, &pattern(bank));
    }
    // 打标全部写完后再统一回读：bank N 读到 bank M 的图样即为别名。
    let mut independent = 0u32;
    for bank in 0..limit {
        save::switch_ram_bank(link, kind, bank);
        let mut got = [0u8; PROBE_LEN];
        if link.gbc_read(save::RAM_WINDOW, &mut got) && mask(kind, &got) == mask(kind, &pattern(bank))
        {
            independent += 1;
        } else {
            break; // 第一个对不上的 bank 即容量边界
        }
    }
    r.banks = independent.max(1);
    r.mirrored = r.banks < limit;
    r.size_bytes = r.banks as u64 * save::RAM_BANK;
    if r.mirrored {
        r.notes.push(crate::i18n::t("probe.mirrored"));
    }
    if declared > 0 && r.size_bytes != declared {
        r.notes.push(crate::i18n::tf(
            "probe.declared_mismatch",
            &[("declared", &declared.to_string()), ("actual", &r.size_bytes.to_string())],
        ));
    }

    // ---------- 还原 ----------
    r.restored = true;
    for bank in 0..limit {
        if let Some(keep) = backup[bank as usize] {
            save::switch_ram_bank(link, kind, bank);
            r.restored &= restore(link, kind, save::RAM_WINDOW, &keep);
        }
    }
    save::switch_ram_bank(link, kind, 0);
    if have0 {
        r.restored &= restore(link, kind, save::RAM_WINDOW, &keep0);
    }

    r.notes.push(crate::i18n::tf(
        "probe.banks",
        &[
            ("n", &r.banks.to_string()),
            ("size", &r.bank_size.to_string()),
            ("total", &r.size_bytes.to_string()),
        ],
    ));
    r.notes.push(crate::i18n::t("probe.fram_hint"));
    finish_notes(&mut r, log);
    r
}

fn finish_notes(r: &mut SaveProbe, log: &mut dyn FnMut(&str)) {
    r.notes.push(crate::i18n::t(if r.restored {
        "probe.restore_ok"
    } else {
        "probe.restore_fail"
    }));
    for n in &r.notes {
        log(n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_patterns_are_unique_so_aliasing_is_identifiable() {
        // bank 探测靠「读到别的 bank 的图样」判别名，图样必须两两不同。
        let all: Vec<_> = (0..MAX_BANKS).map(pattern).collect();
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "bank {i} 与 bank {j} 图样撞车");
            }
        }
    }

    #[test]
    fn mbc2_masking_compares_low_nibble_only() {
        // MBC2 存档只有低 4 位有效，高位读回是 1，不能算不符。
        assert_eq!(mask(MbcKind::Mbc2, &[0xF5, 0x0A]), vec![0x05, 0x0A]);
        assert_eq!(mask(MbcKind::Mbc5, &[0xF5, 0x0A]), vec![0xF5, 0x0A]);
    }
}
