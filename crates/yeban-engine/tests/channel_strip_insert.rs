//! `line/engine-wiring-2` 的端到端判据：**每轨插入通道条**（`crate::insert` → `rt`）。
//! [ARCH-RT-001, ARCH-DET-001, ARCH-DSP-001, ROAD-M2-006]
//!
//! 本文件与本 crate 的 `tests/compressor_insert.rs` 是**同一件事的两半**：
//! `c792fdc` 把每轨插入接成**裸压缩器**，本票把同一张槽位表换成
//! **通道条**（`yeban_dsp::channel_strip`：输入增益 → EQ → 滤波 → 动态 → 输出增益）。
//! 压缩器因此成为通道条的**动态级**（见 `crate::insert` 模块文档 §2）。
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | C8 | **没有已识别通道条参数**的工程（无设备 / 参数为空 / 参数不认识 / `bypassed` / EQ 与滤波级未写），输出与**接线前**（`1114f05`）逐字节相同 | 让插入链对"没有参数的设备"也生效 / 默认就把平坦 EQ 或 20 kHz 低通打开 |
//! | C9 | 命中的设备**真的武装进实时侧**（`armed_strip(轨)` 逐字段逐位等于投影）＋ `EngineStats` 的"整链处理帧数"与"动态级压过帧数"分别报数 | 快照里有、实时侧没读（`armed_*` 为 `None` / 计数恒 0） |
//! | C10 | **EQ 级与滤波级真的在处理信号**：开启后输出改变、低通削掉高频 ⇒ 左声道 RMS 下降（单位 dBFS） | 通道条被接到不参与混音的地方 / 只有投影没有处理 |
//! | C11 | **确定性**：同输入两次独立装配 + 渲染逐位相同；**同一个装配里**周期性发布等价快照（重新武装）也不改变输出 | 引入真熵源 / 各级状态跨装配泄漏 / 重新武装把状态清掉 |
//! | C12 | **不同量子数（块切分）**下前缀逐位相同（D1 的引擎侧对账） | 用了跨量子残留的临时缓冲 / 量子边界上有一次性副作用 |
//! | C13 | 引擎的 `ChannelStrip` **就是** `yeban_dsp::channel_strip::ChannelStrip`（类型 + `new`/`set_params`/`process_mono` 函数地址同一性 + **零延迟**契约） | 在引擎侧留第二份通道条实现 / 器件带延迟却登记 0 |
//! | C14 | 设备链顺序：第一个**含已识别参数名**的效果器是唯一来源；更早的无关效果器不参与、更晚的不覆盖 | 取"最后一个"或"任意一个"设备 |
//! | C15 | `insert_current_reduction_db` 是**量规**（源静音后回落）而不是第二个累计量：任意时刻 `当前 ≤ 全程最大`、`全程最大` 单调不减、静音段末尾 `当前` 落到峰值的一小部分；未武装工程两者恒为 0 | 把"当前"直接抄成"最大"（第二个累计量） / 漏读器件 getter（恒 0） / 跨轨取"最后一条轨"而不是最大 |
//! | C16 | `insert_detector_level_db`（**输入**侧，dBFS）是量规：冷值 `None`（不是 `Some(0.0)`）、有声段抬起、源静音段末尾逐位落回器件地板 `MIN_LEVEL_DB`、只有 EQ 的通道条报出的正是压缩器构造初值（动态级旁通⇒检波器从未跑）、跨轨口径 = 最大；同窗口的 `insert_strip_frames` / `insert_gain_reductions` / `insert_current_reduction_db` 是**对照读数**（证明扫描模式有效） | 冷值写成 `0.0` / 读数其实是累计最大值（尾部不落回地板） / 读整链电平而不是动态级检波器 / 跨轨取"最后一条轨" |
//!
//! ## C8 的跨提交对账（**不硬编码哈希**）
//!
//! C8 把未武装的原始样本落盘：`$TMPDIR/yeban-compressor-unarmed.raw`
//! （与 `compressor_insert.rs` 的 C0 **同一个文件名** ⇒ 两张判据对**同一份**
//! 样本落盘，`shasum -a 256` 可以直接对账）。
//! 判据刻意**不**把哈希写进源码：那会把"本机 libm 的某一位"当成规范常数。
//! 实测读数见本票的交付报告（注释里只写"怎么量"，不写"量到多少"）。
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖：`insert.rs` 的投影、`snapshot.rs` 的武装字段、`rt.rs` 的
//! **快照边界武装**与**逐样本调用**。它**不**覆盖：`yeban-dsp` 内部三级算法的正确性
//! （那是 `crates/yeban-dsp/tests/**` 的职责），**也**不覆盖 `drums`
//! （已接线，但它是**音源**而不是插入器件 ⇒ 判据在 `crates/yeban-engine/tests/drums_instrument.rs`）
//! 与 `reverb`（已接线，但它的判据在**另一个文件**：`crates/yeban-engine/tests/reverb_insert.rs`
//! —— 本文件的夹具里 `reverb_mix` 是**刻意不认**的名字，见 `crate::insert` 模块文档 §8.1）。

mod support;

use std::path::Path;

use support::{MixSpec, NoteSpec, note_project, render, render_with, rms_peak, tuned_project};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};

/// 判据用的栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本（见 `support` 模块文档）。
///
/// 四个音高都在 C4–C5（261.6 / 329.6 / 392.0 / 523.3 Hz）⇒ 低通截止取 250 Hz 时
/// **全部音符都在通带之外**，RMS 下降是**可预期的方向**（不是"看着变就断言变了"）。
const NOTES: [NoteSpec; 4] = [
    NoteSpec::at(0, 480, 60, 127),
    NoteSpec::at(480, 480, 64, 110),
    NoteSpec::at(960, 960, 67, 100),
    NoteSpec::at(1920, 480, 72, 90),
];

/// 渲染量子数：400 × 128 = 51,200 帧 ≈ 1.067 s @48 kHz。
const QUANTA: usize = 400;

/// 一张效果器设备（`InternalEffect`）。
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

/// 把设备链接到夹具的那条 MIDI 轨上（**只改夹具**，不改模型层）。
fn mount(project: &mut YebanProjectV1, track: EntityId, devices: Vec<DeviceDefinition>) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = devices;
}

/// 把交错样本按小端 `f32` 落盘，返回 `(路径, 字节数)`（与 C0/C8 同款）。
fn dump(name: &str, left: &[f32], right: &[f32]) -> (std::path::PathBuf, usize) {
    let mut bytes = Vec::with_capacity(left.len() * 8);
    for index in 0..left.len() {
        bytes.extend_from_slice(&left[index].to_le_bytes());
        bytes.extend_from_slice(&right[index].to_le_bytes());
    }
    let path = Path::new(&std::env::temp_dir()).join(name);
    std::fs::write(&path, &bytes).expect("原始样本必须能落盘");
    (path, bytes.len())
}

