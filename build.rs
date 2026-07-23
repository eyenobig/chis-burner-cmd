//! 构建脚本：把子库 `vendor/chis-burner-rule/profiles/{agb,dmg}/*.json` 收集成
//! 一个编译期数组，供 `profile.rs` `include_str!` 进二进制作为内置 profile。
//!
//! 生成的 `OUT_DIR/profiles_gen.rs` 形如：
//!   pub const BUILTIN: &[(&str, &str)] = &[
//!       ("agb/S29GL256", include_str!("../vendor/chis-burner-rule/profiles/agb/S29GL256.json")),
//!       ...
//!   ];

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=vendor/chis-burner-rule/profiles");
    println!("cargo:rerun-if-changed=src/profiles");

    let mut entries: Vec<(String, PathBuf)> = Vec::new();

    // 子库 profile（vendor/chis-burner-rule/profiles/{agb,dmg}）。
    collect_dir(Path::new("vendor/chis-burner-rule/profiles"), &mut entries);

    // cfb 自带的少量内置 profile（src/profiles/*.json，如 s29gl/mbc_default）。
    collect_dir(Path::new("src/profiles"), &mut entries);

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut src = String::from(
        "// 由 build.rs 自动生成，勿手改。\n\
         // (标签, JSON 源)。标签仅用于诊断；加载后用 profile 自身的 name。\n\
         pub const BUILTIN: &[(&str, &str)] = &[\n",
    );
    for (tag, path) in &entries {
        // include_str! 路径相对当前文件(OUT_DIR/profiles_gen.rs)。
        let abs = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        src.push_str(&format!(
            "    ({tag:?}, include_str!(r\"{}\")),\n",
            abs.display()
        ));
    }
    src.push_str("];\n");

    let out = std::env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out).join("profiles_gen.rs"), src).unwrap();

    println!(
        "cargo:warning=profile: 内置 {} 个 profile (子库 + src/profiles)",
        entries.len()
    );
}

/// 递归收集 dir 下所有 *.json，标签为相对 dir 的路径（去 .json）。
fn collect_dir(dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.is_dir() {
            collect_dir(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let rel = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let tag = rel.trim_end_matches(".json").to_string();
            out.push((tag, path));
        }
    }
}
