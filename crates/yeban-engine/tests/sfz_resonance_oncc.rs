//! **T-FIX-280 判据**：`resonance_onccN` / `resonance_curveccN` 的接线。
//!
//! # 缺口（开工前实测）
//!
//! `resonance` 族**根本没进调制表**：`ModTarget` 里没有 `Resonance` 这一格、
//! `parse_modulation_name` 里没有 `resonance` 那一支 ⇒ `resonance_oncc1=-20` 落进
//! **未知 opcode**（响亮告警、不生效）。静态 `resonance=20` 一直是生效的
//! （`Region::resonance` → `configured_voice_filter`），所以缺口只在**"逐块可变的那一半"**。
//!
//! # 量什么（对象 ＋ 单位，先说再跑）
//!
//! 对象 ＝ 一条 `cutoff=1000` 的 2 极低通上的**谐振**（单位 **dB**）；
//! 手段 ＝ 同一段确定性噪声（LCG，⛔ 不是随机数）喂进去，比**输出位模式**
//! 与**块尾峰值**（线性幅度）。
//!
//! 三条硬主张（每一条都配"它自己该怎样"的断言，⛔ 不是只断言"两者不同"）：
//!
//! 1. **落到同一格**：`resonance=3 ＋ resonance_oncc1=24` 在 `CC1=127` 时与
//!    `resonance=27`（静态）**逐位相同** —— 增量真的进了 `retune` 的谐振位，
//!    且加法的量级一分不差（若把增量错加到截止、或错乘 127，这条立刻红）。
//! 2. **不接线也逐位不变**：`CC1=0` 时，"带 `resonance_oncc1` 的区域"与
//!    "不带这条 opcode 的区域"**逐位相同**（`delta == 0.0` ⇒ `+ 0.0`）。
//! 3. **回落要复位**：CC 从 127 掉回 0 之后，滤波器的振铃必须回到基准谐振的
//!    衰减速度上 —— ⛔ 不许卡在最后一次被调制到的 27 dB（`retune_cutoff` 保留
//!    滤波器自带谐振，不复位就会卡住）。
//!
//! ⚠ 判决分支本身（`resonance_target` / `resonance_cc_live` 的五支）在
//! `yeban-engine` 的 lib 单元判据 `resonance_oncc_tests` 里逐支压（`--lib` 才跑得到）。

use std::cell::Cell;
use std::sync::Arc;

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::modulation::{ModTarget, ModUnit};

const HZ: f32 = 48_000.0;

/// 确定性噪声（LCG；⛔ `rand`、⛔ 时间种子）—— 两侧夹具必须吃到**同一段** PCM。
fn noise(frames: usize, seed: u32) -> Vec<f32> {
    let mut state = seed;
    (0..frames)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            // 取 bit 8..24（16 位）归一到 `[-0.5, 0.5)`：逐位确定、与平台无关。
            f32::from((state >> 8) as u16) / 65_536.0 - 0.5
        })
        .collect()
}

fn sample(pcm: Vec<f32>) -> InMemorySample {
    InMemorySample {
        pcm: Arc::new(pcm),
        channels: 1,
        sample_hz: HZ,
        loop_points: None,
    }
}

fn prepare(text: &str, source: &InMemorySample) -> PreparedSfz {
    let instrument = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 解析");
    PreparedSfz::prepare(&instrument, source, HZ).expect("摊平")
}

fn render_offline(
    text: &str,
    source: &InMemorySample,
    cc: Option<&dyn Fn(u8) -> u8>,
    frames: usize,
) -> Vec<f32> {
    let instrument = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 解析");
    let mut out = vec![0.0f32; frames];
    let written = instrument
        .render_note_into_with_cc(source, 60, 127, cc, 0.0, &mut out, HZ)
        .expect("离线渲染");
    assert_eq!(written, frames, "整段必须写满（⛔ 挡住静默不发声也算一致）");
    out
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |worst, &v| worst.max(v.abs()))
}

/// 滤波器在场 ＋ 宽带激励 ⇒ 输出必须真的出声（⛔ 挡住"全零也算逐位相同"）。
fn assert_audible(samples: &[f32], what: &str) {
    let peak = peak(samples);
    assert!(
        peak > 1e-3,
        "{what}：输出峰值 {peak:e} 太小 ⇒ 这条判据可能是空判（滤波器没被激励）"
    );
}

