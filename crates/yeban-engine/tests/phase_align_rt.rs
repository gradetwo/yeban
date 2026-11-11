//! `line/phase-align-manual` 的端到端判据：**每轨延时＋极性反转**（`TrackV3` →
//! `TrackParams` → `PhaseBank` → `render_branch`）。手动相位对齐第一层。
//!
//! 本文件与 `tests/sample_delay_insert.rs` 是**同一条链的另一件**：采样延迟
//! 器件挂在插入链里，这里的延时/反转是**轨自身的参数**（`delay_samples` /
//! `invert_polarity`），在 PDC 之后、声相/母线求和之前施加。
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | R0 | **恒等**（`None` / `Some(0)` + `Some(false)`）与无相位设置逐字节相同 | 恒等轨也占槽位 / 反转标志初值写错 |
//! | R1 | 命中的轨**真的武装进实时侧**（`armed_phase_delay(轨)` = `Some((240, true))`）＋槽位 1 ＋ `phase_processed_blocks` 递增 ＋无拒绝 | 快照里有、实时侧没读（`armed_phase_delay` 为 `None` / 计数恒 0） |
//! | R2 | **脉冲延时 N 采样后峰值位置精确移动 N**：武装渲染是未武装渲染的**逐位平移** | 延迟线读错下标 / 延迟量被钳到别的值 / 相位级接在 PDC 之前 |
//! | R3 | **极性反转后信号取反**：只开反转的渲染逐样本等于未武装渲染的相反数 | 反转乘在错误的位置 / 反转只改左声道 |
//! | R4 | **负延时 = 相对提前**：A −240、B 0 与 A 0、B +240 逐位相同，且前 240 帧等于单轨 A（B 未到） | 负值被钳到 0（半个量程变谎言）/ 最小值在逐轨处各算一遍 |
//! | R5 | **确定性**：两次独立装配逐位相同 | 引入真熵源 / 重新武装清空延迟线状态 |
//! | R6 | **主总线不进相位表**：master 上写延时不改变输出 | master 也被当成可对齐的轨 |
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖：`snapshot.rs` 的轨参数投影、`graph.rs` 的 `PhaseBank`
//! 武装与施加、`rt.rs` 的**构造期预分配** / **快照边界武装** / **逐样本调用**。
//! 它**不**覆盖离线链（那是 `yeban-render` 的 `tests/phase_align.rs` 的职责），
//! 也不覆盖采样延迟器件（那是 `tests/sample_delay_insert.rs` 的职责）。

mod support;

use support::{NoteSpec, note_project, render, render_with};
use yeban_model::{EntityId, RoutingEdge, RoutingKind, YebanProjectV1};

/// 判据用的栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本。
const NOTES: [NoteSpec; 1] = [NoteSpec::at(0, 480, 60, 127)];

/// 渲染量子数：200 × 128 = 25_600 帧 = 0.533 s @48 kHz（音符 0.25 s，留尾巴）。
const QUANTA: usize = 200;

/// 给夹具的那条 MIDI 轨写相位参数（**只改夹具**，不改模型层）。
fn set_phase(
    project: &mut YebanProjectV1,
    track: EntityId,
    delay: Option<i32>,
    invert: Option<bool>,
) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.delay_samples = delay;
    entry.invert_polarity = invert;
}

/// 挂相位参数的夹具（延时 `delay` 采样，可选反转）。
fn phased_project(delay: Option<i32>, invert: Option<bool>) -> (YebanProjectV1, EntityId) {
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    set_phase(&mut fixture.project, track, delay, invert);
    (fixture.project, track)
}

