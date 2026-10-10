//! `MODEL-ISO-001` —— **三层状态物理隔离**的常驻判据。
//!
//! 规范来源：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2 `[MODEL-ISO-001]`：
//!
//! 1. `ProjectDocument`（持久化文档层）—— 本 crate 的实现叫 [`YebanProjectV1`]；
//! 2. `SessionRuntimeState`（挥发性运行时状态层）—— "**严禁持久化存入 Commit**"；
//! 3. `LocalMachineConfig`（本机配置层）—— 凭据只存**引用指针**，落在工程之外。
//!
//! 本线接手时第 2、3 层**根本不存在**（`grep -rn "struct SessionRuntimeState" crates/`
//! 与 `grep -rn "struct LocalMachineConfig" crates/` 均无命中），于是：
//! 撤销游标 [ADR-0001 `D45`] 无处安放、走带的 `playhead` / `is_playing` 没有归宿、
//! 本机声卡绑定 / 外部编辑器路径 / 云端 Token 连类型都没有。
//!
//! 本文件把那三件事变成**机械判据**：不靠注释声明，靠**键集合 / 逐字节 / 类型层 / 权限
//! 位 / 探针对照**五类证据。
//!
//! ## 判据清单（16 条 + 1 条 `#[ignore]`）
//!
//! | # | 判据 | 钉住什么 |
//! | :-- | :--- | :--- |
//! | ① | [`three_layers_live_in_three_distinct_modules`] | 三层是三个模块，不是三个注释 |
//! | ② | [`project_json_top_level_keys_are_frozen`] | 工程 `project.json` 的键集合一位不变 |
//! | ③ | [`project_json_recursive_key_paths_are_frozen`] | 递归键路径集合不变（嵌套字段也拦） |
//! | ④ | [`project_json_byte_samples_are_frozen`] | 逐字节样本 SHA-256 不变 |
//! | ⑤ | [`session_mutation_cannot_move_project_bytes`] | 改播放头 / is_playing ⇒ 工程字节一位不变 |
//! | ⑥ | [`session_state_has_no_serde_surface`] | 会话态在**源码层**没有 serde 面 + 扫描器有牙 |
//! | ⑦ | [`local_config_is_absent_from_the_project_container`] | 本机配置（及引用名）不进容器 |
//! | ⑧ | [`local_config_stays_outside_after_container_round_trip`] | 容器往返后配置仍在本机路径 |
//! | ⑨ | [`local_config_file_is_0600`] | 落盘权限 `0600`（含"已存在的宽权限文件被收紧"） |
//! | ⑩ | [`local_config_round_trip_is_field_exact_and_deterministic`] | 往返逐字段相等 + 两次写字节相同 |
//! | ⑪ | [`secret_material_never_reaches_disk`] | 密钥本体不入盘；文件里只有引用名 |
//! | ⑫ | [`unknown_secret_backend_is_an_explicit_error`] | 未知 / 未接入后端 ⇒ 明确错误，不 panic |
//! | ⑬ | [`playhead_is_integer_ticks_at_960_ppq`] | 播放头是整数 tick，960 PPQ 语义 |
//! | ⑭ | [`window_and_pid_sets_are_deterministic`] | `BTreeSet` / `BTreeMap` 确定性顺序 |
//! | ⑮ | [`crate_keeps_forbid_unsafe_and_zero_gui_deps`] | `forbid(unsafe_code)` + 零 GUI 依赖 |
//! | ⑯ | [`editor_path_absoluteness_follows_platform_semantics`] | 编辑器路径绝对性是**平台语义**（Unix / Windows 分开钉） |
//! | ⑰ | [`print_frozen_isolation_table`]（`#[ignore]`） | 重新冻结常量时的取证入口 |
//!
//! 另外两条"判据自身有牙"的反空洞证明写在 ⑥ / ③ 里（合成输入 ⇒ 扫描器 / 哈希必须变红）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::str::FromStr;
use yeban_model::container::{
    ContainerLimits, HISTORY_DAG_NAME, PROJECT_JSON_NAME, read_container, read_project_container,
    write_project_container, write_project_container_borrowed,
};
use yeban_model::local_config::{
    AudioPortBinding, DEFAULT_SECRET_BACKEND, EditorRole, ExternalEditor, KNOWN_SECRET_BACKENDS,
    LOCAL_CONFIG_VERSION, LocalMachineConfig, MAX_SECRET_REF_LEN, SecretMaterial, SecretRef,
    SecretStore, SecretStoreError, UnavailableSecretStore, secret_store_for,
};

use yeban_model::commit::{CommitDraft, CommitGraph, encode_history_dag};
use yeban_model::ids::AssetHash;
use yeban_model::ops::{Op, StampedOp};
use yeban_model::samples::{
    default_project, default_stamped_op, filled_project, filled_stamped_op,
};
use yeban_model::session::{
    PluginProcess, SessionRuntimeState, SessionStateError, TaskId, TaskKind, TaskProgress, WindowId,
};
use yeban_model::{EntityId, PPQ, YebanProjectV1};

// ---------------------------------------------------------------------------
// 冻结常量（`print_frozen_isolation_table` 是唯一的重新冻结入口）
// ---------------------------------------------------------------------------

/// `project.default.json` 与 `project.filled.json` 的**逐字节 SHA-256**。
///
/// 只要 `YebanProjectV1` 的序列化输出变了一个字节（加字段、改字段名、改顺序），
/// 判据 ④ 立刻变红。
const FROZEN_PROJECT_JSON_SHA256: [(&str, &str); 2] = [
    (
        "project.default.json",
        "6bd76fcba2ad050dad2f3d5d8e9b92399d23636a2d2fd79911f42f29f61430f0",
    ),
    (
        "project.filled.json",
        "0218fa7d620d48e23363f3a49621d8af647f6abb10f8423feeaebafce057265b",
    ),
];

/// `YebanProjectV1` 的**顶层**键集合（`serde_json::to_value` 的键，已排序）。
const FROZEN_PROJECT_TOP_LEVEL_KEYS: [&str; 19] = [
    "assets",
    "audio_config",
    "author",
    "bpm",
    "clip_pool",
    "id",
    "master_bus_track_id",
    "metadata",
    "min_reader_version",
    "rng_seed",
    "routing_graph",
    "scenes",
    "schema_version",
    "sections",
    "time_signature",
    "title",
    "tracks",
    "transport",
    "writer_version",
];

/// 工程 JSON 的**递归键路径集合**（`a.b.*` 形式，已排序）的 SHA-256。
const FROZEN_PROJECT_KEY_PATH_SHA256: &str =
    "067b6c04205ba26cc1ff049c8bccd03938841b7e81736194ac18b4413febdfbc";

/// 上面那个集合的元素个数（防止"哈希对了但集合空了"这类退化）。
const FROZEN_PROJECT_KEY_PATH_COUNT: usize = 223;

/// 本机配置 JSON 的键集合（冻结：加字段即红）。
const FROZEN_LOCAL_CONFIG_KEYS: [&str; 4] = [
    "audio_binding",
    "cloud_tokens",
    "external_editors",
    "version",
];

/// 判据 ⑪ 用的**显式假密钥**探针（真实密钥永不出现 —— 这个字符串就是"真实密钥"的替身）。
const FAKE_SECRET_MATERIAL: &str = "sk-PROBE-5f4d3c2b1a0F9E8D7C6B5A4f3e2d1c0b-DO-NOT-PERSIST";

/// 波形编辑器的**平台正确**绝对路径夹具。
///
/// 为什么不能用一套路径打天下：`Path::is_absolute()` 的语义是**平台相关**的 ——
/// 在 Windows 上 `/Applications/...` 只是"有根"（rooted），**不是绝对路径**
/// （见 `std::path::absolute` 的文档与 `Prefix` 组件语义）。
///
/// 夹具若写死 macOS 路径，判据会在 Windows 上因为**夹具自己不是本机合法路径**
/// 而红 —— 那不是被测对象的缺陷，而是夹具把"我这台机器"当成了规范。
/// 实测事故（CI run 37268168161，windows 腿）：三条 `local_config*` 判据红，
/// 原文 `写本机配置: EditorPathNotAbsolute { path: "/Applications/Audacity.app/..." }`。
///
/// 修法是**让夹具平台正确**（而不是放宽断言）：生产侧"非绝对路径必须拒绝"的
/// 校验一字未改，并且另有一条判据把这条平台语义**显式断言**出来。
#[cfg(windows)]
const WAVEFORM_EDITOR_PATH: &str = r"C:\Program Files\Audacity\Audacity.exe";
#[cfg(not(windows))]
const WAVEFORM_EDITOR_PATH: &str = "/Applications/Audacity.app/Contents/MacOS/Audacity";

/// 乐谱编辑器的**平台正确**绝对路径夹具（理由同 [`WAVEFORM_EDITOR_PATH`]）。
#[cfg(windows)]
const SCORE_EDITOR_PATH: &str = r"C:\Program Files\MuseScore 4\bin\MuseScore4.exe";
#[cfg(not(windows))]
const SCORE_EDITOR_PATH: &str = "/Applications/MuseScore 4.app/Contents/MacOS/mscore";

/// 平台正确的**非绝对**路径：判据用它证明"相对路径必须被拒"。
const RELATIVE_EDITOR_PATH: &str = "relative/editor";

/// Windows 专属的"**有根但不是绝对**"路径：只有根目录、没有盘符前缀。
///
/// 它只在 `#[cfg(windows)]` 下存在：在 Unix 上 `/Program Files/...` 是**绝对**路径，
/// 同名常量会变成谎言（也会变成 `dead_code`）。这正是本线要显式钉住的平台语义。
#[cfg(windows)]
const ROOTED_BUT_NOT_ABSOLUTE_EDITOR_PATH: &str = r"\Program Files\Audacity\Audacity.exe";

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // `write!` 到 `String` 不会失败。
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn project_bytes(doc: &YebanProjectV1) -> Vec<u8> {
    serde_json::to_vec(doc).expect("工程必须可序列化")
}

fn project_value(doc: &YebanProjectV1) -> Value {
    serde_json::to_value(doc).expect("工程必须可序列化为 Value")
}

