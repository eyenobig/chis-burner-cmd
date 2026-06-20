using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Threading;

namespace ChisFlashBurner.Core
{
    /// <summary>NOR flash 信息（CFI）。</summary>
    public sealed class FlashInfo
    {
        public byte[] Id;
        public long DeviceSize;        // 字节
        public int BufferWriteBytes;   // 写缓冲大小(字节)，0=仅单字编程
        public int SectorSize;         // 统一扇区大小(字节)
        public int SectorCount;
        public string IdHex => Id == null ? "" : BitConverter.ToString(Id).Replace("-", " ");
    }

    public sealed class BurnOptions
    {
        /// <summary>烧录前是否整片擦除（false 则按扇区即擦即写）。</summary>
        public bool ChipErase = false;
        /// <summary>开始前是否自动解锁 PPB（上半区被保护时必需）。</summary>
        public bool UnlockPpb = true;
        /// <summary>烧完后是否全量校验+修复。</summary>
        public bool VerifyAfter = true;
        /// <summary>每包连续无应答多少次触发一次重连复活。</summary>
        public int ReconnectEveryFails = 5;
        /// <summary>单包最大尝试次数，超过则判定坏块放弃。</summary>
        public int MaxTriesPerPacket = 60;
        /// <summary>校验失败后最多修复轮数。</summary>
        public int MaxRepairRounds = 8;
    }

    public sealed class BurnResult
    {
        public bool Success;
        public long BytesWritten;
        public int Reconnects;
        public int AckFlushes;
        public long FirstBadAddress = -1;  // 放弃时停在的地址
        public int MismatchBytes;          // 最终校验不符字节数
        public double Seconds;
    }

    /// <summary>
    /// GBA flash 高层操作：CFI/ID 读取、擦除、PPB 解锁、健壮编程、校验。
    /// 健壮性策略（实测有效）：每包必须收到 ACK 才前进；连续无应答说明 MCU 卡死，
    /// 自动关/重开串口复活后重试；最后逐扇区校验，对不符扇区擦除+重写修复。
    /// </summary>
    public sealed class GbaFlasher
    {
        public const int Sector = 0x20000; // 128KB（S29GL 系列统一扇区）
        private const int Packet = 4096;

        private readonly CartLink _link;
        private long _progressTotal;
        public Action<string> Log { get; set; }
        /// <summary>进度回调 (已完成字节, 总字节)。</summary>
        public Action<long, long> Progress { get; set; }

        public GbaFlasher(CartLink link)
        {
            _link = link ?? throw new ArgumentNullException(nameof(link));
        }

        private void Trace(string s) => Log?.Invoke(s);

        // ---------------- 信息读取 ----------------

        /// <summary>读取 ID + CFI。注意：调用前应已 PowerOn + WarmUp。</summary>
        public FlashInfo ReadInfo()
        {
            var info = new FlashInfo();

            var id = new byte[8];
            if (_link.RomReadId(id)) info.Id = id;

            // CFI 一次性连续读整块（分多次零散读在某些片子上会掉出 CFI 模式）
            _link.RomWrite(0x55, new byte[] { 0x98, 0x00 });
            var cfi = new byte[(0x34 - 0x27 + 1) * 2]; // 覆盖 word 0x27..0x34
            _link.RomRead(0x27 << 1, cfi);
            _link.RomWrite(0x00, new byte[] { 0xf0, 0x00 });

            byte CfiWord(int w) => cfi[(w - 0x27) * 2];

            info.DeviceSize = (long)Math.Pow(2, CfiWord(0x27));
            int bufN = CfiWord(0x2a);
            info.BufferWriteBytes = bufN == 0 ? 0 : (int)Math.Pow(2, bufN);

            int blockRegions = CfiWord(0x2c);
            int sectorCount0 = ((CfiWord(0x2e) << 8) | CfiWord(0x2d)) + 1;
            int sectorSize0 = ((CfiWord(0x30) << 8) | CfiWord(0x2f)) * 256;
            info.SectorCount = sectorCount0;
            info.SectorSize = sectorSize0 > 0 ? sectorSize0 : Sector;
            if (blockRegions != 1)
                Trace($"注意: 检测到 {blockRegions} 个擦除区(非统一扇区)，按 {info.SectorSize} 处理");

            return info;
        }

