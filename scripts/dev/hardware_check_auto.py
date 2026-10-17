#!/usr/bin/env python3
"""`scripts/dev/hardware-check.sh --auto` 的**判据侧**：只用 Python 标准库。

为什么不装包、也不加 Rust 判据：
- 判据只需要三件事 —— 读 RIFF/WAV、做 Goertzel、读 PNG（zlib 解 stored-deflate）。
  标准库全有（`struct` / `zlib` / `wave` / `array`），零编译、零 Cargo 依赖。
- 判据必须**来自真实运行产物**：本文件只读别人跑出来的 WAV / PNG / SFZ / MIDI，
  ⛔ 不生成任何"期望读数"以外的数字。

用法（每个子命令都只打印 `key=value` 行；退出码 0 = 判据通过，1 = 判据失败，2 = 用法错）：

    python3 scripts/dev/hardware_check_auto.py wav --path take.wav --expect-hz 440
    python3 scripts/dev/hardware_check_auto.py sfz --wav sfz.wav --midi demo.mid --sfz x.sfz --mode fallback
    python3 scripts/dev/hardware_check_auto.py pick-sfz --midi demo.mid
    python3 scripts/dev/hardware_check_auto.py png --path shot.png
    python3 scripts/dev/hardware_check_auto.py synth-wav --path 880.wav --hz 880 --seconds 2
"""

from __future__ import annotations

import argparse
import collections
import math
import os
import struct
import sys
import wave
import zlib

# ---------------------------------------------------------------------------
# WAV（RIFF）—— 支持 PCM 16/24/32 与 IEEE float 32；WAVE_FORMAT_EXTENSIBLE 的 PCM 子格式
# ---------------------------------------------------------------------------

FMT_PCM = 1
FMT_FLOAT = 3
FMT_EXTENSIBLE = 0xFFFE


class WavError(Exception):
    pass


class Wav:
    def __init__(self, sample_rate, channels, bits, fmt, frames, channel_samples):
        self.sample_rate = sample_rate
        self.channels = channels
        self.bits = bits
        self.format = fmt
        self.frames = frames
        self.channel_samples = channel_samples  # list[list[float]]

    def mono(self):
        if self.channels == 1:
            return list(self.channel_samples[0])
        left = self.channel_samples[0]
        right = self.channel_samples[1]
        return [(a + b) * 0.5 for a, b in zip(left, right)]


