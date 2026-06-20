using System;

namespace Core
{
    /// <summary>功能 1：识别烧录器 —— 列出串口并标出烧录器。</summary>
    internal static class DetectCommand
    {
        public static int Run()
        {
            var list = DeviceScan.Enumerate();
            Console.WriteLine(L.T("detect.found", list.Count) + "\n");
            Console.WriteLine(L.T("detect.header"));
            Console.WriteLine("  -------  -----------  -------  --------------------------------");

            int burners = 0;
            foreach (var p in list)
            {
                string vidpid = p.Vid != null ? $"{p.Vid}:{p.Pid}" : "-";
                string open = DeviceScan.CanOpen(p.Port) ? L.T("detect.open_yes") : L.T("detect.open_no");
                string tag = p.IsBurner ? L.T("detect.burner_tag") : "";
                if (p.IsBurner) burners++;
                Console.WriteLine($"  {p.Port,-7}  {vidpid,-11}  {open,-7}  {p.Name}{tag}");
            }
            Console.WriteLine();

            if (burners == 0)
            {
                Console.WriteLine(L.T("detect.none", CartLink.UsbVid, CartLink.UsbPid));
                return 1;
            }
            Console.WriteLine(L.T("detect.summary", burners));
            return 0;
        }
    }
}
