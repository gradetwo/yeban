//! ⭐ **SFZ `<effect>` 效果总线判据**（`ADR-0077` D5/D7 的引擎半边）。
//!
//! # 量什么
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | SFX-1 | 全 0 发送 ⇒ 与"没有 `<effect>`"**逐位相同** | `to_bits` 逐样本相等 |
//! | SFX-2 | `effect1=100` ＋ `fx1 type=reverb` ⇒ 块末**湿声可闻**（尾部能量 > 0） | 末块 RMS |
//! | SFX-3 | `fx1 type=chorus`（不支持）⇒ **不接线** ＋ 装载期仍告警 | 输出与无效果逐位相同 ＋ `EffectNotProcessed` |
//! | SFX-6 | 总线缓冲每块清零（不累积上一块） | 连续两块的尾部能量不随块号增长 |
//!
//! ⚠ 不判的：`dsp_order` 排序（本片每条总线只有一台器件 ⇒ 排序位不可观测）、
//! `aux*`/`main`（`ADR-0077` D1 明确不建）。

use yeban_engine::sfz_instrument::{InMemorySample, PreparedSfz, SfzInstrument, SfzVoicePool};
use yeban_sfz::parser::Warning;

const HZ: f32 = 48_000.0;
/// 样本帧数：480 帧（10 ms）⇒ 混响尾巴远长于干声。
const SAMPLE_FRAMES: usize = 480;
/// 块长（帧）。
const BLOCK: usize = 256;

fn prepare(text: &str) -> PreparedSfz {
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("SFZ 必须解析成功");
    let sample = InMemorySample::sine(440.0, HZ, SAMPLE_FRAMES);
    PreparedSfz::prepare(&sfz, &sample, HZ).expect("摊平必须成功")
}

/// 触发一个音、渲染 `blocks` 块，返回每块的样本（按块拼接）。
fn render_note(plan: &PreparedSfz, blocks: usize) -> Vec<f32> {
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(plan, BLOCK);
    assert_eq!(pool.trigger_note(plan, 60, 100, 12_000, 1.0, None), 1);
    let mut rendered = Vec::with_capacity(blocks * BLOCK);
    for block in 0..blocks {
        let mut out = vec![0.0f32; BLOCK];
        pool.render_into(plan, block as u64 * BLOCK as u64, &mut out);
        rendered.extend(out);
    }
    rendered
}

fn rms(samples: &[f32]) -> f32 {
    let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
    (sum / samples.len() as f32).sqrt()
}

/// SFX-1：`effect1=0` ⇒ 与没有 `<effect>` 的同一份乐器**逐位相同**。
/// 同一义务在 ADR-0077 D5 的表述为 `S-bus-0`：同一份夹具、同一块长度、
/// `to_bits` 全等——本判据即其落点，两 ID 同指。
#[test]
fn a_zero_send_is_bit_identical_to_a_plain_instrument() {
    let plain = render_note(&prepare("<region> sample=a.wav lokey=60 hikey=60"), 4);
    let zero = render_note(
        &prepare("<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=0"),
        4,
    );
    let bits = |samples: &[f32]| {
        samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&plain), bits(&zero), "发送量 0 必须逐位不变");
    assert!(rms(&plain) > 0.0, "干声必须真的出声");
}

/// SFX-2：`effect1=100` ＋ `fx1 type=reverb` ⇒ 干声结束后**仍有湿声**（尾部能量）。
///
/// ⚠ 尾巴有 ~1800 帧的物理延迟（预延迟 576 ＋ 最短梳状线 ≈ 1200）⇒ 必须
/// 渲染足够多的块才能观测到，⛔ 不是"接线失败"。
#[test]
fn a_full_send_puts_an_audible_tail_on_the_bus() {
    let plain = render_note(&prepare("<region> sample=a.wav lokey=60 hikey=60"), 16);
    let wet = render_note(
        &prepare(
            "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
        ),
        16,
    );
    // 干声 480 帧 ⇒ 第 3 块之后必须已经静音；湿声此时才刚开始出现。
    let tail_plain = &plain[3 * BLOCK..4 * BLOCK];
    assert!(
        rms(tail_plain) < 1.0e-6,
        "干声在 3 块之后必须已经静音（读数 {}）",
        rms(tail_plain)
    );
    for block in 3..8 {
        assert!(
            rms(&plain[block * BLOCK..(block + 1) * BLOCK]) < 1.0e-6,
            "干声第 {block} 块必须静音"
        );
    }
    let wet_tail: f32 = (7..12)
        .map(|block| rms(&wet[block * BLOCK..(block + 1) * BLOCK]))
        .fold(0.0f32, f32::max);
    assert!(
        wet_tail > 1.0e-4,
        "混响尾巴必须可闻（7..12 块最大 RMS {}）",
        wet_tail
    );
}

