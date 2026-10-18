#!/usr/bin/env bash
# 夜半采样库 —— **一条命令**验收（任务单 #203）。
#
# 为什么需要它（负责人原话）:
#   "采样下载好之后怎么测试？详细命令、能自动化的都自动化、让我少点操作。"
#   本脚本把"库根 → 清单 → 挑完整 instrument → SFZ 探测 → WAV 自检 → 阴性对照 → 总结"
#   七步串成一条命令；每一步都打印 PASS／FAIL 与一行数字；退出码 0/1/2。
#
# 用法（无参数即可）:
#   bash scripts/dev/verify-samples.sh
#   echo $?        # 0 = 全过；1 = 有失败；2 = 无法判定
#
# 可选参数（都不是必需的）:
#   --library DIR    库根（缺省 = ${YEBAN_SAMPLE_LIBRARY}；再缺省 = 程序预设路径）
#   --manifest FILE  清单（缺省 = $YEBAN_SAMPLES_MANIFEST；再缺省 = 仓库清单；仅自测/fixture 用）
#   --app PATH       yeban-app 二进制（缺省 = $YEBAN_APP → target/release → target/debug → cargo 构建）
#   --workdir DIR    工作目录（缺省 = 系统临时目录；成功时自动删除）
#   --keep           保留工作目录（失败排查用）
#
# ⚠ 一条**必须写下来的事实**（本脚本因此比"只看能量"更严）:
#   `--sfz` 的 `probe(note:vel:frames:energy)` 用的是 **内存正弦**，不是样本文件
#   （`crates/yeban-app/src/sfz_load.rs`: `probe_note_energy` → `InMemorySample::sine`）。
#   所以 `energy > 0` **不能**证明采样被解码：样本全缺时它照样 > 0，而且可能更大。
#   本脚本的判据因此是三条一起：
#     ① regions > 0；② energy > 0；③ `exported-wav` 的 `source=` 里出现 `file(`
#        且**没有** `回退`（`in-memory-sine(440Hz, 回退: 样本读取失败: …)` 就是没解码）。
#   第③条才是"样本真的出声"的承重条件；第①/②条是任务单要求的读数门。
#
# 退出口径（与 fetch-samples.py / check_vendor.sh 同一纪律）:
#   0 = 六步全部 PASS（SKIP 不阻断：2 个无上游坐标的 instrument 共 52 项永远拿不到）
#   1 = 至少一步 FAIL（缺件 / 字节不符 / 样本没解码 / WAV 全零 / 阴性对照竟然成功）
#   2 = 无法判定（库根/清单/二进制不存在，拿不到可判定的读数）
#
# 本脚本只读：⛔ 不下载、⛔ 不写库根、⛔ 不改清单；临时文件只在 --workdir。
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO" || exit 2

if ! command -v python3 >/dev/null 2>&1; then
  echo "需要 python3（清单解析 / WAV 头解析）" >&2
  exit 2
fi

MANIFEST="${YEBAN_SAMPLES_MANIFEST:-$REPO/assets/samples/manifest.json}"
LIBRARY="${YEBAN_SAMPLE_LIBRARY:-}"
APP="${YEBAN_APP:-}"
WORKDIR=""
KEEP=0
OWN_WORKDIR=0
MANIFEST_IS_FIXTURE=0
LIBRARY_SOURCE="preset"
MANIFEST_SOURCE="repo"
if [ -n "${YEBAN_SAMPLES_MANIFEST:-}" ]; then
  MANIFEST_IS_FIXTURE=1
  MANIFEST_SOURCE="env(YEBAN_SAMPLES_MANIFEST)"
fi

PASS_N=0
FAIL_N=0
SKIP_N=0
CAND_SKIP_N=0
UNKNOWN=0
FINALIZED=0
SELECTED_ID=""
SELECTED_SFZ=""
SELECTED_REGIONS=""
SELECTED_ENERGY=""

