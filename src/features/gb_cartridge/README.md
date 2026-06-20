# 功能 3 · GB / GBC 卡带内容读写

针对 **GB / GBC 卡带**的 ROM 读取 / 烧录 / 擦除。与 GBA 的关键区别：GB 卡带通过
**MBC mapper**（MBC1 / MBC2 / MBC3 / MBC5 等）做 ROM bank 切换，不是线性地址空间。

## 操作清单

| 操作 | 说明 |
|------|------|
| 读取 ROM | 按 bank 逐段切换并回读，拼成完整 ROM dump |
| 烧录 ROM | 擦除 → 按 bank 写入 → 校验 |
| 擦除 ROM | 整片 / 按区擦除 |

## 对应命令（规划）

```
cfburn gb read  --port COM7 --out dump.gb
cfburn gb write --port COM7 --rom game.gb [--mbc auto|mbc5|mbc3|mbc1]
cfburn gb erase --port COM7
```

## 实现状态：🔴 待实现（需移植）

当前 `Core` 只有 GBA 线性 Flash 引擎，**没有** GB/MBC 的 bank 切换逻辑。

> 原 WinForms 上位机里曾有 MBC 相关代码（`mission_mbc5.cs`、MBC3 RTC 窗体等），
> 但精简成命令行版时只保留了 GBA 引擎。GB 支持需要从那部分移植 / 重写为 `Core` 组件。

### 待办

- [ ] `Core` 新增 `GbCartridge` / `MbcFlasher`：mapper 探测（读卡带头 0x0147~0x0149）
- [ ] bank 切换协议（写 mapper 寄存器 → 切 ROM bank → 读/写当前 bank）
- [ ] ROM 大小 / RAM 大小解析（卡带头 0x0148 / 0x0149）
- [ ] 接入 RAM 读写见 [功能 5 · ram_ops](../ram_ops/README.md)

## 技术要点

- 卡带头：`0x0134` 标题、`0x0147` 卡带类型(含 MBC)、`0x0148` ROM 大小、`0x0149` RAM 大小。
- MBC5 支持到 8MB ROM；MBC3 带 RTC（实时钟）。
- 复用功能 1 的连接层 `Core.CartLink`（串口协议本身与平台无关）。
