# 功能 1 · 识别烧录器

检测并连接碳酸丐烧录器，读出设备信息，作为其余所有功能的前置步骤。

## 操作清单

| 操作 | 说明 |
|------|------|
| 列出串口 | 枚举系统所有 COM 口 |
| 识别烧录器 | 按 **USB VID 0483 / PID 0721** 定位本烧录器所在的 COM 口 |
| 读设备信息 | 上电后读 Flash 芯片 **ID + CFI**（容量 / 写缓冲 / 扇区大小） |

## 对应命令（规划）

```
cfburn detect              # 列出所有串口并标出烧录器
cfburn info --port COM7    # 连接并打印芯片 ID + 容量
```

## 实现状态：✅ 已实现

| 能力 | 后端 API |
|------|----------|
| 串口枚举 | `System.IO.Ports.SerialPort.GetPortNames()`（CLI 侧） |
| 打开/上电/热身 | `Core.CartLink.Open / PowerOn3v3 / WarmUp` |
| 读芯片 ID | `Core.CartLink.RomReadId` |
| 读 ID + CFI | `Core.GbaFlasher.ReadInfo` → `FlashInfo` |

实测：COM7 上读到 ID `01 00 7E 22 22 22 01 22`，容量 32MB（S29GL256），写缓冲 32B，扇区 128KB×256。

## 技术要点

- VID 0483 = STMicroelectronics（USB CDC 虚拟串口）。
- `WarmUp()` 用来吸收"上电/复位后第一条命令被吞"的固件怪癖。
- CFI 必须一次性连续读，分散读会掉出 CFI 模式。
- 端口被占用会报 `UnauthorizedAccessException`；可改 COM 号或重插设备释放句柄。