        // ---------------- 擦除 ----------------

        /// <summary>全片擦除并等待完成。</summary>
        public void EraseChip(int timeoutSec = 200)
        {
            _link.RomEraseChip();
            var sw = Stopwatch.StartNew();
            var probe = new byte[2];
            while (true)
            {
                if (_link.RomRead(0, probe) && probe[0] == 0xff && probe[1] == 0xff) break;
                Thread.Sleep(500);
                if (sw.Elapsed.TotalSeconds > timeoutSec) throw new TimeoutException("全片擦除超时");
            }
            Trace($"全片擦除完成 {sw.Elapsed.TotalSeconds:f0}s");
        }

        /// <summary>扇区擦除（byteBase 须扇区对齐）。失败自动重连重试。</summary>
        public bool EraseSector(uint byteBase, int retries = 5)
        {
            var probe = new byte[2];
            for (int k = 0; k < retries; k++)
            {
                try
                {
                    _link.RomWrite(0x555, new byte[] { 0xaa, 0x00 });
                    _link.RomWrite(0x2aa, new byte[] { 0x55, 0x00 });
                    _link.RomWrite(0x555, new byte[] { 0x80, 0x00 });
                    _link.RomWrite(0x555, new byte[] { 0xaa, 0x00 });
                    _link.RomWrite(0x2aa, new byte[] { 0x55, 0x00 });
                    _link.RomWrite(byteBase >> 1, new byte[] { 0x30, 0x00 });

                    var sw = Stopwatch.StartNew();
                    while (true)
                    {
                        if (_link.RomRead(byteBase, probe) && probe[0] == 0xff && probe[1] == 0xff)
                            return true;
                        Thread.Sleep(20);
                        if (sw.Elapsed.TotalSeconds > 6) throw new TimeoutException();
                    }
                }
                catch { _link.Reconnect(); }
            }
            return false;
        }

        // ---------------- PPB 解锁 ----------------

        /// <summary>All PPB Erase：清除全部扇区的持久保护位（上半区写不进时多半因 PPB）。</summary>
        public void UnlockAllPpb()
        {
            Trace("解锁 PPB (All PPB Erase) ...");
            // 退出任何命令集
            _link.RomWrite(0, new byte[] { 0x90, 0x00 });
            _link.RomWrite(0, new byte[] { 0x00, 0x00 });
            _link.RomWrite(0, new byte[] { 0xf0, 0x00 });

            // 进入非易失扇区保护命令集并 All PPB Erase
            _link.RomWrite(0x555, new byte[] { 0xaa, 0x00 });
            _link.RomWrite(0x2aa, new byte[] { 0x55, 0x00 });
            _link.RomWrite(0x555, new byte[] { 0xc0, 0x00 });
            _link.RomWrite(0, new byte[] { 0x80, 0x00 });
            _link.RomWrite(0, new byte[] { 0x30, 0x00 }); // All PPB Erase
            Thread.Sleep(2000);
            _link.RomWrite(0, new byte[] { 0x90, 0x00 });
            _link.RomWrite(0, new byte[] { 0x00, 0x00 });
            _link.RomWrite(0, new byte[] { 0xf0, 0x00 });
        }

        // ---------------- 编程 ----------------

        // 编程 [from,to)；每包必须 ACK 才前进，连续失败重连复活。返回首个失败地址或 -1(完成)。
        private long ProgramFlow(byte[] rom, long from, long to, int bufWr, BurnResult res)
        {
            long pos = from;
            while (pos < to)
            {
                int len = (int)Math.Min(Packet, to - pos);
                var pk = new byte[len];
                Array.Copy(rom, pos, pk, 0, len);

                int tries = 0;
                while (true)
                {
                    bool ok;
                    try { ok = _link.RomProgram((uint)pos, pk, (ushort)bufWr); }
                    catch { ok = false; }
                    if (ok) break;

                    res.AckFlushes++;
                    tries++;
                    if (tries % 5 == 0) { res.Reconnects++; _link.Reconnect(); }
                    if (tries >= 60) return pos;
                }

                pos += len;
                res.BytesWritten += len;
                Progress?.Invoke(pos, _progressTotal > 0 ? _progressTotal : to);
            }
            return -1;
        }

