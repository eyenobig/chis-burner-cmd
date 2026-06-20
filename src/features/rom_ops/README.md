# 功能 4 · ROM 通用操作

平台无关的 ROM 数据层：**写入 / 导出 / 校验**。功能 2（GBA）和功能 3（GB）的卡带
读写都建立在这套原语之上——它们负责"按平台寻址/切 bank"，本模块负责"实际搬字节 + 比对"。

## 操作清单

| 操作 | 说明 |
|------|------|
| 写入 ROM | 把 ROM 字节流编程进 Flash（擦除后逐包写，每包 ACK） |
| 导出 ROM | 从 Flash 回读字节流到文件（dump） |
| 校验 ROM | 回读并与源文件逐字节比对，报告不符字节数 / 首个坏地址 |

## 对应命令（规划）

```
cfburn rom write  --port COM7 --in game.bin  --addr 0x0 [--no-verify]
cfburn rom export --port COM7 --out dump.bin --addr 0x0 --size 16M
cfburn rom verify --port COM7 --ref game.bin --addr 0x0
```

## 实现状态：🟡 部分实现

| 操作 | 状态 | 后端 API |
|------|------|----------|
| 写入 | ✅ 原语就绪 | `Core.CartLink.RomWrite` / `RomProgram`（带写缓冲）、`GbaFlasher.Burn` 封装了完整流程 |
| 导出 | 🟡 仅底层 | `Core.CartLink.RomRead`（分块回读）；缺"读满整片到文件"的高层命令 |
| 校验 | 🟡 内嵌 | `Core.GbaFlasher.FindBadSectors` + `Burn` 的 `VerifyAfter`；缺独立的 `verify` 命令 |

## 技术要点

- 写入有两种粒度：单字 `RomWrite(wordAddr,…)` 与带写缓冲的 `RomProgram(byteAddr,…,bufferWriteBytes)`（更快）。
- 校验策略：逐扇区回读比对，`FindBadSectors` 返回坏扇区集合用于定点修复，避免整片重烧。
- `BurnResult.FirstBadAddress` / `MismatchBytes` 是校验结果的标准出口。
- 建议把"整片 dump"和"独立 verify"抽成本模块的公共方法，供功能 2 / 3 复用。
