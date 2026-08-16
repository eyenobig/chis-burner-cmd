//! flashGBX 风格的 flash 芯片 profile（命令序列外部化）。
//!
//! 每份 profile 描述一种 flash 芯片：按 Autoselect ID 匹配，承载 reset / read_id /
//! read_cfi / sector_erase / chip_erase 等操作的**命令序列**。序列里每条是 `[地址, 值]`，
//! 地址/值可以是字面数（`"0x0555"`/`0x0555`）或占位符（`"SA"`=扇区基址、`"PA"`=编程地址、
//! `"PD"`=编程数据）；配套 `*_wait_for` 数组做轮询（`[地址, lo, hi]`，三项任一可为 null=跳过）。
//!
//! 格式对齐 flashGBX 的 `fc_*.txt`（`FlashGBX/config/`），用户可把 flashGBX 的文件直接
//! 拷到 `~/.cfb/profiles/` 用。
//!
//! **加载**：内置（`include_str!`）+ 外部 `~/.cfb/profiles/*.json`（外部按 `name` 覆盖内置；
//! 单个文件解析失败只 stderr 告警 + 跳过，不致命）。
//! **匹配**：`match_by_id` 按 `flash_ids` 前 4 字节比对 Autoselect ID。
//! **未命中**：调用方走原硬编码序列，行为与今天一致（零回归）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::cartridge_link::CartridgeLink;

// build.rs 生成的内置 profile 清单（子库 vendor/chis-burner-rule + src/profiles）。
include!(concat!(env!("OUT_DIR"), "/profiles_gen.rs"));

/// 轮询一行默认超时（秒）。
const WAIT_TIMEOUT_SECS: u64 = 30;

/// 一个接受 "0x.." 字符串或整数的数字（用于 flash_ids / wait_for 等纯数值字段）。
#[derive(Clone, Copy, Debug)]
pub struct FlexNum(pub u32);

impl<'de> Deserialize<'de> for FlexNum {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => parse_num(&s)
                .map(FlexNum)
                .ok_or_else(|| serde::de::Error::custom(format!("expected number, got '{s}'"))),
            serde_json::Value::Number(n) => Ok(FlexNum(n.as_u64().ok_or_else(|| serde::de::Error::custom("non-u64"))? as u32)),
            other => Err(serde::de::Error::custom(format!("expected number, got {other}"))),
        }
    }
}

/// 卡型。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ProfileKind {
    Agb,
    Dmg,
}

/// 地址：字面数（"0x0555" / 0x0555 / 170）或占位符 SA/PA。
#[derive(Clone, Debug)]
pub enum CmdAddr {
    Sym(String), // "SA" / "PA"
    Lit(u32),
}

/// 值：字面数（"0xAA" / 0xAA / 170）或占位符 PD。
#[derive(Clone, Debug)]
pub enum CmdVal {
    Sym(String), // "PD"
    Lit(u8),
}

/// 已知占位符集合；非占位符的字符串按数字解析。
const ADDR_SYMS: &[&str] = &["SA", "PA"];
const VAL_SYMS: &[&str] = &["PD"];

impl<'de> Deserialize<'de> for CmdAddr {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        deserialize_addr_or_val(d, ADDR_SYMS).map(|raw| match raw {
            RawAV::Sym(s) => CmdAddr::Sym(s),
            RawAV::Num(n) => CmdAddr::Lit(n),
        })
    }
}
impl<'de> Deserialize<'de> for CmdVal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        deserialize_addr_or_val(d, VAL_SYMS).map(|raw| match raw {
            RawAV::Sym(s) => CmdVal::Sym(s),
            RawAV::Num(n) => CmdVal::Lit(n as u8),
        })
    }
}

enum RawAV {
    Sym(String),
    Num(u32),
}

