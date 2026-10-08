//! 节拍器（`transport.metronome_enabled`）的**端到端**判据。
//! [ARCH-DET-001, MUST-GATE-001, MODEL-AST-002]
//!
//! ## 这些判据在测什么
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | M0 | 节拍器**关闭**（默认）时，混音链的输出与"从未接过节拍器"逐字节相同（落盘原始样本，跨提交比 sha256） | 关闭时仍混入咔哒声 / 悄悄改了别的接线 |
//! | M1 | 咔哒声只出现在**拍边界**的窗口里，且每个拍点都真的出声（多速度、逐帧断言） | 不发声 / 拍点算错 / 到处撒咔哒声 |
//! | M2 | 强弱拍：第 1 拍 `0.5`、第 2 拍 `0.25`，且弱拍是强拍的**逐位** `× 0.5` | 不区分小节第一拍 / 增益不在构造期定 |
//! | M3 | 咔哒声**不驱动**母线限制器（峰值 0.5 < 阈值 0.9 ⇒ 一次压限都没有） | 增益过大 / 混在限制器之后 |
//! | M4 | 停住（`Stop`）时咔哒声立即静音（不留 4 ms 尾巴），恢复播放后重新从拍栅格起 | 停住时继续放尾巴 |
//! | M5 | 同输入两次独立装配 + 渲染**逐位相同**（含咔哒声） | 引入真熵源 |
//! | M6 | 播放中换拍号（4/4 → 3/2）⇒ 拍格**重对齐**（旧格上的咔哒声必须消失） | 换快照时把旧的下拍 tick 带进新格 |
//!
//! ## 证据的落盘（M0 的跨提交比对）
//!
//! M0 把交错样本按**小端 `f32`** 写成 `$TMPDIR/yeban-metronome-disabled.raw`
//! （M1 写 `…-enabled.raw`）⇒ 提交前后各跑一次、`shasum -a 256` 两张原始样本
//! 即可判"关闭节拍器时逐字节不变"。判据刻意**不**把哈希硬编码进源码：
//! 那会把"本机 libm 的某一位"当成规范常数（跨平台金标准不是本票的事）。

mod support;

use std::path::Path;

use support::{NoteSpec, Render, empty_project, note_project, render};
use yeban_model::PPQ;
use yeban_model::YebanProjectV1;

/// 判据用的采样率：默认工程 `audio_config.sample_rate`（快照投影的来源）。
const SAMPLE_RATE: u64 = 48_000;

/// 一拍几个 tick：4/4 拍号下 = `PPQ`（960）。
const TICKS_PER_BEAT: u64 = PPQ;

/// 构造夹具：**只有主总线、没有音符** ⇒ 输出里除了咔哒声什么都没有。
///
/// 这样"咔哒声真的出现"这件事不被音乐掩盖（峰值判据因此是字面读数）。
fn metronome_project(bpm: f64, enabled: bool) -> YebanProjectV1 {
    let mut project = empty_project();
    project.bpm = bpm;
    project.transport.metronome_enabled = enabled;
    project
}

/// 速度量化（与 `transport::quantise_bpm` 同口径：微 BPM 四舍五入）。
///
/// 判据自己算一遍是**故意的**：它把"拍点的帧位置"变成可从 BPM 推出的**预测**，
/// 而不是把渲染结果当成定义（否则判据只是复述实现）。
fn tick_num(bpm: f64) -> u128 {
    let micro = (bpm * 1_000_000.0).round() as u128;
    micro * u128::from(PPQ)
}

/// 每帧推进的 tick 数的分母（`60 × 1e6 × sample_rate`）。
fn tick_den() -> u128 {
    60 * 1_000_000 * u128::from(SAMPLE_RATE)
}

/// 第 `beat`（0 起）拍的**绝对帧位置**：`ceil(beat × 960 × tick_den / tick_num)`。
///
/// 这就是 [`yeban_engine::transport::Transport::frames_until_tick`] 从 tick 0
/// 起算的逆运算（推导见该函数的文档）。
fn beat_frame(bpm: f64, beat: u64) -> usize {
    let numerator = u128::from(beat) * u128::from(TICKS_PER_BEAT) * tick_den();
    usize::try_from(numerator.div_ceil(tick_num(bpm))).expect("帧位置必须装进 usize")
}

