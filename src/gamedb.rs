//! GBA/GB 游戏名数据库 + GB 免电存档布局库（源自 flashGBX）。
//!
//! - `db_AGB.json` / `db_DMG.json`：header SHA1 → 游戏名 / game code
//! - `db_DMG_bl.json`：ROM 标题 → 免电存档 `[offset, size, layout?]`

use sha1::{Digest, Sha1};

include!(concat!(env!("OUT_DIR"), "/gamedb_gen.rs"));

/// 一条游戏库条目（GBA / DMG 共用字段子集）。
#[derive(serde::Deserialize, Clone, Debug)]
pub struct GameDbEntry {
    pub gn: String,
    #[serde(default)]
    pub gc: Option<String>,
}

/// flashGBX `db_DMG_bl.json`：免电存档在 ROM flash 中的布局。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatterylessConfig {
    /// 存档区在 ROM 中的起始偏移。
    pub offset: u64,
    /// 逻辑存档字节数（layout 1/2 时 ROM 占用为 `size * 2`）。
    pub size: u64,
    /// 0=连续；1=每 16KiB bank 取前半；2=取后半（flashGBX layout_names）。
    pub layout: u8,
}

impl BatterylessConfig {
    /// 实际需要读写的 ROM 跨度（layout 1/2 时翻倍）。
    pub fn rom_span(&self) -> u64 {
        if matches!(self.layout, 1 | 2) {
            self.size << 1
        } else {
            self.size
        }
    }
}

/// GBA：MultiBoot 用 0x100，否则 0x180。
pub fn header_sha1_agb(rom: &[u8]) -> String {
    let is_multiboot = rom.get(0xAC).copied() == Some(b'M');
    let end = if is_multiboot { 0x100 } else { 0x180 };
    sha1_range(rom, end)
}

/// GB/GBC：flashGBX 恒取 `SHA1(rom[0..0x180])`。
pub fn header_sha1_dmg(rom: &[u8]) -> String {
    sha1_range(rom, 0x180)
}

fn sha1_range(rom: &[u8], end: usize) -> String {
    let slice = &rom[..rom.len().min(end)];
    let mut hasher = Sha1::new();
    hasher.update(slice);
    format!("{:x}", hasher.finalize())
}

#[allow(dead_code)]
pub fn header_sha1(rom: &[u8]) -> String {
    header_sha1_agb(rom)
}

pub fn lookup_agb(rom: &[u8]) -> Option<GameDbEntry> {
    let sha = header_sha1_agb(rom);
    lookup_file("db_AGB.json", &sha).or_else(|| query_game(GAMEDB_AGB_SRC, &sha))
}

pub fn lookup_dmg(rom: &[u8]) -> Option<GameDbEntry> {
    let sha = header_sha1_dmg(rom);
    lookup_file("db_DMG.json", &sha).or_else(|| query_game(GAMEDB_DMG_SRC, &sha))
}

#[allow(dead_code)]
pub fn lookup_game_name(rom: &[u8]) -> Option<String> {
    lookup_agb(rom).map(|e| e.gn)
}

/// 按 flashGBX 规则从 GB 头生成标题候选，查 `db_DMG_bl`。
pub fn lookup_dmg_batteryless(rom: &[u8]) -> Option<BatterylessConfig> {
    for title in dmg_title_candidates(rom) {
        if let Some(cfg) = lookup_bl_title(&title) {
            return Some(cfg);
        }
    }
    None
}

fn dmg_title_candidates(rom: &[u8]) -> Vec<String> {
    if rom.len() < 0x144 {
        return Vec::new();
    }
    let cgb = rom[0x143];
    let end = if matches!(cgb, 0x80 | 0xC0) { 0x143 } else { 0x144 };
    let raw_bytes = &rom[0x134..end.min(rom.len())];
    let raw: String = raw_bytes
        .iter()
        .map(|&b| if (0x20..0x7f).contains(&b) || b == 0 { b as char } else { '\u{FFFD}' })
        .collect();
    let stripped = raw.replace('\0', "").trim_end().to_string();
    let rstrip0: String = raw_bytes
        .iter()
        .cloned()
        .rev()
        .skip_while(|&b| b == 0)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|b| b as char)
        .collect();
    let mut out = Vec::new();
    for t in [raw, rstrip0, stripped] {
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn lookup_bl_title(title: &str) -> Option<BatterylessConfig> {
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let path = std::path::PathBuf::from(home).join(".cfb").join("db_DMG_bl.json");
        if let Ok(src) = std::fs::read_to_string(path) {
            if let Some(cfg) = query_bl(&src, title) {
                return Some(cfg);
            }
        }
    }
    query_bl(GAMEDB_DMG_BL_SRC, title)
}

