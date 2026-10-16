//! `crate::drums` 的 `needs-3` **模型面**那一半：模型**不禁止**同一轨多件音源（任务单 `#96`）。
//!
//! ## 这条判据钉住的**一句话**
//!
//! `crates/yeban-engine/src/drums.rs` 的 needs 表逐字写着：「`needs-3` | 同轨多件音源的裁决……
//! 今天引擎的裁决是"鼓机优先"（§4.1）；**模型若禁止这种组合，本条删除**」。
//! ⇒ 本文件机械回答那一支：**模型不禁止** —— 同一条轨**同时**可以有
//! [`TrackV3::sampler`]（`Some`）**与**一台会被引擎当成鼓机的 [`DeviceDefinition`]。
//!
//! ## 为什么"模型不禁止"是**可判**的（而不是一句感想）
//!
//! 判据 ① 用模型的公开 API 走**两次 `Op`**（[`Op::InsertDevice`] ＋ [`Op::SetTrackSampler`]），
//! 断言两次**都被接受**，并在同一个 `YebanProjectV1` 上把两个读数**同时**读出来；
//! 判据 ② 把"哪台设备会被引擎当成鼓机"这件事**从引擎源码读出来**对账 —— ⛔ 不手抄一份
//! 可能陈旧的名字表（仓内既有手法：`tests/schema_sampler_branch.rs` 直接读契约文件）。
//!
//! ## 引擎怎么认鼓机（本文件引用的**唯一**事实源）
//!
//! `crates/yeban-engine/src/drums.rs` 的 `DrumsParams::from_devices`（规则 1）＋
//! `from_device`（规则 5）：
//!
//! 1. 只看 `kind == DeviceKind::InternalInstrument` **且** `bypassed == false` 的设备；
//! 2. 该设备的 `params` 里**五个键位名**（大小写不敏感）**全部**出现且取值**有限**：
//!    `kick_note` / `snare_note` / `closed_hat_note` / `open_hat_note` / `clap_note`
//!    （`fn note_slot` 的 match 臂就是这份名单）。
//!
//! ⚠ 同一个 `devices` 序列还被 `crates/yeban-engine/src/synth.rs` 的
//! `ToneParams::from_devices` **独立**扫一遍（识别键 `cutoff_hz`/`cutoff` 或六个振荡器键）
//! ⇒「一台设备链里既有复音合成器参数、又有完整鼓机映射」在模型里就是**一条 `devices` 里的
//! 普通内容**，模型层既不认识"鼓机"、也不认识"复音合成器"。
//!
//! ## 判据（2 条，全部进 `cargo test -p yeban-model`）
//!
//! | # | 判据 | 测的是什么 |
//! | :-- | :--- | :--- |
//! | ① | [`model_allows_a_track_to_hold_both_a_sampler_and_a_drum_shaped_instrument`] | 两次 `Op` 都被接受 ＋ 两个读数同时成立 ＋ 整份文档仍合法可往返；⭐ 附**两条正对照**（只设 `sampler` / 只设设备）证明断言不是空转 |
//! | ② | [`the_five_key_names_are_exactly_the_engines_drum_recognition_names`] | 判据 ① 用的五个键位名 ⇔ `crates/yeban-engine/src/drums.rs::note_slot` 的 match 臂（**读源码**，⛔ 不手抄） |
//!
//! ## 边界（诚实声明）
//!
//! - 本文件**只加判据**：⛔ 不改任何生产代码、⛔ 不改 `schemas/**`（那是集成者独占）、
//!   ⛔ 不动 `crates/yeban-engine/**`（那是另一条线的树）。判据 ② 对引擎源码**只读**；
//! - 本文件**不主张**"引擎的裁决对不对" —— `鼓机 ＞ SFZ ＞ 复音` 是**引擎层**的取舍
//!   （`crate::drums` §4.1 ＋ `sfz-needs-3` 的信号腿），本文件只说**模型层不禁止**这个组合；
//! - ⚠ `needs-1`（"这台设备是鼓机"没有类型级信息）**仍然成立**：本文件正是靠"五个键位名"
//!   这条**名称约定**构造鼓机的；类型级信息落下后，本文件的设备夹具必须跟着改（判据 ② 会红）。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr as _;

use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, Op, ParameterValue, SamplerBinding, TrackKind, TrackV3,
    YebanProjectV1,
};