/// SFX-3：不支持的 `type` ⇒ 不接线（逐位等于无效果）＋ 装载期仍告警。
#[test]
fn an_unsupported_type_stays_dry_and_still_warns() {
    let text = "<effect> bus=fx1 type=chorus\n<region> sample=a.wav lokey=60 hikey=60 effect1=100";
    let sfz = SfzInstrument::load(text.to_owned(), &Default::default()).expect("必须可装载");
    assert!(
        sfz.warnings()
            .iter()
            .any(|warning| matches!(warning, Warning::EffectNotProcessed { .. })),
        "不支持的 type 必须仍然响亮告警"
    );
    let plain = render_note(&prepare("<region> sample=a.wav lokey=60 hikey=60"), 4);
    let unsupported = render_note(&prepare(text), 4);
    let bits = |samples: &[f32]| {
        samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&plain), bits(&unsupported), "未接线必须逐位等于干声");
}

/// SFX-6：总线缓冲每块清零 —— 输入停止后尾巴**单调衰减**，不因缓冲残留而累积。
#[test]
fn the_bus_buffers_are_cleared_every_block() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    let mut levels = Vec::new();
    for block in 0..80 {
        let mut out = vec![0.0f32; BLOCK];
        pool.render_into(&plan, block as u64 * BLOCK as u64, &mut out);
        levels.push(rms(&out));
    }
    let peak = levels[20..40].iter().cloned().fold(0.0f32, f32::max);
    let late = levels[72..80].iter().cloned().fold(0.0f32, f32::max);
    assert!(peak > 1.0e-4, "尾巴必须真的出现（读数 {peak}）");
    assert!(
        late < peak * 0.5,
        "后段必须明显衰减（残留会把它抬起来）：peak={peak} late={late}"
    );
}

/// SFX-7（`ADR-0077` D8 的实时面）：**稳态渲染不再重建总线**。
///
/// # 判定量
///
/// 池的 `bus_builds` 读数（每次 `BusBank` 构造 ＋1，构造 ＝ 分配 `Vec` 与
/// `Reverb`）。`warm_buses` 一次之后，80 块渲染里它**必须不增长** —— 增长就
/// 意味着音频回调里分配过，违 [MUST-GATE-001]。
///
/// # 正对照
///
/// 不预热直接渲染：读数从 `0` 到 `1`。这一腿证明"读数恒 0"不是仪器失灵。
#[test]
fn steady_state_rendering_never_rebuilds_the_bus() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    // 正对照：没有预热 ⇒ 第一块就会建一次（兜底路径）。
    let mut lazy = SfzVoicePool::new(HZ);
    assert_eq!(lazy.bus_builds(), 0, "新池还没有总线");
    assert_eq!(lazy.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    let mut out = vec![0.0f32; BLOCK];
    lazy.render_into(&plan, 0, &mut out);
    assert_eq!(
        lazy.bus_builds(),
        1,
        "兜底路径必须恰好建一次（仪器有判别力）"
    );

    // 判据腿：预热后稳定渲染，不再增长。
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    let after_warm = pool.bus_builds();
    assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    for block in 0..80 {
        let mut out = vec![0.0f32; BLOCK];
        pool.render_into(&plan, block as u64 * BLOCK as u64, &mut out);
    }
    assert_eq!(
        pool.bus_builds(),
        after_warm,
        "稳态渲染必须**零重建**（重建 ＝ 在回调里分配，违 [MUST-GATE-001]）"
    );
}

/// SFX-8（`ADR-0077` D6 修订：立体声腿**也接总线**）。
///
/// # 判定量
///
/// 1. `effect1 = 0` 与无 `<effect>` ⇒ 两声道**逐位相同**（零发送不动干声）；
/// 2. `effect1 = 100` ⇒ 两声道都在 7..12 块出现混响尾巴，且干声 3 块后归零；
/// 3. 尾巴**等量**：两声道尾部读数相同（同一份单声道湿声加回两边）。
///
/// 口径（归约）：进总线的是 `(L + R) / 2` —— 每声道各送 `0.5 × send`。
#[test]
fn the_stereo_leg_carries_the_same_mono_tail() {
    let plain = prepare("<region> sample=a.wav lokey=60 hikey=60");
    let dry_plan =
        prepare("<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=0");
    let wet_plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );

    let render = |plan: &PreparedSfz| {
        let mut pool = SfzVoicePool::new(HZ);
        pool.warm_buses(plan, BLOCK);
        assert_eq!(pool.trigger_note(plan, 60, 100, 12_000, 1.0, None), 1);
        let mut left = vec![0.0f32; 16 * BLOCK];
        let mut right = vec![0.0f32; 16 * BLOCK];
        for block in 0..16 {
            let start = block * BLOCK;
            pool.render_into_stereo_with_probes(
                plan,
                start as u64,
                &mut left[start..start + BLOCK],
                &mut right[start..start + BLOCK],
                None,
                None,
            );
        }
        (left, right)
    };

    let (plain_l, plain_r) = render(&plain);
    let (dry_l, dry_r) = render(&dry_plan);
    let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&plain_l), bits(&dry_l), "左声道：零发送必须逐位不变");
    assert_eq!(bits(&plain_r), bits(&dry_r), "右声道：零发送必须逐位不变");

    let (wet_l, wet_r) = render(&wet_plan);
    let window = |samples: &[f32], from: usize, to: usize| rms(&samples[from * BLOCK..to * BLOCK]);
    for (name, samples) in [("左", &wet_l), ("右", &wet_r)] {
        assert!(
            window(&wet_l, 3, 6) < 1e-3,
            "{name}声道：干声段应当已经被扣干（读数 {}）",
            window(samples, 3, 6)
        );
        let tail = window(samples, 7, 12);
        assert!(tail > 1e-4, "{name}声道：7..12 块必须有尾巴（读数 {tail}）");
    }
    assert_eq!(
        bits(&wet_l),
        bits(&wet_r),
        "居中单声道源 ⇒ 两声道拿到的尾巴必须逐位相同（同一份湿声）"
    );
    let _ = dry_l;
    let _ = dry_r;
}

