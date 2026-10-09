//! 引擎侧**采样率转发**的运行期判据（`line/engine-27`）。
//! [ARCH-DSP-001, ARCH-RT-001, ARCH-DET-001, MUST-GATE-001]
//!
//! ## 这一票量的是哪一件事
//!
//! 快照里的 `audio_config.sample_rate` 是**唯一**的采样率事实源。它必须到达每一个
//! "按采样率算系数"的器件：平滑器（`α`，含 `exp`）、声部池（相位递增量与 `ADSR`
//! 系数）、通道条（EQ/滤波器/动态级）、鼓机、电平池弹道、走带栅格。
//! 少转发一处，那个器件就在**旧采样率**的口径下工作 —— 输出不会 panic、不会
//! 非有限、也不会让"逐位相同"那类判据变红，只是**物理量错了一倍**
//! （音高、时间常数、截止频率）。
//!
//! ## 量法（先说指标，再给数）
//!
//! 数 `crates/yeban-engine/src` 里含 `set_sample_rate` 的**命中行数**
//! （量法：`git grep -c set_sample_rate HEAD -- crates/yeban-engine/src`，
//! 单位：行）。`origin/main`（`461c9cb`）上的读数：
//!
//! | 文件 | 命中行数 | 其中**跨器件转发**的调用点 |
//! | :--- | ---: | :--- |
//! | `src/rt.rs` | 11 | `params.set_sample_rate` / `bank.set_quanta_per_second` / `strip.set_sample_rate` / `reverb_pool[..].set_sample_rate` / `conv_pool` 的采样率入口 |
//! | `src/param.rs` | 9 | 表内转发（逐轨槽 + 主总线槽） |
//! | `src/insert.rs` | 5 | 全部是**文档提及**，不是调用点 |
//! | `src/synth.rs` | 5 | `slot.synth.set_sample_rate` / `drums[..].set_sample_rate`（两处） |
//! | `src/drums.rs` | 1 | **文档提及**，不是调用点 |
//! | 合计 | **31** | —— |
//!
//! 本文件把其中四条**可观测**的转发钉成判据（另两条已有的判据见下）。
//! 剩下的入口（混响 / 卷积混响的换率）在采用"整段不武装 + 计数"的显式拒绝口径
//! （`insert_reverb_rate_rejects` / `insert_convolution_rejects`），由
//! `tests/reverb_insert.rs` / `tests/convolution_insert.rs` 覆盖。
//!
//! ## 判据表
//!
//! | 编号 | 转发点 | 物理量（单位） | 怎么变红（注入） |
//! | :--- | :--- | :--- | :--- |
//! | F1 | 快照采样率 → `ParamTable`（逐轨平滑器 `α`） | 一阶低通的**剩余量**（线性幅度）在同一批样本数之后满足 `r₉₆² = r₄₈·(T−1)` | 把 `params.set_sample_rate(current.sample_rate())` 写死 `48_000.0`（实测变红） |
//! | F2 | 快照采样率 → 声部池 | 音高（Hz，零交叉率折算） | `begin_snapshot` 的采样率实参写死 `48_000`，或 `TrackSlot::empty(48_000)`（实测变红） |
//! | F3 | 换率时**在册**声部槽位的 `PolySynth::set_sample_rate` | 换率之后的整段样本**位模式**（相对"从头就是新率"的参照） | 删掉 `slot.synth.set_sample_rate(sample_rate_u32)`（实测变红） |
//! | F4 | 换率时**在册**通道条槽位的 `ChannelStrip::set_sample_rate` | 换率之后的整段样本**位模式**（同上） | 删掉 `strip.set_sample_rate(sample_rate)`（实测变红） |
//! | F5 | 换率时**在册**鼓机槽位的 `DrumMachine::set_sample_rate` | 换率之后的整段样本**位模式**（同上） | 删掉 `drums[index].set_sample_rate(sample_rate_u32)`（`else if sample_rate_changed` 那一支，实测变红） |
//! | F6 | **率不变**的快照里**新出现**的轨道的声部槽位 | 切换之后的整段样本**位模式**（相对“刚出现时就带上它”的参照） | `TrackSlot::empty(sample_rate_u32)` 的实参写死 `48_000`（实测变红） |
//! | F7 | **在册**（已武装）的逐轨槽与主总线槽在快照换率时必须重算 `α` | 换率之后每个量子的**平滑器输出序列**（线性幅度，逐位；与"一开始就在新率"逐位相同、与"率没变"逐位不同） | `ParamTable::set_sample_rate` 的 `self.gains[..self.len]` 改成 `self.gains[..0]`，或删掉 `self.master.set_sample_rate(sample_rate)`（均实测变红，见该判据自己的文档） |
//!
//! F1 / F6 与 F7 的分界：前两条测的是**槽位建立那一刻**的采样率（`accept` 惰性建槽、
//! `begin_snapshot` 在率不变时新建槽），F7 测的是**已经在册**（已武装、可能正在自动化中）
//! 的槽位在快照换率时有没有被重算。`tests/param_automation.rs` 的 P8 只断言
//! "换率之后收敛到目标"，而 48 kHz 与 96 kHz 的 `α` 在 400 个量子里都会吸附
//! ⇒ P8 对 `α` **不敏感**；P13 与 `src/param.rs` 单元判据 ⑧ 覆盖的是主总线槽位
//! **还没武装**时的那条顺序。⚠ 与 F3/F4/F5 不同，F7 比较的是**平滑器输出序列**
//! （不是音频样本）：换率之后同一份工程的音频本来就按采样率变化，序列才是可比的量。
//!
//! ## F3/F4/F5 的构造（为什么"换率臂"必须与"从头就是新率"的臂逐位相同）
//!
//! 三臂都是**切换前的窗口里一个音源都不出声**（音符起点取 `NOTE_TICK`，
//! 96 kHz 下落在第 1250 个量子，远在切换点 `SWITCH_QUANTUM` 之后）：
//!
//! - 采样率切换**不改变任何器件的时间状态**（喂的是静音 ⇒ 延迟线的环、滤波器
//!   的积分器、包络的当前值都是零/初值，与采样率无关）；
//! - 因此切换点上两条臂的**状态**相同、**系数**应当在重算之后相同
//!   ⇒ 切换之后的样本必须**逐位**相同。
//!
//! 若某个器件的系数没有跟着新率重算，它的**物理**行为就变了
//! （96 kHz 下仍按 48 kHz 的系数 ⇒ 时间常数减半、截止频率减半、音高翻倍），
//! 逐位相同立刻被打破。
//!
//! 覆盖度见证（缺了它们，"逐位相同"可能是"两边都没走到那个器件"）：
//! ① `snapshot_switches == 1`（切换真的发生）；
//! ② `notes_triggered == 1`（音符真的在切换之后被触发）；
//! ③ 与"全程 48 kHz、不切换"的对照臂在切换窗口上**必须不同**（切换真的改了音频）。
//!
//! ⚠ 本文件的字面常量只有采样率、tick 与量子数（无浮点期望值）：
//! 所有断言都是"两臂互相比较"或"由契约代数推出的等式"，不含本机 libm 的读数。

