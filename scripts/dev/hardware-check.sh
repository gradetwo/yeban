#!/usr/bin/env bash
# 硬件与真机验证的交互式检查单（人类手工跑）。
#
# 为什么需要它：CI 上没有真声卡、真扬声器、真 MIDI 键盘与真窗口。
# 这些项由人类验证。本脚本负责提问、收答案、写报告。
#
# 用法：
#   bash scripts/dev/hardware-check.sh
#   bash scripts/dev/hardware-check.sh --only 1        # 只跑第 1 项
#   bash scripts/dev/hardware-check.sh --list          # 列出所有项
#
# 报告落在 /tmp/yeban-hardware-report-<时间戳>.txt。把报告发回即可。
#
# ⚠ 本脚本必须兼容 bash 3.2（本仓守卫 G14）。⛔ 不要用关联数组。

set -u

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
STAMP="$(date +%Y%m%d-%H%M%S)"
REPORT="/tmp/yeban-hardware-report-${STAMP}.txt"
ONLY=""

while [ $# -gt 0 ]; do
  case "$1" in
    --only) ONLY="${2:-}"; shift 2 ;;
    --list) ONLY="list"; shift ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "未知参数: $1"; exit 2 ;;
  esac
done

if [ "$ONLY" = "list" ]; then
  cat <<'EOF'
1  录音       GUI 录音键与 CLI --record-wav 对照（疑为静音）
2  SFZ 音色   经调音台三个控件试听
3  视觉基准   macOS golden 基准补录
4  窗口帧率   fps_gpu example 的 p50/p99
5  MIDI 键盘  实体键盘输入
6  声学往返   回环线的往返时延
EOF
  exit 0
fi

# ---------- 基础设施 ----------

say() { printf '%s\n' "$*"; }
blank() { say ""; }
rule() { say "------------------------------------------------------------"; }

need() {
  # need <命令> -> 0/1
  command -v "$1" >/dev/null 2>&1
}

ask_yn() {
  # ask_yn <问题> <键> -> 设 ANS_<键>=yes/no/unknown
  local q="$1" key="$2" a=""
  while :; do
    printf '%s [y/n/s(跳过)]: ' "$q"
    if ! IFS= read -r a; then a="s"; fi
    case "$a" in
      y|Y|yes|YES) eval "ANS_${key}=yes"; return 0 ;;
      n|N|no|NO)   eval "ANS_${key}=no";  return 0 ;;
      s|S|skip)    eval "ANS_${key}=skipped"; return 0 ;;
      *) say "请输入 y、n 或 s。" ;;
    esac
  done
}

ask_text() {
  # ask_text <问题> <键> -> 设 TXT_<键>
  local q="$1" key="$2" a=""
  printf '%s （直接回车＝留空）: ' "$q"
  if ! IFS= read -r a; then a=""; fi
  eval "TXT_${key}=\$a"
}

ask_pause() {
  printf '%s  —  完成后按回车继续: ' "$1"
  IFS= read -r _ || true
}

note() {
  # note <键> <文本> -> 追加进报告
  eval "NOTE_${1}=\"\${NOTE_${1:-}:-}${2}\n\""
}

section() {
  rule
  say "【$1】"
  rule
}

# ---------- 报告 ----------

report_open() {
  {
    say "# yeban 硬件验证报告"
    say "# 时间：$(date '+%Y-%m-%d %H:%M:%S %Z')"
    say "# 主机：$(uname -a)"
    say "# 仓库：$REPO"
    if need git; then say "# 提交：$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo 未知)"; fi
    if need sw_vers; then say "# 系统：$(sw_vers -productVersion 2>/dev/null || echo 未知)"; fi
    say ""
  } >"$REPORT"
  say "报告文件：$REPORT"
}

kv() {
  # kv <键> <值>
  printf '%s: %s\n' "$1" "${2:-<空>}" >>"$REPORT"
}

# ---------- 各项 ----------

