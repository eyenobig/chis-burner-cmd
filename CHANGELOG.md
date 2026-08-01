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

## [v0.3.4] - 2026-08-02

### 新增

- **GBA EEPROM 存档**：新增 `eeprom4k`（512 B）与 `eeprom64k`（8 KiB），覆盖导出、写入、校验和清除，并校验固定容量与 EEPROM 串行协议。
- **GB/GBC Mapper**：新增 MBC1、MBC2 自动识别和 ROM bank 映射，烧录、导出、擦除路径按 mapper 切换。
- **MBC2 存档**：支持固定 512 字节、低 4 位有效的导出、写入与校验。

### 变更

- **进度与测试命令**：补充 EEPROM/MBC 操作进度，`burn --no-erase` 可用于已擦除 Flash 的纯写入吞吐测试。
- **帮助与错误信息**：存档类型提示加入 EEPROM 4K/64K，并统一中英文无效类型说明。

### 修复

- **MBC1/MBC2 地址切换**：按 mapper 修正高 ROM bank、RAM bank 和 MBC2 片内 RAM 地址，避免沿用 MBC3/MBC5 映射。
- **GBA 删除/写入流程**：修正跳过擦除、重试和进度边界，使测试烧录与读回校验保持一致。

## [v0.3.3] - 2026-07-26

### 变更

- **GBA 烧录对齐 beggar 稳定路径**：应答超时 3000ms；默认整片擦后连续写（`--sector` 才逐扇区）；烧前 `soft_unplug_gba`；`rom_program` 失败走 DTR/RTS 复位重试（最多 4 次），不再频繁关口 reconnect。
- **profile 地址按字节空间**：AGB flashGBX 序列 `0xAAA`/`0x555` 按字节解析再 `>> 1` 写总线；内置 `s29gl.json` 地址同步修正。
- **ChisFlash 命名**：S29GL256 profile 显示名改为 ChisFlash，保留 insideGadgets 别名；多 ID 命中时优先 ChisFlash。

### 修复

- **整片擦超时**：固件 `0xf1` 优先，超时放宽到 240s；profile 软件擦作回落，避免大片擦未完成即进入编程。

## [v0.3.2] - 2026-07-25

### 变更

- **MBC 烧录对齐稳定路径**：整片擦 + 16KB 扇区、擦后软插拔 3.3V、JS28F256 `buf_wr=256`；以临时成功路径合入标准 MBC 烧录流程。
- **统一进度展示**：百分比 + 时间（`progress_display`）。
- **默认空闲 3.3V**：`power_idle` / `soft_unplug_3v3`；重连路径统一 3.3V。

## [v0.3.1] - 2026-07-24

### 修复

- **发版 CI 附件永远是空的**：`release` job 里 `actions/checkout@v4` 放在了下载/整理二进制产物之后，其默认 `clean: true` 会 `git clean -ffdx` 清空工作区里未纳入版本控制的文件，把刚准备好的 `release/`/`artifacts/` 目录整个删掉，导致 Release 正文有、二进制附件却永远为空（v0.2.0、v0.3.0 均中招）。把 checkout 挪到最前面即可。

## [v0.3.0] - 2026-07-24

### 变更

- **授权由 MIT 改为 GPL-3.0**：因 `chis-burner-rule` 子库（flashGBX 派生）是 GPL-3.0，其 profile 数据被 `build.rs` 编进 cfb 二进制后，整体按 GPL 传染；下游客户端 beggar_chis 打包 cfb 二进制后亦 GPL。统一 GPL-3.0 消除授权矛盾（cfb 复刻的 beggar_socket 为 WTFPL，允许 relicensing）。
- **烧录 / 擦除优先按卡上实读 MBC 代次寻址**：`burn`/`erase` 不再只信 ROM 头 0x147 的 mapper 类型，改为优先用卡上实读到的类型选 bank 切换/地址映射，读不到才回退 ROM 头声明的类型（默认按 MBC5 处理空白/噪声片），避免烧录器 flash 卡与 ROM 头 mapper 不一致导致高位 bank 擦除/烧录失败。
- **`rom_get_cfi` 增加复位重试**：CFI 查询前后显式复位 flash，最多重试 3 次，同时探测均匀扇区大小，减少「容量读回 0」的偶发失败。

### 新增

- **`cfb erase` 支持分段进度**：按 CFI 探测到的扇区大小逐个擦除并汇报 `progress`/`log` 事件（容量未知时回落整片擦除，行为与之前一致，零回归）。GBA/MBC 两侧均已支持。
- **GB/GBC 卡带识别扩展**：`info` 事件新增 `cartridge_type`/`mbc_name`（卡带类型原始字节 + 对应 MBC 代次名，如 `MBC5`），以及免电存档（batteryless，靠 `db_DMG_bl` 按标题查表）的 ROM 内偏移/大小/布局字段，供客户端展示。
- **`cfb save-erase [--mbc] [--type T] [--len N]`**：擦除存档（按类型写满 `0xFF`；FLASH 路径会先整片擦除）。与 `save-dump`/`save-write`/`save-verify` 同一套存档类型与进度事件。
- **`CFB_RULE_DIR` 构建期环境变量**：覆盖内置的 `vendor/chis-burner-rule` 数据源目录，供下游客户端（如 beggar_chis）指定本地 rule 版本重新编译 sidecar，而不必依赖固定的子模块路径。
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