/// 解析“占位符字符串 or 数字(可带 0x)”。
fn deserialize_addr_or_val<'de, D: serde::Deserializer<'de>>(
    d: D,
    syms: &[&str],
) -> Result<RawAV, D::Error> {
    use serde::Deserialize;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => {
            let up = s.trim().to_uppercase();
            if syms.contains(&up.as_str()) {
                Ok(RawAV::Sym(up))
            } else {
                let n = parse_num(&s).ok_or_else(|| {
                    serde::de::Error::custom(format!("expected symbol or number, got '{s}'"))
                })?;
                Ok(RawAV::Num(n))
            }
        }
        serde_json::Value::Number(n) => {
            let v = n.as_u64().ok_or_else(|| serde::de::Error::custom("non-u64 number"))? as u32;
            Ok(RawAV::Num(v))
        }
        other => Err(serde::de::Error::custom(format!("expected string/number, got {other}"))),
    }
}

/// "0x.." 十六进制或十进制字符串 → u32。
fn parse_num(s: &str) -> Option<u32> {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        t.parse::<u32>().ok()
    }
}

/// 轮询地址：可为 null（无轮询）、字面数、或 "SA" 占位符（用扇区基址）。
#[derive(Clone, Debug)]
pub enum WaitAddr {
    Lit(u32),
    Sa,
}

impl<'de> Deserialize<'de> for WaitAddr {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match deserialize_addr_or_val(d, ADDR_SYMS)? {
            RawAV::Sym(s) if s == "SA" => WaitAddr::Sa,
            RawAV::Sym(_) => return Err(serde::de::Error::custom("wait addr 只支持 SA 占位符")),
            RawAV::Num(n) => WaitAddr::Lit(n),
        })
    }
}

/// 轮询一行：[地址, lo, hi]，任一可为 null=不约束该项。
/// flashGBX 语义：读「地址」得到一个值，要求 lo <= val <= hi 才算完成（仍轮询直到满足）。
/// lo/hi 为 null 表示该维度不约束；地址为 null 表示此 cmd 无轮询。
#[derive(Clone, Debug, Deserialize)]
pub struct WaitRow {
    pub addr: Option<WaitAddr>,
    pub lo: Option<FlexNum>,
    pub hi: Option<FlexNum>,
}

/// 一组命令集（每个 key 对应一段序列；序列字段名后缀 `_wait_for` 是其轮询）。
/// 用宽松反序列化：原始 JSON 是扁平的 `{"sector_erase":[...],"sector_erase_wait_for":[...]}`，
/// 这里在 [`Profile::seq`] 手动配对。
#[derive(Clone, Debug, Default)]
pub struct SeqFull {
    pub cmds: Vec<(CmdAddr, CmdVal)>,
    pub waits: Vec<WaitRow>,
}

/// 一个 flash 芯片 profile。
#[derive(Clone, Debug, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ProfileKind,
    #[serde(default)]
    pub flash_ids: Vec<Vec<FlexNum>>,
    #[serde(default)]
    pub voltage: f32,
    /// flash 容量：数字(字节) 或某些芯片的列表布局。cfb 目前未接入，原样存。
    #[serde(default)]
    #[allow(dead_code)]
    pub flash_size: serde_json::Value,
    /// 扇区大小：数字(字节) 或非均匀扇区的 [[size,count],...] 列表。cfb 目前未接入。
    #[serde(default)]
    #[allow(dead_code)]
    pub sector_size: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code)]
    pub sector_size_from_cfi: bool,
    #[serde(default)]
    pub chip_erase_timeout: u64,
    #[serde(default)]
    pub commands: serde_json::Map<String, serde_json::Value>,
}