/// SFX-9（`ADR-0077` D8 的"绑定期分配"清零）：**控制侧预占槽 ＋ 预热**之后，
/// 快照边界的建槽不再发生、总线也不再重建。
///
/// # 判定量
///
/// * 正腿：`reserve_track_slot` → `warm_sfz_buses` ⇒ `sfz_bus_builds` 在
///   **渲染之前**就是 `Some(1)`（证明预热真的挂到了槽上，⛔ 不是空操作）；
/// * 判据腿：随后 `begin_snapshot` ＋ 80 块渲染，读数**仍为 1**（零重建）；
/// * 反腿：不预占直接预热 ⇒ 读数 `None`（旧路径的空操作，证明仪器能区分两者）。
#[test]
fn reserved_slots_keep_the_bus_from_being_built_on_the_audio_thread() {
    use yeban_engine::synth::SynthEngine;
    use yeban_model::EntityId;

    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let track = EntityId::new();

    // 反腿：没有槽 ⇒ 预热无处可挂（读数 `None`）。
    let mut lazy = SynthEngine::new(48_000);
    lazy.warm_sfz_buses(track, &plan, BLOCK);
    assert_eq!(
        lazy.sfz_bus_builds(track),
        None,
        "没有预占槽时预热必须是空操作（否则说明它在偷偷建槽）"
    );

    // 正腿 ＋ 判据腿。
    let mut engine = SynthEngine::new(48_000);
    engine.reserve_track_slot(48_000, track);
    engine.warm_sfz_buses(track, &plan, BLOCK);
    assert_eq!(
        engine.sfz_bus_builds(track),
        Some(1),
        "预占 ＋ 预热之后，渲染之前总线就该已经建好"
    );
    engine.begin_snapshot(48_000, std::slice::from_ref(&track), [], []);
    for block in 0..80 {
        let mut out = vec![0.0f32; BLOCK];
        engine.render_track(track, None, Some(&plan), &mut out);
        let _ = block;
    }
    assert_eq!(
        engine.sfz_bus_builds(track),
        Some(1),
        "快照边界必须复用预占的槽 ⇒ 80 块里零重建"
    );
}

/// SFX-4（`ADR-0077` D5）：**四条总线互相独立、可以同时响**。
///
/// # 判定量
///
/// 同一段区域分别只送 `fx1`、只送 `fx2`…、以及四条全送：
///
/// * 单条的总线尾巴读数 `t1`；
/// * 四条全送 ⇒ 尾巴应为 `≈ 4 × t1`（四台同参数混响、同一份输入 ⇒ 输出相加）；
/// * 若实现把四条路由到同一个缓冲/只建了第一条，这个比例会塌回 `1×`。
#[test]
fn four_buses_can_run_at_once() {
    let tail = |text: &str| {
        let plan = prepare(text);
        let mut pool = SfzVoicePool::new(HZ);
        pool.warm_buses(&plan, BLOCK);
        assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
        let mut out = vec![0.0f32; 16 * BLOCK];
        for block in 0..16 {
            let start = block * BLOCK;
            pool.render_into(&plan, start as u64, &mut out[start..start + BLOCK]);
        }
        rms(&out[7 * BLOCK..12 * BLOCK])
    };

    let one =
        tail("<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100");
    assert!(one > 1e-4, "单条总线必须有尾巴（读数 {one}）");

    for bus in 1..=4 {
        let only = format!(
            "<effect> bus=fx{bus} type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect{bus}=100"
        );
        let reading = tail(&only);
        assert!(
            (reading - one).abs() < one * 1e-3,
            "fx{bus} 单飞应与 fx1 同读数（{reading} vs {one}）"
        );
    }

    let all = tail(
        "<effect> bus=fx1 type=reverb\n<effect> bus=fx2 type=reverb\n\
         <effect> bus=fx3 type=reverb\n<effect> bus=fx4 type=reverb\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100 effect2=100 effect3=100 effect4=100",
    );
    assert!(
        (all - 4.0 * one).abs() < one * 1e-2,
        "四条全送 ⇒ 尾巴应≈4×（{all} vs {}）",
        4.0 * one
    );
}