/// 两轨夹具：A、B 演**同一个音符**（同一音高/力度/起点），各走独立的声部槽。
/// 返回 `(工程, A, B)`。B 是 A 的完整复制（新摆放 id、新片段 id、新路由边）。
fn two_track_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut fixture = note_project(&NOTES);
    let track_a = fixture.track;
    let master = fixture.master;
    let track_b = EntityId::new();

    let entry_a = fixture
        .project
        .tracks
        .get(&track_a)
        .expect("夹具里必须有那条 MIDI 轨")
        .clone();
    // 片段池条目也复制一份（新 id），摆放指向新片段。
    let (_, old_clip) = fixture
        .project
        .clip_pool
        .iter()
        .next()
        .expect("夹具必须有一个片段");
    let mut new_clip = old_clip.clone();
    let new_clip_id = EntityId::new();
    new_clip.id = new_clip_id;
    fixture.project.clip_pool.insert(new_clip_id, new_clip);
    let mut entry_b = entry_a.clone();
    entry_b.id = track_b;
    entry_b.clips.clear();
    let old_placement = *entry_a.clips.values().next().expect("夹具必须有一个摆放");
    let mut placement = old_placement;
    let new_placement_id = EntityId::new();
    placement.id = new_placement_id;
    placement.clip_id = new_clip_id;
    entry_b.clips.insert(new_placement_id, placement);
    fixture.project.tracks.insert(track_b, entry_b);
    fixture.project.routing_graph.nodes.push(track_b);
    let edge = EntityId::new();
    fixture.project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track_b,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    (fixture.project, track_a, track_b)
}

/// R0：**恒等**相位设置与无相位设置逐字节相同。
#[test]
fn identity_phase_is_bitwise_identical_to_no_phase() {
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA);
    assert!(plain.peak() > 0.0, "夹具必须真的出声");

    // ① 全 None。
    let (project, _) = phased_project(None, None);
    assert_eq!(
        render(&project, QUANTA).fingerprint(),
        plain.fingerprint(),
        "全 None 不得改变输出"
    );
    // ② 显式 Some(0) + Some(false)。
    let (project, _) = phased_project(Some(0), Some(false));
    assert_eq!(
        render(&project, QUANTA).fingerprint(),
        plain.fingerprint(),
        "显式恒等不得改变输出"
    );
}

/// R1：命中的轨**真的武装进实时侧**（读数＋计数）。
#[test]
fn armed_phase_delay_is_readable_and_counted() {
    let (project, track) = phased_project(Some(240), Some(true));
    let mut armed: Option<(usize, bool)> = None;
    let mut slots = usize::MAX;
    let rendered = render_with(&project, QUANTA, 1, |quantum, rig| {
        // 钩子在 `process_quantum` 之前跑：quantum 0 时武装尚未发生
        //（与 `sample_delay_insert.rs` 的 S1 同一时序，读 quantum 1）。
        if quantum == 1 {
            armed = rig.runtime.armed_phase_delay(&track);
            slots = rig.runtime.armed_phase_delay_slot_count();
        }
    });
    assert_eq!(armed, Some((240, true)), "武装读数必须逐字段等于快照投影");
    assert_eq!(slots, 1, "武装槽位必须恰好 1");
    assert_eq!(rendered.stats.phase_unarmed_nodes, 0, "正常武装不得有拒绝");
    assert_eq!(rendered.stats.phase_clamped_frames, 0, "正常武装不得被钳制");
    assert!(
        rendered.stats.phase_processed_blocks > 0,
        "相位池必须真的被调用过（见证计数）"
    );
    assert!(rendered.peak() > 0.0, "夹具必须真的出声");
}

/// R2：延时 N 采样 ⇒ 输出是未武装渲染的**逐位平移**。
#[test]
fn delay_shifts_the_whole_render_by_exactly_n_samples() {
    const DELAY: usize = 240;
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA);
    let (project, _track) = phased_project(Some(DELAY as i32), Some(false));
    let delayed = render(&project, QUANTA);
    assert_eq!(delayed.frames(), plain.frames(), "两边帧数必须相同");
    assert!(
        delayed.left[..DELAY].iter().all(|sample| *sample == 0.0),
        "前 {DELAY} 帧必须是延迟线的初值 0"
    );
    assert_eq!(
        delayed.left[DELAY..]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        plain.left[..plain.frames() - DELAY]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        "延时 {DELAY}：左声道必须是未武装渲染的逐位平移"
    );
    assert_eq!(
        delayed.right[DELAY..]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        plain.right[..plain.frames() - DELAY]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        "延时 {DELAY}：右声道同样逐位平移"
    );
}