impl Profile {
    /// 把扁平 commands map 配对成 SeqFull（cmds + waits）。
    fn seq(&self, base: &str) -> Option<SeqFull> {
        let cmds_val = self.commands.get(base)?;
        let cmds: Vec<(CmdAddr, CmdVal)> = serde_json::from_value(cmds_val.clone()).ok()?;
        let waits: Vec<WaitRow> = self
            .commands
            .get(&format!("{base}_wait_for"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Some(SeqFull { cmds, waits })
    }
    #[allow(dead_code)]
    pub fn reset(&self) -> Option<SeqFull> {
        self.seq("reset")
    }
    #[allow(dead_code)]
    pub fn read_identifier(&self) -> Option<SeqFull> {
        self.seq("read_identifier")
    }
    #[allow(dead_code)]
    pub fn read_cfi(&self) -> Option<SeqFull> {
        self.seq("read_cfi")
    }
    pub fn sector_erase(&self) -> Option<SeqFull> {
        self.seq("sector_erase")
    }
    pub fn chip_erase(&self) -> Option<SeqFull> {
        self.seq("chip_erase")
    }
    /// 返回所有匹配键（前 4 字节 ID）。
    pub fn id_keys(&self) -> Vec<[u8; 4]> {
        self.flash_ids
            .iter()
            .filter_map(|v| {
                if v.len() >= 4 {
                    Some([v[0].0 as u8, v[1].0 as u8, v[2].0 as u8, v[3].0 as u8])
                } else {
                    None
                }
            })
            .collect()
    }
    /// 卡型标签（"AGB" / "DMG"）。
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            ProfileKind::Agb => "AGB",
            ProfileKind::Dmg => "DMG",
        }
    }
}

/// 外部 profile 目录：`CFB_RULE_DIR` 环境变量优先（客户端设置注入，见 beggar_chis
/// cfb_config.rs 同名语义——指向「含 profiles/ 子目录的根」；也兼容直接放 json 的目录），
/// 未设置时回落 `~/.cfb/profiles/`。
pub fn external_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CFB_RULE_DIR") {
        let dir = dir.to_string_lossy().trim().to_string();
        if !dir.is_empty() {
            let root = PathBuf::from(&dir);
            let with_profiles = root.join("profiles");
            return Some(if with_profiles.is_dir() { with_profiles } else { root });
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".cfb").join("profiles"))
}

/// 加载所有 profile：内置 + 外部（外部按 name 覆盖内置；坏的文件 stderr 告警跳过）。
pub fn load_all() -> Vec<Profile> {
    let mut builtins: Vec<Profile> = BUILTIN
        .iter()
        .filter_map(|(tag, src)| parse_one(src, tag))
        .collect();

    if let Some(dir) = external_dir() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            // 外部按 name 去重覆盖内置。
            let mut by_name: HashMap<String, usize> =
                builtins.iter().enumerate().map(|(i, p)| (p.name.clone(), i)).collect();
            for ent in entries.flatten() {
                let path = ent.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(src) = std::fs::read_to_string(&path).ok() else { continue };
                let name_str = path.display().to_string();
                match parse_one(&src, &name_str) {
                    Some(p) => {
                        if let Some(&i) = by_name.get(&p.name) {
                            builtins[i] = p; // 同名覆盖
                        } else {
                            by_name.insert(p.name.clone(), builtins.len());
                            builtins.push(p);
                        }
                    }
                    None => eprintln!("profile: 跳过无法解析的 {name_str}"),
                }
            }
        }
    }
    if builtins.is_empty() {
        // 完全无 profile：既无内置（剥离构建）又无外部 rule（首运行未下载）。
        // 给出明确指引，避免后续烧录因"找不到 profile"而莫名其妙失败。
        let dir = external_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "~/.cfb/profiles/".into());
        eprintln!(
            "profile: 无可用 profile（内置为空且外部目录 {dir} 无文件）。\n\
             请在客户端首运行时下载 rule，或手动把 profiles 放到 {dir}。"
        );
    }
    builtins
}

fn parse_one(src: &str, _label: &str) -> Option<Profile> {
    serde_json::from_str(src).ok()
}

