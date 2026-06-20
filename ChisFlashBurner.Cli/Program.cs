using System;
using System.Diagnostics;
using System.IO;
using ChisFlashBurner.Core;

namespace ChisFlashBurner.Cli
{
    /// <summary>
    /// 命令行烧录工具：写入 GBA ROM + 校验 + 记录日志。
    /// 用法:
    ///   cfburn --port COM7 --rom game.gba [--log out.log]
    ///          [--chip-erase] [--no-ppb] [--no-verify]
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
            string port = "COM7";
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

            if (string.IsNullOrEmpty(logPath))
                logPath = Path.Combine(Environment.CurrentDirectory,
                    "cfburn_" + DateTime.Now.ToString("yyyyMMdd_HHmmss") + ".log");

            try { _logFile = new StreamWriter(logPath, false) { AutoFlush = true }; }
            catch (Exception e) { Console.Error.WriteLine("无法写日志: " + e.Message); }

            byte[] data = File.ReadAllBytes(rom);
            Line($"==== ChisFlashBurner CLI ====");
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

        private static string Next(string[] a, ref int i)
        {
            if (i + 1 >= a.Length) throw new ArgumentException("参数缺少值: " + a[i]);
            return a[++i];
        }

        private static void PrintUsage()
        {
            Console.WriteLine(@"用法:
  cfburn --port COM7 --rom <file.gba> [选项]

选项:
  --port <COMx>     串口 (默认 COM7)
  --rom  <path>     要烧录的 GBA ROM
  --log  <path>     日志文件 (默认 cfburn_<时间>.log)
  --chip-erase      整片擦除模式 (默认逐扇区即擦即写)
  --no-ppb          跳过 PPB 解锁
  --no-verify       跳过校验+修复
  -h, --help        显示帮助");
        }
    }
}
