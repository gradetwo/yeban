//! 判据：**工程模板机制**（照 Logic Pro，Wang 2026-11-17 定案）。
//!
//! ## 机制口径（本文件钉住的四句话）
//!
//! 1. **模板不是新格式**：模板文件就是 `.yeban` 工程容器，存进模板目录
//!    就是模板（落盘走唯一写入口 `save::save_archive_file`）。
//! 2. **三级分类**：两个根目录下都按 曲风 / 子曲风 / 模板名 落盘；
//!    同名判定以三级路径为准（不同子曲风可同名）。
//! 3. **选择器分组与三级结构一致**：分组列表 = 来源 → 曲风 → 子曲风 →
//!    模板名，顺序确定（工厂在前、同级按字符串序）。
//! 4. **副本不回写**：从模板新建后工程无路径关联（保存落点为空 ⇒ 首存
//!    走问路径流程），编辑并保存后模板文件**字节不变**。
//!
//! ## 分工
//!
//! - 纯函数判据（本文件前半）：目录扫描的深度纪律、分组行模型、字段校验、
//!   落盘 / 已存在 / 覆盖 —— 全部对着 `crate::templates` 的纯函数量，
//!   无窗口、无对话框。
//! - 活窗口判据（本文件末尾一条）：生产同款装配（`LivePort` +
//!   `host::build_main_window` + `PortOpenProject`）里真的走完
//!   「另存为模板 → 同名拒写 → 覆盖 → 换掉会话 → 从模板新建 → 保存副本」
//!   全程，断言每一步的真实副作用（文件落点、工程指纹、模板字节）。
//!
//! ## 平台约束：活窗口判据只有一条
//!
//! `slint::platform::set_platform` 是线程局部的（与
//! `tests/aether_callback_matrix.rs` 同款约束）：一个文件里只建一次活窗口。

#![allow(unused)]
#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::ScriptedPicker;
use yeban_app::file_picker::FilePicker;
use yeban_app::templates::{
    ChooserRowKind, SaveTemplateError, TemplateRoots, TemplateSource, build_chooser_rows,
    ensure_user_template_root, list_templates, save_template_archive, template_target_path,
    user_category_hint, validate_template_name, validate_template_segment,
};
use yeban_model::container::ProjectArchive;
use yeban_model::ids::AssetHash;
use yeban_model::project::{AssetMetadata, ClipContent, MediaKind};