/// 收集**递归键路径**：对象键用 `.` 连接，数组元素折叠成 `[]`（集合语义）。
fn key_paths(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                out.insert(path.clone());
                key_paths(child, &path, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                let path = format!("{prefix}[]");
                out.insert(path.clone());
                key_paths(item, &path, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn key_paths_of(value: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    key_paths(value, "", &mut out);
    out
}

fn key_paths_digest(paths: &BTreeSet<String>) -> (String, usize) {
    let joined = paths.iter().cloned().collect::<Vec<_>>().join("\n");
    (sha256_hex(joined.as_bytes()), paths.len())
}

/// **剥掉注释行**后的源码 —— 判据 ⑥ / ⑮ 的扫描对象。
///
/// 为什么必须剥注释：会话态的"严禁持久化"证据**以 `compile_fail` doc-test 的形式**
/// 写在注释里，那里面必然出现 `Serialize` 字样。扫描器只审**真代码**，
/// 注释里的编译期证据由 `cargo test` 的 doctest 阶段独立执行。
///
/// 保守性：只剥"整行注释"，行尾注释（`let x = 1; // serde`）**照旧参与扫描**
/// ⇒ 扫描器只会更严，不会更松。
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 真代码里是否出现某个记号。
fn code_has(src: &str, needle: &str) -> bool {
    code_only(src).contains(needle)
}

/// 顶层键集合（已排序）。
fn top_level_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .expect("工程 JSON 必须是对象")
        .keys()
        .cloned()
        .collect()
}

/// 递归收集**全部键名**（值层面，去掉路径前缀）。
fn all_key_names(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                out.insert(key.clone());
                all_key_names(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                all_key_names(item, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// 自建临时目录（不引入 `tempfile`：本线**零新增依赖**，也不许改根 `Cargo.lock`）。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yeban-model-iso-{}-{tag}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("建临时目录");
        Self { path }
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn populated_config() -> LocalMachineConfig {
    let mut config = LocalMachineConfig {
        version: LOCAL_CONFIG_VERSION,
        audio_binding: AudioPortBinding {
            input_device: Some("Focusrite Scarlett 2i2 USB".to_owned()),
            output_device: Some("MacBook Pro Speakers".to_owned()),
            input_ports: BTreeMap::from([(0_u32, "Input 1".to_owned()), (1, "Input 2".to_owned())]),
            output_ports: BTreeMap::from([
                (0_u32, "Output 1".to_owned()),
                (1, "Output 2".to_owned()),
            ]),
        },
        external_editors: BTreeMap::new(),
        cloud_tokens: BTreeMap::new(),
    };
    config.external_editors.insert(
        EditorRole::Waveform,
        ExternalEditor {
            absolute_path: PathBuf::from(WAVEFORM_EDITOR_PATH),
            arguments: vec!["--open".to_owned()],
        },
    );
    config.external_editors.insert(
        EditorRole::Score,
        ExternalEditor {
            absolute_path: PathBuf::from(SCORE_EDITOR_PATH),
            arguments: Vec::new(),
        },
    );
    config.cloud_tokens.insert(
        "anthropic".to_owned(),
        SecretRef::new("yeban/cloud/anthropic").expect("合法条目名"),
    );
    config.cloud_tokens.insert(
        "elevenlabs".to_owned(),
        SecretRef::new("yeban/cloud/elevenlabs").expect("合法条目名"),
    );
    config
}

/// 判据 ⑪ 用的内存密钥库（**只**存在于测试进程内，绝不落盘）。
#[derive(Default)]
struct ProbeSecretStore {
    entries: std::cell::RefCell<BTreeMap<String, Vec<u8>>>,
}

impl SecretStore for ProbeSecretStore {
    fn backend_name(&self) -> &'static str {
        "test-memory"
    }

    fn put(
        &self,
        reference: &SecretRef,
        material: &SecretMaterial,
    ) -> Result<(), SecretStoreError> {
        self.entries
            .borrow_mut()
            .insert(reference.entry().to_owned(), material.expose().to_vec());
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<SecretMaterial, SecretStoreError> {
        self.entries
            .borrow()
            .get(reference.entry())
            .map(|bytes| SecretMaterial::from_bytes(bytes.clone()))
            .ok_or_else(|| SecretStoreError::NotFound {
                backend: "test-memory",
                entry: reference.entry().to_owned(),
            })
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), SecretStoreError> {
        self.entries.borrow_mut().remove(reference.entry());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ① 三层是三个模块
// ---------------------------------------------------------------------------

#[test]
fn three_layers_live_in_three_distinct_modules() {
    let project_layer = std::any::type_name::<YebanProjectV1>();
    let session_layer = std::any::type_name::<SessionRuntimeState>();
    let config_layer = std::any::type_name::<LocalMachineConfig>();

    let mut modules: Vec<&str> = vec![project_layer, session_layer, config_layer]
        .into_iter()
        .map(|name| name.rsplit_once("::").map_or(name, |(module, _)| module))
        .collect();
    modules.sort_unstable();
    modules.dedup();
    assert_eq!(modules.len(), 3, "三层必须是三个不同的模块: {modules:?}");

    assert!(project_layer.starts_with("yeban_model::project::"));
    assert!(session_layer.starts_with("yeban_model::session::"));
    assert!(config_layer.starts_with("yeban_model::local_config::"));
}

// ---------------------------------------------------------------------------
// ②③④ 持久化层：键集合 + 递归键路径 + 逐字节样本（都在 `YebanProjectV1` 上）
// ---------------------------------------------------------------------------

#[test]
fn project_json_top_level_keys_are_frozen() {
    for doc in [default_project(), filled_project()] {
        let keys = top_level_keys(&project_value(&doc));
        assert_eq!(
            keys, FROZEN_PROJECT_TOP_LEVEL_KEYS,
            "工程顶层键集合变了 —— 只要新增的键里出现 playhead / is_playing / session / \
             本机配置字段名，就是 MODEL-ISO-001 违规"
        );
    }
}

#[test]
fn project_json_recursive_key_paths_are_frozen() {
    let mut union = BTreeSet::new();
    for doc in [default_project(), filled_project()] {
        union.extend(key_paths_of(&project_value(&doc)));
    }
    let (digest, count) = key_paths_digest(&union);
    assert_eq!(
        count, FROZEN_PROJECT_KEY_PATH_COUNT,
        "递归键路径数量变了 ⇒ 工程里多/少了字段（嵌套字段也拦得住）"
    );
    assert_eq!(
        digest, FROZEN_PROJECT_KEY_PATH_SHA256,
        "递归键路径集合变了 ⇒ 工程序列化结构漂移"
    );

    // ---- 判据自身有牙（反空洞）：合成"往工程里加一个会话字段"必须让摘要变红 ----
    let mut tampered = project_value(&filled_project());
    tampered
        .as_object_mut()
        .expect("对象")
        .insert("session_playhead_ticks".to_owned(), Value::from(0_u64));
    let (tampered_digest, tampered_count) = key_paths_digest(&key_paths_of(&tampered));
    assert_ne!(
        tampered_digest, FROZEN_PROJECT_KEY_PATH_SHA256,
        "注入一个会话字段后摘要必须变红 —— 否则这条判据是空转的"
    );
    assert_eq!(tampered_count, FROZEN_PROJECT_KEY_PATH_COUNT + 1);
}

#[test]
fn project_json_byte_samples_are_frozen() {
    let samples = [
        ("project.default.json", default_project()),
        ("project.filled.json", filled_project()),
    ];
    for ((name, doc), (frozen_name, frozen_sha)) in samples.iter().zip(FROZEN_PROJECT_JSON_SHA256) {
        assert_eq!(*name, frozen_name);
        let bytes = project_bytes(doc);
        // `to_string` 与 `to_vec` 必须一致（另一条常驻冗余：防止"两套序列化入口"）。
        assert_eq!(
            bytes,
            serde_json::to_string(doc).expect("序列化").into_bytes(),
            "{name}: to_vec 与 to_string 必须逐字节一致"
        );
        assert_eq!(
            sha256_hex(&bytes),
            frozen_sha,
            "{name}: 逐字节样本变了。若 diff 只来自 workspace 版本号（writer_version），\
             那是合法版本升级，用 `cargo test -p yeban-model --test model_isolation -- \
             --ignored --nocapture` 重新冻结；若 diff 里出现任何新键，那是 MODEL-ISO-001 违规"
        );
    }

    // ---- 判据自身有牙：改一个**工程**字段必须让字节变红 ----
    let mut tampered = filled_project();
    tampered.title.push('!');
    assert_ne!(
        sha256_hex(&project_bytes(&tampered)),
        FROZEN_PROJECT_JSON_SHA256[1].1,
        "改工程标题后摘要必须变红 —— 否则字节判据是空转的"
    );
}

// ---------------------------------------------------------------------------
// ⑤ 会话态改不动工程字节
// ---------------------------------------------------------------------------

#[test]
fn session_mutation_cannot_move_project_bytes() {
    let project = filled_project();
    let before = project_bytes(&project);
    let before_json = serde_json::to_string(&project).expect("序列化");

    let mut session = SessionRuntimeState::default();
    assert!(!session.is_playing());
    session.play();
    session.seek_ticks(960 * 7 + 3);
    session.advance_ticks(PPQ);
    session.set_playing(false);
    session.seek_ticks(0);
    session.open_window(WindowId::Mixer);
    session.open_window(WindowId::PluginEditor(EntityId::default()));
    session.track_plugin_process(
        EntityId::default(),
        PluginProcess {
            pid: 4242,
            sandboxed: true,
        },
    );
    session.set_task(
        TaskId::new(1),
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 3,
            total_units: 10,
            cancellable: true,
            label: "Bounce master".to_owned(),
        },
    );
    session.undo_cursor.skip = 5;
    session.validate().expect("会话态自洽");
    assert!(session.is_window_open(&WindowId::Mixer));
    assert_eq!(session.plugin_pids().len(), 1);

    // 工程侧：字节一位不变（同一个对象，重新序列化）。
    assert_eq!(before, project_bytes(&project), "会话操作不得改动工程字节");
    assert_eq!(
        before_json,
        serde_json::to_string(&project).expect("序列化"),
        "会话操作不得改动工程 JSON 文本"
    );
    // 反面对照：工程自己的字段确实能改字节（证明上面那条不是"序列化坏了"）。
    let mut changed = project.clone();
    changed.bpm += 1.0;
    assert_ne!(before, project_bytes(&changed));
}

// ---------------------------------------------------------------------------
// ⑥ 会话态在源码层没有 serde 面（类型层的 `compile_fail` 证据在 doc-test 里）
// ---------------------------------------------------------------------------

#[test]
fn session_state_has_no_serde_surface() {
    const SESSION_SRC: &str = include_str!("../src/session.rs");
    const PROJECT_SRC: &str = include_str!("../src/project.rs");
    const LOCAL_SRC: &str = include_str!("../src/local_config.rs");

    // ---- 会话态：真代码里不许出现任何持久化机制 ----
    for needle in ["serde", "Serialize", "Deserialize"] {
        assert!(
            !code_has(SESSION_SRC, needle),
            "session.rs 的真代码里出现了 `{needle}` ⇒ 会话态获得了持久化通道 [MODEL-ISO-001 第 2 层]"
        );
    }
    // 红线 4：即使是非持久化层也不许 HashMap/HashSet（G01 对整个 src 生效）。
    for needle in ["HashMap", "HashSet"] {
        assert!(
            !code_has(SESSION_SRC, needle),
            "session.rs 出现了 `{needle}`（红线 4 / MODEL-AST-003）"
        );
    }

    // ---- 反向对照：同一个扫描器在别的文件上**必须**能命中 ----
    // 否则"没命中"可能只是因为扫描器坏了 / include_str! 读空了。
    assert!(
        code_has(PROJECT_SRC, "Serialize"),
        "对照失败：project.rs 必然含 Serialize ⇒ 扫描器或 include_str! 有问题"
    );
    assert!(
        code_has(PROJECT_SRC, "Deserialize"),
        "对照失败：project.rs 必然含 Deserialize"
    );
    assert!(
        code_has(LOCAL_SRC, "Serialize"),
        "对照失败：local_config.rs 必须可持久化（含 Serialize）"
    );

    // ---- 扫描器自身有牙：合成输入逐个命中 ----
    assert!(code_has("#[derive(Serialize)]\nstruct X;", "Serialize"));
    assert!(code_has("use serde::Serialize;", "serde"));
    assert!(code_has("struct X { m: HashMap<u8, u8> }", "HashMap"));
    assert!(code_has("struct X { m: HashSet<u8> }", "HashSet"));
    assert!(!code_has(
        "// 注释里的 Serialize 不算\nstruct X;",
        "Serialize"
    ));
}

/// 红线 4 / [MODEL-AST-003]：**整个 `src/**`** 的真代码里不许出现 `HashMap` / `HashSet`。
///
/// 为什么需要：既有 `session_state_has_no_serde_surface` 只对 `session.rs` 做这条扫描
/// （它的注释写的是"G01 对整个 src 生效"，但 G01 是 `scripts/guards/policy_check.py`，
/// 在**本 crate 的判据面之外**）。实测（本次注入）：往 `src/project.rs` 里加
/// `use std::collections::HashMap;` 或 `use std::collections::HashSet;` 时，
/// 本 crate 的**全部 303 条判据保持全绿** —— 持久化 AST 的"确定性集合"红线在
/// 本 crate 内没有任何判据守着。本判据递归遍历 `src/**` 的每一个 `*.rs`，
/// 逐文件剥掉整行注释后扫描这两个记号（`lib.rs` 只在文档注释里提到它们）。
#[test]
fn no_hash_containers_anywhere_in_src() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", dir.display()))
            .map(|entry| entry.expect("目录项").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }

    let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src_root, &mut files);
    // 反空洞：遍历必须真的走完 `src/**`（当前 16 个文件），否则"没命中"是空转。
    assert!(
        files.len() >= 16,
        "src/** 至少 16 个 *.rs, 实际 {}",
        files.len()
    );

    for path in &files {
        let text = std::fs::read_to_string(path).expect("读取源文件");
        for needle in ["HashMap", "HashSet"] {
            assert!(
                !code_has(&text, needle),
                "{} 的真代码里出现了 `{needle}`（红线 4 / MODEL-AST-003: \
                 持久化 AST 实体集合必须用 BTreeMap 以保证迭代顺序确定）",
                path.display()
            );
        }
    }

    // 扫描器自身有牙：合成输入逐个命中，注释里的记号不算。
    assert!(code_has("use std::collections::HashMap;", "HashMap"));
    assert!(code_has("struct X { m: HashSet<u8> }", "HashSet"));
    assert!(!code_has("// HashMap 只出现在注释里\n", "HashMap"));
}

// ---------------------------------------------------------------------------
// ⑦⑧ 本机配置不进工程容器
// ---------------------------------------------------------------------------

fn container_of(doc: &YebanProjectV1) -> Vec<u8> {
    write_project_container(doc, br#"{"graph":"dag-bytes"}"#, &BTreeMap::new())
        .expect("写容器必须成功")
}

#[test]
fn local_config_is_absent_from_the_project_container() {
    let config = populated_config();
    let config_keys: BTreeSet<String> = FROZEN_LOCAL_CONFIG_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();

    // 本机配置的键集合本身被冻结：加字段即红。
    let config_json: Value =
        serde_json::from_slice(&config.to_json().expect("配置可序列化")).expect("配置 JSON 合法");
    assert_eq!(
        top_level_keys(&config_json)
            .into_iter()
            .collect::<BTreeSet<_>>(),
        config_keys,
        "本机配置键集合变了（新增字段必须同时更新判据与 notes）"
    );

    let bytes = container_of(&filled_project());
    let archive = read_container(&bytes, &ContainerLimits::default()).expect("读容器");
    let project_json = archive.get(PROJECT_JSON_NAME).expect("project.json");
    let value: Value = serde_json::from_slice(&project_json.data).expect("工程 JSON");

    // 顶层：容器里的工程键集合与冻结集合完全相同。
    assert_eq!(top_level_keys(&value), FROZEN_PROJECT_TOP_LEVEL_KEYS);

    // 递归：容器里**任何一层**都不许出现本机配置的字段名。
    let mut names = BTreeSet::new();
    all_key_names(&value, &mut names);
    for forbidden in &config_keys {
        assert!(
            !names.contains(forbidden),
            "工程容器里出现了本机配置字段 `{forbidden}` [MODEL-ISO-001 第 3 层]"
        );
    }
    for extra in ["cloud_tokens", "secret", "token", "keychain"] {
        assert!(
            !names.contains(extra),
            "工程容器里出现了凭据相关字段 `{extra}`"
        );
    }

    // 字节层：容器原始字节里不许出现配置里的引用名 / 设备名 / 假密钥。
    for leak in [
        b"yeban/cloud/anthropic".as_slice(),
        b"yeban/cloud/elevenlabs".as_slice(),
        b"Focusrite Scarlett".as_slice(),
        b"Audacity".as_slice(),
        WAVEFORM_EDITOR_PATH.as_bytes(),
        SCORE_EDITOR_PATH.as_bytes(),
        FAKE_SECRET_MATERIAL.as_bytes(),
    ] {
        assert!(
            !contains_subslice(&bytes, leak),
            "工程容器字节里出现了本机配置内容: {:?}",
            String::from_utf8_lossy(leak)
        );
    }
    assert_eq!(
        archive.names().collect::<Vec<_>>(),
        vec![PROJECT_JSON_NAME, HISTORY_DAG_NAME],
        "容器条目集合必须恰好是 project.json + history.dag（配置不得成为第三个条目）"
    );

    // 反面对照：把配置内容**真的**塞进容器后，上面那条探针必须能抓到（证明它有牙）。
    let leaky = yeban_model::container::write_container(&[
        yeban_model::container::ContainerEntry::new(PROJECT_JSON_NAME, project_json.data.clone()),
        yeban_model::container::ContainerEntry::new(HISTORY_DAG_NAME, b"dag".to_vec()),
        yeban_model::container::ContainerEntry::new(
            "local/config.json",
            config.to_json().expect("配置"),
        ),
    ])
    .expect("写容器");
    assert!(contains_subslice(&leaky, b"yeban/cloud/anthropic"));
}

#[test]
fn local_config_stays_outside_after_container_round_trip() {
    let home = TempDir::new("home");
    let project_dir = TempDir::new("project");
    let config = populated_config();

    // 1) 配置落在**本机**路径（工程之外）。
    let config_path = LocalMachineConfig::path_for_home(&home.path);
    assert!(
        config_path.ends_with(Path::new(".yeban").join("config.json")),
        "本机配置必须落在 ~/.yeban/config.json，实际: {config_path:?}"
    );
    assert!(
        !config_path.starts_with(&project_dir.path),
        "本机配置不得落在工程目录内"
    );
    config.save_to(&config_path).expect("写本机配置");

    // 2) 工程容器写在工程目录里。
    let container_path = project_dir.join("song.yeban");
    std::fs::write(&container_path, container_of(&filled_project())).expect("写容器");
    let reopened = read_project_container(
        &std::fs::read(&container_path).expect("读容器"),
        &ContainerLimits::default(),
    )
    .expect("容器往返");
    assert_eq!(reopened.project.title, filled_project().title);

    // 3) 往返之后：工程目录里只有那个容器；配置仍在本机路径且逐字段相同。
    let mut listed: Vec<String> = std::fs::read_dir(&project_dir.path)
        .expect("列工程目录")
        .map(|entry| {
            entry
                .expect("目录项")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        vec!["song.yeban".to_owned()],
        "工程目录里不得多出配置文件"
    );
    assert_eq!(
        LocalMachineConfig::load_from(&config_path).expect("读回配置"),
        config,
        "容器往返后本机配置必须逐字段不变"
    );

    // 4) 默认路径也必须在工程之外。
    if let Ok(default_path) = LocalMachineConfig::default_path() {
        assert!(default_path.is_absolute());
        assert!(default_path.ends_with(Path::new(".yeban").join("config.json")));
        assert!(!default_path.starts_with(env!("CARGO_MANIFEST_DIR")));
    }
}

// ---------------------------------------------------------------------------
// ⑨ 本机配置文件权限 0600
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn local_config_file_is_0600() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = TempDir::new("perm");
    let config_path = LocalMachineConfig::path_for_home(&home.path);
    let config = populated_config();
    config.save_to(&config_path).expect("写本机配置");

    let mode = std::fs::metadata(&config_path)
        .expect("stat 配置")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode,
        yeban_model::local_config::LOCAL_CONFIG_FILE_MODE,
        "本机配置文件权限必须是 0600（实际 {mode:o}）"
    );
    let dir_mode = std::fs::metadata(config_path.parent().expect("父目录"))
        .expect("stat 目录")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        dir_mode, 0o700,
        "本机配置目录权限必须是 0700（实际 {dir_mode:o}）"
    );

    // 关键边界：`OpenOptions::mode` 对**已存在**的文件是空转 ⇒ 必须显式收紧。
    let loose = TempDir::new("loose");
    let loose_path = LocalMachineConfig::path_for_home(&loose.path);
    std::fs::create_dir_all(loose_path.parent().expect("父目录")).expect("建目录");
    std::fs::write(&loose_path, b"{}").expect("预置一个宽权限文件");
    std::fs::set_permissions(&loose_path, std::fs::Permissions::from_mode(0o644))
        .expect("放宽权限");
    config.save_to(&loose_path).expect("覆盖写");
    let repaired = std::fs::metadata(&loose_path)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        repaired,
        yeban_model::local_config::LOCAL_CONFIG_FILE_MODE,
        "已存在的 0644 文件在保存后必须被收紧到 0600（实际 {repaired:o}）"
    );

    // 对照：探针真的能分辨权限（0o644 ≠ 0o600）。
    assert_ne!(
        0o644 & 0o777,
        yeban_model::local_config::LOCAL_CONFIG_FILE_MODE
    );
}

/// 判据 ⑯：**外部编辑器路径的绝对性是平台语义**，显式断言而不是靠夹具"碰巧"过。
///
/// 为什么值得单独一条：CI run 37268168161 的 windows 腿红过三条 `local_config*` 判据，
/// 原文是 `写本机配置: EditorPathNotAbsolute { path: "/Applications/Audacity.app/..."
/// }` —— 生产侧的校验**完全正确**，错的是夹具把 macOS 路径当成了"本机合法路径"。
/// 这条判据把三个平台事实**分开钉死**：
///
/// | 平台 | 路径形态 | 期望 |
/// | :--- | :--- | :--- |
/// | 所有平台 | `relative/editor` | 拒绝（相对路径不是本机事实） |
/// | Unix | `/usr/bin/vi` | 接受（POSIX 绝对路径） |
/// | Windows | `C:\Program Files\...` | 接受（盘符前缀） |
/// | Windows | `\Program Files\...`（只有根、无盘符） | **拒绝**（`is_absolute()` 为假） |
#[test]
fn editor_path_absoluteness_follows_platform_semantics() {
    let mut config = LocalMachineConfig {
        version: LOCAL_CONFIG_VERSION,
        audio_binding: AudioPortBinding::default(),
        external_editors: BTreeMap::new(),
        cloud_tokens: BTreeMap::new(),
    };
    config.external_editors.insert(
        EditorRole::Waveform,
        ExternalEditor {
            absolute_path: PathBuf::from(WAVEFORM_EDITOR_PATH),
            arguments: Vec::new(),
        },
    );
    assert!(
        Path::new(WAVEFORM_EDITOR_PATH).is_absolute(),
        "夹具本身必须是本平台的绝对路径，否则判据测的是夹具: {WAVEFORM_EDITOR_PATH}"
    );
    config.validate().expect("平台正确的绝对路径必须被接受");

    // 相对路径：所有平台都必须拒绝。
    assert!(!Path::new(RELATIVE_EDITOR_PATH).is_absolute());
    let mut relative = config.clone();
    relative.external_editors.insert(
        EditorRole::Sample,
        ExternalEditor {
            absolute_path: PathBuf::from(RELATIVE_EDITOR_PATH),
            arguments: Vec::new(),
        },
    );
    assert!(
        matches!(
            relative.validate(),
            Err(yeban_model::local_config::LocalConfigError::EditorPathNotAbsolute { .. })
        ),
        "相对路径必须被拒绝: {:?}",
        relative.validate()
    );

    // Unix 专属：POSIX 绝对路径必须接受。
    #[cfg(unix)]
    {
        let mut posix = config.clone();
        posix.external_editors.insert(
            EditorRole::Score,
            ExternalEditor {
                absolute_path: PathBuf::from("/usr/bin/vi"),
                arguments: Vec::new(),
            },
        );
        posix.validate().expect("POSIX 绝对路径必须被接受");
    }

    // Windows 专属：没有盘符的"有根路径"不是绝对路径 ⇒ 必须拒绝。
    #[cfg(windows)]
    {
        assert!(
            !Path::new(ROOTED_BUT_NOT_ABSOLUTE_EDITOR_PATH).is_absolute(),
            "Windows: 只有根、没有盘符的路径不是绝对路径"
        );
        let mut rooted = config.clone();
        rooted.external_editors.insert(
            EditorRole::Sample,
            ExternalEditor {
                absolute_path: PathBuf::from(ROOTED_BUT_NOT_ABSOLUTE_EDITOR_PATH),
                arguments: Vec::new(),
            },
        );
        assert!(
            matches!(
                rooted.validate(),
                Err(yeban_model::local_config::LocalConfigError::EditorPathNotAbsolute { .. })
            ),
            "Windows: 无盘符的有根路径必须被拒绝"
        );
    }
}

#[cfg(not(unix))]
#[test]
fn local_config_file_is_0600() {
    // 非 Unix 平台没有 POSIX 权限位语义（Windows ACL 不等价于 0600）。
    // 如实 SKIP，并声明这里**没有**被验证过 —— 不假装绿。
    eprintln!("SKIP: 本平台不是 Unix ⇒ 0600 权限位判据未执行（Windows ACL 语义需另立判据）");
}

// ---------------------------------------------------------------------------
// ⑩ 本机配置往返 + 确定性
// ---------------------------------------------------------------------------

#[test]
fn local_config_round_trip_is_field_exact_and_deterministic() {
    let config = populated_config();

    let first = config.to_json().expect("序列化");
    let second = config.to_json().expect("序列化");
    assert_eq!(first, second, "同一配置两次序列化必须逐字节相同");

    let loaded = LocalMachineConfig::from_json(&first).expect("反序列化");
    assert_eq!(loaded, config, "往返必须逐字段相等");
    assert_eq!(loaded.version, LOCAL_CONFIG_VERSION);
    assert_eq!(
        loaded.external_editors.get(&EditorRole::Waveform),
        config.external_editors.get(&EditorRole::Waveform)
    );
    assert_eq!(
        loaded.cloud_tokens.get("anthropic"),
        config.cloud_tokens.get("anthropic")
    );

    // 插入顺序不同的两份配置必须序列化出**相同**字节（BTreeMap 确定性，红线 4）。
    let mut reordered = LocalMachineConfig {
        version: LOCAL_CONFIG_VERSION,
        audio_binding: AudioPortBinding {
            input_ports: BTreeMap::from([(1_u32, "Input 2".to_owned()), (0, "Input 1".to_owned())]),
            output_ports: BTreeMap::from([
                (1_u32, "Output 2".to_owned()),
                (0, "Output 1".to_owned()),
            ]),
            ..config.audio_binding.clone()
        },
        external_editors: BTreeMap::new(),
        cloud_tokens: BTreeMap::new(),
    };
    reordered.cloud_tokens.insert(
        "elevenlabs".to_owned(),
        SecretRef::new("yeban/cloud/elevenlabs").expect("合法"),
    );
    reordered.cloud_tokens.insert(
        "anthropic".to_owned(),
        SecretRef::new("yeban/cloud/anthropic").expect("合法"),
    );
    reordered.external_editors.insert(
        EditorRole::Score,
        ExternalEditor {
            absolute_path: PathBuf::from(SCORE_EDITOR_PATH),
            arguments: Vec::new(),
        },
    );
    reordered.external_editors.insert(
        EditorRole::Waveform,
        ExternalEditor {
            absolute_path: PathBuf::from(WAVEFORM_EDITOR_PATH),
            arguments: vec!["--open".to_owned()],
        },
    );
    assert_eq!(
        reordered.to_json().expect("序列化"),
        first,
        "BTreeMap 插入顺序不得影响字节（红线 4 的确定性）"
    );

    // 反面对照：改一个字段必须改字节。
    let mut changed = config.clone();
    changed.audio_binding.input_device = Some("另一个声卡".to_owned());
    assert_ne!(changed.to_json().expect("序列化"), first);

    // 拒绝项逐条实测（不允许静默写入）。
    assert!(
        LocalMachineConfig::from_json(br#"{"version":1}"#).is_err(),
        "缺键必须拒绝"
    );
    assert!(
        LocalMachineConfig::from_json(
            br#"{"version":1,"audio_binding":{"input_device":null,"output_device":null,"input_ports":{},"output_ports":{}},"external_editors":{},"cloud_tokens":{},"mystery":1}"#
        )
        .is_err(),
        "未知字段必须拒绝（deny_unknown_fields）"
    );
    assert!(
        LocalMachineConfig::from_json(
            br#"{"version":99,"audio_binding":{"input_device":null,"output_device":null,"input_ports":{},"output_ports":{}},"external_editors":{},"cloud_tokens":{}}"#
        )
        .is_err(),
        "版本过新必须拒绝"
    );
    let mut relative = LocalMachineConfig::default();
    relative.external_editors.insert(
        EditorRole::Sample,
        ExternalEditor {
            absolute_path: PathBuf::from(RELATIVE_EDITOR_PATH),
            arguments: Vec::new(),
        },
    );
    assert!(relative.to_json().is_err(), "相对路径必须拒绝");
}

/// 类别⑤／③：同一份本机配置**写两次**必须把文件覆盖成**同一串字节**，
/// 而且"读回来的配置再写一次"仍是同一串字节。
///
/// 判什么：`std::fs::read` 回来的文件字节（单位 = 字节）。
/// 为什么需要：同一个文件的 `to_json` 两次逐字节相同已有判据
/// （`local_config_round_trip_is_field_exact_and_deterministic`），但**落盘那一步**没有：
/// `save_to` 一旦从 `truncate(true)` 变成追加（`append(true)` 或漏掉 `truncate`），
/// 多次保存会让文件里叠出多份 JSON —— `to_json` 的判据全绿，坏掉的却正是磁盘上的事实源。
#[test]
fn repeated_saves_overwrite_in_place_and_are_byte_identical() {
    let config = populated_config();
    let home = TempDir::new("repeated-save");
    let path = LocalMachineConfig::path_for_home(&home.path);

    config.save_to(&path).expect("第一次保存");
    let first = std::fs::read(&path).expect("读文件");
    config.save_to(&path).expect("第二次保存");
    let second = std::fs::read(&path).expect("读文件");
    assert_eq!(
        first, second,
        "同一配置连续两次保存必须把文件写成同一串字节（不得追加）"
    );
    assert_eq!(
        first,
        config.to_json().expect("序列化"),
        "落盘字节必须就是 to_json 的字节（不得多出包装或补白）"
    );

    // 重载再写：字节必须原样回到磁盘（类别③ 的"重新加载后与全新实例一致"）。
    let reloaded = LocalMachineConfig::load_from(&path).expect("重载");
    reloaded.save_to(&path).expect("重载后再保存");
    assert_eq!(
        std::fs::read(&path).expect("读文件"),
        first,
        "重载再写必须逐字节回到同一串字节"
    );
}

/// 覆盖写的**尖锐**形态：先写一份更长的配置，再写一份更短的，磁盘上必须恰好是
/// 第二次的字节（上一次的尾巴不许留下）。
///
/// 为什么需要：上一条判据两次写的是**同一份**配置（长度相同），因此"先截断再写"与
/// "只覆盖前 N 个字节"在那里不可区分。实测：把 `save_to` 的 `.truncate(true)` 改成
/// `.truncate(false)` 时，全仓判据保持全绿 —— 而留下的尾巴会让下一次
/// `load_from` 在 JSON 尾随垃圾上失败（配置被静默损坏），正是上一条判据的文档
/// 警告过的那种破损。
#[test]
fn a_shorter_save_leaves_no_stale_tail() {
    let home = TempDir::new("shorter-save");
    let path = LocalMachineConfig::path_for_home(&home.path);

    let mut long = populated_config();
    long.cloud_tokens.insert(
        "a-very-long-provider-name".to_owned(),
        SecretRef::new("yeban/cloud/a-very-long-provider-name").expect("条目名"),
    );
    long.save_to(&path).expect("写长配置");
    let long_bytes = std::fs::read(&path).expect("读长配置");
    let short_bytes = populated_config().to_json().expect("序列化短配置");
    assert!(
        long_bytes.len() > short_bytes.len(),
        "夹具必须真的更长 ({} vs {})",
        long_bytes.len(),
        short_bytes.len()
    );

    populated_config().save_to(&path).expect("写短配置");
    assert_eq!(
        std::fs::read(&path).expect("读短配置"),
        short_bytes,
        "更短的一次保存必须把文件截断成恰好这一串字节（不得留上一次的尾巴）"
    );
    LocalMachineConfig::load_from(&path).expect("留下的字节必须还能读回来");
}

/// `SecretMaterial::drop` 的 best-effort 清零必须还在。
///
/// 为什么用源码扫描而不是运行时断言：清零的观察面是"释放后的堆页"，而本 crate
/// `forbid(unsafe_code)`，安全代码读不到已释放内存 ⇒ 没有运行时判据。既有的
/// `secret_material_never_reaches_disk` 只钉住**不落盘**，`Debug` 打码由另一条
/// 判据钉住。实测：删掉 `Drop` 里的 `self.bytes.fill(0);` 时全仓判据保持全绿。
///
/// 扫描器自身有牙：同一手法（`include_str!` + 定点取块）在
/// `session_state_has_no_serde_surface` 里已有正/负对照的先例。
#[test]
fn secret_material_is_wiped_on_drop() {
    const LOCAL_SRC: &str = include_str!("../src/local_config.rs");

    /// 从源码里取出 `impl Drop for SecretMaterial` 的函数体文本。
    fn drop_body(source: &str) -> String {
        let start = source
            .find("impl Drop for SecretMaterial")
            .expect("SecretMaterial 必须实现 Drop");
        let end = source[start..].find("\n}").expect("Drop 实现必须有结尾") + start;
        source[start..end].to_owned()
    }

    let body = drop_body(LOCAL_SRC);
    assert!(
        body.contains("fn drop(&mut self)"),
        "取到的块不是 Drop 实现体: {body}"
    );
    assert!(
        body.contains("self.bytes.fill(0);"),
        "SecretMaterial::drop 的 best-effort 清零不见了: {body}"
    );
    // 扫描器自身有牙：同一手法在"清零被删掉"的合成输入上必须给出不同的结论
    // （否则本判据可能只是在扫描器恒真上通过）。
    let withered = LOCAL_SRC.replace("self.bytes.fill(0);", "");
    assert_ne!(withered, LOCAL_SRC, "合成输入必须真的被改动过");
    assert!(
        !drop_body(&withered).contains("self.bytes.fill(0);"),
        "扫描器对'清零被删掉'的输入必须报否"
    );
}

// ---------------------------------------------------------------------------
// ⑪ 密钥本体不入盘：只有引用名
// ---------------------------------------------------------------------------

#[test]
fn secret_material_never_reaches_disk() {
    // 假密钥 = "真实密钥"的替身；它只进内存密钥库，不进任何文件。
    let material = SecretMaterial::from_text(FAKE_SECRET_MATERIAL);
    let store = ProbeSecretStore::default();
    let reference = SecretRef::new("yeban/cloud/anthropic").expect("合法条目名");
    store.put(&reference, &material).expect("内存后端写入");
    assert_eq!(
        store.get(&reference).expect("内存后端读取").expose(),
        FAKE_SECRET_MATERIAL.as_bytes(),
        "探针必须真的把假密钥放进去了（否则'没落盘'是空转）"
    );

    let mut config = LocalMachineConfig::default();
    config
        .cloud_tokens
        .insert("anthropic".to_owned(), reference.clone());

    let home = TempDir::new("secret");
    let path = LocalMachineConfig::path_for_home(&home.path);
    config.save_to(&path).expect("写本机配置");
    let on_disk = std::fs::read(&path).expect("读文件");
    let text = String::from_utf8(on_disk.clone()).expect("配置是 UTF-8 JSON");

    // 1) 文件里**只有引用名**（指针），没有密钥本体。
    assert!(
        text.contains("yeban/cloud/anthropic"),
        "引用指针必须落盘（否则无从找回密钥）"
    );
    assert!(
        !text.contains("PROBE"),
        "文件里出现了假密钥片段 ⇒ 密钥本体泄漏"
    );
    assert!(!contains_subslice(
        &on_disk,
        FAKE_SECRET_MATERIAL.as_bytes()
    ));
    assert!(!text.contains("sk-"), "文件里出现了密钥形态的前缀");

    // 2) 结构层：每个 token 条目就是一个 JSON **字符串**，没有承载 material 的位置。
    let value: Value = serde_json::from_slice(&on_disk).expect("JSON");
    let tokens = value["cloud_tokens"]
        .as_object()
        .expect("cloud_tokens 对象");
    assert_eq!(tokens.len(), 1);
    for (provider, token) in tokens {
        assert!(
            token.is_string(),
            "cloud_tokens[{provider}] 必须是字符串引用指针，实际: {token}"
        );
    }
    let ref_value: Value = serde_json::to_value(&reference).expect("序列化引用");
    assert!(ref_value.is_string(), "SecretRef 的序列化形状必须是字符串");

    // 3) `Debug` 打码（最容易被顺手打进日志的通道）。
    let debug = format!("{material:?}");
    assert!(!debug.contains("PROBE"), "SecretMaterial 的 Debug 必须打码");
    assert!(
        debug.contains("redacted"),
        "Debug 必须显式标注已打码: {debug}"
    );

    // 4) 反面对照：**故意**把假密钥塞进文件 ⇒ 上面的探针必须抓到。
    let leaky = format!(r#"{{"cloud_tokens":{{"anthropic":"{FAKE_SECRET_MATERIAL}"}}}}"#);
    assert!(contains_subslice(
        leaky.as_bytes(),
        FAKE_SECRET_MATERIAL.as_bytes()
    ));

    // 5) 引用名的校验边界：空白 / 控制字符 / 空串 / 超长一律拒绝。
    assert!(SecretRef::new("").is_err());
    assert!(SecretRef::new("has space").is_err());
    assert!(SecretRef::new("has\nnewline").is_err());
    assert!(SecretRef::new("A".repeat(129)).is_err());
    assert!(SecretRef::new("A".repeat(128)).is_ok());
}

// ---------------------------------------------------------------------------
// ⑫ 未知 / 未接入的 keychain 后端 ⇒ 明确错误
// ---------------------------------------------------------------------------

#[test]
fn unknown_secret_backend_is_an_explicit_error() {
    // 未知后端：明确错误（不是 panic、不是静默成功）。
    match secret_store_for("definitely-not-a-keychain") {
        Err(SecretStoreError::UnknownBackend { requested, known }) => {
            assert_eq!(requested, "definitely-not-a-keychain");
            for name in KNOWN_SECRET_BACKENDS {
                assert!(known.contains(name), "已知后端清单里必须列出 {name}");
            }
        }
        other => panic!(
            "未知后端必须返回 UnknownBackend，实际: {:?}",
            other.err().map(|error| error.to_string())
        ),
    }

    // 默认（`unavailable`）后端：认识，但每个动作都返回明确错误。
    let store = secret_store_for(DEFAULT_SECRET_BACKEND).expect("默认后端名必须被认识");
    assert_eq!(store.backend_name(), DEFAULT_SECRET_BACKEND);
    let reference = SecretRef::new("yeban/cloud/probe").expect("合法");
    let material = SecretMaterial::from_text("probe-material");
    for outcome in [
        store.put(&reference, &material).err(),
        store.get(&reference).err(),
        store.delete(&reference).err(),
    ] {
        match outcome {
            Some(SecretStoreError::BackendUnavailable { backend, detail }) => {
                assert_eq!(backend, DEFAULT_SECRET_BACKEND);
                assert!(!detail.is_empty());
            }
            other => panic!("默认后端必须返回 BackendUnavailable，实际: {other:?}"),
        }
    }

    // 已登记但未接入的后端：仍是明确错误（绝不静默降级成"成功"）。
    for backend in ["os-keychain", "macos-keychain", "secret-service"] {
        let store = secret_store_for(backend).expect("已登记后端名必须被认识");
        assert_eq!(store.backend_name(), backend);
        let error = store
            .get(&reference)
            .expect_err("未接入的后端必须报错而不是返回空密钥");
        assert!(
            matches!(error, SecretStoreError::BackendUnavailable { backend: b, .. } if b == backend),
            "{backend}: {error}"
        );
    }

    // 占位实现自身的错误不可为空（"明确错误"的最低标准）。
    let unavailable = UnavailableSecretStore::default();
    assert_eq!(unavailable.backend(), DEFAULT_SECRET_BACKEND);
    assert!(!unavailable.detail().is_empty());
}

// ---------------------------------------------------------------------------
// ⑬ 播放头是 960 PPQ 整数 tick
// ---------------------------------------------------------------------------

#[test]
fn playhead_is_integer_ticks_at_960_ppq() {
    // 类型层：字段是 `u64`。改成浮点 ⇒ 这一行**编译失败**。
    let session = SessionRuntimeState::default();
    let _: u64 = session.playhead_ticks;

    assert_eq!(PPQ, 960);
    assert_eq!(SessionRuntimeState::ticks_per_beat(), 960);
    assert_eq!(SessionRuntimeState::ticks_per_bar(4, 4), Some(3840));
    assert_eq!(SessionRuntimeState::ticks_per_bar(6, 8), Some(2880));
    assert_eq!(SessionRuntimeState::ticks_per_bar(3, 4), Some(2880));
    // 不整除 ⇒ 拒绝（不取整、不 panic）。
    assert_eq!(SessionRuntimeState::ticks_per_bar(4, 7), None);
    assert_eq!(SessionRuntimeState::ticks_per_bar(0, 4), None);
    assert_eq!(SessionRuntimeState::ticks_per_bar(4, 0), None);

    let mut session = SessionRuntimeState::default();
    session.seek_ticks(960 * 3 + 480);
    assert_eq!(
        session.playhead_bar_and_offset(4, 4),
        Some((0, 3 * 960 + 480)),
        "整数 tick 的小节内偏移"
    );
    session.seek_ticks(3840 * 2 + 100);
    assert_eq!(session.playhead_bar_and_offset(4, 4), Some((2, 100)));
    // 饱和：不 wrap、不 panic。
    session.seek_ticks(u64::MAX);
    assert_eq!(session.advance_ticks(1), u64::MAX);
    session.rewind();
    assert_eq!(session.playhead_ticks(), 0);

    // 源码层：会话态不许出现浮点。
    const SESSION_SRC: &str = include_str!("../src/session.rs");
    for needle in ["f32", "f64"] {
        assert!(
            !code_has(SESSION_SRC, needle),
            "session.rs 出现了浮点类型 `{needle}` ⇒ 播放头必须是整数 tick"
        );
    }
}

// ---------------------------------------------------------------------------
// ⑭ 视窗 / PID 集合的确定性顺序
// ---------------------------------------------------------------------------

#[test]
fn window_and_pid_sets_are_deterministic() {
    let first = EntityId::default();
    let ulid_b = {
        use std::str::FromStr as _;
        EntityId::from_str("01J8ZQ00000000000000000002").expect("合法 ULID")
    };
    let third = {
        use std::str::FromStr as _;
        EntityId::from_str("01J8ZQ00000000000000000003").expect("合法 ULID")
    };

    let all_windows = [
        WindowId::Arrangement,
        WindowId::PianoRoll,
        WindowId::Mixer,
        WindowId::Browser,
        WindowId::PluginEditor(first),
        WindowId::PluginEditor(ulid_b),
    ];

    // 乱序插入 ⇒ 迭代顺序仍必须按 `Ord` 升序（HashSet 会让这条抖动）。
    let mut session = SessionRuntimeState::default();
    for index in [3_usize, 0, 5, 2, 4, 1] {
        assert!(session.open_window(all_windows[index]));
    }
    // 幂等：重复打开不改变集合。
    assert!(!session.open_window(all_windows[0]));
    assert_eq!(session.open_windows.len(), 6);

    let ordered = session.windows_in_order();
    let mut expected = all_windows.to_vec();
    expected.sort_unstable();
    assert_eq!(ordered, expected, "视窗集合必须按 Ord 升序迭代（BTreeSet）");
    for pair in ordered.windows(2) {
        assert!(pair[0] < pair[1], "相邻元素必须严格升序: {pair:?}");
    }
    // ⛔ 原写法 `assert_eq!(a(), a())` 是**恒真**（同一个纯函数、同一份未变状态被调用两次，
    //    两侧文本完全相同 ⇒ R75/R80 的形状①）。改成**跨实例**确定性：另起一个会话
    //    （走**另一条合法构造路径** `new()`，R86 要求驱动不得对两个被测实例做相同初始化），
    //    按同样的插入顺序放同样 6 个视窗 ⇒ 两个**不同实例**的遍历必须逐元素相同。
    //    这样"哈希种子不影响顺序"才是真的被检验，而不是自比。
    let mut twin = SessionRuntimeState::new();
    for index in [3_usize, 0, 5, 2, 4, 1] {
        assert!(twin.open_window(all_windows[index]));
    }
    assert_eq!(
        twin.open_windows.len(),
        6,
        "R93：对照组必须真的装进 6 个视窗，否则下面两条断言是空转的"
    );
    assert_eq!(
        session.windows_in_order(),
        twin.windows_in_order(),
        "两个独立实例（同样插入顺序、不同构造路径）必须给出同一顺序 —— BTreeSet 与哈希种子无关"
    );
    assert_eq!(
        session.windows_in_order().len(),
        6,
        "R93：被遍历的集合非空且达到下界"
    );

    // PID 集合：重复 PID 去重 + 升序。
    session.track_plugin_process(
        first,
        PluginProcess {
            pid: 4242,
            sandboxed: true,
        },
    );
    session.track_plugin_process(
        ulid_b,
        PluginProcess {
            pid: 7,
            sandboxed: false,
        },
    );
    session.track_plugin_process(
        third,
        PluginProcess {
            pid: 4242,
            sandboxed: true,
        },
    );
    assert_eq!(
        session.plugin_pids().into_iter().collect::<Vec<_>>(),
        vec![7, 4242],
        "PID 集合必须去重且升序（BTreeSet）"
    );
    assert!(session.forget_plugin_process(&third));
    assert!(!session.forget_plugin_process(&third));
    assert_eq!(
        session.plugin_pids().into_iter().collect::<Vec<_>>(),
        vec![7, 4242]
    );

    assert!(session.close_window(&WindowId::Mixer));
    assert!(!session.is_window_open(&WindowId::Mixer));

    // 任务进度：整数百分比 + 校验边界。
    let task = TaskId::new(9);
    session.set_task(
        task,
        TaskProgress {
            kind: TaskKind::AssetImport,
            completed_units: 1,
            total_units: 3,
            cancellable: false,
            label: "import".to_owned(),
        },
    );
    assert_eq!(session.task_progress(task).expect("任务存在").percent(), 33);
    assert!(!session.task_progress(task).expect("任务存在").is_finished());
    assert!(session.clear_task(task));
    assert!(session.task_progress(task).is_none());

    session.set_task(
        TaskId::new(1),
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 2,
            total_units: 0,
            cancellable: false,
            label: String::new(),
        },
    );
    assert!(session.validate().is_err(), "total_units == 0 必须被拒绝");
    session.set_task(
        TaskId::new(1),
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 5,
            total_units: 2,
            cancellable: false,
            label: String::new(),
        },
    );
    assert!(session.validate().is_err(), "completed > total 必须被拒绝");
    session.set_task(
        TaskId::new(1),
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 2,
            total_units: 2,
            cancellable: false,
            label: String::new(),
        },
    );
    session.validate().expect("合法任务必须通过");
    assert_eq!(
        session
            .task_progress(TaskId::new(1))
            .expect("任务")
            .percent(),
        100
    );
    assert!(
        session
            .task_progress(TaskId::new(1))
            .expect("任务")
            .is_finished()
    );
}

/// `total_units == 0` 必须报**具体**的 [`SessionStateError::TaskTotalZero`]。
///
/// 为什么需要：上面那条判据只断言 `validate().is_err()`，而
/// `completed = 2 > total = 0` 会**顺带**给出 `TaskProgressOutOfRange`
/// ⇒ "错误码正确"这一条没有被钉住。实测：把 `progress.total_units == 0` 改成
/// `== 1` 时全仓判据保持全绿。这里把 `completed_units` 也置零，于是唯一的拒绝
/// 理由只能是 `TaskTotalZero`。
#[test]
fn a_zero_unit_task_is_rejected_with_the_total_zero_error() {
    let mut session = SessionRuntimeState::default();
    session.set_task(
        TaskId::new(7),
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 0,
            total_units: 0,
            cancellable: false,
            label: String::new(),
        },
    );
    assert_eq!(
        session.validate(),
        Err(SessionStateError::TaskTotalZero { task: 7 })
    );
    // 正侧对照：1/1 必须合法（否则本判据会退化成"什么都拒绝"的假绿）。
    session.set_task(
        TaskId::new(7),
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 1,
            total_units: 1,
            cancellable: false,
            label: String::new(),
        },
    );
    assert_eq!(session.validate(), Ok(()));
}

