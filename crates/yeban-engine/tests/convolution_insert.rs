//! `line/engine-7` 的端到端判据：**每轨插入卷积混响**
//! （`crate::insert` → `snapshot` → `rt`）。[ARCH-RT-001, ARCH-DET-001, ARCH-PDC-001]
//!
//! 本文件与本 crate 的 `tests/compressor_insert.rs`（裸压缩器）、
//! `tests/channel_strip_insert.rs`（通道条）与 `tests/reverb_insert.rs`（Freeverb 混响）
//! 是**同一条插入链上的第四件器件**（第三件插入器件）：
//! `c792fdc` 接压缩器、`db1850f` 把槽位换成通道条、`line/engine-reverb` 加 Freeverb 混响、
//! 本票在**同一条链**上加卷积混响（`yeban_dsp::convolution_reverb`）。
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | V0 | **没有已识别 `conv_` 参数**的四种口径（参数为空 / 参数不认识（含 `reverb_mix`、`reverb_width`、`ir_frames`）/ `bypassed` / 内置乐器）与"没有设备链"逐字节相同，且 `insert_convolution_frames == 0` | 收下不带 `conv_` 前缀的名字 / 对"没有参数的设备"也挂卷积混响 / bypass 后偷偷取默认参数 |
//! | V1 | 命中的设备**真的武装进实时侧**（`armed_convolution(轨)` 逐字段逐位等于快照投影 ＋ 武装的 IR 帧数 = 引擎常量）＋ `insert_convolution_frames` 等于渲染帧数 | 快照里有、实时侧没读（`armed_convolution` 为 `None` / 计数恒 0） |
//! | V2 | **卷积真的加上了尾巴**：在**测得**的"干信号逐位静音"窗口里，未武装的渲染逐位静音而武装的渲染有能量 | 只有投影没有处理 / 把湿路接到不参与混音的地方 / IR 长度或位置错了 |
//! | V3 | `conv_wet` 与 `conv_ir_decay_s` **方向正确**：湿声与衰减变长都让尾窗能量单调上升 | 参数投影到错误的字段 / IR 合成器忽略旋钮 |
//! | V4 | **确定性**：两次独立装配逐位相同；同一装配里周期性发布**等价**快照（重新武装）也不改变输出 | 引入真熵源 / 每次重新武装都重设 IR（那会复位器件的时间状态） |
//! | V5 | **不同量子数（块切分）**下前缀逐位相同（[ARCH-DET-001] 的引擎侧对账） | 用了跨量子残留的临时缓冲 / 量子边界上有一次性副作用 |
//! | V6 | 引擎的 `ConvolutionReverb` **就是** `yeban_dsp::convolution_reverb::ConvolutionReverb`（类型 + `new`/`set_params`/`set_impulse_response`/`process` 函数地址同一性 ＋ **零延迟**契约） | 在引擎侧留第二份卷积实现 / 器件带延迟却登记 0 |
//! | V7 | 第一个**含已识别 `conv_` 参数**的效果器是唯一来源；更早的无关效果器不参与、更晚的不覆盖；只带通道条参数或只带 Freeverb 参数的设备**不**产生卷积混响 | 取"最后一个"或"任意一个"设备 / 把别的器件的参数名当卷积参数 |
//! | V8 | `conv_wet = 0` 是**逐位直通**（引擎在 `is_active()` 上整段跳过） | 把 `is_active()` 守卫删掉 |
//! | V9 | **换采样率的快照不武装卷积混响**（IR 的帧数由采样率决定 ⇒ 那条边界上重设 IR 会分配）：拒绝计数按设备数累加、槽位归零、`insert_convolution_frames` 不再增长；换回 48 kHz ⇒ 重新武装 | 在快照边界无条件下重设 IR（那会在音频线程分配） |
//! | V10 | **改了 IR 旋钮就必须换 IR**（内容标识那道闸门不许把合法的变更一起挡掉）：把 `conv_ir_decay_s` 改掉再发布 ⇒ 输出与"一直发等价快照"逐位不同 | 把内容标识的比较写成恒真 / 永不重设 IR |
//!
//! ## 跨提交对账（**不硬编码哈希**）
//!
//! V0 把未武装的原始样本落盘：`$TMPDIR/yeban-convolution-unarmed.raw`
//! （交错小端 `f32`）。判据刻意**不**把哈希写进源码：那会把"本机 libm 的某一位"
//! 当成规范常数。实测读数见本票的交付报告（注释里只写"怎么量"，不写"量到多少"）。
//!
//! ## 分类（[ADR-0001 D32]）—— 本文件**不**声称跨架构逐位相同
//!
//! 卷积核的旋转因子表在**构造期**由宿主 libm 的 `f32::cos`/`f32::sin` 算出
//! （`crates/yeban-dsp/src/convolution.rs`，现位于第 309 行）⇒ 本文件的全部
//! "逐位相同"断言都属于**超越函数类**：它们在**冻结架构（aarch64，本项目基准）**上、
//! 同一工具链内是硬判据；**不**跨架构成立。IR 的**合成**本身是 IEEE 精确类
//! （`Rng` ＋ `libm::expf` ＋ `abs`/`max`/除法，见 `crate::insert` 模块文档 §9.2），
//! 但 IR 只是输入，输出还要过卷积核。
//!
//! ## 覆盖范围（**明说**）
//!
//! 本文件覆盖：`insert.rs` 的卷积混响投影与 IR 合成、`snapshot.rs` 的插入链字段、
//! `rt.rs` 的**构造期预建** / **快照边界武装** / **逐样本调用** / **采样率守卫** /
//! **内容标识闸门**。它**不**覆盖 `yeban-dsp` 内部卷积算法的正确性（那是
//! `crates/yeban-dsp/src/convolution*.rs` 与 `crates/yeban-dsp/tests/convolution_rt_zero_alloc.rs`
//! 的职责），也**不**覆盖"真立体声四条独立 IR"（引擎的插入点是单声道 ⇒ 投影交的是
//! `h_LL = h_RR`、交叉通路全零，见 `crate::insert` 模块文档 §9.4）。