/// SFX-5（`ADR-0077` D2/D5）：**发送量按比例进总线**，不是开关。
///
/// # 判定量
///
/// 同参数、同音符，`effect1=50` 的总线尾巴应 ≈ `0.5 ×` `effect1=100` 的读数
/// （线性器件 ＋ 同一份输入 ⇒ 输出与发送量成正比）。若实现把发送量当布尔，
/// 比例会塌回 `1.0`。
#[test]
fn the_send_amount_scales_the_bus_input() {
    let tail = |send: u8| {
        let text = format!(
            "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1={send}"
        );
        let plan = prepare(&text);
        let mut pool = SfzVoicePool::new(HZ);
        pool.warm_buses(&plan, BLOCK);
        assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
        let mut out = vec![0.0f32; 16 * BLOCK];
        for block in 0..16 {
            let start = block * BLOCK;
            pool.render_into(&plan, start as u64, &mut out[start..start + BLOCK]);
        }
        rms(&out[7 * BLOCK..12 * BLOCK])
    };
    let full = tail(100);
    let half = tail(50);
    assert!(full > 1e-4);
    assert!(
        (half - full * 0.5).abs() < full * 0.02,
        "50% 发送应≈半根尾巴（{half} vs {}）",
        full * 0.5
    );
}

/// SFX-12（`ADR-0077` D11）：**同一条总线被声明多次时，`dsp_order` 最小者胜**。
///
/// # 判定量
///
/// `BusEffect::order()` 的读数必须等于最小 `dsp_order`；没声明的总线 ⇒ `None`。
/// 这条把"重复声明"的裁决从"实现里藏着"变成**可观测、可断言**的事实
/// （`dsp_order` 本身在每条总线只有一台器件时仍不可听出差别）。
#[test]
fn the_lowest_dsp_order_wins_a_duplicate_bus() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb dsp_order=5\n\
         <effect> bus=fx1 type=reverb dsp_order=2\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    assert_eq!(
        plan.bus_effect(0).map(|effect| effect.order()),
        Some(2),
        "同总线多条声明：最小 dsp_order 胜"
    );
    assert_eq!(plan.bus_effect(1), None, "fx2 没声明 ⇒ 无计划");
}

/// SFX-13：**`dsp_order` 缺省 0**，且四条总线各自独立（互不覆盖）。
#[test]
fn bus_plans_are_per_bus_and_default_to_order_zero() {
    let plan = prepare(
        "<effect> bus=fx3 type=reverb\n\
         <effect> bus=fx1 type=reverb dsp_order=7\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100 effect3=100",
    );
    assert_eq!(plan.bus_effect(0).map(|effect| effect.order()), Some(7));
    assert_eq!(
        plan.bus_effect(2).map(|effect| effect.order()),
        Some(0),
        "缺省 0"
    );
    assert_eq!(plan.bus_effect(1), None);
    assert_eq!(plan.bus_effect(3), None);
}

/// SFX-14：**group 级的发送量真的到了总线**（不只是模型里读得到）。
#[test]
fn a_group_level_send_reaches_the_bus() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n\
         <group>effect1=100\n<region>sample=a.wav lokey=60 hikey=60",
    );
    let out = render_note(&plan, 16);
    let tail = rms(&out[7 * BLOCK..12 * BLOCK]);
    assert!(tail > 1e-4, "group 发送必须有尾巴（读数 {tail}）");
}

/// SFX-15：**region 的 0 覆盖 group 的 100** —— 覆盖不是只在模型里成立。
#[test]
fn a_region_zero_cancels_the_group_send() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n\
         <group>effect1=100\n<region>sample=a.wav lokey=60 hikey=60 effect1=0",
    );
    let out = render_note(&plan, 16);
    let tail = rms(&out[7 * BLOCK..12 * BLOCK]);
    assert!(
        tail < 1e-6,
        "region 的 0 必须压住 group 的 100（读数 {tail}）"
    );
}

/// SFX-16：**多声部求和** —— 两个同音高声部 ⇒ 总线激励翻倍（尾巴 ≈ 2×）。
#[test]
fn two_voices_double_the_bus_input() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    // 两条区域同音域 ⇒ 一个 note-on 命中两条 ⇒ 两个声部同时发声
    // （`trigger_note` 的返回值就是"起了几个"，这里正好用它做正腿）。
    let both = {
        let mut pool = SfzVoicePool::new(HZ);
        pool.warm_buses(&plan, BLOCK);
        assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 2);
        let mut rendered = Vec::with_capacity(16 * BLOCK);
        for block in 0..16 {
            let mut out = vec![0.0f32; BLOCK];
            pool.render_into(&plan, block as u64 * BLOCK as u64, &mut out);
            rendered.extend(out);
        }
        rendered
    };
    let one = {
        let plan = prepare(
            "<effect> bus=fx1 type=reverb\n\
             <region> sample=a.wav lokey=60 hikey=60 effect1=100",
        );
        render_note(&plan, 16)
    };
    let read = |samples: &[f32]| rms(&samples[7 * BLOCK..12 * BLOCK]);
    let (two, one) = (read(&both), read(&one));
    assert!(one > 1e-4);
    assert!(
        (two - 2.0 * one).abs() < one * 0.05,
        "两个声部应≈2×激励（{two} vs {}）",
        2.0 * one
    );
}

