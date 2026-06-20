# Core

碳酸丐烧录器的**协议 + 烧录引擎**，从上位机里抽出来的独立 DLL（.NET Framework 4.8）。
由 `Cli`（cfburn 命令行工具）引用；与具体前端（曾经的 WinForms 上位机）无关。

## 为什么独立成 DLL

原上位机的串口逻辑有两个老问题，会导致烧大 ROM 时"断在中间"：

1. `getRespon()` 用 `while (port.BytesToRead == 0) ;` **无限忙等**，一旦某次 ACK 丢失就永久卡死。
2. 无超时、无重试、无 MCU 复活机制。

本库把这套逻辑重写为**带超时 + 自动重连复活**的健壮实现，并做成可复用组件。

## 组成

- **`CartLink`** — 串口协议层（USB CDC，VID 0483/PID 0721）
  - 所有应答读取都有超时（`ResponseTimeoutMs`，默认 800ms），不再无限等待
  - `Reconnect()`：关/重开串口并重新上电，用于 MCU 持续编程后卡死的复活
  - `WarmUp()`：吸收"上电/复位后第一条命令被吞"的固件怪癖
  - 命令：`RomRead / RomWrite / RomProgram / RomReadId / RomEraseChip / PowerOn3v3`
- **`GbaFlasher`** — 高层操作
  - `ReadInfo()`：ID + CFI（容量、写缓冲、扇区大小）。CFI 一次性连续读，规避分散读掉出 CFI 模式的坑
  - `EraseChip() / EraseSector()`
  - `UnlockAllPpb()`：All PPB Erase，清除扇区持久保护
  - `Burn(rom, length, options)`：解锁PPB → 擦除 → **每包必须 ACK 才前进，连续失败自动重连** → 校验+逐扇区修复
  - `Log` / `Progress` 回调，UI 无关

## 用法示例

```csharp
using var link = new CartLink("COM7") { Log = Console.WriteLine };
link.Open();
link.PowerOn3v3();
link.WarmUp();

var flasher = new GbaFlasher(link) { Log = Console.WriteLine };
flasher.Progress = (done, total) => Console.WriteLine($"{done}/{total}");

var rom = File.ReadAllBytes("game.gba");
var result = flasher.Burn(rom, rom.Length, new BurnOptions {
    ChipErase = false,   // false = 逐扇区即擦即写（推荐，便于闯过卡死点）
    UnlockPpb = true,
    VerifyAfter = true,
});
Console.WriteLine(result.Success ? "OK" : $"停在 0x{result.FirstBadAddress:X8}");
```

## 在 cmd 工具中调用

控制台前端见同级的 `../Cli`（编译输出 `cfburn.exe`）：解析参数后调用
`GbaFlasher.Burn(...)` 即可。用法见仓库根 [README.md](../../README.md)。

## 已知硬件现象（实测 S29GL256 / 32MB）

烧 16MB ROM 时，**前 ~6–8MB 编程稳定可靠**；超过该范围设备会**持续编程后停止应答**
（在不同次复现于 0x618340 / 0x63F000 / 0x800000 附近，不是固定坏块）。本库会自动重连
并在重试无效时**精确报告停止地址**（而非像旧版那样卡死）。该现象偏硬件层面（疑似持续编程
时供电/时序，或地址线接触），建议：换用逐扇区模式、检查卡带接触/供电、或降低编程速率。