// ---------------------------------------------------------------------------
// ⑮ forbid(unsafe_code) + 零 GUI 依赖
// ---------------------------------------------------------------------------

#[test]
fn crate_keeps_forbid_unsafe_and_zero_gui_deps() {
    const LIB_SRC: &str = include_str!("../src/lib.rs");
    const MANIFEST: &str = include_str!("../Cargo.toml");

    assert!(
        LIB_SRC.contains("#![forbid(unsafe_code)]"),
        "yeban-model 必须保持 #![forbid(unsafe_code)]（红线 8）"
    );
    // 对照：扫描器能分辨"有 / 没有"这条属性。
    assert!(!code_has(
        "#![allow(unsafe_code)]\n",
        "#![forbid(unsafe_code)]"
    ));

    // 零 GUI 依赖（红线 3 / G02）：清单里不许出现任何 GUI 相关依赖名。
    for gui in [
        "slint", "winit", "glutin", "glow", "wgpu", "egui", "gtk", "qt", "skia", "i-slint",
        "opengl",
    ] {
        assert!(
            !MANIFEST.contains(gui),
            "yeban-model 的清单里出现了 GUI 依赖记号 `{gui}`（红线 3）"
        );
    }
    // 对照：清单确实被读到了（否则"没命中"是空转）。
    assert!(MANIFEST.contains("serde"));
    assert!(MANIFEST.contains("[dependencies]"));
    // 本线**零新增依赖**：清单里不许出现密钥链类依赖。
    for keychain in ["keyring", "security-framework", "keychain", "zeroize"] {
        assert!(
            !MANIFEST.contains(keychain),
            "本机配置层不得为了'看起来完成'引入 `{keychain}`（需人类裁决）"
        );
    }
    // 会话态 / 本机配置层的源码也不许引用 GUI。
    const SESSION_SRC: &str = include_str!("../src/session.rs");
    const LOCAL_SRC: &str = include_str!("../src/local_config.rs");
    for (name, src) in [("session.rs", SESSION_SRC), ("local_config.rs", LOCAL_SRC)] {
        for gui in ["slint", "winit", "wgpu"] {
            assert!(!code_has(src, gui), "{name} 引用了 GUI `{gui}`");
        }
    }
}