/// 按 8 字节 Autoselect ID 的前 4 字节匹配 profile。
///
/// 多条同 ID 时优先 **ChisFlash** 名称（本机卡带品牌），其次带明确容量的
/// FlashGBX/iG 条目，避免命中笼统的「GBA 默认」回落 profile。
/// 均匀扇区大小（字节）：profile `sector_size` 为纯数字且在 4KiB–256KiB 时返回。
/// 列表布局（非均匀扇区）返回 None——调用方回落 CFI/硬编码路径。
pub fn uniform_sector_size(p: &Profile) -> Option<u32> {
    if p.sector_size_from_cfi {
        return None;
    }
    if let serde_json::Value::Number(n) = &p.sector_size {
        let v = n.as_u64()?;
        if (4096..=256 * 1024).contains(&v) {
            return Some(v as u32);
        }
    }
    None
}

pub fn match_by_id<'a>(profiles: &'a [Profile], id: &[u8; 8]) -> Option<&'a Profile> {
    let key = [id[0], id[1], id[2], id[3]];
    let mut hits: Vec<&Profile> = profiles.iter().filter(|p| p.id_keys().contains(&key)).collect();
    if hits.is_empty() {
        return None;
    }
    hits.sort_by_key(|p| {
        let name = p.name.to_ascii_lowercase();
        let chis = !name.contains("chisflash");
        // flash_size 为具体数字时更优先于 0 / 空
        let sized = match &p.flash_size {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) == 0,
            _ => true,
        };
        (chis, sized, p.name.as_str())
    });
    Some(hits[0])
}

/// 解析一条命令的地址/值为真实 (addr, val)。
/// `sa`=扇区基址，`pa`=编程地址（本版未用，预留），`pd`=编程数据（本版未用，预留）。
/// 未知占位符返回 None（视为该 profile 不支持，调用方回落硬编码）。
fn resolve(cmd: &(CmdAddr, CmdVal), sa: u32, pa: u32, pd: u8) -> Option<(u32, u8)> {
    let addr = match &cmd.0 {
        CmdAddr::Lit(n) => *n,
        CmdAddr::Sym(s) => match s.as_str() {
            "SA" => sa,
            "PA" => pa,
            _ => return None,
        },
    };
    let val = match &cmd.1 {
        CmdVal::Lit(n) => *n,
        CmdVal::Sym(s) => match s.as_str() {
            "PD" => pd,
            _ => return None,
        },
    };
    Some((addr, val))
}

/// 在 GBA 总线执行一段序列。
///
/// flashGBX AGB profile 里的地址一律是**字节地址**（如 unlock `0xAAA`/`0x555`，
/// 与硬编码 `erase_sector` 的字地址 `0x555`/`0x2AA` 对应：`byte >> 1`）。
/// `rom_write` 吃字地址，故字面地址与 `SA` 都先按字节解析再 `>> 1`。
pub fn run_gba(link: &mut CartridgeLink, seq: &SeqFull, sa_byte: u32) -> bool {
    for (i, cmd) in seq.cmds.iter().enumerate() {
        let Some((addr_byte, val)) = resolve(cmd, sa_byte, 0, 0) else { return false };
        if !link.rom_write(addr_byte >> 1, &[val, 0x00]) {
            return false;
        }
        if let Some(wait) = seq.waits.get(i) {
            if !poll_gba(link, wait, sa_byte) {
                return false;
            }
        }
    }
    true
}

/// 在 GB/DMG 总线执行一段序列（地址为字节地址，走 gbc_write 写 [val]）。
pub fn run_dmg(link: &mut CartridgeLink, seq: &SeqFull, sa: u32) -> bool {
    for (i, cmd) in seq.cmds.iter().enumerate() {
        let Some((addr, val)) = resolve(cmd, sa, 0, 0) else { return false };
        if !link.gbc_write(addr, &[val]) {
            return false;
        }
        if let Some(wait) = seq.waits.get(i) {
            if !poll_dmg(link, wait, sa) {
                return false;
            }
        }
    }
    true
}