/// ⭐ 解析层：`resonance` 族**真的进调制表**，单位 dB、域 `-40..=40`。
///
/// 同时压三条"不归本族"的边界（各自独立断言，⛔ 不是合并成一句）：
/// 裸 `resonance=`（静态基准）、`resonance_curveccN`（只有曲线 ⇒ 不产生项）、
/// `resonance_smoothccN`（⛔ 本单不做，仍是未知 opcode ⇒ 不进表）。
#[test]
fn the_resonance_family_lands_in_the_modulation_table() {
    let text = "<region> sample=a.wav key=60 pitch_keycenter=60 \
                cutoff=1000 resonance=3 resonance_oncc1=-20\n";
    let instrument = SfzInstrument::load(text.to_owned(), &Default::default()).expect("解析");
    let rows: Vec<(ModTarget, u8, f32)> = instrument
        .with_instrument(|parsed| {
            parsed.regions()[0]
                .cc_modulations()
                .iter()
                .map(|item| (item.target, item.cc, item.delta))
                .collect()
        })
        .expect("重解析");
    assert_eq!(
        rows,
        vec![(ModTarget::Resonance, 1, -20.0)],
        "`resonance_oncc1=-20` 必须归约成 (Resonance, cc 1, -20 dB) 一项"
    );
    assert_eq!(ModTarget::Resonance.unit(), "dB", "规范页表 Unit ＝ dB");
    assert_eq!(ModTarget::Resonance.mod_unit(), ModUnit::Decibels);
    assert_eq!(
        ModTarget::Resonance.delta_range(),
        (-40.0, 40.0),
        "`resonance` 页 Modulation 表 Range ＝ `-40 to 40`（增量域，负值合法）"
    );

    let count = |text: &str| -> usize {
        let instrument = SfzInstrument::load(text.to_owned(), &Default::default()).expect("解析");
        instrument
            .with_instrument(|parsed| parsed.regions()[0].cc_modulations().len())
            .expect("重解析")
    };
    assert_eq!(
        count("<region> sample=a.wav key=60 pitch_keycenter=60 resonance=20\n"),
        0,
        "⭐ 裸 `resonance=` 是**静态基准**，⛔ 不是调制项"
    );
    assert_eq!(
        count("<region> sample=a.wav key=60 pitch_keycenter=60 cutoff=1000 resonance_curvecc1=9\n"),
        0,
        "只有曲线、没有 delta 的 `resonance_curveccN` ⛔ 不产生项（与其余四族同口径）"
    );
    assert_eq!(
        count(
            "<region> sample=a.wav key=60 pitch_keycenter=60 cutoff=1000 resonance_smoothcc1=-20\n"
        ),
        0,
        "`resonance_smoothccN` 本单不做 ⇒ 仍是未知 opcode（⛔ 不静默进表）"
    );
    assert_eq!(
        count("<region> sample=a.wav key=60 pitch_keycenter=60 cutoff=1000 resonance_cc1=-20\n"),
        1,
        "`_ccN` 后缀与 `_onccN` 同族（与 `cutoff_ccN` 同款）"
    );
    assert_eq!(
        count("<region> sample=a.wav key=60 pitch_keycenter=60 cutoff=1000 resonance2_oncc1=-20\n"),
        0,
        "⚠ `resonance2_*`（第二滤波器）不归本族 —— 兄弟单 T-FIX-279 的地盘"
    );
}

/// ⭐⭐ 核心：`resonance_oncc1=+24` 在 `CC1=127` 时**恰等于**静态 `resonance=27`。
///
/// 逐位相同而不是"差不多"：两条路的滤波器都经 `VoiceFilter::configured(kind, 1000,
/// 27.0, gain, hz)` 生成系数（一条在构造期、一条在块首 `retune`），入参逐位相同
/// ⇒ 系数逐位相同 ⇒ 整段输出逐位相同。若增量错加到**截止**上（＋24 Hz）或
/// 错乘成 `24 × 127`，这里立刻红。
#[test]
fn a_cc_delta_lands_exactly_where_the_static_opcode_would() {
    const FRAMES: usize = 8_192;
    let source = sample(noise(FRAMES + 1_024, 0x5eed));
    let with_cc = "<region> sample=a.wav key=60 pitch_keycenter=60 \
                   cutoff=1000 resonance=3 resonance_oncc1=24\n";
    let static_resonance = "<region> sample=a.wav key=60 pitch_keycenter=60 \
                            cutoff=1000 resonance=27\n";
    let hot = |_: u8| 127u8;
    let modulated = render_offline(with_cc, &source, Some(&hot), FRAMES);
    let reference = render_offline(static_resonance, &source, None, FRAMES);
    assert_audible(&modulated, "CC1=127 的被调制腿");
    assert_audible(&reference, "静态 `resonance=27` 的参照腿");
    assert_eq!(
        bits(&modulated),
        bits(&reference),
        "⭐ `resonance=3 ＋ resonance_oncc1=24`（CC1=127）必须与 `resonance=27` 逐位相同"
    );

    // ⭐ 反空判：同一夹具 `CC1=0` 必须与 `CC1=127` **逐位不同**
    // （否则上面那条"逐位相同"可能只证明了两边都恒等于基准）。
    let cold = render_offline(with_cc, &source, Some(&|_: u8| 0u8), FRAMES);
    assert_audible(&cold, "CC1=0 的基准腿");
    assert_ne!(
        bits(&cold),
        bits(&modulated),
        "⭐ CC1=0 与 CC1=127 必须给出不同输出（⛔ 挡住「接线了但没生效」）"
    );
}

