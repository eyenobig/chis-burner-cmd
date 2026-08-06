//! GBA/GB 游戏名数据库（源自 flashGBX）。
//!
//! - `db_AGB.json` / `db_DMG.json`：header SHA1 → 游戏名 / game code

use sha1::{Digest, Sha1};

include!(concat!(env!("OUT_DIR"), "/gamedb_gen.rs"));

/// 一条游戏库条目（GBA / DMG 共用字段子集）。
#[derive(serde::Deserialize, Clone, Debug)]
pub struct GameDbEntry {
    pub gn: String,
    #[serde(default)]
    pub gc: Option<String>,
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

pub fn lookup_agb(rom: &[u8]) -> Option<GameDbEntry> {
    let sha = header_sha1_agb(rom);
    lookup_file("db_AGB.json", &sha).or_else(|| query_game(GAMEDB_AGB_SRC, &sha))
}

pub fn lookup_dmg(rom: &[u8]) -> Option<GameDbEntry> {
    let sha = header_sha1_dmg(rom);
    lookup_file("db_DMG.json", &sha).or_else(|| query_game(GAMEDB_DMG_SRC, &sha))
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
