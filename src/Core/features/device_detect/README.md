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
| 解析 VID/PID | WMI `Win32_PnPEntity` → [DeviceScan.cs](DeviceScan.cs) |
| 识别烧录器 | 比对 `Core.CartLink.UsbVid` / `UsbPid`（0483/0721）|
| 占用检测 | 试开串口判断是否被占用 |
| 打开/上电/热身 | `Core.CartLink.Open / PowerOn3v3 / WarmUp` |
| 读 ID + CFI | `Core.GbaFlasher.ReadInfo` → `FlashInfo` |

实测输出：
```
COM7   0483:0721   是   USB 串行设备 (COM7)  <= 烧录器
芯片 ID: 01 00 7E 22 22 22 01 22   容量: 32 MB   写缓冲: 32B   扇区: 128KB x 256
```

## 自动识别（无需 --port）

所有命令省略 `--port` 时都会自动按 VID/PID 选用烧录器：`cfb info`、`cfb burn --rom x.gba`
直接就能跑。只有同时插多台、或想指定某口时才需要 `--port`。

## 命名标识（可选）

默认设备显示为通用名"USB 串行设备 (COMx)"。要让它插上就显示专属名（如 `ChisBurner (COMx)`），
见 [../../../../driver/](../../../../driver/README.md)：装个 INF（PC 端）或改固件 iProduct 描述符。
注意：`cfb` 靠 VID/PID 识别，**不依赖**这个显示名。

## 技术要点

- VID 0483 = STMicroelectronics（USB CDC 虚拟串口）。
- `WarmUp()` 用来吸收"上电/复位后第一条命令被吞"的固件怪癖。
- CFI 必须一次性连续读，分散读会掉出 CFI 模式。
- 端口被占用会报 `UnauthorizedAccessException`；可改 COM 号或重插设备释放句柄。
