//! **D46 长度约束的机械守卫**（R142/R146/R151：契约与实现**两侧**都要判）＋ **私有目录守卫**（R141）。
//!
//! ## 为什么需要这一份
//!
//! 第六批实测（已入 R159）：把 `maxLength` 写进 `schemas/**` **不会**改变任何行为 ——
//! "契约说有界、实现无界"是 **R142 的静默永不生效**；而 `properties.insert` 的**覆盖**
//! 又会让"校验面"与"广播面"分叉（`yeban_edit_notes` 自己声明了一份 `idempotencyKey`
//! ⇒ 公共那份的 `maxLength` 在 `tools/list` 里**消失**）。本文件把这三件事机械化：
//!
//! | 判据 | 它挡住什么 |
//! | :--- | :--- |
//! | [`every_declared_bound_is_broadcast_and_every_broadcast_bound_is_declared`] | 声明了约束却不广播 / 广播了没声明的约束（**两方向**） |
//! | [`every_bounded_param_has_a_registered_behavioural_criterion`] | 有约束但**没有**任何判据证明它**真的判**（allowlist 归零） |
//! | [`no_private_work_directory_lives_inside_the_repository`] | 私有目录落进仓库树（R141 ⇒ R103 干净性检查会 REFUSED） |
//!
//! ## 单位（R149/R150/R156：口径进表头，⛔ 不数注释行）
//!
//! * 判据一数的单位 = **`ParamSpec` 实例**（`ToolSpec::all_params()` 的笛卡尔展开），
//!   口径是"`min_len`/`max_len` 为 `Some` 的实例"与"`input_schema()` 里带
//!   `minLength`/`maxLength` 的**属性**"两边的**集合相等**；
//! * 判据二数的单位 = **注册表的条目**（`(参数名, 判据名)` 对），⛔ 不是源码行数。

use std::collections::BTreeSet;
use std::path::PathBuf;

use yeban_mcp::tools::{TOOLS, ToolSpec};

/// **已被行为判据证明"真的判"**的受约束参数（R151：每个实例各配一条判据）。
///
/// ⚠ **allowlist 必须归零**（R157）：注册表 = 全部受约束参数 ⇒ 多一个就红、少一个也红。
const ENFORCED_BOUNDS: [(&str, &str); 2] = [
    (
        "idempotencyKey",
        "the_idempotency_key_boundary_is_emptiness_and_256_codepoints",
    ),
    (
        "name",
        "an_empty_import_name_is_rejected_and_never_lands_in_the_document",
    ),
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 一个受约束参数在某工具里的 `(工具名, 参数名, min, max)`。
fn declared_bounds(spec: &ToolSpec) -> Vec<(String, String, Option<usize>, Option<usize>)> {
    spec.all_params()
        .into_iter()
        .filter(|param| param.min_len.is_some() || param.max_len.is_some())
        .map(|param| {
            (
                spec.name.to_owned(),
                param.name.to_owned(),
                param.min_len,
                param.max_len,
            )
        })
        .collect()
}

#[test]
fn every_declared_bound_is_broadcast_and_every_broadcast_bound_is_declared() {
    // ⭐ R56：先喂**一条已知红**（广播缺 `minLength`）与**一条已知绿**（广播与声明一致）。
    let red = serde_json::json!({"type": "string"});
    let green = serde_json::json!({"type": "string", "minLength": 1});
    let broadcast = |property: &serde_json::Value| -> (Option<u64>, Option<u64>) {
        (
            property
                .get("minLength")
                .and_then(serde_json::Value::as_u64),
            property
                .get("maxLength")
                .and_then(serde_json::Value::as_u64),
        )
    };
    assert_eq!(broadcast(&red), (None, None), "已知红样本：广播里没有约束");
    assert_eq!(
        broadcast(&green),
        (Some(1), None),
        "已知绿样本：广播里有约束"
    );

    // 方向 ①：凡**声明**了约束的实例，`tools/list` 里必须**如实广播**（同一对数值）。
    // 方向 ②：凡**广播**了约束的属性，注册表里必须**真的声明**了它。
    let mut declared: BTreeSet<(String, Option<usize>, Option<usize>)> = BTreeSet::new();
    let mut broadcasted: BTreeSet<(String, Option<usize>, Option<usize>)> = BTreeSet::new();
    let mut instances = 0usize;
    let mut constrained_instances = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for spec in &TOOLS {
        let schema = spec.input_schema();
        let properties = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("`{}` 的 inputSchema 必须有 properties", spec.name));
        for (tool, name, min, max) in declared_bounds(spec) {
            instances += 1;
            constrained_instances += 1;
            declared.insert((name.clone(), min, max));
            let (seen_min, seen_max) = broadcast(&properties[&name]);
            if seen_min != min.map(|value| value as u64)
                || seen_max != max.map(|value| value as u64)
            {
                failures.push(format!(
                    "{tool}.{name} 声明 (min={min:?}, max={max:?}) 但广播 (min={seen_min:?}, max={seen_max:?})"
                ));
            }
        }
        for (name, property) in properties {
            let (min, max) = broadcast(property);
            if min.is_some() || max.is_some() {
                broadcasted.insert((
                    name.clone(),
                    min.map(|v| v as usize),
                    max.map(|v| v as usize),
                ));
            }
        }
    }
    // ⭐ R119/R120（覆盖假象）：被扫集合必须非空且达下界 —— 否则"没有违规"可能只是**什么都没扫到**。
    assert!(
        declared.len() >= 2 && constrained_instances >= 17,
        "受约束参数的种类必须 ≥ 2、实例必须 ≥ 17（公共参数会展开到每个工具）：\
         declared={declared:?} instances={instances} constrained={constrained_instances}"
    );
    assert_eq!(
        declared.len(),
        broadcasted.len(),
        "受约束的**种类**数必须两边相等（声明 {declared:?} vs 广播 {broadcasted:?}）"
    );
    assert_eq!(
        declared, broadcasted,
        "声明与广播必须是**同一个集合**（方向 ① 声明了要广播 / 方向 ② 广播了要声明）"
    );
    assert!(failures.is_empty(), "声明与广播的数值不一致：{failures:#?}");
}

