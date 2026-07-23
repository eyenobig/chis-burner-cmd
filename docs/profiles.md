# flash profile（芯片命令序列配置）

`cfb` 的烧录流程默认按 S29GL 系列（GBA）/ AMD-JEDEC（MBC）的**硬编码** flash 命令序列工作。
**profile 系统**把每片 flash 的命令序列（reset / read_id / read_cfi / sector_erase / chip_erase）
外部化为 JSON，按 **Autoselect ID 匹配**选用——这样支持非 S29GL 芯片、GB 复制卡，且**加新芯片只改 JSON 不改代码**。

格式对齐 [flashGBX](https://github.com/lesserkuma/FlashGBX) 的 `fc_*.txt`（`FlashGBX/config/`），可把 flashGBX 的文件直接拿来用。

## 工作原理

1. `cfb burn`/`info` 读卡带的 8 字节 Autoselect ID（如 `01 00 7E 22 22 22 01 22`）。
2. 取前 4 字节（`01 00 7E 22`）与所有 profile 的 `flash_ids` 比对。
3. **命中** → 用该 profile 的命令序列做擦除（sector/chip erase）；**未命中** → 走原硬编码序列（行为与无 profile 时完全一致，零回归）。
4. 编程（`rom_program`/`gbc_rom_program`）是固件命令，不归 profile 管。

## profile 文件位置

| 来源 | 路径 | 说明 |
|------|------|------|
| 内置（子库） | `vendor/chis-burner-rule/profiles/{agb,dmg}/*.json`（[chis-burner-rule](https://github.com/eyenobig/chis-burner-rule) 子库，build.rs 编译进二进制） | 156 个芯片，源自 flashGBX，无需配置即可工作 |
| 内置（自带） | `src/profiles/*.json` | cfb 自带默认（S29GL / MBC） |
| 外部（覆盖） | `~/.cfb/profiles/*.json` | 用户自加；**同名覆盖内置**；单个文件解析失败只 stderr 告警跳过，不致命 |

查看：`cfb profile list`（列全部）、`cfb profile path`（打印外部目录）。

## 内置 profile

内置共 **156 个**，主体来自子库 [chis-burner-rule](https://github.com/eyenobig/chis-burner-rule)（把 flashGBX 的 154 个芯片定义转换而来：AGB 56 + DMG 98），外加 cfb 自带 2 个默认：

| 来源 | 卡型 | 说明 |
|------|------|------|
| 子库 `profiles/agb/` | AGB(GBA) | 56 个：S29GL / MX29 / AM29 / M29W / 28F / insideGadgets 系等 |
| 子库 `profiles/dmg/` | DMG(GB/GBC) | 98 个：AM29F / SST39 / M29W / iG / GBFlash 系等 |
| `src/profiles/s29gl.json` | AGB | S29GL/MX29GL/Spansion 默认（含 `C2`/`01` ID） |
| `src/profiles/mbc_default.json` | DMG | AMD/JEDEC `0xAAA/0x555` 默认（MBC 暂不自动匹配） |

> 子库数据源自 [flashGBX](https://github.com/lesserkuma/FlashGBX)（GPL-3.0），转换脚本与归属见子库仓库。

## JSON 格式

```json
{
  "name": "S29GL / MX29GL (GBA 默认)",
  "type": "AGB",
  "flash_ids": [ ["0xC2","0x22","0x28","0x22"] ],
  "voltage": 3.3,
  "flash_size": 0,
  "sector_size": 131072,
  "sector_size_from_cfi": true,
  "chip_erase_timeout": 200,
  "command_set": "AMD",
  "commands": {
    "reset":            [[ "0x0000", "0xF0" ]],
    "read_identifier":  [[ "0x0555","0xAA" ],[ "0x02AA","0x55" ],[ "0x0555","0x90" ]],
    "read_cfi":         [[ "0x0055","0x98" ]],
    "sector_erase": [
      [ "0x0555","0xAA" ],[ "0x02AA","0x55" ],[ "0x0555","0x80" ],
      [ "0x0555","0xAA" ],[ "0x02AA","0x55" ],[ "SA",   "0x30" ]
    ],
    "sector_erase_wait_for": [
      [null,null,null],[null,null,null],[null,null,null],
      [null,null,null],[null,null,null],[ "SA","0xFFFF","0xFFFF" ]
    ],
    "chip_erase": [
      [ "0x0555","0xAA" ],[ "0x02AA","0x55" ],[ "0x0555","0x80" ],
      [ "0x0555","0xAA" ],[ "0x02AA","0x55" ],[ "0x0555","0x10" ]
    ],
    "chip_erase_wait_for": [
      [null,null,null],[null,null,null],[null,null,null],
      [null,null,null],[null,null,null],[ "0x0000","0xFFFF","0xFFFF" ]
    ]
  }
}
```

### 字段

| 字段 | 类型 | 含义 |
|------|------|------|
| `name` | string | 显示名（`cfb profile list` / burn 日志） |
| `type` | `"AGB"` / `"DMG"` | 卡型：AGB=GBA（字地址，`rom_write` 写 2 字节 `[val,0x00]`）；DMG=GB/GBC（字节地址，`gbc_write` 写 1 字节） |
| `flash_ids` | `[[b0,b1,b2,b3], ...]` | 匹配键，每项 4 字节（Autoselect ID 前 4 字节）。字节可写 `"0xC2"` 或 `194` |
| `voltage` | number | 该芯片电压（3.3 / 5.0），仅信息用 |
| `flash_size` | number | flash 容量字节；`0` = 走 CFI（默认） |
| `sector_size` | number | 扇区字节；与 `sector_size_from_cfi` 配合 |
| `chip_erase_timeout` | number | chip erase 兜底超时（秒） |
| `commands` | object | 各操作的命令序列（见下） |

### 命令序列（`commands.*`）

每段是一串 `[地址, 值]` 对：

- **地址**：`"0x0555"` / `0x555` / `1365`（字面数），或占位符：
  - `"SA"` = 扇区基址（sector erase 时由 cfb 填当前扇区）
  - `"PA"` = 编程地址（预留，本版 program 走固件未用）
- **值**：`"0xAA"` / `0xAA` / `170`（字面数），或占位符 `"PD"`（编程数据，预留）。

配套 `*_wait_for` 数组与命令**逐条对应**，每行 `[地址, lo, hi]`：
- 读「地址」得到一个值（AGB 读 2 字节、DMG 读 1 字节），要求 `lo <= val <= hi` 才算完成，否则继续轮询直到满足或超时。
- 任一项为 `null` 表示不约束（地址为 null = 此 cmd 无轮询；lo/hi 为 null = 该维度不限）。

支持的命令键：`reset` / `read_identifier` / `read_cfi` / `sector_erase` / `chip_erase`（`single_write` / `page_write` 可解析、暂未接入）。

## 从 flashGBX 拷文件

flashGBX 的 `FlashGBX/config/fc_*.txt`（如 `fc_AGB_29LV128DT.txt`）格式与本系统兼容，可直接拷到 `~/.cfb/profiles/` 用（建议改后缀 `.json`，并按需补 `flash_ids` 用于自动匹配）：

```bash
mkdir -p ~/.cfb/profiles
cp fc_AGB_29LV128DT.txt ~/.cfb/profiles/29lv128dt.json
cfb profile list   # 确认加载
```

## 故障排查

- **`cfb burn` 没显示 `Profile:` 行**：当前卡带 ID 没命中任何 profile，走的是硬编码默认序列（正常）。用 `cfb info` 看 ID，必要时新建一个 profile 含该 ID。
- **外部 profile 没加载**：`cfb profile path` 看目录对不对；stderr 会有「跳过无法解析的 <文件>」告警——多半是 JSON 语法错或字段类型不对。
- **擦除/烧录失败**：profile 命令序列与该芯片不符。可临时删掉对应外部 profile 让它回落硬编码，或核对 flashGBX 同型号芯片的序列。

## 源码

- [`src/profile.rs`](../src/profile.rs)：数据类型 + serde 反序列化（兼容 flashGBX 格式）+ 加载/匹配/执行。
- [`build.rs`](../build.rs)：编译期把子库 + `src/profiles/` 的 JSON 收集成内置清单（`include_str!` 进二进制）。
- [`src/profiles/*.json`](../src/profiles/)：cfb 自带默认芯片。
- [`vendor/chis-burner-rule/`](../vendor/chis-burner-rule/)（git submodule）：156 个芯片 profile 数据库（源自 flashGBX）。
- 接入点：[`src/rom/gba/ops/write.rs`](../src/rom/gba/ops/write.rs)（`burn` 里 `match_by_id`）、[`src/rom/gba/ops/delete.rs`](../src/rom/gba/ops/delete.rs)（`*_profile` 擦除）。

> **构建提示**：本仓库含 git submodule（`vendor/chis-burner-rule`）。克隆后需 `git submodule update --init`，否则 `cargo build` 找不到子库 profile（仍可用 `src/profiles/` 自带的 2 个）。CI 已配置 `--recurse-submodules`。
