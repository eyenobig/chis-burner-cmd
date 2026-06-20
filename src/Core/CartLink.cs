using System;
using System.Diagnostics;
using System.IO.Ports;
using System.Threading;

namespace Core
{
    /// <summary>
    /// 碳酸丐烧录器的串口协议层（USB CDC，VID 0483 / PID 0721）。
    /// 复刻原上位机的命令格式，但所有应答读取都带超时，
    /// 并提供 Reconnect() 在 MCU 卡死（持续编程后不再应答）时关/重开串口复活设备。
    /// </summary>
    public sealed class CartLink : IDisposable
    {
        /// <summary>本烧录器的 USB 标识（STMicroelectronics CDC 虚拟串口）。</summary>
        public const string UsbVid = "0483";
        public const string UsbPid = "0721";

        private readonly string _portName;
        private SerialPort _sp;

        /// <summary>单次应答超时(ms)。USB 正常应答是毫秒级，超时即认为这一包应答丢失。</summary>
        public int ResponseTimeoutMs { get; set; } = 800;

        /// <summary>诊断日志回调（可选）。</summary>
        public Action<string> Log { get; set; }

        public bool IsOpen => _sp != null && _sp.IsOpen;
        public string PortName => _portName;

        public CartLink(string portName)
        {
            _portName = portName ?? throw new ArgumentNullException(nameof(portName));
        }

        private void Trace(string s) => Log?.Invoke(s);

        public void Open()
        {
            Close();
            _sp = new SerialPort(_portName, 9600, Parity.None, 8, StopBits.One)
            {
                ReadTimeout = 2000,
                WriteTimeout = 2000,
                ReadBufferSize = 1 << 20,
                WriteBufferSize = 1 << 20,
            };
            _sp.Open();
            ToggleResetLines();
            Thread.Sleep(60);
            DiscardAll();
        }

        public void Close()
        {
            try { if (_sp != null && _sp.IsOpen) _sp.Close(); } catch { }
            _sp?.Dispose();
            _sp = null;
        }

        /// <summary>关/重开串口并重新上电——用于 MCU 卡死后的复活。</summary>
        public void Reconnect()
        {
            Trace("reconnect...");
            Close();
            Thread.Sleep(700);
            Open();
            PowerOn3v3();
            WarmUp();
        }

        // DTR/RTS 置位再清零：重置单片机内的命令 buffer
        private void ToggleResetLines()
        {
            _sp.RtsEnable = true; _sp.DtrEnable = true;
            _sp.RtsEnable = false; _sp.DtrEnable = false;
        }

        private void DiscardAll()
        {
            try { _sp.DiscardInBuffer(); _sp.DiscardOutBuffer(); } catch { }
        }

        /// <summary>给卡带上电（GBA = 3.3V）。</summary>
        public void PowerOn3v3()
        {
            SendPackage(new byte[] { 0xa0, 0x01 });
            Thread.Sleep(10);
            ToggleResetLines();
            Thread.Sleep(10);
        }

        public void PowerOff()
        {
            SendPackage(new byte[] { 0xa0, 0x00 });
            Thread.Sleep(10);
            ToggleResetLines();
            Thread.Sleep(10);
        }

        /// <summary>
        /// 吸收上电/复位后“被吞的第一条命令”，并把 flash 复位回读阵列模式。
        /// 这是该固件的已知怪癖：紧跟 DTR/RTS 复位之后的第一条命令会被丢弃。
        /// </summary>
        public void WarmUp()
        {
            int save = ResponseTimeoutMs;
            ResponseTimeoutMs = 700;
            for (int i = 0; i < 2; i++)
            {
                RomWrite(0x00, new byte[] { 0xf0, 0x00 }); // reset to read array
                DiscardAll();
            }
            ResponseTimeoutMs = save;
        }

        // ---------------- 低层收发 ----------------