mod support;

use std::sync::Arc;

use support::{NoteSpec, note_project, render, render_with, two_track_project, zero_crossings};
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_engine::meter::meter_channel;
use yeban_engine::param::{MASTER_GAIN_SLOT, TRACK_GAIN_SLOT};
use yeban_engine::ring::{EngineEvent, ParamAddress, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, SampleRate, YebanProjectV1,
};

/// 切换发生的量子下标（切换**在渲染该量子之前**发布 ⇒ 该量子已经是新率）。
const SWITCH_QUANTUM: usize = 40;

/// F7 的观察窗（量子）：96 kHz 上一条 5 ms 斜坡要走 ≈ 42 个量子才吸附 ⇒ 10 个量子
/// 一定仍在走；而 48 kHz 的 `α` 在同一窗里已经走得远得多（F7 的判别力见证 ③）。
const F7_WINDOW: usize = 10;

/// 每臂渲染的量子数：96 kHz 下 2000 × 128 = 256 000 帧。
const QUANTA: usize = 2_000;

/// 音符起点（tick）：96 kHz 下 = `3200 × 50 = 160 000` 帧 = 第 1250 个量子
/// ⇒ 落在切换点之后很远，且窗内一直在响。
const NOTE_TICK: u64 = 3_200;

/// 采样率不同的两份**同内容**工程（实体身份逐位相同 ⇒ 槽位不会换主人）。
fn at_rate(project: YebanProjectV1, rate: SampleRate) -> YebanProjectV1 {
    let mut project = project;
    project.audio_config.sample_rate = rate;
    project
}

/// 一张效果器设备（`InternalEffect`），参数名走 `crate::insert` 的识别表。
fn effect(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Strip".to_owned(),
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
    }
}

/// 把设备链接到夹具的那条 MIDI 轨上（**只改夹具**）。
fn mount(project: &mut YebanProjectV1, track: EntityId, devices: Vec<DeviceDefinition>) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = devices;
}

/// 一条**鼓机**设备（五个鼓件的键位映射都在）。
fn drum_kit() -> DeviceDefinition {
    let params: [(&str, f32); 5] = [
        ("kick_note", 36.0),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
    ];
    DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
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
    }
}

