//! GBA 游戏名数据库（按 ROM 头 SHA1 查游戏真名）。
//!
//! 复刻 flashGBX `RomFileAGB.GetHeader`/`GetDatabaseEntry`：
//! - 普通 ROM：`header_sha1 = SHA1(rom[0x000..0x180])`
//! - MultiBoot ROM（game_code 以 'M' 开头）：`SHA1(rom[0x000..0x100])`
//! - 用 `header_sha1` 作 key 查 `db_AGB.json`，命中取 `gn`(游戏名)。
//!
//! 数据库源自 flashGBX（GPL-3.0），经 chis-burner-rule 子库 `games/db_AGB.json` 由
//! `build.rs` `include_str!` 内置；外部 `~/.cfb/db_AGB.json` 可覆盖（同名优先）。

use sha1::{Digest, Sha1};

// build.rs 生成的内置 db_AGB.json 源（来自子库 games/db_AGB.json）。
include!(concat!(env!("OUT_DIR"), "/gamedb_gen.rs"));

/// 一条 db_AGB 条目（只取 cfb 关心的字段）。
#[derive(serde::Deserialize)]
struct DbEntry {
    gn: String,
}

/// 算 ROM 头的 SHA1（复刻 flashGBX：MultiBoot 用 0x100，否则 0x180）。
/// `rom` 须至少 0x180 字节（不足则按实际长度算——退化为该范围）。
pub fn header_sha1(rom: &[u8]) -> String {
    let is_multiboot = rom.get(0xAC).copied() == Some(b'M'); // game_code[0] == 'M'
    let end = if is_multiboot { 0x100 } else { 0x180 };
    let slice = &rom[..rom.len().min(end)];
    let mut hasher = Sha1::new();
    hasher.update(slice);
    format!("{:x}", hasher.finalize())
}

/// 按 header SHA1 查游戏名。命中返回 `gn`，否则 None。
/// 先查外部 `~/.cfb/db_AGB.json`（若存在），再查内置。
pub fn lookup_game_name(rom: &[u8]) -> Option<String> {
    let sha = header_sha1(rom);
    lookup_by_sha(&sha).map(|e| e.gn)
}

fn lookup_by_sha(sha: &str) -> Option<DbEntry> {
    // 外部覆盖优先。
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let path = std::path::PathBuf::from(home).join(".cfb").join("db_AGB.json");
        if let Ok(src) = std::fs::read_to_string(&path) {
            if let Some(e) = query(&src, sha) {
                return Some(e);
            }
        }
    }
    query(GAMEDB_SRC, sha)
}

fn query(db_src: &str, sha: &str) -> Option<DbEntry> {
    let db: serde_json::Value = serde_json::from_str(db_src).ok()?;
    let entry = db.get(sha)?;
    serde_json::from_value(entry.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiboot_uses_0x100_range() {
        // game_code[0] = 'M' (0x4D) → MultiBoot，取 0x100。
        let mut rom = vec![0u8; 0x180];
        rom[0xAC] = b'M';
        let sha_mb = header_sha1(&rom);
        // MultiBoot 的 SHA 应等于只取 0x100 的 SHA1（在改 rom 前/用独立副本算）。
        let mut h = Sha1::new();
        h.update(&rom[..0x100]);
        assert_eq!(sha_mb, format!("{:x}", h.finalize()));
        // 同样字节但 game_code[0] 非 M → 取 0x180，应不同。
        rom[0xAC] = b'B';
        let sha_normal = header_sha1(&rom);
        assert_ne!(sha_mb, sha_normal, "MultiBoot 与普通 ROM 的 SHA 范围应不同");
    }

    #[test]
    fn builtin_db_loads() {
        let db: serde_json::Value = serde_json::from_str(GAMEDB_SRC).expect("内置 db_AGB.json 必须可解析");
        let obj = db.as_object().expect("db 顶层应是对象");
        assert!(obj.len() >= 2000, "内置 db 至少 2000 条，实际 {}", obj.len());
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;
    #[test]
    fn real_db_key_resolves() {
        // 取 db 里第一个真实 key，确认 query 通路能命中并返回 gn。
        let db: serde_json::Value = serde_json::from_str(GAMEDB_SRC).unwrap();
        let (sha, val) = db.as_object().unwrap().iter().next().unwrap();
        let expected = val.get("gn").unwrap().as_str().unwrap().to_string();
        let got = query(GAMEDB_SRC, sha).expect("真实 key 应命中");
        assert_eq!(got.gn, expected);
    }
}
