#!/usr/bin/env python3
"""真实 `tools/call` 会话下, 幂等重放的**两次响应都通过 schema 校验**。

用法:

    python3 crates/yeban-mcp/verify/idempotency_replay_schema.py [--binary <path>]

缺省 `--binary` 是 `target/debug/yeban-mcp`(先跑
`cargo build -p yeban-mcp --bin yeban-mcp`)。

为什么需要这个脚本: `schemas/mcp-tools.schema.json` 的根是
`oneOf(ToolCall, ToolResponse, ReplayedToolResponse)`。幂等重放(`arguments.idempotencyKey`
命中缓存)的 `result` **不是**裸 `ToolResponse`, 而是
`{"replayed": true, "response": <当前 id 重组的完整 JSON-RPC 响应>}`。
在这条路径进契约之前, 会校验 schema 的客户端会**拒收**重放响应。

这个脚本用**真二进制**跑一次 stdio 会话:

1. `yeban_open_project` 打开一份真容器工程;
2. `yeban_import_audio` 用 `idempotencyKey = "verify-replay"` 调**两次**;

然后把**每一条** `tools/call` 响应的 `result` 交给 Python `jsonschema`:

- 两次 `result` 都必须通过**根** schema;
- 第二次必须在 `definitions.ReplayedToolResponse` 上**恰好**成立;
- 第一次(裸 `ToolResponse`)必须**不**在 `ReplayedToolResponse` 上成立 ——
  否则"重放路径改回旧形状"这条判据就是空转的。

起点工程由 `crates/yeban-mcp/verify/e2e_audio_clip_seed.py` 产出(与
`e2e_audio_clip_stdio.sh` 同一份种子), 因此它依赖模型规范样本
`target/schema-samples/project.filled.json`
(`cargo run -p yeban-model --example export_schema_samples`)。
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

try:
    import jsonschema
except ImportError:  # pragma: no cover - 环境缺失时响亮失败, 不伪造绿
    print("需要 python3 的 jsonschema 包(与 scripts/gates/validate_schemas.py 同一条依赖)")
    sys.exit(2)

REPO = Path(__file__).resolve().parents[3]
SCHEMA_PATH = REPO / "schemas" / "mcp-tools.schema.json"
SEED_SCRIPT = REPO / "crates" / "yeban-mcp" / "verify" / "e2e_audio_clip_seed.py"
OUT_DIR = REPO / "target" / "idempotency-replay-schema"

KEY = "verify-replay"


def fail(message: str) -> None:
    print(f"[FAIL] {message}")
    sys.exit(1)


def call(ident: int, name: str, arguments: dict) -> str:
    return json.dumps(
        {
            "jsonrpc": "2.0",
            "id": ident,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        },
        ensure_ascii=False,
    )


def subschema_validator(name: str):
    """把 `definitions.<name>` 提成一份自洽的 schema(同文档内解析 `$ref`)。"""
    document = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    if name not in document["definitions"]:
        fail(f"schema 里没有 definitions.{name} —— 重放信封未被契约描述")
    return jsonschema.Draft202012Validator(
        {"$ref": f"#/definitions/{name}", "definitions": document["definitions"]}
    )


def schema_errors(validator, instance) -> list[str]:
    errors = sorted(validator.iter_errors(instance), key=lambda error: list(error.path))
    return [
        f"{'/'.join(str(part) for part in error.path) or '<根>'}: {error.message}"
        for error in errors
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(REPO / "target" / "debug" / "yeban-mcp"))
    args = parser.parse_args()

    binary = Path(args.binary)
    if not binary.is_file():
        fail(f"缺少可执行文件 {binary} —— 先跑 cargo build -p yeban-mcp --bin yeban-mcp")
    if not SCHEMA_PATH.is_file():
        fail(f"缺少契约 {SCHEMA_PATH}")

    if OUT_DIR.exists():
        shutil.rmtree(OUT_DIR)
    OUT_DIR.mkdir(parents=True)
    seeded = subprocess.run(
        [sys.executable, str(SEED_SCRIPT), "--out", str(OUT_DIR)],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    if seeded.returncode != 0:
        print(seeded.stdout)
        print(seeded.stderr, file=sys.stderr)
        fail("种子工程产出失败(见上面的输出)")

    project = OUT_DIR / "project.yeban"
    shutil.copyfile(OUT_DIR / "seed-clean.yeban", project)
    (OUT_DIR / "project.yeban.lock").unlink(missing_ok=True)
    kick = OUT_DIR / "kick.wav"

    requests = [
        call(1, "yeban_open_project", {"path": str(project)}),
        call(2, "yeban_import_audio", {"name": "ReplayKick", "path": str(kick), "idempotencyKey": KEY}),
        call(3, "yeban_import_audio", {"name": "ReplayKick", "path": str(kick), "idempotencyKey": KEY}),
    ]
    session = subprocess.run(
        [str(binary), "--stdio", "--token-file", str(OUT_DIR / "token.txt")],
        cwd=REPO,
        input="\n".join(requests) + "\n",
        capture_output=True,
        text=True,
    )
    if session.returncode != 0:
        print(session.stderr, file=sys.stderr)
        fail(f"stdio 会话退出码 {session.returncode}")

    responses = {}
    for line in session.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        document = json.loads(line)
        responses[document["id"]] = document

    for ident in (1, 2, 3):
        if ident not in responses:
            fail(f"会话缺少 id={ident} 的响应")

    root = jsonschema.Draft202012Validator(json.loads(SCHEMA_PATH.read_text(encoding="utf-8")))
    replay = subschema_validator("ReplayedToolResponse")
    tool_response = subschema_validator("ToolResponse")

    first = responses[2]["result"]
    second = responses[3]["result"]

    print(f"requests.jsonl:")
    for line in requests:
        print(f"  {line}")
    print()
    print(f"[id 2] 首次 result (截断): {json.dumps(first, ensure_ascii=False)[:160]}…")
    print(f"[id 3] 重放 result (截断): {json.dumps(second, ensure_ascii=False)[:200]}…")
    print()

    problems = []
    for label, value in (("id 2 首次", first), ("id 3 重放", second)):
        errors = schema_errors(root, value)
        if errors:
            problems.append(f"{label} 未通过根 schema: {errors}")
        else:
            print(f"[ok] {label} 的 result 通过 mcp-tools.schema.json 根")
    if not schema_errors(tool_response, first):
        print("[ok] id 2 首次 的 result 通过 definitions.ToolResponse (裸 ToolResponse)")
    else:
        problems.append("id 2 首次 不是合法 ToolResponse")
    if not schema_errors(replay, second):
        print("[ok] id 3 重放 的 result 通过 definitions.ReplayedToolResponse")
    else:
        problems.append(
            "id 3 重放 不是 ReplayedToolResponse: " + str(schema_errors(replay, second))
        )
    if schema_errors(replay, first):
        print("[ok] id 2 首次 的 result **不**通过 definitions.ReplayedToolResponse (旧形状不是重放)")
    else:
        problems.append("裸 ToolResponse 竟然也通过 ReplayedToolResponse —— 判据无牙")

    if second.get("replayed") is not True:
        problems.append("重放响应缺少 `replayed: true` —— 重放路径可能已改回旧形状")

    if problems:
        print()
        for problem in problems:
            print(f"[FAIL] {problem}")
        return 1
    print()
    print("两次 tools/call 响应都通过 schema 校验, 且重放走 ReplayedToolResponse 分支。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