check_env() {
  section "0 环境预检"
  local ok="yes"
  if [ ! -f "$REPO/Cargo.toml" ]; then say "⚠ 找不到 $REPO/Cargo.toml"; ok="no"; fi
  if ! need cargo; then say "⚠ 找不到 cargo"; ok="no"; fi
  if [ "$ok" = "yes" ]; then
    say "环境看起来正常。"
  else
    say "⚠ 环境不完整。后面的检查可能失败。"
  fi
  kv "环境预检" "$ok"
  note ENV "环境预检=$ok"
}

check_record() {
  # 第 1 项：录音
  section "1 录音：GUI 对比 CLI"
  say "背景：GUI 的录音键可能录到静音。请做一次对照。"
  blank
  say "第 1 步：跑 CLI 录音（这一步由脚本执行，约 5 秒）。"
  local cli_out="/tmp/yeban-hc-cli-rec.wav"
  if need cargo; then
    say "命令：cargo run -p yeban-app -- --record-wav $cli_out --seconds 5"
    ask_yn "现在跑这条命令吗？（会占用约 1 分钟）" REC_RUN
    if [ "${ANS_REC_RUN:-no}" = "yes" ]; then
      if ( cd "$REPO" && cargo run -p yeban-app --locked -- --record-wav "$cli_out" --seconds 5 ); then
        say "CLI 录音完成。"
        kv "CLI 录音命令" "成功"
      else
        say "⚠ CLI 录音命令失败。"
        kv "CLI 录音命令" "失败"
      fi
    fi
  fi
  if [ -f "$cli_out" ]; then
    kv "CLI 录音文件字节数" "$(wc -c <"$cli_out" | tr -d ' ')"
  fi
  blank
  say "第 2 步：跑 GUI（另开一个终端，脚本不代跑）。"
  say "命令：cargo run -p yeban-app -- --open <你的工程>"
  say "然后：点录音键 → 录 5 秒 → 停止 → 找到落盘的 WAV。"
  ask_pause "GUI 录音"
  say "第 3 步：播放两个 WAV 文件，比较。"
  ask_yn "CLI 录的文件有声音吗？" REC_CLI_SOUND
  ask_yn "GUI 录的文件有声音吗？" REC_GUI_SOUND
  ask_yn "GUI 录的文件是全静音吗？" REC_GUI_SILENT
  ask_text "GUI 录音文件的路径与字节数" REC_GUI_PATH
  note REC "CLI有声=${ANS_REC_CLI_SOUND:-} GUI有声=${ANS_REC_GUI_SOUND:-} GUI全静音=${ANS_REC_GUI_SILENT:-}"
}

check_sfz() {
  # 第 2 项：SFZ 音色
  section "2 SFZ 音色：经调音台试听"
  say "前提：一份 SFZ 音色，且它引用的样本是 PCM WAV。"
  say "（FLAC/OGG 支持正在做，先只用 WAV 样本。）"
  blank
  say "步骤："
  say "  1. cargo run -p yeban-app -- --open <你的工程>"
  say "  2. 点「调音台」标签"
  say "  3. 通道条上找三个控件：路径文本框、SET、CLR"
  say "  4. 在文本框粘贴 SFZ 的绝对路径"
  say "  5. 点 SET（或按回车）"
  say "  6. 播放该轨音符"
  say "  7. 换成第二个 SFZ，再 SET，再听"
  say "  8. 点 CLR，再听"
  ask_pause "试听"
  ask_yn "听到的音色对吗？" SFZ_RIGHT
  ask_yn "有咔嗒声或爆音吗？" SFZ_CLICK
  ask_yn "只有一边（左或右）有声音吗？" SFZ_ONESIDE
  ask_yn "换路径后音色立即改变吗？" SFZ_SWITCH
  ask_yn "点 CLR 后回到内置合成器吗？" SFZ_CLEAR
  ask_yn "点 SET 时界面有错误提示吗？" SFZ_ERR_UI
  ask_yn "终端（stderr）有输出吗？" SFZ_ERR_TTY
  ask_text "机型 / macOS 版本 / 音频接口" SFZ_HW
  ask_text "其他观察（有咔嗒就写下来）" SFZ_NOTE
  note SFZ "音色对=${ANS_SFZ_RIGHT:-} 咔嗒=${ANS_SFZ_CLICK:-} 单边=${ANS_SFZ_ONESIDE:-} 即换=${ANS_SFZ_SWITCH:-} CLR回合成器=${ANS_SFZ_CLEAR:-} UI错误=${ANS_SFZ_ERR_UI:-} 终端输出=${ANS_SFZ_ERR_TTY:-}"
}

