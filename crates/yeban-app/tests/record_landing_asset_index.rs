//! T-FIX-468：录音落地必须与 CAS 池**双登记**（`project.assets` 索引 ＋ `assets/{sha256}` 字节）。
//!
//! ## 这条判据要证的句子
//!
//! | # | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① | 落地后 `audit_project_assets(&archive).unindexed_pool` **为空** | 证"池里的字节在 `project.assets` 里有对应索引项"。不证字节内容对（那由 ④ 的哈希比对证） |
//! | ② | 元数据自洽：`hash` 与池内键一致、`byte_len` == 池内字节数、`media_kind = Audio`、`license = user-recorded` | 证"索引项不是占位" |
//! | ③ | 落地 → `save_archive_file` → 回读：该哈希在 `project.assets` **与** `assets` 池里都能查到 | 证"这次双登记经得起一次保存往返" |
//! | ④ | 索引里**已有**同哈希时再落一次 ⇒ 不重复声明（`Op::SetAsset` 不重复、池不重复） | 证"幂等"，⛔ 不是"覆盖" |
//!
//! ## 为什么钉在这里
//!
//! 产品路径是 `host::wire_record` → 停录 → [`record_landing::land_recording_at`]
//! （`crates/yeban-app/src/host.rs` 的停录分支）。本判据直接调**同一个**落地函数，
//! 只跳过"声卡 + 无头窗口"那一层：落地函数之后的每一跳（提交、池、索引、保存）
//! 都是产品代码。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! 删掉 `land_recording_at` 里那段 `Op::SetAsset` ⇒ ① 红（`unindexed_pool` 变成
//! `[录音哈希]`）、③ 回读时 `project.assets` 里查不到（严格打开还会直接
//! `MissingAsset`）。② 同时红（索引项根本不存在）。

use std::path::PathBuf;

use yeban_app::open::{ProjectOpenOptions, audit_project_assets, open_project_archive_file};
use yeban_app::record_landing::{RecordTarget, land_recording_at};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::container::ProjectArchive;
use yeban_model::project::MediaKind;
use yeban_model::samples::{default_project, filled_project, filled_project_assets};
use yeban_model::{AssetHash, YebanProjectV1};

const NOW: u64 = 1_760_000_000_000;

/// 临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-record-index-{tag}-{}-{}",
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

fn assemble(project: YebanProjectV1) -> UndoPort {
    UndoPort::new(
        UndoSession::open("record-asset-index", "yeban-app", project, NOW).expect("打开撤销会话"),
    )
}

/// 从会话现场拼一份归档（= 保存时的那份输入）。
fn archive_of(port: &UndoPort) -> ProjectArchive {
    ProjectArchive {
        project: port.try_project().expect("会话里有活跃工程"),
        history_dag: Vec::new(),
        proposals: None,
        assets: port.assets(),
        embedded_presets: Vec::new(),
        load_notes: Vec::new(),
    }
}

/// 一份**真的** WAV：直接用 `filled_kick_wav` 的字节（`filled_project_assets` 给出）。
fn write_kick_wav(scratch: &Scratch) -> (PathBuf, AssetHash, usize) {
    let assets = filled_project_assets();
    assert_eq!(assets.len(), 1, "filled 夹具应恰好带一份 kick");
    let (hash, bytes) = assets.into_iter().next().expect("kick");
    let path = scratch.join("take-01.wav");
    std::fs::write(&path, &bytes).expect("写 WAV");
    (path, hash, bytes.len())
}

