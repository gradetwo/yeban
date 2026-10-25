//! `Op::SetClipPlacementLoop` 的模型侧判据（任务单 `#336`）。
//!
//! 这个变体补的是 `#335` 实测报告里排第一的那堵墙：
//! `ClipPlacement::loop_config` **只在创建时**能给，改一个已有摆放的循环只能
//! "删掉摆放再重建"，而那会**换掉摆放身份**。
//!
//! 四条判据（音频那两条在 `yeban-mcp` 的 `set_clip_placement_loop.rs`）：
//!
//! | # | 判据 | 钉住什么 |
//! | :-- | :--- | :--- |
//! | 1 | [`setting_a_loop_round_trips_the_whole_loop_config`] | 单遍摆放经本变体改成循环后，文档**逐字段**等于请求的 `LoopConfig` |
//! | 2 | [`stale_old_loop_config_is_rejected_and_leaves_the_placement_unchanged`] | 旧值**只从文档读**；陈旧载荷 ⇒ `OpStateMismatch`，且文档一个字节都没变 |
//! | 3 | [`each_of_the_three_loop_fields_changes_independently`] | `enabled` / `start_tick` / `end_tick` 各自生效、各自可区分于"没变" |
//! | 4 | [`undo_restores_the_previous_loop_config_exactly`] | 结构逆换成旧配置 ⇒ 逐字节复原 |
//!
//! 外加一条**合法性镜像**判据
//! [`set_loop_accepts_exactly_what_add_accepts_and_rejects_what_add_rejects`]：
//! 本变体复用 `add` 的同一个 `LoopConfig::validate`，⛔ 不另立第二套"合法循环"。
//!
//! ## 一条被证伪的前提（如实登记）
//!
//! 任务单原文说 "`placement` 的 `add` 形态接受任何东西"。**实测不是**：
//! [`ClipPlacement::validate`] 调 [`LoopConfig::validate`]，后者对
//! `enabled && end_tick <= start_tick` 报 `ZeroDuration` ⇒ `add` **会**拒绝
//! "启用但区间为空/反向"的配置。它接受的是**其余**退化形态：
//! `enabled == false`（哪怕 `end <= start`）、`end >= duration_ticks`（含恰好覆盖）。
//! 因此"镜像 `add`"= **调用同一个 `validate`**，而不是"什么都收"。
//! 下面的镜像判据把两半都钉住（同收 + 同拒）。

use std::collections::BTreeMap;
use std::str::FromStr;

use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, ModelError, Op,
    TrackKind, TrackV3, YebanProjectV1,
};

/// 构造确定性的规范 ULID 文本（与 crate 内夹具同一口径）。
fn id(index: u128) -> EntityId {
    EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
}

/// 摆放跨度（tick）：两个 960 PPQ 的四分音符 = 半小节（`D = 1920`）。
const DURATION_TICKS: u64 = 1_920;

/// 夹具的五个身份。
struct Fixture {
    track: EntityId,
    clip: EntityId,
    placement: EntityId,
}

