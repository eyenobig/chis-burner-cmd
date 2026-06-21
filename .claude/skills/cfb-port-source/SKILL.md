---
name: cfb-port-source
description: cfb（Rust 烧录器命令行）的复刻参考源。要实现/移植任何串口协议、GBA/MBC 烧录、RTC 等功能时必读——权威参考是外部 C# 工程 Z:\Project\beggar_socket\client（ChisFlashBurner 上位机），不是本仓库里精简的 csharp/。
---

# cfb 的复刻参考源（移植从哪读）

cfb（本仓库的 Rust 实现）的功能是**从 C# 上位机复刻**而来。实现 `cartridge_link` / `rom/gba` /
`rom/mbc` / RTC 等时，**以下面这个外部工程为权威参考**，按其行为/协议用地道 Rust 重写。

## 权威参考：`Z:\Project\beggar_socket\client`

`ChisFlashBurner.sln`，Windows Forms 上位机，.NET Framework 4.8，无 NuGet 依赖。
**只读参考，不要去 build 它**（需完整版 MSBuild，`dotnet build` 会失败，见其 `BUILD.md`）。

源文件 → Rust 落点映射：

| C# 源（beggar_socket/client） | 行数 | 作用 | Rust 落点 |
|------|------|------|-----------|
| `ChisFlashBurner.Core/CartLink.cs` | ~227 | 串口协议层（USB CDC，收发/超时/重连/上电/复位） | `src/cartridge_link.rs` |
| `ChisFlashBurner.Core/GbaFlasher.cs` | ~308 | GBA flash 引擎（CFI/ID、整片/逐扇区擦除、PPB 解锁、健壮编程、校验修复） | `src/rom/gba/` |
| `ChisFlashBurner/mission_gba.cs` | ~1387 | GBA 高层任务编排（ROM/RAM/RTC 流程） | `src/rom/gba/` |
| `ChisFlashBurner/mission_mbc5.cs` | ~1208 | **MBC5 (GB/GBC) 任务**（chis 这边原本空缺，以此为准） | `src/rom/mbc/` |
| `ChisFlashBurner/cart_adapter.cs` | ~510 | 卡带适配/硬件抽象 | `src/rom/`（或新建 `cart_adapter`） |
| `ChisFlashBurner/mission_tools.cs` | ~694 | 任务共用工具 | 按需拆入 `rom/*` 或公共模块 |
| `ChisFlashBurner/Form_gba_rtc.cs` / `Form_mbc3_rtc.cs` | — | RTC（实时时钟）功能 | 后续 RTC 模块 |
| `ChisFlashBurner.Cli/Program.cs` | ~146 | 命令行参考（参数/子命令风格） | `src/main.rs` |
| `ChisFlashBurner/Form1.cs` 等 Form* | — | WinForms GUI，**不复刻**（cfb 是 CLI） | — |

> 注意 USB 标识仍是 VID `0483` / PID `0721`（STM32 CDC），与本仓库 `cartridge_link.rs` 常量一致。

## 关于本仓库内的 `csharp/`

`chis-burner-cmd/csharp/` 是早期从上位机**精简出的部分副本**（只有 detect/info/gba-rom），
现已被 `beggar_socket/client` 这个更全的原始工程取代为参考源。移植**优先看 beggar_socket/client**；
`csharp/` 若已删除则忽略。

## 移植做法

1. 读懂对应 C# 源的协议/时序/边界（尤其 CartLink 的“被吞首包”“卡死重连”这类怪癖必须照搬）。
2. 用地道 Rust 重写，不照抄结构；数据类型进 `device/data` 或各 `rom/*`，函数进对应模块。
3. 输出遵循 [cfb-output](../cfb-output/SKILL.md)：双模式（人类 + `--json` NDJSON 事件），
   新事件先登记；所有用户文案进 `src/i18n/*.json`（zh-CN + en）。
4. 端口解析用 `device::resolve_port`；诊断走 stderr，不污染 `--json` 流。
5. 插上烧录器（VID 0483/PID 0721）实测每一步，协议时序错了不会报错只会数据不对。
