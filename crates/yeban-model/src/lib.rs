//! # yeban-model — 夜半权威工程数据模型
//!
//! 本 crate 是夜半 (Yeban) 的**唯一权威状态源**：960 PPQ 整数时钟、`EntityId` 身份体系、
//! 全部 `BTreeMap` 确定性集合的持久化 AST、领域操作日志 (Ops Log) 与 Commit DAG。
//!
//! ## 规范来源 (Normative sources of truth)
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2 (核心数据模型 `MODEL-*`)、§6 (Ops Log / Commit DAG)
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5 (`.yeban` 容器, 原子保存, `.yeban.lock`)
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M1-001 … ROAD-M1-006
//! - `AGENTS.md` §2 红线 3/4/8、§3 DoD
//!
//! ## 硬性约束 (Hard constraints, 违反即一票否决)
//!
//! 1. **零 GUI 依赖** [ARCH-TOP-003]: 本 crate 及其下游 `yeban-dsp` / `yeban-theory` /
//!    `yeban-render` / `yeban-engine` / `yeban-sfz` / `yeban-decode` 永不引入
//!    slint / winit / OpenGL / Qt 依赖。由 `scripts/guards/policy_check.py` 机械校验。
//! 2. **确定性状态** [MODEL-AST-003]: 持久化 AST 实体集合严禁 `HashMap` / `HashSet`，
//!    一律 `BTreeMap`，以保障跨进程、跨重启的迭代顺序一致。
//! 3. **内存安全** [AGENTS.md §2 红线 8]: 本 crate `#![forbid(unsafe_code)]`。
//! 4. **三层状态物理隔离** [MODEL-ISO-001]: 持久化文档
//!    ([`project::YebanProjectV1`])、会话运行态 ([`session::SessionRuntimeState`])、
//!    本机配置 ([`local_config::LocalMachineConfig`]) 三者**必须**放在三个模块里、
//!    永不互相嵌套。会话态刻意**不实现** `Serialize`（"严禁持久化存入 Commit"是
//!    类型层事实，不是注释）；本机配置落在工程之外（`~/.yeban/config.json`，`0600`），
//!    凭据只以 [`local_config::SecretRef`] 指针形式出现。常驻判据见
//!    `crates/yeban-model/tests/model_isolation.rs`。
//!
//! ## 版本
//!
//! 文案档 schema 版本从 `1` 开始：原 Groove v3 编号体系已在
//! `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` 中重基设为
//! `v0.0.1` 起步 / `v1.0.0` 首发，且历史上从未发布过任何 `schema_version = 2|3` 文档。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod automation;
pub mod commit;
pub mod container;
pub mod error;
pub mod ids;
pub mod local_config;
pub mod music;
pub mod note_plan;
pub mod ops;
pub mod project;
pub mod samples;
pub mod session;

pub use automation::{NOMINAL_GAIN_MAX_DB, NOMINAL_GAIN_MIN_DB};
pub use commit::{
    ANONYMOUS_BRANCH_PREFIX, BranchHead, Commit, CommitDraft, CommitGraph, CommitGraphError,
    HistoryDagError, SNAPSHOT_INTERVAL, UndoCursor, decode_history_dag, encode_history_dag,
    snapshot_due_at_depth,
};
pub use container::{
    ContainerArchive, ContainerEntry, ContainerError, ContainerLimits, ProjectArchive,
};
pub use error::ModelError;
pub use ids::{AssetHash, AssetHasher, ContentHash, EntityId, PPQ, ULID_TEXT_LEN};
pub use local_config::{
    AudioPortBinding, DEFAULT_SECRET_BACKEND, EditorRole, ExternalEditor, KNOWN_SECRET_BACKENDS,
    LOCAL_CONFIG_DIR_NAME, LOCAL_CONFIG_FILE_NAME, LOCAL_CONFIG_VERSION, LocalConfigError,
    LocalMachineConfig, MAX_SECRET_REF_LEN, SecretMaterial, SecretRef, SecretStore,
    SecretStoreError, UnavailableSecretStore, load_default, secret_store_for,
};
pub use music::{CurveType, MidiNote, SlideConfig};
pub use ops::{Op, OpOrigin, StampedOp};
pub use project::{
    AssetMetadata, AutomationLane, AutomationPoint, AutomationTarget, AutomationUnit,
    AutomationValueDomain, AutomationWriteMode, BitDepth, BlockSize, ClipContent, ClipPlacement,
    ClipPoolEntry, DeviceDefinition, DeviceKind, LaunchQuantization, LoopConfig,
    MIN_READER_VERSION, MacroMapping, MacroParameter, MediaKind, PanLaw, ParameterValue,
    ProjectAudioConfig, ProjectMetadata, READER_SCHEMA_VERSION, RoutingEdge, RoutingGraph,
    RoutingKind, SCHEMA_VERSION, SampleRate, SceneV3, SectionV3, TimeSignature, TrackKind, TrackV3,
    TransportConfig, YebanProjectV1,
};
pub use samples::{SampleExportError, export_all, export_to_default_dir};
pub use session::{
    PluginProcess, SessionRuntimeState, SessionStateError, TaskId, TaskKind, TaskProgress, WindowId,
};
