using System;

namespace Core
{
    /// <summary>help 子命令 + 总用法（文案来自语言包）。</summary>
    internal static class HelpCommand
    {
        public static int Run(string[] args)
        {
            // help / -h / --help 后面可带一个具体命令名
            string topic = args.Length > 1 ? args[1] : null;
            switch (topic)
            {
                case "detect":
                case "devices":
                    Console.WriteLine(L.T("help.detect", CartLink.UsbVid, CartLink.UsbPid));
                    return 0;
                case "info":
                    Console.WriteLine(L.T("help.info"));
                    return 0;
                case "burn":
                case "write":
                    Console.WriteLine(L.T("help.burn"));
                    return 0;
                case null:
                    PrintUsage();
                    return 0;
                default:
                    Console.Error.WriteLine(L.T("help.none", topic) + "\n");
                    PrintUsage();
                    return 2;
            }
        }

        public static void PrintUsage() => Console.WriteLine(L.T("help.usage"));
    }
}
