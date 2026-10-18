#!/usr/bin/env bash
# 夜半「GUI 打开即听」—— 一条命令准备好工程并拉起图形界面（任务单 `#207`）。
#
# 它做什么：
#   1. 新鲜度：与 play-sample.sh 共用同一份判定（旧二进制 ⇒ ⚠ + 重建命令）
#   2. 造 `.yeban` 夹具：内置演示工程（6 个上行音）＋ 第一条非 master 轨挂上 SFZ
#      （`TrackV3::sampler`，模型里**已有**的字段；不是新发明的格式）
#   3. 无头验收：`--headless --open` 的 `project-counts:`（音符数）
#      ＋ `--dump-elements` 的 `track-0-sampler-path`（挂的是哪份 SFZ）
#   4. 声音验收：`--open 夹具 --export-wav` 走**模型** SFZ 链条（不是 --sfz 旁路），
#      读数里出现 `sfz(tracks=1 … samples=48 silent=0 missing=[])`
#   5. 拉起 GUI：有 `.app` bundle ⇒ `open -n -a … --args --open <夹具>`；
#      没有 bundle ⇒ 直接运行同一个二进制（同样是 GUI，命令会逐字打印）
#
# 负责人只需：跑下面这一条命令，然后在窗口里按一次空格（播放）。
#
# 用法：
#   bash scripts/dev/play-sample-gui.sh
#   bash scripts/dev/play-sample-gui.sh --no-launch   # 只造夹具并验收，不开窗
#
# 选项：
#   --instrument <id>  已证解码的乐器（默认西班牙古典吉他）
#   --sfz <path>       直接指定 SFZ
#   --fixture <path>   夹具落点（默认 target/listening/yeban-listening.yeban）
#   --app <path>       指定 yeban-app
#   --library <dir>    指定采样库根
#   --bundle <path>    用哪个 `.app`（默认 target/Yeban.app → /Applications/Yeban.app）
#   --no-launch        只准备 + 验收，不启动 GUI
#   --keep             保留临时目录
#   -h, --help         本帮助
#
# 退出码：0 = 夹具就绪且已（或按 --no-launch 未）拉起；1 = 夹具/验收/GUI 启动 FAIL；
#         2 = 无法判定（没有二进制 / 库根 / 清单）。
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=./listening-common.sh
source "$REPO/scripts/dev/listening-common.sh"

usage() { sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; }

APP_ARG=""
SFZ_ARG=""
INSTRUMENT=""
LIBRARY=""
FIXTURE=""
BUNDLE=""
NO_LAUNCH=0
KEEP=0
WORKDIR=""
OWN_WORKDIR=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --instrument) [ "$#" -ge 2 ] || { echo "缺少 --instrument 的值" >&2; exit 2; }; INSTRUMENT="$2"; shift 2 ;;
    --sfz) [ "$#" -ge 2 ] || { echo "缺少 --sfz 的值" >&2; exit 2; }; SFZ_ARG="$2"; shift 2 ;;
    --app) [ "$#" -ge 2 ] || { echo "缺少 --app 的值" >&2; exit 2; }; APP_ARG="$2"; shift 2 ;;
    --library) [ "$#" -ge 2 ] || { echo "缺少 --library 的值" >&2; exit 2; }; LIBRARY="$2"; shift 2 ;;
    --fixture) [ "$#" -ge 2 ] || { echo "缺少 --fixture 的值" >&2; exit 2; }; FIXTURE="$2"; shift 2 ;;
    --bundle) [ "$#" -ge 2 ] || { echo "缺少 --bundle 的值" >&2; exit 2; }; BUNDLE="$2"; shift 2 ;;
    --no-launch) NO_LAUNCH=1; shift ;;
    --keep) KEEP=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数: $1" >&2; usage >&2; exit 2 ;;
  esac
done

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/yeban-listen-gui.XXXXXX")" || { echo "无法创建临时目录" >&2; exit 2; }
OWN_WORKDIR=1
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
  sed 's/^/  /' "$WORKDIR/app.err" 2>/dev/null
  note "没有可执行的 yeban-app。重建：cd $REPO && cargo build -p yeban-app --release --locked"
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
  note "库根不存在 ⇒ 先跑 bash scripts/dev/verify-samples.sh"
  exit 2
fi
SFZ=""
INSTRUMENT_ID=""
LABEL=""
if [ -n "$SFZ_ARG" ]; then
  [ -f "$SFZ_ARG" ] || { note "SFZ 不存在: $SFZ_ARG"; exit 2; }
  SFZ="$SFZ_ARG"; INSTRUMENT_ID="(explicit)"; LABEL="(命令行指定)"