mod support;

use std::ops::Range;
use std::path::Path;

use support::{NoteSpec, note_project, render, render_with, rms_peak};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};

/// 判据用的栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本（见 `support` 模块文档）。
///
/// 音符在 tick 2880 结束 = 样本 72,000（1.5 s）。尾窗的位置**不**写死在这里：
/// V2 从**未武装**的渲染里量出"最后一个非零样本"，窗口紧接在它之后
/// （卷积的尾巴只有 IR 那么长，写死一个 2 秒后的窗口会量到"尾巴早就走完了"）。
const NOTES: [NoteSpec; 4] = [
    NoteSpec::at(0, 480, 60, 127),
    NoteSpec::at(480, 480, 64, 110),
    NoteSpec::at(960, 960, 67, 100),
    NoteSpec::at(1920, 960, 72, 90),
];

/// 渲染量子数：900 × 128 = 115,200 帧 = 2.4 s @48 kHz。
///
/// 比混响那一条（6.4 s）短得多：卷积混响的尾巴**只有 IR 那么长**（0.1 s），
/// 2.4 s 已经足够覆盖"干声的释放走完 + 再补一条 IR"。
const QUANTA: usize = 900;

/// 采样率（Hz）：`support` 的夹具固定用它 ⇒ IR 帧数 = `48_000 ÷ 10`。
const SAMPLE_RATE: u32 = 48_000;

/// 引擎常量的 IR 帧数（与 [`yeban_engine::insert::convolution_ir_frames`] 必须同值）。
const IR_FRAMES: usize = 4_800;

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

/// 把交错样本按小端 `f32` 落盘，返回 `(路径, 字节数)`（与 C0/C8/R0 同款）。
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

/// 挂一台**全湿**卷积混响设备（`dry = 0`、`wet = 1`、无预延迟）的夹具。
///
/// `dry = 0` 是刻意的：输出因此**只有**湿路 ⇒ "尾窗里的能量只能来自卷积"这条归因
/// 不需要额外论证。
fn wet_project() -> (YebanProjectV1, EntityId) {
    let mut fixture = note_project(&NOTES);
    let track = fixture.track;
    mount(
        &mut fixture.project,
        track,
        vec![effect(
            "Convolution",
            &[
                ("conv_dry", 0.0),
                ("conv_wet", 1.0),
                ("conv_ir_decay_s", 0.35),
                ("conv_ir_seed", 7.0),
            ],
        )],
    );
    (fixture.project, track)
}