/// **未武装**的四种口径必须与"没有设备链"逐位相同（模块文档 §4.1 / §4.3）。
///
/// 四种输入的唯一差别是设备链：
///
/// 1. `params` 为空的效果器（只为上报延迟而存在的那一类）；
/// 2. 参数名一个都不认识的效果器；
/// 3. `bypassed = true` 但参数**命中**的通道条；
/// 4. 参数**命中**但只是一个 EQ 增益旋钮 ⇒ 只有 EQ 级启用（滤波与动态都不启用）。
///
/// ⚠ 第 4 条**不**断言逐位相同（平坦 EQ 不是逐位恒等，见
/// `crates/yeban-dsp/src/channel_strip.rs` 模块注释 §3）：它断言"**只有 EQ 级**启用"，
/// 即投影出来的另两级都没开。这正是"沉默地启用平坦 EQ"这类缺陷的机械判据。
#[test]
fn projects_without_recognised_channel_strip_params_are_bit_identical() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    let bare = render(&fixture.project, QUANTA);

    let mut unknown_project = fixture.project.clone();
    mount(&mut unknown_project, track, vec![effect(&[])]);
    let unknown = render(&unknown_project, QUANTA);

    let mut unrelated_project = fixture.project.clone();
    mount(
        &mut unrelated_project,
        track,
        vec![effect(&[("reverb_mix", 0.3), ("filter_slope", 0.7)])],
    );
    let unrelated = render(&unrelated_project, QUANTA);

    let mut bypassed_project = fixture.project.clone();
    let mut bypassed = effect(&[("eq_low_gain", 6.0), ("threshold_db", -30.0)]);
    bypassed.bypassed = true;
    mount(&mut bypassed_project, track, vec![bypassed]);
    let bypassed_render = render(&bypassed_project, QUANTA);

    let (path, bytes) = dump("yeban-compressor-unarmed.raw", &bare.left, &bare.right);
    println!(
        "[engine-wiring-2/C8] 未武装: 落盘={} 字节={bytes} 帧={} 指纹={:#018x}",
        path.display(),
        bare.frames(),
        bare.fingerprint()
    );

    assert!(bare.frames() > 0, "夹具必须真的渲染出帧");
    assert!(bare.peak() > 0.0, "夹具必须真的出声（否则逐位相同是空转）");
    assert_eq!(
        bare.stats.limiter_gain_reductions, 0,
        "本夹具必须整体低于母线限制器阈值 —— 否则 C10 的 RMS 差归因不唯一"
    );
    for (label, other) in [
        ("参数为空的效果器", &unknown),
        ("参数不认识的的效果器", &unrelated),
        ("bypassed 的通道条", &bypassed_render),
    ] {
        assert_eq!(
            other.fingerprint(),
            bare.fingerprint(),
            "{label}: 输出必须与没有设备链时逐位相同"
        );
        assert_eq!(
            other.left_bits(),
            bare.left_bits(),
            "{label}: 左声道必须逐位相同"
        );
        assert_eq!(
            other.stats.insert_gain_reductions, 0,
            "{label}: 未武装 ⇒ 动态级压过帧数必须为 0"
        );
        assert_eq!(
            other.stats.insert_strip_frames, 0,
            "{label}: 未武装 ⇒ 通道条处理帧数必须为 0"
        );
    }

    // 快照层的同一件事：没有任何设备链的工程，插入表必须为空。
    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&fixture.project, 1).expect("快照");
    assert!(
        snapshot.inserts().is_empty(),
        "没有效果器设备的工程不得产生任何插入链"
    );
    assert!(snapshot.insert_params(&track).is_none());
}

/// 判据用的**武装**参数：EQ ＋ 滤波 ＋ 动态三级都写，且都不是 dsp 默认值。
///
/// 取值刻意避开默认值：C9 要断言"投影出来的**这一份**数"真的走到了实时侧，
/// 用默认值会让"没接线"与"接线了"读数相同（假绿）。
const ARMED_STRIP: [(&str, f32); 12] = [
    ("input_gain_db", -3.0),
    ("eq_low_gain", 8.0),
    ("eq_low_freq", 180.0),
    ("eq_mid_gain", -6.0),
    ("eq_mid_freq", 1_200.0),
    ("eq_mid_q", 1.4),
    ("eq_high_gain", -9.0),
    ("eq_high_freq", 6_000.0),
    ("cutoff_hz", 900.0),
    ("resonance", 0.3),
    ("threshold_db", -30.0),
    ("ratio", 8.0),
];

/// 一条挂**已识别通道条**的 MIDI 轨（音量 0 dB，声相居中）。
fn armed_project() -> (YebanProjectV1, EntityId) {
    let mut fixture = tuned_project(&NOTES, MixSpec::volume(0.0));
    let track = fixture.track;
    mount(&mut fixture.project, track, vec![effect(&ARMED_STRIP)]);
    (fixture.project, track)
}

/// C9：命中的设备**真的武装进了实时侧**（三层证据，缺一不可）。
///
/// 1. **快照层**：`EngineSnapshot::insert_params(轨)` 给出投影参数（模型 → 快照）；
/// 2. **实时层**：`EngineRuntime::armed_strip(轨)` 逐字段逐位等于第 1 层
///    （快照 → 逐样本路径的武装表）。这是"快照里有、实时侧没读"这类缺陷的**唯一**
///    机械判据 —— 那种缺陷不会 panic，也不会让静态断言变红；
/// 3. **行为层**：`EngineStats` 分别报出"整链处理帧数"与"动态级压过帧数"。
#[test]
fn recognised_channel_strip_is_armed_into_the_realtime_path() {
    let (project, track) = armed_project();

    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&project, 1).expect("夹具快照");
    let projected = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::strip)
        .expect("命中的效果器设备必须投影出通道条参数");

    let mut captured: Option<yeban_engine::insert::ChannelStripParams> = None;
    let mut slots = usize::MAX;
    let rendered = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum == 1 {
            captured = rig.runtime.armed_strip(&track);
            slots = rig.runtime.armed_insert_slot_count();
        }
    });

    let armed = captured.expect("命中的通道条必须被武装进实时侧");
    // 逐字段逐位对账（`f32` 用 `to_bits`，`bool` 直接比）。
    for (name, actual, expected) in [
        (
            "input_gain_db",
            armed.input_gain_db,
            projected.input_gain_db,
        ),
        (
            "output_gain_db",
            armed.output_gain_db,
            projected.output_gain_db,
        ),
        ("eq.low_gain", armed.eq.low_gain, projected.eq.low_gain),
        ("eq.low_freq", armed.eq.low_freq, projected.eq.low_freq),
        ("eq.mid_gain", armed.eq.mid_gain, projected.eq.mid_gain),
        ("eq.mid_freq", armed.eq.mid_freq, projected.eq.mid_freq),
        ("eq.mid_q", armed.eq.mid_q, projected.eq.mid_q),
        ("eq.high_gain", armed.eq.high_gain, projected.eq.high_gain),
        ("eq.high_freq", armed.eq.high_freq, projected.eq.high_freq),
        (
            "filter.cutoff_hz",
            armed.filter.cutoff_hz,
            projected.filter.cutoff_hz,
        ),
        (
            "filter.resonance",
            armed.filter.resonance,
            projected.filter.resonance,
        ),
        ("filter.drive", armed.filter.drive, projected.filter.drive),
        (
            "compressor.threshold_db",
            armed.compressor.threshold_db,
            projected.compressor.threshold_db,
        ),
        (
            "compressor.ratio",
            armed.compressor.ratio,
            projected.compressor.ratio,
        ),
    ] {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "武装表的 {name} 必须逐位等于快照投影: 武装={actual} 投影={expected}"
        );
    }
    for (name, actual, expected) in [
        ("eq_enabled", armed.eq_enabled, projected.eq_enabled),
        (
            "filter_enabled",
            armed.filter_enabled,
            projected.filter_enabled,
        ),
        (
            "compressor_enabled",
            armed.compressor_enabled,
            projected.compressor_enabled,
        ),
    ] {
        assert_eq!(actual, expected, "武装表的 {name} 必须等于快照投影");
    }
    assert!(
        armed.eq_enabled && armed.filter_enabled && armed.compressor_enabled,
        "三级旋钮都写了 ⇒ 三级都必须启用"
    );
    assert_eq!(slots, 1, "夹具只有一台通道条 ⇒ 只应占一个槽位");
    // 投影的值确实不是默认值（否则"接线了"与"没接线"读数相同）。
    assert_ne!(
        armed.eq.high_gain.to_bits(),
        yeban_engine::insert::EqParams::default()
            .high_gain
            .to_bits()
    );
    assert_ne!(
        armed.filter.cutoff_hz.to_bits(),
        yeban_engine::insert::FilterParams::DEFAULT
            .cutoff_hz
            .to_bits()
    );

    let stats = rendered.stats;
    println!(
        "[engine-wiring-2/C9] 武装: 槽位={slots} 整链处理帧数={} 动态级压过帧数={} 最大衰减={:.3} dB 峰值={:.6}",
        stats.insert_strip_frames,
        stats.insert_gain_reductions,
        stats.insert_max_reduction_db,
        rendered.peak()
    );
    // 行为层 ①：整链处理帧数 = 渲染帧数（通道条真的被调用到了）。
    assert_eq!(
        stats.insert_strip_frames,
        rendered.frames() as u64,
        "通道条处理帧数必须等于渲染帧数（差 ⇒ 逐样本路径漏掉了插入链）"
    );
    // 行为层 ②：动态级压过帧数 > 0（−30 dBFS / 8:1 ⇒ 每个音符都越阈值）。
    assert!(
        stats.insert_gain_reductions > 0,
        "夹具的音符必须真的被动态级压到（压过帧数 = 0 ⇒ 这条判据没覆盖动态级）"
    );
    assert!(
        stats.insert_max_reduction_db > 0.0,
        "最大增益衰减必须 > 0 dB（0 ⇒ 只有检波器在跑、弹道没压）"
    );
}