else
  PICK="$(lc_pick_instrument "$LIBRARY" "$REPO/assets/samples/manifest.json" "$INSTRUMENT" 2>"$WORKDIR/pick.err")"
  PICK_RC=$?
  if [ -z "$PICK" ] || [ "$PICK_RC" -ne 0 ]; then
    sed 's/^/  /' "$WORKDIR/pick.err" 2>/dev/null
    note "无法挑出乐器（可用: bash scripts/dev/play-sample.sh --list）"
    exit 2
  fi
  IFS=$'\t' read -r INSTRUMENT_ID SFZ LABEL <<<"$PICK"
fi
printf '  instrument: %s（%s）\n' "$INSTRUMENT_ID" "$LABEL"
printf '  sfz: %s\n' "$SFZ"

# ---------------------------------------------------------------------------
# [3/5] 造 `.yeban` 夹具
# ---------------------------------------------------------------------------
step "3/5" "造 .yeban 夹具（演示工程 + TrackV3.sampler）"
[ -n "$FIXTURE" ] || FIXTURE="$REPO/target/listening/yeban-listening.yeban"
mkdir -p "$(dirname "$FIXTURE")"
BASE="$WORKDIR/base.yeban"
if ! "$APP" --headless --save-as "$BASE" >"$WORKDIR/save.log" 2>&1; then
  note "保存演示工程失败（日志 $WORKDIR/save.log）"
  tail -n 5 "$WORKDIR/save.log" | sed 's/^/    /'
  exit 1
fi
printf '  %s\n' "$(grep -m1 '^saved:' "$WORKDIR/save.log")"
if ! lc_make_fixture "$BASE" "$FIXTURE" "$SFZ" "$LABEL" >"$WORKDIR/patch.log" 2>&1; then
  note "给工程挂 SFZ 失败（日志 $WORKDIR/patch.log）"
  sed 's/^/    /' "$WORKDIR/patch.log"
  exit 1
fi
sed 's/^/  /' "$WORKDIR/patch.log"
printf '  fixture: %s\n' "$FIXTURE"
stat -f '  fixture-bytes: %z' "$FIXTURE" 2>/dev/null || stat -c '  fixture-bytes: %s' "$FIXTURE"

# ---------------------------------------------------------------------------
# [4/5] 无头验收：工程里有音符 + 挂的是这份 SFZ + 模型链条真的解码
# ---------------------------------------------------------------------------
step "4/5" "无头验收（音符 / 乐器 / 声音）"
if ! lc_verify_fixture "$APP" "$FIXTURE" "$SFZ" "$WORKDIR/open.log"; then
  exit 1
fi
AUDIO_WAV="$WORKDIR/fixture.wav"
if ! lc_verify_fixture_audio "$APP" "$FIXTURE" "$AUDIO_WAV" "$WORKDIR/fixture.log" 4; then
  exit 1
fi
printf '  （上面那行只证明「模型的 TrackV3.sampler 链条解码出了样本」；离线导出路径\n'
printf '    对多音符有既存缺陷，所以旋律读数不取它，取下面的 --export-midi。）\n'
# 旋律 / 音高：用同一份工程的 MIDI 导出（与 GUI 里的音序同源，且不受离线渲染缺陷影响）。
if "$APP" --headless --open "$FIXTURE" --export-midi "$WORKDIR/fixture.mid" \
    >"$WORKDIR/midi.log" 2>&1; then
  grep '^exported-midi:' "$WORKDIR/midi.log" | sed 's/^/  /'
  python3 - "$WORKDIR/fixture.mid" <<'PY'
import struct
import sys

data = open(sys.argv[1], "rb").read()
if data[:4] != b"MThd":
    print("  FAIL  MIDI 头不是 MThd")
    raise SystemExit(1)
pos = 14
notes = []
while pos + 8 <= len(data):
    cid = data[pos:pos + 4]
    size = struct.unpack(">I", data[pos + 4:pos + 8])[0]
    body = data[pos + 8:pos + 8 + size]
    pos += 8 + size
    if cid != b"MTrk":
        continue
    i = 0
    tick = 0
    while i < len(body):
        delta = 0
        while True:
            b = body[i]
            i += 1
            delta = (delta << 7) | (b & 0x7F)
            if not b & 0x80:
                break
        tick += delta
        status = body[i]
        if status == 0xFF:
            meta = body[i + 1]
            i += 2
            length = 0
            while True:
                b = body[i]
                i += 1
                length = (length << 7) | (b & 0x7F)
                if not b & 0x80:
                    break
            i += length
            if meta == 0x2F:
                break
        elif status & 0xF0 == 0x90:
            pitch, velocity = body[i + 1], body[i + 2]
            i += 3
            if velocity:
                notes.append((tick, pitch))
        elif status & 0xF0 == 0x80:
            i += 3
        elif status & 0xF0 in (0xA0, 0xB0, 0xE0):
            i += 3
        elif status & 0xF0 in (0xC0, 0xD0):
            i += 2
        else:
            break
