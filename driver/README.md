# 给烧录器一个名称标识

默认插上烧录器时，Windows 显示的是通用名 **"USB 串行设备 (COMx)"**。要让它一插上就显示
专属名称（如 **"ChisBurner (COMx)"**），有两条路线：

## 路线 A：装一个 INF（PC 端，不改硬件）

本目录的 [chis-burner.inf](chis-burner.inf) 把 VID_0483/PID_0721 这台设备改名，仍用系统
自带的 `usbser.sys` 驱动，不替换驱动本身。

安装（需管理员）：

```powershell
# 方式1: 设备管理器 -> 右键设备 -> 更新驱动 -> 浏览本文件夹
# 方式2: 管理员 PowerShell
pnputil /add-driver chis-burner.inf /install
```

- 装完后，设备管理器和 `cfb detect` 的"说明"列都会显示 `ChisBurner (COMx)`。
- ⚠️ Win11 对未签名 INF 会弹警告，选"仍然安装"。
- 想显示**中文名**：改 INF 里 `[Strings]` 的 `DESCRIPTION`，并把文件另存为
  **Unicode (UTF-16 LE)** 编码，否则中文乱码。

## 路线 B：改固件 USB 描述符（最干净，需重刷 MCU）

在 STM32 固件的 USB CDC 描述符里设置 **iProduct 字符串**（产品名）。这样设备**无需装任何
INF**，Windows 会直接用这个名字。属于 `mcu` 固件工程的改动，不在本 CLI 仓库内。

## 说明：cfb 本身不依赖系统显示名

`cfb` 通过 **USB VID/PID（0483/0721）** 识别烧录器（常量见 [../src/cartridge_link.rs](../src/cartridge_link.rs)，枚举/detect 见 [../src/device/](../src/device/)），
所以 `cfb detect` / `cfb info` / `cfb burn` **无需端口、无需改名**就能自动找到设备。
上面两条路线只是让**人在设备管理器里**看得更清楚，属于锦上添花。
