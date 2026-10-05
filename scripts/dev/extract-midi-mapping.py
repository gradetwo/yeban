#!/usr/bin/env python3
"""把 app 的 MIDI 映射层（`export_midi.rs`）抽到共享 crate `yeban-midi`（账本第 283-309 轮）。

做法：正文整体下移，**只**在两侧分别保留该保留的东西：
  * 共享侧（`yeban-midi/src/export.rs`）：映射 + 编解码，导入用 `crate::midi::`；
  * app 侧（`yeban-app/src/export_midi.rs`）：**写文件**（`Save` 变体 + `export_project_to_file`）+ 原测试模块，
    导入用 `yeban_midi::midi::`。

**写入前先预检**（第 302/308 轮）：只统计**代码行**的括号（跳过注释/围栏示例、剥离字符串），
并要求 writer 在共享侧出现 0 次、app 侧 1 次。预检不过 ⇒ **不写任何文件**（树保持不动）。
"""
from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
APP = ROOT / "crates/yeban-app/src/export_midi.rs"
SHARED = ROOT / "crates/yeban-midi/src/export.rs"


def code_braces(text: str) -> int:
    """只统计代码行的花括号：跳过注释行，剥离行内注释与字符串字面量。"""
    bal = 0
    for line in text.split("\n"):
        s = line.strip()
        if s.startswith(("//", "#!", "/*", "*")):
            continue
        s = re.sub(r'"(?:\\.|[^"\\])*"', '""', s)
        s = re.sub(r"//.*", "", s)
        bal += s.count("{") - s.count("}")
    return bal


def strip_uses(lines: list[str]) -> list[str]:
    """删掉**整条** use（多行感知：扫到以 ';' 结尾的行）。"""
    out, i = [], 0
    while i < len(lines):
        if lines[i].startswith("use "):
            while i < len(lines) and not lines[i].rstrip().endswith(";"):
                i += 1
            i += 1
            continue
        out.append(lines[i])
        i += 1
    return out


def span_of(lines: list[str], i: int) -> tuple[int, int]:
    """条目跨度：向上吃文档/属性；有括号体则配对，否则只到本行。"""
    j = i
    while j > 0 and (lines[j - 1].strip().startswith("///") or lines[j - 1].strip().startswith("#[")):
        j -= 1
    if lines[i].rstrip().endswith((",", ";")):
        return j, i
    k = i
    while k < len(lines) and "{" not in lines[k]:
        k += 1
    depth, p = 0, k
    while p < len(lines):
        s = re.sub(r'"(?:\\.|[^"\\])*"', '""', lines[p])
        s = re.sub(r"//.*", "", s)
        depth += s.count("{") - s.count("}")
        if depth == 0:
            break
        p += 1
    return j, p


def collect_imports(lines: list[str]) -> tuple[list[str], list[str]]:
    """返回（单行 use 列表, 多行 use 块里的名字列表）。"""
    single = [l for l in lines if l.startswith("use ") and l.rstrip().endswith(";")]
    names, i = [], 0
    while i < len(lines):
        l = lines[i]
        if l.startswith("use ") and "{" in l and not l.rstrip().endswith(";"):
            buf = l.split("{", 1)[1]
            i += 1
            while i < len(lines) and "}" not in lines[i]:
                buf += " " + lines[i]
                i += 1
            if i < len(lines):
                buf += " " + lines[i].split("}", 1)[0]
            names += [
                t for t in (x.strip() for x in re.split(r"[,\s]+", buf)) if t and t.isidentifier()
            ]
        i += 1
    return single, sorted(set(names))