// ---------------------------------------------------------------------------
// ⑰ 重新冻结入口（`#[ignore]`：不进 CI，只供人工取证）
// ---------------------------------------------------------------------------

/// 打印当前的真实冻结值 —— **唯一**的重新冻结入口。
///
/// ```text
/// cargo test -p yeban-model --test model_isolation -- --ignored --nocapture
/// ```
#[test]
#[ignore = "取证入口：只在需要重新冻结常量时手动跑"]
fn print_frozen_isolation_table() {
    for (name, doc) in [
        ("project.default.json", default_project()),
        ("project.filled.json", filled_project()),
    ] {
        println!("(\"{name}\", \"{}\"),", sha256_hex(&project_bytes(&doc)));
    }

    let value = project_value(&filled_project());
    println!("top level keys: {:?}", top_level_keys(&value));
    let mut union = BTreeSet::new();
    for doc in [default_project(), filled_project()] {
        union.extend(key_paths_of(&project_value(&doc)));
    }
    let (digest, count) = key_paths_digest(&union);
    println!("key path count = {count}");
    println!("key path sha256 = {digest}");

    let config = populated_config();
    let config_json: Value =
        serde_json::from_slice(&config.to_json().expect("配置")).expect("json");
    println!("local config keys: {:?}", top_level_keys(&config_json));
    println!(
        "local config json: {}",
        config.to_json().expect("配置").len()
    );
    println!(
        "session type: {}",
        std::any::type_name::<SessionRuntimeState>()
    );
}