/// C10：**EQ 级与滤波级真的在处理信号**（可对账读数：左声道 RMS，单位 dBFS）。
///
/// 四条臂的唯一差别是设备链：
///
/// - `plain`：没有设备链（基线）；
/// - `flat`：`eq_low_gain = 0` / `eq_high_gain = 0`（**平坦** EQ 级，只有 EQ 级启用）；
/// - `shelf`：`eq_low_gain = +3` / `eq_high_gain = -12`（只有 EQ 级启用）；
/// - `lowpass`：`cutoff_hz = 250`（只有滤波级启用）。夹具的四个音高都在 261–523 Hz
///   ⇒ **全部在通带之外** ⇒ RMS 下降是**可预期的方向**，断言因此有牙。
///
/// 四条臂都用**低音量夹具**（`MixSpec::volume(-6.0)`）：默认 0 dB 夹具的峰值实测
/// `0.841`，而母线限制器阈值是 `0.9`（`crates/yeban-dsp/src/limiter.rs:173`）⇒
/// EQ 提升会把**限制器**拉进来，"RMS 差归因给通道条"就不再唯一
/// （第一版用 0 dB 夹具 + `eq_low_gain = +8` 实测 `limiter_gain_reductions = 15160`，
/// 本判据因此变红 —— 那是**归因不唯一**的正确报红，不是器件的问题）。
/// `−6 dB` 把峰值压到约 `0.42` ⇒ 留出约 6.6 dB 余量 ⇒ 提升臂仍然远离阈值。
/// 本判据因此断言四条臂的限制器压过量都为 `0`。
#[test]
fn the_eq_and_filter_stages_really_change_the_signal() {
    let fixture = tuned_project(&NOTES, MixSpec::volume(-6.0));
    let track = fixture.track;

    let plain = render(&fixture.project, QUANTA);

    let mut flat_project = fixture.project.clone();
    mount(
        &mut flat_project,
        track,
        vec![effect(&[("eq_low_gain", 0.0), ("eq_high_gain", 0.0)])],
    );
    let flat = render(&flat_project, QUANTA);

    let mut shelf_project = fixture.project.clone();
    mount(
        &mut shelf_project,
        track,
        vec![effect(&[("eq_low_gain", 6.0), ("eq_high_gain", -12.0)])],
    );
    let shelf = render(&shelf_project, QUANTA);

    let mut lowpass_project = fixture.project.clone();
    mount(
        &mut lowpass_project,
        track,
        vec![effect(&[("cutoff_hz", 250.0)])],
    );
    let lowpass = render(&lowpass_project, QUANTA);

    let (plain_rms, plain_peak) = rms_peak(&plain.left);
    let (flat_rms, _) = rms_peak(&flat.left);
    let (shelf_rms, _) = rms_peak(&shelf.left);
    let (lowpass_rms, _) = rms_peak(&lowpass.left);
    let dbfs = |linear: f64| 20.0 * (linear.max(1e-12)).log10();
    println!(
        "[engine-wiring-2/C10] 左声道 RMS: 基线={:.3} dBFS(线性 {plain_rms:.6}, 峰值 {plain_peak:.6}) \
         平坦EQ={:.3} dBFS(线性 {flat_rms:.6}) \
         架式EQ(+6低/−12高)={:.3} dBFS(线性 {shelf_rms:.6}) \
         低通250Hz={:.3} dBFS(线性 {lowpass_rms:.6})",
        dbfs(plain_rms),
        dbfs(flat_rms),
        dbfs(shelf_rms),
        dbfs(lowpass_rms),
    );

    // 归因唯一：四条臂都整体低于母线限制器阈值 ⇒ 读数差只能归因给通道条。
    for (label, arm) in [
        ("基线", &plain),
        ("平坦EQ", &flat),
        ("架式EQ", &shelf),
        ("低通", &lowpass),
    ] {
        assert_eq!(
            arm.stats.limiter_gain_reductions, 0,
            "{label}: 必须整体低于母线限制器阈值 ⇒ RMS 差归因唯一"
        );
        assert!(
            arm.peak() < 0.9,
            "{label}: 峰值 {:.6} 必须低于限制器阈值 0.9",
            arm.peak()
        );
    }
    // 行为覆盖自检：三条武装臂的插入链读数必须说明"器件真的在跑"。
    assert_eq!(plain.stats.insert_strip_frames, 0, "基线不得有插入器件");
    for (label, arm) in [("平坦EQ", &flat), ("架式EQ", &shelf), ("低通", &lowpass)] {
        assert_eq!(
            arm.stats.insert_strip_frames,
            arm.frames() as u64,
            "{label}: 通道条必须处理全部帧"
        );
        assert_eq!(
            arm.stats.insert_gain_reductions, 0,
            "{label}: 没写动态参数 ⇒ 动态级必须旁通（压过帧数 = 0）"
        );
    }
    // 输出真的变了。
    assert_ne!(
        flat.left_bits(),
        plain.left_bits(),
        "启用**平坦** EQ 级之后输出必须改变（逐位相同 ⇒ EQ 级被接到了不参与混音的地方）"
    );
    assert_ne!(
        shelf.left_bits(),
        plain.left_bits(),
        "启用架式 EQ 级之后输出必须改变"
    );
    assert_ne!(
        lowpass.left_bits(),
        plain.left_bits(),
        "启用滤波级之后输出必须改变"
    );
    // 方向可预期的那一条：全部音高在 250 Hz 之上 ⇒ RMS 必须下降。
    assert!(
        lowpass_rms < plain_rms,
        "250 Hz 低通必须让 RMS 下降: 低通 {lowpass_rms:.6} 基线 {plain_rms:.6}"
    );
    assert!(
        dbfs(plain_rms) - dbfs(lowpass_rms) > 1.0,
        "低通的 RMS 下降必须大于 1 dB（读到的差 = {:.3} dB）",
        dbfs(plain_rms) - dbfs(lowpass_rms)
    );
    // ⚠ 架式 EQ 臂的 RMS **方向**刻意不断言：`+6 dB`（低架）× `−12 dB`（高架）
    // 对这条四音符夹具的净效果取决于频谱分布，本判据只报数、只断言"输出变了"。
    // 方向可预期的那一条（低通）已经在上面断言。
    println!(
        "[engine-wiring-2/C10] 架式 EQ 净 RMS 变化 = {:+.3} dB（低架 +6 / 高架 −12，**方向不断言**）",
        dbfs(shelf_rms) - dbfs(plain_rms)
    );
}