usage() {
  cat <<'EOF'
用法: bash scripts/dev/verify-samples.sh [选项]

一条命令验收采样库：库根 → 清单 → 完整 instrument → SFZ 探测 → WAV 自检 → 阴性对照 → 总结。

选项:
  --library DIR    库根（缺省 = ${YEBAN_SAMPLE_LIBRARY}；再缺省 = 程序预设路径）
  --manifest FILE  清单（缺省 = $YEBAN_SAMPLES_MANIFEST；再缺省 = assets/samples/manifest.json；仅自测/fixture 用）
  --app PATH       yeban-app 二进制（缺省 = $YEBAN_APP / target/release / target/debug / cargo 构建）
  --workdir DIR    工作目录（缺省 = 系统临时目录）
  --keep           保留工作目录
  -h, --help       本帮助

退出码: 0 全过 / 1 有失败 / 2 无法判定
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --library)
      [ "$#" -ge 2 ] || { echo "缺少 --library 的值" >&2; exit 2; }
      LIBRARY="$2"
      LIBRARY_SOURCE="--library"
      shift 2
      ;;
    --manifest)
      [ "$#" -ge 2 ] || { echo "缺少 --manifest 的值" >&2; exit 2; }
      MANIFEST="$2"
      MANIFEST_SOURCE="--manifest"
      MANIFEST_IS_FIXTURE=1
      shift 2
      ;;
    --app)
      [ "$#" -ge 2 ] || { echo "缺少 --app 的值" >&2; exit 2; }
      APP="$2"
      shift 2
      ;;
    --workdir)
      [ "$#" -ge 2 ] || { echo "缺少 --workdir 的值" >&2; exit 2; }
      WORKDIR="$2"
      shift 2
      ;;
    --keep)
      KEEP=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "未知参数: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

# 库根缺省值：必须与 `crates/yeban-app/src/samples.rs::library_root()` 同一条形状。
if [ -z "$LIBRARY" ]; then
  case "$(uname -s)" in
    Darwin*)
      LIBRARY="$HOME/Library/Application Support/dev.yeban.app/Samples"
      LIBRARY_SOURCE="preset(macOS)"
      ;;
    Linux*)
      LIBRARY="${XDG_DATA_HOME:-$HOME/.local/share}/yeban/samples"
      LIBRARY_SOURCE="preset(Linux)"
      ;;
    MINGW*|MSYS*|CYGWIN*)
      LIBRARY="${APPDATA:-$HOME/AppData/Roaming}/yeban/samples"
      LIBRARY_SOURCE="preset(Windows)"
      ;;
    *)
      LIBRARY="$HOME/.local/share/yeban/samples"
      LIBRARY_SOURCE="preset(unknown→Linux)"
      ;;
  esac
elif [ "$LIBRARY_SOURCE" = "preset" ]; then
  LIBRARY_SOURCE="env(YEBAN_SAMPLE_LIBRARY)"
fi

if [ -z "$WORKDIR" ]; then
  WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/yeban-verify-samples.XXXXXX")" || {
    echo "无法创建临时目录" >&2
    exit 2
  }
  OWN_WORKDIR=1
else
  mkdir -p "$WORKDIR" || { echo "无法创建 --workdir: $WORKDIR" >&2; exit 2; }
fi

cleanup() {
  if [ "$OWN_WORKDIR" = 1 ] && [ "$KEEP" = 0 ]; then
    rm -rf "$WORKDIR"
  fi
}
trap cleanup EXIT

note() { printf '  · %s\n' "$*"; }
header() { printf '\n== [%s] %s ==\n' "$1" "$2"; }
pass() { PASS_N=$((PASS_N + 1)); printf '  PASS  %s\n' "$*"; }
fail() { FAIL_N=$((FAIL_N + 1)); printf '  FAIL  %s\n' "$*"; }
skip() { SKIP_N=$((SKIP_N + 1)); printf '  SKIP  %s\n' "$*"; }
cand_skip() { CAND_SKIP_N=$((CAND_SKIP_N + 1)); printf '  SKIP  候选 %s\n' "$*"; }