/// V0：**未武装**的四种口径必须与"没有设备链"逐字节相同
/// （`crate::insert` 模块文档 §9.3）。
///
/// 四种输入的唯一差别是设备链：
///
/// 1. `params` 为空的效果器（只为上报延迟而存在的那一类）；
/// 2. 参数名一个都不认识的效果器 —— 别的器件已经占用的名字（`reverb_mix`、
///    `reverb_width`、`filter_cutoff_hz`）加上**看起来像但不是** `conv_` 前缀的
///    `ir_frames` / `ir_data` / `convolution_ir`；
/// 3. `bypassed = true` 但 `conv_` 参数**命中**；
/// 4. `InternalInstrument` 设备带 `conv_` 参数（乐器不是插入器件）。
#[test]
fn projects_without_recognised_convolution_params_are_bit_identical() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    let bare = render(&fixture.project, QUANTA);

    let mut unknown_project = fixture.project.clone();
    mount(&mut unknown_project, track, vec![effect("Unknown", &[])]);
    let unknown = render(&unknown_project, QUANTA);

    let mut unrelated_project = fixture.project.clone();
    mount(
        &mut unrelated_project,
        track,
        vec![effect(
            "Unrelated",
            &[
                ("reverb_mix", 0.3),
                ("reverb_width", 0.9),
                ("filter_cutoff_hz", 800.0),
                ("ir_frames", 4_800.0),
                ("ir_data", 1.0),
                ("convolution_ir", 1.0),
            ],
        )],
    );
    let unrelated = render(&unrelated_project, QUANTA);

    let mut bypassed_project = fixture.project.clone();
    let mut bypassed_device = effect(
        "Convolution",
        &[("conv_wet", 1.0), ("conv_ir_decay_s", 0.35)],
    );
    bypassed_device.bypassed = true;
    mount(&mut bypassed_project, track, vec![bypassed_device]);
    let bypassed = render(&bypassed_project, QUANTA);

    let mut instrument_project = fixture.project.clone();
    let mut instrument = effect("Hollow", &[("conv_wet", 1.0)]);
    instrument.kind = DeviceKind::InternalInstrument;
    mount(&mut instrument_project, track, vec![instrument]);
    let instrument = render(&instrument_project, QUANTA);

    let (path, bytes) = dump("yeban-convolution-unarmed.raw", &bare.left, &bare.right);
    println!(
        "[engine-7/V0] 未武装: 落盘={} 字节={bytes} 帧={} 指纹={:#018x}",
        path.display(),
        bare.frames(),
        bare.fingerprint()
    );

    assert!(bare.frames() > 0, "夹具必须真的渲染出帧");
    assert!(bare.peak() > 0.0, "夹具必须真的出声（否则逐位相同是空转）");
    for (label, other) in [
        ("参数为空的效果器", &unknown),
        (
            "参数不认识的效果器（别的器件的名字 ＋ 像但不是 conv_ 的名字）",
            &unrelated,
        ),
        ("bypassed 的卷积混响", &bypassed),
        ("内置乐器设备带 conv_ 参数", &instrument),
    ] {
        assert_eq!(
            bare.left_bits(),
            other.left_bits(),
            "{label}: 输出必须与没有设备链时逐位相同（指纹 {:#018x} vs {:#018x}）",
            bare.fingerprint(),
            other.fingerprint()
        );
        assert_eq!(bare.fingerprint(), other.fingerprint(), "{label}: 指纹相同");
        assert_eq!(
            other.stats.insert_convolution_frames, 0,
            "{label}: 未武装 ⇒ 卷积混响处理帧数必须是 0"
        );
        assert_eq!(
            other.stats.insert_convolution_rejects, 0,
            "{label}: 未武装不是'配置不足' ⇒ 拒绝计数必须是 0"
        );
    }
}

