//! 语言包（i18n）。嵌入 zh-CN / en 两套 JSON 键值表，运行时按 `--lang` 或系统语言选用。
//!
//! 用法：启动时 `i18n::init(lang)`，之后 `t("key")` 取译文，`tf("key", &[("name","值")])`
//! 做占位替换（模板里用 `{name}`）。缺失键回退 zh-CN，再缺失则原样返回 key。
//!
//! 加语言：在 `src/i18n/` 放一个 `<lang>.json`，在下面 `load()` 里登记即可。

use std::collections::HashMap;
use std::sync::OnceLock;

const ZH: &str = include_str!("i18n/zh-CN.json");
const EN: &str = include_str!("i18n/en.json");

const FALLBACK: &str = "zh-CN";

struct Packs {
    cur: HashMap<String, String>,
    fb: HashMap<String, String>,
}

static PACKS: OnceLock<Packs> = OnceLock::new();

fn parse(src: &str) -> HashMap<String, String> {
    serde_json::from_str(src).unwrap_or_default()
}

fn load(lang: &str) -> Option<HashMap<String, String>> {
    match lang {
        "zh-CN" | "zh" => Some(parse(ZH)),
        "en" => Some(parse(EN)),
        _ => None,
    }
}

/// 初始化语言。`lang` 为 None 时跟随系统，找不到则回退中文。只生效一次。
pub fn init(lang: Option<&str>) {
    let fb = parse(ZH);
    let want = lang
        .map(|s| s.to_string())
        .unwrap_or_else(detect_system);
    let cur = load(&want).unwrap_or_else(|| fb.clone());
    let _ = PACKS.set(Packs { cur, fb });
}

fn packs() -> &'static Packs {
    PACKS.get_or_init(|| {
        let fb = parse(ZH);
        Packs { cur: fb.clone(), fb }
    })
}

/// 取译文（无占位）。
pub fn t(key: &str) -> String {
    let p = packs();
    p.cur
        .get(key)
        .or_else(|| p.fb.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// 取译文并替换 `{name}` 占位。
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// 跟随系统语言：读 LANG/LC_ALL/LANGUAGE；Windows 上多为空则默认中文。
fn detect_system() -> String {
    let raw = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANGUAGE"))
        .unwrap_or_default()
        .to_lowercase();
    if raw.starts_with("zh") {
        "zh-CN".to_string()
    } else if raw.starts_with("en") {
        "en".to_string()
    } else {
        FALLBACK.to_string()
    }
}
