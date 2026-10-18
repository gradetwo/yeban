#!/usr/bin/env bash
# 夜半「一条命令听到声音」—— 纯命令行路径（任务单 `#207`）。
#
# 它做什么（五个阶段，每阶段都有可读读数）：
#   1. 新鲜度：`yeban-app` 二进制 vs 最后一次改到源码的提交（旧 ⇒ ⚠ 警告 + 重建命令）
#   2. 挑乐器：从**已证真解码**的 9 个 instrument 里挑一个（默认西班牙古典吉他）
#   3. 渲染：`--headless --sfz <sfz> --export-wav <out>`（导出音源 = 该 SFZ）
#   4. 自证：`source=sfz(file(…))`（⛔ 不是回退）+ WAV 帧/峰值 + 分段音高变化
#   5. 播放：`afplay <out>`（没有播放器 ⇒ 一行"文件在 <path>"，⛔ 不 FAIL）
#
# 用法（无参数即可）：
#   bash scripts/dev/play-sample.sh
#   # 听到一段上行的六个音：C4 E4 G4 C5 D5 E5（吉他音色）
#
# 常用选项：
#   --instrument <id>   挑另一个已证解码的乐器（见 --list）
#   --sfz <path>        直接指定 SFZ 文件（跳过清单挑选）
#   --seconds <秒>      渲染时长（默认 4；演示工程的短句在 3.5 秒内结束）
#   --out <path>        WAV 落点（默认 /tmp/yeban-listen-<乐器>.wav）
#   --no-play           只渲染 + 自证，不调用 afplay
#   --app <path>        指定 yeban-app（默认 target/release → target/debug → PATH）
#   --library <dir>     指定采样库根（默认 macOS 预设路径）
#   --selftest          不渲染，只跑本脚本的判据自测（注入自证用）
#   -h, --help          本帮助
#
# 退出码：0 = 渲染成功（能听/文件在）；1 = 渲染或判据 FAIL；2 = 无法判定。
#
# 它**不**做什么：⛔ 不写库根、⛔ 不改工程、⛔ 不 cargo clean、⛔ 不声称"好听"。
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=./listening-common.sh
source "$REPO/scripts/dev/listening-common.sh"

usage() {
  sed -n '2,34p' "$0" | sed 's/^# \{0,1\}//'
}

APP_ARG=""
SFZ_ARG=""
INSTRUMENT=""
LIBRARY=""
OUT=""
SECONDS_TO_RENDER=4
NO_PLAY=0
KEEP=0
SELFTEST=0
WORKDIR=""
OWN_WORKDIR=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --instrument) [ "$#" -ge 2 ] || { echo "缺少 --instrument 的值" >&2; exit 2; }; INSTRUMENT="$2"; shift 2 ;;
    --sfz) [ "$#" -ge 2 ] || { echo "缺少 --sfz 的值" >&2; exit 2; }; SFZ_ARG="$2"; shift 2 ;;
    --app) [ "$#" -ge 2 ] || { echo "缺少 --app 的值" >&2; exit 2; }; APP_ARG="$2"; shift 2 ;;
    --library) [ "$#" -ge 2 ] || { echo "缺少 --library 的值" >&2; exit 2; }; LIBRARY="$2"; shift 2 ;;
    --out) [ "$#" -ge 2 ] || { echo "缺少 --out 的值" >&2; exit 2; }; OUT="$2"; shift 2 ;;
    --seconds) [ "$#" -ge 2 ] || { echo "缺少 --seconds 的值" >&2; exit 2; }; SECONDS_TO_RENDER="$2"; shift 2 ;;
    --no-play) NO_PLAY=1; shift ;;
    --keep) KEEP=1; shift ;;
    --selftest) SELFTEST=1; shift ;;
    --list)
      LIBRARY="${LIBRARY:-$(lc_default_library)}"
      lc_pick_instrument "$LIBRARY" "$REPO/assets/samples/manifest.json" "${INSTRUMENT:-}" >/dev/null 2>&1 || true
      # 直接列出 9 个偏好（存在与否分别标注）。
      python3 - "$LIBRARY" "$REPO/assets/samples/manifest.json" <<'PY'
import json, pathlib, sys
library = pathlib.Path(sys.argv[1]); manifest = pathlib.Path(sys.argv[2])
doc = json.loads(manifest.read_text(encoding="utf-8"))
by_id = {i.get("id"): i for i in doc.get("instruments", [])}
PREFIX = "assets/samples/"
PROVEN = ["freepats-spanish-classical-guitar","freepats-button-accordion-hn",
          "dsmolken-double-bass","vsco2ce","ganjo","karoryfer-pastabass",
          "karoryfer-cowsynth","karoryfer-272-merry-orks","freepats-fsbs-dist2"]