def main() -> int:
    lines = APP.read_text(encoding="utf-8").split("\n")
    single, midi_names = collect_imports(lines)
    ts = min(i for i, l in enumerate(lines) if l.strip().startswith("#[cfg(test)]"))
    body_raw, tests = lines[:ts], "\n".join(lines[ts:])

    kept, seen = [], False
    for l in body_raw:
        if l.startswith("//!"):
            if not seen:
                kept.append(l)
        elif l.strip():
            seen = True

    body = strip_uses(body_raw)
    drop: set[int] = set()
    for i, l in enumerate(body):
        if (
            l.strip().startswith("Save(SaveError)")
            or l.startswith("pub fn export_project_to_file(")
            or l.strip().startswith("Self::Save(")
        ):
            a, b = span_of(body, i)
            drop.update(range(a, b + 1))
    lib = [l for i, l in enumerate(body) if i not in drop and not l.startswith("//!")]

    base = [u for u in single if "crate::save" not in u]
    midi_import = ["use " + "PREFIX" + "midi::{ " + ", ".join(midi_names) + " };"] if midi_names else []
    shared = "\n".join(
        kept
        + [""]
        + [u.replace("yeban_render::midi::", "crate::midi::") for u in base]
        + [m.replace("PREFIX", "crate::") for m in midi_import]
        + [""]
        + lib
    ) + "\n"

    wrapper = "\n".join(
        [
            "//! `--export-midi` 的**消费侧**：映射与编码已下移到 `yeban-midi`（与 MCP 工具**共用同一实现**）。",
            "pub use yeban_midi::export::{MidiExportReport, export_from_project};",
            "",
        ]
        + base
        + [m.replace("PREFIX", "yeban_midi::") for m in midi_import]
        + [
            "",
            "/// 导出失败的原因。",
            "#[derive(Debug)]",
            "pub enum MidiExportError {",
            "    /// 映射/编码失败（领域侧，来自共享 crate）。",
            "    Export(yeban_midi::export::MidiExportError),",
            "    /// 编码器拒绝。",
            "    Encode(MidiError),",
            "    /// 原子落盘失败（I/O）。",
            "    Save(crate::save::SaveError),",
            "}",
            "",
            "impl core::fmt::Display for MidiExportError {",
            "    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {",
            "        match self {",
            '            Self::Export(e) => write!(f, "{e}"),',
            '            Self::Encode(e) => write!(f, "{e}"),',
            '            Self::Save(e) => write!(f, "{e}"),',
            "        }",
            "    }",
            "}",
            "",
            "impl std::error::Error for MidiExportError {}",
            "",
            "impl From<yeban_midi::export::MidiExportError> for MidiExportError {",
            "    fn from(error: yeban_midi::export::MidiExportError) -> Self {",
            "        Self::Export(error)",
            "    }",
            "}",
            "",
            "/// 把工程写成 `.mid` 文件（**语义与下移前完全一致**）。",
            "pub fn export_project_to_file(",
            "    project: &YebanProjectV1,",
            "    path: impl AsRef<Path>,",
            ") -> Result<MidiExportReport, MidiExportError> {",
            "    let export = export_from_project(project)?;",
            "    let bytes = export.to_smf_bytes().map_err(MidiExportError::Encode)?;",
            "    let saved = crate::save::write_file_atomically(&bytes, path).map_err(MidiExportError::Save)?;",
            "    Ok(MidiExportReport {",
            "        path: saved.path,",
            "        bytes: saved.bytes,",
            "        temp_name: saved.temp_name,",
            "        tracks: export.tracks.len(),",
            "        notes: export.tracks.iter().map(|t| t.notes.len()).sum(),",
            "        tempos: export.tempos.len(),",
            "        ppq: export.ppq,",
            "        format: export.format,",
            "    })",
            "}",
            "",
        ]
    )
    app = wrapper + tests + "\n"

    problems = []
    if code_braces(shared):
        problems.append(f"共享侧代码括号不平衡 {code_braces(shared):+d}")
    if code_braces(app):
        problems.append(f"app 侧代码括号不平衡 {code_braces(app):+d}")
    for name, text, want in (("export.rs", shared, 0), ("export_midi.rs", app, 1)):
        got = text.count("pub fn export_project_to_file(")
        if got != want:
            problems.append(f"{name}: writer 出现 {got} 次（期望 {want}）")
    if "yeban_midi::" in shared:
        problems.append("共享侧出现 yeban_midi:: 自引用")
    if problems:
        print("[预检失败] " + "; ".join(problems))
        print("⇒ 不写任何文件（树保持不变）")
        return 3

    SHARED.write_text(shared, encoding="utf-8")
    APP.write_text(app, encoding="utf-8")
    lib_rs = ROOT / "crates/yeban-midi/src/lib.rs"
    t = lib_rs.read_text(encoding="utf-8")
    if "pub mod export;" not in t:
        lib_rs.write_text(t.replace("pub mod midi;", "pub mod midi;\npub mod export;", 1), encoding="utf-8")
    cargo = ROOT / "crates/yeban-app/Cargo.toml"
    c = cargo.read_text(encoding="utf-8")
    if "yeban-midi" not in c:
        cargo.write_text(
            c.replace("[dependencies]", '[dependencies]\nyeban-midi = { path = "../yeban-midi" }', 1),
            encoding="utf-8",
        )
    print("[预检通过] 已写入 export.rs / export_midi.rs / lib.rs / Cargo.toml")
    return 0


if __name__ == "__main__":
    sys.exit(main())