check_golden() {
  # 第 3 项：视觉基准
  section "3 视觉基准：补 macOS golden"
  say "背景：macOS 的基准文件已被删除。现在 macOS 没有视觉判决。"
  say "这一步补上它。命令由脚本执行。"
  ask_yn "现在跑补录命令吗？（约 1 分钟）" GOLD_RUN
  if [ "${ANS_GOLD_RUN:-no}" = "yes" ] && need cargo; then
    if ( cd "$REPO" && YEBAN_WRITE_GOLDEN=1 cargo test -p yeban-app --locked --test test_port_adapter ); then
      say "补录命令完成。"
      kv "补录命令" "成功"
    else
      say "⚠ 补录命令失败。"
      kv "补录命令" "失败"
    fi
    say "生成的文件："
    ( cd "$REPO" && git status --short crates/yeban-app/tests/golden/ ) 2>/dev/null | sed 's/^/  /'
    ( cd "$REPO" && git status --short crates/yeban-app/tests/golden/ ) 2>/dev/null >>"$REPORT"
  fi
  ask_yn "生成了 PNG 文件吗？" GOLD_PNG
  ask_yn "生成了 MANIFEST.txt 吗？" GOLD_MANIFEST
  say "⚠ 重要：逐张把 PNG 与界面比较。"
  say "⚠ 如果某张图不对，不要录为基准。告诉我。"
  ask_yn "每张图与界面一致吗？" GOLD_OK
  ask_text "如果某张不对，是哪张、哪里不对" GOLD_BAD
  note GOLD "有PNG=${ANS_GOLD_PNG:-} 有MANIFEST=${ANS_GOLD_MANIFEST:-} 全一致=${ANS_GOLD_OK:-}"
}

check_fps() {
  # 第 4 项：窗口与帧率
  section "4 窗口与帧率"
  say "背景：winit 事件循环只能在主线程创建。所以帧率档是 example，不是测试。"
  say "命令由脚本执行。它会开一个真窗口，跑 600 帧。"
  ask_yn "现在跑帧率档吗？" FPS_RUN
  local log="/tmp/yeban-hc-fps.log"
  if [ "${ANS_FPS_RUN:-no}" = "yes" ] && need cargo; then
    say "命令：cargo run -p yeban-app --example fps_gpu --release"
    if ( cd "$REPO" && cargo run -p yeban-app --locked --example fps_gpu --release ) 2>&1 | tee "$log"; then
      say "帧率档完成。"
    fi
    say "读数（含 p50/p99 的行）："
    grep -i -E 'p50|p99|frame|ms' "$log" 2>/dev/null | tail -20 | sed 's/^/  /'
    grep -i -E 'p50|p99' "$log" 2>/dev/null >>"$REPORT"
  fi
  ask_yn "界面流畅吗？" FPS_SMOOTH
  ask_yn "有卡顿、撕裂或闪白吗？" FPS_ARTIFACT
  ask_yn "三个 SFZ 控件的位置合理吗？" FPS_LAYOUT
  ask_yn "切换标签时控件重绘正确吗？" FPS_TAB
  ask_text "机型 / 显示器 / 刷新率" FPS_HW
  note FPS "流畅=${ANS_FPS_SMOOTH:-} 异常=${ANS_FPS_ARTIFACT:-} 布局=${ANS_FPS_LAYOUT:-} 标签=${ANS_FPS_TAB:-}"
}

