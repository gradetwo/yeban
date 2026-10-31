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
# ⭐ **它有退出码**（2026-10-31 起）：末尾的读数块同时是**带名字的断言**
#    —— 7 个 `check(...)` 调用点、其中 2 个在逐渲染循环里 ⇒ 一次复跑共 **9 条**判据，
#    逐条打印 `[PASS]/[FAIL]`，任一为红 ⇒ `exit 1`。
#    执行者是 `.github/workflows/ci.yml` 的 `e2e (MCP stdio 复跑 / 幂等重放契约)` 这条腿
#    （此前全库没有执行者，台账 §20.6/§21.8）。

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
#
# ⚠ `license` 是**必填**：`yeban_import_audio` 逐条查白名单
# （`assets/samples/manifest.json` 的 `licence_whitelist`，`MUST-GATE-014` 的「仅限」），
# 缺了就是结构化拒绝 `licenseRequired`，而不是有一个默认值。这里的声明**为真**：
# `kick.wav` 的样本值是帧号的纯函数、由上面的种子脚本自己生成（无第三方素材），
# 因此按其自述许可写 `CC0-1.0`。⛔ 不是随手挑一个白名单里的字符串。
print(
    call(
        2,
        "yeban_import_audio",
        {
            "name": "Kick",
            "path": kick,
            "trackId": track,
            "startTick": 0,
            "gainDb": -3.0,
            "license": "CC0-1.0",
        },
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
"""判据：把上面每个读数变成**有名字的断言**，任一失败 ⇒ 退出码 1。

口径来源（⛔ 本脚本不发明不变式）：
- `master-a.wav` 与 `master-b.wav` 逐位相同 —— 与
  `crates/yeban-mcp/tests/render_master.rs` 的
  `two_renders_under_different_wall_clocks_are_byte_identical_chunk_by_chunk` 同一条；
  墙钟只进报告不进文件，见同文件
  `the_wall_clock_reaches_the_report_but_not_the_file` ⇒ 下面**只打印** `bext` 墙钟，不断言。
- `data.sha256` / `data.bytes` 必须等于落盘文件的实测值 —— 与 `stat -f%z` /
  `shasum -a 256` 的读数一致（AGENTS.md §6.4）。
- ⛔ 不断言 `unsupported == []`：本序列里该项应为空的语义**未证**，只登记读数。
"""
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

# 任何一条响应带 `error` 或缺 `result` ⇒ 先把**整行**打出来再退（否则下面的读数会
# 在半路上抛 KeyError，把"哪一步拒绝的"这个最有用的信息丢掉）。
refused = [ident for ident, msg in sorted(responses.items()) if "error" in msg or "result" not in msg]
missing = [ident for ident in (1, 2, 3, 4, 5) if ident not in responses]
if refused or missing:
    print(f'缺响应 id={missing}；被拒绝的 id={refused}', file=sys.stderr)
    for ident in refused:
        print(responses[ident], file=sys.stderr)
    sys.exit(1)

print(f'响应里出现 "bytesPresent":false 的次数 = {raw.count(chr(34) + "bytesPresent" + chr(34) + ":false")}')

placement = responses[2]["result"]["data"]["placement"]
print(f'import  : created={responses[2]["result"]["data"]["created"]} '
      f'placed={placement["placed"]} durationTicks={placement["durationTicks"]} '
      f'durationRule={placement["durationRule"]}')
print(f'         clipId={responses[2]["result"]["data"]["clip"]["clipId"]} '
      f'asset={responses[2]["result"]["data"]["clip"]["asset"]}')
print(f'save    : assets={responses[3]["result"]["data"]["assets"]} '
      f'bytes={responses[3]["result"]["data"]["bytes"]}')

renders = {}
for ident in (4, 5):
    data = responses[ident]["result"]["data"]
    path = pathlib.Path(data["path"])
    payload = path.read_bytes()
    renders[ident] = (data, payload)
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

first = renders[4][1]
second = renders[5][1]
print(f'"cmp master-a.wav master-b.wav" => {"identical" if first == second else "DIFFERENT"}')
print(f'bext 墙钟字段: a={renders[4][0]["bwf"]["originationDate"]} '
      f'{renders[4][0]["bwf"]["originationTime"]} | '
      f'b={renders[5][0]["bwf"]["originationDate"]} '
      f'{renders[5][0]["bwf"]["originationTime"]}')

checks = []


def check(name, ok, detail):
    checks.append((name, ok, detail))


# 1. 五个请求全部有响应，且没有任何一条是 JSON-RPC 错误。
check(
    "responses_are_complete_and_error_free",
    sorted(responses) == [1, 2, 3, 4, 5]
    and not [ident for ident, msg in responses.items() if "error" in msg or "result" not in msg],
    f'ids={sorted(responses)} errors={[i for i, m in responses.items() if "error" in m]}',
)

# 2. 导入：登记成功且真的摆放了一个片段，长度由素材全长换算（>0）。
#    `placed` 是**布尔**（`import_audio.rs` 的 `"placed": self.placement_created()`），
#    `durationRule` 是**中文说明句**（含「素材全长换算」），⛔ 不是 `"asset-length"` 这类枚举名。
import_data = responses[2]["result"]["data"]
check(
    "import_registers_and_places_the_clip",
    import_data["created"] is True
    and import_data["imported"] is True
    and placement["placed"] is True
    and placement["durationTicks"] > 0
    and "素材全长换算" in str(placement["durationRule"])
    and import_data["source"]["kind"] == "diskPath"
    and import_data["source"]["registeredBytes"] > 0,
    f'created={import_data["created"]} placed={placement["placed"]} '
    f'durationTicks={placement["durationTicks"]} durationRule={placement["durationRule"]} '
    f'source={import_data["source"]}',
)

# 3. 保存：资产进了容器，字节数非零（`saved` / `atomic` 是回执自述）。
save_data = responses[3]["result"]["data"]
check(
    "save_persists_the_asset",
    save_data["saved"] is True
    and save_data["skipped"] is False
    and save_data["assets"] >= 1
    and save_data["bytes"] > 0,
    f'saved={save_data["saved"]} skipped={save_data["skipped"]} '
    f'assets={save_data["assets"]} bytes={save_data["bytes"]}',
)

# 4. 每次渲染：回执自报的 sha256 / bytes 与落盘文件的实测值一致。
for ident in (4, 5):
    data, payload = renders[ident]
    check(
        f"render{ident}_receipt_matches_the_file",
        data["sha256"] == hashlib.sha256(payload).hexdigest() and data["bytes"] == len(payload),
        f'receipt bytes={data["bytes"]} sha256={data["sha256"]} | '
        f'actual bytes={len(payload)}',
    )
    # 5. 回执里的每个资产都有字节、且渲染出非零帧。
    assets = data["audio"]["assets"]
    check(
        f"render{ident}_every_asset_has_bytes_and_frames",
        all(a["bytesPresent"] is True and a["renderedFrames"] > 0 for a in assets)
        and data["audio"]["clipsRendered"] >= 1,
        f'bytesPresent={[a["bytesPresent"] for a in assets]} '
        f'renderedFrames={[a["renderedFrames"] for a in assets]} '
        f'clipsRendered={data["audio"]["clipsRendered"]}',
    )

# 6. 整份响应里不出现任何一次 `bytesPresent: false`（起点已去掉悬空声明 ⇒ 应为 0 次）。
check(
    "no_asset_reported_as_missing_bytes",
    raw.count(chr(34) + "bytesPresent" + chr(34) + ":false") == 0,
    f'occurrences={raw.count(chr(34) + "bytesPresent" + chr(34) + ":false")}',
)

# 7. 两次渲染逐位相同（墙钟不进文件）。
check(
    "two_renders_are_byte_identical",
    first == second,
    f'a={len(first)}B b={len(second)}B '
    f'sha256 a={hashlib.sha256(first).hexdigest()[:12]}… b={hashlib.sha256(second).hexdigest()[:12]}…',
)

print("\n== 判据 ==")
failed = 0
for name, ok, detail in checks:
    print(f'[{"PASS" if ok else "FAIL"}] {name} :: {detail}')
    if not ok:
        failed += 1
if failed:
    print(f'\n{failed} / {len(checks)} 条判据为红 ⇒ exit 1', file=sys.stderr)
    sys.exit(1)
print(f'\n{len(checks)} / {len(checks)} 条判据为绿')
PY