/// GBA 轮询：profile 地址为字节地址（与 `run_gba` 一致）；`rom_read` 也吃字节地址。
fn poll_gba(link: &mut CartridgeLink, w: &WaitRow, sa_byte: u32) -> bool {
    let Some(addr) = &w.addr else { return true };
    let addr_byte = match addr {
        WaitAddr::Sa => sa_byte,
        WaitAddr::Lit(n) => *n,
    };
    let lo = w.lo.map(|n| n.0 as u16);
    let hi = w.hi.map(|n| n.0 as u16);
    let start = Instant::now();
    let mut probe = [0u8; 2];
    loop {
        if link.rom_read(addr_byte, &mut probe) {
            let v = u16::from_le_bytes(probe);
            if lo.is_none_or(|l| v >= l) && hi.is_none_or(|h| v <= h) {
                return true;
            }
        }
        if start.elapsed() > Duration::from_secs(WAIT_TIMEOUT_SECS) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// DMG 轮询：地址当字节地址（SA 用扇区字节地址），读 1 字节判 lo<=v<=hi。
fn poll_dmg(link: &mut CartridgeLink, w: &WaitRow, sa: u32) -> bool {
    let Some(addr) = &w.addr else { return true };
    let addr_byte = match addr {
        WaitAddr::Sa => sa,
        WaitAddr::Lit(n) => *n,
    };
    let lo = w.lo.map(|n| n.0 as u8);
    let hi = w.hi.map(|n| n.0 as u8);
    let start = Instant::now();
    let mut probe = [0u8; 1];
    loop {
        if link.gbc_read(addr_byte, &mut probe) {
            let v = probe[0];
            if lo.is_none_or(|l| v >= l) && hi.is_none_or(|h| v <= h) {
                return true;
            }
        }
        if start.elapsed() > Duration::from_secs(WAIT_TIMEOUT_SECS) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `cfb profile list|path` —— profile 管理/诊断（无需硬件）。
/// - `list`：列内置+外部所有 profile（name / kind / flash_ids / voltage）。
/// - `path`：打印外部目录 `~/.cfb/profiles/`（提示往这放 JSON）。
pub fn cmd_profile(json: bool, sub: Option<&str>) -> std::process::ExitCode {
    use crate::event::{emit, Event};
    match sub {
        Some("path") => {
            let dir = external_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(无法确定主目录)".to_string());
            if json {
                emit(&Event::Log { message: dir.clone() });
            } else {
                println!("{}", crate::i18n::tf("profile.path_hint", &[("dir", &dir)]));
            }
            std::process::ExitCode::SUCCESS
        }
        Some("list") | None => {
            let profiles = load_all();
            if json {
                emit(&Event::Log {
                    message: format!("{} profiles", profiles.len()),
                });
                for p in &profiles {
                    let ids = p
                        .id_keys()
                        .into_iter()
                        .map(|k| format!("[{:02X} {:02X} {:02X} {:02X}]", k[0], k[1], k[2], k[3]))
                        .collect::<Vec<_>>()
                        .join(" ");
                    emit(&Event::Log {
                        message: format!("{}|{}|{}V|{}", p.name, p.kind_label(), p.voltage, ids),
                    });
                }
            } else {
                println!("{}", crate::i18n::tf("profile.list_header", &[("n", &profiles.len().to_string())]));
                for p in &profiles {
                    let ids = p
                        .id_keys()
                        .into_iter()
                        .map(|k| format!("[{:02X} {:02X} {:02X} {:02X}]", k[0], k[1], k[2], k[3]))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!(
                        "  {} [{}] {}V  {}",
                        p.name,
                        p.kind_label(),
                        p.voltage,
                        if ids.is_empty() { "(无 ID 匹配键)".to_string() } else { ids }
                    );
                }
            }
            std::process::ExitCode::SUCCESS
        }
        Some(other) => {
            let msg = crate::i18n::tf("profile.unknown_sub", &[("sub", other)]);
            if json {
                emit(&Event::Error { command: "profile".to_string(), message: msg });
            } else {
                eprintln!("{msg}");
            }
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 tag 从内置清单取 JSON 源。
    fn builtin_src(tag: &str) -> &'static str {
        BUILTIN.iter().find(|(t, _)| *t == tag).map(|(_, s)| *s).unwrap()
    }

    #[test]
    fn parses_builtin_s29gl() {
        let p: Profile = serde_json::from_str(builtin_src("s29gl")).expect("内置 s29gl 必须可解析");
        assert_eq!(p.kind, ProfileKind::Agb);
        assert!(!p.id_keys().is_empty());
        assert_eq!(p.id_keys()[0], [0xC2, 0x22, 0x28, 0x22]);
        let se = p.sector_erase().expect("有 sector_erase");
        assert_eq!(se.cmds.len(), 6);
        // 最后一条是 SA 占位符 + 0x30。
        assert!(matches!(se.cmds[5].0, CmdAddr::Sym(_)));
        assert!(matches!(se.cmds[5].1, CmdVal::Lit(_)));
        // 对应 wait 行应配对上。
        assert_eq!(se.waits.len(), 6);
        assert!(se.waits[5].addr.is_some()); // SA 轮询
    }

    #[test]
    fn parses_builtin_mbc() {
        let p: Profile = serde_json::from_str(builtin_src("mbc_default")).expect("内置 mbc 必须可解析");
        assert_eq!(p.kind, ProfileKind::Dmg);
        let ce = p.chip_erase().expect("有 chip_erase");
        assert_eq!(ce.cmds.len(), 6);
    }

    #[test]
    fn all_builtin_profiles_parse() {
        // 子库 + src/profiles 全部内置 profile 都必须能解析（154+2）。
        let bad: Vec<&str> = BUILTIN
            .iter()
            .filter(|(_, src)| serde_json::from_str::<Profile>(src).is_err())
            .map(|(t, _)| *t)
            .collect();
        assert!(bad.is_empty(), "以下内置 profile 解析失败: {bad:?}");
        assert!(BUILTIN.len() >= 150, "内置 profile 应 ≥150 个，实际 {}", BUILTIN.len());
    }

    #[test]
    fn parses_flashgbx_native_format() {
        // flashGBX 原版格式：数字裸写、null 占位、地址/值混用。确保兼容。
        let src = r#"{
            "name":"t","type":"DMG","voltage":5.0,
            "commands":{
                "sector_erase":[[4096,170],[4096,85]],
                "sector_erase_wait_for":[[null,null,null],["SA",255,255]]
            }
        }"#;
        let p: Profile = serde_json::from_str(src).unwrap();
        let se = p.sector_erase().unwrap();
        assert_eq!(se.cmds.len(), 2);
        assert!(matches!(se.cmds[0].1, CmdVal::Lit(170)));
        assert_eq!(se.waits[1].lo.unwrap().0, 255);
    }

    #[test]
    fn match_by_id_finds_profile() {
        let profiles = load_all();
        // S29GL 的 ID 前 4 字节。
        let hit = match_by_id(&profiles, &[0xC2, 0x22, 0x28, 0x22, 0, 0, 0, 0]);
        assert!(hit.is_some(), "应匹配到内置 S29GL");
        // 未知 ID 不命中。
        assert!(match_by_id(&profiles, &[0xDE, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn agb_s29gl_unlock_addrs_are_byte_space() {
        // flashGBX AGB：0xAAA/0x555（字节）≡ 硬编码 erase_sector 的字地址 0x555/0x2AA。
        let profiles = load_all();
        let p = match_by_id(&profiles, &[0x01, 0x00, 0x7E, 0x22, 0x22, 0x22, 0x01, 0x22])
            .expect("ChisFlash / S29GL256");
        assert!(
            p.name.to_ascii_lowercase().contains("chisflash"),
            "应优先命中 ChisFlash profile，实际: {}",
            p.name
        );
        let se = p.sector_erase().expect("sector_erase");
        assert!(matches!(&se.cmds[0].0, CmdAddr::Lit(0xAAA)));
        assert!(matches!(&se.cmds[1].0, CmdAddr::Lit(0x555)));
        // run_gba 必须 byte>>1 后才与 GbaFlasher.EraseSector 一致
        assert_eq!(0xAAA_u32 >> 1, 0x555);
        assert_eq!(0x555_u32 >> 1, 0x2AA);
    }
}