check_midi() {
  # 第 5 项：MIDI 键盘
  section "5 MIDI 键盘"
  say "前提：一个 MIDI 键盘。"
  say "命令：cargo run -p yeban-app --features midi-input -- --open <你的工程>"
  ask_pause "弹奏测试"
  ask_yn "识别到设备吗？" MIDI_DEV
  ask_yn "弹奏有响应吗？" MIDI_RESP
  ask_yn "有可感知的延迟吗？" MIDI_LAG
  ask_text "键盘型号 / 连接方式（USB 或蓝牙）" MIDI_HW
  note MIDI "识别=${ANS_MIDI_DEV:-} 响应=${ANS_MIDI_RESP:-} 延迟=${ANS_MIDI_LAG:-}"
}

check_acoustic() {
  # 第 6 项：声学往返
  section "6 声学往返时延"
  say "前提：物理回环线，连接输出与输入。"
  say "测量命令由集成者提供（见台账 BASELINE-005）。"
  ask_yn "你接了回环线吗？" ACU_READY
  if [ "${ANS_ACU_READY:-no}" = "yes" ]; then
    say "请告诉集成者。他会给你确切命令。"
  fi
  ask_text "设备名称" ACU_DEV
  ask_text "缓冲大小（如 128 / 256）" ACU_BUF
  ask_text "往返毫秒数（测 3 次取中间值）" ACU_MS
  note ACU "设备=${TXT_ACU_DEV:-} 缓冲=${TXT_ACU_BUF:-} 往返ms=${TXT_ACU_MS:-}"
}

# ---------- 主流程 ----------

run_one() {
  case "$1" in
    0) check_env ;;
    1) check_record ;;
    2) check_sfz ;;
    3) check_golden ;;
    4) check_fps ;;
    5) check_midi ;;
    6) check_acoustic ;;
    *) say "未知项：$1"; return 2 ;;
  esac
}

report_close() {
  {
    say ""
    say "## 答案汇总"
  } >>"$REPORT"
  local k
  for k in ENV REC SFZ GOLD FPS MIDI ACU; do
    eval "local v=\"\${NOTE_${k}:-}\""
    if [ -n "$v" ]; then
      printf '%s: %s\n' "$k" "$v" >>"$REPORT"
    fi
  done
  {
    say ""
    say "## 文本答案"
    say "GUI 录音路径：${TXT_REC_GUI_PATH:-<空>}"
    say "SFZ 机型：${TXT_SFZ_HW:-<空>}"
    say "SFZ 其他观察：${TXT_SFZ_NOTE:-<空>}"
    say "golden 不对的图：${TXT_GOLD_BAD:-<空>}"
    say "帧率机型：${TXT_FPS_HW:-<空>}"
    say "MIDI 键盘：${TXT_MIDI_HW:-<空>}"
    say "声学设备：${TXT_ACU_DEV:-<空>}"
    say "声学缓冲：${TXT_ACU_BUF:-<空>}"
    say "声学往返 ms：${TXT_ACU_MS:-<空>}"
  } >>"$REPORT"
}

main() {
  say "夜半（yeban）硬件与真机验证 —— 交互式检查单"
  say "每一项都会问你 y / n / s（跳过）。"
  say "答案写进报告文件。把报告发回即可。"
  report_open
  check_env
  if [ -n "$ONLY" ]; then
    run_one "$ONLY"
  else
    local i
    for i in 1 2 3 4 5 6; do
      run_one "$i"
      blank
      ask_yn "继续下一项吗？" NEXT
      if [ "${ANS_NEXT:-yes}" = "no" ]; then
        say "提前结束。"
        break
      fi
    done
  fi
  report_close
  blank
  rule
  say "完成。报告文件：$REPORT"
  say "把这个文件的内容发回给集成者。"
  rule
}

main "$@"