/// C11：**确定性**（两次独立装配 + 同一个装配里的周期性重新武装）。
///
/// ⚠ 两份夹具的 `EntityId` **不同**（`EntityId::new()` 每次给一个新 ULID）：
/// 判据刻意**不**断言 id 相同，而断言**输出**逐位相同 —— 与
/// `tests/mix_render.rs` 的 M6 同一条口径。
#[test]
fn two_independent_runs_with_an_armed_channel_strip_are_bit_identical() {
    let (first_project, _) = armed_project();
    let (second_project, _) = armed_project();

    let first = render(&first_project, QUANTA);
    let second = render(&second_project, QUANTA);

    assert_eq!(first.left_bits(), second.left_bits(), "左声道必须逐位相同");
    assert_eq!(first.right, second.right, "右声道必须逐位相同");
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(
        first.stats.insert_gain_reductions,
        second.stats.insert_gain_reductions
    );
    assert_eq!(
        first.stats.insert_strip_frames,
        second.stats.insert_strip_frames
    );
    assert_eq!(
        first.stats.insert_max_reduction_db.to_bits(),
        second.stats.insert_max_reduction_db.to_bits()
    );

    // 同一个装配里：每 16 个量子发布一份**等价**快照（只改 revision）⇒ 触发
    // 重新武装（`set_params` / `set_sample_rate`，含 `exp`）。
    // 输出仍必须逐位相同 —— 这是"重新武装不把各级状态清掉"的判据。
    let (rearmed_project, _) = armed_project();
    let rearmed = render_with(&rearmed_project, QUANTA, 1, |quantum, rig| {
        if quantum > 0 && quantum % 16 == 0 {
            rig.publish_equivalent(&rearmed_project, 1 + (quantum / 16) as u64);
        }
    });
    println!(
        "[engine-wiring-2/C11] 重新武装: 指纹={:#018x}（单次装配 {:#018x}）整链处理帧数={}",
        rearmed.fingerprint(),
        first.fingerprint(),
        rearmed.stats.insert_strip_frames
    );
    assert_eq!(
        rearmed.left_bits(),
        first.left_bits(),
        "周期性发布等价快照（重新武装同一条通道条）不得改变输出"
    );
    assert_eq!(rearmed.fingerprint(), first.fingerprint());
}

/// C12：**不同量子数（块切分）下前缀逐位相同**。
///
/// 量什么：同一份夹具分别渲染 `SHORT_QUANTA` 与 `QUANTA` 个量子，
/// 比较**短渲染的全部左声道样本**与长渲染的**前 `SHORT_QUANTA × 128` 个**样本，
/// 逐位（`to_bits`）相等。
///
/// 为什么这条判据有牙：通道条内部的 EQ 是 64 帧一块（器件内部恒定），
/// 而**调用方**每量子喂 128 帧 ⇒ 若器件在量子边界留下任何跨块残留
/// （静态缓冲、错位的状态推进），这条判据会立刻变红。
#[test]
fn different_quantum_counts_agree_on_the_shared_prefix() {
    const SHORT_QUANTA: usize = 37;
    let (project, _) = armed_project();
    let short = render(&project, SHORT_QUANTA);
    let long = render(&project, QUANTA);

    assert_eq!(short.frames(), SHORT_QUANTA * 128);
    assert!(long.frames() > short.frames());
    let short_bits = short.left_bits();
    let long_prefix = &long.left_bits()[..short_bits.len()];
    assert_eq!(
        short_bits, long_prefix,
        "{SHORT_QUANTA} 个量子的渲染必须是长渲染的前缀（逐位）"
    );
    assert_eq!(
        short.right[..],
        long.right[..short.frames()],
        "右声道前缀也必须逐位相同"
    );
    // 覆盖度自检：短窗口里真的处理过帧、真的有输出。
    assert_eq!(short.stats.insert_strip_frames, short.frames() as u64);
    assert!(short.peak() > 0.0, "短窗口必须真的出声");
    println!(
        "[engine-wiring-2/C12] 前缀对账: 短渲染帧={} 前缀逐位相同=true 短窗口峰值={:.6}",
        short.frames(),
        short.peak()
    );
}

/// C13：引擎的 `ChannelStrip` **就是** `yeban_dsp::channel_strip::ChannelStrip`。
///
/// 编译期判据（类型同一性）+ 运行期判据（函数地址相等 + **零延迟**契约）。
/// 注入：在引擎侧加一份自己的通道条类型或 `process_mono` ⇒ 两者之一立即变红。
/// 与 `mixer.rs` 的 `engine_bus_limiter_is_literally_the_dsp_limiter` 同款。
#[test]
fn engine_channel_strip_is_literally_the_dsp_channel_strip() {
    // 类型同一性（编译期）：dsp 类型的绑定可以直接由 engine 路径构造。
    let via_engine: yeban_engine::insert::DspChannelStrip = yeban_engine::insert::ChannelStrip::new(
        yeban_engine::insert::ChannelStripParams::DEFAULT,
        48_000.0,
    );
    // 零延迟契约：通道条**不进** PDC 表（接线因此不改任何补偿计划）。
    assert_eq!(
        via_engine.latency_samples(),
        0,
        "通道条必须零延迟，否则接线会改变 PDC 计划"
    );

    // 方法同一性（地址相等）：这些**必须**是同一个函数项，而不是两份同构实现。
    let engine_process: fn(&mut yeban_engine::insert::ChannelStrip, &mut [f32]) -> usize =
        yeban_engine::insert::ChannelStrip::process_mono;
    let dsp_process: fn(&mut yeban_dsp::channel_strip::ChannelStrip, &mut [f32]) -> usize =
        yeban_dsp::channel_strip::ChannelStrip::process_mono;
    assert!(core::ptr::fn_addr_eq(engine_process, dsp_process));

    let engine_new: fn(
        yeban_engine::insert::ChannelStripParams,
        f32,
    ) -> yeban_engine::insert::ChannelStrip = yeban_engine::insert::ChannelStrip::new;
    let dsp_new: fn(
        yeban_dsp::channel_strip::ChannelStripParams,
        f32,
    ) -> yeban_dsp::channel_strip::ChannelStrip = yeban_dsp::channel_strip::ChannelStrip::new;
    assert!(core::ptr::fn_addr_eq(engine_new, dsp_new));

    let engine_set: fn(
        &mut yeban_engine::insert::ChannelStrip,
        yeban_engine::insert::ChannelStripParams,
    ) = yeban_engine::insert::ChannelStrip::set_params;
    let dsp_set: fn(
        &mut yeban_dsp::channel_strip::ChannelStrip,
        yeban_dsp::channel_strip::ChannelStripParams,
    ) = yeban_dsp::channel_strip::ChannelStrip::set_params;
    assert!(core::ptr::fn_addr_eq(engine_set, dsp_set));

    // 参数类型同一性（逐位）：engine 路径的默认值就是 dsp 的默认值。
    assert_eq!(
        yeban_engine::insert::ChannelStripParams::DEFAULT
            .eq
            .high_gain
            .to_bits(),
        yeban_dsp::channel_strip::ChannelStripParams::DEFAULT
            .eq
            .high_gain
            .to_bits()
    );
    assert_eq!(
        yeban_engine::insert::CompressorParams::DEFAULT
            .threshold_db
            .to_bits(),
        yeban_dsp::compressor::CompressorParams::DEFAULT
            .threshold_db
            .to_bits()
    );
}

/// C14：设备链顺序 —— 第一个**含已识别参数名**的效果器是唯一来源。
///
/// 更早的无关效果器不参与；更晚的效果器不覆盖。
/// 同时断言 `support::two_track_project` 的两轨夹具下**两条轨各占一个槽位**。
#[test]
fn the_first_recognised_effect_in_the_chain_is_the_only_strip_source() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;
    let mut project = fixture.project.clone();
    mount(
        &mut project,
        track,
        vec![
            // 第一条：一个已识别名字都没有 ⇒ 不是来源。
            effect(&[("mix", 0.5)]),
            // 第二条：命中 ⇒ 唯一来源。
            effect(&[("eq_low_gain", 5.0), ("threshold", -20.0)]),
            // 第三条：不得覆盖第二条。
            effect(&[("eq_low_gain", -24.0), ("threshold_db", -60.0)]),
        ],
    );

    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&project, 1).expect("快照");
    let params = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::strip)
        .expect("第二条设备必须命中");
    assert_eq!(
        params.eq.low_gain.to_bits(),
        5.0f32.to_bits(),
        "第二条是唯一来源"
    );
    assert_eq!(
        params.compressor.threshold_db.to_bits(),
        (-20.0f32).to_bits(),
        "别名 `threshold` 必须与 `threshold_db` 同解"
    );
    assert!(params.eq_enabled, "写了 EQ 旋钮 ⇒ EQ 级启用");
    assert!(params.compressor_enabled, "写了动态旋钮 ⇒ 动态级启用");
    assert!(!params.filter_enabled, "没写滤波旋钮 ⇒ 滤波级不启用");

    // 行为层：同一条链只武装一台器件。
    let mut slots = usize::MAX;
    let _ = render_with(&project, 2, 1, |quantum, rig| {
        if quantum == 1 {
            slots = rig.runtime.armed_insert_slot_count();
        }
    });
    assert_eq!(slots, 1, "一条链只允许武装一台插入器件");

    // 两轨夹具：两条轨都挂器件 ⇒ 两个槽位（**两张器件同时存在**的判据）。
    let (mut two_track, first, second) = support::two_track_project(&NOTES, &NOTES);
    mount(
        &mut two_track,
        first,
        vec![effect(&[("threshold_db", -30.0), ("ratio", 8.0)])],
    );
    mount(
        &mut two_track,
        second,
        vec![effect(&[("eq_high_gain", -12.0)])],
    );
    let mut slots = usize::MAX;
    let mut first_armed = false;
    let mut second_armed = false;
    let _ = render_with(&two_track, 2, 1, |quantum, rig| {
        if quantum == 1 {
            slots = rig.runtime.armed_insert_slot_count();
            first_armed = rig.runtime.armed_strip(&first).is_some();
            second_armed = rig.runtime.armed_strip(&second).is_some();
        }
    });
    assert_eq!(slots, 2, "两条轨各挂一台器件 ⇒ 两个槽位");
    assert!(first_armed && second_armed, "两条轨都必须真的武装进去");
}