/// V1：命中的设备**真的武装进了实时侧**（三层证据，缺一不可）。
///
/// 1. **快照层**：`EngineSnapshot::insert_params(轨).convolution()` 给出投影
///    （模型 → 快照）；
/// 2. **实时层**：`EngineRuntime::armed_convolution(轨)` 逐字段逐位等于第 1 层
///    （快照 → 逐样本路径的武装表），且武装的 IR 帧数 = 引擎常量；
/// 3. **行为层**：`EngineStats::insert_convolution_frames` 等于渲染帧数。
#[test]
fn recognised_convolution_is_armed_into_the_realtime_path() {
    let (project, track) = wet_project();

    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&project, 1).expect("夹具快照");
    let plan = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::convolution)
        .expect("命中的效果器设备必须投影出卷积混响计划");
    let projected = plan.params();
    let projected_hash = plan.ir_hash();
    assert_eq!(plan.ir_frames(), IR_FRAMES, "IR 帧数必须等于引擎常量");
    assert_eq!(
        IR_FRAMES,
        yeban_engine::insert::convolution_ir_frames(SAMPLE_RATE),
        "本文件的 IR 常量必须与引擎的换算函数同值"
    );

    let mut captured: Option<yeban_engine::insert::ConvolutionReverbParams> = None;
    let mut armed_frames: Option<usize> = None;
    let mut slots = usize::MAX;
    let mut armed_sample_rate = 0u32;
    let rendered = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum == 1 {
            captured = rig.runtime.armed_convolution(&track);
            armed_frames = rig.runtime.armed_convolution_ir_frames(&track);
            slots = rig.runtime.armed_convolution_slot_count();
            armed_sample_rate = rig.runtime.armed_convolution_sample_rate();
        }
    });

    let armed = captured.expect("命中的卷积混响必须被武装进实时侧");
    for (name, actual, expected) in [
        ("pre_delay_s", armed.pre_delay_s, projected.pre_delay_s),
        ("dry", armed.dry, projected.dry),
        ("wet", armed.wet, projected.wet),
        ("ir_gain_db", armed.ir_gain_db, projected.ir_gain_db),
    ] {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "武装表的 {name} 必须逐位等于快照投影: 武装={actual} 投影={expected}"
        );
    }
    assert_eq!(
        armed_frames,
        Some(IR_FRAMES),
        "器件真正接受的 IR 帧数必须等于投影的帧数（差 ⇒ 交进去的 IR 长度不对）"
    );
    assert_eq!(slots, 1, "夹具只有一台卷积混响 ⇒ 只应占一个槽位");
    assert_eq!(
        armed_sample_rate, SAMPLE_RATE,
        "IR 缓冲池必须按初始快照的采样率预建"
    );
    // 投影的值确实不是默认值（否则"接线了"与"没接线"读数相同）。
    let default = yeban_engine::insert::ConvolutionReverbParams::default();
    assert_ne!(armed.dry.to_bits(), default.dry.to_bits());
    assert_ne!(armed.wet.to_bits(), default.wet.to_bits());
    assert_eq!(projected_hash, plan.ir_hash(), "内容标识必须随计划一起走");

    let stats = rendered.stats;
    println!(
        "[engine-7/V1] 武装: 槽位={slots} 采样率={armed_sample_rate} IR帧数={IR_FRAMES} \
         卷积处理帧数={} 渲染帧数={} 峰值={:.6}",
        stats.insert_convolution_frames,
        rendered.frames(),
        rendered.peak()
    );
    // 行为层：卷积处理帧数 = 渲染帧数（逐样本路径真的调用了器件；`wet = 1` ⇒ 每个量子都处理）。
    assert_eq!(
        stats.insert_convolution_frames,
        rendered.frames() as u64,
        "卷积处理帧数必须等于渲染帧数（差 ⇒ 逐样本路径漏掉了这一级）"
    );
    assert_eq!(
        stats.insert_convolution_rejects, 0,
        "采样率没有变、IR 合法 ⇒ 不许出现拒绝"
    );
}

/// V2：**卷积混响真的加上了尾巴**。
///
/// 尾窗**从测量得到**，不写死：先从未武装的渲染里找出"最后一个非零样本"，
/// 窗口取 `last+1 .. last+1+IR_FRAMES`。
///
/// 为什么必须这样：IR 只有 [`IR_FRAMES`] 帧（0.1 s）⇒ 卷积尾巴的长度也是 0.1 s。
/// 写死一个"最后一个音符结束 0.5 秒之后"的窗口只会量到一段本来就该是静音的地方。
///
/// 归因的唯一性：未武装的渲染在同一窗口里必须**逐位静音**（干信号真的走完了），
/// 因此武装的渲染在那个窗口里的任何能量都只能来自卷积。
#[test]
fn the_convolution_really_adds_a_tail_after_the_dry_signal_ends() {
    let fixture = note_project(&NOTES);
    let (wet, _track) = wet_project();

    let bare = render(&fixture.project, QUANTA);
    let convolved = render(&wet, QUANTA);

    let last_dry = bare
        .left
        .iter()
        .rposition(|sample| *sample != 0.0)
        .expect("未武装的夹具必须真的出声");
    let tail: Range<usize> = (last_dry + 1)..(last_dry + 1 + IR_FRAMES);
    assert!(
        tail.end <= bare.left.len(),
        "渲染窗口必须长到能覆盖整条 IR 的尾巴：需要 {} 帧，实得 {}",
        tail.end,
        bare.left.len()
    );

    let (bare_tail_rms, bare_tail_peak) = rms_peak(&bare.left[tail.clone()]);
    let (wet_tail_rms, wet_tail_peak) = rms_peak(&convolved.left[tail.clone()]);
    println!(
        "[engine-7/V2] 干信号最后一个非零样本下标={last_dry} 尾窗={tail:?}（帧）：\
         未武装 rms={bare_tail_rms:.9} 峰值={bare_tail_peak:.9}；\
         武装 rms={wet_tail_rms:.9} 峰值={wet_tail_peak:.9}（线性幅度）"
    );
    assert_eq!(
        bare_tail_peak, 0.0,
        "未武装的渲染在尾窗里必须逐位静音（峰值 {bare_tail_peak}）—— 否则本判据的归因不唯一"
    );
    assert!(
        wet_tail_peak > 1e-4,
        "武装卷积混响的渲染在同一个尾窗里必须有能量，实测峰值 {wet_tail_peak}"
    );
    assert!(
        wet_tail_rms > 1e-5,
        "尾窗 RMS 必须 > 1e-5（线性幅度），实测 {wet_tail_rms}"
    );
}

