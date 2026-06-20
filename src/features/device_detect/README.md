# 功能 1 · 识别烧录器

检测并连接碳酸丐烧录器，读出设备信息，作为其余所有功能的前置步骤。

## 操作清单

| 操作 | 说明 |
|------|------|
| 列出串口 | 枚举系统所有 COM 口 |
| 识别烧录器 | 按 **USB VID 0483 / PID 0721** 定位本烧录器所在的 COM 口 |
| 读设备信息 | 上电后读 Flash 芯片 **ID + CFI**（容量 / 写缓冲 / 扇区大小） |

## 命令（✅ 已实现）

```
cfb detect              # 列出所有串口, 标出烧录器, 并显示端口是否被占用
cfb info [--port COM7]  # 连接并打印芯片 ID + 容量 (省略 --port 自动选烧录器)
```

## 实现状态：✅ 已实现并实测

| 能力 | 后端 API / 实现 |
|------|----------------|
| 串口枚举 | `System.IO.Ports.SerialPort.GetPortNames()` |
| 解析 VID/PID | WMI `Win32_PnPEntity` → [Cli/DeviceScan.cs](../../Cli/DeviceScan.cs) |
| 识别烧录器 | 比对 `Core.CartLink.UsbVid` / `UsbPid`（0483/0721）|
| 占用检测 | 试开串口判断是否被占用 |
| 打开/上电/热身 | `Core.CartLink.Open / PowerOn3v3 / WarmUp` |
| 读 ID + CFI | `Core.GbaFlasher.ReadInfo` → `FlashInfo` |

实测输出：
```
COM7   0483:0721   是   USB 串行设备 (COM7)  <= 烧录器
芯片 ID: 01 00 7E 22 22 22 01 22   容量: 32 MB   写缓冲: 32B   扇区: 128KB x 256
```

## 技术要点

- VID 0483 = STMicroelectronics（USB CDC 虚拟串口）。
- `WarmUp()` 用来吸收"上电/复位后第一条命令被吞"的固件怪癖。
- CFI 必须一次性连续读，分散读会掉出 CFI 模式。
- 端口被占用会报 `UnauthorizedAccessException`；可改 COM 号或重插设备释放句柄。