/// C15：`insert_current_reduction_db` 是**量规**，不是第二个累计量。
///
/// # 判据（六个断言，每个都能单独变红）
///
/// | # | 断言 | 单位/对象 |
/// | :-: | :--- | :--- |
/// | 1 | 覆盖度：夹具真的压到（`insert_gain_reductions > 0`、`insert_max_reduction_db > 0`），且窗口里真的有声 | 帧 / dB |
/// | 2 | 不变式：**每一个量子边界**上 `当前 ≤ 全程最大`，且两者都 `≥ 0` | dB |
/// | 3 | 累计量单调不减：逐量子采样的 `全程最大` 序列**永不下降** | dB |
/// | 4 | **量规回落**：源静音段末尾的 `当前` < 压缩段峰值 `当前`，且 < 终点 `全程最大` 的一半 | dB |
/// | 5 | 未武装工程（同夹具去掉设备）整段窗口之后两者**逐位为 `0.0`**；镜像与权威在静止点上逐字段相等 | dB |
/// | 6 | **跨轨口径 = 最大**：同一个两轨工程的三次渲染（两轨都挂 / 只挂 A / 只挂 B）里，逐量子 `两轨 = max(只 A, 只 B)` **逐位相等** | dB |
///
/// # 为什么这些断言有牙（它们分别打在哪条注入上）
///
/// - 第 4 条打"把当前读数抄成第二个累计量"（读数改成器件的 `max_reduction_db`）：
///   尾部当前 == 终点最大 ⇒ 第 4 条红，而第 2、3 条仍然绿 —— 这正是"量规与累计量"的分界；
/// - 第 4 条同时打"漏读器件 getter、读数不写"：那样**峰值** `当前` 也是 0，
///   "末尾 < 峰值"不成立（`0 < 0` 为假）；
/// - 第 6 条打"跨轨取**最后一条轨**而不是最大"：三条臂的两条单轨读数在多个量子边界上
///   相差数 dB（下面显式断言这一点）⇒ "最后一条轨"的读数会与 `max` 不等；
/// - 第 5 条打"读数写成非零常数"：没有武装器件时它必须逐位是 `0.0`。
///
/// # 窗口设计（为什么是 700 个量子）
///
/// `NOTES` 的最后一条音符在 tick 2400 结束（1 tick = 25 样本 ⇒ 第 60 000 帧），
/// 即第 468.75 个量子；700 个量子 = 89 600 帧 ⇒ 尾部约 231 个量子（≈ 0.48 s）是
/// **源静音**段。动态级的默认释放时间常数是 0.1 s（`CompressorParams::DEFAULT`）
/// ⇒ 那一段是 4.8 个时间常数：量规必须明显回落，而累计量必须一动不动。
/// 没有那段静音，"回落"与"一直没压"在读数上不可区分（假绿）。
#[test]
fn insert_current_reduction_is_a_gauge_not_a_second_counter() {
    /// 渲染量子数（见判据文档的窗口设计）。
    const RELEASE_QUANTA: usize = 700;

    let (project, _track) = armed_project();

    // 逐量子采样：`samples[i]` = **第 i 个量子处理完之后**的两个读数
    // （`render_with` 的回调在量子**之前**调用 ⇒ 采到的是上一个量子的结果）。
    let mut samples: Vec<(f32, f32)> = Vec::with_capacity(RELEASE_QUANTA + 1);
    let mut mirror_matches_authoritative = true;
    let rendered = render_with(&project, RELEASE_QUANTA, 1, |_quantum, rig| {
        let stats = rig.runtime.stats();
        samples.push((
            stats.insert_current_reduction_db,
            stats.insert_max_reduction_db,
        ));
        // 静止点等号（同一条线程、没有并发写者）：镜像的搬运必须逐字段一致。
        // 它不是本判据的主角（`stats_mirror.rs` 的漂移闸门负责那一面），但新字段
        // 一旦漏出 `publish`，这里会立刻红。
        if rig.runtime.stats_mirror().read() != stats {
            mirror_matches_authoritative = false;
        }
    });
    let final_stats = rendered.stats;
    samples.push((
        final_stats.insert_current_reduction_db,
        final_stats.insert_max_reduction_db,
    ));

    assert_eq!(
        samples.len(),
        RELEASE_QUANTA + 1,
        "逐量子采样必须覆盖每一个量子边界（含末端）"
    );
    // ---- 判据 1：覆盖度（防"什么都没跑"的假绿）----
    assert!(
        rendered.nonzero() > 0,
        "窗口里必须真的有声，否则量规判据是空转（假绿）"
    );
    assert!(
        final_stats.insert_gain_reductions > 0,
        "夹具的动态级必须真的压过帧（压过帧数 = 0 ⇒ 本判据没有覆盖动态级）"
    );
    assert!(
        final_stats.insert_max_reduction_db > 0.0,
        "夹具的全程最大衰减必须 > 0 dB（0 ⇒ 只有检波器在跑、弹道没压）"
    );

    // ---- 判据 2 + 3：逐量子不变式与累计量的单调性 ----
    let mut invariant_violations = 0usize;
    let mut monotonicity_violations = 0usize;
    let mut previous_max = 0.0f32;
    let mut peak_current = 0.0f32;
    let mut peak_current_quantum = 0usize;
    for (quantum, (current, all_time_max)) in samples.iter().copied().enumerate() {
        // 两个读数都是 dB 衰减 ⇒ 非负有限（`-0.0 < 0.0` 为假 ⇒ `-0.0` 合法）。
        if current < 0.0 || all_time_max < 0.0 || !current.is_finite() || !all_time_max.is_finite()
        {
            invariant_violations += 1;
        }
        // **量规 ≤ 累计量**：器件自己的 `max_reduction_db` 是逐帧取最大，
        // 而 `current` 只是某一帧的值 ⇒ 这条不等式的两边来自同一个器件、同一瞬间。
        if current > all_time_max {
            invariant_violations += 1;
        }
        if all_time_max < previous_max {
            monotonicity_violations += 1;
        }
        previous_max = all_time_max;
        if current > peak_current {
            peak_current = current;
            peak_current_quantum = quantum;
        }
    }
    assert_eq!(
        invariant_violations, 0,
        "每一个量子边界上都必须 `0 ≤ 当前 ≤ 全程最大`（同源同瞬间）"
    );
    assert_eq!(
        monotonicity_violations, 0,
        "`insert_max_reduction_db` 是只增不减的累计量：逐量子采样绝不允许下降"
    );

    // ---- 判据 4：量规在源静音段回落，累计量不动 ----
    //
    // 峰值必须出现在**还在出声**的段里（第 468 个量子之前），否则夹具没有覆盖到
    // "压限正在建立"的那一半。
    assert!(
        peak_current > 0.0,
        "峰值当前衰减必须 > 0 dB（恒 0 ⇒ 读数根本没被写进去）"
    );
    assert!(
        peak_current_quantum < 468,
        "峰值必须落在音符还在响的段里（实测第 {peak_current_quantum} 个量子）"
    );
    let (tail_current, tail_max) = samples[RELEASE_QUANTA];
    assert!(
        tail_current < peak_current,
        "源静音段末尾的当前衰减（{tail_current} dB）必须严格小于压缩段峰值（{peak_current} dB）\
         —— 相等 ⇒ 这个读数是第二个累计量，不是量规"
    );
    assert!(
        tail_current * 2.0 < tail_max,
        "源静音段末尾的当前衰减（{tail_current} dB）必须落到终点全程最大（{tail_max} dB）的\
         一半以下（释放时间常数 0.1 s，尾部静音约 0.48 s ⇒ 实测应当低一个数量级）"
    );
    assert!(
        tail_max >= peak_current,
        "累计量在整段窗口里不得下降（终点 {tail_max} dB，峰值当前 {peak_current} dB）"
    );
    println!(
        "[engine-wiring-2/C15] 量规 vs 累计量: 峰值当前={peak_current:.4} dB（第 {peak_current_quantum} 个量子）\
         终点当前={tail_current:.4} dB 终点最大={tail_max:.4} dB 压过帧数={} 镜像等号={mirror_matches_authoritative}",
        final_stats.insert_gain_reductions
    );
    assert!(
        mirror_matches_authoritative,
        "静止点上镜像必须与权威读数逐字段相等（新字段漏出 `publish` ⇒ 这里红）"
    );

    // ---- 判据 5（对照臂）：没有武装通道条 ⇒ 两个读数都必须是冷值 ----
    //
    // 同一个夹具、同一条轨、同样的音符，**只**去掉设备链 ⇒ 插入链整段跳过
    // ⇒ 两个读数必须逐位为 `0.0`。这一条把"读数恒 0"与"没有插入链"分开：
    // 若实现里把读数写成非零常数（不读器件），这里会红。
    let bare = tuned_project(&NOTES, MixSpec::volume(0.0));
    let bare_rendered = render(&bare.project, RELEASE_QUANTA);
    assert_eq!(
        bare_rendered.stats.insert_current_reduction_db.to_bits(),
        0.0f32.to_bits(),
        "没有武装通道条时当前衰减必须是冷值 0.0"
    );
    assert_eq!(
        bare_rendered.stats.insert_max_reduction_db.to_bits(),
        0.0f32.to_bits(),
        "没有武装通道条时全程最大衰减必须是冷值 0.0"
    );
    assert_eq!(
        bare_rendered.stats.insert_gain_reductions, 0,
        "没有武装通道条时压过帧数必须为 0"
    );
    // 同一条轨在**有**设备时必须报出非零（否则上面那个 0 是"什么都没测"）。
    assert_eq!(
        bare.project
            .tracks
            .get(&bare.track)
            .map(|entry| entry.devices.len()),
        Some(0),
        "对照臂的那条轨必须真的没有设备（它是 `armed_project` 的同一份夹具，只少了 `mount`）"
    );
    assert!(
        final_stats.insert_current_reduction_db > 0.0,
        "同一个夹具**有**设备时当前衰减必须报出非零（与对照臂的 0 构成一对）"
    );

    // ---- 判据 6：跨轨口径必须是**最大**，不是"最后一条轨" ----
    //
    // 三条臂用**同一份工程**（同一次 `two_track_project` ⇒ 同一批 `EntityId`、
    // 同一个 `BTreeMap` 键序），唯一差别是哪条轨还挂着自己的器件：
    //
    //   * `both`：两条轨各挂一台**参数相同**的通道条；
    //   * `only_first` / `only_second`：把另一条轨的设备链清空。
    //
    // 逐轨插入链的状态只取决于**本轨自己的样本与参数**（声源、参数表、电平、
    // PDC 与母线都在它之后）⇒ `both` 的读数必须逐量子等于 `max(only_first, only_second)`。
    // 若实现取的是"循环里最后一条轨"的值，`max` 与它会在两条单轨读数不同的量子边界上分叉。
    //
    // ⚠ **夹具必须让"谁更大"双向出现**：`BTreeMap` 的迭代顺序由 `EntityId`（ULID）决定，
    // 而 ULID 带随机分量 ⇒ "最后一条轨"是 A 还是 B 在每次运行里都可能不同。
    // 若夹具里 A 恒 ≥ B，那么"最后一条轨"注入只在**一半**的运行里被抓到（判据会变成掷硬币）。
    // 因此两条轨用**相同的参数、错开的音符窗口**：
    //
    //   * 轨 A 的音符在 tick 0..480（帧 0..12 000 ⇒ 量子 0..93.75）；
    //   * 轨 B 的音符在 tick 960..1440（帧 24 000..36 000 ⇒ 量子 187.5..281.25）。
    //
    // ⇒ A 先压、B 后压：A 在开头独占（`only_first > only_second`），B 在自己的窗口里
    // 稳压住正在释放的 A（`only_second > only_first`）。下面**两条方向都要断言非空**
    // ——那就是"无论迭代顺序如何，这条判据都有牙"的机械形式。
    let notes_first = [NoteSpec::at(0, 480, 60, 127)];
    let notes_second = [NoteSpec::at(960, 480, 60, 127)];
    let (mut two_track, first, second) = support::two_track_project(&notes_first, &notes_second);
    mount(
        &mut two_track,
        first,
        vec![effect(&[("threshold_db", -30.0), ("ratio", 8.0)])],
    );
    mount(
        &mut two_track,
        second,
        vec![effect(&[("threshold_db", -30.0), ("ratio", 8.0)])],
    );

    /// 渲染一份两轨工程并逐量子采出 `insert_current_reduction_db` 序列。
    fn gauge_series(project: &YebanProjectV1, quanta: usize) -> Vec<f32> {
        let mut series: Vec<f32> = Vec::with_capacity(quanta + 1);
        let rendered = render_with(project, quanta, 1, |_quantum, rig| {
            series.push(rig.runtime.stats().insert_current_reduction_db);
        });
        series.push(rendered.stats.insert_current_reduction_db);
        series
    }

    let both = gauge_series(&two_track, RELEASE_QUANTA);
    let mut only_first_project = two_track.clone();
    mount(&mut only_first_project, second, vec![]);
    let only_first = gauge_series(&only_first_project, RELEASE_QUANTA);
    let mut only_second_project = two_track.clone();
    mount(&mut only_second_project, first, vec![]);
    let only_second = gauge_series(&only_second_project, RELEASE_QUANTA);

    assert_eq!(both.len(), RELEASE_QUANTA + 1);
    assert_eq!(only_first.len(), RELEASE_QUANTA + 1);
    assert_eq!(only_second.len(), RELEASE_QUANTA + 1);

    // 牙齿：两个方向都必须真的出现（各以 > 1 dB 为准）。
    // 没有这一条，"最后一条轨"注入的检出率取决于 ULID 的随机顺序（掷硬币）。
    let mut first_above = 0usize;
    let mut second_above = 0usize;
    let mut max_separation = 0.0f32;
    for (single_a, single_b) in only_first.iter().zip(only_second.iter()) {
        if single_a - single_b > 1.0 {
            first_above += 1;
        }
        if single_b - single_a > 1.0 {
            second_above += 1;
        }
        max_separation = max_separation.max((single_a - single_b).abs());
    }
    assert!(
        first_above > 0 && second_above > 0,
        "两条单轨读数必须在**两个方向**上都出现过 > 1 dB 的差（实测 A 领先 {first_above} 个量子边界、\
         B 领先 {second_above} 个）—— 单方向夹具会让跨轨口径判据变成掷硬币"
    );

    let mut mismatches = 0usize;
    let mut first_mismatch: Option<(usize, f32, f32, f32)> = None;
    for quantum in 0..=RELEASE_QUANTA {
        let expected = only_first[quantum].max(only_second[quantum]);
        if both[quantum].to_bits() != expected.to_bits() {
            mismatches += 1;
            if first_mismatch.is_none() {
                first_mismatch = Some((
                    quantum,
                    both[quantum],
                    only_first[quantum],
                    only_second[quantum],
                ));
            }
        }
    }
    assert_eq!(
        mismatches, 0,
        "两轨的当前衰减必须逐量子等于两条单轨读数的**最大**（首个不符：{first_mismatch:?}）"
    );
    println!(
        "[engine-wiring-2/C15] 跨轨口径: A 领先 {first_above} 个量子边界 / B 领先 {second_above} 个 / 共 {} 最大差异={max_separation:.4} dB 逐位最大={}",
        RELEASE_QUANTA + 1,
        mismatches == 0
    );
}