/// `AudioPortBinding::is_unbound` 的"一个端口都没绑定"必须**四个字段全空**才成立。
///
/// 为什么需要（第五轮注入实测）：本方法是全 crate **零调用点、零判据引用**的公开判定器
/// （`grep -w is_unbound` 只命中定义行自身），因此把它整段换成 `false` 时全仓判据
/// 保持全绿。
#[test]
fn audio_port_binding_is_unbound_only_when_all_four_fields_are_empty() {
    assert!(
        AudioPortBinding::default().is_unbound(),
        "全默认（四字段皆空）必须判为未绑定"
    );

    // 四个字段**逐个**都能单独把它变成"已绑定"。
    let device_in = AudioPortBinding {
        input_device: Some("Focusrite".to_owned()),
        ..AudioPortBinding::default()
    };
    let device_out = AudioPortBinding {
        output_device: Some("BuiltIn".to_owned()),
        ..AudioPortBinding::default()
    };
    let port_in = AudioPortBinding {
        input_ports: BTreeMap::from([(0_u32, "Mic 1".to_owned())]),
        ..AudioPortBinding::default()
    };
    let port_out = AudioPortBinding {
        output_ports: BTreeMap::from([(1_u32, "Out 2".to_owned())]),
        ..AudioPortBinding::default()
    };
    for (label, binding) in [
        ("input_device", &device_in),
        ("output_device", &device_out),
        ("input_ports", &port_in),
        ("output_ports", &port_out),
    ] {
        assert!(!binding.is_unbound(), "{label} 非空时不得判为未绑定");
        binding.validate().expect("非空标识串必须自洽");
    }

    let all = AudioPortBinding {
        input_device: Some("Focusrite".to_owned()),
        output_device: Some("BuiltIn".to_owned()),
        input_ports: BTreeMap::from([(0_u32, "Mic 1".to_owned())]),
        output_ports: BTreeMap::from([(1_u32, "Out 2".to_owned())]),
    };
    assert!(!all.is_unbound(), "四字段都非空时同样判为已绑定");
}

