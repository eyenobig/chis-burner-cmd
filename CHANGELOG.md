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

## [v0.4.3] - 2026-09-08

### 新增

- **`cfb save-probe`**：新子命令，探测存档芯片而不是靠默认值猜。把此前只在 Python 协议脚本里的四项手法收敛进 cfb：JEDEC ID 识别（`0xAA@0x5555`/`0x55@0x2AAA`/`0x90@0x5555` → 读 → `0xF0` 退出，命中 GBATEK 已知型号表得容量）、SRAM/FLASH 判别（位能否 0→1）、bank 拓扑（独立 / 镜像 / bank 内 32KiB 折返）、接触健康（同址多读一致性）。真机实测四张卡：GBA SBTP = SRAM 2×64KiB=128KiB；GBA 4BTP = FLASH `C2:09` Macronix MX29L010 2×64KiB=128KiB；GB MBC5 TESTROM 与 MBC3+RTC PM_CRYSTAL = SRAM 4×8KiB=32KiB。
  - 探测**会写卡**（JEDEC 命令字节会落进 SRAM 数据区），凡碰过的字节一律备份 → 还原 → 逐字节读回校验；还原失败以退出码 4 报错，不静默放过。四张卡探测前后存档均 0 字节差异；MBC3+RTC 卡另确认时钟仍在走且 `day_count` 未变（bank 掩码 `&0x07` 保证探测触不到 `0x08`-`0x0C` 的 RTC 寄存器）。
  - FLASH 认出 ID 后**直接返回、不下任何直写探针**：未擦除的 FLASH 直写会永久清位且不可还原。
  - 同址多读不一致（总线噪声 = 卡没插到底）时**拒绝下写命令**并以退出码 3 结束，避免把垃圾写进真存档。`--no-write` 只做只读健康检查。
- **NDJSON `save_probe` 事件**：见 [docs/client-protocol.md](docs/client-protocol.md)。
- **`info` 的 GBA RTC 改为实测**：不再按 GameCode 前缀名单猜。`info` 现在经 GPIO 向 S3511 发读命令 0xA6、收 7 个时间寄存器并校验合法性。名单只收录了几个官方卡号，自制卡会被漏判 —— 实测 `4BTP` 卡（S3511 在走）此前被报成 `rtc:false`，现在为 `true`。开销可忽略（`info` 全程 284ms）。判据取「7 字节不得全同 + 合法 BCD + 各字段在范围内」：无 GPIO 的卡上 SIO 是个固定 ROM 位，接收例程反复读同一地址的该位，收到的字节只能是 `0x00` 或 `0xFF` 且必然全同，这就是「无 RTC」的结构性签名。反过来「GPIO 数据口回读是否跟随写入值」不能作判据 —— 实测引脚设为输出时该口恒读 `0x00`。
  - 正反两例均实测：`4BTP`（带 RTC）读回 `00 01 23 01 06 18 46` → 判有；`SBTP`（无 RTC）读回 `00 00 00 00 00 00 00` → 判无。后者上 GPIO 使能与关闭的读数完全一致，印证了「无 S3511 应答时该位只是固定 ROM 位」。探测不扰动时钟（读命令不写 RTC 寄存器），实测前后秒数单调递增。
  - `rom-info` 解析 **ROM 文件** 时无卡可探，仍用 GameCode 启发式；两条路径的区别已在 [docs/read-id.md](docs/read-id.md) 写明。

### 修复

- **GBA `save-dump` 读出错误数据**：缺 `--type` / `--len` 时此前恒按 SRAM + 64KiB 处理，在 128KiB 卡上**静默只导一半还报成功**，导出结果拿去和整份存档比对即表现为「读出错误数据」；FLASH 卡被当 SRAM 时 bank 切换序列不对，高 64KiB 会读成低 bank 的镜像。现在缺省值一律先探测定型定尺寸（FLASH 查 JEDEC 表，SRAM 用 bank 独立性），确实判不出容量时显式警告而不是闷头截半。`save-write` / `save-verify` 缺 `--type` 时同样先认芯片。
- **GBA 存档 bank latch 收尾**：`save::dump` / `save::write` 处理超过 64KiB 后把 bank 拨回 0，避免下一条只读低 64KiB 的命令读到高 bank 数据。
- **`cfb rtc` 在无 RTC 的 GBA 卡上不再编造读数**：`read_s3511` 的文档一直写着「失败返回 None」，实际却无条件返回 `Some`，于是无 RTC 的卡会打印 `2000-00-00 00:00:00` 并以退出码 0 成功返回。现在读数不合法即返回 None，按退出码 3 报错（`SBTP` 卡实测）。i18n 里那句「无 GPIO 功能？」此前因该分支不可达而是死文案，现在才真正用上。副作用是电池耗尽的 RTC 卡也会判为不可用 —— 这比报个假时间诚实。