finalize() {
  [ "$FINALIZED" = 1 ] && return
  FINALIZED=1
  header "7/7" "总结"
  printf '  SUMMARY: PASS %d | FAIL %d | SKIP %d | 候选跳过 %d\n' \
    "$PASS_N" "$FAIL_N" "$SKIP_N" "$CAND_SKIP_N"
  if [ -n "$SELECTED_ID" ]; then
    printf '  DETAIL: instrument=%s regions=%s energy=%s\n' \
      "$SELECTED_ID" "$SELECTED_REGIONS" "$SELECTED_ENERGY"
    printf '  DETAIL: sfz=%s\n' "$SELECTED_SFZ"
  fi
  if [ "$FAIL_N" -gt 0 ]; then
    printf '  VERDICT: FAIL（退出码 1）\n'
    printf '  NEXT: 修上面 FAIL 的项后重跑: bash scripts/dev/verify-samples.sh\n'
    exit 1
  fi
  if [ "$UNKNOWN" = 1 ]; then
    printf '  VERDICT: UNKNOWN（退出码 2：无法判定）\n'
    printf '  NEXT: 检查库根/清单/yeban-app 二进制；或看下面的 SKIP 原因\n'
    exit 2
  fi
  printf '  VERDICT: PASS（退出码 0）\n'
  printf '  NEXT: 打开 GUI，控制台 Tab 3 → 装 %s → 听（本脚本只证明"解码并出声"，不证明"好听"）\n' \
    "${SELECTED_ID:-<上面选中的 instrument>}"
  exit 0
}

remaining_skips() {
  # 提前终止时，把没跑的步骤补成 SKIP，保证 PASS+FAIL+SKIP == 6。
  local from="$1"
  while [ "$from" -le 6 ]; do
    skip "步骤 $from 未运行（前一步已无法判定）"
    from=$((from + 1))
  done
}

# ---------------------------------------------------------------------------
# [1/7] 库根
# ---------------------------------------------------------------------------
header "1/7" "库根"
printf '  path: %s\n' "$LIBRARY"
printf '  resolved-from: %s\n' "$LIBRARY_SOURCE"
if [ ! -e "$LIBRARY" ]; then
  skip "库根不存在: $LIBRARY"
  UNKNOWN=1
  remaining_skips 2
  finalize
fi
if [ ! -d "$LIBRARY" ]; then
  fail "库根存在但不是目录: $LIBRARY"
  remaining_skips 2
  finalize
fi
if [ ! -r "$LIBRARY" ]; then
  skip "库根不可读: $LIBRARY"
  UNKNOWN=1
  remaining_skips 2
  finalize
fi
ROOT_STATS="$(python3 - "$LIBRARY" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
files = 0
total = 0
dirs = 0
for entry in root.rglob("*"):
    try:
        if entry.is_dir():
            dirs += 1
        elif entry.is_file():
            files += 1
            total += entry.stat().st_size
    except OSError:
        pass
print(f"{files}\t{total}\t{dirs}")
PY
)"
IFS=$'\t' read -r ROOT_FILES ROOT_BYTES ROOT_DIRS <<<"$ROOT_STATS"
printf '  numbers: files=%s bytes=%s dirs=%s\n' "$ROOT_FILES" "$ROOT_BYTES" "$ROOT_DIRS"
if [ "$ROOT_FILES" -eq 0 ] 2>/dev/null; then
  fail "库根是空的（files=0）⇒ 后面按缺件判"
else
  pass "库根存在且可读（files=$ROOT_FILES bytes=${ROOT_BYTES}）"
fi

# ---------------------------------------------------------------------------
# [2/7] 清单校验
# ---------------------------------------------------------------------------
header "2/7" "清单校验（fetch-samples.py --check-only）"
printf '  manifest: %s\n' "$MANIFEST"
if [ "$MANIFEST_IS_FIXTURE" = 1 ]; then
  printf '  ⚠ 清单覆盖: 不是仓库清单（%s），仅用于自测/fixture；读数不可当仓库结论\n' "$MANIFEST_SOURCE"
fi
if [ ! -f "$MANIFEST" ]; then
  skip "清单不存在: $MANIFEST"
  UNKNOWN=1
  remaining_skips 3
  finalize
