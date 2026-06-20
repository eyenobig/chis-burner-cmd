using System;
using System.Collections.Generic;
using System.IO.Ports;
using System.Management;
using System.Text.RegularExpressions;
using Core;

namespace Cli
{
    /// <summary>一个串口设备的信息。</summary>
    internal sealed class PortInfo
    {
        public string Port;   // COM7
        public string Name;   // 友好名
        public string Vid;    // 0483 (大写, 可能为 null)
        public string Pid;    // 0721

        /// <summary>是否本烧录器（VID/PID 匹配）。</summary>
        public bool IsBurner =>
            string.Equals(Vid, CartLink.UsbVid, StringComparison.OrdinalIgnoreCase) &&
            string.Equals(Pid, CartLink.UsbPid, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>枚举串口并通过 WMI 解析 USB VID/PID，用于识别烧录器。</summary>
    internal static class DeviceScan
    {
        private static readonly Regex RxCom = new Regex(@"\((COM\d+)\)", RegexOptions.IgnoreCase);
        private static readonly Regex RxId = new Regex(@"VID_([0-9A-Fa-f]{4})&PID_([0-9A-Fa-f]{4})", RegexOptions.IgnoreCase);

        /// <summary>列出系统所有串口（含 VID/PID）。</summary>
        public static List<PortInfo> Enumerate()
        {
            // 先收集裸 COM 口名，WMI 命中后逐个剔除，剩下的就是 WMI 未覆盖的
            var bare = new HashSet<string>(SerialPort.GetPortNames(), StringComparer.OrdinalIgnoreCase);
            var result = new List<PortInfo>();

            try
            {
                using (var s = new ManagementObjectSearcher("SELECT Name, PNPDeviceID FROM Win32_PnPEntity"))
                {
                    foreach (ManagementObject mo in s.Get())
                    {
                        string name = mo["Name"] as string;
                        if (string.IsNullOrEmpty(name)) continue;
                        var m = RxCom.Match(name);
                        if (!m.Success) continue;

                        string port = m.Groups[1].Value.ToUpperInvariant();
                        string pnp = mo["PNPDeviceID"] as string ?? "";
                        var mi = RxId.Match(pnp);
                        result.Add(new PortInfo
                        {
                            Port = port,
                            Name = name,
                            Vid = mi.Success ? mi.Groups[1].Value.ToUpperInvariant() : null,
                            Pid = mi.Success ? mi.Groups[2].Value.ToUpperInvariant() : null,
                        });
                        bare.Remove(port);
                    }
                }
            }
            catch (Exception e)
            {
                Console.Error.WriteLine("WMI 查询失败, 仅按串口名列出: " + e.Message);
            }

            foreach (var p in bare)
                result.Add(new PortInfo { Port = p, Name = "(未知)" });

            result.Sort((a, b) => string.CompareOrdinal(a.Port, b.Port));
            return result;
        }

        /// <summary>第一个识别到的烧录器端口；没有则返回 null。</summary>
        public static string FirstBurner()
        {
            foreach (var p in Enumerate())
                if (p.IsBurner) return p.Port;
            return null;
        }

        /// <summary>测试端口当前是否可打开（用于检测占用）。</summary>
        public static bool CanOpen(string port)
        {
            try { using (var sp = new SerialPort(port)) { sp.Open(); return true; } }
            catch { return false; }
        }
    }
}
