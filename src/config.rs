//! 本地配置（持久化的用户偏好）：记住的烧录器端口、语言、电压。
//!
//! 存用户主目录下的 `~/.cfb.json`（Windows 为 `%USERPROFILE%\.cfb.json`），
//! 形如 `{"port":"COM7","lang":"en","voltage":"5V"}`。读改写保留其它键。

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".cfb.json"))
}

fn load_all() -> Value {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| Value::Object(Default::default()))
}

fn save_all(v: &Value) {
    if let Some(p) = config_path() {
        let _ = fs::write(p, v.to_string());
    }
}

/// 读一个字符串键。
fn get(key: &str) -> Option<String> {
    load_all().get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// 写一个字符串键（保留其它键）。
fn set(key: &str, val: &str) {
    let mut v = load_all();
    if let Value::Object(map) = &mut v {
        map.insert(key.to_string(), Value::String(val.to_string()));
    }
    save_all(&v);
}

/// 删一个键（保留其它键）。
fn remove(key: &str) {
    let mut v = load_all();
    if let Value::Object(map) = &mut v {
        map.remove(key);
    }
    save_all(&v);
}

// ---- 端口（select 记住的烧录器）----
pub fn load_selected() -> Option<String> {
    get("port")
}
pub fn save_selected(port: &str) {
    set("port", port);
}
pub fn clear_selected() {
    remove("port");
}

// ---- 语言 ----
pub fn load_lang() -> Option<String> {
    get("lang")
}
pub fn save_lang(lang: &str) {
    set("lang", lang);
}

// ---- 电压（记住的供电偏好；尤其 MBC 5V）----
pub fn load_voltage() -> Option<String> {
    get("voltage")
}
pub fn save_voltage(v: &str) {
    set("voltage", v);
}
pub fn clear_voltage() {
    remove("voltage");
}
