# 版本记录（Changelog）

本文件记录 `cfb`（碳酸丐烧录器命令行）所有已发布版本的变更，是 **GitHub Release 正文的唯一来源**——发版 CI 会按 tag 号从本文件抽取对应段落，注入到 Release 正文里展示（见 [docs/ci.md](docs/ci.md) 的「CHANGELOG / 版本记录」）。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，只用手写维护，**不自动生成**。

## 如何维护

1. 日常改动先记在 [`## [Unreleased]`](#unreleased) 下，按 `新增` / `变更` / `修复` 分组。
2. 打 tag 发版前：
   - 把 `## [Unreleased]` 标题改成 `## [vX.Y.Z] - YYYY-MM-DD`（带方括号版本号、ISO 日期）；
   - 在文件**最上面**新开一个空的 `## [Unreleased]`，留待下个版本继续记。
3. 标题格式**必须**是 `## [vX.Y.Z] - YYYY-MM-DD`（发版 CI 靠它定位段落）。版本号要和 git tag 完全一致（含 `v` 前缀）。

> 新版本在**最上面**（最近的在前），`## [Unreleased]` 永远是第一条。

---

## [Unreleased]

### 新增

- **flash 芯片 profile 子库**：把 [flashGBX](https://github.com/lesserkuma/FlashGBX)（Lesserkuma，GPL-3.0）的 154 个 flash 芯片定义转换成 cfb profile，作为独立子库 [chis-burner-rule](https://github.com/eyenobig/chis-burner-rule)（git submodule 挂 `vendor/`）。`build.rs` 在编译期把子库 + `src/profiles/` 共 **156 个** profile 编进二进制——无需配置即覆盖 S29GL / MX29 / AM29 / M29W / SST39 / 28F / insideGadgets 系等常见 GBA/GB 可写卡带。CI 加 `submodules: true`。
- **flashGBX 风格 flash profile**：把烧录流程里硬编码的 flash 命令序列（reset/read_id/read_cfi/sector_erase/chip_erase）外部化为 JSON，按 Autoselect ID 前 4 字节匹配芯片。内置 S29GL（GBA，含 Macronix+Spansion ID）与 MBC 默认两套；外部 `~/.cfb/profiles/*.json` 可覆盖/补充，格式兼容 flashGBX 的 `fc_*.txt`（可直接拷来用）。`cfb profile list/path` 子命令管理与诊断。**未命中走原硬编码，零回归。** 详见 [docs/profiles.md](docs/profiles.md)。

## [v0.2.0] - 2026-07-23

### 新增

- **八国语言**：语言包从 zh-CN / en 扩充为 **zh-CN / en / ja / ko / fr / de / es / pt-BR** 八套（完整 72 键）。`i18n.rs` 改为表驱动（`LANGS`），加语言只需放一个 json 并登记一项；`--lang` 与系统 locale 支持简写/前缀归一化（`zh`→zh-CN、`pt`→pt-BR、`ja_JP`→ja 等），命中不了回退中文。
- **存档操作**：`cfb save-dump` / `save-write` / `save-verify` 三个子命令，覆盖 GBA 存档 RAM（SRAM/FLASH/FRAM）、GBA 免电存档（batteryless，靠 `<3 from Maniac` 魔数定位）、MBC 存档 RAM（SRAM/FRAM）。协议原语 `0xf7/0xf8/0xf9/0xe7/0xe8/0xea/0xeb` 从 C# `cart_adapter.cs` 移植到 `cartridge_link`；操作逻辑复刻 `mission_gba.cs` / `mission_mbc5.cs`。`--type` 选存档类型（默认 sram），`--len` 指定 dump 字节数。新增 NDJSON `save_info` 事件。⚠️ 待硬件验证。
- **`cfb version` / `--version`**：报告版本号（来自 Cargo.toml）。`--json` 模式输出 NDJSON `version` 事件（`{"type":"version","version":"0.2.0"}`），供桌面客户端读取展示。

## [v0.1.0] - 2026-07-22

首个 Rust 版跨平台发布。从 C# WinForms 上位机复刻为跨平台命令行 `cfb`，产出 Windows / macOS（Intel + ARM）/ Linux 四平台 Tauri sidecar 二进制，供 [beggar_chis](https://github.com/eyenobig/beggar_chis) 客户端通过 NDJSON 协议调用。

### 新增

- **跨平台命令行 `cfb`**：Rust 实现，Windows / macOS / Linux 统一 API，用 `serialport` crate 做 USB CDC 串口 + VID/PID 枚举（`0483/0721`），无需像 C# 版那样按平台分别走 WMI/ioreg//sys。
- **子命令**：`detect`（只列烧录器）、`select`/`disconnect`（记住/释放烧录器，存 `~/.cfb.json`）、`voltage`（3v3/5v/off/auto 供电偏好）、`info`（读 flash + 卡带/游戏信息）、`rom-info`（离线解析 ROM 头）、`burn`（写 ROM）、`erase`（整片擦除）、`dump`（导出 ROM）、`rtc`（读 RTC）、`help`。
- **GB/GBC（MBC）支持**：`--mbc` 切换；MBC3/MBC5 按 ROM 头 `0x147` 自动识别；MBC3 RTC 读取。
- **GBA 烧录**：flash 读取、GBA 判别、头解析；burn 默认读回校验，支持 `--chip-erase` / `--no-ppb` / `--no-verify`。
- **给 JS 客户端的 NDJSON 事件流**：全局 `--json` 开关，每行一个 `{"type":...}` 事件，Electron/Tauri 客户端逐行 `JSON.parse` 流式展示。协议契约见 [docs/client-protocol.md](docs/client-protocol.md)，由 `src/event.rs` 实现。
- **i18n**：`--lang zh-CN|en`，缺省跟随系统、回退中文；文案走 `src/i18n/` 语言包（`zh-CN.json` / `en.json`），加语言只需加一个 json。
- **GitHub Actions 跨平台 CI**：tag + master 双重门禁触发，4 平台原生编译，产出 Tauri sidecar 命名二进制，按 tag 发 GitHub Release。

### 修复

- **发版 CI 容错**：`release` 作业改用 `if: !cancelled()`，单平台 build 偶发失败不再阻塞整条发版——已成功的平台照常发布；兜底是 flatten 步骤至少要有一个 `cfb-*` 二进制，否则报错退出（不发空 Release）。保证 `beggar_chis` 的 win/mac/linux 三主平台稳定供货。
- **Intel macOS runner 迁移**：`x86_64-apple-darwin` 构建的 runner 从 `macos-13` 迁到官方替代镜像 `macos-15-intel`（GitHub 已于 2025-12 下线 `macos-13`，这是上一轮 v0.1.0 发版被 skip 的根因）。
