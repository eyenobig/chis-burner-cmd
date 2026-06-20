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
│  └─ Cli/           控制台前端，编译输出 cfburn.exe
│     └─ Program.cs
├─ chis-burner-cmd.sln
└─ README.md
```

目标框架：.NET Framework **4.8**。无 NuGet 依赖，仅引用框架自带程序集。

## 构建

无 WinForms / resx，可直接用 SDK 工具链：

```powershell
dotnet build chis-burner-cmd.sln -c Release
# 输出: src/Cli/bin/Release/net48/cfburn.exe
```

或用 Visual Studio / 完整版 MSBuild 打开 `chis-burner-cmd.sln` 构建。

## 用法

```
cfburn --port COM7 --rom <file.gba> [选项]

选项:
  --port <COMx>     串口 (默认 COM7)
  --rom  <path>     要烧录的 GBA ROM
  --log  <path>     日志文件 (默认 cfburn_<时间>.log)
  --chip-erase      整片擦除模式 (默认逐扇区即擦即写)
  --no-ppb          跳过 PPB 解锁
  --no-verify       跳过校验+修复
  -h, --help        显示帮助
```