/// 切换窗口的第一帧下标（新率从这一帧开始）。
const SWITCH_FRAME: usize = SWITCH_QUANTUM * DEFAULT_BLOCK_FRAMES;

/// 一条"48 kHz 起跑、在第 [`SWITCH_QUANTUM`] 个量子切到 96 kHz"的臂。
///
/// `at_rate` 已经把音符起点放在切换之后 ⇒ 切换窗口里两条臂的状态都是初值。
fn switched_render(project48: &YebanProjectV1, project96: &YebanProjectV1) -> support::Render {
    let target = project96.clone();
    render_with(project48, QUANTA, 1, move |quantum, rig| {
        if quantum == SWITCH_QUANTUM {
            rig.publish_equivalent(&target, 2);
        }
    })
}

/// 逐位比较两段样本，**只在第一处不同点**给消息（整段 `assert_eq!` 的输出是几十万个数）。
fn assert_bit_identical(label: &str, got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len(), "{label}: 长度必须相等");
    for (index, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        if g.to_bits() != w.to_bits() {
            panic!(
                "{label}: 第 {index} 个样本位模式不同（{:#010x} vs {:#010x}）；总样本数 {}",
                g.to_bits(),
                w.to_bits(),
                got.len()
            );
        }
    }
}

/// 三条"换率重算"判据的公共收尾：覆盖度见证 + 逐位契约 + 反面对照。
fn assert_rate_switch_is_transparent(
    label: &str,
    reference: &support::Render,
    switched: &support::Render,
    no_switch_48k: &support::Render,
) {
    assert_eq!(
        switched.stats.snapshot_switches, 1,
        "{label}: 覆盖度 —— 快照必须**真的**被切换过一次（否则本判据是空转）"
    );
    assert_eq!(
        reference.stats.snapshot_switches, 0,
        "{label}: 参照臂不得切换过快照"
    );
    assert_eq!(
        reference.stats.notes_triggered, 1,
        "{label}: 覆盖度 —— 音符必须在切换**之后**被触发（恰好一次）"
    );
    assert_eq!(
        switched.stats.notes_triggered, 1,
        "{label}: 切换臂同样必须触发恰好一次音符"
    );
    assert!(
        reference.stats.rendered_samples > NOTE_TICK * 25,
        "{label}: 覆盖度 —— 渲染窗口必须真的跨过音符起点"
    );
    assert!(
        switched.left[SWITCH_FRAME..] != no_switch_48k.left[SWITCH_FRAME..],
        "{label}: 反面对照 —— 切换窗口上'切到 96 kHz'必须与'全程 48 kHz'不同，\
         否则下面的逐位相同是空转"
    );
    let compared = reference.left.len() - SWITCH_FRAME;
    assert_bit_identical(
        &format!("{label} 左声道（{compared} 个样本）"),
        &switched.left[SWITCH_FRAME..],
        &reference.left[SWITCH_FRAME..],
    );
    assert_bit_identical(
        &format!("{label} 右声道（{compared} 个样本）"),
        &switched.right[SWITCH_FRAME..],
        &reference.right[SWITCH_FRAME..],
    );
    println!(
        "[engine-rate/{label}] 切换后逐位相同=true 对照帧数={compared} \
         切换前量子数={SWITCH_QUANTUM} 音符起点tick={NOTE_TICK}"
    );
}

// ---------------------------------------------------------------------------
// F1：快照采样率 → 参数目标表（平滑器 α）
// ---------------------------------------------------------------------------

/// 一条只用来发 `SetParam` 并读回平滑器输出的装配。
struct ParamRig {
    sender: yeban_engine::ring::EventSender,
    runtime: EngineRuntime,
    output: Vec<f32>,
    _slot: Arc<SnapshotSlot>,
    _queue: yeban_engine::snapshot::RetireQueue,
}

impl ParamRig {
    fn new(project: &YebanProjectV1) -> Self {
        let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具工程必须能编译");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, queue) = retire_channel(64);
        let (sender, receiver) = event_channel(256);
        let (publisher, _collector) = meter_channel(65_536);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Self {
            sender,
            runtime,
            output: vec![0.0f32; DEFAULT_BLOCK_FRAMES * 2],
            _slot: slot,
            _queue: queue,
        }
    }

    /// 发布一份**等价**快照（只改采样率与修订号）⇒ 触发音频线程的快照边界。
    ///
    /// 与 `support::Runtime::publish_equivalent` 同义；本文件用的是本地 `ParamRig`
    /// （它留住了事件生产端），所以自己带一份。
    fn publish_equivalent(&self, project: &YebanProjectV1, revision: u64) {
        self._slot
            .publish(EngineSnapshot::from_project(project, revision).expect("等价快照"));
    }
}