fi
if [ ! -f "$REPO/scripts/dev/fetch-samples.py" ]; then
  skip "缺 scripts/dev/fetch-samples.py"
  UNKNOWN=1
  remaining_skips 3
  finalize
fi

SURVEY="$WORKDIR/survey.tsv"
if ! python3 - "$MANIFEST" "$LIBRARY" >"$SURVEY" <<'PY'
import json
import pathlib
import sys

manifest = pathlib.Path(sys.argv[1])
root = pathlib.Path(sys.argv[2])
PREFIX = "assets/samples/"

doc = json.loads(manifest.read_text(encoding="utf-8"))
items = doc.get("items", [])
instruments = doc.get("instruments", [])


def library_rel(item: dict) -> str:
    rel = item.get("relative_path", "")
    return rel[len(PREFIX):] if rel.startswith(PREFIX) else rel


def target(item: dict) -> pathlib.Path:
    return root / library_rel(item)


def has_coords(inst: dict) -> bool:
    return bool(inst.get("repo")) and bool(inst.get("pin"))


by_inst: dict[str, list] = {}
for item in items:
    by_inst.setdefault(item.get("instrument", ""), []).append(item)

coord_ids = {inst.get("id") for inst in instruments if has_coords(inst)}
nocoord_insts = [inst for inst in instruments if not has_coords(inst)]
nocoord_ids = {inst.get("id") for inst in nocoord_insts}

nocoord_items = sum(len(by_inst.get(inst_id, [])) for inst_id in nocoord_ids)
coord_items = len(items) - nocoord_items
present_coord = 0
missing_coord = []
present_nocoord = 0
for item in items:
    inst_id = item.get("instrument", "")
    exists = target(item).is_file()
    if inst_id in coord_ids:
        if exists:
            present_coord += 1
        else:
            missing_coord.append(item)
    elif exists:
        present_nocoord += 1

print(
    "manifest\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}".format(
        len(items),
        len(instruments),
        len(nocoord_insts),
        nocoord_items,
        coord_items,
        present_coord,
        len(missing_coord),
        nocoord_items - present_nocoord,
        present_nocoord,
    )
)
for item in missing_coord[:20]:
    print(
        "missing\t{}\t{}\t{}\t{}".format(
            item.get("id", ""),
            item.get("instrument", ""),
            item.get("relative_path", ""),
            item.get("size_bytes", 0),
        )
    )
if len(missing_coord) > 20:
    print("missing_more\t{}".format(len(missing_coord) - 20))
for inst in nocoord_insts:
    print(
        "nocoord\t{}\t{}\t{}".format(
            inst.get("id", ""),
            len(by_inst.get(inst.get("id"), [])),
            inst.get("sfz", ""),
        )
    )

complete = 0
no_sfz = 0
for inst in instruments:
    inst_id = inst.get("id", "")
    if not has_coords(inst):
        continue
    own = by_inst.get(inst_id, [])
    if not own:
        continue
    if not all(target(item).is_file() for item in own):
        continue
    sfz_rel = inst.get("sfz", "")
    prefix = inst.get("relative_root", "")
    if prefix.startswith(PREFIX):
        prefix = prefix[len(PREFIX):]
    sfz = root / prefix / sfz_rel if sfz_rel else None
    if sfz is None or not sfz.is_file():
        no_sfz += 1
        print("complete_no_sfz\t{}\t{}".format(inst_id, sfz_rel))
        continue
    complete += 1
    print(
        "candidate\t{}\t{}\t{}\t{}".format(
            inst_id, len(own), len(own), sfz
        )
    )
print("complete\t{}".format(complete))
PY
then
  fail "清单解析失败（见上）"
  remaining_skips 3
  finalize
fi

MANIFEST_LINE="$(grep -m1 '^manifest' "$SURVEY" || true)"
if [ -z "$MANIFEST_LINE" ]; then
  fail "清单解析没有产出行"
  remaining_skips 3
  finalize
fi
IFS=$'\t' read -r _ M_ITEMS M_INSTS M_NOCOORD_INSTS M_NOCOORD_ITEMS M_COORD_ITEMS \
  M_PRESENT_COORD M_MISSING_COORD M_MISSING_NOCOORD M_PRESENT_NOCOORD <<<"$MANIFEST_LINE"