names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
pieces = []
for tick, pitch in notes:
    pieces.append((tick, names[pitch % 12] + str(pitch // 12 - 1)))
distinct = []
for _tick, name in pieces:
    if name not in distinct:
        distinct.append(name)
print("  midi-notes: count={} distinct={} order=[{}] (tick,pitch)=[{}]".format(
    len(pieces), len(distinct), ", ".join(distinct),
    ", ".join("{},{}".format(t, p) for t, p in notes)))
if len(distinct) < 3:
    print("  FAIL  MIDI 里只有 {} 个不同的音高 ⇒ 不是短句".format(len(distinct)))
    raise SystemExit(1)
PY
  if [ "$?" -ne 0 ]; then
    exit 1
  fi
  printf '  PASS  工程里的音符构成上行短句（见上面的 order）\n'
else
  printf '  FAIL  --export-midi 失败（日志 %s）\n' "$WORKDIR/midi.log"
  tail -n 5 "$WORKDIR/midi.log" | sed 's/^/    /'
  exit 1
fi

# ---------------------------------------------------------------------------
# [5/5] 拉起 GUI
# ---------------------------------------------------------------------------
step "5/5" "拉起 GUI"
if [ -z "$BUNDLE" ]; then
  for candidate in "$REPO/target/Yeban.app" "/Applications/Yeban.app"; do
    if [ -d "$candidate" ]; then BUNDLE="$candidate"; break; fi
  done
fi
# ⚠ `open` 跑的是 bundle 里的副本，不是 $APP ⇒ 新鲜度必须**也**问它一遍。
if [ -n "$BUNDLE" ]; then
  BUNDLE_EXE="$BUNDLE/Contents/MacOS/yeban"
  if [ -x "$BUNDLE_EXE" ]; then
    printf '  bundle-exe: %s\n' "$BUNDLE_EXE"
    lc_freshness "$BUNDLE_EXE"
    if [ "$LC_STALE" = 1 ]; then
      printf '  ⚠ bundle 里的二进制比源码旧 ⇒ 重跑 bash scripts/dev/bundle-macos.sh（重建 + 重新装配）\n'
    fi
  else
    printf '  ⚠ bundle 里没有可执行文件: %s ⇒ 改走直接运行二进制\n' "$BUNDLE_EXE"
    BUNDLE=""
  fi
fi
if [ "$NO_LAUNCH" = 1 ]; then
  printf '  （--no-launch）夹具已就绪。开窗命令：\n'
  if [ -n "$BUNDLE" ]; then
    printf '    open -n -a %q --args --open %q\n' "$BUNDLE" "$FIXTURE"
  else
    printf '    %q --open %q\n' "$APP" "$FIXTURE"
  fi
  printf '\n  VERDICT: PASS（夹具与声音已验；开窗/按播放是人的一步）\n'
  exit 0
fi
if [ -n "$BUNDLE" ] && command -v open >/dev/null 2>&1; then
  printf '  command: open -n -a %q --args --open %q\n' "$BUNDLE" "$FIXTURE"
  if open -n -a "$BUNDLE" --args --open "$FIXTURE"; then
    printf '  GUI 已启动（open rc=0）。在窗口里按一次空格（播放）。\n'
  else
    rc=$?
    printf '  FAIL  open 返回 %s；可以直接运行：%q --open %q\n' "$rc" "$APP" "$FIXTURE"
    exit 1
  fi
else
  printf '  没有 .app bundle（%s）⇒ 直接运行同一个二进制（GUI 窗口一样会开）。\n' "$REPO/target/Yeban.app"
  printf '  想用 `open`：先跑 bash scripts/dev/bundle-macos.sh 生成 target/Yeban.app（只重建一次），\n'
  printf '  之后再跑本脚本就会自动走 `open -n -a … --args --open …`。\n'
  printf '  command: %q --open %q\n' "$APP" "$FIXTURE"
  "$APP" --open "$FIXTURE" >"$(dirname "$FIXTURE")/gui.log" 2>&1 &
  GUI_PID=$!
  sleep 2
  if kill -0 "$GUI_PID" 2>/dev/null; then
    printf '  GUI 已启动（pid=%s）。在窗口里按一次空格（播放）。\n' "$GUI_PID"
    printf '  （本进程会继续运行；关掉窗口即退出。日志: %s）\n' "$(dirname "$FIXTURE")/gui.log"
  else
    wait "$GUI_PID"; rc=$?
    printf '  FAIL  二进制启动后立即退出（rc=%s）；日志尾部:\n' "$rc"
    tail -n 8 "$(dirname "$FIXTURE")/gui.log" | sed 's/^/    /'
    exit 1
  fi
fi
printf '\n  VERDICT: PASS（工程/声音/开窗都做了；"好不好听"只有人能判）\n'
if [ -n "$BUNDLE" ]; then
  printf '  replay: open -n -a %q --args --open %q\n' "$BUNDLE" "$FIXTURE"
else
  printf '  replay: %q --open %q\n' "$APP" "$FIXTURE"
fi
printf '  声音（不进 GUI）: bash scripts/dev/play-sample.sh\n'
exit 0
