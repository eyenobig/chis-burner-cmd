# 功能 3 · GB / GBC 卡带

针对 **GB / GBC 卡带**的内容读写。与 GBA 的关键区别：GB 卡带通过 **MBC mapper**
（MBC1 / MBC2 / MBC3 / MBC5 等）做 **bank 切换**，ROM 和存档 RAM 都不是线性地址空间。

| 子模块 | 内容 | 链接 |
|--------|------|------|
| ROM | 读取 / 烧录 / 擦除 / 导出 / 校验 | [rom/](rom/README.md) |
| RAM（电池存档）| 写入 / 导出 / 校验 | [ram/](ram/README.md) |

## 共性

- 前置：先经[功能 1 · 识别烧录器](../device_detect/README.md)连上（`Core.CartLink` 协议层与平台无关，可复用）。
- 一切操作都要先**探测 MBC 类型**（读卡带头 `0x0147`），再据此切 bank。
- 命令分开：`cfburn gb rom …` / `cfburn gb ram …`。

## 实现状态：🔴 待移植

当前 `Core` 只有 GBA 线性 Flash 引擎，**没有 GB/MBC 的 bank 切换逻辑**。

> 原 WinForms 上位机里曾有 MBC 相关代码（`mission_mbc5.cs`、MBC3 RTC 窗体等），
> 精简成命令行版时只保留了 GBA 引擎。GB 支持需从那部分移植 / 重写为 `Core` 组件。

## GB 平台要点

- 卡带头：`0x0134` 标题、`0x0147` 卡带类型(含 MBC)、`0x0148` ROM 大小、`0x0149` RAM 大小。
- MBC5 支持到 8MB ROM；MBC3 带 RTC（实时钟）。
- bank 切换：写 mapper 寄存器选 bank → 读/写当前 bank 窗口。
