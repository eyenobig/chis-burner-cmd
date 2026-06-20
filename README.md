# chis-burner-cmd

碳酸丐烧录器的**命令行版**，从原 WinForms 上位机里精简而来。
只保留协议/烧录引擎与 CLI，去掉了图形界面。

## 目录结构

```
chis-burner-cmd/
├─ src/
│  ├─ Core/          协议 + 烧录引擎（类库，输出 Core.dll）
│  │  ├─ CartLink.cs     串口协议层（带超时 + 自动重连）
│  │  ├─ GbaFlasher.cs   高层烧录操作
│  │  └─ README.md       引擎说明
│  ├─ Cli/           控制台前端，编译输出 cfb.exe
│  │  └─ Program.cs
│  └─ features/      按功能拆分的模块 + 各自 README（见下）
├─ chis-burner-cmd.sln
└─ README.md
```

## 功能模块

详见 [src/features/README.md](src/features/README.md)。ROM/RAM 操作按平台归入各自卡带模块。

| # | 功能 | 子模块 | 状态 |
|---|------|--------|------|
| 1 | [识别烧录器](src/features/device_detect/README.md) | — | ✅ 已实现 |
| 2 | [GBA](src/features/gba/README.md) | [rom](src/features/gba/rom/README.md) / [ram](src/features/gba/ram/README.md) | 🟡 / 🔴 |
| 3 | [MBC (GB/GBC)](src/features/mbc/README.md) | [rom](src/features/mbc/rom/README.md) / [ram](src/features/mbc/ram/README.md) | 🔴 / 🔴 |

目标框架：.NET Framework **4.8**。无 NuGet 依赖，仅引用框架自带程序集。

## 构建

无 WinForms / resx，可直接用 SDK 工具链：

```powershell
dotnet build chis-burner-cmd.sln -c Release
# 输出: src/Cli/bin/Release/net48/cfb.exe
```

或用 Visual Studio / 完整版 MSBuild 打开 `chis-burner-cmd.sln` 构建。

## 用法

子命令风格（类似 adb）：

```
cfb <命令> [选项]

命令:
  detect                       列出所有串口并标出烧录器
  info  [--port COMx]          连接并读取芯片 ID + 容量 (省略 --port 自动选烧录器)
  burn  --port COMx --rom <f>  烧录 GBA ROM
  help  [命令]                 显示帮助 (如 `cfb help burn`)
```

burn 选项：`--port` `--rom` `--log` `--chip-erase` `--no-ppb` `--no-verify`
（详见 `cfb help burn`）。旧用法 `cfb --port COM7 --rom x.gba` 仍兼容。