/// 跑一条臂：在第一个量子之后发一条 `SetParam`（目标 2.0），再跑 `quanta` 个量子，
/// 返回 `(平滑器剩余量 2 − 输出值, 武装的每秒量子数)`。
fn run_gain_ramp(rate: SampleRate, quanta: usize) -> (f32, Option<f32>) {
    let fixture = note_project(&[NoteSpec::quarter(60)]);
    let track = fixture.track;
    let project = at_rate(fixture.project.clone(), rate);
    let mut rig = ParamRig::new(&project);
    // 第 1 个量子：走完快照边界（采样率在这里武装进参数表）。
    rig.runtime.process_quantum(&mut rig.output, 2);
    let armed = rig.runtime.stats().quanta_per_second;
    let event = EngineEvent::SetParam {
        target: ParamAddress::new(track, TRACK_GAIN_SLOT),
        value: 2.0,
    };
    rig.sender.publish(&[event]);
    for _ in 0..quanta {
        rig.runtime.process_quantum(&mut rig.output, 2);
    }
    let gain = rig
        .runtime
        .armed_param_gain(&track)
        .expect("SetParam 必须建立逐轨槽位");
    (2.0 - gain, armed)
}

/// **F1**：快照的采样率必须到达参数目标表 ⇒ 同一批**样本数**之后，96 kHz 的
/// 平滑器剩余量必须是 48 kHz 的平方根（`r₉₆² = r₄₈ · (T − 1)`）。
///
/// 为什么不比"墙钟时间"：一阶低通按**样本**递推，`α` 是采样率的函数；
/// 两条臂跑同样多的量子（= 同样多的样本），96 kHz 的那条走在更短的时间里
/// ⇒ 它离目标更远。把这条关系写成代数式就不需要任何绝对时间常量。
#[test]
fn the_param_smoother_follows_the_snapshot_sample_rate() {
    // 目标 2.0（T − 1 = 1）；5 个量子 = 640 个样本。
    // 48 kHz：q^640 = exp(−640/240) ≈ 0.0695；96 kHz：exp(−640/480) ≈ 0.2636。
    const QUANTA_AFTER_EVENT: usize = 5;
    let (r48, armed48) = run_gain_ramp(SampleRate::Hz48000, QUANTA_AFTER_EVENT);
    let (r96, armed96) = run_gain_ramp(SampleRate::Hz96000, QUANTA_AFTER_EVENT);

    assert_eq!(armed48, Some(375.0), "覆盖度：48 kHz 臂的每秒量子数");
    assert_eq!(armed96, Some(750.0), "覆盖度：96 kHz 臂的每秒量子数");
    assert!(
        r48 > 0.0 && r48 < 1.0 && r96 > 0.0 && r96 < 1.0,
        "覆盖度：两臂都必须向目标靠拢了一部分（r48={r48} r96={r96}）—— \
         全未动或全到位都会让下面的平方关系失去判别力"
    );
    assert!(
        r96 > r48,
        "96 kHz 在同样多的样本里走得更慢 ⇒ 剩余量必须更大：r48={r48} r96={r96}"
    );
    let lhs = f64::from(r96) * f64::from(r96);
    let rhs = f64::from(r48) * 1.0;
    let tolerance = 0.02 * rhs.abs() + 1e-4;
    assert!(
        (lhs - rhs).abs() <= tolerance,
        "F1：r₉₆² 必须等于 r₄₈·(T−1)（采样率必须转发到参数表）：\
         r48={r48} r96={r96} ⇒ r96²={lhs} 与 r48·(T−1)={rhs} 相距 {}（容差 {tolerance}）",
        (lhs - rhs).abs()
    );
    println!("[engine-rate/F1] r48={r48} r96={r96} r96²={lhs} r48·(T−1)={rhs}");
}

// ---------------------------------------------------------------------------
// F2：快照采样率 → 声部池（音高）
// ---------------------------------------------------------------------------