#[test]
fn every_bounded_param_has_a_registered_behavioural_criterion() {
    let mut unregistered: Vec<String> = Vec::new();
    let mut bounded_names: BTreeSet<String> = BTreeSet::new();
    for spec in &TOOLS {
        for (_tool, name, _min, _max) in declared_bounds(spec) {
            bounded_names.insert(name);
        }
    }
    for name in &bounded_names {
        if !ENFORCED_BOUNDS.iter().any(|(bound, _)| bound == name) {
            unregistered.push(name.clone());
        }
    }
    // allowlist 归零（R157）：注册表既不能少（有约束没人证明）也不能多（注册了不存在的约束）。
    assert!(
        unregistered.is_empty(),
        "这些受约束参数**没有**行为判据证明它真的判：{unregistered:?}"
    );
    let mut stale: Vec<&str> = Vec::new();
    for (bound, criterion) in ENFORCED_BOUNDS {
        if !bounded_names.contains(bound) {
            stale.push(bound);
        }
        assert!(
            TOOLS.iter().any(|spec| spec
                .all_params()
                .iter()
                .any(|param| param.name == bound
                    && (param.min_len.is_some() || param.max_len.is_some()))),
            "注册表里的 `{bound}` 在 `ParamSpec` 里找不到对应约束（注册表腐烂）"
        );
        assert!(
            !criterion.is_empty(),
            "每个实例都必须点名它自己的判据（⛔ 不许只写'已测'）"
        );
    }
    assert!(stale.is_empty(), "注册表里有不存在的参数：{stale:?}");
    // 判据名必须真的存在于测试源码里（否则"注册"只是字符串）——见下一条判据。
    assert_eq!(
        ENFORCED_BOUNDS.len(),
        bounded_names.len(),
        "注册表必须**恰好**覆盖"
    );
}

#[test]
fn the_registered_criteria_really_exist_in_the_sources() {
    let root = manifest_dir();
    let mut sources = String::new();
    let mut files = 0usize;
    for dir in [root.join("src"), root.join("tests")] {
        let mut stack = vec![dir];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    files += 1;
                    sources.push_str(&text);
                    sources.push('\n');
                }
            }
        }
    }
    // ⭐ R119/R120：扫描面非空且达下界（本 crate 的 .rs 文件数 ≥ 50）。
    assert!(
        files >= 50,
        "源码扫描面太小（{files} 个 .rs）—— 目录枚举可能写错了"
    );
    for (bound, criterion) in ENFORCED_BOUNDS {
        assert!(
            sources.contains(&format!("fn {criterion}(")),
            "注册表点名的判据 `{criterion}`（为 `{bound}`）在源码里找不到"
        );
    }
}

#[test]
fn no_private_work_directory_lives_inside_the_repository() {
    // ⭐ R141：私有工作目录必须在**会话工作区根**（`/Users/crow/work/music/.mod-mcp/`），
    // ⛔ 不在仓库工作树内（否则 R103 的干净性检查会 REFUSED）。
    // 单位 = **仓库工作树内的 `.mod-*` 目录条目数**（必须为 0）。
    let repo_root = manifest_dir()
        .parent() // crates/
        .and_then(|path| path.parent()) // 仓库根（worktree）
        .expect("仓库根")
        .to_path_buf();
    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    // 只扫**仓库根的第一层**与 `crates/**` 的一层：私有目录按约定落在根或 crate 根。
    let mut dirs = vec![repo_root.clone(), repo_root.join("crates")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            scanned += 1;
            if name.starts_with(".mod-") {
                offenders.push(entry.path().display().to_string());
            }
        }
    }
    assert!(
        scanned >= 10,
        "目录扫描面太小（{scanned} 个条目）—— 仓库根解析可能错了"
    );
    assert!(
        offenders.is_empty(),
        "仓库树内不得有私有工作目录（R141：应放在会话工作区根的 `.mod-mcp/`）：{offenders:#?}"
    );
}