/// 咔哒声的窗口长度（帧）：`sample_rate × 4 ms`。
fn click_len() -> usize {
    usize::try_from(SAMPLE_RATE * 4 / 1_000).expect("192 帧")
}

/// 母线限制器的**输出延迟**（帧）：环长 [`yeban_engine::mixer::LOOKAHEAD_SAMPLES`]。
///
/// 咔哒声混在限制器之前 ⇒ 它在输出里整体晚这么多帧（与音乐同一条延迟，
/// 对齐关系不变）。判据把它写进预测，而不是放宽窗口。
const LIMITER_DELAY: usize = yeban_engine::mixer::LOOKAHEAD_SAMPLES;

/// 逐帧差异（左或右任一不同即为差异帧）。
fn diff_frames(enabled: &Render, disabled: &Render) -> Vec<usize> {
    (0..enabled.left.len())
        .filter(|frame| {
            enabled.left[*frame].to_bits() != disabled.left[*frame].to_bits()
                || enabled.right[*frame].to_bits() != disabled.right[*frame].to_bits()
        })
        .collect()
}

/// 把交错左右声道按小端 `f32` 落盘（只有控制线程做，音频线程拿不到这个函数）。
fn write_raw(path: &Path, rendered: &Render) {
    let mut bytes = Vec::with_capacity(rendered.frames() * 8);
    for (left, right) in rendered.left.iter().zip(rendered.right.iter()) {
        bytes.extend_from_slice(&left.to_bits().to_le_bytes());
        bytes.extend_from_slice(&right.to_bits().to_le_bytes());
    }
    std::fs::write(path, bytes).expect("原始样本必须能落盘");
}

/// `frames` 帧之后走带会推进到多少 tick：`floor(frames × tick_num / tick_den)`。
///
/// 与 [`beat_frame`] 同一份有理数（既是"拍点在哪一帧"的逆，也是走带的推进公式）。
fn ticks_after(frames: u64, bpm: f64) -> u64 {
    u64::try_from(u128::from(frames) * tick_num(bpm) / tick_den()).expect("tick 必须装进 u64")
}

/// M0：**关闭节拍器时输出与"未接线"逐字节相同**。
///
/// 夹具是一条**有音符**的轨（真正的混音链：合成 → 声相 → 主总线推子 → 限制器），
/// 因此这条判据的分量远大于"空工程静音"。落盘的原始样本用来跨提交比对：
/// 本判据自身只断言"两次独立装配逐位相同"与"夹具真的出声"
/// （把跨提交的哈希断言交给 `shasum`，不在源码里硬编码一个 libm 相关的常数）。
#[test]
fn disabled_metronome_leaves_the_mix_path_untouched() {
    let notes = [
        NoteSpec::at(0, 480, 60, 127),
        NoteSpec::at(960, 960, 67, 64),
        NoteSpec::at(2_400, 480, 72, 32),
    ];
    let fixture = note_project(&notes);
    let mut project = fixture.project;
    assert!(
        !project.transport.metronome_enabled,
        "夹具的前提：默认工程**关**节拍器（模型默认值）"
    );

    let first = render(&project, 200);
    let second = render(&project, 200);
    // 第三份：显式把开关写成 `false`（与默认值等价）⇒ 输出必须与默认那份逐位相同。
    project.transport.metronome_enabled = false;
    let explicit_off = render(&project, 200);

    let path = std::env::temp_dir().join("yeban-metronome-disabled.raw");
    write_raw(&path, &first);
    println!(
        "[metronome] M0 disabled fingerprint={:#018x} nonzero={} frames={} raw={}",
        first.fingerprint(),
        first.nonzero(),
        first.frames(),
        path.display()
    );

    assert!(first.nonzero() > 0, "夹具必须真的出声（否则这条判据会永真)");
    assert_eq!(first.left_bits(), second.left_bits(), "左声道必须逐位相同");
    assert_eq!(first.right, second.right, "右声道必须逐位相同");
    assert_eq!(
        first.left_bits(),
        explicit_off.left_bits(),
        "默认(缺省)与显式 false 必须给出同一份输出"
    );
    assert_eq!(first.stats.limiter_gain_reductions, 0);
}

