#!/usr/bin/env bash
# **只用 MCP 工具**把一个音频片段从零放上轨道并渲染出来 —— 可复跑的端到端序列。
#
# 用法：
#
#     bash crates/yeban-mcp/verify/e2e_audio_clip_stdio.sh [输出目录] [yeban-mcp 可执行文件]
#
# 缺省：输出目录 `target/e2e-audio-clip`、可执行文件 `target/debug/yeban-mcp`
#       （先跑 `cargo build -p yeban-mcp --bin yeban-mcp`）。
#
# 这件事**为什么需要一个脚本**：`yeban-mcp --stdio` 的批处理语义是"读若干行 JSON-RPC，
# 读到 EOF 就退出"，而**同一个进程**里的会话状态跨行存活 —— 正好是"打开 → 导入 → 保存 →
# 渲染"这条链路需要的形态（与 `tests/stdio_e2e.rs` 同一形态，但这里是**裸命令行**复跑）。
#
# 起点工程从哪来（**不是**本脚本发明的形态）：`crates/yeban-mcp/verify/e2e_audio_clip_seed.py`
# 把模型规范样本 `project.filled.json` 打进 `ARCH-SEC-003` 容器，并去掉那条
# "只声明、没有字节"的音频资产。为什么必须去掉：端到端判据要证明的是"**本次导入的**
# 片段被真的渲染"，而那条悬空声明会如实地在渲染响应里报一次 `bytesPresent: false`
# （见同目录脚本的文档，以及 `seed-dangling.yeban` 保留的那条路径）。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
OUT_DIR="${1:-$REPO_ROOT/target/e2e-audio-clip}"
BIN="${2:-$REPO_ROOT/target/debug/yeban-mcp}"

if [[ ! -x "$BIN" ]]; then
    printf '缺少可执行文件 %s —— 先跑 cargo build -p yeban-mcp --bin yeban-mcp\n' "$BIN" >&2
    exit 2
fi

python3 "$REPO_ROOT/crates/yeban-mcp/verify/e2e_audio_clip_seed.py" --out "$OUT_DIR"

# 每次复跑都从**同一份**起点容器开始（上一次运行会把工程保存回 seed-clean.yeban）。
PROJECT="$OUT_DIR/project.yeban"
cp "$OUT_DIR/seed-clean.yeban" "$PROJECT"
rm -f "$PROJECT.lock" "$OUT_DIR/master-a.wav" "$OUT_DIR/master-b.wav"

# 规范样本里的音频轨（`TrackKind::Audio`，`project.filled.json` 的确定性 ULID）。
TRACK_ID="01J8ZQ00000000000000000003"
KICK="$OUT_DIR/kick.wav"

REQUESTS="$OUT_DIR/requests.jsonl"
python3 - "$PROJECT" "$KICK" "$TRACK_ID" "$OUT_DIR" > "$REQUESTS" <<'PY'
import json
import sys

project, kick, track, out = sys.argv[1:5]


def call(ident, name, arguments):
    return json.dumps(
        {
            "jsonrpc": "2.0",
            "id": ident,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        },
        ensure_ascii=False,
    )


print(call(1, "yeban_open_project", {"path": project}))
# 片段登记 + 摆放（`trackId` 一给就摆放；`durationTicks` 不给 ⇒ 由素材全长换算）。
print(
    call(
        2,
        "yeban_import_audio",
        {"name": "Kick", "path": kick, "trackId": track, "startTick": 0, "gainDb": -3.0},
    )
)
print(call(3, "yeban_save_project", {}))
print(
    call(
        4,
        "yeban_render_master",
        {"format": "wav", "sampleRate": 48000, "path": f"{out}/master-a.wav"},
    )
)
# 第二次渲染：同一工程、同一进程、真实墙钟（用来如实测量 `bext` 的墙钟字段）。
print(
    call(
        5,
        "yeban_render_master",
        {"format": "wav", "sampleRate": 48000, "path": f"{out}/master-b.wav"},
    )
)
PY

printf '== 请求（逐行 JSON-RPC）==\n'
cat "$REQUESTS"
printf '\n== 响应（逐行）==\n'
"$BIN" --stdio --token-file "$OUT_DIR/token-e2e.txt" < "$REQUESTS" > "$OUT_DIR/responses.jsonl"
cat "$OUT_DIR/responses.jsonl"
printf '\n== 工具侧读数 ==\n'
python3 - "$OUT_DIR" <<'PY'
import hashlib
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
responses = {}
for line in (out / "responses.jsonl").read_text(encoding="utf-8").splitlines():
    line = line.strip()
    if line:
        parsed = json.loads(line)
        responses[parsed["id"]] = parsed

raw = (out / "responses.jsonl").read_text(encoding="utf-8")
print(f'响应里出现 "bytesPresent":false 的次数 = {raw.count(chr(34) + "bytesPresent" + chr(34) + ":false")}')

placement = responses[2]["result"]["data"]["placement"]
print(f'import  : created={responses[2]["result"]["data"]["created"]} '
      f'placed={placement["placed"]} durationTicks={placement["durationTicks"]} '
      f'durationRule={placement["durationRule"]}')
print(f'         clipId={responses[2]["result"]["data"]["clip"]["clipId"]} '
      f'asset={responses[2]["result"]["data"]["clip"]["asset"]}')
print(f'save    : assets={responses[3]["result"]["data"]["assets"]} '
      f'bytes={responses[3]["result"]["data"]["bytes"]}')

for ident in (4, 5):
    data = responses[ident]["result"]["data"]
    path = pathlib.Path(data["path"])
    payload = path.read_bytes()
    assets = data["audio"]["assets"]
    print(f'render{ident} : path={path}')
    print(f'         bytes={data["bytes"]} sha256={data["sha256"]}')
    print(f'         masterDigest={data["masterDigest"]} frames={data["frames"]} '
          f'durationSeconds={data["durationSeconds"]}')
    print(f'         bytesPresent={[a["bytesPresent"] for a in assets]} '
          f'renderedFrames={[a["renderedFrames"] for a in assets]} '
          f'clipsRendered={data["audio"]["clipsRendered"]}')
    print(f'         unsupported={data["unsupported"]}')
    print(f'         stat -f%z {path.name} = {len(payload)}  '
          f'shasum -a 256 = {hashlib.sha256(payload).hexdigest()}')

first = (out / "master-a.wav").read_bytes()
second = (out / "master-b.wav").read_bytes()
print(f'"cmp master-a.wav master-b.wav" => {"identical" if first == second else "DIFFERENT"}')
print(f'bext 墙钟字段: a={responses[4]["result"]["data"]["bwf"]["originationDate"]} '
      f'{responses[4]["result"]["data"]["bwf"]["originationTime"]} | '
      f'b={responses[5]["result"]["data"]["bwf"]["originationDate"]} '
      f'{responses[5]["result"]["data"]["bwf"]["originationTime"]}')
PY