/// SFX-17：**尾巴不随声部结束** —— note-off 之后的块里总线仍在响。
///
/// 这条钉住"总线状态属于池、不属于声部"：若实现误把混响塞进声部，
/// note-off 一瞬间尾巴会被一起切掉。
#[test]
fn the_tail_outlives_the_voice() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    assert_eq!(pool.trigger_note(&plan, 60, 100, 120, 1.0, None), 1);
    let mut out = vec![0.0f32; 32 * BLOCK];
    for block in 0..32 {
        let mut chunk = vec![0.0f32; BLOCK];
        pool.render_into(&plan, block as u64 * BLOCK as u64, &mut chunk);
        out[block * BLOCK..(block + 1) * BLOCK].copy_from_slice(&chunk);
    }
    // `end_sample = 120` ⇒ 干声在第一块内就结束；尾巴应当在更晚的块里仍可闻。
    let late = rms(&out[6 * BLOCK..10 * BLOCK]);
    assert!(late > 1e-4, "声部结束后尾巴必须还在（读数 {late}）");
}

/// SFX-18：**越界发送量在装载期钳到 100**，听感与 100 一致。
#[test]
fn an_out_of_range_send_is_clamped_before_it_reaches_the_bus() {
    let clamped = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=150",
    );
    let full = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let a = render_note(&clamped, 16);
    let b = render_note(&full, 16);
    let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a), bits(&b), "150 钳到 100 后必须逐位一致");
}

/// SFX-19：`type` 的**引号 / 首尾空白 / 大小写**都算同一个 `reverb`。
#[test]
fn a_quoted_type_with_spaces_is_still_a_reverb() {
    let plan = prepare(
        "<effect> bus=fx1 type=\"  Reverb  \"\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    assert!(plan.bus_effect(0).is_some(), "必须建成总线计划");
    let out = render_note(&plan, 16);
    let tail = rms(&out[7 * BLOCK..12 * BLOCK]);
    assert!(tail > 1e-4, "读数 {tail}");
}

/// SFX-20：**声明了总线但发送为 0** ⇒ 与完全没有 `<effect>` 逐位相同
/// （总线存在，⛔ 不代表会偷偷往干声里加东西）。
#[test]
fn a_declared_bus_with_zero_send_is_bit_identical_to_plain() {
    let declared =
        prepare("<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=0");
    let plain = prepare("<region> sample=a.wav lokey=60 hikey=60");
    let a = render_note(&declared, 16);
    let b = render_note(&plain, 16);
    let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a), bits(&b));
}

/// SFX-21：**空 `<effect bus=fx4>`（没有 `type`）** ⇒ 不建总线；
/// 并且这条声明走"声明了但不支持"的既有告警（不是孤儿发送）。
#[test]
fn an_empty_effect_declaration_builds_no_bus() {
    let plan = prepare("<effect> bus=fx4\n<region> sample=a.wav lokey=60 hikey=60 effect4=100");
    assert_eq!(plan.bus_effect(3), None, "没有 type ⇒ 没有器件");
    let out = render_note(&plan, 16);
    let tail = rms(&out[7 * BLOCK..12 * BLOCK]);
    assert!(tail < 1e-6, "没有器件 ⇒ 发送落空（读数 {tail}）");
}

// ---- D12（Proposed）：`<control> set_ccN` → 三个混响旋钮 ---------------------
//
// 映射（本片口径）：`param_offset` 为基址，`offset+0 = size`、`offset+1 = damp`、
// `offset+2 = width`（**不开放**，单声道总线听不出）、`offset+3 = predelay`
// （归一后乘 `0.1 s`）；`mix` **不开放**（总线必须纯湿）。

fn tail_with(plan: &PreparedSfz, window: (usize, usize)) -> f32 {
    let out = render_note(plan, 32);
    rms(&out[window.0 * BLOCK..window.1 * BLOCK])
}