/// M1：咔哒声只出现在**拍边界**的窗口里 —— 三个速度、逐个拍点、逐帧断言。
///
/// 每个拍点断言两件事：
/// 1. 该拍窗口**内**有真实差异（咔哒声真的响了）；
/// 2. 窗口**外**一个差异帧都没有（没有别的地方被改动）。
///
/// ## 窗口为什么要加母线限制器的延迟
///
/// 咔哒声混在母线限制器**之前**（[`yeban_engine::metronome`] 模块文档的"混音位置"），
/// 而限制器是一个**纯延迟**（[`yeban_engine::mixer::LOOKAHEAD_SAMPLES`] = 33 帧的环形缓冲，
/// 小于阈值时增益恒为 `1.0`）。因此咔哒声在**输出**里的起点是
/// `拍点 + 33` 帧 —— 与音乐经同一条母线的延迟**相同**（对齐关系不受影响）。
/// 本判据把这条延迟写进预测里，而不是把窗口放宽（放宽会同时放过"拍点算错"）。
#[test]
fn clicks_land_only_on_the_beat_grid_for_multiple_tempos() {
    // 三个速度：整除（120 / 128）与不整除（133.7 ⇒ 需要向上取整）。
    for bpm in [120.0f64, 128.0, 133.7] {
        let quanta = 700; // 89600 帧 ⇒ 120 BPM 下有 4 拍、133.7 BPM 下有 5 拍
        let disabled = render(&metronome_project(bpm, false), quanta);
        let enabled = render(&metronome_project(bpm, true), quanta);
        assert_eq!(disabled.nonzero(), 0, "{bpm} BPM: 关掉时必须逐位静音");

        let beats: Vec<usize> = (0..8)
            .map(|beat| beat_frame(bpm, beat) + LIMITER_DELAY)
            .filter(|frame| frame + click_len() <= enabled.frames())
            .collect();
        assert!(beats.len() >= 3, "{bpm} BPM: 夹具必须覆盖至少 3 拍");

        let diffs = diff_frames(&enabled, &disabled);
        assert!(!diffs.is_empty(), "{bpm} BPM: 咔哒声一次都没响");
        let (first_diff, last_diff) = (diffs[0], diffs[diffs.len() - 1]);

        // ① 每个拍窗口里都有差异（真的响了）。
        let mut window_diffs = Vec::new();
        for frame in &beats {
            let count = diffs
                .iter()
                .filter(|diff| **diff >= *frame && **diff < *frame + click_len())
                .count();
            assert!(
                count > 0,
                "{bpm} BPM: 拍点 {frame}（= 栅格 − {LIMITER_DELAY}）的窗口 [{frame}, {}) \
                 里没有任何差异 ⇒ 这个拍没响",
                frame + click_len()
            );
            window_diffs.push(count);
        }

        // ② 窗口外**一个**差异帧都没有（逐帧断言，不是"大约"）。
        for diff in &diffs {
            assert!(
                beats
                    .iter()
                    .any(|frame| *diff >= *frame && *diff < *frame + click_len()),
                "{bpm} BPM: 差异帧 {diff} 不在任何拍窗口里（咔哒声撒到了别处）"
            );
        }

        // ③ 子帧对齐：每个拍窗口的峰值样本索引必须与首拍相同（同一份波形，
        //    只是整体平移了一个拍长）。
        let peak_index = click_peak_index(&enabled, beats[0]);
        for frame in &beats {
            assert_eq!(
                click_peak_index(&enabled, *frame),
                peak_index,
                "{bpm} BPM: 拍 {frame} 的波形峰值索引必须与首拍相同（同一份波形）"
            );
        }

        println!(
            "[metronome] M1 bpm={bpm} beats(输出帧)={beats:?} 每拍窗口差异数={window_diffs:?} \
             总差异帧={} 首/末差异帧={first_diff}/{last_diff} 峰值索引={peak_index} \
             峰值={:.9} 限制器压限={}",
            diffs.len(),
            enabled.peak(),
            enabled.stats.limiter_gain_reductions,
        );
    }
}

