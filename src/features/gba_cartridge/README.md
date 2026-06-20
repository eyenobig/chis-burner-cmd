# 功能 2 · GBA 卡带内容读写

针对 **GBA 卡带**（NOR Flash，无 mapper，线性地址空间）的 ROM 读取 / 烧录 / 擦除。

## 操作清单

| 操作 | 说明 |
|------|------|
| 读取 ROM | 把卡带 ROM 整片回读（dump）到文件 |
| 烧录 ROM | 解锁 PPB → 擦除 → 逐包写入 → 校验修复 |
| 擦除 ROM | 整片擦除，或逐扇区擦除 |

## 对应命令（规划）

```
cfburn gba read  --port COM7 --out dump.gba [--size 16M]
cfburn gba write --port COM7 --rom game.gba [--chip-erase] [--no-ppb] [--no-verify]
cfburn gba erase --port COM7 [--sector 0x000000]   # 不带 --sector 为整片
```

## 实现状态：🟡 部分实现

| 操作 | 状态 | 后端 API |
|------|------|----------|
| 烧录 ROM | ✅ 已实现并实测 | `Core.GbaFlasher.Burn(rom, length, BurnOptions)` |
| 擦除 ROM | ✅ 已实现 | `Core.GbaFlasher.EraseChip` / `EraseSector` / `UnlockAllPpb` |
| 读取 ROM | 🟡 仅底层 | `Core.CartLink.RomRead`（分块读）已具备；缺"整片 dump 到文件"的高层封装 |

目前 CLI 只暴露了 `write`（即根 README 的 `--rom` 流程）。

## 技术要点

- 烧录逻辑：**每包必须 ACK 才前进，连续失败自动 `Reconnect()`**，最后逐扇区校验修复。
- `BurnOptions`：`ChipErase`（整片/逐扇区）、`UnlockPpb`、`VerifyAfter`、`ReconnectEveryFails`、`MaxTriesPerPacket`、`MaxRepairRounds`。
- `BurnResult` 会精确报告 `FirstBadAddress` 与 `MismatchBytes`，不再死等。
- ⚠️ 已知硬件现象：烧 16MB 时前 ~6–8MB 稳定，超过后设备可能持续编程后停止应答
  （实测停在 0x807000 附近，非固定坏块，偏供电/时序/接触）。见 [../../Core/README.md](../../Core/README.md)。
- 底层写入的实际原语是通用 ROM 操作，见 [功能 4 · rom_ops](../rom_ops/README.md)。