COMPLETE_LINE="$(grep -m1 '^complete	' "$SURVEY" || true)"
M_COMPLETE="${COMPLETE_LINE#complete	}"
[ -n "$M_COMPLETE" ] || M_COMPLETE=0

printf '  numbers: items=%s instruments=%s 有坐标=%s 无坐标=%s(%s 项) 完整=%s\n' \
  "$M_ITEMS" "$M_INSTS" "$((M_INSTS - M_NOCOORD_INSTS))" "$M_NOCOORD_INSTS" "$M_NOCOORD_ITEMS" "$M_COMPLETE"
printf '  🚫 SKIP: %s 项属于 %s 个无 repo+pin 的 instrument（永远拿不到；⛔ 不计 FAIL）\n' \
  "$M_NOCOORD_ITEMS" "$M_NOCOORD_INSTS"
if [ "$M_NOCOORD_ITEMS" -gt 0 ]; then
  while IFS=$'\t' read -r tag id count sfz; do
    [ "$tag" = "nocoord" ] || continue
    printf '       - %s（%s 项，无上游坐标）\n' "$id" "$count"
  done < <(grep '^nocoord	' "$SURVEY" || true)
fi
printf '  registered-but-missing: %s / 有坐标项 %s（库内到位 %s）\n' \
  "$M_MISSING_COORD" "$M_COORD_ITEMS" "$M_PRESENT_COORD"
printf '  无坐标缺件: %s / %s 项（SKIP，⛔ 不计 FAIL；库内到位 %s）\n' \
  "$M_MISSING_NOCOORD" "$M_NOCOORD_ITEMS" "$M_PRESENT_NOCOORD"

CHECK_LOG="$WORKDIR/check-only.log"
python3 "$REPO/scripts/dev/fetch-samples.py" --check-only --manifest "$MANIFEST" --dest "$LIBRARY" >"$CHECK_LOG" 2>&1
CHECK_RC=$?
CHECK_LINE="$(grep -m1 '^fetch-samples:' "$CHECK_LOG" || true)"
printf '  %s\n' "${CHECK_LINE:-（没有汇总行）}"
printf '  fetch-samples rc=%s\n' "$CHECK_RC"
VERIFIED="$(printf '%s\n' "$CHECK_LINE" | sed -nE 's/.*校验通过 ([0-9]+) .*/\1/p')"
[ -n "$VERIFIED" ] || VERIFIED=0
printf '  校验通过 %s/%s（有坐标项；另 SKIP %s 项无坐标）\n' \
  "$VERIFIED" "$M_COORD_ITEMS" "$M_NOCOORD_ITEMS"

if [ "$M_MISSING_COORD" -gt 0 ]; then
  printf '  缺件明细（最多 20 条）:\n'
  while IFS=$'\t' read -r tag id inst rel size; do
    [ "$tag" = "missing" ] || continue
    printf '       - %s  [%s]  %s  (%s bytes)\n' "$id" "$inst" "$rel" "$size"
  done < <(grep '^missing	' "$SURVEY" || true)
  MORE="$(grep -m1 '^missing_more	' "$SURVEY" | cut -f2 || true)"
  [ -n "$MORE" ] && printf '       … 还有 %s 条\n' "$MORE"
fi

if [ "$CHECK_RC" -eq 1 ]; then
  fail "fetch-samples --check-only 报字节不符（rc=1，见 ${CHECK_LOG}）"
elif [ "$M_MISSING_COORD" -gt 0 ]; then
  fail "登记但缺 $M_MISSING_COORD 个文件（有坐标的项，应 FAIL）"
elif [ "$CHECK_RC" -eq 2 ]; then
  skip "fetch-samples --check-only 无法判定（rc=2）"
  UNKNOWN=1
elif [ "$VERIFIED" -lt "$M_PRESENT_COORD" ]; then
  fail "有 $((M_PRESENT_COORD - VERIFIED)) 个到位文件未通过校验（见 ${CHECK_LOG}）"