/// 某一拍窗口内的峰值样本索引（相对窗口起点）。
fn click_peak_index(rendered: &Render, frame: usize) -> usize {
    let window = &rendered.left[frame..frame + click_len()];
    let mut best = 0usize;
    for (index, sample) in window.iter().enumerate() {
        if sample.abs() > window[best].abs() {
            best = index;
        }
    }
    best
}

/// M2：**强弱拍**——小节第一拍 `0.5`、其后各拍 `0.25`，且弱拍是强拍的**逐位** `× 0.5`。
///
/// 128 BPM / 48 kHz / 4-4 ⇒ 一拍 **22500 帧**（`60 × 48000 / 128`）。
/// 第 1 拍（强）与第 2 拍（弱）各取一个 192 帧的窗口（都加母线限制器的 33 帧延迟），
/// 逐样本断言 `weak[i] == strong[i] * 0.5`（`f32` 位模式相等）。两倍是 2 的幂
/// ⇒ 乘法**精确**，所以这不是容差判据。
#[test]
fn bar_downbeat_is_twice_the_weak_beats_sample_for_sample() {
    let quanta = 400; // 51200 帧 > 2 拍
    let enabled = render(&metronome_project(128.0, true), quanta);
    let strong_at = beat_frame(128.0, 0) + LIMITER_DELAY;
    let weak_at = beat_frame(128.0, 1) + LIMITER_DELAY;
    assert_eq!(beat_frame(128.0, 0), 0);
    assert_eq!(beat_frame(128.0, 1), 22_500, "128 BPM: 60 × 48000 / 128");
    assert!(weak_at + click_len() <= enabled.frames());

    let strong = &enabled.left[strong_at..strong_at + click_len()];
    let weak = &enabled.left[weak_at..weak_at + click_len()];
    let strong_peak = strong.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    let weak_peak = weak.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));

    println!(
        "[metronome] M2 strong@{strong_at} peak={strong_peak:.9} ({:#010x}) | \
         weak@{weak_at} peak={weak_peak:.9} ({:#010x}) | 比值={:.9}",
        strong_peak.to_bits(),
        weak_peak.to_bits(),
        f64::from(weak_peak) / f64::from(strong_peak),
    );

    assert_eq!(strong_peak, 0.5, "小节第一拍峰值 = STRONG_GAIN");
    assert_eq!(weak_peak, 0.25, "其余拍峰值 = WEAK_GAIN");
    let mut worst = 0.0f64;
    for (weak_sample, strong_sample) in weak.iter().zip(strong.iter()) {
        assert_eq!(
            weak_sample.to_bits(),
            (*strong_sample * 0.5).to_bits(),
            "弱拍必须逐位等于强拍 × 0.5：{strong_sample} × 0.5 vs {weak_sample}"
        );
        worst = worst.max((f64::from(*weak_sample) - f64::from(*strong_sample * 0.5)).abs());
    }
    println!("[metronome] M2 worst |weak − strong×0.5| = {worst:e}（要求逐位 0）");
    assert_eq!(worst, 0.0, "逐样本偏差必须为 0（浮点比较，不是容差）");
}