/// 尾窗（V3 用）：与 [`the_convolution_really_adds_a_tail_after_the_dry_signal_ends`]
/// 同一个测法，供方向判据复用。
fn measured_tail(bare_left: &[f32]) -> Range<usize> {
    let last_dry = bare_left
        .iter()
        .rposition(|sample| *sample != 0.0)
        .expect("未武装的夹具必须真的出声");
    (last_dry + 1)..(last_dry + 1 + IR_FRAMES)
}

/// V3：`conv_wet` 与 `conv_ir_decay_s` **方向正确**。
///
/// 湿声变大 ⇒ 尾窗能量上升；衰减时间常数变长 ⇒ 同一个尾窗的能量上升。
/// 两条都单调（同一个窗口、同一份干声）。
#[test]
fn the_wet_and_decay_knobs_move_the_tail_in_the_right_direction() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;
    let bare = render(&fixture.project, QUANTA);
    let tail = measured_tail(&bare.left);

    let mut energies = Vec::new();
    for wet in [0.2f32, 0.6, 1.0] {
        let mut project = fixture.project.clone();
        mount(
            &mut project,
            track,
            vec![effect(
                "Convolution",
                &[
                    ("conv_dry", 0.0),
                    ("conv_wet", wet),
                    ("conv_ir_decay_s", 0.35),
                    ("conv_ir_seed", 7.0),
                ],
            )],
        );
        let rendered = render(&project, QUANTA);
        let (rms, _) = rms_peak(&rendered.left[tail.clone()]);
        energies.push((wet, rms));
    }
    println!("[engine-7/V3] 湿声 → 尾窗 rms: {energies:?}（线性幅度）");
    for pair in energies.windows(2) {
        assert!(
            pair[1].1 > pair[0].1,
            "湿声 {} → {} 必须让尾窗能量上升：{:.9} → {:.9}",
            pair[0].0,
            pair[1].0,
            pair[0].1,
            pair[1].1
        );
    }

    let mut decays = Vec::new();
    for decay in [0.05f32, 0.15, 0.5] {
        let mut project = fixture.project.clone();
        mount(
            &mut project,
            track,
            vec![effect(
                "Convolution",
                &[
                    ("conv_dry", 0.0),
                    ("conv_wet", 1.0),
                    ("conv_ir_decay_s", decay),
                    ("conv_ir_seed", 7.0),
                ],
            )],
        );
        let rendered = render(&project, QUANTA);
        let (rms, _) = rms_peak(&rendered.left[tail.clone()]);
        decays.push((decay, rms));
    }
    println!("[engine-7/V3] 衰减 → 尾窗 rms: {decays:?}（线性幅度）");
    for pair in decays.windows(2) {
        assert!(
            pair[1].1 > pair[0].1,
            "衰减 {} → {} 必须让尾窗能量上升：{:.9} → {:.9}",
            pair[0].0,
            pair[1].0,
            pair[0].1,
            pair[1].1
        );
    }
    assert!(fixture.project.tracks.contains_key(&track));
}

/// V4：**确定性**（两次独立装配逐位相同；等价快照的周期发布不改变输出）。
///
/// 第二半是"重新武装不得复位器件状态"的机械判据：器件的时间状态是**频域延迟线与
/// 重叠相加尾**，一次同长度但无条件的 `set_impulse_response` 会把它们清零
/// （那个方法末尾的 `reset()`）⇒ 尾巴被切断。
///
/// ⚠ 这一条正是 `ConvolutionPlan::ir_hash` 那道闸门存在的理由：应用在**每次工程编辑**
/// 时都会发布新快照，没有闸门则每次编辑都切一遍全部卷积尾巴。
#[test]
fn two_independent_runs_with_an_armed_convolution_are_bit_identical() {
    let (project, _track) = wet_project();

    let first = render(&project, QUANTA);
    let second = render(&project, QUANTA);
    assert_eq!(
        first.left_bits(),
        second.left_bits(),
        "两次独立装配必须逐位相同（指纹 {:#018x} vs {:#018x}）",
        first.fingerprint(),
        second.fingerprint()
    );
    assert_eq!(first.fingerprint(), second.fingerprint());

    // 同一个装配里每 128 个量子发布一份**等价**快照（只改 revision）⇒ 重新武装。
    let switched = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum > 0 && quantum % 128 == 0 {
            rig.publish_equivalent(&project, (quantum / 128) as u64 + 100);
        }
    });
    assert_eq!(
        first.left_bits(),
        switched.left_bits(),
        "周期性重新武装不得改变输出（指纹 {:#018x} vs {:#018x}）—— 内容标识相同 ⇒ 只许 set_params",
        first.fingerprint(),
        switched.fingerprint()
    );
    assert_eq!(
        switched.stats.insert_convolution_rejects, 0,
        "等价快照不是'配置不足' ⇒ 拒绝计数必须是 0"
    );
}