/// 引擎识别鼓机的**五个键位名**（事实源见判据 ②：`drums.rs::note_slot` 的 match 臂）。
const DRUM_KEY_PARAMS: [(&str, f32); 5] = [
    ("kick_note", 36.0),
    ("snare_note", 38.0),
    ("closed_hat_note", 42.0),
    ("open_hat_note", 46.0),
    ("clap_note", 39.0),
];

/// 复音合成器侧的**识别键**（`synth.rs::ToneParams::from_devices` 的 `cutoff_hz`/`cutoff`）。
const POLY_SYNTH_PARAMS: [(&str, f32); 2] = [("cutoff_hz", 1200.0), ("resonance", 0.35)];

/// 确定性的规范 ULID 文本（`01J8ZQ` ＋ 20 位十进制序号，与 `samples::fixture_id` 同形）。
fn id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("规范夹具 ULID")
}

/// 仓库根（`<crate>/../../`）。
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 最小音轨夹具（`TrackV3::default()` 起手，只改身份与种类）。
fn track(track_id: EntityId, kind: TrackKind, name: &str) -> TrackV3 {
    TrackV3 {
        id: track_id,
        name: name.to_owned(),
        kind,
        ..TrackV3::default()
    }
}

/// 脚手架文档：一条主总线轨 ＋ 一条空的 MIDI 轨，**已过** [`YebanProjectV1::validate`]。
///
/// ⚠ `master_bus_track_id` **没有对应的 `Op`**（`crates/yeban-model/src/ops.rs` 里只有
/// `RemoveTrack` 读过它）：所以它由字段直写给出 —— 与 `samples::demo_project` 的建法同形。
/// `tracks` 里的两条轨走 [`Op::AddTrack`]、主总线进 `routing_graph.nodes` 走
/// [`Op::AddRoutingNode`]（`validate` 要求"主总线够得着"，见 `project.rs` 的那条注释）。
fn scaffolding_project(target_id: EntityId) -> YebanProjectV1 {
    let master_id = id(1);
    let mut doc = YebanProjectV1 {
        master_bus_track_id: master_id,
        ..YebanProjectV1::default()
    };
    Op::AddTrack {
        track: track(master_id, TrackKind::Master, "Master"),
    }
    .apply(&mut doc)
    .expect("加主总线轨");
    Op::AddRoutingNode { node: master_id }
        .apply(&mut doc)
        .expect("主总线必须是路由节点");
    Op::AddTrack {
        track: track(target_id, TrackKind::Midi, "Lead"),
    }
    .apply(&mut doc)
    .expect("加 MIDI 目标轨");
    doc.validate().expect("脚手架文档必须合法");
    doc
}

/// 一台**同时**带复音合成器键与完整鼓机映射的内置乐器设备。
///
/// 这正是 §4.1 那句"一台既有 `cutoff_hz`（复音合成器）又有完整鼓机映射的设备"
/// 在模型里的**最小形状**：模型只看见"一条设备链、一台 `InternalInstrument`、六个参数"。
fn multi_source_device(device_id: EntityId) -> DeviceDefinition {
    let mut params: Vec<ParameterValue> = Vec::new();
    for (name, value) in POLY_SYNTH_PARAMS.iter().chain(DRUM_KEY_PARAMS.iter()) {
        params.push(ParameterValue {
            name: (*name).to_owned(),
            value: *value,
            unit: None,
        });
    }
    DeviceDefinition {
        id: device_id,
        name: "Yeban PolySynth".to_owned(),
        // 引擎的规则 1：只有内置乐器会被当成鼓机（`InternalEffect` / `External*` 都不认）。
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params,
        latency_samples: 0,
    }
}

// ---------------------------------------------------------------------------
// ① 模型**允许**同轨多件音源（本单要的那条判据）
// ---------------------------------------------------------------------------

