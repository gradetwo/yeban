#!/usr/bin/env bash
# 夜半「听音路径」的共用实现 —— 任务单 `#207`。
#
# 本文件被两条入口 source:
#   scripts/dev/play-sample.sh      纯命令行：挑乐器 → 渲染短句 → afplay
#   scripts/dev/play-sample-gui.sh  造 `.yeban` 夹具 → `open` 图形界面
#
# 为什么抽成一份：
#   - 「二进制新鲜度」只能有**一处**判定（否则两条路会给出两种"旧不旧"的答案）；
#   - 「样本真解码」的判据与 `scripts/dev/verify-samples.sh` **同一条**：
#     `exported-wav:` 行的 `source=sfz(file(` 且**没有** `回退`；
#   - 音高变化证据（分段 f0）只写一份 Python，两个入口共用。
#
# 边界（诚实登记）：
#   - 只读库根（`~/Library/Application Support/dev.yeban.app/Samples`）、只读工程；
#   - 写只落在调用方给的临时目录 / `--out` / `target/listening/`；
#   - ⛔ 不 `cargo clean`、⛔ 不动共享 `target`（本文件一个 `cargo` 都不调用）；
#   - ⛔ 不声称"好听"：本文件只证明"样本被解码 + 有音高变化"，好听与否只有人能判。
#
# 退出码约定（两个入口共用）：
#   0 = 可以听（渲染成功；没有播放器也仍算成功）
#   1 = 渲染 / 自检 FAIL
#   2 = 无法判定（没有二进制 / 库根 / 清单 / python3）

LC_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export LC_REPO

# 与 `verify-samples.sh` 相同的默认库根（macOS 预设；其它平台给同形预设）。
lc_default_library() {
  case "$(uname -s)" in
    Darwin*) printf '%s\n' "$HOME/Library/Application Support/dev.yeban.app/Samples" ;;
    Linux*) printf '%s\n' "${XDG_DATA_HOME:-$HOME/.local/share}/yeban/samples" ;;
    MINGW*|MSYS*|CYGWIN*) printf '%s\n' "${APPDATA:-$HOME/AppData/Roaming}/yeban/samples" ;;
    *) printf '%s\n' "$HOME/.local/share/yeban/samples" ;;
  esac
}

# epoch 秒（macOS 用 stat -f，Linux 用 stat -c）。失败返回 1。
lc_epoch() {
  local path="$1"
  if stat -f '%m' "$path" >/dev/null 2>&1; then
    stat -f '%m' "$path"
  else
    stat -c '%Y' "$path" 2>/dev/null
  fi
}

# 把 epoch 秒格式化成人类可读；两种 date 实现都试。
lc_fmt_epoch() {
  local epoch="$1"
  date -r "$epoch" '+%Y-%m-%d %H:%M:%S' 2>/dev/null \
    || date -d "@$epoch" '+%Y-%m-%d %H:%M:%S' 2>/dev/null \
    || printf 'epoch=%s' "$epoch"
}