/// V5：**不同量子数（块切分）**下前缀逐位相同（[ARCH-DET-001] 的引擎侧对账）。
#[test]
fn different_quantum_counts_agree_on_the_shared_prefix() {
    let (project, _track) = wet_project();

    let short = render(&project, 37);
    let long = render(&project, 400);
    assert_eq!(
        short.left_bits(),
        long.left_bits()[..short.left.len()],
        "37 个量子的前缀必须与 400 个量子的前缀逐位相同"
    );
    assert_eq!(
        short.right.as_slice(),
        &long.right[..short.right.len()],
        "右声道同样"
    );
    assert!(short.peak() > 0.0, "短渲染必须真的出声");
}

/// V6：引擎的 `ConvolutionReverb` **就是** dsp 的卷积混响（类型 + 函数地址同一性 + 零延迟）。
#[test]
fn engine_convolution_reverb_is_literally_the_dsp_convolution_reverb() {
    use yeban_engine::insert::{ConvolutionReverb, DspConvolutionReverb};

    // 两条"四条 IR 一起设"的函数指针类型（`clippy::type_complexity` 要求先命名）。
    type SetIr = fn(&mut ConvolutionReverb, &[f32], &[f32], &[f32], &[f32]) -> usize;
    type SetIrDsp = fn(&mut DspConvolutionReverb, &[f32], &[f32], &[f32], &[f32]) -> usize;

    // 类型同一性：把 engine 路径构造的值赋给 dsp 路径的类型。
    let engine: ConvolutionReverb = ConvolutionReverb::new();
    let as_dsp: DspConvolutionReverb = engine;
    assert!(
        !as_dsp.is_configured(),
        "刚构造的卷积混响必须是未配置的（直通）"
    );
    assert!(
        !as_dsp.is_active(),
        "未配置器件必须自己报告'不可闻'（引擎的整段跳过靠它）"
    );

    // 函数地址同一性（`new` / `set_params` / `set_impulse_response` / `process`）。
    let engine_new: fn() -> ConvolutionReverb = ConvolutionReverb::new;
    let dsp_new: fn() -> DspConvolutionReverb = DspConvolutionReverb::new;
    assert_eq!(
        engine_new as usize, dsp_new as usize,
        "engine 的 ConvolutionReverb::new 必须就是 dsp 的那个函数"
    );
    let engine_params: fn(&mut ConvolutionReverb, yeban_engine::insert::ConvolutionReverbParams) =
        ConvolutionReverb::set_params;
    let dsp_params: fn(&mut DspConvolutionReverb, yeban_engine::insert::ConvolutionReverbParams) =
        DspConvolutionReverb::set_params;
    assert_eq!(
        engine_params as usize, dsp_params as usize,
        "engine 的 ConvolutionReverb::set_params 必须就是 dsp 的那个函数"
    );
    let engine_ir: SetIr = ConvolutionReverb::set_impulse_response;
    let dsp_ir: SetIrDsp = DspConvolutionReverb::set_impulse_response;
    assert_eq!(
        engine_ir as usize, dsp_ir as usize,
        "engine 的 ConvolutionReverb::set_impulse_response 必须就是 dsp 的那个函数"
    );
    let engine_process: fn(&mut ConvolutionReverb, &mut [f32]) -> usize =
        ConvolutionReverb::process;
    let dsp_process: fn(&mut DspConvolutionReverb, &mut [f32]) -> usize =
        DspConvolutionReverb::process;
    assert_eq!(
        engine_process as usize, dsp_process as usize,
        "engine 的 ConvolutionReverb::process 必须就是 dsp 的那个函数"
    );

    // **零延迟契约**：器件登记 `CONV_LATENCY = 0` ⇒ 该轨的 PDC 补偿必须是 0
    // （否则"卷积接在 PDC 之前"这条位置说明会让相位错位）。
    let (project, track) = wet_project();
    let mut delay = None;
    let _ = render_with(&project, 8, 1, |quantum, rig| {
        if quantum == 1 {
            delay = rig.runtime.armed_pdc_delay(&track);
        }
    });
    assert_eq!(delay, Some(0), "零延迟器件不得让该轨获得 PDC 补偿");
}

