using System;
using System.Collections.Generic;

namespace Core
{
    /// <summary>
    /// cfb 命令行入口 —— 解析 --lang、初始化语言包，再把子命令派发到各 feature 的实现:
    ///   detect/info -> features/device_detect, burn -> features/gba/rom, help -> HelpCommand
    ///
    ///   cfb [--lang zh-CN|en] <命令> [选项]
    /// 兼容旧用法: cfb --port COM7 --rom x.gba ...
    /// </summary>
    internal static class Program
    {
        private static int Main(string[] rawArgs)
        {
            string lang;
            string[] args = ExtractLang(rawArgs, out lang);
            L.Init(lang);

            if (args.Length == 0) { HelpCommand.PrintUsage(); return 0; }

            string cmd = args[0];
            switch (cmd)
            {
                case "detect":
                case "devices":
                    return DetectCommand.Run();
                case "info":
                    return InfoCommand.Run(args);
                case "burn":
                case "write":
                    return BurnCommand.Run(CliCommon.Slice(args, 1));
                case "help":
                case "-h":
                case "--help":
                    return HelpCommand.Run(args);
                default:
                    // 向后兼容: cfb --port .. --rom ..
                    if (cmd.StartsWith("--")) return BurnCommand.Run(args);
                    Console.Error.WriteLine(L.T("err.unknown_cmd", cmd) + "\n");
                    HelpCommand.PrintUsage();
                    return 2;
            }
        }

        /// <summary>抽出全局 --lang 选项，返回去掉它之后的参数。</summary>
        private static string[] ExtractLang(string[] args, out string lang)
        {
            lang = null;
            var rest = new List<string>(args.Length);
            for (int i = 0; i < args.Length; i++)
            {
                if (args[i] == "--lang" && i + 1 < args.Length) { lang = args[++i]; continue; }
                rest.Add(args[i]);
            }
            return rest.ToArray();
        }
    }
}
