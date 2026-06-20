using System;
using System.IO;

namespace Core
{
    /// <summary>各命令共用的小工具：参数解析 + 端口自动识别。</summary>
    internal static class CliCommon
    {
        /// <summary>取下一个参数值（用于 --opt value 形式）。</summary>
        public static string Next(string[] a, ref int i)
        {
            if (i + 1 >= a.Length) throw new ArgumentException(L.T("err.missing_value", a[i]));
            return a[++i];
        }

        /// <summary>从 start 起切片（去掉子命令名）。</summary>
        public static string[] Slice(string[] a, int start)
        {
            var r = new string[a.Length - start];
            Array.Copy(a, start, r, 0, r.Length);
            return r;
        }

        /// <summary>指定了 --port 就用它；否则按 VID/PID 自动识别烧录器。返回 null 表示找不到。</summary>
        public static string ResolvePort(string port, Action<string> say)
        {
            if (!string.IsNullOrEmpty(port)) return port;
            string p = DeviceScan.FirstBurner();
            if (p == null)
            {
                Console.Error.WriteLine(L.T("err.no_burner", CartLink.UsbVid, CartLink.UsbPid));
                return null;
            }
            say(L.T("common.auto_port", p));
            return p;
        }
    }

    /// <summary>带时间戳、同时写控制台和日志文件的运行日志（烧录用）。</summary>
    internal sealed class RunLog : IDisposable
    {
        private StreamWriter _file;

        public RunLog(string path)
        {
            try { _file = new StreamWriter(path, false) { AutoFlush = true }; }
            catch (Exception e) { Console.Error.WriteLine(L.T("err.log_open", e.Message)); }
        }

        public void Line(string s)
        {
            string stamped = "[" + DateTime.Now.ToString("HH:mm:ss.fff") + "] " + s;
            Console.WriteLine(stamped);
            try { _file?.WriteLine(stamped); _file?.Flush(); } catch { }
        }

        public void Dispose() => _file?.Dispose();
    }
}
