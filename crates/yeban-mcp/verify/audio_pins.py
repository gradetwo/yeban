#!/usr/bin/env python3
"""**独立**复算音频片段判据里的三个钉死摘要，并**断言**它们与 Rust 判据逐字相同。

为什么它住在仓库里：判据里的常量如果只是"跑一遍生产代码再抄下来"，那条判据就
只是在断言"代码还是昨天那份代码"，证明不了任何事。这个脚本按**源码里写死的口径**
（SHA-256 over IEEE-754 位型 / xorshift32 + TPDF / RIFF 头布局）用另一个语言重写一遍
—— 它算出来的三个值必须与 `crates/yeban-mcp/tests/render_audio_clips.rs` 的
`PINNED_MASTER_DIGEST` / `PINNED_PAYLOAD_SHA256` / `PINNED_FILE_SHA256` **逐字节相同**，
那才是"常量是被推导出来的"的证据。

本脚本**只依赖标准库**：`f32()` 用 `struct` 往返实现 IEEE-754 binary32 的
round-to-nearest-even，与 Rust `f32` 的四则运算同语义（旧的 numpy 版本已去掉，
理由：门禁机器不应为一条 146 行的复算脚本安装第三方包）。

用法（失败退出码非零，可直接当门禁）：

    python3 crates/yeban-mcp/verify/audio_pins.py

它仍然**不是** cargo 目标：Rust 侧的执行面是 `crates/yeban-mcp/tests/render_audio_clips.rs`
（判据 6）；本脚本由 CI 的 `checks` 作业直接执行，两者互为独立实现。
"""

import hashlib
import math
import struct
import sys


def f32(value: float) -> float:
    """把 Python float 舍入成 IEEE-754 binary32（与 Rust `f32` 同语义）。"""
    return struct.unpack('<f', struct.pack('<f', value))[0]


def bits(x: float) -> bytes:
    return struct.pack('<f', float(x))


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
        a = f32(float(self.next_u32()))
        b = f32(float(0xFFFFFFFF))
        return f32(a / b)


def tpdf(rng):
    return f32(rng.next_unit() - rng.next_unit())


def quantize_i24(x, rng):
    scaled = f32(f32(x * f32(8388608.0)) + tpdf(rng))
    # `f32::round` = round half away from zero
    if scaled != scaled:
        r = 0
    elif scaled >= 2**31 - 1:
        r = 2**31 - 1
    elif scaled <= -2**31:
        r = -2**31
    else:
        r = int(math.floor(abs(scaled) + 0.5)) * (1 if scaled >= 0 else -1)
    return max(-8388608, min(8388607, r))


# ---------- 夹具（与 render_audio_clips.rs 判据 6 同款） ----------
PROJECT_SEED = 0x594542414E000001
PROJECT_ULID = "01J8ZR00000000000000000999"
MASTER_NODE = "01J8ZR00000000000000000001"
FRAMES = 48000
CLIP_FRAMES = 480


def pattern(clip_frames):
    out = []
    for i in range(clip_frames):
        left = f32(f32(float(i % 64)) / f32(64.0)) - f32(0.5)
        right = f32(0.25) - f32(f32(float(i % 32)) / f32(64.0))
        out.append(f32(left)); out.append(f32(right))
    return out


master = pattern(CLIP_FRAMES) + [f32(0.0)] * ((FRAMES - CLIP_FRAMES) * 2)
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
#    `#214`：OriginationDate/Time 取工程创建时刻；本夹具的工程没有创建时刻 ⇒ 纪元哨兵。
#    `#255`：`bext` 的 CodingHistory 计入 NUL 终止符（chunk 外不再补位）。
coding_history = "A=PCM,F=48000,W=24,M=stereo,T=Yeban-MCP"
bext = bytearray()
bext += bytes(256)                                  # description
bext += "Yeban DAW".encode().ljust(32, b"\0")       # originator
bext += PROJECT_ULID.encode().ljust(32, b"\0")     # originator_reference = 工程 ULID
bext += "1970-01-01".encode()                       # origination_date (10)
bext += "00:00:00".encode()                         # origination_time (8)
bext += (0).to_bytes(8, 'little')                   # time_reference
bext += (2).to_bytes(2, 'little')                   # version 2
bext += bytes(64)                                   # umid
bext += ((-32768) & 0xFFFF).to_bytes(2, 'little') * 5   # loudness v2 哨兵 UNKNOWN
bext += bytes(180)                                  # v2 保留区
assert len(bext) == 602, len(bext)
bext += coding_history.encode() + b"\0"
assert len(bext) == 602 + len(coding_history) + 1, len(bext)


def chunk(fourcc, body):
    out = bytearray(fourcc) + struct.pack('<I', len(body)) + body
    if len(body) % 2:
        out += b"\0"
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
file_sha = hashlib.sha256(file_bytes).hexdigest()

# ---------- 断言：与 Rust 判据的钉死常量逐字相同 ----------
# 来源：`crates/yeban-mcp/tests/render_audio_clips.rs` 的 PINNED_* 三常量。
PINNED_MASTER_DIGEST = "c600d4f011f6e223d41363cfc8763b13f1c55ba426f0a95b895c0461c8f3a213"
PINNED_PAYLOAD_SHA256 = "1c6747217c02b1c93c3d24673568be2dc5ea58c5644552fb4fd5ca7a92775f31"
PINNED_FILE_SHA256 = "07d5e34c5af6cba4855281ae0b0eb36ecd083ddabd46da68bd8f290b49f45704"


def check(name: str, actual: str, expected: str) -> bool:
    status = "ok" if actual == expected else "MISMATCH"
    print(f"{name:<16} {status}")
    print(f"  actual   {actual}")
    print(f"  pinned   {expected}")
    return actual == expected


print("masterDigest =", master_digest)
print("payloadSha256 =", payload_sha)
print("fileSha256   =", file_sha)
print("headerBytes  =", header_len)
print("payloadBytes =", len(payload))
print("fileBytes    =", len(file_bytes))
print()

ok = all([
    check("masterDigest", master_digest, PINNED_MASTER_DIGEST),
    check("payloadSha256", payload_sha, PINNED_PAYLOAD_SHA256),
    check("fileSha256", file_sha, PINNED_FILE_SHA256),
])
if not ok:
    print("\n[FAIL] 独立复算与 Rust 钉死常量不一致 —— 要么常量过时，要么本脚本的口径漂了",
          file=sys.stderr)
    raise SystemExit(1)
print("[ok] 三个摘要与 Rust 判据的钉死常量逐字节相同")
