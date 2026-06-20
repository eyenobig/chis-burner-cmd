# 功能 2 · GBA

针对 **GBA 卡带**的内容读写。GBA 卡带是**线性地址空间**：ROM 在 NOR Flash 上（无 mapper），
存档在独立的 SRAM / Flash / EEPROM 上。两类操作分到两个子模块：

| 子模块 | 内容 | 链接 |
|--------|------|------|
| ROM | 读取 / 烧录 / 擦除 / 导出 / 校验 | [rom/](rom/README.md) |
| RAM（存档）| 写入 / 导出 / 校验 | [ram/](ram/README.md) |

## 共性

- 前置：先经[功能 1 · 识别烧录器](../device_detect/README.md)连上并上电（`Core.CartLink`）。
- ROM 与 RAM 是**不同的地址通道**，命令上分开：`cfburn gba rom …` / `cfburn gba ram …`。

## GBA 平台要点

- ROM：NOR Flash 线性寻址，实测为 S29GL256（32MB，128KB 扇区）。
- 存档：SRAM / Flash(512K·1M) / EEPROM(4K·64K) 三类，时序各异，需分别处理。
- ⚠️ 已知硬件现象：烧 16MB 时前 ~6–8MB 稳定，超过后可能停止应答（详见 rom 子模块）。
