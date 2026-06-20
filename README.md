# chis-burner-cmd

碳酸丐烧录器的**命令行版** `cfb`，从原 WinForms 上位机里精简而来：只保留协议/烧录引擎与
命令行，去掉了图形界面。单一项目，支持中英文语言包。

## 目录结构

```
chis-burner-cmd/
├─ src/Core/                单一项目，编译输出 cfb.exe
│  ├─ CartLink.cs             串口协议层（带超时 + 自动重连）
│  ├─ GbaFlasher.cs           GBA 烧录引擎
│  ├─ Program.cs              入口：解析 --lang、派发子命令
│  ├─ CliCommon.cs            参数解析 / 端口自动识别 / 运行日志
│  ├─ HelpCommand.cs          help 子命令
│  ├─ i18n/                   语言包：Lang.cs + zh-CN.json / en.json
│  └─ features/               按功能拆分的命令模块 + 各自 README（见下）
├─ driver/                  可选 INF：给设备一个友好名称
├─ chis-burner-cmd.sln
└─ README.md
```

每个命令实现都放在对应 feature 目录里（如 `detect`/`info` 在 `features/device_detect`，
`burn` 在 `features/gba/rom`），Program.cs 只做派发。

## 功能模块

详见 [src/Core/features/README.md](src/Core/features/README.md)。ROM/RAM 操作按平台归入各自模块。

| # | 功能 | 子模块 | 状态 |
|---|------|--------|------|
| 1 | [识别烧录器](src/Core/features/device_detect/README.md) | — | ✅ 已实现 |
| 2 | [GBA](src/Core/features/gba/README.md) | [rom](src/Core/features/gba/rom/README.md) / [ram](src/Core/features/gba/ram/README.md) | 🟡 / 🔴 |
| 3 | [MBC (GB/GBC)](src/Core/features/mbc/README.md) | [rom](src/Core/features/mbc/rom/README.md) / [ram](src/Core/features/mbc/ram/README.md) | 🔴 / 🔴 |

目标框架：.NET Framework **4.8**。仅引用框架自带程序集（System.Management 解析 USB VID/PID，
System.Web.Extensions 解析 JSON 语言包），无 NuGet 依赖。

## 构建

```powershell
dotnet build chis-burner-cmd.sln -c Release
# 输出: build/Release/cfb.exe
```

或用 Visual Studio / 完整版 MSBuild 打开 `chis-burner-cmd.sln` 构建。

## 用法

子命令风格（类似 adb），省略 `--port` 会自动识别烧录器：

```
cfb [--lang zh-CN|en] <命令> [选项]

命令:
  detect                       列出所有串口并标出烧录器
  info  [--port COMx]          连接并读取芯片 ID + 容量
  burn  --rom <f> [--port]     烧录 GBA ROM
  help  [命令]                 显示帮助 (如 `cfb help burn`)
```

burn 选项：`--rom` `--port` `--log` `--chip-erase` `--no-ppb` `--no-verify`
（详见 `cfb help burn`）。旧用法 `cfb --port COM7 --rom x.gba` 仍兼容。

## 语言

界面文案全部走语言包 [src/Core/i18n](src/Core/i18n)（`zh-CN.json` / `en.json`，嵌入程序集）。
默认跟随系统语言、回退中文；可用 `--lang en` 强制指定。加新语言只需加一个 `<lang>.json`
并在 csproj 里登记为 EmbeddedResource。