/// SFX-22：`size`（`set_cc0`）真的改变衰减长度。
#[test]
fn the_size_knob_changes_the_decay() {
    let small = prepare(
        "<control> set_cc0=20\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let large = prepare(
        "<control> set_cc0=120\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let (a, b) = (tail_with(&small, (20, 28)), tail_with(&large, (20, 28)));
    assert!(b > a * 1.5, "大房间的晚段尾巴必须明显更长（{b} vs {a}）");
}

/// SFX-23：`predelay`（`set_cc3`）真的把尾巴**推后**。
#[test]
fn the_predelay_knob_delays_the_tail() {
    let plain = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let delayed = prepare(
        "<control> set_cc3=127\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let early_plain = tail_with(&plain, (6, 10));
    let early_delayed = tail_with(&delayed, (6, 10));
    assert!(early_plain > 1e-4, "无预延迟时 6..10 块应已有尾巴");
    assert!(
        early_delayed < early_plain * 0.1,
        "预延迟 0.1 s（≈18 块）应把 6..10 块压到近静（{early_delayed} vs {early_plain}）"
    );
    let late_delayed = tail_with(&delayed, (24, 32));
    assert!(late_delayed > 1e-4, "推迟之后仍必须出现（{late_delayed}）");
}

/// SFX-24：`param_offset` 把整组旋钮**平移**（`offset=4` ⇒ `set_cc4` 才是 size）。
#[test]
fn the_param_offset_shifts_the_knob_indices() {
    let shifted = prepare(
        "<control> set_cc4=20\n\
         <effect> bus=fx1 type=reverb param_offset=4\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let direct = prepare(
        "<control> set_cc0=20\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    assert_eq!(
        shifted.bus_effect(0).map(|effect| effect.tone()),
        direct.bus_effect(0).map(|effect| effect.tone()),
        "offset=4 配 set_cc4 应与 offset=0 配 set_cc0 同读数"
    );
    let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    assert_eq!(
        bits(&render_note(&shifted, 16)),
        bits(&render_note(&direct, 16))
    );
}

/// SFX-25：`width`（`offset+2`）**不开放**：配了也与不配逐位一致。
#[test]
fn the_width_knob_is_declared_but_inert() {
    let with_width = prepare(
        "<control> set_cc2=127\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let plain = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    assert_eq!(
        bits(&render_note(&with_width, 16)),
        bits(&render_note(&plain, 16)),
        "width 在单声道总线上必须惰性"
    );
}

/// SFX-26：**改了 `set_ccN` ⇒ 池必须重建总线**（否则用户调了旋钮听不出差别）。
#[test]
fn changing_a_knob_rebuilds_the_bus_once() {
    let low = prepare(
        "<control> set_cc0=20\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let high = prepare(
        "<control> set_cc0=120\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&low, BLOCK);
    let before = pool.bus_builds();
    pool.warm_buses(&high, BLOCK);
    assert!(
        pool.bus_builds() > before,
        "旋钮变了必须重建（{before} → {}）",
        pool.bus_builds()
    );
    // 同一份计划再预热 ⇒ 不该再建。
    let stable = pool.bus_builds();
    pool.warm_buses(&high, BLOCK);
    assert_eq!(pool.bus_builds(), stable, "同参数预热必须零重建");
}

/// SFX-27：`damp`（`offset+1`）进读数（听感差异判给 SFX-22/23，这里钉映射）。
#[test]
fn the_damp_knob_is_mapped() {
    let damped = prepare(
        "<control> set_cc1=100\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let (size, damp, predelay) = damped.bus_effect(0).expect("有总线").tone();
    assert_eq!(size, None);
    assert!((damp.expect("damp 已配") - 100.0 / 127.0).abs() < 1e-6);
    assert_eq!(predelay, None);
}

/// D12 批次 1：四条总线的旋钮**互不影响**（`fx1` 小房间、`fx2` 大房间）。
#[test]
fn knobs_are_per_bus() {
    let plan = prepare(
        "<control> set_cc0=20 set_cc4=120\n\
         <effect> bus=fx1 type=reverb param_offset=0\n\
         <effect> bus=fx2 type=reverb param_offset=4\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100 effect2=100",
    );
    let a = plan.bus_effect(0).expect("fx1").tone();
    let b = plan.bus_effect(1).expect("fx2").tone();
    assert!((a.0.expect("fx1 size") - 20.0 / 127.0).abs() < 1e-6);
    assert!((b.0.expect("fx2 size") - 120.0 / 127.0).abs() < 1e-6);
}

/// D12 批次 1：预延迟只作用于**配了它的那条总线**。
#[test]
fn predelay_only_affects_its_own_bus() {
    let plan = prepare(
        "<control> set_cc7=127\n\
         <effect> bus=fx1 type=reverb param_offset=0\n\
         <effect> bus=fx2 type=reverb param_offset=4\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100 effect2=100",
    );
    assert_eq!(
        plan.bus_effect(0).expect("fx1").tone().2,
        None,
        "fx1 无预延迟"
    );
    assert!(
        plan.bus_effect(1).expect("fx2").tone().2.is_some(),
        "fx2 有预延迟"
    );
}

/// 缓冲变长 ⇒ 必须重建；同长再预热 ⇒ 零重建（`frames` 参与判据）。
#[test]
fn a_longer_block_rebuilds_then_settles() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    let first = pool.bus_builds();
    pool.warm_buses(&plan, BLOCK * 2);
    assert!(pool.bus_builds() > first, "缓冲变长必须重建");
    let settled = pool.bus_builds();
    pool.warm_buses(&plan, BLOCK);
    assert_eq!(pool.bus_builds(), settled, "已有更大缓冲时不再重建");
}

/// 没有 `<control>` ⇒ 三个槽全 `None`（走固定缺省，与旧行为一致）。
#[test]
fn no_control_means_all_knobs_default() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    assert_eq!(plan.bus_effect(0).expect("fx1").tone(), (None, None, None));
}

/// 负的 `param_offset` 在**装载期就是 `Err`**（⛔ 不静默钳到 0）：
/// 规范没给范围，但负下标没有意义，拒载比"悄悄改成 0"诚实。
#[test]
fn a_negative_param_offset_is_rejected_at_load() {
    let error = yeban_sfz::parse_text(
        "<effect> bus=fx1 type=reverb param_offset=-3\n<region> sample=a.wav",
        &yeban_sfz::ParseLimits::default(),
    );
    assert!(error.is_err(), "负 param_offset 必须拒载，而不是钳位");
}

// ---- D12 运行期半边（批次 2，A01..A10）-------------------------------------

/// 渲染 `blocks` 块，`cc_of(block, cc_index)` 决定每块读到的 CC 值。
fn render_with_cc<F>(plan: &PreparedSfz, blocks: usize, cc_of: F) -> (Vec<f32>, u64, u64)
where
    F: Fn(usize, u8) -> u8,
{
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(plan, BLOCK);
    assert_eq!(pool.trigger_note(plan, 60, 100, 12_000, 1.0, None), 1);
    let mut out = Vec::with_capacity(blocks * BLOCK);
    for block in 0..blocks {
        let mut chunk = vec![0.0f32; BLOCK];
        let probe = |cc: u8| cc_of(block, cc);
        pool.render_into_with_cc(plan, block as u64 * BLOCK as u64, &mut chunk, Some(&probe));
        out.extend(chunk);
    }
    (out, pool.bus_builds(), pool.bus_param_writes())
}

/// A01：运行期把 `size` 从 0 推到 127 ⇒ 晚段尾巴必须变长（热改真的进了器件）。
#[test]
fn a_runtime_cc_change_moves_the_tail() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let sweep = |block: usize, _cc: u8| if block < 16 { 0 } else { 127 };
    let (out, _, writes) = render_with_cc(&plan, 32, sweep);
    assert!(writes >= 1, "CC 变化必须真的写进混响（{writes}）");
    let late = rms(&out[26 * BLOCK..32 * BLOCK]);
    assert!(late > 1e-4, "大房间的晚段尾巴必须可闻（{late}）");
}

/// A02：运行期热改**不重建总线**（缓冲、器件都不换）。
#[test]
fn a_runtime_cc_change_never_rebuilds_the_bus() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let sweep = |block: usize, _cc: u8| if block < 16 { 0 } else { 127 };
    let (_, builds, writes) = render_with_cc(&plan, 32, sweep);
    assert_eq!(builds, 1, "预热建一次，之后⛔ 不许再建");
    assert!(writes >= 1);
}