/// **模型不禁止**同一轨既有 `sampler`、又有一台会被引擎当成鼓机的乐器设备。
///
/// 本判据的**断言链**（每一步都必须在场，缺一条就是空跑）：
///
/// 1. 两次 `Op`（[`Op::InsertDevice`] → [`Op::SetTrackSampler`]）的 `precondition` 与
///    `apply` **都**成功；
/// 2. 两条 Op **互不干扰**：插设备不动 `sampler`、绑采样器不动设备链；
/// 3. 读数**同时**成立：`sampler == Some(…)` **且**那台设备仍在（身份逐字、六个参数都在）；
/// 4. 整份文档仍过 `validate()`（模型层**没有**"两者互斥"这类校验）；
/// 5. 序列化往返逐字相等（该组合在持久化形态里**表达得出**）；
/// 6. ⭐ **两条正对照**（只设 `sampler` / 只设设备）各自单独成功 —— 若它们不成立，
///    第 3 步的"同时成立"可能只是断言没生效（第 42 条）。
#[test]
fn model_allows_a_track_to_hold_both_a_sampler_and_a_drum_shaped_instrument() {
    let target_id = id(2);
    let device_id = id(3);
    let device = multi_source_device(device_id);
    let binding = SamplerBinding {
        path: "instruments/kit.sfz".to_owned(),
    };

    // ---- 正路：两次 Op 都被接受 -------------------------------------------------
    let mut doc = scaffolding_project(target_id);

    let insert = Op::InsertDevice {
        track_id: target_id,
        slot_index: 0,
        device: device.clone(),
    };
    assert_eq!(
        insert.precondition(&doc),
        Ok(()),
        "空设备链的第 0 格必须可插入（模型对「几件音源」没有任何前置限制）"
    );
    insert
        .apply(&mut doc)
        .expect("鼓机形状的内置乐器必须被接受");
    assert_eq!(
        doc.track(&target_id).expect("目标轨在文档里").sampler,
        None,
        "`InsertDevice` 只写设备链：⛔ 不得顺手绑上采样器"
    );

    let set_sampler = Op::SetTrackSampler {
        track_id: target_id,
        old_sampler: None,
        new_sampler: Some(binding.clone()),
    };
    assert_eq!(
        set_sampler.precondition(&doc),
        Ok(()),
        "在**已有乐器设备**的轨上绑采样器必须可应用 —— 模型层没有互斥前置条件"
    );
    set_sampler.apply(&mut doc).expect("采样器绑定必须被接受");

    // ---- 读数同时成立 -----------------------------------------------------------
    let after = doc.track(&target_id).expect("目标轨在文档里").clone();
    assert_eq!(after.sampler, Some(binding.clone()), "① 采样器绑定必须非空");
    assert_eq!(
        after.devices,
        vec![device.clone()],
        "② 那台设备必须**原样仍在**（`SetTrackSampler` 不得清空设备链）"
    );
    // ⚠ 先给**下界**再读下标：本仓 `model_isolation.rs` 的
    // `no_unbounded_all_any_assertion_in_this_crate` 常驻判据禁止在无下界的集合上断言
    // （空链会让"仍在"真空通过），这里用 `.len()` 相等把下界写在前面。
    assert_eq!(
        after.devices.len(),
        1,
        "设备链必须恰好一格（否则下面的「身份仍在」会真空通过）"
    );
    assert_eq!(
        after.devices[0].id, device_id,
        "设备身份必须逐字仍在链上（不是被 `SetTrackSampler` 顶掉）"
    );
    let written: BTreeSet<&str> = after.devices[0]
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    for (name, _) in DRUM_KEY_PARAMS {
        assert!(
            written.contains(name),
            "鼓机键位 `{name}` 必须仍在设备参数里"
        );
    }
    for (name, _) in POLY_SYNTH_PARAMS {
        assert!(
            written.contains(name),
            "复音合成器键 `{name}` 必须仍在设备参数里"
        );
    }

    // ---- 整份文档仍合法 ＋ 可往返 ------------------------------------------------
    doc.validate()
        .expect("模型必须接受「同轨 sampler ＋ 鼓机形状乐器」这个组合");
    let json = serde_json::to_string(&doc).expect("这份文档必须可序列化");
    let read_back: YebanProjectV1 = serde_json::from_str(&json).expect("这份文档必须可读回");
    assert_eq!(read_back, doc, "组合形态必须逐字节往返");

    // ---- ⭐ 两条正对照（第 42 条：否则"同时成立"可能是断言没生效）------------------
    let mut only_sampler = scaffolding_project(target_id);
    Op::SetTrackSampler {
        track_id: target_id,
        old_sampler: None,
        new_sampler: Some(binding.clone()),
    }
    .apply(&mut only_sampler)
    .expect("正对照 A：只绑采样器必须单独成功");
    let only_sampler_track = only_sampler.track(&target_id).expect("目标轨在文档里");
    assert_eq!(only_sampler_track.sampler, Some(binding.clone()));
    assert!(
        only_sampler_track.devices.is_empty(),
        "正对照 A：只绑采样器⛔ 不得凭空造出设备（否则第 3 步的「设备仍在」没有分辨力）"
    );

    let mut only_device = scaffolding_project(target_id);
    Op::InsertDevice {
        track_id: target_id,
        slot_index: 0,
        device: device.clone(),
    }
    .apply(&mut only_device)
    .expect("正对照 B：只插设备必须单独成功");
    let only_device_track = only_device.track(&target_id).expect("目标轨在文档里");
    assert_eq!(
        only_device_track.sampler, None,
        "正对照 B：只插设备⛔ 不得顺手绑采样器"
    );
    assert_eq!(only_device_track.devices, vec![device]);
    only_device
        .validate()
        .expect("正对照 B 的文档必须合法（它没有任何采样器绑定）");
}

