using System;

namespace Core
{
    /// <summary>功能 1：读设备信息 —— 连接并打印芯片 ID + 容量。</summary>
    internal static class InfoCommand
    {
        public static int Run(string[] args)
        {
            string port = null;
            for (int i = 1; i < args.Length; i++)
            {
                switch (args[i])
                {
                    case "--port": port = CliCommon.Next(args, ref i); break;
                    case "-h":
                    case "--help":
                        Console.WriteLine(L.T("info.usage"));
                        return 0;
                    default:
                        Console.Error.WriteLine(L.T("err.unknown_arg", args[i])); return 2;
                }
            }

            port = CliCommon.ResolvePort(port, Console.WriteLine);
            if (port == null) return 2;

            try
            {
                using (var link = new CartLink(port) { Log = s => Console.WriteLine(L.T("burn.link", s)) })
                {
                    link.Open();
                    link.PowerOn3v3();
                    link.WarmUp();

                    var info = new GbaFlasher(link).ReadInfo();
                    Console.WriteLine(L.T("info.label.port", port));
                    Console.WriteLine(L.T("info.label.id", info.IdHex));
                    Console.WriteLine(L.T("info.label.cap", info.DeviceSize.ToString("N0"), info.DeviceSize / 1024 / 1024));
                    Console.WriteLine(L.T("info.label.buf", info.BufferWriteBytes));
                    Console.WriteLine(L.T("info.label.sector", info.SectorSize.ToString("N0"), info.SectorCount));

                    link.PowerOff();
                    return 0;
                }
            }
            catch (Exception e)
            {
                Console.Error.WriteLine(L.T("info.err.read", e.Message));
                return 3;
            }
        }
    }
}