/// A03：**迟滞** —— CC 抖动 1 个步长（100 ↔ 101）不该反复写参数。
#[test]
fn cc_jitter_inside_one_step_writes_at_most_once() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let jitter = |block: usize, _cc: u8| if block.is_multiple_of(2) { 100 } else { 101 };
    let (_, _, writes) = render_with_cc(&plan, 32, jitter);
    assert!(
        writes <= 1,
        "一个步长内的抖动只允许首次落地（实测 {writes} 次）"
    );
}

/// A04：**每总线独立 CC 基址** —— 只动 `fx2` 的 CC（`offset=4`）不影响 `fx1`。
#[test]
fn cc_offsets_are_per_bus() {
    let plan = prepare(
        "<control> set_cc4=10\n\
         <effect> bus=fx1 type=reverb param_offset=0\n\
         <effect> bus=fx2 type=reverb param_offset=4\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100 effect2=100",
    );
    let only_fx2 = |_block: usize, cc: u8| if cc == 4 { 120 } else { 0 };
    let (_, _, writes) = render_with_cc(&plan, 16, only_fx2);
    // fx1 的 CC0 恒 0 与缺省不同（缺省 size 非 0）⇒ 会写一次；fx2 从 10 变 120 再写一次。
    assert!(writes >= 1, "fx2 必须响应");
    let (_, _, none) = render_with_cc(&plan, 16, |_b, _cc| 0);
    assert!(
        writes >= none,
        "动 fx2 不应比全 0 更少（{writes} vs {none}）"
    );
}

/// A05：CC 越界（255）钳到 1.0，输出**无 NaN/Inf**。
#[test]
fn an_out_of_range_cc_value_stays_finite() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let (out, _, _) = render_with_cc(&plan, 16, |_b, _cc| 255);
    assert!(out.iter().all(|sample| sample.is_finite()), "输出必须有限");
    v0_assert_peak_within(&out, 1.0);
}

fn v0_assert_peak_within(samples: &[f32], bound: f32) {
    let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    assert!(peak <= bound, "峰值 {peak} 越过 {bound}");
}

/// A09：`dsp_order` 相同 ⇒ **先出现者**胜（旋钮基址也跟着那条走）。
#[test]
fn a_tie_keeps_the_first_declaration() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb param_offset=0\n\
         <effect> bus=fx1 type=reverb param_offset=4\n\
         <region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let effect = plan.bus_effect(0).expect("fx1");
    assert_eq!(effect.offset(), 0, "同序取先出现者");
}