def _decode_pcm(body, bits, channels):
    frame_bytes = channels * bits // 8
    if len(body) % frame_bytes:
        raise WavError(f"data 长度 {len(body)} 不是帧长 {frame_bytes} 的整数倍")
    frames = len(body) // frame_bytes
    out = [[] for _ in range(channels)]
    if bits == 16:
        scale = 1.0 / 32768.0
        values = struct.unpack("<%dh" % (len(body) // 2), body)
        for i in range(frames):
            base = i * channels
            for c in range(channels):
                out[c].append(values[base + c] * scale)
    elif bits == 32:
        scale = 1.0 / 2147483648.0
        values = struct.unpack("<%di" % (len(body) // 4), body)
        for i in range(frames):
            base = i * channels
            for c in range(channels):
                out[c].append(values[base + c] * scale)
    elif bits == 24:
        scale = 1.0 / 8388608.0
        for i in range(frames):
            base = i * frame_bytes
            for c in range(channels):
                off = base + c * 3
                value = body[off] | (body[off + 1] << 8) | (body[off + 2] << 16)
                if value & 0x800000:
                    value -= 1 << 24
                out[c].append(value * scale)
    else:
        raise WavError(f"不支持的 PCM 位深: {bits}")
    return frames, out


def read_wav(path):
    data = open(path, "rb").read()
    if len(data) < 12 or data[0:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise WavError("不是 RIFF/WAVE 文件")
    pos = 12
    fmt = None
    payload = None
    while pos + 8 <= len(data):
        cid = data[pos : pos + 4]
        size = struct.unpack("<I", data[pos + 4 : pos + 8])[0]
        body = data[pos + 8 : pos + 8 + size]
        if len(body) < size:
            raise WavError(f"chunk {cid!r} 被截断")
        if cid == b"fmt ":
            if size < 16:
                raise WavError("fmt chunk 太短")
            tag, channels, sample_rate, _byte_rate, _align, bits = struct.unpack(
                "<HHIIHH", body[:16]
            )
            if tag == FMT_EXTENSIBLE:
                if size < 40:
                    raise WavError("WAVE_FORMAT_EXTENSIBLE 的 fmt 太短")
                # cbSize(2) + valid_bits(2) + channel_mask(4) + subformat(16)
                valid_bits = struct.unpack("<H", body[18:20])[0]
                sub = struct.unpack("<H", body[24:26])[0]
                if sub != FMT_PCM:
                    raise WavError(f"EXTENSIBLE 子格式不是 PCM: {sub}")
                tag = FMT_PCM
                bits = valid_bits or bits
            fmt = (tag, channels, sample_rate, bits)
        elif cid == b"data":
            payload = body
        pos += 8 + size + (size & 1)
    if fmt is None:
        raise WavError("没有 fmt chunk")
    if payload is None:
        raise WavError("没有 data chunk")
    tag, channels, sample_rate, bits = fmt
    if tag == FMT_FLOAT and bits == 32:
        frames = len(payload) // (4 * channels)
        values = struct.unpack("<%df" % (len(payload) // 4), payload)
        out = [[] for _ in range(channels)]
        for i in range(frames):
            base = i * channels
            for c in range(channels):
                out[c].append(float(values[base + c]))
        return Wav(sample_rate, channels, bits, tag, frames, out)
    if tag != FMT_PCM:
        raise WavError(f"不支持的音频格式 tag={tag}")
    frames, out = _decode_pcm(payload, bits, channels)
    return Wav(sample_rate, channels, bits, tag, frames, out)


# ---------------------------------------------------------------------------
# Goertzel（单一频率的能量）
# ---------------------------------------------------------------------------


def goertzel_power(samples, freq, sample_rate):
    omega = 2.0 * math.pi * freq / sample_rate
    coeff = 2.0 * math.cos(omega)
    s1 = 0.0
    s2 = 0.0
    for value in samples:
        s0 = value + coeff * s1 - s2
        s2 = s1
        s1 = s0
    return s1 * s1 + s2 * s2 - coeff * s1 * s2


def spectral_peak(samples, sample_rate, low, high, coarse=10.0, fine=1.0, span=10.0):
    """先在 [low, high] 上粗扫，再在峰值附近细扫。返回 (freq, power, purity)。"""
    coarse_bins = []
    freq = low
    while freq <= high + 1e-9:
        coarse_bins.append((goertzel_power(samples, freq, sample_rate), freq))
        freq += coarse
    coarse_bins.sort(reverse=True)
    best_power, best_freq = coarse_bins[0]
    total = sum(power for power, _ in coarse_bins)
    start = max(low, best_freq - span)
    stop = min(high, best_freq + span)
    freq = start
    while freq <= stop + 1e-9:
        power = goertzel_power(samples, freq, sample_rate)
        if power > best_power:
            best_power, best_freq = power, freq
        freq += fine
    purity = best_power / total if total > 0 else 0.0
    return best_freq, best_power, purity


def rms(values):
    if not values:
        return 0.0
    return math.sqrt(sum(v * v for v in values) / len(values))


# ---------------------------------------------------------------------------
# MIDI（只读 note on/off + tempo；够把"哪个音符在什么时刻响"取出来）
# ---------------------------------------------------------------------------


def _varlen(data, pos):
    value = 0
    while True:
        byte = data[pos]
        pos += 1
        value = (value << 7) | (byte & 0x7F)
        if not byte & 0x80:
            return value, pos


class MidiNote:
    def __init__(self, pitch, velocity, start_tick, end_tick):
        self.pitch = pitch
        self.velocity = velocity
        self.start_tick = start_tick
        self.end_tick = end_tick


def read_midi(path):
    data = open(path, "rb").read()
    if data[0:4] != b"MThd":
        raise ValueError("不是 Standard MIDI File")
    _fmt, tracks, division = struct.unpack(">HHH", data[8:14])
    if division & 0x8000:
        raise ValueError("SMPTE 时间格式不支持")
    tempo_map = [(0, 500000)]
    notes = []
    pos = 14
    for _ in range(tracks):
        if data[pos : pos + 4] != b"MTrk":
            raise ValueError("缺少 MTrk")
        size = struct.unpack(">I", data[pos + 4 : pos + 8])[0]
        track = data[pos + 8 : pos + 8 + size]
        pos += 8 + size
        tick = 0
        index = 0
        running = None
        open_notes = {}
        while index < len(track):
            delta, index = _varlen(track, index)
            tick += delta
            status = track[index]
            if status & 0x80:
                index += 1
                if status < 0xF0:
                    running = status
            else:
                if running is None:
                    raise ValueError("running status 缺少前导状态字节")
                status = running
            if status == 0xFF:
                meta = track[index]
                index += 1
                length, index = _varlen(track, index)
                body = track[index : index + length]
                index += length
                if meta == 0x51 and len(body) == 3:
                    tempo_map.append((tick, (body[0] << 16) | (body[1] << 8) | body[2]))
                elif meta == 0x2F:
                    break
            elif status in (0xF0, 0xF7):
                length, index = _varlen(track, index)
                index += length
            else:
                kind = status & 0xF0
                need = 1 if kind in (0xC0, 0xD0) else 2
                body = track[index : index + need]
                index += need
                if kind == 0x90 and body[1] > 0:
                    open_notes[body[0]] = (tick, body[1])
                elif kind == 0x80 or (kind == 0x90 and body[1] == 0):
                    if body[0] in open_notes:
                        start, vel = open_notes.pop(body[0])
                        notes.append(MidiNote(body[0], vel, start, tick))
    tempo_map.sort()
    notes.sort(key=lambda n: (n.start_tick, n.pitch))
    return notes, tempo_map, division


def ticks_to_seconds(tick, tempo_map, division):
    seconds = 0.0
    last_tick = 0
    tempo = 500000
    for at, value in tempo_map:
        if at > tick:
            break
        seconds += (at - last_tick) * tempo / 1_000_000.0 / division
        last_tick = at
        tempo = value
    seconds += (tick - last_tick) * tempo / 1_000_000.0 / division
    return seconds


# ---------------------------------------------------------------------------
# SFZ（只读 region / group / master / global 的键域与调音）
# ---------------------------------------------------------------------------


class SfzRegion:
    def __init__(self, lo, hi, lo_vel, hi_vel, keycenter, tune, transpose, sample, trigger):
        self.lo = lo
        self.hi = hi
        self.lo_vel = lo_vel
        self.hi_vel = hi_vel
        self.keycenter = keycenter
        self.tune = tune
        self.transpose = transpose
        self.sample = sample
        self.trigger = trigger

    def matches(self, note, velocity):
        if not (self.lo <= note <= self.hi):
            return False
        if not (self.lo_vel <= velocity <= self.hi_vel):
            return False
        return self.trigger in ("attack", "")

    def expected_fallback_hz(self, note, sample_hz=440.0):
        """`in-memory-sine(440Hz)` 回退时的输出频率（与 yeban-sfz 的 pitch_ratio 同式）。"""
        cents = (note - self.keycenter + self.transpose) * 100.0 + self.tune
        return sample_hz * (2.0 ** (cents / 1200.0))


_INT_KEYS = {
    "lokey",
    "hikey",
    "lovel",
    "hivel",
    "key",
    "pitch_keycenter",
    "tune",
    "transpose",
}


def parse_sfz(path):
    regions = []
    current = None
    scopes = {"global": {}, "master": {}, "group": {}}

    def flush():
        nonlocal current
        if current is not None:
            regions.append(current)
            current = None

    def merged():
        out = {}
        for name in ("global", "master", "group"):
            out.update(scopes[name])
        return out

    for raw in open(path, "r", encoding="utf-8", errors="replace"):
        line = raw.split("//", 1)[0].strip()
        if not line:
            continue
        if line.startswith("<"):
            tag = line.strip("<>").split()[0].lower()
            if tag == "region":
                flush()
                current = dict(merged())
            elif tag == "global":
                flush()
                scopes["global"] = {}
                scopes["master"] = {}
                scopes["group"] = {}
            elif tag == "master":
                flush()
                scopes["master"] = {}
                scopes["group"] = {}
            elif tag == "group":
                flush()
                scopes["group"] = {}
            continue
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip().lower()
        value = value.strip()
        target = current if current is not None else scopes["group"]
        if key == "sample":
            if current is not None:
                current["sample"] = value
        elif key in _INT_KEYS:
            try:
                target[key] = int(value)
            except ValueError:
                pass
        elif key == "trigger":
            target["trigger"] = value.lower()

    def to_region(values):
        lo = values.get("lokey", 0)
        hi = values.get("hikey", 127)
        if "key" in values:
            lo = hi = values["key"]
        keycenter = values.get("pitch_keycenter", values.get("key", 60))
        return SfzRegion(
            lo=lo,
            hi=hi,
            lo_vel=values.get("lovel", 0),
            hi_vel=values.get("hivel", 127),
            keycenter=keycenter,
            tune=values.get("tune", 0),
            transpose=values.get("transpose", 0),
            sample=values.get("sample", ""),
            trigger=values.get("trigger", "attack"),
        )

    # 文件末尾的最后一个 <region> 没有下一个段头收尾 ⇒ 在这里 flush。
    flush()
    return [to_region(r) for r in regions]


# ---------------------------------------------------------------------------
# PNG（只解 8-bit truecolor / truecolor+alpha / grayscale，filter 0..4）
# ---------------------------------------------------------------------------


class PngError(Exception):
    pass


def read_png(path):
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise PngError("不是 PNG")
    pos = 8
    width = height = depth = color = interlace = None
    idat = bytearray()
    while pos + 8 <= len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        cid = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if cid == b"IHDR":
            width, height, depth, color, _comp, _filt, interlace = struct.unpack(
                ">IIBBBBB", body
            )
        elif cid == b"IDAT":
            idat.extend(body)
        elif cid == b"IEND":
            break
    if width is None:
        raise PngError("没有 IHDR")
    if depth != 8:
        raise PngError(f"只支持 8-bit，遇到 {depth}")
    if interlace != 0:
        raise PngError("不支持隔行 PNG")
    channels = {0: 1, 2: 3, 6: 4}.get(color)
    if channels is None:
        raise PngError(f"不支持的 color type {color}")
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    expected = (stride + 1) * height
    if len(raw) < expected:
        raise PngError(f"解压后长度 {len(raw)} < 期望 {expected}")
    rows = []
    previous = bytearray(stride)
    offset = 0
    for _ in range(height):
        filter_type = raw[offset]
        line = bytearray(raw[offset + 1 : offset + 1 + stride])
        offset += stride + 1
        if filter_type == 0:
            pass
        elif filter_type == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = previous[i]
                c = previous[i - channels] if i >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 0xFF
        else:
            raise PngError(f"未知 filter {filter_type}")
        rows.append(line)
        previous = line
    return width, height, channels, rows


def png_stats(path):
    width, height, channels, rows = read_png(path)
    counter = collections.Counter()
    for line in rows:
        for i in range(0, len(line), channels):
            if channels >= 3:
                counter[(line[i], line[i + 1], line[i + 2])] += 1
            else:
                counter[(line[i], line[i], line[i])] += 1
    total = width * height
    background, background_count = counter.most_common(1)[0]
    return {
        "width": width,
        "height": height,
        "bytes": os.path.getsize(path),
        "colors": len(counter),
        "background": "#%02x%02x%02x" % background,
        "non_background": total - background_count,
        "total": total,
    }


# ---------------------------------------------------------------------------
# 子命令
# ---------------------------------------------------------------------------


def emit(lines):
    for line in lines:
        print(line, flush=True)


def cmd_wav(args):
    wav = read_wav(args.path)
    mono = wav.mono()
    duration = wav.frames / wav.sample_rate
    dc = sum(mono) / len(mono) if mono else 0.0
    peak = max(abs(v) for v in mono) if mono else 0.0
    level = rms(mono)
    clipped = sum(1 for v in mono if abs(v) >= 0.999)
    clip_ratio = clipped / len(mono) if mono else 1.0

    # 先在**全带**上找主峰（这样 880Hz 反例不会被"只扫 440 附近"藏起来），
    # 再在峰值附近用 1 秒窗细扫（频率分辨率 0.5Hz，够 ±2Hz 判据）。
    coarse_len = min(len(mono), wav.sample_rate // 2)
    coarse_start = max(0, (len(mono) - coarse_len) // 2)
    coarse_window = mono[coarse_start : coarse_start + coarse_len]
    coarse_freq, _coarse_power, _coarse_purity = spectral_peak(
        coarse_window, wav.sample_rate, 40.0, 4000.0, coarse=20.0, fine=20.0, span=0.0
    )
    window_len = min(len(mono), int(wav.sample_rate))
    start = max(0, (len(mono) - window_len) // 2)
    window = mono[start : start + window_len]
    low = max(20.0, coarse_freq - 20.0)
    high = coarse_freq + 20.0
    freq, power, purity = spectral_peak(window, wav.sample_rate, low, high, coarse=0.5)

    checks = []
    if wav.bits != args.bits:
        checks.append(f"位深 {wav.bits} != 期望 {args.bits}")
    if abs(duration - args.seconds) > args.seconds_tol:
        checks.append(f"时长 {duration:.4f}s 超出 {args.seconds}±{args.seconds_tol}s")
    if abs(dc) >= args.dc_tol:
        checks.append(f"DC {dc:.6f} 超过 {args.dc_tol}")
    if clip_ratio > 0:
        checks.append(f"削顶样本 {clipped}/{len(mono)}")
    if peak <= 0.01:
        checks.append(f"幅度近零 peak={peak:.6f}")
    if level <= 1e-4:
        checks.append(f"RMS {level:.6f} <= 1e-4")
    if abs(freq - args.expect_hz) > args.tol_hz:
        checks.append(f"主频 {freq:.2f}Hz 超出 {args.expect_hz}±{args.tol_hz}Hz")
    if purity < args.min_purity:
        checks.append(f"主频不突出 purity={purity:.3f} < {args.min_purity}")

    emit(
        [
            "wav=path={} sample_rate={} channels={} bits={} frames={} seconds={:.4f}".format(
                args.path, wav.sample_rate, wav.channels, wav.bits, wav.frames, duration
            ),
            "wav=peak={:.6f} rms={:.6f} dc={:.6f} clip={}/{} purity={:.3f} hz={:.2f}".format(
                peak, level, dc, clipped, len(mono), purity, freq
            ),
            "verdict=" + ("pass" if not checks else "fail"),
            "reason=" + ("；".join(checks) if checks else "全部判据通过"),
        ]
    )
    return 0 if not checks else 1


def _select_notes(regions, notes):
    picks = []
    for note in notes:
        matching = [r for r in regions if r.matches(note.pitch, note.velocity)]
        if len(matching) != 1:
            return None, matching
        picks.append(matching[0])
    return picks, None


def cmd_pick_sfz(args):
    notes, _tempo, _division = read_midi(args.midi)
    if not notes:
        emit(["verdict=fail", "reason=MIDI 里没有音符"])
        return 1
    import pathlib

    scanned = 0
    for path in sorted(pathlib.Path(args.samples_root).rglob("*.sfz")):
        scanned += 1
        try:
            regions = parse_sfz(path)
        except OSError:
            continue
        sample_lines = [r.sample for r in regions if r.sample]
        if len(regions) < 2 or len(set(sample_lines)) < 2:
            continue
        picks, _bad = _select_notes(regions, notes)
        if picks is None:
            continue
        freqs = [r.expected_fallback_hz(n.pitch) for r, n in zip(picks, notes)]
        if not all(40.0 <= f <= 4500.0 for f in freqs):
            continue
        if len({round(f, 1) for f in freqs}) < 3:
            continue
        present = sum(1 for s in set(sample_lines) if (path.parent / s.replace("\\", "/")).is_file())
        emit(
            [
                "sfz=" + str(path),
                "sfz=scanned={} regions={} samples={} samples_present={}".format(
                    scanned, len(regions), len(set(sample_lines)), present
                ),
                "sfz=notes=" + ",".join(str(n.pitch) for n in notes),
                "sfz=fallback_hz=" + ",".join(f"{f:.2f}" for f in freqs),
                "verdict=pass",
                "reason=结构可判（每个音符恰好命中 1 个 region）",
            ]
        )
        return 0
    emit(["verdict=not-automatable", "reason=在 assets/samples 里找不到结构可判的 SFZ"])
    return 2


def cmd_sfz(args):
    wav = read_wav(args.wav)
    mono = wav.mono()
    notes, tempo_map, division = read_midi(args.midi)
    regions = parse_sfz(args.sfz)
    if not notes:
        emit(["verdict=fail", "reason=MIDI 里没有音符"])
        return 1

    checks = []
    level = rms(mono)
    if level <= 1e-4:
        checks.append(f"整段 RMS {level:.6f} <= 1e-4（静音）")
    half = len(mono) // 2
    first = rms(mono[:half])
    second = rms(mono[half:])
    relative = abs(first - second) / max(first, second, 1e-12)
    if relative <= 0.01:
        checks.append(f"前后两半 RMS 差 {relative:.4f} <= 1%")

    # 每个音符都会铺上自己的 1 秒样本（回退正弦 1 s），所以整段是**叠加**的：
    # ⛔ 不能按"该音符的时窗里只有一个频率"来判 —— 实测会取到前一个音符的峰值。
    # 判据改成两条：① 全段主峰必须落在某个期望频率 ±5% 内；
    #             ② 每个期望频率都必须在自己的邻域里**显著**高于噪底（> 20×）。
    expected_by_note = []
    readings = []
    for note in notes:
        matching = [r for r in regions if r.matches(note.pitch, note.velocity)]
        if not matching:
            checks.append(f"音符 {note.pitch} 没有匹配 region")
            continue
        if args.mode == "fallback":
            expected = [r.expected_fallback_hz(note.pitch) for r in matching]
        else:
            expected = [440.0 * 2.0 ** ((note.pitch - 69) / 12.0)] * len(matching)
        start = ticks_to_seconds(note.start_tick, tempo_map, division)
        expected_by_note.append((note, expected, start))
        readings.append(
            "note={} vel={} t={:.3f}s expect={}".format(
                note.pitch,
                note.velocity,
                start,
                "/".join(f"{w:.1f}" for w in expected),
            )
        )

    if not expected_by_note:
        checks.append("没有任何音符命中 SFZ region")

    expected_unique = sorted({round(f, 2) for _n, fs, _t in expected_by_note for f in fs})
    peak_freq, peak_power, _peak_purity = spectral_peak(mono, wav.sample_rate, 40.0, 4500.0)
    global_max = peak_power
    if expected_unique:
        deviation = min(abs(peak_freq - want) / want for want in expected_unique)
        readings.append(
            "peak={:.1f}Hz power={:.4g} nearest_expect_dev={:.2f}%".format(
                peak_freq, peak_power, deviation * 100.0
            )
        )
        if deviation > 0.05:
            checks.append(
                f"全段主频 {peak_freq:.1f}Hz 不在任何期望频率 ±5% 内（最近偏差 {deviation * 100:.1f}%）"
            )
        for want in expected_unique:
            power = goertzel_power(mono, want, wav.sample_rate)
            floor = [
                goertzel_power(mono, want + offset, wav.sample_rate)
                for offset in (-40, -30, -20, -10, 10, 20, 30, 40)
                if want + offset >= 20
            ]
            noise = sum(floor) / len(floor) if floor else 0.0
            ratio = power / noise if noise > 0 else float("inf")
            share = power / global_max if global_max > 0 else 0.0
            readings.append(
                "expect={:.2f}Hz power={:.4g} floor={:.4g} ratio={:.1f}x share={:.3f}".format(
                    want, power, noise, ratio, share
                )
            )
            if ratio < 20.0:
                checks.append(f"期望 {want:.1f}Hz 的能量只有噪底的 {ratio:.1f}× (< 20×)")
            if share < 0.01:
                checks.append(f"期望 {want:.1f}Hz 的能量占比 {share:.3f} < 1%")

    emit(
        [
            "sfz=path={} mode={}".format(args.sfz, args.mode),
            "sfz=seconds={:.4f} rms={:.6f} halves={:.6f}/{:.6f} rel={:.4f}".format(
                wav.frames / wav.sample_rate, level, first, second, relative
            ),
        ]
        + ["sfz=" + line for line in readings]
        + [
            "verdict=" + ("pass" if not checks else "fail"),
            "reason=" + ("；".join(checks) if checks else "全部判据通过"),
        ]
    )
    return 0 if not checks else 1


def cmd_png(args):
    try:
        stats = png_stats(args.path)
    except (PngError, zlib.error) as error:
        emit(["verdict=fail", f"reason=PNG 解码失败: {error}"])
        return 1
    checks = []
    if stats["width"] < args.min_width or stats["height"] < args.min_height:
        checks.append(
            f"尺寸 {stats['width']}x{stats['height']} < {args.min_width}x{args.min_height}"
        )
    ratio = stats["non_background"] / stats["total"]
    if ratio <= args.min_non_background:
        checks.append(f"非背景像素 {ratio * 100:.3f}% <= {args.min_non_background * 100:.1f}%")
    emit(
        [
            "png=path={} bytes={} width={} height={} colors={} background={} non_background={}/{}={:.3f}%".format(
                args.path,
                stats["bytes"],
                stats["width"],
                stats["height"],
                stats["colors"],
                stats["background"],
                stats["non_background"],
                stats["total"],
                ratio * 100.0,
            ),
            "verdict=" + ("pass" if not checks else "fail"),
            "reason=" + ("；".join(checks) if checks else "尺寸与像素占比均达标"),
        ]
    )
    return 0 if not checks else 1


def cmd_synth(args):
    frames = int(args.seconds * 48000)
    handle = wave.open(args.path, "wb")
    handle.setnchannels(2)
    handle.setsampwidth(3)
    handle.setframerate(48000)
    body = bytearray()
    for i in range(frames):
        value = int(0.5 * 8388607 * math.sin(2.0 * math.pi * args.hz * i / 48000.0))
        if value < 0:
            value += 1 << 24
        raw = bytes((value & 0xFF, (value >> 8) & 0xFF, (value >> 16) & 0xFF))
        body += raw
        body += raw
    handle.writeframes(bytes(body))
    handle.close()
    emit([f"synth=path={args.path} hz={args.hz} seconds={args.seconds:.3f}", "verdict=pass"])
    return 0


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    wav_parser = sub.add_parser("wav")
    wav_parser.add_argument("--path", required=True)
    wav_parser.add_argument("--expect-hz", type=float, default=440.0)
    wav_parser.add_argument("--tol-hz", type=float, default=2.0)
    wav_parser.add_argument("--seconds", type=float, default=2.0)
    wav_parser.add_argument("--seconds-tol", type=float, default=0.05)
    wav_parser.add_argument("--bits", type=int, default=24)
    wav_parser.add_argument("--dc-tol", type=float, default=1e-3)
    wav_parser.add_argument("--min-purity", type=float, default=0.25)
    wav_parser.set_defaults(func=cmd_wav)

    sfz_parser = sub.add_parser("sfz")
    sfz_parser.add_argument("--wav", required=True)
    sfz_parser.add_argument("--midi", required=True)
    sfz_parser.add_argument("--sfz", required=True)
    sfz_parser.add_argument("--mode", choices=("fallback", "sample"), default="fallback")
    sfz_parser.set_defaults(func=cmd_sfz)

    pick_parser = sub.add_parser("pick-sfz")
    pick_parser.add_argument("--midi", required=True)
    pick_parser.add_argument("--samples-root", default="assets/samples")
    pick_parser.set_defaults(func=cmd_pick_sfz)

    png_parser = sub.add_parser("png")
    png_parser.add_argument("--path", required=True)
    png_parser.add_argument("--min-width", type=int, default=800)
    png_parser.add_argument("--min-height", type=int, default=600)
    png_parser.add_argument("--min-non-background", type=float, default=0.01)
    png_parser.set_defaults(func=cmd_png)

    synth_parser = sub.add_parser("synth-wav")
    synth_parser.add_argument("--path", required=True)
    synth_parser.add_argument("--hz", type=float, default=880.0)
    synth_parser.add_argument("--seconds", type=float, default=2.0)
    synth_parser.set_defaults(func=cmd_synth)

    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except (WavError, PngError, OSError, ValueError) as error:
        emit(["verdict=fail", f"reason={type(error).__name__}: {error}"])
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
