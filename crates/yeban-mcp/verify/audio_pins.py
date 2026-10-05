#!/usr/bin/env python3
"""**独立**复算音频片段判据里的三个钉死摘要（本机可跑）。

为什么它住在仓库里：判据里的常量如果只是"跑一遍生产代码再抄下来"，那条判据就
只是在断言"代码还是昨天那份代码"，证明不了任何事。这个脚本按**源码里写死的口径**
（SHA-256 over IEEE-754 位型 / xorshift32 + TPDF / RIFF 头布局）用另一个语言重写一遍
—— 它跑出来的三个值与 `crates/yeban-mcp/tests/render_audio_clips.rs` 里的常量
**逐字节相同**，那才是"常量是被推导出来的"的证据。

用法（需要 numpy）:

    python3 crates/yeban-mcp/verify/audio_pins.py

它**不是** cargo 目标，也不参与任何门禁（与 `verify/render_pure.rs` 同一处置）：
Cargo 只认 `src/`、`tests/`、`benches/`、`examples/`。
"""

import hashlib
import struct
import numpy as np

F32 = np.float32
def bits(x): return struct.pack('<f', float(x))

# ---------- dither_seed (rng.rs) ----------
def dither_seed(project_seed: int, node: str) -> int:
    h = 0xcbf29ce484222325
    for b in project_seed.to_bytes(8, 'little'):
        h ^= b
        h = (h * 0x00000100000001B3) & 0xFFFFFFFFFFFFFFFF
    for b in node.encode():
        h ^= b
        h = (h * 0x00000100000001B3) & 0xFFFFFFFFFFFFFFFF
    m = h
    m ^= m >> 30; m = (m * 0xbf58476d1ce4e5b9) & 0xFFFFFFFFFFFFFFFF
    m ^= m >> 27; m = (m * 0x94d049bb133111eb) & 0xFFFFFFFFFFFFFFFF
    m ^= m >> 31
    return m & 0xFFFFFFFF

class Rng:
    FALLBACK = 0x9E3779B9
    def __init__(self, seed):
        self.state = self.FALLBACK if seed == 0 else seed
    def next_u32(self):
        x = self.state
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        self.state = x
        return x
    def next_unit(self):
        # `x as f32 / u32::MAX as f32`，两步都按 f32 舍入。
        a = F32(np.uint32(self.next_u32()).astype(np.float64))  # u32 -> f32 (round-to-nearest)
        b = F32(np.float64(0xFFFFFFFF))
        return F32(a / b)

def tpdf(rng):
    return F32(rng.next_unit() - rng.next_unit())

def quantize_i24(x, rng):
    scaled = F32(F32(x * F32(8388608.0)) + tpdf(rng))
    # `f32::round` = round half away from zero
    v = float(scaled)
    if v != v: r = 0
    elif v >= 2**31 - 1: r = 2**31 - 1
    elif v <= -2**31: r = -2**31
    else:
        r = int(np.floor(abs(v) + 0.5)) * (1 if v >= 0 else -1)
    return max(-8388608, min(8388607, r))

# ---------- 夹具 ----------
PROJECT_SEED = 0x594542414E000001
PROJECT_ULID = "01J8ZR00000000000000000999"
MASTER_NODE = "01J8ZR00000000000000000001"
FRAMES = 48000
CLIP_FRAMES = 480

def pattern(clip_frames):
    out = []
    for i in range(clip_frames):
        left = F32(F32(np.float64(i % 64)) / F32(64.0)) - F32(0.5)
        right = F32(0.25) - F32(F32(np.float64(i % 32)) / F32(64.0))
        out.append(F32(left)); out.append(F32(right))
    return out

master = pattern(CLIP_FRAMES) + [F32(0.0)] * ((FRAMES - CLIP_FRAMES) * 2)
assert len(master) == FRAMES * 2

# 1) 母带浮点摘要
digest = hashlib.sha256()
for s in master:
    digest.update(bits(s))
master_digest = digest.hexdigest()

# 2) 24-bit 负载
rng = Rng(dither_seed(PROJECT_SEED, MASTER_NODE))
payload = bytearray()
for s in master:
    q = quantize_i24(s, rng)
    payload += (q & 0xFFFFFF).to_bytes(3, 'little')
payload_sha = hashlib.sha256(bytes(payload)).hexdigest()

# 3) RIFF 头部（rf64.rs 的 RIFF 分支）
coding_history = "A=PCM,F=48000,W=24,M=stereo,T=Yeban-MCP"
bext = bytearray()
bext += b"" .ljust(0)
bext += bytes(256)                                  # description
bext += "Yeban DAW".encode().ljust(32, b"\0")       # originator
bext += PROJECT_ULID.encode().ljust(32, b"\0")     # originator_reference = 工程 ULID
bext += "2025-10-09".encode()                       # origination_date (10)
bext += "08:53:20".encode()                         # origination_time (8)
bext += (0).to_bytes(8, 'little')                   # time_reference
bext += (2).to_bytes(2, 'little')                   # version 2
bext += bytes(64)                                   # umid
bext += ((-32768) & 0xFFFF).to_bytes(2, 'little') * 5   # loudness v2 哨兵 UNKNOWN
bext += bytes(180)                                  # v2 保留区
assert len(bext) == 602, len(bext)
bext += coding_history.encode()
assert len(bext) == 602 + 39, len(bext)

def chunk(fourcc, body):
    out = bytearray(fourcc) + struct.pack('<I', len(body)) + body
    if len(body) % 2: out += b"\0"
    return bytes(out)

fmt = struct.pack('<HHIIHH', 1, 2, 48000, 48000 * 6, 6, 24)
payload_total = len(payload) + (len(payload) % 2)
header_no_ds64 = 12 + len(chunk(b"fmt ", fmt)) + len(chunk(b"bext", bext)) + 8
header_len = header_no_ds64
riff_size = header_len + payload_total - 8
header = bytearray()
header += b"RIFF" + struct.pack('<I', riff_size) + b"WAVE"
header += chunk(b"fmt ", fmt)
header += chunk(b"bext", bext)
header += b"data" + struct.pack('<I', len(payload))
assert len(header) == header_len, (len(header), header_len)
file_bytes = bytes(header) + bytes(payload)
assert len(file_bytes) % 2 == 0

print("masterDigest =", master_digest)
print("payloadSha256 =", payload_sha)
print("fileSha256   =", hashlib.sha256(file_bytes).hexdigest())
print("headerBytes  =", header_len)
print("payloadBytes =", len(payload))
print("fileBytes    =", len(file_bytes))