else
  pass "清单校验通过（$VERIFIED/${M_COORD_ITEMS}；无坐标 $M_NOCOORD_ITEMS 项按 SKIP 处理）"
fi

# ---------------------------------------------------------------------------
# [3/7] 挑一个完整 instrument
# ---------------------------------------------------------------------------
header "3/7" "挑一个完整 instrument"
CANDIDATES="$WORKDIR/candidates.tsv"
grep '^candidate	' "$SURVEY" >"$CANDIDATES" 2>/dev/null || true
CAND_COUNT="$(wc -l <"$CANDIDATES" | tr -d ' ')"
printf '  完整 instrument: %s / %s（manifest 顺序）\n' "$CAND_COUNT" "$M_INSTS"
if [ "$CAND_COUNT" -eq 0 ]; then
  fail "没有任何 instrument 的文件全部到位 ⇒ 无法挑出可探测对象"
  remaining_skips 4
  finalize
fi
FIRST_ID="$(cut -f2 "$CANDIDATES" | head -1)"
FIRST_PRESENT="$(cut -f3 "$CANDIDATES" | head -1)"
FIRST_TOTAL="$(cut -f4 "$CANDIDATES" | head -1)"
FIRST_SFZ="$(cut -f5- "$CANDIDATES" | head -1)"
printf '  第一个完整: %s（%s/%s 文件）\n' "$FIRST_ID" "$FIRST_PRESENT" "$FIRST_TOTAL"
printf '  sfz: %s\n' "$FIRST_SFZ"
if [ "$M_COMPLETE" -lt "$CAND_COUNT" ]; then
  printf '  ⚠ complete=%s 与候选行数=%s 不一致（complete_no_sfz 见 survey）\n' "$M_COMPLETE" "$CAND_COUNT"
fi
pass "找到 $CAND_COUNT 个完整 instrument；第一个是 $FIRST_ID"

# ---------------------------------------------------------------------------
# [4/7] 音色探测（核心）
# ---------------------------------------------------------------------------
header "4/7" "音色探测（--sfz + --export-wav）"
if [ -z "$APP" ]; then
  for candidate in "$REPO/target/release/yeban-app" "$REPO/target/debug/yeban-app"; do
    if [ -x "$candidate" ]; then
      APP="$candidate"
      break
    fi
  done
fi
if [ -z "$APP" ]; then
  APP="$(command -v yeban-app 2>/dev/null || true)"
fi
if [ -z "$APP" ]; then
  note "未找到 yeban-app；尝试 cargo build -p yeban-app --locked（可能需要几分钟）"
  # shellcheck source=./local-env.sh
  if [ -f "$REPO/scripts/dev/local-env.sh" ]; then
    # local-env.sh 只在受限环境里改 CARGO_HOME/RUSTUP_TOOLCHAIN。
    source "$REPO/scripts/dev/local-env.sh"
    yeban_local_env >/dev/null 2>&1 || true
  fi
  if command -v cargo >/dev/null 2>&1; then
    if cargo build -p yeban-app --locked >"$WORKDIR/build.log" 2>&1; then
      APP="$REPO/target/debug/yeban-app"
    else
      printf '  构建失败，日志尾部:\n'
      tail -n 20 "$WORKDIR/build.log" | sed 's/^/    /'
    fi
  fi
fi
if [ -z "$APP" ] || [ ! -x "$APP" ]; then
  skip "没有可执行的 yeban-app（--app / YEBAN_APP / cargo build 都没成）"
  UNKNOWN=1
  remaining_skips 5
  finalize
fi
printf '  app: %s\n' "$APP"