/// ⭐ `CC1=0`（增量恰 `0.0`）时，带 `resonance_oncc1` 的区域必须与
/// **不带这条 opcode** 的区域**逐位相同** —— 这就是"接线不扰动既有工程"。
#[test]
fn without_the_cc_the_same_region_is_bit_for_bit_unchanged() {
    const FRAMES: usize = 4_096;
    let source = sample(noise(FRAMES + 1_024, 0x0bad_5eed));
    let with_cc = "<region> sample=a.wav key=60 pitch_keycenter=60 \
                   cutoff=1000 resonance=3 resonance_oncc1=24\n";
    let plain = "<region> sample=a.wav key=60 pitch_keycenter=60 cutoff=1000 resonance=3\n";
    let cold = |_: u8| 0u8;
    let wired_cold = render_offline(with_cc, &source, Some(&cold), FRAMES);
    let wired_none = render_offline(with_cc, &source, None, FRAMES);
    let bare = render_offline(plain, &source, None, FRAMES);
    assert_audible(&wired_cold, "接了线但 CC=0 的腿");
    assert_eq!(
        bits(&wired_cold),
        bits(&wired_none),
        "CC 探针「给了一个恒 0 的值」与「根本没给」必须逐位相同（delta 恰 0.0）"
    );
    assert_eq!(
        bits(&wired_cold),
        bits(&bare),
        "⭐ CC=0 时必须与不带 `resonance_oncc1` 的区域逐位相同"
    );
}

