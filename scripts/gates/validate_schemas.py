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
import hashlib
import pathlib
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


def rel(path: pathlib.Path) -> str:
    """相对仓库根的路径(报告里用; 绝对路径太长且会暴露本机目录结构)。"""
    try:
        return str(path.resolve().relative_to(REPO.resolve()))
    except ValueError:
        return str(path)


def verify_repo_asset_manifests(schemas: dict, problems: list[str]) -> None:
    """校验仓库里**自己的**资产清单, 并逐项重算 SHA-256 与磁盘字节对账。

    为什么需要这一步: `--samples-dir` 只校验 Rust 侧导出的样本, 而 `assets/**/manifest.json`
    是**仓库自带的**清单 —— 在此之前它们**从来没有被任何门禁读过**
    （`validate_schemas.py` 只把 `assets.manifest.schema.json` 当 schema 校验了语法）。
    于是"资产必须登记许可与 SHA-256"(AGENTS.md 红线 9)这条纪律**没有任何机械保护**:
    清单可以写错、可以漏项、可以指向不存在的文件, 而全部门禁依旧绿。

    这里做两件事:
    1. 用 `assets.manifest.schema.json` 校验清单**本身**的结构;
    2. **逐项重算 SHA-256 与 size_bytes**, 与磁盘上的真实字节比对 —— 这是"登记"与"事实"的对账。
    """
    from jsonschema import Draft202012Validator

    schema = schemas.get("assets.manifest.schema.json")
    if schema is None:
        problems.append("缺少 assets.manifest.schema.json, 无法校验资产清单")
        return
    validator = Draft202012Validator(schema)
    manifests = sorted(
        path
        for path in (REPO / "assets").rglob("*.json")
        if path.name.lower() == "manifest.json" or path.name == "MANIFEST.json"
    )
    if not manifests:
        problems.append("assets/ 下没有任何清单文件 —— 红线 9 的'登记'没有载体")
        return
    # --- 未登记资产文件检查（红线 9 的机械形式）------------------------------------
    # 为什么要有这一条: "清单逐项对账"只能保证**登记了的**是对的, 它管不住**没登记的**文件 ——
    # 往 `assets/<category>/` 直接丢一个 wav/png 就能绕过"资产必须登记许可与 SHA-256"这条红线,
    # 而所有门禁依旧绿。这里按**根指针清单**声明过的分类逐个目录扫:
    #   目录里有非文档文件 ⇒ 必须有清单, 且每个文件都必须在清单的 items 里出现。
    root_doc = None
    root_file = REPO / "assets" / "manifest.json"
    if root_file.is_file():
        try:
            root_doc = load_json(root_file)
        except Exception:  # noqa: BLE001 - 根清单坏掉时上面已经报过, 这里不重复
            root_doc = None
    if isinstance(root_doc, dict):
        for entry in root_doc.get("sub_manifests", []):
            category = entry.get("category")
            if not category:
                continue
            category_dir = REPO / "assets" / category
            if not category_dir.is_dir():
                problems.append(f"根清单声明了资产分类 `{category}`, 但目录 assets/{category}/ 不存在")
                continue
            files = sorted(
                path
                for path in category_dir.rglob("*")
                if path.is_file()
                and path.suffix.lower() not in {".md", ".json"}
                and not path.name.startswith(".")
            )
            if not files:
                continue
            declared = entry.get("manifest")
            manifest_file = REPO / declared if isinstance(declared, str) else None
            if manifest_file is None or not manifest_file.is_file():
                problems.append(
                    f"assets/{category}/ 下有 {len(files)} 个资产文件, 但根清单没有指向一份存在的清单 "
                    f"(声明的是 {declared!r}) —— 先建清单再放文件"
                )
                continue
            try:
                doc = load_json(manifest_file)
            except Exception as error:  # noqa: BLE001
                problems.append(f"{rel(manifest_file)}: 无法解析为 JSON: {error}")
                continue
            listed = {
                item.get("relative_path")
                for item in doc.get("items", [])
                if isinstance(item, dict)
            }
            for path in files:
                if rel(path) not in listed:
                    problems.append(
                        f"{rel(path)}: 资产文件**未登记**在 {rel(manifest_file)} 里 —— "
                        f"红线 9 要求每个二进制资产都有许可与 SHA-256 记录"
                    )

    for manifest_path in manifests:
        try:
            doc = load_json(manifest_path)
        except Exception as error:  # noqa: BLE001 - 报告并继续, 不因一个坏文件中断全部
            problems.append(f"{rel(manifest_path)}: 无法解析为 JSON: {error}")
            continue
        if "sub_manifests" in doc:
            # 根清单是**指针式**文档: 它自己不列条目, 而是指向各分类清单。
            # 但仍必须校验结构 + 确认被指向的清单真的存在(否则"登记"指向空气)。
            errors = sorted(validator.iter_errors(doc), key=lambda e: list(e.path))
            for err in errors[:5]:
                where = "/".join(str(p) for p in err.path) or "<根>"
                problems.append(f"{rel(manifest_path)}: 违反 assets.manifest.schema.json @ {where}: {err.message}")
            missing = [
                entry.get("manifest")
                for entry in doc.get("sub_manifests", [])
                if entry.get("manifest") and not (REPO / entry["manifest"]).is_file()
            ]
            for target in missing:
                problems.append(f"{rel(manifest_path)}: 指向不存在的子清单: {target}")
            pointer_count = len(doc.get("sub_manifests", []))
            if not missing:
                print(f"[ok] {rel(manifest_path)}: 指针式清单, {pointer_count} 个子清单均存在")
            continue
        if "category" not in doc:
            print(f"[skip] {rel(manifest_path)}: 无 category, 不作为清单校验")
            continue
        errors = sorted(validator.iter_errors(doc), key=lambda e: list(e.path))
        if errors:
            for err in errors[:5]:
                where = "/".join(str(p) for p in err.path) or "<根>"
                problems.append(f"{rel(manifest_path)}: 违反 assets.manifest.schema.json @ {where}: {err.message}")
            continue
        items = doc.get("items", [])
        if not items:
            problems.append(f"{rel(manifest_path)}: items 为空 —— 清单存在但没有登记任何资产")
            continue
        # ⚠ 许可白名单**必须被机械执行**（needs N2，2026-10-05）。
        # 实测过这个洞: 往 items[] 里塞一条结构合法的 `CC-BY-NC-SA`（**非商用**）条目,
        # `--repo-assets` 仍然 EXIT=0 —— 因为当时**全仓没有任何代码读 `license`/`allowed_licenses`**。
        # 红线 2 是"许可合规", 而一条只写在清单里、没人读的白名单等于没有。
        # ⇒ 口径: 用**子清单自己声明的** `licence_whitelist` 逐条判, 并要求它与根清单该 category
        #   的 `allowed_licenses` **一致或为其子集**（两处声明不能各说各话）。
        whitelist = doc.get("licence_whitelist")
        if whitelist is not None:
            # ⚠ 不依赖作用域里的 `root_doc`（它只在处理根清单时被赋值, 子清单轮次可能是 None ——
            # 第一版就踩了这个坑）。这里**自己加载**根清单, 让校验与调用顺序无关。
            root_manifest = REPO / "assets" / "manifest.json"
            root_of_truth = root_doc
            if not isinstance(root_of_truth, dict) and root_manifest.is_file():
                try:
                    root_of_truth = json.loads(root_manifest.read_text(encoding="utf-8"))
                except json.JSONDecodeError:
                    root_of_truth = None
            root_lists = {
                entry.get("category"): entry.get("allowed_licenses")
                for entry in (root_of_truth or {}).get("sub_manifests", [])
            }
            root_allowed = root_lists.get(doc.get("category"))
            if root_allowed is not None:
                extra = sorted(set(whitelist) - set(root_allowed))
                if extra:
                    problems.append(
                        f"{rel(manifest_path)}: 子清单的 licence_whitelist 含根清单 "
                        f"allowed_licenses 之外的许可 {extra} —— 两处声明不一致"
                    )
            for item in items:
                licence = item.get("license") or item.get("licence")
                if licence is None:
                    problems.append(
                        f"{rel(manifest_path)}: 条目 {item.get('id')} 没有声明许可 —— "
                        "素材没有许可是不能用分发的"
                    )
                    continue
                if licence not in whitelist:
                    problems.append(
                        f"{rel(manifest_path)}: 条目 {item.get('id')} 的许可 {licence!r} "
                        f"不在白名单 {whitelist} 内（红线 2: 许可合规）"
                    )
                if item.get("commercial_usable") is False:
                    problems.append(
                        f"{rel(manifest_path)}: 条目 {item.get('id')} 标了 commercial_usable=false "
                        "却仍被登记为可分发素材"
                    )
        checked = 0
        optional_missing = 0
        for item in items:
            rel_path = item.get("relative_path", "")
            # 路径口径 = **仓库根**相对(与根清单的 sub_manifests 一致), 见 schema 的 description。
            target = REPO / rel_path
            if not target.is_file():
                if item.get("optional") is True:
                    # optional=true = 官方仓库不随包分发(例如大权重), 登记义务仍已完成。
                    optional_missing += 1
                    continue
                problems.append(f"{rel(manifest_path)}: 条目 {item.get('id')} 指向不存在的文件: {rel_path}")
                continue
            digest = hashlib.sha256(target.read_bytes()).hexdigest()
            if digest != item.get("sha256"):
                problems.append(
                    f"{rel(manifest_path)}: 条目 {item.get('id')} 的 SHA-256 与磁盘不符 "
                    f"(清单 {str(item.get('sha256'))[:12]}…, 实际 {digest[:12]}…) —— 文件被改过而清单没更新"
                )
                continue
            size = item.get("size_bytes")
            actual_size = target.stat().st_size
            if isinstance(size, int) and size != actual_size:
                problems.append(
                    f"{rel(manifest_path)}: 条目 {item.get('id')} 的 size_bytes 不符 "
                    f"(清单 {size}, 实际 {actual_size})"
                )
                continue
            checked += 1
        note = f"(另有 {optional_missing} 项 optional 资产未随仓库分发)" if optional_missing else ""
        print(f"[ok] {rel(manifest_path)}: {checked}/{len(items)} 条资产的 SHA-256 与磁盘一致{note}")


def main() -> int:
    parser = argparse.ArgumentParser(description="校验 schemas/ 与 Rust 产出的样本")
    parser.add_argument(
        "--repo-assets",
        action="store_true",
        help="校验 assets/**/manifest.json(结构 + 逐项 SHA-256/大小与磁盘对账)",
    )
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

    if args.repo_assets:
        verify_repo_asset_manifests(schemas, problems)

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