/// V7：**首个来源**规则 + 别的器件的参数**不**产生卷积混响
/// （`crate::insert` 模块文档 §9 的 `from_devices` 文档）。
#[test]
fn the_first_recognised_effect_in_the_chain_is_the_only_convolution_source() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    // 更早的无关效果器不参与，更晚的卷积混响不覆盖更早的。
    let mut ordered = fixture.project.clone();
    mount(
        &mut ordered,
        track,
        vec![
            effect("Unrelated", &[("reverb_mix", 0.3)]),
            effect("First", &[("conv_ir_decay_s", 0.5)]),
            effect("Later", &[("conv_ir_decay_s", 0.02)]),
        ],
    );
    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&ordered, 1).expect("快照");
    let first = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::convolution)
        .expect("第二个设备必须命中");
    let later = {
        let mut only_later = fixture.project.clone();
        mount(
            &mut only_later,
            track,
            vec![effect("Later", &[("conv_ir_decay_s", 0.02)])],
        );
        let snapshot =
            yeban_engine::snapshot::EngineSnapshot::from_project(&only_later, 1).expect("快照");
        snapshot
            .insert_params(&track)
            .and_then(yeban_engine::insert::InsertParams::convolution)
            .expect("第三个设备也能命中")
            .ir_hash()
    };
    assert_ne!(
        first.ir_hash(),
        later,
        "首个含已识别名字的设备是唯一来源（它的 IR 必须与'只有第三个设备'时不同）"
    );

    // 只带通道条参数或只带 Freeverb 参数的设备**不**产生卷积混响。
    let mut others_only = fixture.project.clone();
    mount(
        &mut others_only,
        track,
        vec![
            effect(
                "Strip",
                &[
                    ("threshold_db", -24.0),
                    ("ratio", 4.0),
                    ("eq_low_gain", 3.0),
                ],
            ),
            effect("Reverb", &[("reverb_size", 0.9), ("reverb_wet", 0.5)]),
        ],
    );
    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&others_only, 1).expect("快照");
    let insert = snapshot.insert_params(&track).expect("通道条必须命中");
    assert!(insert.strip().is_some(), "通道条必须命中");
    assert!(insert.reverb().is_some(), "Freeverb 混响必须命中");
    assert!(
        insert.convolution().is_none(),
        "只有别的器件的参数不许产生卷积混响"
    );
}

/// V8：`conv_wet = 0` 是**逐位直通**（引擎在 `ConvolutionReverb::is_active()` 上整段跳过）。
///
/// ⚠ 口径与器件内部不同、且这是**明说的**：器件即使 `wet == 0` 也照样算干湿混合
/// （它的模块文档 §2 说明了为什么不能短路），而**引擎**把 `wet ≤ 1e-4` 当作
/// "这个器件不可闻"、整段跳过 —— 与 Freeverb 那一级的 R8 同款。
#[test]
fn a_wet_of_zero_is_a_bit_identical_passthrough() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    let bare = render(&fixture.project, QUANTA);

    let mut silent_wet = fixture.project.clone();
    mount(
        &mut silent_wet,
        track,
        vec![effect(
            "Convolution",
            &[("conv_wet", 0.0), ("conv_ir_decay_s", 0.35)],
        )],
    );
    let rendered = render(&silent_wet, QUANTA);
    println!(
        "[engine-7/V8] wet=0: 卷积处理帧数={} 指纹={:#018x} vs 未武装 {:#018x}",
        rendered.stats.insert_convolution_frames,
        rendered.fingerprint(),
        bare.fingerprint()
    );
    assert_eq!(
        bare.left_bits(),
        rendered.left_bits(),
        "湿声 0 ⇒ 必须与没有卷积混响逐位相同（指纹 {:#018x} vs {:#018x}）",
        bare.fingerprint(),
        rendered.fingerprint()
    );
    assert_eq!(
        rendered.stats.insert_convolution_frames, 0,
        "湿声 0 ⇒ 引擎整段跳过，'处理过帧数'必须是 0"
    );
}