PROBE_WAV="$WORKDIR/probe.wav"
PROBE_LOG="$WORKDIR/probe.log"
STEP4_FAILED=0
LAST_REASON="没有候选同时满足 区域>0 且 能量>0"
CAND_INDEX=0
while IFS=$'\t' read -r tag id present total sfz; do
  [ "$tag" = "candidate" ] || continue
  CAND_INDEX=$((CAND_INDEX + 1))
  rm -f "$PROBE_WAV"
  "$APP" --headless --sfz "$sfz" --export-wav "$PROBE_WAV" >"$PROBE_LOG" 2>&1
  PROBE_RC=$?
  LOADED_LINE="$(grep -m1 '^loaded-sfz:' "$PROBE_LOG" || true)"
  EXPORTED_LINE="$(grep -m1 '^exported-wav:' "$PROBE_LOG" || true)"
  REGIONS="$(printf '%s\n' "$LOADED_LINE" | sed -nE 's/.* regions=([0-9]+) .*/\1/p')"
  PROBE="$(printf '%s\n' "$LOADED_LINE" | sed -nE 's/.*probe\(note:vel:frames:energy\)=([^ ]+).*/\1/p')"
  ENERGY="$(printf '%s\n' "$PROBE" | cut -d: -f4)"
  FRAMES="$(printf '%s\n' "$PROBE" | cut -d: -f3)"
  [ -n "$REGIONS" ] || REGIONS=0
  [ -n "$ENERGY" ] || ENERGY=0
  [ -n "$FRAMES" ] || FRAMES=0

  if [ "$PROBE_RC" -ne 0 ]; then
    ERR_LAST="$(tail -n 1 "$PROBE_LOG" | tr '\n' ' ')"
    cand_skip "$id: rc=${PROBE_RC}（SFZ 装载失败，不是样本问题）— ${ERR_LAST:-无 stderr}"
    LAST_REASON="候选 $id: rc=$PROBE_RC"
    continue
  fi
  if [ -z "$LOADED_LINE" ]; then
    cand_skip "$id: 没有 loaded-sfz 行"
    LAST_REASON="候选 $id: 没有 loaded-sfz 行"
    continue
  fi

  if printf '%s' "$EXPORTED_LINE" | grep -q '回退'; then
    DECODE="fallback(内存正弦)"
  elif printf '%s' "$EXPORTED_LINE" | grep -q 'source=sfz(file('; then
    DECODE="file(样本已解码)"
  else
    DECODE="other"
  fi
  printf '  候选 %s: regions=%s frames=%s energy=%s decode=%s\n' \
    "$id" "$REGIONS" "$FRAMES" "$ENERGY" "$DECODE"
  if [ "$REGIONS" -le 0 ] 2>/dev/null; then
    cand_skip "$id: regions=0（SFZ 可能用了 #include 或没有可播放区域；换个完整 instrument）"
    LAST_REASON="候选 $id: regions=0"
    continue
  fi
  if ! awk -v e="$ENERGY" 'BEGIN{exit !(e+0 > 0)}'; then
    cand_skip "$id: regions=$REGIONS 但 energy=0（默认探测音符 60 未命中该 SFZ 的区域；换个完整 instrument）"
    LAST_REASON="候选 $id: regions=$REGIONS 但 energy=0"
    continue
  fi

  # ⭐ 承重条件：能量 > 0 只说明"有声音"，不说明"用的是样本"。
  #    `--sfz` 的探测本身用内存正弦；导出行的 `source=` 才说样本有没有被真读。
  if printf '%s' "$EXPORTED_LINE" | grep -q '回退' || ! printf '%s' "$EXPORTED_LINE" | grep -q 'source=sfz(file('; then
    printf '  导出 source（逐字）: %s\n' "${EXPORTED_LINE:-（没有 exported-wav 行）}"
    fail "$id: 区域数=$REGIONS 能量=$ENERGY 但**样本没被解码**（导出回退到内存正弦，不是文件样本）"
    STEP4_FAILED=1
    LAST_REASON="候选 $id: 样本没被解码"
    break
  fi

  SELECTED_ID="$id"
  SELECTED_SFZ="$sfz"
  SELECTED_REGIONS="$REGIONS"
  SELECTED_ENERGY="$ENERGY"
  printf '  导出 source（逐字）: %s\n' "$EXPORTED_LINE"
  pass "音色探测：regions=$REGIONS (>0) 且 energy=$ENERGY (>0) 且导出 source=file(…)（样本真解码）"
  break
done <"$CANDIDATES"