/// `task_progress` 必须按**身份**取回那一条任务，而不是"任意一条"。
///
/// 为什么需要（第六轮注入实测）：把 `self.tasks.get(&task)` 换成
/// `self.tasks.values().next()` 时全仓判据保持全绿 —— 既有
/// `window_and_pid_sets_are_deterministic` 只看**集合**，从没有拿两个不同身份
/// 去查同一张表。多任务并发进度是 UI 的直接输入，取错身份是用户可见的错。
#[test]
fn task_progress_returns_the_named_task_and_not_another_one() {
    let first = TaskId::new(1);
    let second = TaskId::new(2);
    let mut state = SessionRuntimeState::default();
    assert!(state.task_progress(first).is_none(), "空会话没有任务");

    state.set_task(
        first,
        TaskProgress {
            kind: TaskKind::ProjectLoad,
            completed_units: 1,
            total_units: 4,
            cancellable: true,
            label: "load".to_owned(),
        },
    );
    state.set_task(
        second,
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 3,
            total_units: 4,
            cancellable: false,
            label: "render".to_owned(),
        },
    );

    assert_eq!(state.task_progress(first).expect("first").label, "load");
    assert_eq!(
        state.task_progress(first).expect("first").completed_units,
        1
    );
    assert_eq!(state.task_progress(second).expect("second").label, "render");
    assert_eq!(
        state.task_progress(second).expect("second").completed_units,
        3
    );
    assert!(
        state.task_progress(TaskId::new(3)).is_none(),
        "陌生身份必须落空"
    );
}

/// `SessionRuntimeState::default()` 必须是**空会话且播放头为 0**。
///
/// 为什么需要（第六轮注入实测）：把 `playhead_ticks: 0` 改成 `1` 时全仓判据保持全绿 ——
/// `playhead_is_integer_ticks_at_960_ppq` 探的是 `seek_ticks` / `advance_ticks`，
/// 从没有断言过**缺省**播放头。
#[test]
fn session_default_is_a_clean_slate_at_tick_zero() {
    let state = SessionRuntimeState::default();
    assert_eq!(state.playhead_ticks, 0);
    assert!(!state.is_playing);
    assert!(state.plugin_pids().is_empty());
    assert!(state.windows_in_order().is_empty());
    assert!(state.task_progress(TaskId::new(1)).is_none());
    assert_eq!(state.undo_cursor.skip, 0);
    assert_eq!(state.validate(), Ok(()), "缺省会话必须自洽");
}

/// `SecretRef` 的 `Display` 必须给出**引用名本身**（不是空串、也不是掩码）。
///
/// 为什么需要（第六轮注入实测）：把 `f.write_str(&self.0)` 换成 `f.write_str("")`
/// 时全仓判据保持全绿。引用名是要进容器 / 日志的**身份**，不是秘密 —— 它必须可读。
#[test]
fn secret_ref_display_is_the_reference_name() {
    let reference = SecretRef::new("vault://yeban/cloud-token").expect("合法引用名");
    assert_eq!(reference.to_string(), "vault://yeban/cloud-token");
    assert_eq!(reference.to_string(), reference.entry());
    assert!(!reference.to_string().is_empty());
}

/// `SecretMaterial` 的 `Debug` 必须**打码但保留字节数**。
///
/// 为什么需要（第六轮注入实测）：把 `"SecretMaterial(<redacted {} bytes>)"` 换成
/// `"SecretMaterial(<redacted>)"` 时全仓判据保持全绿 —— `secret_material_is_wiped_on_drop`
/// 探的是释放清零，从没有钉住打码形态本身。字节数是审计线索（"这次到底带了多少秘密"），
/// 丢了它就只能靠猜。
#[test]
fn secret_material_debug_keeps_the_redacted_byte_count() {
    let material = SecretMaterial::from_text("super-secret");
    let rendered = format!("{material:?}");
    assert_eq!(rendered, "SecretMaterial(<redacted 12 bytes>)");
    assert!(
        !rendered.contains("super-secret"),
        "Debug 通道绝不能泄出密钥本体: {rendered}"
    );
}

/// `ops.default.json` / `ops.filled.json` 的**逐字节冻结**（与 `project.*.json` 同口径）。
///
/// 为什么需要（第六轮发现、第七轮落地）：`project_json_byte_samples_are_frozen` 的
/// 逐字节表**只覆盖 `project.default.json` / `project.filled.json`**；
/// `samples::tests::export_all_writes_four_byte_stable_samples` 只比较"两次导出相等"
/// ⇒ 发布用的两个 **ops 样本**此前**没有任何冻结哈希**，任何确定性的载荷改动都能过。
///
/// 口径与 `samples::write_json` 逐字节同形：`serde_json::to_string_pretty` ＋ 一个换行。
/// 用 `--ignored --nocapture` 的冻结入口（见 `print_frozen_isolation_table`）重新取证。
#[test]
fn ops_json_byte_samples_are_frozen() {
    const FROZEN_OPS_JSON: [(&str, &str, usize); 2] = [
        (
            "ops.default.json",
            "d331a363621468c9135216ba00332e8ddc40f380b0a00360840ed07e55d14069",
            351,
        ),
        (
            "ops.filled.json",
            "9dcf3e5c98070062c6ede3613df4b40cd00ec88579fa794041d2b2cb00b82a4d",
            1932,
        ),
    ];
    let samples = [
        ("ops.default.json", default_stamped_op()),
        ("ops.filled.json", filled_stamped_op()),
    ];
    for ((name, stamped), (frozen_name, frozen_sha, frozen_len)) in
        samples.iter().zip(FROZEN_OPS_JSON)
    {
        assert_eq!(*name, frozen_name);
        let bytes = ops_sample_bytes(stamped);
        assert_eq!(
            bytes.len(),
            frozen_len,
            "{name}: 样本字节数变了（冻结表里的字面读数）"
        );
        // 两个入口必须逐字节一致（与 project 侧同一条冗余）。
        let mut via_vec = serde_json::to_vec_pretty(stamped).expect("to_vec_pretty");
        via_vec.push(b'\n');
        assert_eq!(
            bytes, via_vec,
            "{name}: to_string_pretty 与 to_vec_pretty 必须同字节"
        );
        assert_eq!(
            sha256_hex(&bytes),
            frozen_sha,
            "{name}: ops 样本的逐字节内容变了。若这是有意的契约变更, \
             请用 `cargo test -p yeban-model --test model_isolation -- --ignored --nocapture` \
             重新冻结；若是无意改动, 它此前**没有**任何判据拦得住"
        );
    }

    // ---- 判据自身有牙：改一个 ops 载荷字段必须让摘要变红 ----
    let mut tampered = default_stamped_op();
    let Op::SetSection { new_section, .. } = &mut tampered.op else {
        panic!("默认样本的变体必须是 SetSection");
    };
    new_section.end_tick += 1;
    assert_ne!(
        sha256_hex(&ops_sample_bytes(&tampered)),
        FROZEN_OPS_JSON[0].1,
        "改一个载荷字段后摘要必须变红 —— 否则这条冻结判据是空转的"
    );
}

/// 与 `samples::write_json` 逐字节同形的 ops 样本编码。
fn ops_sample_bytes(stamped: &StampedOp) -> Vec<u8> {
    let mut json = serde_json::to_string_pretty(stamped).expect("序列化");
    json.push('\n');
    json.into_bytes()
}

/// `LocalMachineConfig::default()` 的载荷逐字段钉住。
///
/// 为什么需要（第七轮注入实测）：把 `version: LOCAL_CONFIG_VERSION` 改成 `0` 时全仓判据
/// 保持全绿 —— 既有 `local_config_round_trip_is_field_exact_and_deterministic` 用的是
/// **显式**构造的配置。
#[test]
fn local_machine_config_default_is_frozen_field_by_field() {
    let config = LocalMachineConfig::default();
    assert_eq!(LOCAL_CONFIG_VERSION, 1, "先钉常量本身的取值");
    assert_eq!(config.version, 1);
    assert!(config.audio_binding.is_unbound());
    assert!(config.external_editors.is_empty());
    assert!(config.cloud_tokens.is_empty());
    config.validate().expect("缺省配置必须自洽");
}