// ---------------------------------------------------------------------------
// ② 判据 ① 用的五个键位名 ⇔ 引擎源码里 `note_slot` 的 match 臂（读源码，⛔ 不手抄）
// ---------------------------------------------------------------------------

/// 判据 ① 的设备夹具写的五个键位名，**就是**引擎 `drums.rs::note_slot` 认的那五个。
///
/// 为什么要有这一条：模型层**没有**"鼓机"这个概念（`crates/yeban-model/src/` 里
/// `grep -rni drum` 只有 `ops.rs` 的一句注释），"哪台设备会被引擎当成鼓机"完全由
/// `crates/yeban-engine/src/drums.rs` 的**名称约定**决定。若那条约定改名/改形，
/// 判据 ① 会**继续绿**（它只断言两次 `Op` 被接受）却已经不再构造"鼓机形状"的设备
/// ⇒ 本判据就是那道防线：名字表**从源码读**，⛔ 不在这里另抄一份。
///
/// 读不到 `fn note_slot` / 它的 match 臂时**响亮失败**（⛔ 不静默 SKIP）。
#[test]
fn the_five_key_names_are_exactly_the_engines_drum_recognition_names() {
    let path = repo_root()
        .join("crates")
        .join("yeban-engine")
        .join("src")
        .join("drums.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取引擎源码 {} 失败: {error}", path.display()));

    let recognised = drum_names_in_note_slot(&source);
    let fixture: BTreeSet<&str> = DRUM_KEY_PARAMS.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        recognised, fixture,
        "引擎 `drums.rs::note_slot` 认的键位名与判据 ① 的设备夹具不一致 —— \
         要么夹具跟着改，要么引擎的识别约定真的变了（那是 needs-1 要落地的东西）"
    );
}

/// 从 `crates/yeban-engine/src/drums.rs` 的 `fn note_slot` 函数体里抽出全部字符串字面量。
///
/// ⚠ 这**不是**通用 Rust 解析器：它只吃"函数体是一串 `"name" => …` match 臂"这一种
/// 今天真实存在的形状。形状一变（例如改成查表）就**响亮 panic**，逼作者显式更新 ——
/// 与 `tests/schema_sampler_branch.rs` 的 `accepts()` 对未知关键字 panic 同一口径。
fn drum_names_in_note_slot(source: &str) -> BTreeSet<&str> {
    let signature = "fn note_slot(name: &str) -> Option<usize> {";
    let start = source.find(signature).unwrap_or_else(|| {
        panic!("引擎源码里找不到 `{signature}` —— 识别约定已经改形，请显式更新本判据")
    });
    let body = &source[start..];
    let end = body.find("\n}").unwrap_or_else(|| {
        panic!("`fn note_slot` 的函数体没有以顶格 `}}` 收尾 —— 请显式更新本判据")
    });

    let mut names = BTreeSet::new();
    for line in body[..end].lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix('"') else {
            continue;
        };
        let Some(close) = rest.find('"') else {
            panic!("`fn note_slot` 里出现没有收尾引号的字面量：{line}");
        };
        names.insert(&rest[..close]);
    }
    assert!(
        !names.is_empty(),
        "`fn note_slot` 里一个字面量都没抽到 —— 判据 ① 的名字表失去对账对象"
    );
    names
}