/// 一份**没有摆放**的合法文档：主总线 + 一条 MIDI 轨 + 一条带一个音符的片段。
fn fixture() -> (YebanProjectV1, Fixture) {
    let master = id(1);
    let track = id(2);
    let clip = id(10);
    let placement = id(50);
    let note = id(20);

    let mut doc = YebanProjectV1 {
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    doc.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    doc.tracks.insert(
        track,
        TrackV3 {
            id: track,
            name: "Lead".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(note, MidiNote::new(note, 0, 60, 240));
    doc.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Lead Clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    (
        doc,
        Fixture {
            track,
            clip,
            placement,
        },
    )
}

/// 摆放载荷（身份 / 片段 / 起点 / 跨度 / 循环配置 / 静音）。
fn placement(fixture: &Fixture, loop_config: LoopConfig) -> ClipPlacement {
    ClipPlacement {
        id: fixture.placement,
        clip_id: fixture.clip,
        start_tick: 0,
        duration_ticks: DURATION_TICKS,
        loop_config,
        muted: false,
    }
}

/// 用 `add` 把一条摆放放到夹具音轨上（这是 `#241` 起唯一的**创建**通路）。
fn add_placement(doc: &mut YebanProjectV1, fixture: &Fixture, loop_config: LoopConfig) {
    Op::AddClipPlacement {
        track_id: fixture.track,
        placement: placement(fixture, loop_config),
    }
    .apply(doc)
    .expect("夹具摆放必须可放置");
}

/// 当前文档里那条摆放的循环配置。
fn loop_config(doc: &YebanProjectV1, fixture: &Fixture) -> LoopConfig {
    doc.tracks[&fixture.track]
        .clips
        .get(&fixture.placement)
        .expect("夹具摆放存在")
        .loop_config
}

/// 当前文档里那条摆放的循环展开计划（**行为**读数，不是字段读数）。
fn repeat(doc: &YebanProjectV1, fixture: &Fixture) -> Option<yeban_model::LoopRepeat> {
    doc.tracks[&fixture.track]
        .clips
        .get(&fixture.placement)
        .expect("夹具摆放存在")
        .loop_repeat()
}

fn looping(start_tick: u64, end_tick: u64) -> LoopConfig {
    LoopConfig {
        enabled: true,
        start_tick,
        end_tick,
    }
}

/// 判据 1：单遍摆放经本变体改成循环后，文档**逐字段**等于请求的 `LoopConfig`。
///
/// 量什么：`query` 侧读出的 `ClipPlacement::loop_config` 三项（bool / tick / tick），
/// 与请求值逐项相等；同时 `loop_repeat()` 的 `iterations` 从 `None` 变成 `4`
/// —— 证明写进去的不只是三个字段，行为也跟着变了。
#[test]
fn setting_a_loop_round_trips_the_whole_loop_config() {
    let (mut doc, fixture) = fixture();
    // 单遍：`add` 的缺省循环配置（关闭 / 0..0）。
    add_placement(&mut doc, &fixture, LoopConfig::default());
    assert_eq!(
        repeat(&doc, &fixture),
        None,
        "单遍摆放（循环关闭）必须没有展开计划"
    );

    let requested = looping(0, 480);
    let op = Op::SetClipPlacementLoop {
        track_id: fixture.track,
        placement_id: fixture.placement,
        old_loop_config: LoopConfig::default(),
        new_loop_config: requested,
    };
    op.apply(&mut doc).expect("设置循环必须成功");

    assert_eq!(
        loop_config(&doc, &fixture),
        requested,
        "文档里的循环配置必须**逐字段**等于请求值"
    );
    let plan = repeat(&doc, &fixture).expect("启用 0..480 的循环必须真的展开");
    assert_eq!(plan.start_tick, 0);
    assert_eq!(plan.end_tick, 480);
    assert_eq!(plan.period_ticks, 480);
    assert_eq!(
        plan.iterations, 4,
        "D=1920 / 周期 480 ⇒ N = ceil(1920 / 480) = 4 遍"
    );

    // 序列化形状与 `LoopConfig` 的三种字段一一对应（⛔ 不是内部私有表示）。
    let json = serde_json::to_value(&doc).expect("文档可序列化");
    let stored = &json["tracks"][fixture.track.to_canonical_string()]["clips"]
        [fixture.placement.to_canonical_string()]["loop_config"];
    assert_eq!(
        stored,
        &serde_json::json!({"enabled": true, "start_tick": 0, "end_tick": 480}),
        "落盘形状必须是 `LoopConfig` 的三个字段"
    );
}

/// 判据 2：陈旧 `old_loop_config` ⇒ `OpStateMismatch`，且摆放**逐字节不变**。
///
/// 量什么：错误变体（`ModelError` 的变体名 + `eq`）；错误前后同一条摆放的
/// `serde_json` 字节（UTF-8 字节串逐字节相等 ⇒ 长度与内容都不变）。
///
/// 注入（实测红）：把前置条件里的 `current.loop_config != *old_loop_config`
/// 改成 `false`（或不比较）⇒ 陈旧载荷会被接受，`apply` 返回 `Ok`、文档被改 ⇒ 本判据红。
#[test]
fn stale_old_loop_config_is_rejected_and_leaves_the_placement_unchanged() {
    let (mut doc, fixture) = fixture();
    add_placement(&mut doc, &fixture, LoopConfig::default());

    let before = serde_json::to_vec(&doc).expect("文档可序列化");
    let before_placement = serde_json::to_vec(
        doc.tracks[&fixture.track]
            .clips
            .get(&fixture.placement)
            .expect("摆放存在"),
    )
    .expect("摆放可序列化");

    // 调用方声称旧配置是 0..960，而文档现值是缺省 0..0。
    let stale = Op::SetClipPlacementLoop {
        track_id: fixture.track,
        placement_id: fixture.placement,
        old_loop_config: looping(0, 960),
        new_loop_config: looping(0, 480),
    };
    let error = stale.apply(&mut doc).expect_err("陈旧旧值必须被拒绝");
    assert_eq!(
        error,
        ModelError::OpStateMismatch {
            op: "SetClipPlacementLoop"
        }
    );

    let after = serde_json::to_vec(&doc).expect("文档可序列化");
    assert_eq!(before, after, "被拒的操作不得改变文档的任何字节");
    let after_placement = serde_json::to_vec(
        doc.tracks[&fixture.track]
            .clips
            .get(&fixture.placement)
            .expect("摆放存在"),
    )
    .expect("摆放可序列化");
    assert_eq!(
        before_placement, after_placement,
        "目标摆放必须逐字节不变（⛔ 不许'先写再审'）"
    );
    assert_eq!(
        loop_config(&doc, &fixture),
        LoopConfig::default(),
        "循环配置必须仍是旧值"
    );
}

/// 判据 3：三个字段**各自**生效，且每一处改动都**可区分于"没变"**。
///
/// 量什么（每行三列）：改后配置 vs 基准配置的**字段差异集合**（恰好一个）、
/// `loop_repeat()` 的行为读数（`Some` 的 `iterations` 或 `None`）、以及
/// "改后 ≠ 基准"这条可区分性。
///
/// 基准：`{enabled: true, start: 480, end: 960}`，`D = 1920`
/// ⇒ 周期 480、`N = ceil((1920-480)/480) = 3`。
///
/// 注入（实测红）：
/// - `commit` 里把 `placement.loop_config = *new_loop_config` 改成只写
///   `enabled`（或只写 `start`、只写 `end`）⇒ 对应那一行的"恰好一个字段不同"红；
/// - `structural_inverse` 里不交换任何字段不影响本条，但"不变"对照会发现某一列没变。
#[test]
fn each_of_the_three_loop_fields_changes_independently() {
    let base = looping(480, 960);

    // 三列：标签、目标新配置、期望发生变化的字段、期望的行为读数。
    let cases: [(&str, LoopConfig, &str, Option<u64>); 3] = [
        (
            "只翻 enabled",
            LoopConfig {
                enabled: false,
                start_tick: 480,
                end_tick: 960,
            },
            "enabled",
            None,
        ),
        ("只改 start_tick", looping(0, 960), "start_tick", Some(2)),
        ("只改 end_tick", looping(480, 1_440), "end_tick", Some(2)),
    ];

    for (label, new, changed_field, expected_iterations) in cases {
        let (mut doc, fixture) = fixture();
        add_placement(&mut doc, &fixture, base);
        assert_eq!(
            repeat(&doc, &fixture).map(|plan| plan.iterations),
            Some(3),
            "{label}: 基准必须是 N=3"
        );

        Op::SetClipPlacementLoop {
            track_id: fixture.track,
            placement_id: fixture.placement,
            old_loop_config: base,
            new_loop_config: new,
        }
        .apply(&mut doc)
        .unwrap_or_else(|error| panic!("{label} 必须成功: {error}"));

        let got = loop_config(&doc, &fixture);
        assert_eq!(got, new, "{label}: 三项必须整体等于新配置");
        assert_ne!(got, base, "{label}: 改动必须可区分于'没变'");

        // 差异集合**恰好一个**字段 —— 另外两个字段必须原样不动。
        let mut differing = Vec::new();
        if got.enabled != base.enabled {
            differing.push("enabled");
        }
        if got.start_tick != base.start_tick {
            differing.push("start_tick");
        }
        if got.end_tick != base.end_tick {
            differing.push("end_tick");
        }
        assert_eq!(
            differing,
            vec![changed_field],
            "{label}: 只有 `{changed_field}` 该变（⛔ 不是连带改别的字段）"
        );

        assert_eq!(
            repeat(&doc, &fixture).map(|plan| plan.iterations),
            expected_iterations,
            "{label}: 行为读数（展开遍数 / 无计划）必须随之改变"
        );
    }
}

/// 判据 4：撤销把循环配置**逐字节**还原。
///
/// 量什么：`apply` 前后同一条摆放的 `serde_json` UTF-8 字节串（逐字节相等），
/// 以及 `invert()` 产出的逆操作载荷（`old_loop_config == 新值`、
/// `new_loop_config == 旧值`）。
///
/// 注入（实测红）：把 `structural_inverse` 里的两个字段改成同一侧（例如都写
/// `old_loop_config: *old_loop_config`）⇒ `apply_inverse` 会把配置"还原"成新值
/// ⇒ 字节比较红。
#[test]
fn undo_restores_the_previous_loop_config_exactly() {
    let (mut doc, fixture) = fixture();
    let original = looping(480, 960);
    add_placement(&mut doc, &fixture, original);
    let before = serde_json::to_vec(
        doc.tracks[&fixture.track]
            .clips
            .get(&fixture.placement)
            .expect("摆放存在"),
    )
    .expect("摆放可序列化");

    let op = Op::SetClipPlacementLoop {
        track_id: fixture.track,
        placement_id: fixture.placement,
        old_loop_config: original,
        new_loop_config: looping(0, 480),
    };
    op.apply(&mut doc).expect("设置循环");
    assert_eq!(loop_config(&doc, &fixture), looping(0, 480));

    let inverse = op.invert(&doc).expect("逆操作必须可构造");
    let Op::SetClipPlacementLoop {
        old_loop_config,
        new_loop_config,
        ..
    } = &inverse
    else {
        panic!("本变体的逆必须仍是本变体，实际 {inverse:?}");
    };
    assert_eq!(*old_loop_config, looping(0, 480), "逆操作的旧值 = 新值");
    assert_eq!(*new_loop_config, original, "逆操作的新值 = 原值");

    op.apply_inverse(&mut doc).expect("撤销必须成功");
    let after = serde_json::to_vec(
        doc.tracks[&fixture.track]
            .clips
            .get(&fixture.placement)
            .expect("摆放存在"),
    )
    .expect("摆放可序列化");
    assert_eq!(before, after, "撤销必须把摆放还原到逐字节相同");
    assert_eq!(
        loop_config(&doc, &fixture),
        original,
        "循环配置必须逐字段回到旧值"
    );
}

/// **合法性镜像**：本变体与 `add` 的接受集合逐字相同（不另立第二套合法循环）。
///
/// 这一条同时纠正任务单里那句"`add` 接受任何东西"：`add` 会拒绝
/// `enabled && end <= start`（共享的 [`LoopConfig::validate`]）。本变体调用**同一个**
/// 函数 ⇒ 同收、同拒。
///
/// 量什么：每条配置在两条通路上的结果（`Ok(配置)` / `Err(变体名)`）必须相等；
/// 被接受到的那几条，两条通路写进文档的 `loop_config` 还必须逐字段相等。
///
/// 注入（实测红）：在 `precondition` 里去掉 `new_loop_config.validate()?`
/// ⇒ 最后一条（`enabled` 且 `end == start`）在本变体上变成 `Ok`，与 `add` 的
/// `Err(ZeroDuration)` 不等 ⇒ 红。
#[test]
fn set_loop_accepts_exactly_what_add_accepts_and_rejects_what_add_rejects() {
    // (标签, 配置, 是否被共享校验接受)
    let cases: [(&str, LoopConfig, bool); 4] = [
        ("恰好覆盖 (end == D)", looping(0, DURATION_TICKS), true),
        ("越过末端 (end > D)", looping(0, DURATION_TICKS + 960), true),
        (
            "关闭且 end <= start",
            LoopConfig {
                enabled: false,
                start_tick: 960,
                end_tick: 480,
            },
            true,
        ),
        ("启用且 end == start", looping(960, 960), false),
    ];

    for (label, config, accepted) in cases {
        // 通路 A：`add`。
        let (mut via_add, add_fixture) = fixture();
        let added = Op::AddClipPlacement {
            track_id: add_fixture.track,
            placement: placement(&add_fixture, config),
        }
        .apply(&mut via_add);

        // 通路 B：本变体（从"缺省关闭"的单遍摆放出发）。
        let (mut via_loop, loop_fixture) = fixture();
        add_placement(&mut via_loop, &loop_fixture, LoopConfig::default());
        let placed = Op::SetClipPlacementLoop {
            track_id: loop_fixture.track,
            placement_id: loop_fixture.placement,
            old_loop_config: LoopConfig::default(),
            new_loop_config: config,
        }
        .apply(&mut via_loop);

        assert_eq!(
            added.is_ok(),
            accepted,
            "{label}: `add` 的接受性必须等于共享校验的裁决"
        );
        assert_eq!(
            placed.is_ok(),
            accepted,
            "{label}: 本变体的接受性必须与 `add` 相同（⛔ 不另立第二套）"
        );

        if accepted {
            assert_eq!(
                loop_config(&via_add, &add_fixture),
                config,
                "{label}: `add` 写进文档的配置"
            );
            assert_eq!(
                loop_config(&via_loop, &loop_fixture),
                config,
                "{label}: 本变体写进文档的配置必须与 `add` 逐字段相同"
            );
        } else {
            assert_eq!(
                added.expect_err("应被拒"),
                ModelError::ZeroDuration,
                "{label}: `add` 的拒绝理由"
            );
            assert_eq!(
                placed.expect_err("应被拒"),
                ModelError::ZeroDuration,
                "{label}: 本变体的拒绝理由必须与 `add` 相同"
            );
            assert_eq!(
                loop_config(&via_loop, &loop_fixture),
                LoopConfig::default(),
                "{label}: 被拒后文档必须仍是旧配置"
            );
        }
    }
}