/// **F2**：96 kHz 的快照必须让声部池按 96 kHz 递推相位 ⇒ A4 音符仍是 440 Hz。
///
/// 量什么：左声道在 400 个量子（51 200 帧）上的**零交叉数**（单位：次）。
/// 一个周期两次零交叉 ⇒ 期望值 = `2 × f × 窗口秒数`。判据同时跑 48 kHz 的
/// 对照臂（同一个数），因为"本判据钉的是 Hz，不是 48 kHz 的那一次读数"。
#[test]
fn the_voice_pool_gets_the_snapshot_sample_rate() {
    let fixture = note_project(&[NoteSpec::at(0, 7_680, 69, 127)]); // A4 = 440 Hz
    let project96 = at_rate(fixture.project.clone(), SampleRate::Hz96000);
    let project48 = fixture.project.clone();

    let rendered96 = render(&project96, 400);
    let rendered48 = render(&project48, 400);

    let expected = |rate: f64| 2.0 * 440.0 * (rendered96.frames() as f64 / rate);
    let crossings96 = zero_crossings(&rendered96.left);
    let crossings48 = zero_crossings(&rendered48.left);
    let expected96 = expected(96_000.0);
    let expected48 = expected(48_000.0);

    assert!(
        rendered96.peak() > 0.0 && rendered48.peak() > 0.0,
        "覆盖度：两臂都必须真的出声"
    );
    assert!(
        (crossings96 as f64 - expected96).abs() <= 5.0,
        "F2：96 kHz 下 A4 必须是 440 Hz ⇒ 零交叉数应 ≈ {expected96:.1}，\
         实际 {crossings96}（按 48 kHz 递推相位会得到 ≈ {:.1}）",
        crossings96 as f64 * 2.0
    );
    assert!(
        (crossings48 as f64 - expected48).abs() <= 5.0,
        "对照臂：48 kHz 下同一个音符也应 ≈ {expected48:.1}，实际 {crossings48}"
    );
    println!(
        "[engine-rate/F2] 96k crossings={crossings96} (期望 {expected96:.1})；\
         48k crossings={crossings48} (期望 {expected48:.1})"
    );
}

// ---------------------------------------------------------------------------
// F3：换率时在册声部槽位的系数重算
// ---------------------------------------------------------------------------

/// **F3**：48 kHz 起跑的声部槽位在切到 96 kHz 时必须重算系数 ⇒
/// 切换之后的样本与"从头就是 96 kHz"逐位相同。
#[test]
fn a_mid_run_rate_change_retunes_the_armed_voice_slots() {
    let fixture = note_project(&[NoteSpec::at(NOTE_TICK, 7_680, 69, 127)]);
    let project48 = fixture.project.clone();
    let project96 = at_rate(project48.clone(), SampleRate::Hz96000);

    let reference = render(&project96, QUANTA);
    let switched = switched_render(&project48, &project96);
    let control = render(&project48, QUANTA);
    assert_rate_switch_is_transparent("F3", &reference, &switched, &control);
}

// ---------------------------------------------------------------------------
// F4：换率时在册通道条槽位的系数重算
// ---------------------------------------------------------------------------

/// **F4**：通道条挂在同一条轨上、机器从 48 kHz 换到 96 kHz 时，
/// `ChannelStrip::set_sample_rate` 必须被调用 ⇒ 切换之后的样本逐位相同。
///
/// 参数取**三级都开**（EQ ＋ 低通 ＋ 动态级）：每一级的系数都是采样率的函数，
/// 因此任何一级漏转发都会打破逐位相同。
#[test]
fn a_mid_run_rate_change_recalibrates_the_armed_channel_strip() {
    let fixture = note_project(&[NoteSpec::at(NOTE_TICK, 1_920, 60, 127)]);
    let track = fixture.track;
    let mut project48 = fixture.project.clone();
    mount(
        &mut project48,
        track,
        vec![effect(&[
            ("eq_low_gain", 6.0),
            ("eq_low_freq", 180.0),
            ("cutoff_hz", 500.0),
            ("resonance", 0.5),
            ("threshold_db", -24.0),
            ("ratio", 4.0),
        ])],
    );
    let project96 = at_rate(project48.clone(), SampleRate::Hz96000);

    let reference = render(&project96, QUANTA);
    let switched = switched_render(&project48, &project96);
    let control = render(&project48, QUANTA);

    assert!(
        reference.stats.insert_strip_frames > 0,
        "覆盖度：通道条必须真的处理过帧（否则本判据是空转）"
    );
    assert_eq!(
        switched.stats.insert_strip_frames, reference.stats.insert_strip_frames,
        "覆盖度：切换臂与参照臂的通道条处理帧数必须相同"
    );
    assert_rate_switch_is_transparent("F4", &reference, &switched, &control);
}

// ---------------------------------------------------------------------------
// F5：换率时在册鼓机槽位的系数重算
// ---------------------------------------------------------------------------

