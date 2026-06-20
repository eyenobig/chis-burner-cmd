using System;
using System.Diagnostics;
using System.IO;
using Core;

namespace Cli
{
    /// <summary>
    /// cfb 命令行工具。子命令:
    ///   cfb detect                       列出串口并识别烧录器
    ///   cfb info  --port COM7            连接并读取芯片 ID + 容量
    ///   cfb burn  --rom x.gba [--port COM7] [--chip-erase] [--no-ppb] [--no-verify]
    ///   cfb help  [命令]                 显示帮助
    /// 兼容旧用法: cfb --port COM7 --rom x.gba ...
    /// </summary>
    internal static class Program
    {
        private static StreamWriter _logFile;

        private static void Line(string s)
        {
            string stamped = "[" + DateTime.Now.ToString("HH:mm:ss.fff") + "] " + s;
            Console.WriteLine(stamped);
            try { _logFile?.WriteLine(stamped); _logFile?.Flush(); } catch { }
        }

        private static int Main(string[] args)
        {
            if (args.Length == 0) { PrintUsage(); return 0; }

            string cmd = args[0];
            switch (cmd)
            {
                case "detect":
                case "devices":
                    return RunDetect();
                case "info":
                    return RunInfo(args);
                case "burn":
                case "write":
                    return RunBurn(Slice(args, 1));
                case "help":
                case "-h":
                case "--help":
                    return RunHelp(args);
                default:
                    // 向后兼容: cfb --port .. --rom ..
                    if (cmd.StartsWith("--")) return RunBurn(args);
                    Console.Error.WriteLine("未知命令: " + cmd + "\n");
                    PrintUsage();
                    return 2;
            }
        }

        // ---- help ----