fn lookup_file(file_name: &str, sha: &str) -> Option<GameDbEntry> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let path = std::path::PathBuf::from(home).join(".cfb").join(file_name);
    let src = std::fs::read_to_string(path).ok()?;
    query_game(&src, sha)
}

fn query_game(db_src: &str, sha: &str) -> Option<GameDbEntry> {
    let db: serde_json::Value = serde_json::from_str(db_src).ok()?;
    let entry = db.get(sha)?;
    serde_json::from_value(entry.clone()).ok()
}

fn query_bl(db_src: &str, title: &str) -> Option<BatterylessConfig> {
    let db: serde_json::Value = serde_json::from_str(db_src).ok()?;
    let arr = db.get(title)?.as_array()?;
    let offset = arr.first()?.as_u64()?;
    let size = arr.get(1)?.as_u64()?;
    let layout = arr.get(2).and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    Some(BatterylessConfig { offset, size, layout })
}

/// 把逻辑存档展开为 flashGBX 写入用的 ROM 映像（layout 1/2 填到对应半 bank，其余 0xFF）。
pub fn expand_batteryless_image(save: &[u8], cfg: &BatterylessConfig) -> Vec<u8> {
    let span = cfg.rom_span() as usize;
    let mut image = vec![0xFFu8; span];
    let logical = save.len().min(cfg.size as usize);
    match cfg.layout {
        1 => {
            let banks = span / 0x4000;
            for i in 0..banks {
                let src = i * 0x2000;
                if src >= logical {
                    break;
                }
                let n = (logical - src).min(0x2000);
                let dst = i * 0x4000;
                image[dst..dst + n].copy_from_slice(&save[src..src + n]);
            }
        }
        2 => {
            let banks = span / 0x4000;
            for i in 0..banks {
                let src = i * 0x2000;
                if src >= logical {
                    break;
                }
                let n = (logical - src).min(0x2000);
                let dst = i * 0x4000 + 0x2000;
                image[dst..dst + n].copy_from_slice(&save[src..src + n]);
            }
        }
        _ => {
            image[..logical].copy_from_slice(&save[..logical]);
        }
    }
    image
}

/// 从 ROM 跨度映像中按 layout 抽出逻辑存档。
pub fn extract_batteryless_save(rom_span: &[u8], cfg: &BatterylessConfig) -> Vec<u8> {
    let mut out = Vec::with_capacity(cfg.size as usize);
    match cfg.layout {
        1 => {
            let mut i = 0usize;
            while out.len() < cfg.size as usize && i + 0x2000 <= rom_span.len() {
                let take = ((cfg.size as usize) - out.len()).min(0x2000);
                out.extend_from_slice(&rom_span[i..i + take]);
                i += 0x4000;
            }
        }
        2 => {
            let mut i = 0usize;
            while out.len() < cfg.size as usize && i + 0x4000 <= rom_span.len() {
                let take = ((cfg.size as usize) - out.len()).min(0x2000);
                out.extend_from_slice(&rom_span[i + 0x2000..i + 0x2000 + take]);
                i += 0x4000;
            }
        }
        _ => {
            let n = (cfg.size as usize).min(rom_span.len());
            out.extend_from_slice(&rom_span[..n]);
        }
    }
    out.truncate(cfg.size as usize);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pokemon_red_bl_entry() {
        let mut rom = vec![0u8; 0x180];
        rom[0x134..0x13F].copy_from_slice(b"POKEMON RED");
        rom[0x143] = 0x00;
        let cfg = lookup_dmg_batteryless(&rom).expect("POKEMON RED 应在 db_DMG_bl");
        assert_eq!(cfg.offset, 851968);
        assert_eq!(cfg.size, 32768);
        assert_eq!(cfg.layout, 2);
        assert_eq!(cfg.rom_span(), 65536);
    }

    #[test]
    fn layout2_roundtrip() {
        let cfg = BatterylessConfig {
            offset: 0,
            size: 0x4000,
            layout: 2,
        };
        let mut save = vec![0u8; 0x4000];
        for (i, b) in save.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        let image = expand_batteryless_image(&save, &cfg);
        assert_eq!(image.len(), 0x8000);
        assert!(image[0..0x2000].iter().all(|&b| b == 0xFF));
        let back = extract_batteryless_save(&image, &cfg);
        assert_eq!(back, save);
    }
}