/// M3：咔哒声**不驱动**母线限制器（峰值 0.5 低于阈值 0.9 ⇒ 一次压限都没有）。
///
/// 这条钉住"混在限制器**之前**但增益不过阈值"这个组合：若把咔哒声配得过大，
/// 限制器会开始压 ⇒ `limiter_gain_reductions > 0` ⇒ 这条变红（这正是要发现的事）。
#[test]
fn metronome_alone_never_engages_the_bus_limiter() {
    let enabled = render(&metronome_project(128.0, true), 400);
    println!(
        "[metronome] M3 peak={:.9} 阈值={:.3} 天花板={:.3} 压限样本={} 最大压限={:.9}",
        enabled.peak(),
        yeban_engine::mixer::LIMITER_THRESHOLD,
        yeban_engine::mixer::LIMITER_CEILING,
        enabled.stats.limiter_gain_reductions,
        enabled.stats.limiter_max_reduction,
    );
    assert!(enabled.peak() > 0.0, "夹具必须真的出声");
    assert!(
        enabled.peak() < yeban_engine::mixer::LIMITER_THRESHOLD,
        "咔哒声峰值 {} 必须留在阈值以下",
        enabled.peak()
    );
    assert_eq!(
        enabled.stats.limiter_gain_reductions, 0,
        "咔哒声不得驱动限制器（否则读数是'被压过的咔哒声'）"
    );
    assert_eq!(enabled.stats.limiter_max_reduction, 0.0);
}

/// M4：停住 ⇒ 咔哒声**立即**静音（不留 4 ms 尾巴），恢复播放后从拍栅格重新起。
///
/// 用 `support::TransportRig`（保留走带事件生产端的装配）在量子之间发 `Stop`/`Play`。
#[test]
fn stop_silences_the_click_and_play_resumes_on_the_grid() {
    use support::TransportRig;
    use yeban_engine::ring::TransportCommand;

    let project = metronome_project(128.0, true);
    let mut rig = TransportRig::new(&project, 1);
    rig.quantum(); // 第 1 个量子：tick 0 的强拍（咔哒声从这里起）
    assert!(
        rig.nonzero() > 0,
        "第 1 个量子必须真的出声（tick 0 的首拍）"
    );
    // 咔哒声是 192 帧（4 ms）⇒ 第 2 个量子仍在尾巴里。
    rig.quantum();
    assert!(rig.nonzero() > 0, "第 2 个量子必须还在咔哒声的尾巴里");

    rig.send(&[TransportCommand::Stop]);
    rig.quantum();
    assert_eq!(
        rig.nonzero(),
        0,
        "停住的那一个量子必须**逐位**静音（丢弃尾巴）"
    );
    rig.quantum();
    assert_eq!(rig.nonzero(), 0, "停住期间不许再有咔哒声");

    rig.send(&[TransportCommand::Play]);
    rig.quantum();
    // 走带位置：2 个播放量子（256 帧）+ 1 个停住量子（不动）+ 本次 Play 量子（128 帧）
    // = 384 帧、`floor(384 × tick_num / tick_den)` tick（128 BPM 下是 16）。
    println!(
        "[metronome] M4 停住静音后恢复：state={:?} frames={} ticks={} 本量子非零={}",
        rig.runtime.stats().transport_state,
        rig.runtime.stats().position_frames,
        rig.runtime.position_ticks(),
        rig.nonzero(),
    );
    assert_eq!(
        rig.runtime.stats().position_frames,
        384,
        "停住不推进、恢复播放从停住处继续"
    );
    assert_eq!(
        rig.runtime.position_ticks(),
        ticks_after(384, 128.0),
        "位置必须与推进公式一致（不是'大约'）"
    );
}

/// M5：同输入两次独立装配 + 渲染**逐位相同**（含咔哒声的状态机）。
#[test]
fn metronome_render_is_byte_deterministic_across_independent_assemblies() {
    let project = metronome_project(133.7, true);
    let first = render(&project, 500);
    let second = render(&project, 500);
    println!(
        "[metronome] M5 fingerprint {:#018x} vs {:#018x} 非零={}",
        first.fingerprint(),
        second.fingerprint(),
        first.nonzero(),
    );
    assert!(first.nonzero() > 0, "夹具必须真的出声");
    assert_eq!(first.left_bits(), second.left_bits());
    assert_eq!(first.right, second.right);

    // 落盘"开启"侧样本：与 M0 的"关闭"侧样本一起构成提交前后的比对材料。
    let path = std::env::temp_dir().join("yeban-metronome-enabled.raw");
    write_raw(&path, &first);
    println!("[metronome] M5 enabled raw={}", path.display());
}