/// ⭐⭐ **回落复位**（实时池路径，跨块状态）：CC 从 127 掉回 0 之后，滤波器的振铃
/// 必须按**基准谐振**（3 dB）衰减，⛔ 不许卡在被调制到的 27 dB 上。
///
/// # 怎么量
///
/// 样本＝**第一块是噪声、之后全静音**（只留振铃）。同一份 PCM 跑三臂：
///
/// * `reset`：块 0 用 `CC1=127`，块 1 起用 `CC1=0`；
/// * `stuck`：全程 `CC1=127`（＝"卡在被调制值上"的那个坏行为）；
/// * `base`：全程 `CC1=0`（基准谐振的振铃，作参照）。
///
/// 量**块 1 的峰值**（块 0 之后的第一个静音块）。复位生效 ⇒ `reset` 的振铃按 3 dB
/// 衰减、与 `base` 同量级；复位失效 ⇒ 与 `stuck` 同量级。
///
/// ⚠ 判决分支见 lib 单元判据 `resonance_oncc_tests`（那里能逐支读到返回值）；
/// 这里量的是**落到 DSP 上的后果**。
#[test]
fn a_cc_that_falls_back_to_zero_does_not_leave_the_filter_resonant() {
    // ⚠ **256 样本一块、12 块**：`BLOCK` 取 16384 时，光是 27 dB 谐振的振铃在一个
    // 块里就衰减到 denormal（实测块 2 起三臂全是 1e-4x）⇒ 块级峰值分不出快慢。
    // 块长压到 256 之后，"换谐振前后各一块"的峰值比就是**衰减率**，与初始幅度无关
    // （尺度无关），这才是"复位有没有生效"要量的东西。
    const BLOCK: usize = 256;
    const BLOCKS: usize = 12;
    // CC 在**块 2** 才回落 ⇒ 块 0、1 三臂（reset / stuck）逐位相同，块 2 起分道扬镳。
    const DROP_AT: usize = 2;
    let mut pcm = noise(BLOCK, 0x2800_0123);
    pcm.resize(BLOCK * BLOCKS + 1_024, 0.0);
    let source = sample(pcm);
    let text = "<region> sample=a.wav key=60 pitch_keycenter=60 \
                cutoff=1000 resonance=3 resonance_oncc1=24\n";
    let plan: PreparedSfz = prepare(text, &source);

    let run = |first: u8, later: u8| -> Vec<Vec<f32>> {
        // ⚠ 用 `Rc<Cell<u8>>` 而不是裸 `Cell`：`probe` 是 `move` 闭包，会把 `cc`
        // **移进**闭包，之后就没法在块之间 `set` 了；共享同一个格子才既能读又能写。
        let cc = std::rc::Rc::new(Cell::new(first));
        let handle = std::rc::Rc::clone(&cc);
        let probe = move |_: u8| handle.get();
        let mut pool = SfzVoicePool::empty();
        let created = pool.trigger_note(&plan, 60, 127, u64::MAX, 1.0, Some(&probe));
        assert_eq!(created, 1, "区域必须真的建出声部");
        let mut blocks = Vec::with_capacity(BLOCKS);
        for index in 0..BLOCKS {
            if index == DROP_AT {
                cc.set(later);
            }
            let mut out = vec![0.0f32; BLOCK];
            pool.render_into_with_cc(&plan, (index * BLOCK) as u64, &mut out, Some(&probe));
            blocks.push(out);
        }
        blocks
    };

    let reset = run(127, 0);
    let stuck = run(127, 127);
    let base = run(0, 0);

    for index in 0..BLOCKS {
        println!(
            "[t280] 块 {index} 峰值 reset={:e} stuck={:e} base={:e}",
            peak(&reset[index]),
            peak(&stuck[index]),
            peak(&base[index])
        );
    }

    // 块 0 必须真的被激励起来（否则后面比的是 0 与 0）。
    assert_audible(&reset[0], "块 0");
    // ⭐ 回落之前（块 0、块 1）`reset` 与 `stuck` 必须逐位相同 —— 同一份 CC、同一份
    // PCM、同一套系数，任何差异都说明夹具本身在抖。
    for index in 0..DROP_AT {
        assert_eq!(
            bits(&reset[index]),
            bits(&stuck[index]),
            "块 {index}（CC 回落之前）两臂必须逐位相同"
        );
    }

    // ⭐ 量**回落之后一块的衰减比**（块 3 → 块 4 的峰值比）。
    //
    // 两个实测事实决定了这个口径：
    // 1. **CC 回落要下一块才落到滤波器上** —— 块 2（`DROP_AT`，探针已经读到 0 的那
    //    一块）两臂仍**逐位相同**（实测都是 9.264876e-2），块 3 才分道扬镳；
    // 2. 块 3 的**绝对**峰值不能直接跟 `base` 比 —— 它的初始幅度是 `base` 的 1e5 倍
    //    （块 2 的残余状态不同），所以绝对量永远"同不了量级"。
    //
    // 峰值比 `peak[k+1] / peak[k]` 是**尺度无关**的：它就是滤波器在那两块之间的衰减
    // 率，正好只取决于当时的谐振配置。实测 reset 与 base 都恰是 7.3e-6（逐位同一
    // 极点），stuck 是 4.6e-1。
    let decay = |blocks: &[Vec<f32>], k: usize| -> f64 {
        let current = f64::from(peak(&blocks[k]));
        assert!(
            current > 1e-30,
            "块 {k} 的峰值太小学生（{current:e}）⇒ 衰减比没有意义"
        );
        f64::from(peak(&blocks[k + 1])) / current
    };
    let k = DROP_AT + 1;
    let reset_decay = decay(&reset, k);
    let stuck_decay = decay(&stuck, k);
    let base_decay = decay(&base, k);
    println!(
        "[t280] 衰减比（块 {k}→{}）reset={reset_decay:e} stuck={stuck_decay:e} base={base_decay:e}",
        k + 1
    );

    // ① 复位生效 ⇒ 振铃按**基准谐振**的速度衰减（不是"随便小一点"）。
    assert!(
        reset_decay < 1e-3,
        "复位后必须按基准谐振快速衰减（实测 reset 衰减比 {reset_decay:e}）"
    );
    // ② 判别力：不复位时几乎不衰减（同一块区间）。没有这一条，①可能只是测到了噪声。
    assert!(
        stuck_decay > 1e-1,
        "⭐ 这条判据必须真的能咬合：不复位时衰减比应远大于 0，实测 {stuck_decay:e}"
    );
    assert!(
        stuck_decay > 1_000.0 * reset_decay,
        "⭐ 复位前后必须拉开三个数量级：stuck={stuck_decay:e} reset={reset_decay:e}"
    );
    // ③ 复位后必须**回到基准的行为**：衰减率与"一直 3 dB"的参照一致（差两倍以内）。
    // ⛔ 只是"变小了"还不够 —— 那样把谐振调到任意小值也能过。
    assert!(
        reset_decay <= 2.0 * base_decay && base_decay <= 2.0 * reset_decay,
        "复位后的衰减率应与基准谐振一致：reset={reset_decay:e} base={base_decay:e}"
    );
}