/// **F5**：鼓机挂在同一条轨上、机器从 48 kHz 换到 96 kHz 时，
/// `DrumMachine::set_sample_rate` 必须被调用 ⇒ 切换之后的样本逐位相同。
///
/// 音符起点在切换之后 ⇒ 击打发生在**重算之后**：包络三段系数与鼓件滤波器系数
/// 若仍是 48 kHz 的，衰减时间就短一半，逐位相同立刻被打破。
#[test]
fn a_mid_run_rate_change_recalibrates_the_armed_drum_slots() {
    let fixture = note_project(&[NoteSpec::at(NOTE_TICK, 480, 36, 127)]);
    let track = fixture.track;
    let mut project48 = fixture.project.clone();
    mount(&mut project48, track, vec![drum_kit()]);
    let project96 = at_rate(project48.clone(), SampleRate::Hz96000);

    let reference = render(&project96, QUANTA);
    let switched = switched_render(&project48, &project96);
    let control = render(&project48, QUANTA);

    assert_eq!(
        reference.stats.drum_hits, 1,
        "覆盖度：鼓机必须真的被敲响一次（音符在切换之后）"
    );
    assert_eq!(
        switched.stats.drum_hits, 1,
        "覆盖度：切换臂同样必须敲响一次"
    );
    assert_rate_switch_is_transparent("F5", &reference, &switched, &control);
}

/// 把一条轨**整条**移走（轨表 / 路由节点 / 路由边三处一起），
/// 用来制造"某一份快照里多出一条轨"的形态。
fn without_track(project: &YebanProjectV1, track: EntityId) -> YebanProjectV1 {
    let mut project = project.clone();
    project.tracks.remove(&track);
    project.routing_graph.nodes.retain(|node| *node != track);
    project
        .routing_graph
        .edges
        .retain(|_, edge| edge.source_node != track && edge.destination_node != track);
    project
}

/// **F6**：采样率**没有变化**的那一份快照里**新出现**的轨道，它的声部槽位也必须
/// 按快照的采样率建立。
///
/// 为什么单独立一条：`SynthEngine::begin_snapshot` 只在 `sample_rate_changed` 时
/// 才对**全部在册槽位**调 `PolySynth::set_sample_rate`；新槽位是**惰性**建立的
/// （`TrackSlot::empty(sample_rate_u32)`）。两件事叠起来 ⇒ 若那个实参写死 `48_000`，
/// 而在**同一采样率**的两份快照之间新增了一条轨（`sample_rate_changed == false`
/// ⇒ 那一轮重算根本不跑），新槽位就会永远停在 48 kHz。
/// 本票注入实测：`TrackSlot::empty(48_000)` 在全量 24 个目标里全绿 —— 既有夹具
/// 从不在"采样率不变"的快照之间新增轨。
///
/// 构造：起始快照**整条移走**第二轨（它的槽位因此要到切换那一份快照才建立），
/// 两份快照的采样率都是 96 kHz（唯一差别是"多出一条轨"）。
#[test]
fn a_slot_created_at_an_unchanged_rate_uses_the_snapshot_sample_rate() {
    let (project_full, first, second) =
        two_track_project(&[], &[NoteSpec::at(NOTE_TICK, 7_680, 69, 127)]);
    let project_full = at_rate(project_full, SampleRate::Hz96000);
    assert_ne!(first, second, "夹具的两条轨必须是不同的身份");
    let project_one = without_track(&project_full, second);
    assert!(
        !project_one.tracks.contains_key(&second),
        "起始快照必须真的不含第二轨"
    );
    assert_eq!(
        project_one.audio_config.sample_rate, project_full.audio_config.sample_rate,
        "两份快照的采样率必须相同（本判据测的是'率不变时新建的槽位'）"
    );

    let reference = render(&project_full, QUANTA);
    let switched = switched_render(&project_one, &project_full);
    let control = render(&project_one, QUANTA);

    assert_eq!(
        switched.stats.snapshot_switches, 1,
        "覆盖度：快照必须**真的**被切换过一次"
    );
    assert_eq!(
        reference.stats.notes_triggered, 1,
        "覆盖度：音符必须在切换之后被触发（恰好一次）"
    );
    assert_eq!(
        switched.stats.notes_triggered, 1,
        "覆盖度：切换臂同样触发一次"
    );
    assert!(
        switched.left[SWITCH_FRAME..] != control.left[SWITCH_FRAME..],
        "反面对照：新轨出现之后音频必须真的改变，否则下面的逐位相同是空转"
    );
    assert_bit_identical(
        "F6 左声道（率不变时新建的槽位）",
        &switched.left[SWITCH_FRAME..],
        &reference.left[SWITCH_FRAME..],
    );
    assert_bit_identical(
        "F6 右声道（率不变时新建的槽位）",
        &switched.right[SWITCH_FRAME..],
        &reference.right[SWITCH_FRAME..],
    );
    println!("[engine-rate/F6] 率不变时新建槽位：切换后逐位相同=true");
}