/// M6：**播放中换拍号**（4/4 的 960 tick ⟶ 3/2 的 1920 tick）必须让拍格**重对齐**。
///
/// 为什么这条判据存在：`next_beat_tick` 在换快照时**有意不重置**（拍格以 tick 为单位、
/// 与 BPM 无关 ⇒ 改速度不该让拍点跳一格）。但**拍号分母**换档会换掉"一拍几个 tick"，
/// 于是旧的下拍 tick 落在新格之外，而且每拍 `+= ticks_per_beat` 会**一直**偏半格
/// （960、2880、4800… 永远不是 1920 的整数倍）。实时侧因此在"格变了"时显式重对齐一次。
///
/// 夹具把这个差别做成可判定的两半：
///
/// - 第 2 个量子（128 帧 = 10 tick）时发布 3/2 的快照 ⇒ 重对齐把下拍拉到 **1920 tick
///   = 45000 帧**（`ceil(10 / 1920) × 1920`）；
/// - 不重对齐的旧行为会在 **960 tick = 22500 帧** 打一下（旧格的下拍）。
///
/// 因此：差异必须出现在 `45000 + 33`（母线限制器延迟）的窗口里，而且**一个都不许**
/// 出现在 `22500 + 33` 的窗口里。
#[test]
fn time_signature_change_realigns_the_beat_grid() {
    use support::render_with;
    use yeban_model::TimeSignature;

    let four_four = metronome_project(128.0, true);
    let mut three_two = four_four.clone();
    three_two.time_signature = TimeSignature {
        numerator: 3,
        denominator: 2,
    };
    assert_eq!(
        three_two.time_signature.denominator, 2,
        "3/2 ⇒ 一拍 1920 tick"
    );

    let quanta = 400; // 51200 帧 > 45000 + 192
    let disabled = render(&metronome_project(128.0, false), quanta);
    let switched = render_with(&four_four, quanta, 1, |quantum, rig| {
        if quantum == 2 {
            // 第 2 个量子前发布 3/2 的快照（revision 2 ⇒ 音频线程会切换）。
            rig.publish_equivalent(&three_two, 2);
        }
    });
    assert_eq!(disabled.nonzero(), 0, "参照渲染必须静音");

    let diffs = diff_frames(&switched, &disabled);
    assert!(
        !diffs.is_empty(),
        "换拍号之后必须仍然打拍（静音 = 节拍器死了）"
    );
    let first_beat = LIMITER_DELAY + 1; // tick 0 的强拍（咔哒声首样本是包络零点）
    let realigned = 45_000 + LIMITER_DELAY;
    let stale = 22_500 + LIMITER_DELAY;
    println!(
        "[metronome] M6 差异帧={} 首/末={}/{}；重对齐窗口=[{realigned}, {})；\
         旧格窗口=[{stale}, {})（必须为空）；首拍窗口起点={first_beat}",
        diffs.len(),
        diffs[0],
        diffs[diffs.len() - 1],
        realigned + click_len(),
        stale + click_len(),
    );

    // 前半：重对齐后的拍点真的出现在 45000 帧（= 1920 tick）上。
    assert!(
        diffs
            .iter()
            .any(|diff| *diff >= realigned && *diff < realigned + click_len()),
        "重对齐后的拍点必须在 45000 帧（1920 tick）上打出来"
    );
    // 后半：旧格的 22500 帧（960 tick）上**不许**有咔哒声。
    let stale_hits = diffs
        .iter()
        .filter(|diff| **diff >= stale && **diff < stale + click_len())
        .count();
    assert_eq!(
        stale_hits, 0,
        "旧拍格（960 tick）上的咔哒声必须消失 —— 否则就是'偏半格到永远'"
    );
    // 差异只允许落在两个拍窗口里（tick 0 的强拍 + 重对齐后的拍点）。
    for diff in &diffs {
        let in_first = *diff >= first_beat && *diff < LIMITER_DELAY + click_len();
        let in_realigned = *diff >= realigned && *diff < realigned + click_len();
        assert!(
            in_first || in_realigned,
            "差异帧 {diff} 不在任何一个合法拍窗口里"
        );
    }
}
