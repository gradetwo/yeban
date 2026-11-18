//! ⭐ **stage 24：钳位告警在引擎路径上必须"看得见"**。
//!
//! # 为什么需要这一条（不是重复上游判据）
//!
//! `yeban-sfz` 自己已有两条装载级判据（`tests/modulation_depth_clamp.rs`、
//! `tests/static_pan_clamp.rs`），但它们都走 `parse_text` —— **引擎一路的入口
//! 是 `SfzInstrument::load`**，而它在 stage 23 之前把 `Instrument` 直接 `?` 掉，
//! 只留下 `region_count`。也就是说：**上游钳位告警在引擎侧完全不可见**，
//! 引擎的调用方（`yeban-app` / 离线渲染）拿到的是一个"看起来一切正常"的乐器。
//!
//! 本判据钉三件事：
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | 1 | 静态声像越界（`pan=200`）⇒ 引擎装载成功 ＋ `ClampedStaticValue` 逐字可查 ＋ **生效值就是边界** | `SfzInstrument::warnings` ＋ `PreparedRegion::pan_with_cc(None)` |
//! | 2 | 调制深度越界（`pan_oncc10=200`）⇒ `ClampedModulationDepth` 可查 ＋ 满 CC 时有效声相落在 100 | 同上 ＋ `pan_with_cc(Some(..))` |
//! | 3 | 域内值不产生告警（⛔ 不能"凡是 pan 就告警"） | `warnings().is_empty()` |
//! | 4 | 声像族**之外**的静态越界仍必须响亮 `Err`（⛔ 钳位没有把整片口子放开） | `SfzInstrument::load(..).is_err()` |

use yeban_engine::sfz_instrument::SfzInstrument;
use yeban_sfz::Warning;
use yeban_sfz::modulation::ModTarget;

/// 引擎装载后的**源乐器**（`with_instrument` 的只读重放）：
/// 返回 (静态 `pan`, `pan` 调制项数量, 第一条 `pan` 调制项的 `delta`)。
fn pan_readings(instrument: &SfzInstrument) -> (f32, usize, f32) {
    instrument
        .with_instrument(|source| {
            let region = &source.regions()[0];
            let mods: Vec<&_> = region
                .cc_modulations()
                .iter()
                .filter(|item| item.target == ModTarget::Pan)
                .collect();
            let delta = mods.first().map_or(f32::NAN, |item| item.delta);
            (region.pan, mods.len(), delta)
        })
        .expect("已成功装载的乐器必须可重放")
}

fn load(text: &str) -> SfzInstrument {
    SfzInstrument::load(text.to_owned(), &Default::default())
        .unwrap_or_else(|error| panic!("`{text}` 必须可装载：{error}"))
}

/// 一条输入里全部 `ClampedStaticValue` 载荷：(opcode, 原值, 钳位值)。
fn static_clamps(instrument: &SfzInstrument) -> Vec<(String, String, String)> {
    instrument
        .warnings()
        .iter()
        .filter_map(|warning| match warning {
            Warning::ClampedStaticValue {
                opcode,
                value,
                clamped,
                ..
            } => Some((opcode.clone(), value.clone(), clamped.clone())),
            _ => None,
        })
        .collect()
}

/// 一条输入里全部 `ClampedModulationDepth` 载荷。
fn depth_clamps(instrument: &SfzInstrument) -> Vec<(String, String, String)> {
    instrument
        .warnings()
        .iter()
        .filter_map(|warning| match warning {
            Warning::ClampedModulationDepth {
                opcode,
                value,
                clamped,
                ..
            } => Some((opcode.clone(), value.clone(), clamped.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn clamped_values_are_visible_through_the_engine_entry() {
    // ---- 1) 静态声像越界：装载成功、告警逐字可查、生效值即边界 ----
    let high = load("<region> sample=a.wav key=60 pitch_keycenter=60 pan=200");
    assert_eq!(
        static_clamps(&high),
        vec![("pan".to_owned(), "200".to_owned(), "100".to_owned())],
        "⭐ `pan=200` 必须在引擎侧留下 ClampedStaticValue（opcode/原值/钳位值 逐字）"
    );
    assert_eq!(high.region_count(), 1, "钳位不影响区域数");
    assert_eq!(
        pan_readings(&high).0,
        100.0,
        "⭐ 生效声相必须是边界 100（⛔ 不是 200 原样透传）"
    );

    let low = load("<region> sample=a.wav key=60 pitch_keycenter=60 pan=-250");
    assert_eq!(
        static_clamps(&low),
        vec![("pan".to_owned(), "-250".to_owned(), "-100".to_owned())]
    );
    assert_eq!(pan_readings(&low).0, -100.0);

    // 两个越界值同时出现 ⇒ 各记一条（⛔ 不合并成一条）
    let both = load("<region> sample=a.wav key=60 pitch_keycenter=60 pan=200 position=150");
    assert_eq!(both.warnings().len(), 2, "两个不同 opcode 各一条告警");
    assert!(
        static_clamps(&both)
            .iter()
            .any(|(op, _, _)| op == "position")
    );

    // ---- 2) 调制深度越界：告警可查 ＋ 满 CC 时有效声相落在边界 ----
    let modded = load("<region> sample=a.wav key=60 pitch_keycenter=60 pan_oncc10=200");
    assert_eq!(
        depth_clamps(&modded),
        vec![("pan_oncc10".to_owned(), "200".to_owned(), "100".to_owned())],
        "⭐ `pan_oncc10=200` 的深度必须被钳到 100 且告警在引擎侧可见"
    );
    let (base, mods, delta) = pan_readings(&modded);
    assert_eq!(mods, 1, "调制项本身仍然存在（钳的是深度，⛔ 不是把项删掉）");
    assert_eq!(delta, 100.0, "⭐ 深度必须被钳到 100（载荷与存储值一致）");
    assert_eq!(base, 0.0, "静态 `pan` 未越界 ⇒ 原样 0");

    // ---- 3) 域内值：安静通过，⛔ 不能"凡是声像就告警" ----
    for text in [
        "<region> sample=a.wav key=60 pitch_keycenter=60 pan=100",
        "<region> sample=a.wav key=60 pitch_keycenter=60 pan=-100",
        "<region> sample=a.wav key=60 pitch_keycenter=60 pan=0",
        "<region> sample=a.wav key=60 pitch_keycenter=60 pan_oncc10=100",
        "<region> sample=a.wav key=60 pitch_keycenter=60",
    ] {
        let instrument = load(text);
        assert!(
            instrument.warnings().is_empty(),
            "⭐ `{text}` 在域内 ⇒ 不该有任何告警（实得 {:?}）",
            instrument.warnings()
        );
    }

    // ---- 4) 口径边界：声像族之外仍严格报错 ----
    for text in [
        // ⚠ 这里必须是**静态**参数：`ampeg_attack_oncc1=101` 是调制量 ⇒ 会被钳位
        // （见第 2 段），不能拿来当"严格口径"的对照腿。
        "<region> sample=a.wav amp_veltrack=101",
        "<region> sample=a.wav pan=abc",
    ] {
        assert!(
            SfzInstrument::load(text.to_owned(), &Default::default()).is_err(),
            "⭐ `{text}` 必须响亮 Err（钳位只放宽声像族与调制量，⛔ 不是全放开）"
        );
    }
}
