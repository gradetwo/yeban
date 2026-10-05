//! `[ARCH-DSP-004 / HD-44=A]` `signalsmith-stretch` 的**集成判据**。
//!
//! 为什么需要它: 依赖已按 `HD-44 = A` 引入（负责人裁决 2026-10-05），但**没有任何判据**说它真的能用。
//! "已引入的依赖"不是证据；本文件把它变成四条可复跑的断言。
//!
//! **用法取自该 crate 自带的官方示例** `examples/stretch.rs`（流式 `process(&input_block, output)`），
//! 不是我猜的。第一版我用 `seek(...)` + 一次大 `process` + `flush()`，两条判据因此变红 ——
//! 那是**我的调用方式错**，不是库的错。这条留在这里，免得后来者重犯。
//!
//! 四条判据（每条都能失败）:
//! 1. `preset_default_constructs_and_reports_latency` —— 构造成功且延迟可查询；
//! 2. `silence_in_stays_silent` —— 静音输入不得被"发明"出信号；
//! 3. `same_settings_twice_is_bit_identical` —— **确定性**：同输入同设置两次，输出逐位相同；
//! 4. `transpose_control_changes_the_output` —— 变调控制**真的有效**：`+12` 半音的输出必须与 `0` 半音不同
//!    （若两者相同，说明这个 setter 是空操作 ⇒ 判据红）。
//!
//! **故意不写**的: 精确输出长度等式与音高估计。前者取决于库内部块长与延迟（我没读实现），
//! 后者受瞬态/共振峰处理影响（我未实测）。宁可不写，也不写一条"看起来有"的判据。
use signalsmith_stretch::Stretch;

const SAMPLE_RATE: u32 = 48_000;
const BLOCK: usize = 4096;
const BLOCKS: usize = 8;
const FRAMES: usize = BLOCK * BLOCKS;

/// 确定性输入：静音，或 440 Hz 正弦（幅度 0.5）。
fn signal(sine: bool) -> Vec<f32> {
    if !sine {
        return vec![0.0_f32; FRAMES];
    }
    (0..FRAMES)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
        })
        .collect()
}

/// 按官方示例的流式用法跑一遍，返回拼接后的输出。
fn stream(input: &[f32], semitones: f32) -> Vec<f32> {
    let mut stretch = Stretch::preset_default(1, SAMPLE_RATE);
    stretch.set_transpose_factor_semitones(semitones, None);
    let mut out = Vec::with_capacity(input.len());
    let mut buf = [0.0_f32; BLOCK];
    for chunk in input.chunks_exact(BLOCK) {
        stretch.process(chunk, &mut buf);
        out.extend_from_slice(&buf);
    }
    out
}

#[test]
fn preset_default_constructs_and_reports_latency() {
    let stretch = Stretch::preset_default(1, SAMPLE_RATE);
    // 只要求"可查询"——具体帧数取决于库内部块长，不写成固定值。
    let _ = stretch.input_latency();
    let _ = stretch.output_latency();
}

#[test]
fn silence_in_stays_silent() {
    let peak = stream(&signal(false), 0.0)
        .iter()
        .fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!(
        peak == 0.0,
        "静音输入不得产出信号：峰值 {peak}（拉伸器发明了内容）"
    );
}

#[test]
fn same_settings_twice_is_bit_identical() {
    let input = signal(true);
    let a = stream(&input, 0.0);
    let b = stream(&input, 0.0);
    assert_eq!(a.len(), b.len(), "同设置两次的输出长度必须相同");
    assert!(
        a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
        "同输入同设置两次必须逐位相同（本仓的确定性红线）"
    );
}

#[test]
fn transpose_control_changes_the_output() {
    let input = signal(true);
    let plain = stream(&input, 0.0);
    let up = stream(&input, 12.0);
    assert_eq!(plain.len(), up.len(), "长度由调用方缓冲决定，两次应相同");
    assert!(
        plain.iter().zip(&up).any(|(x, y)| (x - y).abs() > 1e-6),
        "`+12` 半音的输出必须与 `0` 半音不同；若完全相同，说明该 setter 是空操作"
    );
}
