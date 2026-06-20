using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Web.Script.Serialization;

namespace Core
{
    /// <summary>
    /// JSON 语言包加载器。语言包是嵌入资源 i18n/&lt;lang&gt;.json（键值表）。
    /// 用法: L.T("key")、L.T("key", arg0, arg1)。缺失键回退到中文, 再缺失则原样返回 key。
    /// </summary>
    internal static class L
    {
        private const string Fallback = "zh-CN";

        private static Dictionary<string, string> _cur;   // 当前语言
        private static Dictionary<string, string> _fb;    // 回退语言(中文)

        /// <summary>当前生效的语言名。</summary>
        public static string Lang { get; private set; } = Fallback;

        /// <summary>初始化。lang 为 null/空时跟随系统 UI 语言；找不到则回退中文。</summary>
        public static void Init(string lang)
        {
            _fb = Load(Fallback) ?? new Dictionary<string, string>();
            if (string.IsNullOrEmpty(lang)) lang = DetectSystem();

            var d = Load(lang);
            if (d == null) { lang = Fallback; d = _fb; }
            _cur = d;
            Lang = lang;
        }

        /// <summary>取译文，可带 string.Format 参数。</summary>
        public static string T(string key, params object[] args)
        {
            if (_cur == null) Init(null);

            string s;
            if (!_cur.TryGetValue(key, out s) && !_fb.TryGetValue(key, out s))
                s = key;

            return (args != null && args.Length > 0) ? string.Format(s, args) : s;
        }

        private static string DetectSystem()
        {
            string n = CultureInfo.CurrentUICulture.Name; // 例 zh-CN / en-US
            return n.StartsWith("zh", StringComparison.OrdinalIgnoreCase) ? "zh-CN" : "en";
        }

        private static Dictionary<string, string> Load(string lang)
        {
            var asm = Assembly.GetExecutingAssembly();
            using (var st = asm.GetManifestResourceStream(lang + ".json"))
            {
                if (st == null) return null;
                using (var r = new StreamReader(st))
                {
                    string json = r.ReadToEnd();
                    return new JavaScriptSerializer().Deserialize<Dictionary<string, string>>(json);
                }
            }
        }
    }
}