/// `SecretRef` 的 **serde 入口**必须走与 `SecretRef::new` 同一把校验尺子。
///
/// 为什么需要（第七轮注入实测）：把 `Deserialize for SecretRef` 的 `Self::new(raw)` 换成
/// 直接构造 `SecretRef(raw)`（绕过校验）时全仓判据保持全绿 —— 而那条 impl 的文档**明确
/// 承诺**"反序列化也走 `SecretRef::new` 的校验"（R52：判据名/文档 ≠ 覆盖面）。
#[test]
fn secret_ref_deserialization_goes_through_the_validator() {
    let good: SecretRef =
        serde_json::from_str("\"vault://yeban/cloud-token\"").expect("合法条目名");
    assert_eq!(good.entry(), "vault://yeban/cloud-token");

    for bad in [
        "\"\"",
        "\"has space\"",
        "\"has\\ttab\"",
        "\"has\\nnewline\"",
    ] {
        assert!(
            serde_json::from_str::<SecretRef>(bad).is_err(),
            "坏条目名 {bad} 必须在校验处被拒"
        );
    }
    // 长度上界的同一把尺子（`MAX_SECRET_REF_LEN`）。
    let too_long = "x".repeat(MAX_SECRET_REF_LEN + 1);
    assert!(
        serde_json::from_str::<SecretRef>(&format!("\"{too_long}\"")).is_err(),
        "超长条目名必须被拒"
    );
}

/// `TaskProgress.cancellable` 与 `PluginProcess.sandboxed` 必须被**照原样**存入会话态。
///
/// 为什么需要（第七轮全字段读点普查）：这两个字段是**全仓零读点**的公开字段
/// （`crates/yeban-model/src/session.rs:104` / `:133`）—— 它们属于"给调用方/界面准备的
/// 载荷"，模型层唯一的义务是**不篡改**。写入端把它们强制成 `false` 时，此前没有任何
/// 判据看得见（测试自己也不读它们）。
#[test]
fn session_writers_do_not_rewrite_the_payload_they_are_given() {
    let mut state = SessionRuntimeState::default();
    let task = TaskId::new(1);
    state.set_task(
        task,
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 0,
            total_units: 2,
            cancellable: false,
            label: "render".to_owned(),
        },
    );
    assert!(!state.task_progress(task).expect("task").cancellable);
    state.set_task(
        task,
        TaskProgress {
            kind: TaskKind::Render,
            completed_units: 1,
            total_units: 2,
            cancellable: true,
            label: "render".to_owned(),
        },
    );
    assert!(
        state.task_progress(task).expect("task").cancellable,
        "写入端不得改写调用方给的载荷"
    );

    let instance = EntityId::default();
    state.track_plugin_process(
        instance,
        PluginProcess {
            pid: 42,
            sandboxed: true,
        },
    );
    assert!(
        state.plugin_processes[&instance].sandboxed,
        "写入端不得改写调用方给的载荷"
    );
    state.track_plugin_process(
        instance,
        PluginProcess {
            pid: 43,
            sandboxed: false,
        },
    );
    assert!(!state.plugin_processes[&instance].sandboxed);
    assert_eq!(state.plugin_processes[&instance].pid, 43);
}

/// `history.dag` 与容器 ZIP 的**逐字节冻结**（R70②：自比不是字节契约）。
///
/// 为什么需要（第九轮 · R70②）：这两个交付物此前**只有自比式判据** ——
/// `commit::tests::history_dag_round_trips_and_is_byte_stable` 比的是"两次编码相等"、
/// `container_roundtrip.rs` 比的是"两个写出入口相等"。**"两次运行相同"不是字节契约**：
/// 任何**确定性**的改动（换 `to_vec_pretty`、改 ZIP 的 unix mode、改压缩级别）
/// 都会让自比继续全绿而交付字节已经变了。
///
/// 本判据按与生产同形的调用路径取字节，冻结 **sha256 ＋ 长度**两张字面表，
/// 并附"判据自身有牙"的 `assert_ne!`（R58：同一个 `==` 上成对）。
/// 重新冻结：`cargo test -p yeban-model --test model_isolation -- --nocapture`。
#[test]
fn history_dag_and_container_zip_bytes_are_frozen() {
    const DAG_SHA256: &str = "54c9212169a07d34e54ab58114090e88d86f06371c53002f91f3295e8c5cbc32";
    const DAG_LEN: usize = 911;
    const ZIP_SHA256: &str = "a025fdfe020d690e04d1221e5c0beef14c4cf6a16245f9db65a277c0db98f844";
    const ZIP_LEN: usize = 5448;

    let id1 = EntityId::from_str("01J8ZQ00000000000000000001").expect("id1");
    let id2 = EntityId::from_str("01J8ZQ00000000000000000002").expect("id2");

    // ---- `history.dag`：两个提交（固定身份 / 固定时间戳 / 固定种子）----
    let canonical = |created_at: u64| {
        let mut graph = CommitGraph::new();
        graph
            .genesis(
                CommitDraft::new(id1, "main", "agent", "genesis")
                    .with_created_at(1_760_000_000_000)
                    .with_rng_seed(7),
            )
            .expect("genesis");
        graph
            .append(
                CommitDraft::new(id2, "main", "agent", "step")
                    .with_created_at(created_at)
                    .with_rng_seed(8)
                    .with_ops(vec![default_stamped_op()]),
            )
            .expect("append");
        graph
    };
    let graph = canonical(1_760_000_000_001);
    let dag = encode_history_dag(&graph);
    assert_eq!(dag.len(), DAG_LEN, "history.dag 的长度变了（字面读数）");
    assert_eq!(
        sha256_hex(&dag),
        DAG_SHA256,
        "history.dag 的逐字节内容变了。自比式判据（两次编码相等）**拦不住**确定性改动, \
         本表是那一侧唯一的字面契约；有意变更请重新冻结"
    );
    // 判据自身有牙：只改一个提交字段（时间戳 +1）⇒ 摘要必须变红。
    assert_ne!(
        sha256_hex(&encode_history_dag(&canonical(1_760_000_000_002))),
        DAG_SHA256,
        "只改一个提交的 created_at 后摘要必须变红 —— 否则这条冻结判据是空转的"
    );

    // ---- 容器 ZIP：`filled_project` ＋ 一条资产 ----
    let asset = AssetHash::of_bytes(b"yeban-freeze-asset");
    let zip = write_project_container_borrowed(
        &filled_project(),
        b"{\"commits\":[]}",
        &[(&asset, b"yeban-freeze-asset")],
    )
    .expect("写容器");
    assert_eq!(zip.len(), ZIP_LEN, "容器 ZIP 的长度变了（字面读数）");
    assert_eq!(
        sha256_hex(&zip),
        ZIP_SHA256,
        "容器 ZIP 的逐字节内容变了（换 pretty 序列化 / 改 unix mode / 改压缩级别都会命中这里）"
    );
    // 判据自身有牙：换一个字节的资产（连带它的 CAS 键）⇒ 摘要必须变红。
    let other = AssetHash::of_bytes(b"yeban-freeze-asset!");
    assert_ne!(
        sha256_hex(
            &write_project_container_borrowed(
                &filled_project(),
                b"{\"commits\":[]}",
                &[(&other, b"yeban-freeze-asset!")],
            )
            .expect("写容器")
        ),
        ZIP_SHA256,
        "只换一条资产的字节后摘要必须变红 —— 否则这条冻结判据是空转的"
    );
}

// ---------------------------------------------------------------------------
// R93 / R102 / R106 的**常驻**机械化：本 crate 不得出现"无下界的 `.all(…)` 断言"。
// ---------------------------------------------------------------------------