for i in PROVEN:
    inst = by_id.get(i) or {}
    prefix = inst.get("relative_root","")
    if prefix.startswith(PREFIX): prefix = prefix[len(PREFIX):]
    sfz = library / prefix / inst["sfz"] if inst.get("sfz") else None
    print(("OK  " if sfz and sfz.is_file() else "MISS"), i)
PY
      exit 0
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# 自测：不碰库、不碰二进制，只喂合成输入给本脚本的判据函数。
# 注入自证：把 lc_parse_render 的「回退」判定改坏 ⇒ 第 2 条 FAIL。
# ---------------------------------------------------------------------------
if [ "$SELFTEST" = 1 ]; then
  TMP="$(mktemp -d "${TMPDIR:-/tmp}/yeban-listen-selftest.XXXXXX")" || exit 2
  trap 'rm -rf "$TMP"' EXIT
  PASS=0; FAIL=0
  ck() { if [ "$1" = 0 ]; then PASS=$((PASS+1)); printf '  PASS  %s\n' "$2"; else FAIL=$((FAIL+1)); printf '  FAIL  %s\n' "$2"; fi; }

  cat >"$TMP/good.log" <<'EOF'
loaded-sfz: path=/x/guitar.sfz bytes=2450 regions=48 sample_source=in-memory-sine(440Hz) probe(note:vel:frames:energy)=60:100:23999:1153.1989 wired_to_export=yes
exported-wav: path=/tmp/x.wav bytes=1152068 frames=192000 sample_rate=48000 channels=2 bits=24 digest=abc source=sfz(file(/x/samples/C4.wav frames=106627 rate=44100)) from=sample=default
EOF
  cat >"$TMP/fallback.log" <<'EOF'
loaded-sfz: path=/x/guitar.sfz bytes=2450 regions=48 sample_source=in-memory-sine(440Hz) probe(note:vel:frames:energy)=60:100:23999:1153.1989 wired_to_export=yes
exported-wav: path=/tmp/x.wav bytes=1152068 frames=192000 sample_rate=48000 channels=2 bits=24 digest=abc source=sfz(in-memory-sine(440Hz, 回退: 样本读取失败: not found)) from=sample=default
EOF
  cat >"$TMP/nofile.log" <<'EOF'
loaded-sfz: path=/x/guitar.sfz bytes=2450 regions=48 sample_source=in-memory-sine(440Hz)
exported-wav: path=/tmp/x.wav bytes=1152068 frames=192000 sample_rate=48000 channels=2 bits=24 digest=abc source=in-memory-sine(440Hz) from=sample=default
EOF
  lc_parse_render "$TMP/good.log"; ck $? "真解码日志被接受（${LC_REASON}）"
  lc_parse_render "$TMP/fallback.log"; rc=$?; [ "$rc" = 1 ] && printf '%s' "$LC_REASON" | grep -q '回退'; ck $? "回退日志被拒绝（${LC_REASON}）"
  lc_parse_render "$TMP/nofile.log"; rc=$?; [ "$rc" = 1 ]; ck $? "非 file( 的 source 被拒绝（${LC_REASON}）"

  python3 - "$TMP" <<'PY'
import math, struct, sys, pathlib
tmp = pathlib.Path(sys.argv[1])
def wav(path, segs, rate=48000):
    data = bytearray()
    for freq, dur in segs:
        for i in range(int(rate*dur)):
            v = 0 if freq == 0 else int(12000*math.sin(2*math.pi*freq*i/rate))
            data += struct.pack('<h', v)
    hdr = b'RIFF' + struct.pack('<I', 36+len(data)) + b'WAVEfmt ' + struct.pack('<IHHIIHH', 16,1,1,rate,rate*2,2,16) + b'data' + struct.pack('<I', len(data))
    path.write_bytes(hdr+data)