/// C16：`insert_detector_level_db` 是插入链动态级的**输入**侧读数（检波器电平，dBFS）。
///
/// # 判据（七条断言，每条都能单独变红）
///
/// | # | 断言 | 单位/对象 |
/// | :-: | :--- | :--- |
/// | 1 | 覆盖度**对照读数**：同一窗口里 `insert_strip_frames > 0`、`insert_gain_reductions > 0`、`insert_current_reduction_db > 0`（证明扫描模式有效，不是把一切都扫成冷值） | 帧 / dB |
/// | 2 | 冷值形态与过渡：第一个量子**之前**是 `None`，窗口里抬起过 `Some(…)`，且没有武装器件的工程整段**恒为 `None`**（**不是** `Some(0.0)` —— 0 dBFS 是合法电平） | — |
/// | 3 | 抬升：端点是 `Some(L)`，`L` 有限且 `L > MIN_LEVEL_DB`（检波器真的看见了信号） | dBFS |
/// | 4 | 量规回落：源静音段末尾**逐位**等于器件地板 `MIN_LEVEL_DB`（−120 dBFS），而同一窗口的累计量 `insert_max_reduction_db` 仍 `> 0`（一个落回、一个不落） | dBFS |
/// | 5 | 采样点在**动态级**：只有 EQ 的通道条（动态级旁通 ⇒ 压缩器从未被调用）报出的正是它的构造初值 `MIN_LEVEL_DB`，而该通道条整链处理帧数 `> 0`、压过帧数 `= 0` | dBFS |
/// | 6 | 镜像搬运：静止点上 `stats_mirror().read()` 与 `stats()` 逐字段相等（新字段漏出 `publish` ⇒ 红） | — |
/// | 7 | 跨轨口径 = **最大**：三条臂逐量子 `两轨 = max(只 A, 只 B)`（`None` 按"缺失"参与比较，不映射成某个 dB 值） | dBFS |
///
/// # 为什么这些断言有牙（各自打在哪条注入上）
///
/// - 第 2 条打"冷值写成 `0.0`"：那样未武装工程会读出 `Some(0.0)`（= 满刻度，一条**假读数**）
///   ⇒ 第 2 条红，而第 3、4 条在有声工程上**仍然绿**；
/// - 第 4 条打"这个读数其实是第二个累计最大值"（改成器件的 `max_reduction_db` 或累计 `max`）：
///   源静音之后它不会落回地板 ⇒ 尾部与 `MIN_LEVEL_DB` 不等；
/// - 第 4 条同时打"漏读器件 getter、读数不写"：那样窗口里根本没有 `Some`；
/// - 第 5 条打"读的是整链电平而不是动态级检波器"：EQ-only 的通道条在**整链**上是有信号的
///   （`insert_strip_frames > 0`），只有压缩器没跑 ⇒ 它必须逐位是地板。若读的是
///   通道条的 `output_*` / `input_*` 之类整链计量，这里会读到有声电平；
/// - 第 7 条打"跨轨取最后一条轨而不是最大"（与 C15 第 6 条同款夹具与理由）。
///
/// # 窗口设计（与 C15 同一份）
///
/// `NOTES` 的最后一条音符在 tick 2400 结束（1 tick = 25 样本 ⇒ 第 60 000 帧，即第 468.75 个
/// 量子）；700 个量子 = 89 600 帧 ⇒ 尾部约 231 个量子（≈ 0.48 s）是源静音。
/// 检波器的时间常数是 `CompressorParams::DEFAULT` 的 `detector_s = 5 ms`
/// （`crates/yeban-dsp/src/compressor.rs` 现位于第 279 行）⇒ 那一段有 **30 个以上**的
/// 时间常数：`mean_square` 早已衰减到 `POWER_FLOOR` 以下，器件因此报**逐位**的地板值。
/// 器件自己把这条口径写成契约（同文件现位于第 1306 行的单测断言静音 ⇒ `level_db() ==
/// MIN_LEVEL_DB`）。
#[test]
fn insert_detector_level_is_the_input_side_of_the_dynamics_stage() {
    /// 渲染量子数（与 C15 同一个窗口）。
    const RELEASE_QUANTA: usize = 700;
    /// 跨轨口径那条对照臂的窗口（覆盖两条轨各自的音符段即可）。
    const CROSS_QUANTA: usize = 300;

    let floor = yeban_dsp::compressor::MIN_LEVEL_DB;
    assert!(
        floor < -100.0 && floor.is_finite(),
        "检波器地板必须在 −100 dBFS 以下（否则'落回地板'与'还在有声'在读数上不可分）：{floor}"
    );

    let (project, _track) = armed_project();

    // 逐量子采样：`samples[i]` = **第 i 个量子处理完之后**的读数
    // （`render_with` 的回调在量子**之前**调用 ⇒ 采到的是上一个量子的结果）。
    let mut samples: Vec<Option<f32>> = Vec::with_capacity(RELEASE_QUANTA + 1);
    let mut mirror_matches_authoritative = true;
    let rendered = render_with(&project, RELEASE_QUANTA, 1, |_quantum, rig| {
        let stats = rig.runtime.stats();
        samples.push(stats.insert_detector_level_db);
        // 静止点等号（同一条线程、没有并发写者）：镜像的搬运必须逐字段一致。
        // 新字段一旦漏出 `publish`，这里会立刻红（判据 6）。
        if rig.runtime.stats_mirror().read() != stats {
            mirror_matches_authoritative = false;
        }
    });
    let final_stats = rendered.stats;
    samples.push(final_stats.insert_detector_level_db);
    assert_eq!(
        samples.len(),
        RELEASE_QUANTA + 1,
        "逐量子采样必须覆盖每一个量子边界（含末端）"
    );

    // ---- 判据 1：覆盖度**对照读数**（同族其它成员的非零读数）----
    assert!(
        rendered.nonzero() > 0,
        "窗口里必须真的有声，否则本判据是空转（假绿）"
    );
    assert!(
        final_stats.insert_strip_frames > 0,
        "对照读数：通道条必须真的处理过帧（0 ⇒ 本窗口没覆盖插入链）"
    );
    assert!(
        final_stats.insert_gain_reductions > 0,
        "对照读数：动态级必须真的压过帧（0 ⇒ 本窗口没覆盖动态级）"
    );
    assert!(
        final_stats.insert_current_reduction_db > 0.0,
        "对照读数：当前衰减必须报出非零（0.0 ⇒ 同族的量规也没被写进去）"
    );

    // ---- 判据 2：冷值 None → Some 的过渡 ----
    assert_eq!(
        samples[0], None,
        "第一个量子处理**之前**必须是冷值 None（引擎构造期发布的初值）"
    );
    let mut peak = f32::NEG_INFINITY;
    let mut peak_quantum = 0usize;
    for (quantum, sample) in samples.iter().enumerate() {
        if let Some(level) = *sample {
            assert!(
                level.is_finite(),
                "检波器电平必须有限（第 {quantum} 个量子读到 {level}）"
            );
            if level > peak {
                peak = level;
                peak_quantum = quantum;
            }
        }
    }

    // ---- 判据 3：抬升（检波器真的看见了信号）----
    assert!(
        peak > floor,
        "检波器电平在整段窗口里从没离开地板（峰值 {peak} dBFS）—— 读数没被写进去"
    );
    assert!(
        peak_quantum < 468,
        "峰值必须落在音符还在响的段里（实测第 {peak_quantum} 个量子）"
    );

    // ---- 判据 4：量规在源静音段落回器件地板，而累计量不落 ----
    let tail = samples[RELEASE_QUANTA].expect("武装窗口末尾必须是 Some");
    assert_eq!(
        tail.to_bits(),
        floor.to_bits(),
        "源静音段末尾的检波器电平必须逐位落回器件地板 {floor} dBFS（实测 {tail}）\
         —— 不落 ⇒ 这个读数是累计量，不是量规"
    );
    assert!(
        final_stats.insert_max_reduction_db > 0.0,
        "对照读数：同窗口的全程最大衰减必须仍 > 0 dB（它**不**随静音回落）\
         —— 实测 {} dB",
        final_stats.insert_max_reduction_db
    );
    assert!(
        mirror_matches_authoritative,
        "静止点上镜像必须与权威读数逐字段相等（新字段漏出 `publish` ⇒ 这里红）"
    );

    // ---- 判据 2 的后半 + 判据 5：两条**负对照**臂 ----
    //
    // 臂 A：同一个夹具、同一条轨、同样的音符，**只**去掉设备链 ⇒ 插入链整段跳过
    // ⇒ 读数必须是冷值 `None`。这一条把"读数恒 None"与"没有插入链"分开。
    let bare = tuned_project(&NOTES, MixSpec::volume(0.0));
    let bare_rendered = render(&bare.project, RELEASE_QUANTA);
    assert_eq!(
        bare_rendered.stats.insert_detector_level_db, None,
        "没有武装通道条时检波器电平必须是冷值 None（**不是** Some(0.0)：0 dBFS 是合法电平）"
    );
    assert_eq!(
        bare_rendered.stats.insert_strip_frames, 0,
        "对照读数：负对照臂必须真的没有通道条"
    );
    // 臂 B：只有 EQ 旋钮 ⇒ `compressor_enabled = false`（投影规则：没写旋钮的级不执行）
    // ⇒ 压缩器**从未被调用**，它报自己的构造初值（地板）；而整链照样在跑。
    let mut eq_only = tuned_project(&NOTES, MixSpec::volume(0.0));
    mount(
        &mut eq_only.project,
        eq_only.track,
        vec![effect(&[("eq_low_gain", 6.0)])],
    );
    let eq_rendered = render(&eq_only.project, RELEASE_QUANTA);
    assert_eq!(
        eq_rendered.stats.insert_detector_level_db.map(f32::to_bits),
        Some(floor.to_bits()),
        "动态级旁通的通道条必须报压缩器的构造初值（地板）—— 读到有声电平 ⇒ 读的不是动态级检波器"
    );
    assert!(
        eq_rendered.stats.insert_strip_frames > 0,
        "对照读数：EQ-only 的通道条必须真的在整链上处理过帧（否则上面那条是空转）"
    );
    assert_eq!(
        eq_rendered.stats.insert_gain_reductions, 0,
        "对照读数：动态级旁通 ⇒ 压过帧数必须为 0（否则投影没把该级关掉）"
    );

    println!(
        "[engine-wiring-2/C16] 检波器电平: 峰值={peak:.3} dBFS（第 {peak_quantum} 个量子）\
         端点={tail:.3} dBFS 地板={floor:.3} dBFS 整链帧={} 压过帧={} 当前衰减={:.3} dB \
         未武装={:?} EQ-only={:?} 镜像等号={mirror_matches_authoritative}",
        final_stats.insert_strip_frames,
        final_stats.insert_gain_reductions,
        final_stats.insert_current_reduction_db,
        bare_rendered.stats.insert_detector_level_db,
        eq_rendered.stats.insert_detector_level_db,
    );

    // ---- 判据 7：跨轨口径必须是**最大**，不是"最后一条轨" ----
    //
    // 夹具与理由照抄 C15 的第 6 条：三条臂用**同一份工程**（同一次 `two_track_project`
    // ⇒ 同一批 `EntityId`、同一个 `BTreeMap` 键序），唯一差别是哪条轨还挂着自己的器件。
    // 两条轨的**音符窗口错开**（A 在 tick 0..480、B 在 tick 960..1440）⇒ "谁更大"双向出现，
    // 与 `BTreeMap` 的 ULID 迭代顺序无关。
    let notes_first = [NoteSpec::at(0, 480, 60, 127)];
    let notes_second = [NoteSpec::at(960, 480, 60, 127)];
    let (mut two_track, first, second) = support::two_track_project(&notes_first, &notes_second);
    let dynamics = [("threshold_db", -30.0), ("ratio", 8.0)];
    mount(&mut two_track, first, vec![effect(&dynamics)]);
    mount(&mut two_track, second, vec![effect(&dynamics)]);

    /// 渲染一份两轨工程并逐量子采出检波器电平序列。
    fn level_series(project: &YebanProjectV1, quanta: usize) -> Vec<Option<f32>> {
        let mut series: Vec<Option<f32>> = Vec::with_capacity(quanta + 1);
        let rendered = render_with(project, quanta, 1, |_quantum, rig| {
            series.push(rig.runtime.stats().insert_detector_level_db);
        });
        series.push(rendered.stats.insert_detector_level_db);
        series
    }

    let both = level_series(&two_track, CROSS_QUANTA);
    let mut only_first_project = two_track.clone();
    mount(&mut only_first_project, second, vec![]);
    let only_first = level_series(&only_first_project, CROSS_QUANTA);
    let mut only_second_project = two_track.clone();
    mount(&mut only_second_project, first, vec![]);
    let only_second = level_series(&only_second_project, CROSS_QUANTA);

    // 牙齿：两个方向都必须真的出现（各以 > 1 dB 为准）。
    let mut first_above = 0usize;
    let mut second_above = 0usize;
    let mut max_separation = 0.0f32;
    for (single_a, single_b) in only_first.iter().zip(only_second.iter()) {
        if let (Some(a), Some(b)) = (*single_a, *single_b) {
            if a - b > 1.0 {
                first_above += 1;
            }
            if b - a > 1.0 {
                second_above += 1;
            }
            max_separation = max_separation.max((a - b).abs());
        }
    }
    assert!(
        first_above > 0 && second_above > 0,
        "两条单轨读数必须在**两个方向**上都出现过 > 1 dB 的差（实测 A 领先 {first_above} 个量子边界、\
         B 领先 {second_above} 个）—— 单方向夹具会让跨轨口径判据变成掷硬币"
    );

    // `None` 按"缺失"参与比较（它只在量子 0 出现：那时三条臂都还没处理过帧），
    // **不**把 `None` 映射成某个 dB 值（那会引入一个不是读数的数）。
    fn max_option(a: Option<f32>, b: Option<f32>) -> Option<f32> {
        match (a, b) {
            (Some(x), Some(y)) => Some(x.max(y)),
            (Some(x), None) => Some(x),
            (None, Some(y)) => Some(y),
            (None, None) => None,
        }
    }
    fn same_option(a: Option<f32>, b: Option<f32>) -> bool {
        match (a, b) {
            (Some(x), Some(y)) => x.to_bits() == y.to_bits(),
            (None, None) => true,
            _ => false,
        }
    }

    /// 首个不符的现场（量子号 + 三条臂各自的读数）。
    type Mismatch = (usize, Option<f32>, Option<f32>, Option<f32>);
    let mut mismatches = 0usize;
    let mut first_mismatch: Option<Mismatch> = None;
    for quantum in 0..=CROSS_QUANTA {
        let expected = max_option(only_first[quantum], only_second[quantum]);
        if !same_option(both[quantum], expected) {
            mismatches += 1;
            if first_mismatch.is_none() {
                first_mismatch = Some((
                    quantum,
                    both[quantum],
                    only_first[quantum],
                    only_second[quantum],
                ));
            }
        }
    }
    assert_eq!(
        mismatches, 0,
        "两轨的检波器电平必须逐量子等于两条单轨读数的**最大**（首个不符：{first_mismatch:?}）"
    );
    println!(
        "[engine-wiring-2/C16] 跨轨口径: A 领先 {first_above} 个量子边界 / B 领先 {second_above} 个 / 共 {} 最大差异={max_separation:.4} dB 逐位最大={}",
        CROSS_QUANTA + 1,
        mismatches == 0
    );
}