/// ⭐ 判据 ① ②：录音落地后索引与池双登记，且元数据自洽。
#[test]
fn a_landed_recording_is_indexed_in_project_assets() {
    let scratch = Scratch::new("index");
    let (wav, expected_hash, wav_len) = write_kick_wav(&scratch);
    // `default_project()` 没有音频轨 ⇒ 落 `RecordTarget::NoAudioTrack`：
    // 片段照常进 `clip_pool`、资产照常进池，只是不摆放（`plan_placement` 的旧口径）。
    // 正是这个形态最干净地隔离"索引"这一件事。
    let port = assemble(default_project());
    assert!(
        !port
            .try_project()
            .expect("project")
            .assets
            .contains_key(&expected_hash),
        "夹具前提：默认工程不该预先索引这段 WAV"
    );

    let landed =
        land_recording_at(&port, &wav, NOW, 0, RecordTarget::NoAudioTrack).expect("录音必须能落地");
    assert_eq!(landed.asset, expected_hash, "落地哈希取字节 SHA-256");

    let archive = archive_of(&port);
    let audit = audit_project_assets(&archive);

    // ① 池里每一个字节都要有索引项。
    assert!(
        audit.unindexed_pool.is_empty(),
        "录音落地后池内资产必须全部有 project.assets 索引项，实际未索引: {:?}",
        audit.unindexed_pool
    );
    assert_eq!(audit.pool, 1, "会话池里应恰好这一段录音");
    assert_eq!(audit.project_index, 1, "project.assets 里应恰好一条索引项");
    // 没有音频轨 ⇒ 没有被摆放的引用，但索引与池必须成对。
    assert!(
        audit.missing.is_empty(),
        "不应有「被引用但池里没有」的资产: {:?}",
        audit.missing
    );

    // ② 元数据自洽：不是占位项。
    let metadata = archive
        .project
        .assets
        .get(&expected_hash)
        .expect("索引项必须真的在 project.assets 里");
    assert_eq!(metadata.hash, expected_hash, "索引项的 hash 必须等于键");
    assert_eq!(
        metadata.byte_len,
        u64::try_from(wav_len).expect("长度"),
        "byte_len 必须等于池内字节数"
    );
    assert_eq!(metadata.media_kind, MediaKind::Audio, "录音是音频资产");
    assert_eq!(
        metadata.license, "user-recorded",
        "录音来源要如实标注，⛔ 不是 user-imported"
    );
    assert_eq!(
        metadata.original_path, "take-01.wav",
        "original_path 只留文件名（⛔ 不把临时目录钉进 .yeban）"
    );
    let pooled = archive
        .assets
        .iter()
        .find(|(hash, _)| *hash == expected_hash)
        .expect("池里必须有这段字节");
    assert_eq!(
        pooled.1.len(),
        usize::try_from(metadata.byte_len).expect("长度"),
        "索引声明的字节数必须与池内字节数逐字一致"
    );
}

/// ⭐ 判据 ③：双登记经得起一次保存往返。
#[test]
fn the_double_registration_survives_a_save_round_trip() {
    let scratch = Scratch::new("roundtrip");
    let (wav, expected_hash, _) = write_kick_wav(&scratch);
    let port = assemble(default_project());
    land_recording_at(&port, &wav, NOW, 0, RecordTarget::NoAudioTrack).expect("录音必须能落地");

    let archive = archive_of(&port);
    let project_path = scratch.join("saved.yeban");
    yeban_app::save::save_archive_file(&archive, &project_path).expect("全保真归档必须能落盘");

    let reopened =
        open_project_archive_file(&project_path, &ProjectOpenOptions::default()).expect("回读");
    assert!(
        reopened.project.assets.contains_key(&expected_hash),
        "回读后 project.assets 里必须仍有这条索引（⛔ 保存不得丢索引）"
    );
    assert!(
        reopened
            .assets
            .iter()
            .any(|(hash, _)| *hash == expected_hash),
        "回读后 assets 池里必须仍有这段字节"
    );
    let audit = audit_project_assets(&reopened);
    assert!(
        audit.unindexed_pool.is_empty(),
        "回读后仍须双登记，实际未索引: {:?}",
        audit.unindexed_pool
    );
}

/// ⭐ 判据 ④：索引里已有同哈希 ⇒ 第二次落地不重复声明、池不重复。
#[test]
fn landing_the_same_take_twice_does_not_duplicate_the_declaration() {
    let scratch = Scratch::new("idempotent");
    let (wav, expected_hash, _) = write_kick_wav(&scratch);
    // `filled_project()` 的 Audio 片段本来就引用这份 kick ⇒ 索引里**已有**该哈希。
    let port = assemble(filled_project());
    assert!(
        port.try_project()
            .expect("project")
            .assets
            .contains_key(&expected_hash),
        "夹具前提：filled 工程已索引这份 kick"
    );

    // 第一次落地：索引已有 ⇒ 不产 `Op::SetAsset`；会话池初始为空 ⇒ 补一次字节。
    let before = archive_of(&port);
    land_recording_at(&port, &wav, NOW, 0, RecordTarget::NoAudioTrack).expect("落地必须成功");
    let first = archive_of(&port);
    assert_eq!(
        first.project.assets.len(),
        before.project.assets.len(),
        "索引里已有同哈希 ⇒ 不得重复声明"
    );
    assert_eq!(
        first.assets.len(),
        before.assets.len() + 1,
        "会话池原本没有这段字节 ⇒ 补一次（这不叫重复登记）"
    );
    assert_eq!(
        first.project.assets.get(&expected_hash),
        before.project.assets.get(&expected_hash),
        "既有索引项不得被本次落地改写（幂等，⛔ 不覆盖）"
    );

    // 第二次落地：索引与池都已在 ⇒ 一位不动。
    land_recording_at(&port, &wav, NOW, 0, RecordTarget::NoAudioTrack).expect("再次落地必须成功");
    let second = archive_of(&port);
    assert_eq!(
        second.project.assets.len(),
        first.project.assets.len(),
        "第二次落地不得再声明"
    );
    assert_eq!(
        second.assets.len(),
        first.assets.len(),
        "第二次落地不得再登记字节"
    );
}