/// R3：只开反转 ⇒ 输出逐样本取反。
#[test]
fn invert_only_negates_the_render_sample_by_sample() {
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA);
    let (project, _track) = phased_project(Some(0), Some(true));
    let inverted = render(&project, QUANTA);
    assert_eq!(inverted.frames(), plain.frames());
    for (index, (a, b)) in inverted.left.iter().zip(plain.left.iter()).enumerate() {
        assert_eq!(
            *a, -*b,
            "左声道第 {index} 帧必须取反（-0.0 与 0.0 按值相等）"
        );
    }
    for (index, (a, b)) in inverted.right.iter().zip(plain.right.iter()).enumerate() {
        assert_eq!(*a, -*b, "右声道第 {index} 帧必须取反");
    }
}

/// R4：负延时 = 相对提前（最小值平移）。
///
/// A −240、B 0 与 A 0、B +240 必须逐位相同（两组有效延时都是 0 / 240）；
/// 前 240 帧必须等于单轨 A 的渲染（B 的声音还没到）。
#[test]
fn negative_delay_advances_relatively() {
    const DELAY: usize = 240;
    let (solo_project, _) = phased_project(None, None);
    let solo = render(&solo_project, QUANTA);
    assert!(solo.peak() > 0.0, "单轨夹具必须真的出声");

    let (mut neg_project, track_a, track_b) = two_track_project();
    set_phase(&mut neg_project, track_a, Some(-(DELAY as i32)), None);
    set_phase(&mut neg_project, track_b, Some(0), None);
    let neg = render(&neg_project, QUANTA);

    let (mut pos_project, track_a, track_b) = two_track_project();
    set_phase(&mut pos_project, track_a, Some(0), None);
    set_phase(&mut pos_project, track_b, Some(DELAY as i32), None);
    let pos = render(&pos_project, QUANTA);

    assert_eq!(
        neg.fingerprint(),
        pos.fingerprint(),
        "−240/0 与 0/+240 的有效延时相同（0/240），输出必须逐位相同"
    );
    // ⚠ 这里用**值相等**（`==`），不用 `to_bits()`：两轨求和时 B 的静音
    // scratch 会给 A 的样本再加一次 `+0.0`（`-0.0 + 0.0 == +0.0`，值相等、
    // 位不同）；逐位比较会在 -0.0 上假红。
    assert_eq!(
        neg.left[..DELAY],
        solo.left[..DELAY],
        "前 {DELAY} 帧只有 A 在响，必须等于单轨 A"
    );
    assert_ne!(
        neg.left[DELAY..DELAY + 2400],
        solo.left[DELAY..DELAY + 2400],
        "第 {DELAY} 帧起 B 到了（50 ms 窗口），输出必须与单轨 A 不同"
    );
}

/// R5：两次独立装配逐位相同。
#[test]
fn two_independent_rigs_render_bitwise_identical() {
    let (project, _track) = phased_project(Some(240), Some(true));
    let first = render(&project, QUANTA);
    let second = render(&project, QUANTA);
    assert_eq!(
        first.fingerprint(),
        second.fingerprint(),
        "同一参数两次输出必须逐位一致（确定性）"
    );
}

/// R6：主总线上的延时不改变输出（master 不是可对齐的轨）。
#[test]
fn master_delay_is_ignored() {
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA);
    let mut fixture = note_project(&NOTES);
    let master = fixture.master;
    set_phase(&mut fixture.project, master, Some(240), Some(true));
    assert_eq!(
        render(&fixture.project, QUANTA).fingerprint(),
        plain.fingerprint(),
        "master 的延时/反转不得进入相位池"
    );
}
