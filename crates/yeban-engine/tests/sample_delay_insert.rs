//! `line/phase-align-manual` 的端到端判据：**每轨插入采样延迟**（`crate::insert` → `rt`）。
//! 手动相位对齐第一层。
//!
//! 本文件与 `tests/reverb_insert.rs` 是**同一条链的下一件器件**：
//! 混响之后，采样延迟（前馈整数采样延迟＋极性反转，`yeban_dsp::sample_delay`）。
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | S0 | **没有已识别采样延迟参数**的四种口径（参数为空 / 参数不认识 / `bypassed` / 恒等参数）与"没有设备链"逐字节相同 | 收下 `sample_delay_ms` / 对"没有参数的设备"也挂采样延迟 / bypass 后偷偷取默认参数 / 恒等参数也占槽位 |
//! | S1 | 命中的设备**真的武装进实时侧**（`armed_sample_delay(轨)` 逐字段逐位等于快照投影）＋ `EngineStats::insert_sample_delay_frames` 等于渲染帧数 | 快照里有、实时侧没读（`armed_sample_delay` 为 `None` / 计数恒 0） |
//! | S2 | **脉冲延时 N 采样后峰值位置精确移动 N**：武装渲染的左声道是未武装渲染的**逐位平移**（前 N 帧为 0，其余 `armed[N..] == unarmed[..len-N]`） | 延迟线读错下标 / 延迟量被钳到别的值 / 延迟级接在 PDC 之后 |
//! | S3 | **极性反转后信号取反**：只开反转的渲染逐样本等于未武装渲染的相反数 | 反转乘在错误的位置 / 反转只改左声道 |
//! | S4 | **确定性**：两次独立装配逐位相同 | 引入真熵源 / 重新武装清空延迟线状态 |
//! | S5 | **超容延迟不武装**：初始快照最大延迟 4 ⇒ 池容量 4；后续快照把延迟改成 480 ⇒ 该轨不武装、`insert_sample_delay_capacity_rejects` 累加 1、输出回到未武装的逐位读数 | 在快照边界调用 `configure_with_capacity`（那会在音频线程分配）/ 超容时钳成容量（那会把延迟缩短，听上去对齐了其实没对齐） |
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖：`insert.rs` 的采样延迟投影、`rt.rs` 的**构造期预分配** /
//! **快照边界武装** / **逐样本调用** / **容量守卫**。它**不**覆盖 `yeban-dsp`
//! 内部采样延迟算法的正确性（那是 `crates/yeban-dsp/src/sample_delay.rs`
//! 的单元判据的职责），也不覆盖离线链（那是 `yeban-render` 的
//! `InsertRuntime` 判据的职责）。

mod support;

use support::{NoteSpec, note_project, render, render_with};
use yeban_dsp::sample_delay::SampleDelayParams;
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};

/// 判据用的栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本。
const NOTES: [NoteSpec; 1] = [NoteSpec::at(0, 480, 60, 127)];

/// 渲染量子数：200 × 128 = 25_600 帧 = 0.533 s @48 kHz（音符 0.25 s，留尾巴）。
const QUANTA: usize = 200;

/// 一张效果器设备（`InternalEffect`）。
fn effect(name: &str, params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// 把设备链接到夹具的那条 MIDI 轨上（**只改夹具**，不改模型层）。
fn mount(project: &mut YebanProjectV1, track: EntityId, devices: Vec<DeviceDefinition>) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = devices;
}

/// 挂一台采样延迟设备的夹具（延时 `delay` 采样，可选反转）。
fn delayed_project(delay: f32, invert: f32) -> (YebanProjectV1, EntityId) {
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    mount(
        &mut fixture.project,
        track,
        vec![effect(
            "Sample Delay",
            &[
                ("sample_delay_samples", delay),
                ("sample_delay_invert", invert),
            ],
        )],
    );
    (fixture.project, track)
}

/// S0：**未武装**的四种口径必须与"没有设备链"逐字节相同。
#[test]
fn unarmed_variants_are_bitwise_identical_to_no_device() {
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA);
    assert!(plain.peak() > 0.0, "夹具必须真的出声");

    // ① 参数为空。
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    mount(&mut fixture.project, track, vec![effect("Empty", &[])]);
    let rendered = render(&fixture.project, QUANTA);
    assert_eq!(
        rendered.fingerprint(),
        plain.fingerprint(),
        "参数为空的设备不得改变输出"
    );

    // ② 参数不认识（含 `sample_delay_ms`：毫秒不是本层的单位）。
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    mount(
        &mut fixture.project,
        track,
        vec![effect(
            "Unknown",
            &[("sample_delay_ms", 5.0), ("reverb_mix", 0.5)],
        )],
    );
    let rendered = render(&fixture.project, QUANTA);
    assert_eq!(
        rendered.fingerprint(),
        plain.fingerprint(),
        "不认识的参数名不得产生采样延迟"
    );

    // ③ 旁通。
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    let mut device = effect("Bypassed", &[("sample_delay_samples", 240.0)]);
    device.bypassed = true;
    mount(&mut fixture.project, track, vec![device]);
    let rendered = render(&fixture.project, QUANTA);
    assert_eq!(
        rendered.fingerprint(),
        plain.fingerprint(),
        "旁通的设备不得改变输出"
    );

    // ④ 恒等参数（零延迟、不反转）。
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    mount(
        &mut fixture.project,
        track,
        vec![effect(
            "Identity",
            &[("sample_delay_samples", 0.0), ("sample_delay_invert", 0.0)],
        )],
    );
    let rendered = render(&fixture.project, QUANTA);
    assert_eq!(
        rendered.fingerprint(),
        plain.fingerprint(),
        "恒等参数不得改变输出"
    );
}

