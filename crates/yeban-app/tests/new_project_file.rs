//! 判据（任务单 `#129`，无头腿）：**"新建工程"的落盘部分真的写出 884 B／0 轨**。
//!
//! ## 这个文件为什么存在（以及它**不**测什么）
//!
//! `#129` 的产品动作是"按钮 ⇒ 保存对话框 ⇒ 落盘 ⇒ 就地换成新工程"。其中**对话框**那一跳
//! 需要真的原生窗口（本机 CI 起不来），**界面**那一跳需要 Tier-1 执行面（另一个文件
//! `tests/new_project_ui.rs`）。剩下的那一半 —— "一个空工程 ⇒ 一个 `.yeban` 文件" ——
//! 是**纯的**，因此被单独抽成 `host::new_project_archive()` ＋
//! `host::create_new_project_file(path)`，并在这里**无窗口、无对话框**地量。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 文件大小 | `std::fs::metadata(path).len()` | 字节 |
//! | `SaveReport::bytes` | 写入方自报的容器长度 | 字节 |
//! | 轨道数 | 读回后的 `YebanProjectV1.tracks` | 条 |
//! | `project-counts:` | `cli::project_report`（**与 CLI 同一条**投影） | 结构化文本 |
//! | 工程指纹 | `undo::project_fingerprint`（模型容器字节的 SHA-256） | 十六进制串 |
//!
//! ## 这条判据怎么变红（本单注入读数见交付报告）
//!
//! - `new_project_archive()` 换成 `filled_project()`（3 轨）⇒ 字节数与 `tracks-all=0` 两条红；
//! - `create_new_project_file` 不调 `save_archive_file`（例如只 `fs::write` 空字节）⇒
//!   "读回"那条红（不是容器 / 0 轨）；
//! - 去掉 `history_dag: Vec::new()` 的如实置空（改成某份非空图谱）⇒ 字节数红。
//!
//! ## 边界（如实登记）
//!
//! 本文件**不**碰 Slint、**不**建窗口、**不**碰对话框：那两跳在
//! `tests/new_project_ui.rs`（一次 Tier-1 执行面）与产品 `run_gui` 里。

use std::path::PathBuf;

/// 一次性的临时目录（`Drop` 时清理，**绝不污染仓库**）。形态照 `open_project_ui.rs`。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-new-project-file-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// ⭐ 本单的**核心读数**：`host::create_new_project_file` 写出的文件是
/// **884 字节 / 0 轨**，且它与 CLI `--project-sample empty --save-as <path>`
/// 是同一条落点（`save::save_archive_file` + `samples::default_project()`）。
#[test]
fn create_new_project_file_writes_the_884_byte_zero_track_container() {
    let scratch = Scratch::new();
    let path = scratch.join("brand-new.yeban");

    // ---- ① 落盘（无窗口、无对话框）----
    let report = yeban_app::host::create_new_project_file(&path).expect("新建工程的落盘必须成功");
    let size = std::fs::metadata(&path).expect("文件必须真的存在").len();
    yeban_ui_test_port::render::report_line(&format!(
        "[#129/file] 落盘: path={} 文件大小={size} B SaveReport::bytes={} temp_name={:?}",
        path.display(),
        report.bytes,
        report.temp_name,
    ));

    // ---- ② 字节数：884（与 `#128` 实测的 CLI `--save-as` 产物同一条落点）----
    assert_eq!(
        size, 884,
        "新建工程的容器必须恰是 884 字节（`#128` 实测：`--project-sample empty --save-as` \
         的 `saved: … bytes=884`）；实得 {size} 字节 ⇒ 空工程对象 / 图谱 / 资产池有一项变了"
    );
    assert_eq!(
        report.bytes as u64, size,
        "`SaveReport::bytes` 必须等于磁盘上的真实字节数（写出去的与报出来的不能分叉）"
    );
    assert_eq!(
        report.path, path,
        "`SaveReport::path` 必须就是调用方给的那一条"
    );
    assert!(
        !scratch.dir.join(&report.temp_name).exists(),
        "原子写成功后不得留下临时文件 `{}`",
        report.temp_name
    );

    // ---- ③ 读回：0 轨（唯一的读盘入口 `open::open_project_file`）----
    let project = yeban_app::open::open_project_file(&path).expect("刚写出的容器必须能被读回");
    assert!(
        project.tracks.is_empty(),
        "新建工程的轨道集合必须为空（实得 {} 条）",
        project.tracks.len()
    );
    assert!(
        project.master_bus_track_id.is_nil(),
        "0 轨工程没有主总线身份（`validate` 的 0 轨分支要求它是 nil）"
    );
    assert!(
        project.clip_pool.is_empty() && project.scenes.is_empty() && project.sections.is_empty(),
        "空工程不许夹带片段 / 场景 / 段落"
    );
    project.validate().expect("0 轨空工程必须通过模型层校验");
    assert_eq!(
        project,
        yeban_model::samples::default_project(),
        "写出去又读回来的工程必须**逐字段**等于 `samples::default_project()` \
         （`default_project()` 就是 884 B／0 轨那条的模型侧对象）"
    );
    let fingerprint = yeban_app::undo::project_fingerprint(&project).expect("工程指纹");
    assert_eq!(
        Some(fingerprint.as_str()),
        yeban_app::undo::project_fingerprint(&yeban_model::samples::default_project())
            .ok()
            .as_deref(),
        "指纹必须与 `default_project()` 相同（这是与界面那条腿对账的同一个量）"
    );

    // ---- ④ `project-counts: tracks-all=0`（与 CLI **同一条**报告投影）----
    let archive = yeban_app::open::open_project_archive_file(
        &path,
        &yeban_app::open::ProjectOpenOptions::default(),
    )
    .expect("全保真读回");
    assert!(
        archive.history_dag.is_empty() && archive.assets.is_empty(),
        "样本形态的第二 / 第三层状态必须**如实置空**（与 CLI 的样本分支同一条口径）"
    );
    let loaded = yeban_app::cli::Loaded {
        archive,
        source: yeban_app::cli::ProjectSource::File {
            path: path.clone(),
            bytes: size,
        },
    };
    let lines = yeban_app::cli::project_report(&loaded);
    let counts = lines
        .iter()
        .find(|line| line.starts_with("project-counts:"))
        .expect("必须有 project-counts 行");
    yeban_ui_test_port::render::report_line(&format!("[#129/file] {counts}"));
    assert!(
        counts.contains("tracks-all=0 master-track=0"),
        "新建工程必须是**真的 0 轨**（`tracks-all` 含主总线 ⇒ 0 轨 = 0/0）；实得 {counts:?}"
    );

    // 反向对照：`filled_project()` 的轨道数**不是** 0 ⇒ 上面的 `0` 不是"读数函数返回常数"。
    let filled = yeban_app::cli::Loaded {
        archive: yeban_model::container::ProjectArchive {
            project: yeban_model::samples::filled_project(),
            history_dag: Vec::new(),
            assets: Vec::new(),
        },
        source: yeban_app::cli::ProjectSource::Sample(yeban_app::cli::Sample::Filled),
    };
    let filled_counts = yeban_app::cli::project_report(&filled)
        .into_iter()
        .find(|line| line.starts_with("project-counts:"))
        .expect("对照也必须有一行");
    yeban_ui_test_port::render::report_line(&format!(
        "[#129/file] 对照（filled_project）: {filled_counts}"
    ));
    assert!(
        !filled_counts.contains("tracks-all=0 "),
        "对照工程的 `tracks-all` 必须**非** 0（否则『0 轨』这条判据没有对照）；实得 {filled_counts:?}"
    );
}
