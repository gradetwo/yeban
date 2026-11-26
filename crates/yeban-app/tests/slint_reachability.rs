//! T-ISS-MWXKGSHZ：`.slint` 可达性护栏。
//!
//! `crates/yeban-app/ui/` 下每个文件都必须能从 `app.slint` 的 `import` 链到达；
//! 否则它就是一块"改了也不渲染"的死 UI，后续维护者会被误导（见
//! `docs/DEAD-SLINT-FILES.md` 与 T-FIX-729 的 `controls/knob.slint`）。
//! 旧口径只靠 `elements.rs::DEAD_FILES` 人工记忆，没有机械判定，本判据补上。

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// 已登记为死文件的白名单（⛔ 不是"允许新增死文件"的口子）。
/// 每增加一项都必须同时更新 `docs/DEAD-SLINT-FILES.md`；反向断言保证它们
/// 真的从 `app.slint` 不可达（把死文件接回活树时必须把这里删掉）。
const DEAD_FILES: &[&str] = &[
    "controls/knob.slint",
    "module_actions.slint",
    "transport.slint",
    "workspace/arrangement_view.slint",
];

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

/// 抽取 `import { .. } from "p.slint"` / `export { .. } from "p.slint"` 的路径。
/// 注释里出现的 `from "..."` 若指向不存在的文件会被解析步过滤掉，不会误伤。
fn imported_files(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(position) = rest.find("from \"") {
        let after = &rest[position + "from \"".len()..];
        let Some(end) = after.find('"') else {
            break;
        };
        let value = &after[..end];
        if value.ends_with(".slint") {
            out.push(value.to_owned());
        }
        rest = &after[end + 1..];
    }
    out
}

/// 按 `Path` 语义消掉 `.` / `..`，不要求文件已存在。
fn normalize(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn reachable_from_app(ui: &Path) -> BTreeSet<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![PathBuf::from("app.slint")];
    while let Some(relative) = queue.pop() {
        if !seen.insert(relative.clone()) {
            continue;
        }
        let text = std::fs::read_to_string(ui.join(&relative))
            .unwrap_or_else(|error| panic!("读 ui/{} 失败: {error}", relative.display()));
        for import in imported_files(&text) {
            let resolved = normalize(relative.parent().unwrap_or(Path::new("")).join(import));
            if ui.join(&resolved).is_file() {
                queue.push(resolved);
            }
        }
    }
    seen
}

fn all_slint_files(ui: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("遍历 ui/ 目录") {
        let entry = entry.expect("读目录条目");
        let path = entry.path();
        if path.is_dir() {
            all_slint_files(ui, &path, out);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "slint")
        {
            out.push(
                path.strip_prefix(ui)
                    .expect("文件必须在 ui/ 下")
                    .to_path_buf(),
            );
        }
    }
}

#[test]
fn every_slint_file_is_reachable_except_declared_dead_files() {
    let ui = ui_dir();
    let reachable = reachable_from_app(&ui);
    let mut all = Vec::new();
    all_slint_files(&ui, &ui, &mut all);

    let declared: BTreeSet<PathBuf> = DEAD_FILES.iter().map(PathBuf::from).collect();
    let orphans: Vec<String> = all
        .iter()
        .filter(|path| !reachable.contains(*path) && !declared.contains(*path))
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        orphans.is_empty(),
        "ui/ 下有不可达的 .slint 文件（先确认是否死文件，再更新 docs/DEAD-SLINT-FILES.md 与本判据白名单）: {orphans:?}"
    );

    // 反向：白名单里的文件必须真的不可达，防止有人一边把它接回活树一边忘删登记。
    for dead in DEAD_FILES {
        assert!(
            !reachable.contains(&PathBuf::from(dead)),
            "已登记为死文件的 `{dead}` 现在能从 app.slint 到达 —— 请从 DEAD_FILES 与文档中移除"
        );
    }
}