/// R94 掩码：注释与字符串 → 空格，**保留换行**（行号不漂）。
///
/// 字符字面量只认闭合形态 `'x'`：⛔ 否则 `'"'`（内含双引号的字符字面量）会让掩码器
/// 从这里启一段假字符串，把后面的代码整段吞掉 —— 第十一轮实测到的**工具自身**缺陷。
fn mask_rust_source(text: &str) -> String {
    // ⭐ **保字节数**：被掩掉的字符按它的 `len_utf8()` 补空格（`\n` 原样保留）。
    // 为什么必须保字节数：本判据要拿**掩码的偏移**回原文里判定"这一处是否落在
    // 注释/字符串里"。若每个字符只补 1 个空格，含中文注释的文件里 `masked` 会比 `raw` 短
    // ⇒ 偏移**错位** ⇒ 真断言会被误判成"在注释里"而**静默跳过**（第十一轮实测到的**假阴性**：
    // 删掉一处显式下界后常驻判据仍是 GREEN）。
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    fn blank(out: &mut String, c: char) {
        if c == '\n' {
            out.push('\n');
        } else {
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        }
    }
    let mut i = 0_usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                blank(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let mut depth = 0_usize;
            while i < chars.len() {
                if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                    depth += 1;
                    blank(&mut out, chars[i]);
                    blank(&mut out, chars[i + 1]);
                    i += 2;
                    continue;
                }
                if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    depth -= 1;
                    blank(&mut out, chars[i]);
                    blank(&mut out, chars[i + 1]);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                blank(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '"' {
            blank(&mut out, c);
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                if d == '\\' {
                    blank(&mut out, d);
                    i += 1;
                    if i < chars.len() {
                        blank(&mut out, chars[i]);
                        i += 1;
                    }
                    continue;
                }
                if d == '"' {
                    blank(&mut out, d);
                    i += 1;
                    break;
                }
                blank(&mut out, d);
                i += 1;
            }
            continue;
        }
        if c == '\'' {
            let escaped = i + 3 < chars.len() && chars[i + 1] == '\\' && chars[i + 3] == '\'';
            let plain = i + 2 < chars.len() && chars[i + 2] == '\'';
            if escaped || plain {
                let take = if escaped { 4 } else { 3 };
                for _ in 0..take {
                    blank(&mut out, chars[i]);
                    i += 1;
                }
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 找出**没有下界**的 `.all(…)` / `.any(…)` 断言，返回 `文件:行 集合` 列表。
///
/// 为什么需要（R93/R102/R106）：`assert!(x.windows(2).all(|p| p[0] < p[1]))` 在 `x.len() < 2`
/// 时**恒真**；`x` 若是 `filter(...)` 的结果，`[] == []` 同样真空。下界有**两种写法**（R102）：
/// ① 显式（`.len() >= N`、计数器 `ident >= N`、`!x.is_empty()`）；
/// ② 宏隐式相等（`assert_eq!(x.len(), N|CONST)`）。**两种都认**，否则会给假阳性。
fn unbounded_all_any_sites(relative: &str, raw: &str) -> Vec<String> {
    let masked = mask_rust_source(raw);
    let mut found = Vec::new();
    let mut search = 0_usize;
    while let Some(position) = masked[search..].find("assert!(") {
        let start = search + position;
        search = start + 1;
        // 掩码里是空白、原文里不是空白 ⇒ 这一处落在注释/字符串里（假断言），跳过。
        if masked.as_bytes()[start] != raw.as_bytes()[start] {
            continue;
        }
        let hi = (start + 2500).min(raw.len());
        let lo = start.saturating_sub(2500);
        // 前后都看（下界常写在断言**之前**），但下界必须挂在**同一个集合根**上（见 `has_root_bound`）。
        let window = &masked[lo..hi];
        // 只看**本断言自己的实参**（⛔ 否则会把下一条断言的 `.all(` 算到自己头上）。
        let Some(body) = paren_body(&masked, start + "assert!".len()) else {
            continue;
        };
        let Some(call) = body.find(".all(").or_else(|| body.find(".any(")) else {
            continue;
        };
        if !body[call..].contains('|') {
            continue;
        }
        let root = collection_root(body[..call].trim());
        if !has_root_bound(&compact(window), &root) {
            let line = raw[..start].matches('\n').count() + 1;
            found.push(format!("{relative}:{line} 集合根=`{root}`"));
        }
    }
    found
}

/// `open` 是 `(` 的位置（在**已掩码**文本里）⇒ 返回配对括号之间的正文。
///
/// 为什么需要（第十一轮的第二个假阳性）：`assert!(v.len() >= 2); assert!(v.windows(2).all(…))`
/// 里，第一处 `assert!` **没有** `.all(`；若直接向后 find 下一个 `.all(`，就会把**下一条断言**
/// 的 `.all(` 记到它头上，于是"下界缺失"被误报。必须按**本断言的实参**解析。
fn paren_body(text: &str, open: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0_i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[open + 1..i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// 去掉**全部空白**：集合表达式与下界都可能跨行（rustfmt 会把长链折行），
/// 因此比较前必须归一化 —— 否则 `doc.routing_graph\n .nodes` 与
/// `doc.routing_graph.nodes.len() >= 3` 匹配不上（第十一轮实测的**假阳性**）。
fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 集合的**根表达式**：剥掉尾部的"视图方法"链（`.windows(…)` / `.iter()` / `.chars()` …）。
///
/// 为什么需要（第十一轮的假阴性）：第一版在**±900 字符窗口**里找任何 `>=` 就认下有界 ⇒
/// 一旦按 R56 删掉某一处**显式下界**，它会拿**邻近另一条断言**的下界当自己的 ⇒ 判据变绿
/// （假阴性）。改成"下界必须挂在**同一个集合根**上"才是真的在查这一条断言。
fn collection_root(expr: &str) -> String {
    const VIEWS: [&str; 9] = [
        ".windows(",
        ".iter(",
        ".chars(",
        ".bytes(",
        ".values(",
        ".keys(",
        ".copied(",
        ".rev(",
        ".enumerate(",
    ];
    let mut cut = expr.len();
    for marker in VIEWS {
        if let Some(at) = expr.find(marker)
            && at < cut
        {
            cut = at;
        }
    }
    compact(&expr[..cut])
}

/// 该集合的**根**上有没有下界 —— 两种写法都认（R102）：
/// ① 显式：`<root>.len() >= N` / `> N` / `!<root>.is_empty()`；
/// ② 宏隐式相等：`assert_eq!(<root>.len(), N|CONST)`。
fn has_root_bound(window: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    // 标识符边界：`bb.len()` 里**含有** `b.len()` 这个子串 ⇒ 子串匹配会把 `bb` 的下界
    // 记到 `b` 头上（第十二轮由 R114 判据的"near-miss"已知红当场抓到）。
    let boundary_ok = |at: usize| {
        at == 0
            || !window[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
    };
    let len = format!("{root}.len()");
    let mut search = 0_usize;
    while let Some(found) = window[search..].find(&len) {
        let at = search + found;
        search = at + 1;
        if !boundary_ok(at) {
            continue;
        }
        let after = at + len.len();
        let rest = window[after..].trim_start();
        if rest.starts_with(">=") || rest.starts_with('>') {
            return true;
        }
        // 宏隐式相等（R102 写法②）：`assert_eq!(<root>.len(), N|CONST)`
        if window[..at].trim_end().ends_with("assert_eq!(") {
            return true;
        }
    }
    // 显式写法之一：`!<root>.is_empty()`（同样要求标识符边界）。
    let neg = format!("!{root}.is_empty()");
    let mut search = 0_usize;
    while let Some(found) = window[search..].find(&neg) {
        let at = search + found;
        search = at + 1;
        if boundary_ok(at + 1) {
            return true;
        }
    }
    false
}

/// **R93/R102/R106 常驻判据**：本 crate 的 `src/**` 与 `tests/**` 里不得出现"无下界的
/// `.all(…)`/`.any(…)`"断言。一次性审计不会随代码演进复跑；本判据把它机械化。
///
/// R100 的非真空三件套：① 被扫文件数有下界（`>= 30`）；② 扫描器先掩码注释与字符串
/// （R94）⇒ 本判据**自己的源码**里作为示例出现的 `.all(` 不会被自己命中；
/// ③ 扫描器的正/负对照用**合成输入**逐条钉住（R56：已知红 ＋ 已知绿）。
#[test]
fn no_unbounded_all_any_assertion_in_this_crate() {
    // ---- R56 已知红 / 已知绿（合成输入，不依赖仓库内容）----
    let red = "fn f() { assert!(v.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", red).len(),
        1,
        "无下界的 `.all(…)` 必须被扫出来（R56 已知红）"
    );
    let green_explicit =
        "fn f() { assert!(v.len() >= 2); assert!(v.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", green_explicit).len(),
        0,
        "显式下界（写法①）必须被认到（R56 已知绿）"
    );
    let green_macro =
        "fn f() { assert_eq!(v.len(), 4); assert!(v.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", green_macro).len(),
        0,
        "宏隐式相等（写法②）必须被认到（R102：只认一种写法会给假阳性）"
    );
    let in_comment = "fn f() { /* assert!(v.windows(2).all(|p| p[0] < p[1])); */ }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", in_comment).len(),
        0,
        "注释里的示例不得被当真（R94）"
    );

    // ---- R113：掩码必须**保字节数**（⛔ 不是字符数），否则偏移错位 ⇒ 真断言被静默跳过 ----
    let multibyte = "// 中文注释（多字节）：断言在下一行\nfn f() { assert!(v.windows(2).all(|p| p[0] < p[1])); }\n";
    let masked_multibyte = mask_rust_source(multibyte);
    assert_eq!(
        masked_multibyte.len(),
        multibyte.len(),
        "R113：掩码必须保**字节**长度（按 len_utf8() 补空格）"
    );
    assert!(
        multibyte.len() > multibyte.chars().count(),
        "R113 的分母：夹具必须真的含多字节字符（否则本自检是空转的）"
    );
    assert_eq!(
        unbounded_all_any_sites("synthetic", multibyte).len(),
        1,
        "R113：多字节注释**之后**的真断言必须仍被扫到（偏移不得错位）"
    );

    // ---- R111：五种"界"形态逐条给结论（本面只认真正能界定集合的那几种）----
    // 形态②：`!x.is_empty()` —— 认。
    let green_not_empty =
        "fn f() { assert!(!v.is_empty()); assert!(v.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", green_not_empty).len(),
        0,
        "R111 形态②（`!is_empty()`）必须被认到"
    );
    // 形态④：**值界** `assert_eq!(x, CONST)` —— ⛔ **不认**：它界定的是取值，不是**集合大小**，
    // `x` 为空集合时那条断言同样可以成立 ⇒ 后面的 `.all(…)` 仍然真空。
    let value_bound = "fn f() { assert_eq!(v, 4); assert!(v.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", value_bound).len(),
        1,
        "R111 形态④（值界）**不得**被当成集合下界（否则 `.all(…)` 会真空通过）"
    );
    // 形态⑤：**运行期计数器** `ident >= N` —— ⛔ 在本面同样**不认**：计数器不界定被遍历的集合；
    // 它对应的是 R106 的"`filter(...)` ＋ 循环内断言"形态（那条由仓库里的
    // `assert!(judged >= 3, "…判据在空转")` 承担，本判据不改写那条契约）。
    let counter = "fn f() { assert!(seen >= 2); assert!(v.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", counter).len(),
        1,
        "R111 形态⑤（计数器）不界定集合 ⇒ 本面不得把它当集合下界"
    );

    // ---- 真源码扫描（R100：被扫文件数必须有下界）----
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", dir.display()))
            .map(|entry| entry.expect("目录项").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    walk(&root.join("src"), &mut files);
    walk(&root.join("tests"), &mut files);
    // R100 的下界用**带余量的地板**（与 `no_hash_containers_anywhere_in_src` 的
    // `files.len() >= 16` 同口径），不是精确冻结 —— 别的线加测试文件不该把本判据弄红，
    // 但"扫描域塌成空/个位数"必须红。当前实际 **26** 个（第十一轮读数）。
    assert!(
        files.len() >= 20,
        "必须真的扫完 src/** 与 tests/**（至少 20 个 *.rs），实际 {} —— 否则本判据空转",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &files {
        let relative = path
            .strip_prefix(root)
            .expect("crate 根之下")
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(path).expect("读取源文件");
        // R113 的**常驻自检**：对**每一个**被扫文件断言掩码保字节长度
        // （本 crate 的源码里有中文注释 ⇒ 只要掩码按"每字符 1 空格"就会在这里红）。
        assert_eq!(
            mask_rust_source(&text).len(),
            text.len(),
            "{relative}: 掩码必须保字节长度（R113）"
        );
        offenders.extend(unbounded_all_any_sites(&relative, &text));
    }
    assert!(
        offenders.is_empty(),
        "发现无下界的 `.all(…)`/`.any(…)` 断言（len<2 时恒真 / `[] == []` 真空）：\n{}",
        offenders.join("\n")
    );
}

/// **R114 常驻判据**：下界必须**根绑定** —— ⛔ 不得"借用邻居"的下界。
///
/// 为什么需要（第十一轮实测的**假阴性**）：第一版扫描器在断言周围的窗口里找**任何** `>=`
/// 就认下有界 ⇒ 一旦删掉某一处显式下界，它会拿**邻近另一条断言**的下界（例如同一个测试里
/// `a.len() >= 2`）当自己的 ⇒ 该处漏报（`M11-01` 实测 GREEN）。
/// 本判据用合成输入把这个形态**钉死**：下界挂在**别的集合**上 ⇒ 仍算无界；挂在**同一个根**上
/// ⇒ 才算有界（R56：一条已知红 ＋ 一条已知绿）。
#[test]
fn bounds_must_be_root_bound_not_borrowed_from_a_neighbour() {
    // ⛔ 已知红：下界挂在 `a` 上，被断言的集合是 `b` ⇒ 必须仍然报"无界"。
    let borrowed =
        "fn f() { assert!(a.len() >= 2); assert!(b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", borrowed).len(),
        1,
        "R114：挂在**别的集合**上的下界不得算作本断言的下界（借用邻居 = 假阴性）"
    );
    // ✅ 已知绿：下界挂在**同一个根** `b` 上 ⇒ 认。
    let rooted = "fn f() { assert!(b.len() >= 2); assert!(b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", rooted).len(),
        0,
        "R114：同根下界必须被认到"
    );
    // ✅ 已知绿：宏隐式相等同样必须**同根**。
    let rooted_macro =
        "fn f() { assert_eq!(b.len(), 2); assert!(b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", rooted_macro).len(),
        0,
        "R114：同根的宏隐式相等必须被认到"
    );
    // ⛔ 已知红：同根但**只差一个字符**（`bb`）的下界也不得被借用。
    let near_miss =
        "fn f() { assert!(bb.len() >= 2); assert!(b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", near_miss).len(),
        1,
        "R114：根必须精确匹配，⛔ 不得前缀匹配"
    );
}

/// **R119 常驻判据**：根的匹配必须落在**标识符边界**上（⛔ 不得子串匹配）。
///
/// 为什么需要（第十二轮实测的**假阴性**）：`has_root_bound` 原来用
/// `window.find("{root}.len()")` ⇒ `bb.len()` 里**含有** `b.len()` 子串 ⇒ `b` 被误判为有界，
/// 于是"删掉 `b` 的显式下界"这种坏改动可以溜过常驻判据。本判据用合成输入钉死该形态
/// （R56：known-red ＋ known-green 各若干条）。
#[test]
fn bounds_must_match_the_root_at_a_token_boundary() {
    // ⛔ 已知红（near-miss，R119）：界挂在 `bb` 上 ⇒ **不得**记到 `b` 头上。
    let near_miss =
        "fn f() { assert!(bb.len() >= 2); assert!(b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", near_miss).len(),
        1,
        "R119：`bb.len()` 不得被当成 `b` 的下界（子串匹配 = 假阴性）"
    );
    // ✅ 已知绿：同一根 `b` ⇒ 认。
    let same_root =
        "fn f() { assert!(b.len() >= 2); assert!(b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", same_root).len(),
        0,
        "R119：同根且落在标识符边界上必须被认到"
    );
    // ⛔ 已知红：字段访问形态 `outer.b` 与独立变量 `b` 是**不同根**。
    let field_root =
        "fn f() { assert!(outer.b.len() >= 2); assert!(b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", field_root).len(),
        1,
        "R119：`outer.b.len()` 不得记到独立变量 `b` 头上"
    );
    // ✅ 已知绿：同一字段根 `outer.b` ⇒ 认。
    let field_same = "fn f() { assert!(outer.b.len() >= 2); assert!(outer.b.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", field_same).len(),
        0,
        "R119：同一字段根必须被认到"
    );

    // ---- R118：`x.len() == N`（界定**集合**⇒ 认）vs `x[0] == N`（**元素值界** ⇒ 不认）----
    let len_eq = "fn f() { assert_eq!(x.len(), 3); assert!(x.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", len_eq).len(),
        0,
        "R118：`assert_eq!(x.len(), N)` 界定集合 ⇒ 必须认（宏隐式相等）"
    );
    let element_value =
        "fn f() { assert_eq!(x[0], 3); assert!(x.windows(2).all(|p| p[0] < p[1])); }\n";
    assert_eq!(
        unbounded_all_any_sites("synthetic", element_value).len(),
        1,
        "R118：`assert_eq!(x[0], N)` 是**元素值界**，不界定集合 ⇒ 必须报无界（prefix near-miss）"
    );
}