# 找 `yeban-app`：--app > YEBAN_APP > 本仓 target/release > target/debug > PATH。
# 找到就 echo 绝对路径；找不到返回 1。
lc_find_app() {
  local explicit="${1:-}"
  if [ -n "$explicit" ]; then
    if [ -x "$explicit" ]; then
      (cd "$(dirname "$explicit")" && printf '%s/%s\n' "$(pwd)" "$(basename "$explicit")")
      return 0
    fi
    printf '  --app 不是可执行文件: %s\n' "$explicit" >&2
    return 1
  fi
  if [ -n "${YEBAN_APP:-}" ]; then
    if [ -x "$YEBAN_APP" ]; then
      printf '%s\n' "$YEBAN_APP"
      return 0
    fi
    printf '  YEBAN_APP 不是可执行文件: %s\n' "$YEBAN_APP" >&2
    return 1
  fi
  local candidate
  for candidate in "$LC_REPO/target/release/yeban-app" "$LC_REPO/target/debug/yeban-app"; do
    if [ -x "$candidate" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  if command -v yeban-app >/dev/null 2>&1; then
    command -v yeban-app
    return 0
  fi
  return 1
}

# 新鲜的判据（**唯一**实现）：
#
#   二进制 mtime  vs  「最后一次改到源码的提交时间」(`git log -1 -- crates Cargo.toml Cargo.lock`)
#
# 为什么不是 `git log -1 --format=%ct`（整仓）：一次 docs 提交也会让整仓 HEAD 变新，
# 于是**完全没动源码**的二进制会被误报成"旧"。本函数把 `git log` 限制到源码路径，
# 既满足任务单的口径（"二进制是否比源码旧"），又不会对 docs 提交误报。
#
# 若源码有未提交改动 ⇒ 额外打一行 ⚠（无法证明二进制就是这份源码构建的），但**不**
# 因此判旧：是否重建由人决定。这样 `touch` 测试旧二进制时读数只有一个变量。
#
# 输出（逐字，判据读这些行）：
#   [新鲜度] app=<path> mtime=<epoch> (<人读时间>)
#   [新鲜度] source-commit=<sha> ct=<epoch> (<人读时间>) dirty=<yes|no>
#   [新鲜度] ✓ ...
#   [新鲜度] ⚠ ...（旧时）
#
# 全局：LC_STALE=0|1、LC_SRC_EPOCH、LC_SRC_SHA、LC_APP_EPOCH
lc_freshness() {
  local app="$1"
  LC_STALE=0
  LC_SRC_EPOCH=0
  LC_SRC_SHA=""
  LC_APP_EPOCH="$(lc_epoch "$app" 2>/dev/null || true)"
  if [ -z "$LC_APP_EPOCH" ]; then
    printf '[新鲜度] ⚠ 读不到 %s 的 mtime ⇒ 无法判断新旧\n' "$app"
    return 0
  fi
  printf '[新鲜度] app=%s mtime=%s (%s)\n' \
    "$app" "$LC_APP_EPOCH" "$(lc_fmt_epoch "$LC_APP_EPOCH")"
  if ! command -v git >/dev/null 2>&1 \
    || ! git -C "$LC_REPO" rev-parse --git-dir >/dev/null 2>&1; then
    printf '[新鲜度] ⚠ 不是 git 仓库或没有 git ⇒ 无法判断二进制是否比源码旧\n'
    return 0
  fi
  local source_paths="crates Cargo.toml Cargo.lock rust-toolchain.toml"
  # shellcheck disable=SC2086
  LC_SRC_SHA="$(git -C "$LC_REPO" log -1 --format=%h -- $source_paths 2>/dev/null || true)"
  # shellcheck disable=SC2086
  local src_ct
  src_ct="$(git -C "$LC_REPO" log -1 --format=%ct -- $source_paths 2>/dev/null || true)"
  if [ -z "$src_ct" ]; then
    printf '[新鲜度] ⚠ 拿不到源码提交时间（git log 空）⇒ 无法判断新旧\n'
    return 0
  fi
  LC_SRC_EPOCH="$src_ct"
  # shellcheck disable=SC2086
  local dirty
  dirty="$(git -C "$LC_REPO" status --porcelain=v1 -- $source_paths 2>/dev/null | head -n 1 || true)"
  if [ -n "$dirty" ]; then
    printf '[新鲜度] source-commit=%s ct=%s (%s) dirty=yes\n' \
      "$LC_SRC_SHA" "$LC_SRC_EPOCH" "$(lc_fmt_epoch "$LC_SRC_EPOCH")"
    printf '[新鲜度] ⚠ 源码有未提交改动 ⇒ 无法证明这个二进制就是这份源码构建的；\n'
    printf '[新鲜度]   如果你刚改过 crates/**，先重建：cargo build -p yeban-app --release --locked\n'
  else
    printf '[新鲜度] source-commit=%s ct=%s (%s) dirty=no\n' \
      "$LC_SRC_SHA" "$LC_SRC_EPOCH" "$(lc_fmt_epoch "$LC_SRC_EPOCH")"
  fi
  if [ "$LC_APP_EPOCH" -lt "$LC_SRC_EPOCH" ]; then
    LC_STALE=1
    printf '[新鲜度] ⚠ 二进制比源码旧：app=%s < source=%s（差 %s 秒）\n' \
      "$(lc_fmt_epoch "$LC_APP_EPOCH")" "$(lc_fmt_epoch "$LC_SRC_EPOCH")" \
      "$((LC_SRC_EPOCH - LC_APP_EPOCH))"
    printf '[新鲜度] ⚠ 你听到的可能是旧代码的声音（⛔ 本脚本不静默接受这个读数）。重建：\n'
    printf '[新鲜度]   cd %q && cargo build -p yeban-app --release --locked\n' "$LC_REPO"
    printf '[新鲜度]   然后重跑同一条命令：bash scripts/dev/play-sample.sh\n'
  else
    printf '[新鲜度] ✓ 二进制不比源码旧（app ≥ source，差 %s 秒）\n' \
      "$((LC_APP_EPOCH - LC_SRC_EPOCH))"
  fi
  return 0
}

# 从清单挑一个「已证真解码」的乐器。参数：
#   $1 库根  $2 清单  $3 请求的 instrument id（可为空）
# 输出一行：id<TAB>sfz<TAB>中文说明；失败返回 1 并把原因打到 stderr。
lc_pick_instrument() {
  local library="$1" manifest="$2" requested="${3:-}"
  python3 - "$library" "$manifest" "$requested" <<'PY'
import json
import pathlib
import sys

library = pathlib.Path(sys.argv[1])
manifest = pathlib.Path(sys.argv[2])
requested = sys.argv[3]

# ⭐ 任务单 `#205` 之后**已证真解码**的 9 个 instrument（顺序 = 本脚本的偏好）。
#    第一个是本次默认：西班牙古典吉他（48 区域，C4..E5 的短句全部命中）。
PROVEN = [
    "freepats-spanish-classical-guitar",
    "freepats-button-accordion-hn",
    "dsmolken-double-bass",
    "vsco2ce",
    "ganjo",
    "karoryfer-pastabass",
    "karoryfer-cowsynth",
    "karoryfer-272-merry-orks",
    "freepats-fsbs-dist2",
]
LABEL = {
    "freepats-spanish-classical-guitar": "西班牙古典吉他",
    "freepats-button-accordion-hn": "按钮手风琴",
    "dsmolken-double-bass": "低音提琴（arco）",
    "vsco2ce": "巴松（staccato）",
    "ganjo": "五弦班卓",
    "karoryfer-pastabass": "Pasta 贝斯",
    "karoryfer-cowsynth": "Cow 合成器",
    "karoryfer-272-merry-orks": "Ork 人声",
    "freepats-fsbs-dist2": "失真电吉他",
}

if not manifest.is_file():
    print(f"清单不存在: {manifest}", file=sys.stderr)
    raise SystemExit(1)
doc = json.loads(manifest.read_text(encoding="utf-8"))
by_id = {inst.get("id"): inst for inst in doc.get("instruments", [])}
PREFIX = "assets/samples/"


def resolve(inst):
    if not inst or not inst.get("sfz"):
        return None
    prefix = inst.get("relative_root", "")
    if prefix.startswith(PREFIX):
        prefix = prefix[len(PREFIX):]
    return library / prefix / inst["sfz"]


if requested:
    inst = by_id.get(requested)
    if inst is None:
        print(f"清单里没有 instrument `{requested}`；可用（已证解码）: {', '.join(PROVEN)}",
              file=sys.stderr)
        raise SystemExit(1)
    sfz = resolve(inst)
    if sfz is None:
        print(f"instrument `{requested}` 在清单里没有 sfz 路径", file=sys.stderr)
        raise SystemExit(1)
    if not sfz.is_file():
        print(f"instrument `{requested}` 的 SFZ 不在库根: {sfz}", file=sys.stderr)
        raise SystemExit(1)
    print(f"{requested}\t{sfz}\t{LABEL.get(requested, requested)}")
    raise SystemExit(0)

for inst_id in PROVEN:
    inst = by_id.get(inst_id)
    sfz = resolve(inst)
    if sfz is not None and sfz.is_file():
        print(f"{inst_id}\t{sfz}\t{LABEL.get(inst_id, inst_id)}")
        raise SystemExit(0)

print("库里没有任何一个「已证真解码」的 instrument（先跑 bash scripts/dev/verify-samples.sh）",
      file=sys.stderr)
raise SystemExit(1)
PY
}

# 解析一次渲染的日志。设置：
#   LC_LOADED / LC_EXPORTED / LC_REGIONS / LC_ENERGY / LC_FRAMES / LC_SOURCE
# 返回 0 = 三项判据全过（regions>0、frames>0、source=sfz(file( 且无"回退"）；
# 返回 1 = 不过，原因在 LC_REASON。
lc_parse_render() {
  local log="$1"
  LC_LOADED="$(grep -m1 '^loaded-sfz:' "$log" 2>/dev/null || true)"
  LC_EXPORTED="$(grep -m1 '^exported-wav:' "$log" 2>/dev/null || true)"
  LC_REGIONS=0
  LC_ENERGY=0
  LC_FRAMES=0
  LC_SOURCE=""
  LC_REASON=""
  if [ -z "$LC_LOADED" ]; then
    LC_REASON="没有 loaded-sfz 行（SFZ 没装载）"
    return 1
  fi
  if [ -z "$LC_EXPORTED" ]; then
    LC_REASON="没有 exported-wav 行（渲染没完成）"
    return 1
  fi
  LC_REGIONS="$(printf '%s\n' "$LC_LOADED" | sed -nE 's/.* regions=([0-9]+) .*/\1/p')"
  LC_ENERGY="$(printf '%s\n' "$LC_LOADED" \
    | sed -nE 's/.*probe\(note:vel:frames:energy\)=([^ ]+).*/\1/p' | cut -d: -f4)"
  LC_FRAMES="$(printf '%s\n' "$LC_EXPORTED" | sed -nE 's/.* frames=([0-9]+) .*/\1/p')"
  LC_SOURCE="$(printf '%s\n' "$LC_EXPORTED" | sed -E 's/.* (source=.*) from=.*/\1/')"
  [ -n "$LC_REGIONS" ] || LC_REGIONS=0
  [ -n "$LC_ENERGY" ] || LC_ENERGY=0
  [ -n "$LC_FRAMES" ] || LC_FRAMES=0
  if [ "$LC_REGIONS" -le 0 ] 2>/dev/null; then
    LC_REASON="regions=${LC_REGIONS}（SFZ 没有可播放区域）"
    return 1
  fi
  if [ "$LC_FRAMES" -le 0 ] 2>/dev/null; then
    LC_REASON="frames=${LC_FRAMES}（没有渲染出帧）"
    return 1
  fi
  if printf '%s' "$LC_SOURCE" | grep -q '回退'; then
    LC_REASON="source 里有「回退」⇒ 样本没被解码（回退到内存正弦）"
    return 1
  fi
  if ! printf '%s' "$LC_SOURCE" | grep -q 'source=sfz(file('; then
    LC_REASON="source 不是 source=sfz(file(…)) ⇒ 没证明样本文件被真读"
    return 1
  fi
  return 0
}

# 读取 WAV 头 + 峰值。设置 LC_W_BYTES / LC_W_FRAMES / LC_W_PEAK / LC_W_NORM /
# LC_W_RATE / LC_W_CH / LC_W_BITS；失败设置 LC_W_REASON 并返回 1。
lc_wav_stats() {
  local wav="$1"
  LC_W_REASON=""
  LC_W_BYTES=0
  LC_W_FRAMES=0
  LC_W_PEAK=0
  LC_W_NORM=0
  LC_W_RATE=0
  LC_W_CH=0
  LC_W_BITS=0
  if [ ! -f "$wav" ]; then
    LC_W_REASON="WAV 不存在: $wav"
    return 1
  fi
  local stats
  stats="$(python3 - "$wav" <<'PY'
import struct
import sys

path = sys.argv[1]
data = open(path, "rb").read()
if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
    print("ERR\tnot-a-riff-wave")
    raise SystemExit(0)
pos = 12
fmt = None
payload = None
while pos + 8 <= len(data):
    cid = data[pos:pos + 4]
    size = struct.unpack("<I", data[pos + 4:pos + 8])[0]
    body = data[pos + 8:pos + 8 + size]
    if cid == b"fmt " and len(body) >= 16:
        fmt = struct.unpack("<HHIIHH", body[:16])
        if fmt[0] == 0xFFFE and len(body) >= 40:
            fmt = (struct.unpack("<H", body[24:26])[0],) + fmt[1:]
    elif cid == b"data":
        payload = body
    pos += 8 + size + (size & 1)
if fmt is None or payload is None:
    print("ERR\tmissing-fmt-or-data")
    raise SystemExit(0)
tag, channels, rate, _byte_rate, _align, bits = fmt
if channels == 0 or bits == 0:
    print("ERR\tzero-channels-or-bits")
    raise SystemExit(0)
step = bits // 8
frames = len(payload) // (channels * step)
peak = 0
if tag == 3 and bits == 32:
    values = struct.iter_unpack("<f", payload[:len(payload) // 4 * 4])
    for (value,) in values:
        if abs(value) > peak:
            peak = abs(value)
    norm = peak
else:
    if bits == 16:
        values = struct.iter_unpack("<h", payload[:len(payload) // 2 * 2])
    elif bits == 24:
        raw = payload
        values = (
            (int.from_bytes(raw[i:i + 3], "little", signed=True),)
            for i in range(0, len(raw) - 2, 3)
        )
    elif bits == 32:
        values = struct.iter_unpack("<i", payload[:len(payload) // 4 * 4])
    else:
        values = ()
    for (value,) in values:
        if abs(value) > peak:
            peak = value if value >= 0 else -value
    norm = peak / float(2 ** (bits - 1)) if bits else 0.0
print("{}\t{}\t{}\t{:.6f}\t{}\t{}\t{}".format(
    len(data), frames, peak, norm, rate, channels, bits))
PY
)"
  if printf '%s' "$stats" | grep -q '^ERR'; then
    LC_W_REASON="WAV 头解析失败: $stats"
    return 1
  fi
  IFS=$'\t' read -r LC_W_BYTES LC_W_FRAMES LC_W_PEAK LC_W_NORM LC_W_RATE LC_W_CH LC_W_BITS <<<"$stats"
  return 0
}

# 分段音高证据。参数：WAV、窗长（秒，默认 0.5）、段步长（秒，默认 = 窗长）。
# 输出 `seg:` 行 + 一行 `pitch-summary:`（shell 只认 summary）。
# 设置 LC_PITCH_DISTINCT / LC_PITCH_LIST / LC_PITCH_NOTES。
lc_pitch_evidence() {
  local wav="$1" window="${2:-0.5}" step="${3:-}"
  [ -n "$step" ] || step="$window"
  local out
  out="$(python3 - "$wav" "$window" "$step" <<'PY'
import math
import struct
import sys

path = sys.argv[1]
window = float(sys.argv[2])
step = float(sys.argv[3])
data = open(path, "rb").read()
pos = 12
fmt = None
payload = None
while pos + 8 <= len(data):
    cid = data[pos:pos + 4]
    size = struct.unpack("<I", data[pos + 4:pos + 8])[0]
    body = data[pos + 8:pos + 8 + size]
    if cid == b"fmt " and len(body) >= 16:
        fmt = struct.unpack("<HHIIHH", body[:16])
        if fmt[0] == 0xFFFE and len(body) >= 40:
            fmt = (struct.unpack("<H", body[24:26])[0],) + fmt[1:]
    elif cid == b"data":
        payload = body
    pos += 8 + size + (size & 1)
if fmt is None or payload is None:
    print("pitch-summary: distinct=0 f0=[] notes=[] error=no-fmt")
    raise SystemExit(0)
tag, channels, rate, _br, _al, bits = fmt
step_bytes = bits // 8
frames = len(payload) // (channels * step_bytes)

# 单声道化（逐帧平均）；只留前 6 秒，够判一段短句。
mono = []
limit = min(frames, int(rate * 6))
for i in range(limit):
    acc = 0
    base = i * channels * step_bytes
    for c in range(channels):
        off = base + c * step_bytes
        raw = payload[off:off + step_bytes]
        if tag == 3 and bits == 32:
            val = struct.unpack("<f", raw)[0]
        else:
            val = int.from_bytes(raw, "little", signed=True) / float(1 << (bits - 1))
        acc += val
    mono.append(acc / channels)

# 12 kHz 抽头（4:1 平均）。f0 36..1000 Hz ⇒ lag 12..333。
dec = 4
if rate % dec == 0:
    ds_rate = rate // dec
else:
    ds_rate = rate / dec
small = [sum(mono[i:i + dec]) / dec for i in range(0, len(mono) - dec + 1, dec)]
lo = max(2, int(ds_rate / 1000.0))
hi = min(int(ds_rate / 36.0), 400)

names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]


def note_name(freq):
    if freq <= 0:
        return "?"
    midi = int(round(69 + 12 * math.log(freq / 440.0, 2)))
    return names[midi % 12] + str(midi // 12 - 1)


def f0_of(seg):
    if len(seg) < hi * 2:
        return 0.0, 0.0
    mean = sum(seg) / len(seg)
    # 只用 0.30 s 的中心段；去均值。
    take = min(len(seg), int(ds_rate * 0.30))
    x = [v - mean for v in seg[:take]]
    norm0 = sum(v * v for v in x) or 1.0
    scores = []
    best = 0.0
    for lag in range(lo, hi):
        acc = 0.0
        # 步长 2：判据要的是"音高变化"，不是实验室级 f0。
        for j in range(0, len(x) - lag, 2):
            acc += x[j] * x[j + lag]
        score = acc / (norm0 * (1.0 - lag / len(x)))
        scores.append(score)
        if score > best:
            best = score
    if best <= 0:
        return 0.0, 0.0
    # 自相关的倍周期峰几乎一样高 ⇒ 取「最靠前、且达到最大值 85%」的局部峰，
    # 否则 440 Hz 会被报成 40 Hz（11 倍周期，完全同相）。
    best_lag = 0
    for i, score in enumerate(scores):
        left = scores[i - 1] if i > 0 else -1.0
        right = scores[i + 1] if i + 1 < len(scores) else -1.0
        if score >= 0.85 * best and score >= left and score >= right:
            best_lag = lo + i
            break
    if best_lag == 0:
        return 0.0, 0.0
    return ds_rate / best_lag, best


t = 0.0
distinct = []
notes = []
summary = []
while t + 0.06 < len(mono) / rate:
    a = int(t * rate)
    b = min(len(mono), a + int(window * rate))
    seg = mono[a:b]
    if not seg:
        break
    rms = (sum(v * v for v in seg) / len(seg)) ** 0.5
    peak = max(abs(v) for v in seg)
    zc = sum(1 for i in range(1, len(seg)) if (seg[i - 1] < 0) != (seg[i] < 0))
    zcr = zc * rate / max(1, len(seg) - 1)
    smallseg = small[int(t * ds_rate):int(t * ds_rate) + int(window * ds_rate)]
    freq = 0.0
    if rms > 1e-4:
        freq, _ = f0_of(smallseg)
    if freq > 0 and rms > 1e-4:
        # 3% 聚类：同一个音的窗口不重复计数（音符尾音 / 两段窗重叠）。
        if not any(abs(freq - f) / f < 0.03 for f in distinct):
            distinct.append(freq)
            notes.append(note_name(freq))
    print("seg: t={:.2f} rms={:.4f} peak={:.4f} zcr={:.1f} f0={:.1f}".format(
        t, rms, peak, zcr, freq))
    summary.append("{:.1f}".format(freq) if freq else "-")
    t += step

print("pitch-summary: distinct={} f0=[{}] notes=[{}] windows=[{}]".format(
    len(distinct),
    ", ".join("{:.1f}".format(f) for f in distinct),
    ", ".join(notes),
    ", ".join(summary),
))
PY
)"
  printf '%s\n' "$out" | grep '^seg:' || true
  local summary
  summary="$(printf '%s\n' "$out" | grep '^pitch-summary:' | tail -n 1)"
  printf '%s\n' "$summary"
  LC_PITCH_DISTINCT="$(printf '%s' "$summary" | sed -nE 's/.*distinct=([0-9]+).*/\1/p')"
  LC_PITCH_LIST="$(printf '%s' "$summary" | sed -E 's/.*f0=\[([^]]*)\].*/\1/')"
  LC_PITCH_NOTES="$(printf '%s' "$summary" | sed -E 's/.*notes=\[([^]]*)\].*/\1/')"
  [ -n "$LC_PITCH_DISTINCT" ] || LC_PITCH_DISTINCT=0
}

# 播放。没有播放器 / 播放失败都**不** FAIL（渲染已经成功），只打一行"文件在哪"。
# 用 `YEBAN_AFPLAY` 可以覆盖播放器（自测"没有播放器"路径时设为不存在的路径）。
lc_play() {
  local wav="$1"
  local player="${YEBAN_AFPLAY:-afplay}"
  if [ "$player" = "none" ] || ! command -v "$player" >/dev/null 2>&1; then
    printf '无法播放（没有播放器）⇒ 文件在 %s\n' "$wav"
    return 0
  fi
  printf '▶ 播放: %s %q\n' "$player" "$wav"
  if "$player" "$wav"; then
    printf '[播放] 完成（afplay rc=0）\n'
  else
    local rc=$?
    printf '无法播放（%s rc=%s）⇒ 文件在 %s\n' "$player" "$rc" "$wav"
  fi
  return 0
}

# 造 `.yeban` 听音夹具：把 --save-as 落下的 base 容器里**第一条非 master 轨**
# 加上 `TrackV3::sampler = { path: <sfz> }`，其余条目逐字节保留。
# 这是模型里**已经存在**的字段（`#76`），本函数不发明新字段、不碰 schema。
lc_make_fixture() {
  local base="$1" fixture="$2" sfz="$3" label="${4:-SFZ 乐器}"
  python3 - "$base" "$fixture" "$sfz" "$label" <<'PY'
import json
import pathlib
import sys
import zipfile

base = pathlib.Path(sys.argv[1])
fixture = pathlib.Path(sys.argv[2])
sfz = sys.argv[3]
label = sys.argv[4] if len(sys.argv) > 4 else "SFZ 乐器"

with zipfile.ZipFile(base) as zin:
    entries = [(info.filename, zin.read(info.filename)) for info in zin.infolist()]

project = None
for name, data in entries:
    if name == "project.json":
        project = json.loads(data)

if project is None:
    print("base 容器里没有 project.json", file=sys.stderr)
    raise SystemExit(1)

master = project.get("master_bus_track_id")
target = None
for track_id, track in project.get("tracks", {}).items():
    if track_id != master:
        target = (track_id, track)
        break
if target is None:
    print("base 工程没有非 master 轨 ⇒ 无处挂 SFZ", file=sys.stderr)
    raise SystemExit(1)
track_id, track = target
track["name"] = label
track["sampler"] = {"path": sfz}

patched = json.dumps(project, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
fixture.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(fixture, "w", zipfile.ZIP_STORED) as zout:
    for name, data in entries:
        zout.writestr(name, patched if name == "project.json" else data)
print(f"fixture-track: {track_id} name={track.get('name','')} sampler={sfz}")
PY
}

# 夹具的只读验收：`--headless --open` 的读数 + `--dump-elements` 里的 sampler 路径。
# 参数：APP、FIXTURE、SFZ。设置 LC_FIXTURE_COUNTS / LC_FIXTURE_NOTES。
lc_verify_fixture() {
  local app="$1" fixture="$2" sfz="$3" log="$4"
  LC_FIXTURE_COUNTS=""
  LC_FIXTURE_NOTES=0
  "$app" --headless --open "$fixture" --dump-elements >"$log" 2>&1
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    printf '[夹具] FAIL：--headless --open rc=%s（日志 %s）\n' "$rc" "$log"
    tail -n 5 "$log" | sed 's/^/    /'
    return 1
  fi
  LC_FIXTURE_COUNTS="$(grep -m1 '^project-counts:' "$log" || true)"
  LC_FIXTURE_NOTES="$(printf '%s' "$LC_FIXTURE_COUNTS" | sed -nE 's/.* midi-notes=([0-9]+) .*/\1/p')"
  [ -n "$LC_FIXTURE_NOTES" ] || LC_FIXTURE_NOTES=0
  if [ "$LC_FIXTURE_NOTES" -le 0 ] 2>/dev/null; then
    printf '[夹具] FAIL：工程里没有音符（%s）\n' "${LC_FIXTURE_COUNTS:-没有 project-counts 行}"
    return 1
  fi
  local sampler_line
  sampler_line="$(grep -m1 'element track-[0-9]*-sampler-path ' "$log" || true)"
  if ! printf '%s' "$sampler_line" | grep -qF "$sfz"; then
    printf '[夹具] FAIL：界面投影里没有挂上 SFZ（找不到含 %s 的 sampler-path 元素）\n' "$sfz"
    return 1
  fi
  printf '[夹具] %s\n' "$LC_FIXTURE_COUNTS"
  printf '[夹具] %s\n' "$sampler_line"
  return 0
}

# 夹具的声音验收：`--open 夹具 --export-wav` 走**模型** SFZ 链条（不是 --sfz 旁路）。
# 判据：exported-wav 行含 `sfz(tracks=`、`samples=`>0、`missing=[]`、`silent=0`，且峰值 > 0。
lc_verify_fixture_audio() {
  local app="$1" fixture="$2" wav="$3" log="$4" seconds="$5"
  "$app" --headless --open "$fixture" --export-wav "$wav" --wav-seconds "$seconds" >"$log" 2>&1
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    printf '[夹具声音] FAIL：rc=%s（日志 %s）\n' "$rc" "$log"
    tail -n 5 "$log" | sed 's/^/    /'
    return 1
  fi
  local exported
  exported="$(grep -m1 '^exported-wav:' "$log" || true)"
  if [ -z "$exported" ]; then
    printf '[夹具声音] FAIL：没有 exported-wav 行\n'
    return 1
  fi
  printf '[夹具声音] %s\n' "$exported"
  if ! printf '%s' "$exported" | grep -q 'sfz(tracks='; then
    printf '[夹具声音] FAIL：导出没有走模型的 TrackV3.sampler 链条\n'
    return 1
  fi
  local samples missing silent
  samples="$(printf '%s' "$exported" | sed -nE 's/.*samples=([0-9]+).*/\1/p' | head -n 1)"
  missing="$(printf '%s' "$exported" | sed -nE 's/.*missing=\[([^]]*)\].*/\1/p' | head -n 1)"
  silent="$(printf '%s' "$exported" | sed -nE 's/.*silent=([0-9]+).*/\1/p' | head -n 1)"
  [ -n "$samples" ] || samples=0
  [ -n "$silent" ] || silent=0
  if [ "$samples" -le 0 ] 2>/dev/null; then
    printf '[夹具声音] FAIL：samples=0（模型链没有解码任何样本）\n'
    return 1
  fi
  if [ -n "$missing" ]; then
    printf '[夹具声音] FAIL：missing=[%s]（有样本找不到）\n' "$missing"
    return 1
  fi
  if [ "$silent" -ne 0 ] 2>/dev/null; then
    printf '[夹具声音] FAIL：silent=%s（有区域静音）\n' "$silent"
    return 1
  fi
  lc_wav_stats "$wav" || {
    printf '[夹具声音] FAIL：%s\n' "$LC_W_REASON"
    return 1
  }
  printf '[夹具声音] WAV frames=%s peak=%s rate=%s channels=%s bits=%s\n' \
    "$LC_W_FRAMES" "$LC_W_PEAK" "$LC_W_RATE" "$LC_W_CH" "$LC_W_BITS"
  if [ "$LC_W_FRAMES" -le 0 ] 2>/dev/null || [ "$LC_W_PEAK" -le 0 ] 2>/dev/null; then
    printf '[夹具声音] FAIL：WAV 全零 / 零帧\n'
    return 1
  fi
  return 0
}