/// 两个 `f32` 序列是否**逐位**相同（`+0.0` 与 `-0.0` 算不同）。
fn bitwise_equal(left: &[f32], right: &[f32]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(l, r)| l.to_bits() == r.to_bits())
}

/// **F7**：**已经在册**的槽位（逐轨槽与主总线槽）在快照换采样率时必须重算 `α`
/// —— 也就是与"一开始就在新采样率"的同类装配**逐位相同**。
///
/// ## 与 F1 / F6 的区别（为什么必须单独立一条）
///
/// * F1 的槽位是**换率之后**才惰性建立的（`accept` 里的
///   `ParamSmoother::new(self.sample_rate, …)` 直接取当时的新率）；
/// * F6 测的是"率不变时新建的槽位"；
/// * `tests/param_automation.rs` 的 P8 只断言换率之后平滑器**收敛到目标**
///   —— 48 kHz 与 96 kHz 的 `α` 在 400 个量子里都会吸附，所以 P8 对 `α` 不敏感；
/// * P13 与 `src/param.rs` 的单元判据 ⑧ 覆盖的是**主总线槽位在换率时还没武装**
///   的那条顺序。
///
/// ⇒ "**在册**（已经武装）的槽位在换率时重不重算 `α`"此前没有任何判据。
/// 本票（engine-28）注入实测（`ParamTable::set_sample_rate` 的
/// `self.gains[..self.len]` 改成 `self.gains[..0]`，主总线那一行不动）：
/// `tests/sample_rate_forwarding.rs` 与全量 24 个目标**全绿**。后果：一条已经在
/// 自动化中的轨在 44.1/48 → 96 kHz 之后仍按旧率走 5 ms（96 kHz 下变成 10 ms 的斜坡）。
///
/// ## 构造（三臂状态逐位相同，只差"换率时 `α` 有没有被重算"）
///
/// | 步骤 | 三臂共同 | A（换率） | B（参照） | C（判别力见证） |
/// | :--- | :--- | :--- | :--- | :--- |
/// | 1 | 在 `start_rate` 上装配，发两条**恒等**（`1.0`）目标 ⇒ 建槽位/武装但不碰样本 | 48 kHz | 96 kHz | 48 kHz |
/// | 2 | 第 0 个量子消费它们（此刻 value == target == 1.0） | | | |
/// | 3 | 发布一份 `switch_to` 采样率的快照（修订 2）⇒ 快照边界的 `set_sample_rate` | 96 kHz | 96 kHz | 48 kHz |
/// | 4 | 发两条**非恒等**目标，跑 [`F7_WINDOW`] 个量子，逐量子读平滑器输出 | | | |
///
/// 步骤 1–2 之后三臂的**状态**逐位相同（都是恒等、都已吸附），因此步骤 4 的增益
/// 序列只取决于 `α`：A 与 B 必须逐位相同，而 C（率从未变过）必须与 B 不同 ——
/// 后者是本判据"真的对 `α` 敏感"的机械证据。
#[test]
fn armed_slots_follow_a_mid_life_sample_rate_change() {
    const WINDOW: usize = F7_WINDOW;
    const TRACK_TARGET: f32 = 0.25;
    const MASTER_TARGET: f32 = 0.7;

    /// 一条臂的读数。
    struct ArmedSeries {
        /// 每个量子的逐轨槽平滑输出（线性幅度）。
        track: Vec<f32>,
        /// 每个量子的主总线槽平滑输出（线性幅度）。
        master: Vec<f32>,
        /// 换率**之前**在册的逐轨槽位数。
        slots_before: usize,
        /// 换率**之前**主总线槽的平滑输出（未武装时是 `None`）。
        master_before: Option<f32>,
    }

    /// 跑一条臂：`start_rate` 是构造采率，`switch_to` 是第 1 个量子之前发布的
    /// 快照采率（可以与 `start_rate` 相同 = "率没变"）。
    fn arm(start_rate: SampleRate, switch_to: SampleRate) -> ArmedSeries {
        let fixture = note_project(&[NoteSpec::at(0, 7_680, 69, 127)]);
        let track = fixture.track;
        let master = fixture.master;
        let project = at_rate(fixture.project.clone(), start_rate);
        let switched = at_rate(project.clone(), switch_to);
        let mut rig = ParamRig::new(&project);

        // 1) 两条恒等目标 ⇒ 建槽位 / 武装主总线，但一个样本都不改（恒等快路径）。
        rig.sender.publish(&[
            EngineEvent::SetParam {
                target: ParamAddress::new(track, TRACK_GAIN_SLOT),
                value: 1.0,
            },
            EngineEvent::SetParam {
                target: ParamAddress::new(master, MASTER_GAIN_SLOT),
                value: 1.0,
            },
        ]);
        // 2) 第 0 个量子：事件边界消费它们，三臂状态逐位相同（1.0 / 1.0）。
        rig.runtime.process_quantum(&mut rig.output, 2);
        let slots_before = rig.runtime.armed_param_slot_count();
        let master_before = rig.runtime.armed_master_param_gain();

        // 3) 快照边界：只有 A 的这一份快照换了采样率。
        rig.publish_equivalent(&switched, 2);
        rig.runtime.process_quantum(&mut rig.output, 2);

        // 4) 非恒等目标 ⇒ 三臂同时开始一条 5 ms 的斜坡，只有 `α` 可能不同。
        rig.sender.publish(&[
            EngineEvent::SetParam {
                target: ParamAddress::new(track, TRACK_GAIN_SLOT),
                value: TRACK_TARGET,
            },
            EngineEvent::SetParam {
                target: ParamAddress::new(master, MASTER_GAIN_SLOT),
                value: MASTER_TARGET,
            },
        ]);
        let mut track_series = Vec::with_capacity(WINDOW);
        let mut master_series = Vec::with_capacity(WINDOW);
        for _ in 0..WINDOW {
            rig.runtime.process_quantum(&mut rig.output, 2);
            track_series.push(
                rig.runtime
                    .armed_param_gain(&track)
                    .expect("逐轨槽位必须在册"),
            );
            master_series.push(
                rig.runtime
                    .armed_master_param_gain()
                    .expect("主总线槽位必须已武装"),
            );
        }
        ArmedSeries {
            track: track_series,
            master: master_series,
            slots_before,
            master_before,
        }
    }

    let switched = arm(SampleRate::Hz48000, SampleRate::Hz96000);
    let fresh = arm(SampleRate::Hz96000, SampleRate::Hz96000);
    let unchanged = arm(SampleRate::Hz48000, SampleRate::Hz48000);

    // ---- 覆盖度 ①：两条被测臂的槽位在换率**之前**就已在册（这是与 F1 的分界）----
    for (label, arm) in [("A(48→96)", &switched), ("B(96→96)", &fresh)] {
        assert_eq!(
            arm.slots_before, 1,
            "{label}: 逐轨槽位必须已在册（否则测的是惰性建立那一路，属 F1）"
        );
        assert_eq!(
            arm.master_before,
            Some(1.0),
            "{label}: 主总线槽位必须已武装且是恒等值（否则测的是 P13 那一路）"
        );
    }
    // ---- 覆盖度 ②：参照臂在窗口末尾仍在平滑中（吸附会把 `α` 的差别吃掉）----
    let last_track = *fresh.track.last().expect("窗口非空");
    let last_master = *fresh.master.last().expect("窗口非空");
    assert!(
        last_track > TRACK_TARGET && last_track < 1.0,
        "覆盖度：参照臂的逐轨斜坡必须仍在走（实得 {last_track}）"
    );
    assert!(
        last_master > MASTER_TARGET && last_master < 1.0,
        "覆盖度：参照臂的主总线斜坡必须仍在走（实得 {last_master}）"
    );

    // ---- 主判据：换率的那条臂必须与"一开始就在新率"逐位相同 ----
    assert_bit_identical(
        "F7 逐轨槽平滑输出（换率 vs 一开始就在新率）",
        &switched.track,
        &fresh.track,
    );
    assert_bit_identical(
        "F7 主总线槽平滑输出（换率 vs 一开始就在新率）",
        &switched.master,
        &fresh.master,
    );

    // ---- 覆盖度 ③（判别力见证）：率**没变**的那条臂必须给出另一个轨迹 ----
    assert!(
        !bitwise_equal(&unchanged.track, &fresh.track),
        "率没变的那条臂必须与 96 kHz 参照不同，否则本判据对 `α` 不敏感（逐轨首值 {} vs {}）",
        unchanged.track[0],
        fresh.track[0]
    );
    assert!(
        !bitwise_equal(&unchanged.master, &fresh.master),
        "率没变的那条臂的主总线轨迹必须与 96 kHz 参照不同（首值 {} vs {}）",
        unchanged.master[0],
        fresh.master[0]
    );
    println!(
        "[engine-rate/F7] 在册槽位换率：逐轨 {} 个量子、主总线 {} 个量子逐位相同；\
         48 kHz 判别力见证首值 逐轨 {} vs 参照 {}",
        switched.track.len(),
        switched.master.len(),
        unchanged.track[0],
        fresh.track[0]
    );
}
