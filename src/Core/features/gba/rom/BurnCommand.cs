using System;
using System.Diagnostics;
using System.IO;

namespace Core
{
    /// <summary>功能 2 · GBA · ROM：烧录 GBA ROM + 校验 + 记录日志。</summary>
    internal static class BurnCommand
    {
        public static int Run(string[] args)
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
                    case "--port": port = CliCommon.Next(args, ref i); break;
                    case "--rom": rom = CliCommon.Next(args, ref i); break;
                    case "--log": logPath = CliCommon.Next(args, ref i); break;
                    case "--chip-erase": chipErase = true; break;
                    case "--no-ppb": ppb = false; break;
                    case "--no-verify": verify = false; break;
                    case "-h":
                    case "--help": Console.WriteLine(L.T("help.burn")); return 0;
                    default: Console.Error.WriteLine(L.T("err.unknown_arg", args[i])); return 2;
                }
            }

            if (string.IsNullOrEmpty(rom)) { Console.Error.WriteLine(L.T("burn.err.no_rom")); return 2; }
            if (!File.Exists(rom)) { Console.Error.WriteLine(L.T("burn.err.rom_notfound", rom)); return 2; }

            port = CliCommon.ResolvePort(port, Console.WriteLine);
            if (port == null) return 2;

            if (string.IsNullOrEmpty(logPath))
                logPath = Path.Combine(Environment.CurrentDirectory,
                    "cfb_" + DateTime.Now.ToString("yyyyMMdd_HHmmss") + ".log");

            using (var log = new RunLog(logPath))
            {
                byte[] data = File.ReadAllBytes(rom);
                log.Line(L.T("burn.banner"));
                log.Line(L.T("burn.head", port, rom, data.Length.ToString("N0")));
                log.Line(L.T("burn.opts", chipErase, ppb, verify));
                log.Line(L.T("burn.log", logPath));

                var sw = Stopwatch.StartNew();
                int exit = 1;
                try
                {
                    using (var link = new CartLink(port) { Log = s => log.Line(L.T("burn.link", s)) })
                    {
                        link.Open();
                        link.PowerOn3v3();
                        link.WarmUp();

                        var flasher = new GbaFlasher(link) { Log = log.Line };
                        long lastMb = -1;
                        flasher.Progress = (done, total) =>
                        {
                            long mb = done / (1024 * 1024);
                            if (mb != lastMb) { lastMb = mb; log.Line(L.T("burn.progress", mb, total / (1024 * 1024))); }
                        };

                        // 启动前先报告设备信息
                        try
                        {
                            var info = flasher.ReadInfo();
                            log.Line(L.T("burn.dev_id", info.IdHex));
                            log.Line(L.T("burn.dev_cap", info.DeviceSize.ToString("N0"), info.BufferWriteBytes, info.SectorSize, info.SectorCount));
                        }
                        catch (Exception e) { log.Line(L.T("burn.dev_read_fail", e.Message)); }

                        var opt = new BurnOptions { ChipErase = chipErase, UnlockPpb = ppb, VerifyAfter = verify };
                        var res = flasher.Burn(data, data.Length, opt);

                        log.Line(L.T("burn.result"));
                        log.Line(L.T("burn.success", res.Success));
                        log.Line(L.T("burn.written", res.BytesWritten.ToString("N0")));
                        log.Line(L.T("burn.reconnect", res.Reconnects, res.AckFlushes));
                        if (res.FirstBadAddress >= 0) log.Line(L.T("burn.stopaddr", res.FirstBadAddress.ToString("X8")));
                        log.Line(L.T("burn.mismatch", res.MismatchBytes));
                        log.Line(L.T("burn.time", res.Seconds.ToString("f0")));

                        exit = res.Success ? 0 : 1;
                    }
                }
                catch (Exception e)
                {
                    log.Line(L.T("burn.fatal", e));
                    exit = 3;
                }

                sw.Stop();
                log.Line(L.T("burn.end", exit, sw.Elapsed.TotalSeconds.ToString("f0")));
                return exit;
            }
        }
    }
}
