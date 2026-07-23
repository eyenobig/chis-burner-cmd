//! 语言包（i18n）。嵌入八套 JSON 键值表（zh-CN / en / ja / ko / fr / de / es / pt-BR），
//! 运行时按 `--lang` 或系统语言选用。
//!
//! 用法：启动时 `i18n::init(lang)`，之后 `t("key")` 取译文，`tf("key", &[("name","值")])`
//! 做占位替换（模板里用 `{name}`）。缺失键回退 zh-CN，再缺失则原样返回 key。
//!
//! 加语言：在 `src/i18n/` 放一个 `<lang>.json`，并在下面 [`LANGS`] 登记一项即可
//! （包含源串 + 接受的系统 locale 前缀）。

use std::collections::HashMap;
use std::sync::OnceLock;

const ZH: &str = include_str!("i18n/zh-CN.json");
const EN: &str = include_str!("i18n/en.json");
const JA: &str = include_str!("i18n/ja.json");
const KO: &str = include_str!("i18n/ko.json");
const FR: &str = include_str!("i18n/fr.json");
const DE: &str = include_str!("i18n/de.json");
const ES: &str = include_str!("i18n/es.json");
const PT_BR: &str = include_str!("i18n/pt-BR.json");

const FALLBACK: &str = "zh-CN";

/// 一条语言登记：lang 代码、源 JSON、匹配的系统 locale 前缀（小写，如 "ja"、"pt_br"）。
struct Lang {
    code: &'static str,
    src: &'static str,
    /// 该语言接受的系统 locale 前缀（小写）。`detect_system` 用前缀匹配第一个命中。
    prefixes: &'static [&'static str],
}

/// 语言表（按优先级：具体区域在前，避免被通用前缀抢先）。
/// 例如 pt-BR 放在 pt 通用前缀前；zh-CN/zh 在 zh 前。
static LANGS: &[Lang] = &[
    Lang { code: "zh-CN", src: ZH, prefixes: &["zh_cn", "zh_sg", "zh"] },
    Lang { code: "en", src: EN, prefixes: &["en_us", "en_gb", "en"] },
    Lang { code: "ja", src: JA, prefixes: &["ja"] },
    Lang { code: "ko", src: KO, prefixes: &["ko"] },
    Lang { code: "fr", src: FR, prefixes: &["fr"] },
    Lang { code: "de", src: DE, prefixes: &["de"] },
    Lang { code: "es", src: ES, prefixes: &["es"] },
    Lang { code: "pt-BR", src: PT_BR, prefixes: &["pt_br", "pt"] },
];

struct Packs {
    cur: HashMap<String, String>,
    fb: HashMap<String, String>,
}

static PACKS: OnceLock<Packs> = OnceLock::new();

fn parse(src: &str) -> HashMap<String, String> {
    serde_json::from_str(src).unwrap_or_default()
}

/// 归一化用户/`--lang` 输入为已登记的 lang 代码；未登记返回 None。
/// 兼容 `zh` → `zh-CN`、`pt` → `pt-BR` 这类简写（取该前缀命中的第一条登记）。
fn normalize(lang: &str) -> Option<&'static str> {
    let l = lang.to_lowercase();
    // 先精确匹配 code（大小写不敏感）。
    if let Some(entry) = LANGS.iter().find(|e| e.code.eq_ignore_ascii_case(lang)) {
        return Some(entry.code);
    }
    // 再按系统 locale 前缀匹配（处理 `zh`/`pt`/`en_US` 这类输入）。
    for entry in LANGS {
        if entry.prefixes.iter().any(|p| l.starts_with(p)) {
            return Some(entry.code);
        }
    }
    None
}

fn load(lang: &str) -> Option<HashMap<String, String>> {
    normalize(lang).map(|code| {
        let src = LANGS.iter().find(|e| e.code == code).unwrap().src;
        parse(src)
    })
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

/// 跟随系统语言：读 LANG/LC_ALL/LANGUAGE，按 [`LANGS`] 前缀匹配；命中不了回退中文。
/// Windows 上这些环境变量多为空——上层若需要应另行读系统 UI 语言，此处保持现状。
fn detect_system() -> String {
    let raw = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANGUAGE"))
        .unwrap_or_default()
        .to_lowercase();
    let stripped = raw.split('.').next().unwrap_or(&raw).replace('-', "_");
    for entry in LANGS {
        if entry.prefixes.iter().any(|p| stripped.starts_with(p)) {
            return entry.code.to_string();
        }
    }
    FALLBACK.to_string()
}