        // ---------------- 校验 ----------------

        /// <summary>逐扇区校验，返回不一致的扇区基址集合；out 总不符字节数。</summary>
        public HashSet<long> FindBadSectors(byte[] rom, long total, out int mismatchBytes)
        {
            var bad = new HashSet<long>();
            mismatchBytes = 0;
            long pos = 0;
            var buf = new byte[Packet];
            while (pos < total)
            {
                int len = (int)Math.Min(Packet, total - pos);
                var b = len == Packet ? buf : new byte[len];
                if (!_link.RomRead((uint)pos, b)) { _link.Reconnect(); continue; }
                for (int i = 0; i < len; i++)
                {
                    if (b[i] != rom[pos + i])
                    {
                        bad.Add(((pos + i) / Sector) * Sector);
                        mismatchBytes++;
                    }
                }
                pos += len;
            }
            return bad;
        }

        // ---------------- 全流程 ----------------

        /// <summary>完整烧录：(可选解锁PPB) → (整片或逐扇区)擦除+编程 → 校验+修复。</summary>
        public BurnResult Burn(byte[] rom, long length, BurnOptions opt = null)
        {
            opt = opt ?? new BurnOptions();
            var res = new BurnResult();
            _progressTotal = length;
            var grand = Stopwatch.StartNew();

            int bufWr = 32; // S29GL256 写缓冲；ReadInfo 可覆盖
            try
            {
                var info = ReadInfo();
                if (info.BufferWriteBytes > 0) bufWr = info.BufferWriteBytes;
                Trace($"ID:{info.IdHex} 容量:{info.DeviceSize} BuffWr:{bufWr}");
            }
            catch (Exception e) { Trace("读取 CFI 失败，沿用默认 bufWr=32：" + e.Message); }

            if (opt.UnlockPpb)
            {
                try { UnlockAllPpb(); } catch (Exception e) { Trace("PPB 解锁异常: " + e.Message); }
            }

            if (opt.ChipErase)
            {
                EraseChip();
                Trace("整片擦除模式：直接编程");
                long fail = ProgramFlow(rom, 0, length, bufWr, res);
                if (fail >= 0) { res.FirstBadAddress = fail; }
            }
            else
            {
                // 逐扇区：先擦该扇区再写该扇区，闯过卡死点
                for (long b = 0; b < length; b += Sector)
                {
                    long end = Math.Min(b + Sector, length);
                    if (!EraseSector((uint)b))
                    {
                        Trace($"扇区 0x{b:X8} 擦除失败");
                        res.FirstBadAddress = b;
                        break;
                    }
                    long fail = ProgramFlow(rom, b, end, bufWr, res);
                    if (fail >= 0) { res.FirstBadAddress = fail; break; }
                }
            }

            if (opt.VerifyAfter && res.FirstBadAddress < 0)
            {
                for (int round = 1; round <= opt.MaxRepairRounds; round++)
                {
                    var bad = FindBadSectors(rom, length, out int mm);
                    res.MismatchBytes = mm;
                    Trace($"校验(第{round}轮): {mm} 字节不符, {bad.Count} 扇区");
                    if (bad.Count == 0) break;

                    var list = new List<long>(bad);
                    list.Sort();
                    foreach (var bsec in list)
                    {
                        if (EraseSector((uint)bsec))
                            ProgramFlow(rom, bsec, Math.Min(bsec + Sector, length), bufWr, res);
                        else
                            Trace($"修复: 扇区 0x{bsec:X8} 擦除失败");
                    }
                }
            }

            res.Success = res.FirstBadAddress < 0 && res.MismatchBytes == 0;
            res.Seconds = grand.Elapsed.TotalSeconds;
            Trace(res.Success
                ? $"烧录成功 ✅  {res.BytesWritten:N0} 字节, 重连{res.Reconnects}次, {res.Seconds:f0}s"
                : $"烧录未完成 ❌  停在 0x{res.FirstBadAddress:X8}, 不符 {res.MismatchBytes} 字节");
            return res;
        }
    }
}