/// 临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `open_project_ui.rs`。
struct Scratch {
    dir: PathBuf,
    user_dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let id = yeban_model::ids::EntityId::new().to_canonical_string();
        let dir = std::env::temp_dir().join(format!(
            "yeban-project-templates-{}-{id}",
            std::process::id()
        ));
        // 用户模板根必须位于用户主目录下（生产同款约束）；测试目录在
        // Drop 里整棵删除，不污染仓库或用户的其它模板。
        let user_dir = yeban_model::preset::user_home_dir()
            .expect("测试机必须能解析用户主目录")
            .join(".yeban-project-template-tests")
            .join(format!("{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        std::fs::create_dir_all(&user_dir).expect("建用户模板测试目录");
        Self { dir, user_dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn roots(&self) -> TemplateRoots {
        TemplateRoots {
            factory: self.join("factory"),
            user: self.user_dir.clone(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.user_dir);
    }
}

/// 一份可辨识的归档（默认工程 + 可选的创建时间标记 ⇒ 字节可区分）。
fn archive() -> yeban_model::container::ProjectArchive {
    yeban_app::host::new_project_archive()
}

fn write_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("父目录")).expect("建目录");
    std::fs::write(path, bytes).expect("写夹具文件");
}

fn asset_metadata(hash: &AssetHash, original_path: &str, byte_len: usize) -> AssetMetadata {
    AssetMetadata {
        hash: hash.clone(),
        original_path: original_path.to_owned(),
        byte_len: byte_len as u64,
        media_kind: MediaKind::Audio,
        license: "test-fixture".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// 纯函数判据
// ---------------------------------------------------------------------------

/// 用户模板目录与用户预制库**同源**：同一个父目录（`Audio Music Apps`），
/// 名字是 Logic 的 `Project Templates`。
#[test]
fn user_template_dir_shares_the_preset_library_parent() {
    let templates = yeban_model::preset::user_template_library_dir()
        .expect("用户主目录必须可解析（Windows 可用 USERPROFILE，Unix 用 HOME）");
    let presets = yeban_model::preset::user_preset_library_dir()
        .expect("用户主目录必须可解析（Windows 可用 USERPROFILE，Unix 用 HOME）");
    assert_eq!(
        templates.parent(),
        presets.parent(),
        "模板目录必须与预制库同父（同源），不得新造一套路径规则"
    );
    assert_eq!(
        templates.file_name().and_then(|name| name.to_str()),
        Some("Project Templates")
    );
    // 工厂根 = 仓根 assets/templates（与仓内其它随分发资源同一棵树）。
    let roots = TemplateRoots::production().expect("生产模板根必须可解析");
    assert!(
        roots
            .factory
            .ends_with(Path::new("assets").join("templates")),
        "工厂模板根必须是 <仓根>/assets/templates，实际 {}",
        roots.factory.display()
    );
    assert!(
        roots.user.is_absolute(),
        "生产用户模板根必须是绝对路径，实际 {}",
        roots.user.display()
    );
}

#[cfg(windows)]
#[test]
fn windows_user_template_root_comes_from_userprofile() {
    let profile = std::env::var("USERPROFILE").expect("Windows 测试机应有 USERPROFILE");
    let root = yeban_model::preset::user_template_library_dir().expect("用户主目录必须可解析");
    assert!(
        root.starts_with(PathBuf::from(profile)),
        "Windows 模板根必须落在 USERPROFILE 下，实际 {}",
        root.display()
    );
    assert_eq!(
        root.file_name().and_then(|name| name.to_str()),
        Some("Project Templates")
    );
}

#[test]
fn user_template_root_rejects_relative_path() {
    let relative = Path::new("Music")
        .join("Audio Music Apps")
        .join("Project Templates");
    let error = ensure_user_template_root(&relative).expect_err("相对模板根必须拒绝");
    assert!(error.contains("绝对路径"), "错误须点名绝对路径: {error}");
    let error = save_template_archive(&relative, "流行", "摇滚", "相对", &archive(), false)
        .expect_err("相对模板根不得落盘");
    assert!(
        matches!(&error, SaveTemplateError::Invalid(message) if message.contains("绝对路径")),
        "保存也必须拒绝相对根: {error}"
    );
}

/// 另存为模板 ⇒ 文件落**用户目录三级路径**，且能被列表读回（来源 / 三级
/// 名字逐字对上）。
#[test]
fn save_lands_on_the_three_level_user_path_and_lists_back() {
    let scratch = Scratch::new();
    let roots = scratch.roots();
    let report = save_template_archive(&roots.user, "流行", "摇滚", "出发", &archive(), false)
        .expect("首次另存为模板必须成功");
    assert_eq!(
        report.path,
        roots.user.join("流行").join("摇滚").join("出发.yeban"),
        "落盘路径必须是 <用户根>/曲风/子曲风/模板名.yeban"
    );
    assert!(
        report.path.exists() && report.bytes > 0,
        "模板文件必须真的落盘且非空"
    );
    assert_eq!(report.undo_entries, 0, "模板快照不得携带撤销历史");
    assert_eq!(report.assets, 0, "默认工程没有资产引用，模板资产池必须为空");

    let listing = list_templates(&roots);
    assert_eq!(listing.len(), 1, "列表必须恰好读回这一条");
    let entry = &listing[0];
    assert_eq!(entry.source, TemplateSource::User);
    assert_eq!(entry.style, "流行");
    assert_eq!(entry.substyle, "摇滚");
    assert_eq!(entry.name, "出发");
    assert_eq!(entry.path, report.path);
}

/// 模板快照是**干净起点**：撤销历史 / 未合并提案 / 内嵌用户预制不进模板，
/// 资产池与 `project.assets` 索引只保留当前工程引用到的资产。判据同时对照
/// 同源全保真归档的字节数，证明模板真的小了，而不是仅仅换了个字段。
#[test]
fn template_snapshot_drops_history_proposals_presets_and_unreferenced_assets() {
    let scratch = Scratch::new();
    let roots = scratch.roots();

    let mut project = yeban_model::samples::filled_project();
    let referenced_assets = yeban_model::samples::filled_project_assets();
    assert_eq!(referenced_assets.len(), 1, "filled 夹具应恰好带一份 kick");
    let kick = referenced_assets[0].clone();
    assert!(
        project.assets.contains_key(&kick.0),
        "filled 工程的资产索引必须包含 kick"
    );

    // 1) 会话池里的未引用资产（已删片段残留的典型形态）。
    let garbage_bytes = vec![0x5A; 512 * 1024];
    let garbage_hash = AssetHash::of_bytes(&garbage_bytes);
    project.assets.insert(
        garbage_hash.clone(),
        asset_metadata(
            &garbage_hash,
            "unreferenced/garbage.wav",
            garbage_bytes.len(),
        ),
    );

    // 2) `embedded_samples` 引用到的资产：模板必须保留它（不是“全部丢掉”）。
    let embedded_bytes = vec![0x33; 8 * 1024];
    let embedded_hash = AssetHash::of_bytes(&embedded_bytes);
    project.assets.insert(
        embedded_hash.clone(),
        asset_metadata(
            &embedded_hash,
            "samples/kit/embedded.wav",
            embedded_bytes.len(),
        ),
    );
    project.embedded_samples.insert(
        "samples/kit/embedded.wav".to_owned(),
        embedded_hash.as_str().to_owned(),
    );

    let archive = ProjectArchive {
        project,
        history_dag: vec![0xA5; 128 * 1024],
        proposals: Some(vec![0xB6; 32 * 1024]),
        assets: vec![
            kick.clone(),
            (garbage_hash.clone(), garbage_bytes),
            (embedded_hash.clone(), embedded_bytes),
        ],
        embedded_presets: vec![(
            "presets/user/01ARZ3NDEKTSV4RRFFQ69G5FAV.json".to_owned(),
            vec![0xC7; 16 * 1024],
        )],
        load_notes: vec!["fixture load note".to_owned()],
    };

    let full_path = scratch.join("full.yeban");
    let full =
        yeban_app::save::save_archive_file(&archive, &full_path).expect("全保真对照归档必须能落盘");
    let report = save_template_archive(&roots.user, "流行", "摇滚", "快照", &archive, false)
        .expect("模板快照必须落盘");
    assert_eq!(
        report.assets, 2,
        "模板只应保留 clip 与 embedded_samples 引用到的资产"
    );
    assert_eq!(report.undo_entries, 0, "模板不得携带撤销历史");
    assert!(
        report.bytes < full.bytes,
        "模板必须显著小于全保真归档：template={} full={}",
        report.bytes,
        full.bytes
    );
    eprintln!(
        "[T-FIX-295] template_bytes={} full_bytes={} assets={} undo={}",
        report.bytes, full.bytes, report.assets, report.undo_entries
    );

    let read = yeban_app::open::open_project_archive_file(
        &report.path,
        &yeban_app::open::ProjectOpenOptions::default(),
    )
    .expect("快照模板必须能按生产口径打开");
    assert!(read.history_dag.is_empty(), "history.dag 必须是空字节");
    assert!(read.proposals.is_none(), "proposals 不得进模板");
    assert!(read.embedded_presets.is_empty(), "内嵌用户预制不得进模板");
    assert!(read.load_notes.is_empty(), "模板不该携带来源会话的装载留痕");

    let pool: Vec<&AssetHash> = read.assets.iter().map(|(hash, _bytes)| hash).collect();
    assert!(pool.contains(&&kick.0), "被 clip 引用的 kick 必须保留");
    assert!(
        pool.contains(&&embedded_hash),
        "embedded_samples 引用的资产必须保留"
    );
    assert!(!pool.contains(&&garbage_hash), "未引用资产字节不得进模板");
    assert!(read.project.assets.contains_key(&kick.0));
    assert!(read.project.assets.contains_key(&embedded_hash));
    assert!(
        !read.project.assets.contains_key(&garbage_hash),
        "未引用资产的索引项也必须裁掉，不能留下有元数据没字节的半截索引"
    );
}

/// 扫描只认**恰好三级**深度的 `.yeban`：根下直放、二级深度、四级深度、
/// 非 `.yeban` 后缀、文件冒充分类目录 —— 全部不进列表。
#[test]
fn listing_recognizes_only_exactly_three_levels() {
    let scratch = Scratch::new();
    let roots = scratch.roots();
    // 合法的一条（工厂）。
    write_file(
        &roots.factory.join("电子").join("舞曲").join("底鼓.yeban"),
        b"ok",
    );
    // 诱饵们。
    write_file(&roots.factory.join("根下直放.yeban"), b"decoy");
    write_file(&roots.factory.join("电子").join("二级深度.yeban"), b"decoy");
    write_file(
        &roots
            .factory
            .join("电子")
            .join("舞曲")
            .join("深处")
            .join("四级.yeban"),
        b"decoy",
    );
    write_file(
        &roots.factory.join("电子").join("舞曲").join("不是模板.txt"),
        b"decoy",
    );
    write_file(
        &roots.factory.join("电子").join("文件冒充子曲风.yeban"),
        b"decoy",
    );
    // 用户根不存在 ⇒ 该来源为空（不报错）。
    let listing = list_templates(&roots);
    assert_eq!(
        listing.len(),
        1,
        "只有恰好三级深度的 .yeban 是模板，实际读回 {listing:?}"
    );
    assert_eq!(listing[0].source, TemplateSource::Factory);
    assert_eq!(listing[0].name, "底鼓");
}

/// 选择器分组：来源（工厂在前）→ 曲风 → 子曲风 → 模板名，同级字符串序；
/// 两个来源有同名曲风时各自成组；行模型与三级结构逐行一致。
#[test]
fn chooser_rows_group_by_source_style_substyle_in_order() {
    let scratch = Scratch::new();
    let roots = scratch.roots();
    for (root, style, substyle, name) in [
        (&roots.user, "流行", "摇滚", "出发"),
        (&roots.user, "流行", "民谣", "归途"),
        (&roots.user, "流行", "摇滚", "安可"),
        (&roots.factory, "流行", "摇滚", "出厂"),
        (&roots.factory, "电子", "舞曲", "底鼓"),
    ] {
        write_file(
            &root
                .join(style)
                .join(substyle)
                .join(format!("{name}.yeban")),
            b"x",
        );
    }
    let listing = list_templates(&roots);
    let rows = build_chooser_rows(&listing, yeban_app::cli::Language::ZhCn);
    let shape: Vec<(ChooserRowKind, &str)> = rows
        .iter()
        .map(|row| (row.kind, row.label.as_str()))
        .collect();
    assert_eq!(
        shape,
        vec![
            (ChooserRowKind::Source, "工厂模板"),
            (ChooserRowKind::Style, "流行"),
            (ChooserRowKind::Substyle, "摇滚"),
            (ChooserRowKind::Entry, "出厂"),
            (ChooserRowKind::Style, "电子"),
            (ChooserRowKind::Substyle, "舞曲"),
            (ChooserRowKind::Entry, "底鼓"),
            (ChooserRowKind::Source, "我的模板"),
            (ChooserRowKind::Style, "流行"),
            (ChooserRowKind::Substyle, "摇滚"),
            (ChooserRowKind::Entry, "出发"),
            (ChooserRowKind::Entry, "安可"),
            (ChooserRowKind::Substyle, "民谣"),
            (ChooserRowKind::Entry, "归途"),
        ],
        "分组行必须与三级结构逐行一致（来源 → 曲风 → 子曲风 → 模板名；同级按字符串码点序）"
    );
    // 条目行的下标必须指回清单里那一条（宿主靠它打开文件）。
    for row in &rows {
        if let Some(index) = row.entry_index {
            assert_eq!(listing[index].name, row.label);
        } else {
            assert_ne!(row.kind, ChooserRowKind::Entry, "标题行不得带条目下标");
        }
    }
    // 行种类的整数编码与 `.slint` 侧同值（0/1/2/3）。
    assert_eq!(ChooserRowKind::Source.code(), 0);
    assert_eq!(ChooserRowKind::Style.code(), 1);
    assert_eq!(ChooserRowKind::Substyle.code(), 2);
    assert_eq!(ChooserRowKind::Entry.code(), 3);
    // 分类提示只含用户模板的组合（排序去重）。
    assert_eq!(
        user_category_hint(&listing),
        vec!["流行 / 摇滚".to_owned(), "流行 / 民谣".to_owned()]
    );
}

/// 同名判定以三级路径为准：不同子曲风下同名可共存；同一三级路径二次
/// 保存（不覆盖）⇒ `Exists` 且**一字节不动**；明说覆盖 ⇒ 替换。
#[test]
fn same_name_scopes_to_the_three_level_path() {
    let scratch = Scratch::new();
    let roots = scratch.roots();
    let first = archive();
    let mut second_project = yeban_model::samples::default_project();
    second_project.record_creation_time(42);
    let second = yeban_model::container::ProjectArchive {
        project: second_project,
        history_dag: Vec::new(),
        proposals: None,
        assets: Vec::new(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };

    // 不同子曲风，同名 —— 两条都成立。
    save_template_archive(&roots.user, "流行", "摇滚", "同名", &first, false)
        .expect("第一条同名模板");
    save_template_archive(&roots.user, "流行", "民谣", "同名", &second, false)
        .expect("不同子曲风下的同名模板必须允许");
    assert_eq!(list_templates(&roots).len(), 2);

    // 同一三级路径，不覆盖 ⇒ Exists，且文件字节一位不动。
    let target = template_target_path(&roots.user, "流行", "摇滚", "同名");
    let before = std::fs::read(&target).expect("读模板字节");
    let refused = save_template_archive(&roots.user, "流行", "摇滚", "同名", &second, false);
    assert!(
        matches!(refused, Err(SaveTemplateError::Exists(_))),
        "同名拒写必须回 Exists，实际 {refused:?}"
    );
    assert_eq!(
        std::fs::read(&target).expect("再读模板字节"),
        before,
        "拒写之后模板文件必须一字节不动"
    );

    // 明说覆盖 ⇒ 字节换成第二份归档的。
    let report = save_template_archive(&roots.user, "流行", "摇滚", "同名", &second, true)
        .expect("覆盖保存必须成功");
    assert_eq!(report.path, target);
    assert_ne!(
        std::fs::read(&target).expect("读覆盖后的模板字节"),
        before,
        "覆盖后模板字节必须换成新归档"
    );
}

/// T-FIX-294：两个线程同时对同一三级路径 `overwrite=false` 保存 ——
/// 恰好一个成功、一个 `Exists`，最终文件是胜者的字节。锁内重查存在性，
/// 不再有 check-then-write 的并发覆盖窗口。
#[test]
fn concurrent_saves_without_overwrite_have_one_winner_and_one_exists() {
    use std::sync::{Arc, Barrier};

    let scratch = Scratch::new();
    let roots = scratch.roots();
    let first = archive();
    let mut second_project = yeban_model::samples::default_project();
    second_project.record_creation_time(42);
    let second = yeban_model::container::ProjectArchive {
        project: second_project,
        history_dag: Vec::new(),
        proposals: None,
        assets: Vec::new(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    };

    fn serialized(archive: &yeban_model::container::ProjectArchive) -> Vec<u8> {
        let assets: Vec<(&yeban_model::ids::AssetHash, &[u8])> = archive
            .assets
            .iter()
            .map(|(hash, data)| (hash, data.as_slice()))
            .collect();
        let presets: Vec<(&str, &[u8])> = archive
            .embedded_presets
            .iter()
            .map(|(name, data)| (name.as_str(), data.as_slice()))
            .collect();
        yeban_model::container::write_project_container_with_proposals_borrowed(
            &archive.project,
            &archive.history_dag,
            &assets,
            &presets,
            archive.proposals.as_deref(),
        )
        .expect("序列化期望字节")
    }

    let expected_first = serialized(&first);
    let expected_second = serialized(&second);
    let user_root = roots.user.clone();
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for (label, archive) in [("first", first), ("second", second)] {
        let user_root = user_root.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let result = save_template_archive(&user_root, "流行", "摇滚", "并发", &archive, false);
            (label, result)
        }));
    }

    let mut successes = Vec::new();
    let mut exists = Vec::new();
    for handle in handles {
        let (label, result) = handle.join().expect("并发保存线程");
        match result {
            Ok((path, bytes)) => successes.push((label, path, bytes)),
            Err(SaveTemplateError::Exists(path)) => exists.push((label, path)),
            Err(error) => panic!("并发保存只能成功或 Exists，实际 {label}: {error}"),
        }
    }
    assert_eq!(successes.len(), 1, "恰好一个线程成功");
    assert_eq!(exists.len(), 1, "恰好一个线程拿到 Exists");

    let target = template_target_path(&roots.user, "流行", "摇滚", "并发");
    let (winner, path, bytes) = &successes[0];
    let (loser, exists_path) = &exists[0];
    assert_eq!(path, &target);
    assert_eq!(exists_path, &target);
    assert!(*bytes > 0, "胜者必须写出非空字节");
    assert_ne!(winner, loser);
    let expected = if winner == &"first" {
        &expected_first
    } else {
        &expected_second
    };
    assert_eq!(
        &std::fs::read(&target).expect("读并发保存后的模板"),
        expected,
        "最终文件必须是胜者的完整字节，败者不得写入任何字节"
    );
}

/// 字段校验：路径段绝不能逃出模板根（分隔符 / 点段 / 隐藏名 / 空）。
#[test]
fn segment_validation_rejects_path_escapes() {
    for bad in ["", "   ", "a/b", "a\\b", ".", "..", ".隐藏", "a\tb"] {
        assert!(
            validate_template_segment("曲风", bad).is_err(),
            "字段 `{bad}` 必须被拒绝"
        );
    }
    assert_eq!(
        validate_template_segment("曲风", " 流行 ").expect("合法字段"),
        "流行",
        "首尾空白要裁掉"
    );
    // 模板名顺手带了后缀 ⇒ 剥掉一个，落盘不产生双后缀。
    assert_eq!(
        validate_template_name("出发.yeban").expect("带后缀的名字"),
        "出发"
    );
    assert_eq!(validate_template_name("出发").expect("普通名字"), "出发");
}

/// 菜单 → 命令 → 宿主落点的源码对账（与 `ui_menu_completeness` 互补：
/// 那条只查命令字面量在 host.rs 出现，本条钉住**分发臂的形状**）。
#[test]
fn template_menu_commands_have_dispatch_arms() {
    let app = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"))
        .expect("读 app.slint");
    let host = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"))
        .expect("读 host.rs");
    assert!(
        app.contains("从模板新建..."),
        "文件菜单必须有「从模板新建...」"
    );
    assert!(
        app.contains("另存为模板..."),
        "文件菜单必须有「另存为模板...」"
    );
    assert!(
        host.contains("\"file-new-from-template\" => ui.invoke_new_from_template(),"),
        "dispatch_aether_command 必须有 file-new-from-template 臂"
    );
    assert!(
        host.contains("\"file-save-as-template\" => ui.invoke_save_as_template(),"),
        "dispatch_aether_command 必须有 file-save-as-template 臂"
    );
    // elements.rs 的行族登记范围必须与画布上限同源（同一个常数）。
    let elements =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/elements.rs"))
            .expect("读 elements.rs");
    assert!(
        elements.contains("crate::templates::TEMPLATE_CHOOSER_MAX_ROWS"),
        "elements.rs 登记 template-row-* 必须引用 TEMPLATE_CHOOSER_MAX_ROWS 常数"
    );
}

// ---------------------------------------------------------------------------
// 活窗口判据：全流程
// ---------------------------------------------------------------------------

/// 撤销会话的"现在"（判据不需要墙钟，与同类判据同款常量）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 全流程：另存为模板 → 同名拒写 → 覆盖 → 换掉会话 → 从模板新建
/// （副本）→ 保存副本到新路径 ⇒ **模板文件字节不变**。
#[test]
fn template_round_trip_in_a_live_window_never_writes_back() {
    use slint::Model as _;
    use yeban_app::bridge::ViewState;
    use yeban_app::host;
    use yeban_app::host::ProductionLoop;
    use yeban_app::scene::DemoScene;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    let scratch = Scratch::new();
    let roots = scratch.roots();
    std::fs::create_dir_all(&roots.factory).expect("建工厂根");

    // ---- 装配（生产同款形态）：会话 = 工程 A（filled）-------------------
    let project_a = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project_a).expect("工程 A 必须能投影");
    let scene = DemoScene::from_view(&view);
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<templates>", "yeban-app", project_a.clone(), NOW_MS)
            .expect("撤销会话必须能打开"),
    ));
    // T-FIX-411（330 返工）：`filled_project()` 的 Audio 片段引用运行期 kick；
    // 真实会话是从 `.yeban` 容器装载的，资产池里必有这份字节。旧夹具直接从
    // 工程对象构造会话、池为空 ⇒ 另存模板写出一个缺资产的模板，330 严格打开
    // 正确地拒绝它。这里按生产装载形态把同一份资产字节填进会话池（修法 b），
    // 并在下面断言模板容器真的带上了它。
    undo.replace_assets(yeban_model::samples::filled_project_assets());
    // 真正种一段撤销历史：模板必须把它剥掉，而不是只剥一个空字段。
    let first_track = project_a
        .tracks
        .keys()
        .next()
        .cloned()
        .expect("filled 夹具必须至少有一条轨");
    let pan_before = project_a.tracks[&first_track].pan;
    let pan_new = if pan_before > 0.0 { -0.5 } else { 0.5 };
    undo.commit_ops(
        NOW_MS + 1,
        "fixture: template must not inherit undo history",
        vec![yeban_model::Op::SetParam {
            target: yeban_model::AutomationTarget::TrackPan {
                track_id: first_track,
            },
            old_val: pan_before,
            new_val: pan_new,
        }],
    )
    .expect("夹具声相提交必须被接受");
    let project_a = undo.try_project().expect("提交后的工程 A");
    assert!(
        undo.display().undoable >= 1,
        "夹具必须真的有一段可撤销历史（否则本判据证明不了剥离）"
    );
    let fingerprint_a = undo.fingerprint().expect("会话指纹");
    let production = Rc::new(RefCell::new(
        ProductionLoop::start(&project_a, 0).expect("引擎重建"),
    ));