wav(tmp/'silent.wav', [(0,0.2)])
wav(tmp/'tones.wav', [(440,0.2),(880,0.2)])
PY
  lc_wav_stats "$TMP/tones.wav"; ck $? "合成 WAV 头可解析（frames=${LC_W_FRAMES} peak=${LC_W_PEAK}）"
  lc_pitch_evidence "$TMP/tones.wav" 0.2 0.2 >/dev/null
  [ "${LC_PITCH_DISTINCT:-0}" -ge 2 ]; ck $? "合成 440/880Hz 被认出 2 个音高（distinct=$LC_PITCH_DISTINCT f0=[$LC_PITCH_LIST]）"
  lc_wav_stats "$TMP/silent.wav"
  [ "$LC_W_PEAK" = 0 ]; ck $? "全零 WAV 的 peak=0（默认可判）"

  # 新鲜度：造一个 2000 年的假二进制 ⇒ 必须判旧；一个现在的 ⇒ 必须判不旧。
  OLD="$TMP/old-app"; NEW="$TMP/new-app"; : >"$OLD"; : >"$NEW"
  touch -t 200001010000 "$OLD"
  lc_freshness "$OLD" >"$TMP/fresh-old.log" 2>&1
  [ "$LC_STALE" = 1 ] && grep -q '⚠ 二进制比源码旧' "$TMP/fresh-old.log"; ck $? "旧二进制被判旧（LC_STALE=${LC_STALE}）"
  lc_freshness "$NEW" >"$TMP/fresh-new.log" 2>&1
  [ "$LC_STALE" = 0 ]; ck $? "新二进制不被误判（LC_STALE=${LC_STALE}）"

  printf '\n  SELFTEST: PASS %d | FAIL %d\n' "$PASS" "$FAIL"
  [ "$FAIL" = 0 ] || exit 1
  exit 0
fi

# ---------------------------------------------------------------------------
# 工作目录 / 清理
# ---------------------------------------------------------------------------
if [ -z "$WORKDIR" ]; then
  WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/yeban-listen.XXXXXX")" || { echo "无法创建临时目录" >&2; exit 2; }
  OWN_WORKDIR=1
fi
cleanup() { if [ "$OWN_WORKDIR" = 1 ] && [ "$KEEP" = 0 ]; then rm -rf "$WORKDIR"; fi; }
trap cleanup EXIT

step() { printf '\n== [%s] %s ==\n' "$1" "$2"; }
note() { printf '  · %s\n' "$*"; }

# ---------------------------------------------------------------------------
# [1/5] 新鲜度
# ---------------------------------------------------------------------------
step "1/5" "二进制新鲜度"
APP="$(lc_find_app "$APP_ARG" 2>"$WORKDIR/app.err")"
APP_RC=$?
if [ -z "$APP" ] || [ "$APP_RC" -ne 0 ]; then
  cat "$WORKDIR/app.err" 2>/dev/null | sed 's/^/  /'
  note "没有可执行的 yeban-app。重建："
  note "  cd $REPO && cargo build -p yeban-app --release --locked"
  exit 2
fi
printf '  app: %s\n' "$APP"
lc_freshness "$APP"

# ---------------------------------------------------------------------------
# [2/5] 挑乐器
# ---------------------------------------------------------------------------
step "2/5" "挑一个已证真解码的乐器"
LIBRARY="${LIBRARY:-$(lc_default_library)}"
printf '  library: %s\n' "$LIBRARY"
if [ ! -d "$LIBRARY" ]; then
  note "库根不存在 ⇒ 先跑 bash scripts/dev/fetch-samples.py（或 verify-samples.sh 的提示）"
  exit 2
fi
SFZ=""
INSTRUMENT_ID=""
LABEL=""
if [ -n "$SFZ_ARG" ]; then
  if [ ! -f "$SFZ_ARG" ]; then
    note "SFZ 不存在: $SFZ_ARG"
    exit 2
  fi
  SFZ="$SFZ_ARG"
  INSTRUMENT_ID="(explicit)"
  LABEL="(命令行指定)"
  printf '  sfz: %s\n' "$SFZ"
else
  PICK="$(lc_pick_instrument "$LIBRARY" "$REPO/assets/samples/manifest.json" "$INSTRUMENT" 2>"$WORKDIR/pick.err")"
  PICK_RC=$?
  if [ -z "$PICK" ] || [ "$PICK_RC" -ne 0 ]; then
    sed 's/^/  /' "$WORKDIR/pick.err" 2>/dev/null
    note "无法挑出乐器（可用: bash scripts/dev/play-sample.sh --list）"
    exit 2
  fi
  IFS=$'\t' read -r INSTRUMENT_ID SFZ LABEL <<<"$PICK"
  printf '  instrument: %s（%s）\n' "$INSTRUMENT_ID" "$LABEL"
  printf '  sfz: %s\n' "$SFZ"
fi
[ -n "$OUT" ] || OUT="/tmp/yeban-listen-${INSTRUMENT_ID#freepats-}.wav"
mkdir -p "$(dirname "$OUT")" 2>/dev/null || true
OUT="$(cd "$(dirname "$OUT")" 2>/dev/null && pwd)/$(basename "$OUT")"
rm -f "$OUT"