if [ -z "$SELECTED_ID" ] && [ "$STEP4_FAILED" = 0 ]; then
  fail "所有完整候选都没能同时满足 区域>0 且 能量>0；最后原因: $LAST_REASON"
fi

# ---------------------------------------------------------------------------
# [5/7] WAV 自检
# ---------------------------------------------------------------------------
header "5/7" "WAV 自检（大小 / 帧数 / 峰值）"
if [ -z "$SELECTED_ID" ] || [ ! -f "$PROBE_WAV" ]; then
  skip "没有可检查的 probe.wav（第 4 步没有 PASS）"
else
  WAV_STATS="$(python3 - "$PROBE_WAV" <<'PY'
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
            peak = abs(value)
    norm = peak / float(2 ** (bits - 1)) if bits else 0.0
print(
    "{}\t{}\t{}\t{:.6f}\t{}\t{}\t{}".format(
        len(data), frames, peak, norm, rate, channels, bits
    )
)
PY
)"
  if printf '%s' "$WAV_STATS" | grep -q '^ERR'; then
    fail "probe.wav 头解析失败: $WAV_STATS"
  else
    IFS=$'\t' read -r W_BYTES W_FRAMES W_PEAK W_NORM W_RATE W_CH W_BITS <<<"$WAV_STATS"
    printf '  numbers: bytes=%s frames=%s peak=%s norm=%s rate=%s channels=%s bits=%s\n' \
      "$W_BYTES" "$W_FRAMES" "$W_PEAK" "$W_NORM" "$W_RATE" "$W_CH" "$W_BITS"
    if afinfo "$PROBE_WAV" >"$WORKDIR/afinfo.log" 2>&1; then
      AF_DURATION="$(grep -m1 'estimated duration' "$WORKDIR/afinfo.log" | sed 's/^[[:space:]]*//')"
      [ -n "$AF_DURATION" ] && printf '  afinfo: %s\n' "$AF_DURATION"
    fi
    if [ "$W_FRAMES" -le 0 ] 2>/dev/null; then
      fail "probe.wav frames=0"
    elif ! awk -v p="$W_PEAK" 'BEGIN{exit !(p+0 > 0)}'; then
      fail "probe.wav 峰值为 0（全零/静音）"
    else
      pass "probe.wav 非空且非静音（frames=$W_FRAMES peak=$W_PEAK norm=${W_NORM}）"
    fi
  fi
fi

# ---------------------------------------------------------------------------
# [6/7] 阴性对照
# ---------------------------------------------------------------------------
header "6/7" "阴性对照（一个故意不存在的 SFZ）"
if [ ! -x "$APP" ]; then
  skip "没有 yeban-app，无法跑对照"
  UNKNOWN=1
else
  CONTROL_SFZ="$WORKDIR/does-not-exist-$$.sfz"
  CONTROL_WAV="$WORKDIR/control.wav"
  rm -f "$CONTROL_SFZ" "$CONTROL_WAV"
  "$APP" --headless --sfz "$CONTROL_SFZ" --export-wav "$CONTROL_WAV" \
    >"$WORKDIR/control.log" 2>&1
  CONTROL_RC=$?
  CONTROL_LINE="$(grep -m1 '^loaded-sfz:' "$WORKDIR/control.log" || true)"
  printf '  control: rc=%s\n' "$CONTROL_RC"
  printf '  control: %s\n' "$(tail -n 1 "$WORKDIR/control.log" | cut -c1-240)"
  if [ "$CONTROL_RC" -ne 0 ] && [ ! -f "$CONTROL_WAV" ]; then
    pass "不存在的 SFZ ⇒ rc=$CONTROL_RC 且没有产出 WAV（证明第 4 步不是恒真）"
  elif [ "$CONTROL_RC" -ne 0 ]; then
    pass "不存在的 SFZ ⇒ rc=${CONTROL_RC}（非零；对照成立）"
  else
    fail "不存在的 SFZ 竟然 rc=0（对照失效：第 4 步可能是恒真）"
    [ -n "$CONTROL_LINE" ] && printf '  control loaded: %s\n' "$CONTROL_LINE"
  fi
fi

finalize