    let live = LivePort::new(Size::new(1440, 900), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 活窗口");
    let ui = live.ui();

    // ---- 未命名变体的 after（与 main.rs 的 `make_after(true)` 同一条语义：
    //      保存落点 = None；保存要问的路径由脚本选择器给）------------------
    let copy_path = scratch.join("副本.yeban");
    let picker = ScriptedPicker::answering_save(copy_path.clone());
    let after: host::OpenProjectAfter = {
        let port_after = Rc::clone(&undo);
        let picker_after: Rc<dyn FilePicker> = {
            let owned: Rc<ScriptedPicker> = Rc::clone(&picker);
            owned
        };
        Rc::new(move |ui, project, _path, _view| {
            host::wire_save_with_engine(
                ui,
                host::SaveStatus {
                    target: None,
                    project: Rc::new(RefCell::new(project.clone())),
                },
                Some(Rc::clone(&port_after)),
                Rc::clone(&picker_after),
                None,
                None,
            );
            Ok(())
        })
    };
    let runtime: Rc<dyn host::OpenProjectRuntime> = Rc::new(host::PortOpenProject {
        port: Rc::clone(&undo),
        production: Some(production),
        quanta: 0,
        after,
    });
    host::wire_project_templates(ui, Some(Rc::clone(&undo)), runtime, roots.clone());

    // ---- 1. 另存为模板 ---------------------------------------------------
    ui.invoke_save_as_template();
    assert!(
        ui.get_template_save_open(),
        "菜单回调必须打开另存为模板对话框"
    );
    ui.set_template_save_style("流行".into());
    ui.set_template_save_substyle("摇滚".into());
    ui.set_template_save_name("出发".into());
    ui.invoke_confirm_save_as_template();
    let template_path = roots.user.join("流行").join("摇滚").join("出发.yeban");
    assert!(
        template_path.exists(),
        "另存为模板后文件必须落在用户目录三级路径 {}",
        template_path.display()
    );
    // 模板必须带上当前工程引用的资产：否则从模板新建会被 330 的严格打开拒绝
    // （旧夹具在步骤 4 就停在这里，指纹停在 C 而不是 A）。
    let template_archive = yeban_app::open::open_project_archive_file(
        &template_path,
        &yeban_app::open::ProjectOpenOptions::default(),
    )
    .expect("模板必须是可打开的容器");
    assert_eq!(
        template_archive.assets.len(),
        1,
        "filled 模板的 kick 资产字节不得在另存为模板时丢失"
    );
    assert!(
        template_archive.history_dag.is_empty(),
        "来源会话确实有可撤销历史，但模板必须把 history.dag 剥空"
    );
    assert!(!ui.get_template_save_open(), "保存成功后对话框必须关");
    assert!(
        ui.get_module_status().contains("save-as-template")
            && ui.get_module_status().contains("outcome=ok")
            && ui.get_module_status().contains("assets=1")
            && ui.get_module_status().contains("undo=0"),
        "回执必须走 module-status 且报 ok + 资产数 + 撤销条目数，实际 {}",
        ui.get_module_status()
    );

    // ---- 2. 同名拒写 → 覆盖 ----------------------------------------------
    ui.invoke_save_as_template();
    ui.set_template_save_style("流行".into());
    ui.set_template_save_substyle("摇滚".into());
    ui.set_template_save_name("出发".into());
    let bytes_before_refusal = std::fs::read(&template_path).expect("读模板字节");
    ui.invoke_confirm_save_as_template();
    assert!(
        ui.get_template_save_exists(),
        "同名再存必须置 exists（等用户确认覆盖），不得静默覆盖"
    );
    assert!(ui.get_template_save_open(), "等覆盖确认时对话框必须留着");
    assert_eq!(
        std::fs::read(&template_path).expect("再读模板字节"),
        bytes_before_refusal,
        "拒写后模板字节必须不变"
    );
    ui.invoke_confirm_template_overwrite();
    assert!(!ui.get_template_save_open(), "覆盖确认后对话框必须关");
    assert!(
        ui.get_module_status().contains("overwrite=true"),
        "覆盖回执必须如实报 overwrite=true，实际 {}",
        ui.get_module_status()
    );
    let template_bytes = std::fs::read(&template_path).expect("读覆盖后的模板字节");

    // ---- 3. 把会话换成另一份工程（模拟用户已经在做别的工程） -------------
    let project_c = yeban_model::samples::default_project();
    undo.open_project_session("setup-c", project_c, Vec::new(), NOW_MS)
        .expect("换会话");
    let fingerprint_c = undo.fingerprint().expect("会话 C 指纹");
    assert_ne!(fingerprint_a, fingerprint_c, "夹具：A 与 C 必须是两份工程");

    // ---- 4. 从模板新建：行模型分组正确，选中后会话回到 A -----------------
    ui.invoke_new_from_template();
    assert!(ui.get_template_chooser_open(), "菜单回调必须打开模板选择器");
    let rows = ui.get_template_chooser_rows();
    let shape: Vec<(i32, String)> = rows
        .iter()
        .map(|row| (row.kind, row.label.to_string()))
        .collect();
    assert_eq!(
        shape,
        vec![
            (0, "我的模板".to_owned()),
            (1, "流行".to_owned()),
            (2, "摇滚".to_owned()),
            (3, "出发".to_owned()),
        ],
        "选择器行模型必须按 来源 → 曲风 → 子曲风 → 模板名 分组"
    );
    assert_eq!(ui.get_template_chooser_hidden_rows(), 0, "四行不得有截断");
    let entry_index = rows
        .iter()
        .find(|row| row.kind == 3)
        .map(|row| row.entry_index)
        .expect("必须有条目行");
    ui.invoke_template_selected(entry_index);
    assert!(!ui.get_template_chooser_open(), "选中后选择器必须关");
    assert_eq!(
        undo.try_project().as_ref(),
        Some(&project_a),
        "从模板新建后工程内容必须是模板里的工程"
    );
    assert!(
        !undo.display().can_undo && undo.display().undoable == 0,
        "从模板新建后撤销栈必须从空开始（不得继承来源会话历史）"
    );
    assert_eq!(
        undo.fingerprint().as_deref(),
        Some(fingerprint_a.as_str()),
        "模板里的工程内容必须等于 A；历史已剥离，撤销栈空由独立断言证明"
    );
    assert!(
        ui.get_module_status().contains("new-from-template")
            && ui.get_module_status().contains("outcome=ok"),
        "从模板新建回执必须报 ok，实际 {}",
        ui.get_module_status()
    );

    // ---- 5. 保存副本：落点为空 ⇒ 问路径 ⇒ 写到新文件；模板字节不变 -------
    assert_eq!(picker.call_count("save"), 0, "打开模板不得触发任何保存问询");
    ui.invoke_save_project();
    assert_eq!(
        picker.call_count("save"),
        1,
        "副本无路径关联 ⇒ 首存必须问一次路径（走另存流程）"
    );
    assert!(copy_path.exists(), "副本必须写到问来的新路径");
    assert_eq!(
        std::fs::read(&template_path).expect("读模板字节"),
        template_bytes,
        "编辑并保存副本之后，模板文件必须一字节不变（绝不回写）"
    );
}