# ---------------------------------------------------------------------------
# [3/5] 渲染
# ---------------------------------------------------------------------------
step "3/5" "渲染"
LOG="$WORKDIR/render.log"
printf '  command: %q --headless --sfz %q --export-wav %q --wav-seconds %s\n' \
  "$APP" "$SFZ" "$OUT" "$SECONDS_TO_RENDER"
"$APP" --headless --sfz "$SFZ" --export-wav "$OUT" --wav-seconds "$SECONDS_TO_RENDER" \
  >"$LOG" 2>&1
RENDER_RC=$?
if [ "$RENDER_RC" -ne 0 ]; then
  note "渲染失败（rc=${RENDER_RC}）；日志尾部:"
  tail -n 8 "$LOG" | sed 's/^/    /'
  exit 1
fi
grep -E '^loaded-sfz:|^exported-wav:|^headless ok' "$LOG" | sed 's/^/  /'

# ---------------------------------------------------------------------------
# [4/5] 自证：真解码 + 非静音 + 音高变化
# ---------------------------------------------------------------------------
step "4/5" "自证（不是回退 / 有声音 / 音高在变）"
RC=0
if lc_parse_render "$LOG"; then
  printf '  PASS  regions=%s energy=%s frames=%s（source=sfz(file(…)，无回退）\n' \
    "$LC_REGIONS" "$LC_ENERGY" "$LC_FRAMES"
  printf '  source（逐字）: %s\n' "$LC_SOURCE"
else
  printf '  FAIL  渲染日志判据: %s\n' "$LC_REASON"
  RC=1
fi
if lc_wav_stats "$OUT"; then
  printf '  PASS  WAV bytes=%s frames=%s peak=%s norm=%s rate=%s channels=%s bits=%s\n' \
    "$LC_W_BYTES" "$LC_W_FRAMES" "$LC_W_PEAK" "$LC_W_NORM" "$LC_W_RATE" "$LC_W_CH" "$LC_W_BITS"
  if [ "$LC_W_FRAMES" -le 0 ] 2>/dev/null || [ "$LC_W_PEAK" -le 0 ] 2>/dev/null; then
    printf '  FAIL  WAV 是全零 / 零帧 ⇒ 听不到声音\n'
    RC=1
  fi
else
  printf '  FAIL  %s\n' "$LC_W_REASON"
  RC=1
fi
printf '  分段音高（窗 0.5 s，演示工程的短句每 0.5 s 一个音）：\n'
lc_pitch_evidence "$OUT" 0.5 0.5 >"$WORKDIR/pitch.log"
sed 's/^/    /' "$WORKDIR/pitch.log"
if [ "${LC_PITCH_DISTINCT:-0}" -ge 3 ]; then
  printf '  PASS  音高变化：%s 个不同的音（%s）\n' "$LC_PITCH_DISTINCT" "$LC_PITCH_NOTES"
elif [ "${LC_PITCH_DISTINCT:-0}" -ge 2 ]; then
  printf '  WARN  只认出 %s 个不同的音（%s）—— 仍能听，但旋律不明显\n' \
    "$LC_PITCH_DISTINCT" "$LC_PITCH_NOTES"
else
  printf '  FAIL  听不出音高变化（distinct=%s）\n' "${LC_PITCH_DISTINCT:-0}"
  RC=1
fi

# ---------------------------------------------------------------------------
# [5/5] 播放
# ---------------------------------------------------------------------------
step "5/5" "播放"
printf '  WAV: %s（%s 帧 / %s Hz / %s 声道）\n' \
  "$OUT" "$LC_W_FRAMES" "$LC_W_RATE" "$LC_W_CH"
if [ "${LC_PITCH_DISTINCT:-0}" -ge 3 ]; then
  printf '  你会听到：%s 个音，音高依次为 %s（每 0.5 秒一个，吉他/所选乐器音色）\n' \
    "$LC_PITCH_DISTINCT" "$LC_PITCH_NOTES"
fi
if [ "$RC" -ne 0 ]; then
  printf '\n  VERDICT: FAIL（渲染/判据没过；文件在 %s）\n' "$OUT"
  exit 1
fi
if [ "$NO_PLAY" = 1 ]; then
  printf '未播放（--no-play）⇒ 文件在 %s\n' "$OUT"
  exit 0
fi
lc_play "$OUT"
printf '\n  VERDICT: PASS（渲染并自证成功；"好不好听"只有人能判）\n'
exit 0