/// V9：**换采样率的快照不武装卷积混响**（IR 的帧数由采样率决定 ⇒ 那条边界上重设 IR
/// 会重建缓冲 = 在音频线程分配）。
///
/// 观测方式：先用 48 kHz 的快照渲染（武装），再发布一份 **44.1 kHz** 的快照：
///
/// 1. 拒绝计数 += 这一份快照里的卷积混响**设备数**、槽位归零；
/// 2. 那一份快照的量子不再推进 `insert_convolution_frames`；
/// 3. 换回 48 kHz ⇒ 重新武装（守卫是"拒绝这一份"，不是"永久停用"）。
#[test]
fn a_sample_rate_change_does_not_rebuild_the_convolution_buffers() {
    let (project, _track) = wet_project();

    let mut rejects = 0u64;
    let mut slots_after_shift = usize::MAX;
    let mut frames_after_shift = 0u64;
    let mut slots_after_restore = usize::MAX;
    let mut shift_quantum = usize::MAX;
    let mut scratch = [0.0f32; 128 * 2];

    let rendered = render_with(&project, 24, 1, |quantum, rig| {
        if quantum == 12 {
            // 发布一份换采样率的快照（44.1 kHz）：IR 缓冲不为它重建。
            let mut shifted = project.clone();
            shifted.audio_config.sample_rate = yeban_model::SampleRate::Hz44100;
            rig.publish_equivalent(&shifted, 2);
        }
        if quantum == 16 {
            let before = rig.runtime.stats().insert_convolution_frames;
            // 再推一个量子：这一份快照里卷积混响必须**整段不武装**。
            rig.runtime.process_quantum(&mut scratch, 2);
            let after = rig.runtime.stats();
            frames_after_shift = after.insert_convolution_frames.saturating_sub(before);
            slots_after_shift = rig.runtime.armed_convolution_slot_count();
            rejects = after.insert_convolution_rejects;
            shift_quantum = quantum;
            // 换回 48 kHz：必须重新武装。
            rig.publish_equivalent(&project, 3);
            rig.runtime.process_quantum(&mut scratch, 2);
            slots_after_restore = rig.runtime.armed_convolution_slot_count();
        }
    });

    println!(
        "[engine-7/V9] 换采样率: 第 {shift_quantum} 个量子后 拒绝累计={rejects} \
         武装槽位={slots_after_shift} 该量子新增处理帧数={frames_after_shift}；\
         换回 48 kHz 后槽位={slots_after_restore}；总渲染帧={}",
        rendered.frames()
    );
    assert_eq!(
        rejects, 1,
        "换采样率必须按'这一份快照里的卷积混响设备数'累加拒绝（夹具是 1 台）"
    );
    assert_eq!(slots_after_shift, 0, "换采样率之后卷积混响必须整段不武装");
    assert_eq!(frames_after_shift, 0, "不武装的量子不许推进'卷积处理帧数'");
    assert_eq!(slots_after_restore, 1, "换回 48 kHz 后必须重新武装");
    assert!(rendered.peak() > 0.0, "夹具必须真的出声");
}

/// V10：**改了 IR 旋钮就必须换 IR**（内容标识那道闸门不许把合法变更一起挡掉）。
///
/// 三条渲染：
///
/// 1. `steady`：不发布任何新快照；
/// 2. `equivalent`：周期性发布**等价**快照（IR 的内容标识不变）；
/// 3. `changed`：在同一个量子位置发布一份 `conv_ir_decay_s` **改过**的快照。
///
/// 断言：`equivalent` 与 `steady` 逐位相同（V4 的第二半），而 `changed` 与 `steady`
/// **不同** ⇒ 闸门放行了真正的变更。
#[test]
fn a_changed_ir_knob_reaches_the_device() {
    let (project, track) = wet_project();
    let mut changed_project = project.clone();
    mount(
        &mut changed_project,
        track,
        vec![effect(
            "Convolution",
            &[
                ("conv_dry", 0.0),
                ("conv_wet", 1.0),
                ("conv_ir_decay_s", 0.05),
                ("conv_ir_seed", 7.0),
            ],
        )],
    );

    let steady = render(&project, QUANTA);
    let equivalent = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum > 0 && quantum % 128 == 0 {
            rig.publish_equivalent(&project, (quantum / 128) as u64 + 100);
        }
    });
    let changed = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum > 0 && quantum % 128 == 0 {
            rig.publish_equivalent(&changed_project, (quantum / 128) as u64 + 200);
        }
    });

    let differences = steady
        .left_bits()
        .iter()
        .zip(changed.left_bits().iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "[engine-7/V10] 等价重发差异样本={}；改衰减后差异样本={differences}；\
         指纹 steady={:#018x} equivalent={:#018x} changed={:#018x}",
        steady
            .left_bits()
            .iter()
            .zip(equivalent.left_bits().iter())
            .filter(|(a, b)| a != b)
            .count(),
        steady.fingerprint(),
        equivalent.fingerprint(),
        changed.fingerprint()
    );
    assert_eq!(
        steady.left_bits(),
        equivalent.left_bits(),
        "内容标识相同 ⇒ 只许 set_params，输出必须逐位不变"
    );
    assert!(
        differences > 0,
        "改了 `conv_ir_decay_s` 之后输出必须有差别（否则内容标识那道闸门把合法变更也挡掉了）"
    );
    assert_ne!(steady.fingerprint(), changed.fingerprint());
}