/// S1：命中的设备**真的武装进实时侧**（读数＋帧计数）。
#[test]
fn armed_sample_delay_is_readable_and_counted() {
    let (project, track) = delayed_project(240.0, 1.0);
    let mut armed: Option<SampleDelayParams> = None;
    let mut slots = usize::MAX;
    let rendered = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum == 0 {
            armed = rig.runtime.armed_sample_delay(&track);
            slots = rig.runtime.armed_sample_delay_slot_count();
        }
    });
    assert_eq!(
        armed,
        Some(SampleDelayParams {
            delay_samples: 240,
            invert: true,
        }),
        "武装读数必须逐字段等于快照投影"
    );
    assert_eq!(slots, 1, "武装槽位必须恰好 1");
    assert_eq!(
        rendered.stats.insert_sample_delay_frames,
        rendered.frames() as u64,
        "处理帧数必须等于渲染帧数"
    );
    assert_eq!(
        rendered.stats.insert_sample_delay_capacity_rejects, 0,
        "正常武装不得有拒绝"
    );
    assert!(rendered.peak() > 0.0, "夹具必须真的出声");
}

/// S2：延时 N 采样 ⇒ 输出是未武装渲染的**逐位平移**。
#[test]
fn delay_shifts_the_whole_render_by_exactly_n_samples() {
    const DELAY: usize = 240;
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA);
    let (project, _track) = delayed_project(DELAY as f32, 0.0);
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

/// S3：只开反转 ⇒ 输出逐样本取反。
#[test]
fn invert_only_negates_the_render_sample_by_sample() {
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA);
    let (project, _track) = delayed_project(0.0, 1.0);
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

/// S4：两次独立装配逐位相同。
#[test]
fn two_independent_rigs_render_bitwise_identical() {
    let (project, _track) = delayed_project(240.0, 1.0);
    let first = render(&project, QUANTA);
    let second = render(&project, QUANTA);
    assert_eq!(
        first.fingerprint(),
        second.fingerprint(),
        "同一参数两次输出必须逐位一致（确定性）"
    );
}

/// S5：超容延迟不武装（容量守卫），输出回到未武装。
#[test]
fn a_delay_beyond_pool_capacity_is_rejected_not_clamped() {
    // 初始快照最大延迟 4 ⇒ 池容量 4。
    let (project, track) = delayed_project(4.0, 0.0);
    let mut rejects = 0u64;
    let mut slots_after_shift = usize::MAX;
    let mut frames_after_shift = 0u64;
    let mut scratch = [0.0f32; 128 * 2];

    let rendered = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum == 12 {
            // 发布一份延迟 480 的快照：超过池容量 4 ⇒ 不武装。
            let mut shifted = project.clone();
            mount(
                &mut shifted,
                track,
                vec![effect("Sample Delay", &[("sample_delay_samples", 480.0)])],
            );
            rig.publish_equivalent(&shifted, 2);
        }
        if quantum == 16 {
            let before = rig.runtime.stats().insert_sample_delay_frames;
            // 再推一个量子：这一份快照里采样延迟必须**整段不武装**。
            rig.runtime.process_quantum(&mut scratch, 2);
            let after = rig.runtime.stats();
            frames_after_shift = after.insert_sample_delay_frames.saturating_sub(before);
            slots_after_shift = rig.runtime.armed_sample_delay_slot_count();
            rejects = after.insert_sample_delay_capacity_rejects;
        }
    });

    println!(
        "[engine-sample-delay/S5] 超容: 拒绝累计={rejects} 武装槽位={slots_after_shift} 该量子新增处理帧数={frames_after_shift}；总渲染帧={}",
        rendered.frames()
    );
    assert_eq!(rejects, 1, "超容必须恰好累加 1 次拒绝");
    assert_eq!(slots_after_shift, 0, "超容之后采样延迟必须整段不武装");
    assert_eq!(
        frames_after_shift, 0,
        "不武装的量子不许推进'采样延迟处理帧数'"
    );
    // 输出回到未武装的逐位读数：闭包里在第 16 个量子多推了一个量子 ⇒
    // 第 16 个量子之后的每一段都比未武装渲染**晚一个量子**（合成器状态只与
    // 量子序号有关，与墙钟无关 ⇒ 错一位比较仍是逐位证据）。
    let baseline = note_project(&NOTES);
    let plain = render(&baseline.project, QUANTA + 1);
    let segment = 100 * 128;
    assert_eq!(
        rendered.left[segment..segment + 128]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        plain.left[segment + 128..segment + 256]
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        "超容之后中段必须回到未武装的逐位读数（错一位）"
    );
}
