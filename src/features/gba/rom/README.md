# GBA · ROM 读写

GBA 卡带 ROM（线性 NOR Flash）的读取 / 烧录 / 擦除 / 导出 / 校验。

## 操作清单

| 操作 | 说明 |
|------|------|
| 读取 / 导出 ROM | 整片回读（dump）到文件 |
| 烧录 / 写入 ROM | 解锁 PPB → 擦除 → 逐包写入（每包 ACK）→ 校验修复 |
| 擦除 ROM | 整片擦除，或逐扇区擦除 |
| 校验 ROM | 回读与源文件逐字节比对 |

## 对应命令（规划）

```
cfb gba rom read   --port COM7 --out dump.gba [--size 16M]
cfb gba rom write  --port COM7 --rom game.gba [--chip-erase] [--no-ppb] [--no-verify]
cfb gba rom erase  --port COM7 [--sector 0x000000]   # 不带 --sector 为整片
cfb gba rom verify --port COM7 --ref game.gba
```

## 实现状态：🟡 部分实现

| 操作 | 状态 | 后端 API |
|------|------|----------|
| 烧录 / 写入 | ✅ 已实现并实测 | `Core.GbaFlasher.Burn(rom, length, BurnOptions)` |
| 擦除 | ✅ 已实现 | `Core.GbaFlasher.EraseChip` / `EraseSector` / `UnlockAllPpb` |
| 读取 / 导出 | 🟡 仅底层 | `Core.CartLink.RomRead`（分块读）已具备；缺"整片 dump 到文件"高层封装 |
| 校验 | 🟡 内嵌 | `Core.GbaFlasher.FindBadSectors` + `Burn` 的 `VerifyAfter`；缺独立 `verify` 命令 |

目前 CLI 只暴露了 `write`（根 README 的 `--rom` 流程）。

## 技术要点

- 写入两种粒度：单字 `RomWrite(wordAddr,…)` 与带写缓冲的 `RomProgram(byteAddr,…,bufferWriteBytes)`（更快）。
- 烧录逻辑：**每包必须 ACK 才前进，连续失败自动 `Reconnect()`**，最后逐扇区校验修复。
- `BurnOptions`：`ChipErase` / `UnlockPpb` / `VerifyAfter` / `ReconnectEveryFails` / `MaxTriesPerPacket` / `MaxRepairRounds`。
- `BurnResult` 精确报告 `FirstBadAddress` 与 `MismatchBytes`，不死等。
- ⚠️ 已知硬件现象：烧 16MB 时前 ~6–8MB 稳定，超过后设备可能持续编程后停止应答
  （实测停在 0x807000 附近，非固定坏块，偏供电/时序/接触）。见 [../../../Core/README.md](../../../Core/README.md)。