### 移除

- **`burn --chip-erase`**：该开关把整片擦和写入绑进一次进程，超时与克隆片命令都和能用的 `cfb erase` 对不齐。整片清场请先 `cfb erase` 再 `cfb burn`。仍传入该 flag 会以退出码 2 明确报错。已空白扇区跳过二次 `0x30` 的防护保留在 `erase` 路径。

### 修复

- **MBC 扇区擦**：已空白扇区不再补发 `0x30`，避免 S29GL 克隆片对同一 128KB 块二次擦除卡死。
- **GBA 擦完再烧**：`cfb erase` 结束后复位 flash；`burn` 发现 ROM 范围已空白则跳过扇区擦（已空白扇区也不再发 `0x30`）。此前勾选全片清理后写入会卡在 0%、`@0x2000` 失败。
- **GBA 烧录/擦除收尾断电**：对齐 C# `port.Close()`，`burn`/`erase` 结束发 `power_off` 而不是保持 3.3V 空闲。PPB 解锁后、空白跳过擦除后再软件插拔，避免下一轮卡在 `@0x2000`。

## [v0.4.2] - 2026-08-17

### 新增

- **`erase --boot`**：GB/GBC 擦除时连物理 0x0-0x3FFF 的隐藏头部区（开机窗）一起清。MBC5 线性映射下常规擦除从 phys 0x4000 起，旧 ROM 的卡头残留在隐藏区会让识别继续报旧游戏；`--boot` 复用烧录路径的开机窗专用擦除序列（`mbc::ops::write::erase_boot_window`），让卡真正回到空白态。仅 MBC 路径有效，GBA 忽略。
- **`CFB_RULE_DIR` 环境变量**：外部 profile 目录支持环境变量覆盖（优先级：`CFB_RULE_DIR` → `~/.cfb/profiles/`）。兼容「含 `profiles/` 子目录的根」（与 beggar_chis 的 rule 数据目录语义一致）和「直接放 json 的目录」两种形状。客户端据此把设置页的 rule 目录绑定到 cfb。

### 变更

- 版本号 0.4.1 → 0.4.2（行为变更：上述两项需要新版本才生效；旧版对 `--boot` 静默忽略）。
- `--help` 的 erase 用法行（8 语言）补 `--boot`。

## [v0.3.5] - 2026-08-07

### 变更

- **移除 batteryless（免电）存档类型**：删除 `SaveType::Batteryless` 变体及 `--type batteryless`/`bat` 入口。GBA 免电存档（靠 `<3 from Maniac` 魔数定位）与 GB/GBC 免电存档（`db_DMG_bl.json` 布局库）相关实现、命令分发、事件字段一并移除。存档类型精简为 5 类：`eeprom4k` / `eeprom64k` / `sram` / `flash` / `fram`。
- **i18n 补全 EEPROM**：`save.type_invalid` 的 8 种语言文案此前仅英文列出 `eeprom4k`/`eeprom64k`，其余 7 种语言漏列。现已统一补全，非法类型提示在所有语言下都完整列出 5 类。

### 移除

- `db_DMG_bl.json`（GB/GBC 免电存档布局库，28 条目）从 rule 子库删除，`build.rs` 不再嵌入 `GAMEDB_DMG_BL_SRC`。
- `Event::Info` 的 `batteryless_offset` / `batteryless_size` / `batteryless_layout` 三个 NDJSON 字段移除（客户端此前未消费）。

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
