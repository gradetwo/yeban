#!/usr/bin/env python3
"""JSON Schema 契约校验 (schemas/ 目录与 Rust 产出的样本对账)。

规范来源:
  - `schemas/*.json` 是机器校验契约 (MUST-GATE 体系的数据侧)
  - AGENTS.md §3 DoD 3: 需求规范 ID 闭环

做三件事:
  1. 每个 `schemas/*.json` 自身必须是合法的 JSON Schema (Draft 2020-12);
  2. `$id` 必须唯一 (重复的 $id 会让 $ref 解析到错误的文档);
  3. 若提供了 `--samples-dir`, 则把 Rust 侧产出的规范样本逐个对账 ——
     这是**跨实现交叉验证**: serde 写的字节与 Python jsonschema 读的契约必须一致。

样本命名约定 (前缀决定用哪份 schema):

    <prefix>.<name>.json          契约实例 —— 会拿去与 <prefix> 对应的 schema 对账
    <prefix>.<name>.meta.json     文档样本 —— 顶层是统计/清单/快照, **跳过**对账

⚠ 为什么需要 `.meta.`: 有些样本的价值是"把契约缺口机器可读地记下来"(例如错误码并集的缺项清单),
它**本来就不是**契约实例。用同一套 `oneOf` 根去校验它属于类型错误。
但 `.meta.` 必须配一条守卫: **每个前缀至少要有一份真实例**, 否则"全是 meta"意味着该契约
一份都没对账 —— 那是"判据跑起来了但什么都没检查"的假绿(见 docs/DEVELOPMENT_LEDGER.md)。
    project.<name>.json   -> schemas/project.schema.json
    ops.<name>.json       -> schemas/ops.schema.json
    mcp-tools.<name>.json -> schemas/mcp-tools.schema.json
    assets.<name>.json    -> schemas/assets.manifest.schema.json

用法:
    python3 scripts/gates/validate_schemas.py
    python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCHEMAS = REPO / "schemas"

SAMPLE_SCHEMA_MAP = {
    "project": "project.schema.json",
    "ops": "ops.schema.json",
    "mcp-tools": "mcp-tools.schema.json",
    "assets": "assets.manifest.schema.json",
}


def load_json(path: Path) -> object:
    with path.open(encoding="utf-8") as fh:
        return json.load(fh)


def main() -> int:
    parser = argparse.ArgumentParser(description="校验 schemas/ 与 Rust 产出的样本")
    parser.add_argument("--samples-dir", default=None, help="Rust 侧导出的样本目录")
    args = parser.parse_args()

    try:
        import jsonschema
        from jsonschema import Draft202012Validator
    except ImportError:
        print(
            "缺少 jsonschema 依赖。CI 上安装方式: pip install 'jsonschema>=4.24'\n"
            "本机: python3 -m pip install --user 'jsonschema>=4.24'",
            file=sys.stderr,
        )
        return 2

    schema_files = sorted(SCHEMAS.glob("*.json"))
    if not schema_files:
        print(f"未找到任何 schema: {SCHEMAS}", file=sys.stderr)
        return 1

    problems: list[str] = []
    ids: dict[str, str] = {}
    schemas: dict[str, dict] = {}

    for path in schema_files:
        try:
            doc = load_json(path)
        except json.JSONDecodeError as exc:
            problems.append(f"{path.name}: JSON 解析失败: {exc}")
            continue
        try:
            Draft202012Validator.check_schema(doc)
        except jsonschema.exceptions.SchemaError as exc:
            problems.append(f"{path.name}: 不是合法的 Draft 2020-12 schema: {exc.message}")
            continue
        schemas[path.name] = doc
        schema_id = doc.get("$id")
        if schema_id:
            if schema_id in ids:
                problems.append(f"{path.name}: $id `{schema_id}` 与 {ids[schema_id]} 重复")
            ids[schema_id] = path.name
        print(f"[ok] {path.name}: 合法 JSON Schema ({doc.get('title', '?')})")

    if args.samples_dir:
        samples_dir = (REPO / args.samples_dir).resolve()
        if not samples_dir.is_dir():
            problems.append(f"样本目录不存在: {samples_dir}")
        else:
            samples = sorted(samples_dir.glob("*.json"))
            if not samples:
                problems.append(
                    f"样本目录为空: {samples_dir} —— Rust 侧应导出规范样本, 否则这条判据是空转"
                )
            meta_samples = [s for s in samples if ".meta." in s.name]
            real_samples = [s for s in samples if ".meta." not in s.name]
            for sample in meta_samples:
                print(f"[skip] {sample.name}: 文档样本(.meta.), 不对账 schema")
            # 守卫: 每个前缀都必须有真实例, 否则该契约无对账
            real_prefixes = {s.name.split(".", 1)[0] for s in real_samples}
            for prefix in sorted({s.name.split(".", 1)[0] for s in samples} - real_prefixes):
                problems.append(
                    f"前缀 `{prefix}` 只有 .meta. 文档样本, 没有任何真实例 —— "
                    "该契约等于没有对账(全是 meta 就是假绿)"
                )

            for sample in real_samples:
                prefix = sample.name.split(".", 1)[0]
                schema_name = SAMPLE_SCHEMA_MAP.get(prefix)
                if schema_name is None:
                    problems.append(f"{sample.name}: 文件名前缀 `{prefix}` 没有对应 schema 约定")
                    continue
                if schema_name not in schemas:
                    problems.append(f"{sample.name}: 需要 {schema_name}, 但它不存在或非法")
                    continue
                validator = Draft202012Validator(schemas[schema_name])
                errors = sorted(validator.iter_errors(load_json(sample)), key=lambda e: list(e.path))
                if errors:
                    for err in errors[:5]:
                        where = "/".join(str(p) for p in err.path) or "<根>"
                        problems.append(f"{sample.name}: 违反 {schema_name} @ {where}: {err.message}")
                else:
                    print(f"[ok] {sample.name}: 通过 {schema_name}")

    if problems:
        print("\n契约校验未通过:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1

    print(f"\n契约校验通过 ({len(schemas)} 份 schema)。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