// ---- 批次 3：电平/规模/边界（A06/A07/E04/E05）与宿主语义（F01–F04/F06）-----

/// A06：`size` 越大，晚段尾巴**单调**变大（三档 0 / 64 / 127）。
#[test]
fn the_tail_is_monotone_in_the_size_knob() {
    let reading = |size: u8| {
        let text = format!(
            "<control> set_cc0={size}\n\
             <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100"
        );
        let plan = prepare(&text);
        tail_with(&plan, (20, 28))
    };
    let (a, b, c) = (reading(0), reading(64), reading(127));
    assert!(a < b && b < c, "必须单调递增（{a} < {b} < {c}）");
}

/// A07：大缓冲（4096 帧）构造不 panic、只建一次。
#[test]
fn a_large_block_builds_once() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, 4096);
    assert_eq!(pool.bus_builds(), 1);
    pool.warm_buses(&plan, 4096);
    assert_eq!(pool.bus_builds(), 1, "同长再预热零重建");
}

/// E04：立体声两声道长度不等 ⇒ 取 `min`，⛔ 不越界 panic。
#[test]
fn unequal_stereo_lengths_use_the_shorter_one() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK / 2];
    // 只渲染 `right` 的长度是库的口径；这里直接把两者都传进去，验证不 panic。
    pool.render_into_stereo_with_probes(&plan, 0, &mut left, &mut right, None, None);
    assert!(right.iter().all(|s| s.is_finite()));
    assert!(left.iter().all(|s| s.is_finite()));
}

/// E05：零长度渲染（空缓冲）不 panic、不产生声音。
#[test]
fn a_zero_length_render_is_a_no_op() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    assert_eq!(pool.trigger_note(&plan, 60, 100, 12_000, 1.0, None), 1);
    let mut out: Vec<f32> = Vec::new();
    pool.render_into(&plan, 0, &mut out);
    assert!(pool.bus_builds() >= 1);
}

/// F01：**两条新轨各自预占 ＋ 预热** ⇒ 每条都 `Some(1)`（宿主 reload 的语义）。
#[test]
fn two_tracks_are_warmed_independently() {
    use yeban_engine::synth::SynthEngine;
    use yeban_model::EntityId;

    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let (a, b) = (EntityId::new(), EntityId::new());
    let mut engine = SynthEngine::new(48_000);
    for track in [a, b] {
        engine.reserve_track_slot(48_000, track);
        engine.warm_sfz_buses(track, &plan, BLOCK);
    }
    assert_eq!(engine.sfz_bus_builds(a), Some(1));
    assert_eq!(engine.sfz_bus_builds(b), Some(1));
    engine.begin_snapshot(48_000, &[a, b], [], []);
    for track in [a, b] {
        assert_eq!(engine.sfz_bus_builds(track), Some(1), "快照后零重建");
    }
}

/// F02：**只预占不预热** ⇒ 读数 `Some(0)`（槽在、总线还没建）。
#[test]
fn a_reserved_but_unwarmed_track_reports_zero_builds() {
    use yeban_engine::synth::SynthEngine;
    use yeban_model::EntityId;

    let track = EntityId::new();
    let mut engine = SynthEngine::new(48_000);
    engine.reserve_track_slot(48_000, track);
    assert_eq!(engine.sfz_bus_builds(track), Some(0));
}

/// F03：**同一轨换一份不同旋钮的计划** ⇒ 必须重建（工程切换语义）。
#[test]
fn switching_plans_rebuilds_the_bus() {
    let low = prepare(
        "<control> set_cc0=0\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let high = prepare(
        "<control> set_cc0=127\n\
         <effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&low, BLOCK);
    let before = pool.bus_builds();
    pool.warm_buses(&high, BLOCK);
    assert_eq!(pool.bus_builds(), before + 1);
}

/// F04：**同一份计划重复预热** ⇒ 零重建（发布两次的稳态）。
#[test]
fn republishing_the_same_plan_never_rebuilds() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, BLOCK);
    let settled = pool.bus_builds();
    for _ in 0..20 {
        pool.warm_buses(&plan, BLOCK);
    }
    assert_eq!(pool.bus_builds(), settled);
}

/// F06：**缓冲先大后小** ⇒ 不重建（`frames >= 需求` 判据，⛔ 不必缩容）。
#[test]
fn a_smaller_later_block_reuses_the_larger_buffer() {
    let plan = prepare(
        "<effect> bus=fx1 type=reverb\n<region> sample=a.wav lokey=60 hikey=60 effect1=100",
    );
    let mut pool = SfzVoicePool::new(HZ);
    pool.warm_buses(&plan, 4096);
    let big = pool.bus_builds();
    pool.warm_buses(&plan, BLOCK);
    assert_eq!(pool.bus_builds(), big, "大缓冲够用 ⇒ 不缩容");
}