        private static int RunHelp(string[] args)
        {
            // help / -h / --help 后面可带一个具体命令名
            string topic = args.Length > 1 ? args[1] : null;
            switch (topic)
            {
                case "detect":
                case "devices":
                    Console.WriteLine(@"cfb detect — 列出所有串口并识别烧录器

  枚举系统串口, 通过 USB VID/PID (" + CartLink.UsbVid + "/" + CartLink.UsbPid + @") 标出烧录器,
  并显示每个端口当前是否可打开(占用检测)。无需参数。

  示例: cfb detect");
                    return 0;
                case "info":
                    Console.WriteLine(@"cfb info [--port COMx] — 读取芯片信息

  连接烧录器, 上电后读取 Flash 芯片 ID 与 CFI(容量/写缓冲/扇区)。
  省略 --port 时自动选择第一个识别到的烧录器。

  示例: cfb info
        cfb info --port COM7");
                    return 0;
                case "burn":
                case "write":
                    Console.WriteLine(@"cfb burn --rom <file.gba> [选项] — 烧录 GBA ROM

  --rom  <path>     要烧录的 GBA ROM
  --port <COMx>     串口 (省略则自动识别烧录器)
  --log  <path>     日志文件 (默认 cfb_<时间>.log)
  --chip-erase      整片擦除模式 (默认逐扇区即擦即写)
  --no-ppb          跳过 PPB 解锁
  --no-verify       跳过校验+修复

  示例: cfb burn --rom game.gba          (自动选端口)
        cfb burn --port COM7 --rom game.gba");
                    return 0;
                case null:
                    PrintUsage();
                    return 0;
                default:
                    Console.Error.WriteLine("没有该命令的帮助: " + topic + "\n");
                    PrintUsage();
                    return 2;
            }
        }

        // ---- 功能 1: 识别烧录器 ----

        private static int RunDetect()
        {
            var list = DeviceScan.Enumerate();
            Console.WriteLine($"发现 {list.Count} 个串口:\n");
            Console.WriteLine("  端口     VID:PID      可打开   说明");
            Console.WriteLine("  -------  -----------  -------  --------------------------------");
            int burners = 0;
            foreach (var p in list)
            {
                string vidpid = p.Vid != null ? $"{p.Vid}:{p.Pid}" : "-";
                string open = DeviceScan.CanOpen(p.Port) ? "是" : "否(占用)";
                string tag = p.IsBurner ? "  <= 烧录器" : "";
                if (p.IsBurner) burners++;
                Console.WriteLine($"  {p.Port,-7}  {vidpid,-11}  {open,-7}  {p.Name}{tag}");
            }
            Console.WriteLine();
            if (burners == 0)
            {
                Console.WriteLine($"未发现烧录器 (期望 USB VID {CartLink.UsbVid} / PID {CartLink.UsbPid})。");
                return 1;
            }
            Console.WriteLine($"找到 {burners} 个烧录器。可直接运行 `cfb info` 或 `cfb burn --rom <f>`，会自动选用，无需 --port。");
            return 0;
        }

        // ---- 功能 1: 读设备信息 ----

        private static int RunInfo(string[] args)
        {
            string port = null;
            for (int i = 1; i < args.Length; i++)
            {
                switch (args[i])
                {
                    case "--port": port = Next(args, ref i); break;
                    case "-h":
                    case "--help":
                        Console.WriteLine("用法: cfb info [--port COMx]  (省略 --port 时自动识别烧录器)");
                        return 0;
                    default:
                        Console.Error.WriteLine("未知参数: " + args[i]); return 2;
                }
            }

            port = ResolvePort(port, Console.WriteLine);
            if (port == null) return 2;

            try
            {
                using (var link = new CartLink(port) { Log = s => Console.WriteLine("  [link] " + s) })
                {
                    link.Open();
                    link.PowerOn3v3();
                    link.WarmUp();

                    var info = new GbaFlasher(link).ReadInfo();
                    Console.WriteLine($"端口:    {port}");
                    Console.WriteLine($"芯片 ID: {info.IdHex}");
                    Console.WriteLine($"容量:    {info.DeviceSize:N0} 字节 ({info.DeviceSize / 1024 / 1024} MB)");
                    Console.WriteLine($"写缓冲:  {info.BufferWriteBytes} 字节");
                    Console.WriteLine($"扇区:    {info.SectorSize:N0} 字节 x {info.SectorCount}");

                    link.PowerOff();
                    return 0;
                }
            }
            catch (Exception e)
            {
                Console.Error.WriteLine("读取失败: " + e.Message);
                return 3;
            }
        }

        // ---- 功能 2: 烧录 GBA ROM ----

        private static int RunBurn(string[] args)
        {
            string port = null;
            string rom = null;
            string logPath = null;
            bool chipErase = false;
            bool ppb = true;
            bool verify = true;

            for (int i = 0; i < args.Length; i++)
            {
                switch (args[i])
                {
                    case "--port": port = Next(args, ref i); break;
                    case "--rom": rom = Next(args, ref i); break;
                    case "--log": logPath = Next(args, ref i); break;
                    case "--chip-erase": chipErase = true; break;
                    case "--no-ppb": ppb = false; break;
                    case "--no-verify": verify = false; break;
                    case "-h":
                    case "--help": PrintUsage(); return 0;
                    default: Console.Error.WriteLine("未知参数: " + args[i]); PrintUsage(); return 2;
                }
            }

            if (string.IsNullOrEmpty(rom)) { Console.Error.WriteLine("缺少 --rom"); PrintUsage(); return 2; }
            if (!File.Exists(rom)) { Console.Error.WriteLine("找不到 ROM: " + rom); return 2; }

            port = ResolvePort(port, Console.WriteLine);
            if (port == null) return 2;

            if (string.IsNullOrEmpty(logPath))
                logPath = Path.Combine(Environment.CurrentDirectory,
                    "cfb_" + DateTime.Now.ToString("yyyyMMdd_HHmmss") + ".log");

            try { _logFile = new StreamWriter(logPath, false) { AutoFlush = true }; }
            catch (Exception e) { Console.Error.WriteLine("无法写日志: " + e.Message); }

            byte[] data = File.ReadAllBytes(rom);
            Line($"==== cfb burn ====");
            Line($"端口={port}  ROM={rom}  大小={data.Length:N0} 字节");
            Line($"选项: chipErase={chipErase} unlockPPB={ppb} verify={verify}");
            Line($"日志: {logPath}");

            var sw = Stopwatch.StartNew();
            int exit = 1;
            try
            {
                using (var link = new CartLink(port) { Log = s => Line("  [link] " + s) })
                {
                    link.Open();
                    link.PowerOn3v3();
                    link.WarmUp();

                    var flasher = new GbaFlasher(link) { Log = Line };
                    long lastMb = -1;
                    flasher.Progress = (done, total) =>
                    {
                        long mb = done / (1024 * 1024);
                        if (mb != lastMb) { lastMb = mb; Line($"  进度 {mb} / {total / (1024 * 1024)} MB"); }
                    };

                    // 启动前先报告设备信息
                    try
                    {
                        var info = flasher.ReadInfo();
                        Line($"设备 ID: {info.IdHex}");
                        Line($"容量: {info.DeviceSize:N0} 字节  写缓冲: {info.BufferWriteBytes}  扇区: {info.SectorSize} x {info.SectorCount}");
                    }
                    catch (Exception e) { Line("读取设备信息失败: " + e.Message); }

                    var opt = new BurnOptions
                    {
                        ChipErase = chipErase,
                        UnlockPpb = ppb,
                        VerifyAfter = verify,
                    };

                    var res = flasher.Burn(data, data.Length, opt);

                    Line("---- 结果 ----");
                    Line($"成功: {res.Success}");
                    Line($"已写入: {res.BytesWritten:N0} 字节");
                    Line($"重连次数: {res.Reconnects}  应答冲刷: {res.AckFlushes}");
                    if (res.FirstBadAddress >= 0) Line($"停止地址: 0x{res.FirstBadAddress:X8}");
                    Line($"校验不符: {res.MismatchBytes} 字节");
                    Line($"用时: {res.Seconds:f0}s");

                    exit = res.Success ? 0 : 1;
                }
            }
            catch (Exception e)
            {
                Line("致命错误: " + e);
                exit = 3;
            }

            sw.Stop();
            Line($"==== 结束 (exit={exit}, 总耗时 {sw.Elapsed.TotalSeconds:f0}s) ====");
            _logFile?.Dispose();
            return exit;
        }

        // ---- 工具 ----

        private static string Next(string[] a, ref int i)
        {
            if (i + 1 >= a.Length) throw new ArgumentException("参数缺少值: " + a[i]);
            return a[++i];
        }

        /// <summary>指定了 --port 就用它；否则自动识别烧录器。返回 null 表示找不到。</summary>
        private static string ResolvePort(string port, Action<string> say)
        {
            if (!string.IsNullOrEmpty(port)) return port;
            string p = DeviceScan.FirstBurner();
            if (p == null)
            {
                Console.Error.WriteLine("未指定 --port 且未自动发现烧录器 (期望 USB VID "
                    + CartLink.UsbVid + " / PID " + CartLink.UsbPid + ")。先运行 `cfb detect` 查看。");
                return null;
            }
            say("自动识别到烧录器: " + p);
            return p;
        }

        private static string[] Slice(string[] a, int start)
        {
            var r = new string[a.Length - start];
            Array.Copy(a, start, r, 0, r.Length);
            return r;
        }

        private static void PrintUsage()
        {
            Console.WriteLine(@"cfb — 碳酸丐烧录器命令行工具

用法: cfb <命令> [选项]

命令:
  detect                       列出所有串口并标出烧录器
  info  [--port COMx]          连接并读取芯片 ID + 容量 (省略 --port 自动识别烧录器)
  burn  --rom <f> [--port]     烧录 GBA ROM (省略 --port 自动识别烧录器)
  help  [命令]                 显示帮助 (如 `cfb help burn` 看 burn 详细选项)

burn 选项:
  --rom  <path>     要烧录的 GBA ROM
  --port <COMx>     串口 (省略则自动识别烧录器)
  --log  <path>     日志文件 (默认 cfb_<时间>.log)
  --chip-erase      整片擦除模式 (默认逐扇区即擦即写)
  --no-ppb          跳过 PPB 解锁
  --no-verify       跳过校验+修复

用 `cfb help <命令>` 查看单个命令的详细帮助。");
        }
    }
}
