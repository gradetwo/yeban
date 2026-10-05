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
//! ## 判据清单（14 条 + 1 条 `#[ignore]`）
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
//! | ⑯ | [`print_frozen_isolation_table`]（`#[ignore]`） | 重新冻结常量时的取证入口 |
//!
//! 另外两条"判据自身有牙"的反空洞证明写在 ⑥ / ③ 里（合成输入 ⇒ 扫描器 / 哈希必须变红）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use sha2::{Digest, Sha256};
use yeban_model::container::{
    ContainerLimits, HISTORY_DAG_NAME, PROJECT_JSON_NAME, read_container, read_project_container,
    write_project_container,
};
use yeban_model::local_config::{
    AudioPortBinding, DEFAULT_SECRET_BACKEND, EditorRole, ExternalEditor, KNOWN_SECRET_BACKENDS,
    LOCAL_CONFIG_VERSION, LocalMachineConfig, SecretMaterial, SecretRef, SecretStore,
    SecretStoreError, UnavailableSecretStore, secret_store_for,
};
use yeban_model::samples::{default_project, filled_project};
use yeban_model::session::{
    PluginProcess, SessionRuntimeState, TaskId, TaskKind, TaskProgress, WindowId,
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
            absolute_path: PathBuf::from("/Applications/Audacity.app/Contents/MacOS/Audacity"),
            arguments: vec!["--open".to_owned()],
        },
    );
    config.external_editors.insert(
        EditorRole::Score,
        ExternalEditor {
            absolute_path: PathBuf::from("/Applications/MuseScore 4.app/Contents/MacOS/mscore"),
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
            absolute_path: PathBuf::from("/Applications/MuseScore 4.app/Contents/MacOS/mscore"),
            arguments: Vec::new(),
        },
    );
    reordered.external_editors.insert(
        EditorRole::Waveform,
        ExternalEditor {
            absolute_path: PathBuf::from("/Applications/Audacity.app/Contents/MacOS/Audacity"),
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
            absolute_path: PathBuf::from("relative/editor"),
            arguments: Vec::new(),
        },
    );
    assert!(relative.to_json().is_err(), "相对路径必须拒绝");
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
    // 连续两次遍历必须完全相同（哈希种子不得影响顺序）。
    assert_eq!(session.windows_in_order(), session.windows_in_order());

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
// ⑯ 重新冻结入口（`#[ignore]`：不进 CI，只供人工取证）
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
