---
name: mbc-burn-flow
description: cfb 烧录 GB/GBC (MBC) ROM 的端到端流程与 MBC3/MBC5 协议差异。实现或调试 mbc::ops 下的 burn/dump/erase，或排查「bank 写错地址 / 16KB 处卡死 / MBC3 ROM 烧不进」类 bug 时必读。含寻址差异表、verify 规范、配置驱动愿景。
---

# cfb MBC burn 流程

## 核心坑：MBC3 ≠ MBC5 的 bank 寻址

cfb 曾硬编码 MBC5 协议，烧 MBC3 ROM 在 16KB（bank 1 起点）卡死 125s。根因：
- **bank 0 数据地址**：MBC3 走固定窗口 `0x0000-0x3FFF`（不切 bank）；MBC5 恒 `0x4000`。
- **bank 切换寄存器**：MBC3 只写 `0x2000`（bank 0 硬件重映射为 1）；MBC5 写 `0x3000`(高位)+`0x2000`(低位)。
- **flash 命令序列**（unlock 0xAAA/0x555、sector erase 0x30、program cmd 0xfc）两代**完全相同**——差异只在上面两处。

所有差异封装在 `mbc::ops::read::{switch_bank, bus_addr}` 的 `MbcKind` 分发里。改寻址逻辑只改这两处，调用点传 `kind`。

## 流程化 burn 的 6 步（`mbc/ops/write.rs::burn`）

1. **识别**：从 ROM 文件头 `rom[0x147]` 读 cartridge_type → `MbcKind::from_cartridge_type`。不读卡带（避开上电首包被吞）。
2. **查 flash**：`rom_get_size`（CFI）取 device_size / buffer_write。（阶段 2 在此注入 profile）
3. **空间校验**：`length <= device_size`，否则失败退出。
4. **擦除**：`erase_range(link, kind, 0, length)` 逐 16KB bank sector erase，每片轮询 0xFF。
5. **写入**：`program_flow` 每 4096B 一包 0xfc，失败每 5 包重连、60 次放弃；**重连后必须 re-`switch_bank`**（reconnect 内部走 GBA warm_up，不重置 MBC bank 寄存器）。
6. **校验**：`verify_flow` 每 4096B `gbc_read` 逐字节比对，累计 `mismatch_bytes`（修了原来恒 0 的 bug）。MBC 不做修复轮。

## MBC3/MBC5 寻址差异表（严格对照 C# `mission_mbc5.cs:82-134`）

| 项 | MBC3 | MBC5 |
|---|------|------|
| bank 0 数据地址 | **`0x0000-0x3FFF` 固定窗口**（不切 bank） | `0x4000-0x7FFF` |
| bank ≥1 数据地址 | `0x4000-0x7FFF` | `0x4000-0x7FFF` |
| 切 bank | `gbc_write(0x2000, (bank==0?1:bank)&0xff)` | `gbc_write(0x3000, bank>>8)` 后 `gbc_write(0x2000, bank&0xff)` |
| bank 寄存器位宽 | 7 位（物理上限 2MB） | 9 位（物理上限 8MB） |
| bank 0 请求的硬件行为 | 自动重映射为 bank 1（switchable 区） | 即 bank 0 |
| flash 命令序列 | 相同（AMD/JEDEC 标准） | 相同 |
| RAM bank 切换 | `gbc_write(0x4000, bank&0x07)` | `gbc_write(0x4000, bank&0xff)` |

⚠️ **MBC3 bank 0 必须走固定窗口 `0x0000`**，绝不要 `switch_bank(0)+bus_addr→0x4000`。向 `0x0000` 写 sector erase 命令 0x30 安全——MBC3 把它当 RAM enable（0x30≠0x0A → 禁用 RAM，无害副作用），flash 仍正常收到命令。C# 生产代码同此行为。

## verify 规范

- 4096B 粒度（`PACKET` 常量），与 `program_flow` **同寻址**（同一 switch_bank/bus_addr/kind），绝不让校验用与写入不同的地址。
- 累计 `mismatch_bytes` 填入 `BurnResult` → 进 NDJSON `result` 事件。
- 单包 `gbc_read` 失败 → `reconnect` + re-`switch_bank` + 重试本包（不前进 read）。
- MBC 阶段 1 不做修复轮；如需，复用 `program_flow` + `erase_range`（sector = 16KB），与 GBA `find_bad_sectors` 同构。

## 上电时序：第一条命令会被吞

`device::power()` 末尾 `toggle_reset_lines()` 重置 MCU 命令 buffer，下一条串口包必被吞（见 `cartridge_link.rs` 注释）。
- GBA：`warm_up()` 吸收（双发 `rom_write(0x00, 0xf0)` reset + 丢弃响应）。
- MBC：`gbc_warm_up()`（GB 总线版，`gbc_write(0x00, 0xf0)`）。`open_powered` 对 MBC 分支已调用。
- dump 读卡带头另加 `read_cart_byte`（自带 1 次重试）兜底。burn 的 MBC 类型从 ROM 文件读，不受影响。

## dump 长度决策

`cmd_dump` MBC 缺省长度优先用 header `0x148`（游戏 ROM 大小，`32KB << code`，code ≤ 8），无效则回落 CFI `rom_get_size`（flash 芯片大小）。`--len N` 显式覆盖最高优先。

## 配置驱动愿景（阶段 2，方向）

内建 `MbcKind::from_cartridge_type` 即「特征→行为」查表，是配置的代码版兜底。阶段 2 引入 `profiles/default.toml`（flashGBX 风格）：
- 按 `cartridge_type` 区间 / `flash_id` 字符串匹配；
- 字段：`mbc_kind` / `voltage` / flash 命令字节（unlock_addr、sector_erase_cmd…）/ `buffer_write` 覆盖；
- CFI 永远提供 device_size / sector 布局（物理量），profile 提供协议常量；
- burn 第 2 步（查 flash）是 profile 注入点——加 profile 时只改这一步。

> 分阶段依据：flashGBX 兼容几百种卡带，但「many share their flash chip command set」。cfb 当前单一 AMD/Spansion 命令集，唯一变量是 MBC 类型 + 映射，故阶段 1 内建映射即覆盖目标卡带。

## 端到端验证

```bash
cargo run --bin cfb -- rom-info --file testrom/pokemon_green.gb --json   # 离线识别 cartridge_type
cargo run --bin cfb -- burn --rom testrom/gb_check.gb --mbc --json        # MBC3 烧录+校验
cargo run --bin cfb -- dump --out dumped.gb --mbc --json                  # 回读
cmp testrom/gb_check.gb dumped.gb && echo OK                              # 比对（应无输出）
```

期望 burn 事件流：`识别: cartridge_type=0x?? MBC3 → MBC3` → `flash: 容量…` → `擦除目标区` → progress 平滑越过 16KB（修 bug 前在此卡死）→ `校验: 0 字节不符` → `result ok:true mismatch_bytes:0`。

参考源：C# `Z:\Project\beggar_socket\client\ChisFlashBurner\mission_mbc5.cs`（`mbc_BaseAddressOfBank`/`mbc_romSwitchBank` 在 82-134，`mission_verifyRom_mbc5` 在 757-868）；`utility.cs:202-248`（cartridge_type 区间表 + mbcTypeDetect）。