        private void SendPackage(byte[] payload)
        {
            int size = 2 + payload.Length + 2; // 包大小含末尾2字节(忽略的)CRC
            var buf = new byte[size];
            buf[0] = (byte)(size & 0xff);
            buf[1] = (byte)((size >> 8) & 0xff);
            Array.Copy(payload, 0, buf, 2, payload.Length);
            _sp.Write(buf, 0, size);
        }

        // 读满 n 字节，超时返回 false（有数据进展则重置计时）
        private bool ReadExact(byte[] buf, int n)
        {
            int got = 0;
            var sw = Stopwatch.StartNew();
            while (got < n)
            {
                int avail = _sp.BytesToRead;
                if (avail > 0)
                {
                    int toRead = Math.Min(avail, n - got);
                    int r;
                    try { r = _sp.Read(buf, got, toRead); }
                    catch { return false; }
                    got += r;
                    sw.Restart();
                }
                else if (sw.ElapsedMilliseconds > ResponseTimeoutMs)
                {
                    return false;
                }
            }
            return true;
        }

        // 读 1 字节 ack，期望 0xaa
        private bool ReadAck()
        {
            var b = new byte[1];
            if (!ReadExact(b, 1)) return false;
            DiscardAll();
            return b[0] == 0xaa;
        }

        // 读 n 字节数据（前 2 字节是被忽略的 CRC）
        private bool ReadDataBytes(byte[] outBuf)
        {
            var raw = new byte[outBuf.Length + 2];
            if (!ReadExact(raw, raw.Length)) return false;
            Array.Copy(raw, 2, outBuf, 0, outBuf.Length);
            return true;
        }

        private static byte[] Addr4(uint a) => new[]
        {
            (byte)(a & 0xff), (byte)((a >> 8) & 0xff), (byte)((a >> 16) & 0xff), (byte)((a >> 24) & 0xff)
        };

        // ---------------- 协议命令 ----------------

        /// <summary>rom 直接写(透传)，地址为“字”地址。返回是否收到 ack。</summary>
        public bool RomWrite(uint wordAddr, byte[] data)
        {
            var pl = new byte[5 + data.Length];
            pl[0] = 0xf5;
            Array.Copy(Addr4(wordAddr), 0, pl, 1, 4);
            Array.Copy(data, 0, pl, 5, data.Length);
            SendPackage(pl);
            return ReadAck();
        }

        /// <summary>rom 读取，地址为“字节”地址。失败(超时)返回 false。</summary>
        public bool RomRead(uint byteAddr, byte[] outBuf)
        {
            var pl = new byte[7];
            pl[0] = 0xf6;
            Array.Copy(Addr4(byteAddr), 0, pl, 1, 4);
            pl[5] = (byte)(outBuf.Length & 0xff);
            pl[6] = (byte)((outBuf.Length >> 8) & 0xff);
            SendPackage(pl);
            return ReadDataBytes(outBuf);
        }

        /// <summary>rom 编程(缓冲写)，地址为“字节”地址。</summary>
        public bool RomProgram(uint byteAddr, byte[] data, ushort bufferWriteBytes)
        {
            var pl = new byte[7 + data.Length];
            pl[0] = 0xf4;
            Array.Copy(Addr4(byteAddr), 0, pl, 1, 4);
            pl[5] = (byte)(bufferWriteBytes & 0xff);
            pl[6] = (byte)((bufferWriteBytes >> 8) & 0xff);
            Array.Copy(data, 0, pl, 7, data.Length);
            SendPackage(pl);
            return ReadAck();
        }

        /// <summary>读 Autoselect ID（8 字节）。</summary>
        public bool RomReadId(byte[] id8)
        {
            SendPackage(new byte[] { 0xf0 });
            return ReadDataBytes(id8);
        }

        /// <summary>全片擦除命令（仅下发，不等待完成）。</summary>
        public bool RomEraseChip()
        {
            SendPackage(new byte[] { 0xf1 });
            return ReadAck();
        }

        public void Dispose() => Close();
    }
}
