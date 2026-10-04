//! `yeban-app` 的**公开** `.yeban` 打开入口（`yeban_app::open`）的集成判据。
//!
//! 规范来源 (Normative)：
//! - `[ARCH-SEC-003]` / `[MUST-GATE-006]` / `[MUST-GATE-007]`：容器防御由
//!   `yeban_model::container` 执行，本入口只负责**如实**转达裁决；
//! - `[MODEL-AST-002]`：打开的结果就是权威工程结构 `YebanProjectV1`；
//! - `AGENTS.md` §3 DoD 6：UI 相关的验证必须是可自动化的稳定断言 —— 本判据不碰界面，
//!   它证明的是"app 侧真的能打开一个磁盘上的 `.yeban`"，也就是"界面不再只能渲染内置样本"。
//!
//! ## 为什么另有一条集成判据（而不是只放在 `src/open.rs` 的单元判据里）
//!
//! 单元判据能用到 crate 的私有项；这一条**只能用公开 API**（`yeban_app::open::*`
//! 与 `yeban_model::container::write_project_container`）。它回答的问题是
//! "这个入口对 crate 外部的调用者（未来的 CLI / 会话层）真的可用吗" ——
//! 这正是任务书要的"可被调用的入口"。
//!
//! ## 本机 vs CI
//!
//! 本文件不含 Slint 代码，依赖也只用到 `yeban-model`（`yeban-app` 的普通依赖），
//! 因此它的逻辑与 `src/open.rs` 的判据一样可以在本机用 `rustc --edition 2024 --test`
//! 真跑（见 `docs/ledger/app-completion-notes.md` §4）。整包的 `cargo test -p yeban-app`
//! 仍然需要 CI（它要编译 Slint），本机不编译。

use std::collections::BTreeMap;

use yeban_app::open::{
    MAX_PROJECT_FILE_BYTES, OpenError, ProjectOpenOptions, open_project_archive_file,
    open_project_file,
};
use yeban_model::container::{ContainerError, ContainerLimits, write_project_container};
use yeban_model::ids::AssetHash;
use yeban_model::samples::filled_project;

/// 判据 23: 磁盘上的真 `.yeban` 能被公开入口打开，工程 / 历史 / 资产全保真。
#[test]
fn public_entry_opens_a_real_container_from_disk() {
    let project = filled_project();
    let asset = b"yeban-integration-asset".to_vec();
    let hash = AssetHash::of_bytes(&asset);
    let mut assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    assets.insert(hash.clone(), asset.clone());
    let bytes =
        write_project_container(&project, b"integration-history", &assets).expect("写出真容器");

    let dir = std::env::temp_dir().join(format!("yeban-open-entry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let path = dir.join("integration.yeban");
    std::fs::write(&path, &bytes).expect("写临时容器");

    // (1) 任务书点名的入口：只要工程。
    let opened = open_project_file(&path).expect("公开入口必须能打开真容器");
    assert_eq!(opened, project);
    assert_eq!(opened.title, "Yeban Model Core Sample");
    assert_eq!(opened.tracks.len(), project.tracks.len());

    // (2) 全保真入口：历史与资产不得丢失。
    let archive = open_project_archive_file(&path, &ProjectOpenOptions::default())
        .expect("全保真入口必须能打开真容器");
    assert_eq!(archive.project, project);
    assert_eq!(archive.history_dag, b"integration-history");
    assert_eq!(archive.assets.len(), 1);
    assert_eq!(archive.assets[0].0, hash);
    assert_eq!(archive.assets[0].1, asset);

    // (3) 缺省上限就是文档常量（回退口径不得漂移）。
    assert_eq!(
        ProjectOpenOptions::default().max_file_bytes,
        MAX_PROJECT_FILE_BYTES
    );
    assert_eq!(MAX_PROJECT_FILE_BYTES, 1 << 32);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 24: 公开入口对**坏输入**必须报错，绝不退化成空工程 / 默认工程。
#[test]
fn public_entry_refuses_broken_and_oversized_files() {
    let project = filled_project();
    let bytes = write_project_container(&project, b"dag", &BTreeMap::new()).expect("写出容器");

    let dir = std::env::temp_dir().join(format!("yeban-open-broken-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");

    // (a) 截断的容器：报容器层的错，而不是"打开成空工程"。
    let truncated = dir.join("truncated.yeban");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).expect("写截断容器");
    match open_project_file(&truncated) {
        Err(OpenError::Container(_)) => {}
        other => panic!("截断容器必须报容器错误, 实测 {other:?}"),
    }
    assert_ne!(
        open_project_file(&truncated).ok(),
        Some(yeban_model::project::YebanProjectV1::default()),
        "错误不得退化成空工程"
    );

    // (b) 超过注入上限的文件：在 `read` 之前就被拦下。
    let strict = ProjectOpenOptions {
        max_file_bytes: 4,
        ..ProjectOpenOptions::default()
    };
    match open_project_archive_file(&truncated, &strict) {
        Err(OpenError::FileTooLarge { len, max, .. }) => {
            assert!(len > 4);
            assert_eq!(max, 4);
        }
        other => panic!("超限文件必须报 FileTooLarge, 实测 {other:?}"),
    }

    // (c) 容器层的部署上限（炸弹防御）也能注入到**文件**入口。
    let bomb = ProjectOpenOptions {
        limits: ContainerLimits {
            max_entry_bytes: 1,
            ..ContainerLimits::default()
        },
        ..ProjectOpenOptions::default()
    };
    let full = dir.join("full.yeban");
    std::fs::write(&full, &bytes).expect("写完整容器");
    assert!(matches!(
        open_project_archive_file(&full, &bomb),
        Err(OpenError::Container(ContainerError::EntryTooLarge { .. }))
    ));

    let _ = std::fs::remove_dir_all(&dir);
}
