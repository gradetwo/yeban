//! ⭐ 阶段 2（engine 侧）：记法适配在 [`SfzInstrument`] 上的施加与摊平继承。
//!
//! 设计：`docs/ledger/sfz-core-notes.md` §14.4。判定本体在 `yeban-sfz`
//! （`detect_notation_shift`，边界判据在该 crate 的单元测试里）；本文件只证
//! **接线**：
//!
//! | 判据 | 内容 |
//! | :--- | :--- |
//! | [`hit_shifts_selection_range_and_the_flattened_plan`] | (b) 命中 ⇒ 选择谓词、键位范围、**摊平表**都落在移位后的键上 |
//! | [`manual_override_short_circuits_detection`] | (c) 在场 ⇒ (b) 停跑（报告为 `None`），覆盖值直接生效，显式 0 同理 |
//! | [`scattered_roots_miss_and_keep_the_report`] | SSO 式散列根音 ⇒ 不命中、不移位，结论带原因存进乐器 |
//! | [`no_adaptation_means_unshifted`] | 从不适配 ⇒ 外部移位 0、无报告、键位照旧（回归） |

use yeban_engine::sfz_instrument::{InMemorySample, NotationShiftSource, SfzInstrument, SfzPlan};
use yeban_sfz::parser::ParseLimits;

/// Yamaha 式小乐器：`key=c3` 意指 60（作者记法比 IPN 低一个八度），
/// 六个 region 的嵌入根音 60..=65 与写出键位逐一差 +12。
const YAMAHA_STYLE: &str = "\
<region> sample=n0.wav key=c3
<region> sample=n1.wav key=c#3
<region> sample=n2.wav key=d3
<region> sample=n3.wav key=d#3
<region> sample=n4.wav key=e3
<region> sample=n5.wav key=f3
";

/// `n{i}.wav` → 嵌入根音 `60 + i`（与 [`YAMAHA_STYLE`] 的写出键位差恒 +12）。
fn yamaha_root(sample_path: &str) -> Option<i32> {
    let index: i32 = sample_path
        .strip_prefix('n')?
        .strip_suffix(".wav")?
        .parse()
        .ok()?;
    Some(60 + index)
}

/// SSO 式散列根音（缺省值形状）：与写出键位的差是 12/11/12/4/8/−5。
fn scattered_root(sample_path: &str) -> Option<i32> {
    let index: usize = sample_path
        .strip_prefix('n')?
        .strip_suffix(".wav")?
        .parse()
        .ok()?;
    [60, 60, 62, 55, 60, 48].get(index).copied()
}

fn load_yamaha() -> SfzInstrument {
    SfzInstrument::load(YAMAHA_STYLE.to_string(), &ParseLimits::default())
        .expect("夹具 SFZ 必须合法")
}

#[test]
fn hit_shifts_selection_range_and_the_flattened_plan() {
    let mut instrument = load_yamaha();
    let adaptation = instrument
        .adapt_notation(&yamaha_root, None)
        .expect("适配不得失败");
    assert_eq!(adaptation.applied_shift, 12);
    assert_eq!(adaptation.source, NotationShiftSource::AutoDetected);
    let detection = adaptation.detection.expect("(b) 跑过 ⇒ 必须有报告");
    assert_eq!(detection.evidence_count, 6);
    assert_eq!(detection.sampled_region_count, 6);
    assert_eq!(detection.matching_count(), 6);
    assert_eq!(instrument.external_shift(), 12);
    assert!(
        instrument.notation_detection().is_some(),
        "结论必须存进乐器供回执/徽标读"
    );

    // 选择谓词落在移位后的键上：60 命中、48（移位前的写出键）不命中。
    assert!(
        !instrument
            .regions_matching(60, 100)
            .expect("查询")
            .is_empty(),
        "⭐ 命中后按 60 必须选得到 region"
    );
    assert!(
        instrument
            .regions_matching(48, 100)
            .expect("查询")
            .is_empty(),
        "⭐ 移位前的键 48 不许再命中"
    );
    // 键位范围是移位后的实际覆盖范围。
    assert_eq!(instrument.key_range().expect("范围"), Some((60, 65)));

    // ⭐ 摊平表继承移位（PreparedSfz 路径与控制侧查询同一份解析）。
    let plan = SfzPlan::prepare(
        &instrument,
        &InMemorySample::sine(440.0, 48_000.0, 4_800),
        48_000.0,
    )
    .expect("摊平");
    let regions = plan.prepared().regions();
    assert_eq!(regions.len(), 6);
    assert!(
        regions[0].accepts(60, 100),
        "⭐ 摊平后的 region 0 必须接受 60"
    );
    assert!(
        !regions[0].accepts(48, 100),
        "⭐ 摊平后的 region 0 不许再接受 48"
    );
}

#[test]
fn manual_override_short_circuits_detection() {
    // 根音散列（(b) 本会不命中），但 (c) = +12 在场 ⇒ 停跑检测、直接生效。
    let mut instrument = load_yamaha();
    let adaptation = instrument
        .adapt_notation(&scattered_root, Some(12))
        .expect("适配不得失败");
    assert_eq!(adaptation.applied_shift, 12);
    assert_eq!(adaptation.source, NotationShiftSource::ManualOverride);
    assert_eq!(adaptation.detection, None, "(c) 在场 ⇒ (b) 停跑、无报告");
    assert_eq!(instrument.notation_detection(), None);
    assert!(
        !instrument
            .regions_matching(60, 100)
            .expect("查询")
            .is_empty(),
        "⭐ 覆盖后 60 必须触发（SSO 鼓组靠 (c) 解决的那条路）"
    );

    // 显式 0 同样是覆盖：停跑检测、不移位。
    let mut instrument = load_yamaha();
    let adaptation = instrument
        .adapt_notation(&yamaha_root, Some(0))
        .expect("适配不得失败");
    assert_eq!(adaptation.applied_shift, 0);
    assert_eq!(adaptation.source, NotationShiftSource::ManualOverride);
    assert_eq!(adaptation.detection, None);
    assert!(
        !instrument
            .regions_matching(48, 100)
            .expect("查询")
            .is_empty(),
        "显式 0 ⇒ 键位照写出值（48）"
    );
}

#[test]
fn scattered_roots_miss_and_keep_the_report() {
    let mut instrument = load_yamaha();
    let adaptation = instrument
        .adapt_notation(&scattered_root, None)
        .expect("适配不得失败");
    assert_eq!(adaptation.applied_shift, 0);
    assert_eq!(adaptation.source, NotationShiftSource::None);
    let detection = adaptation.detection.expect("未命中也必须有报告");
    assert_eq!(
        detection.miss_reason,
        Some(yeban_sfz::NotationMissReason::NoConsistentOctave)
    );
    assert_eq!(instrument.external_shift(), 0);
    assert_eq!(instrument.key_range().expect("范围"), Some((48, 53)));
}

#[test]
fn no_adaptation_means_unshifted() {
    let instrument = load_yamaha();
    assert_eq!(instrument.external_shift(), 0);
    assert_eq!(instrument.notation_detection(), None);
    assert_eq!(instrument.key_range().expect("范围"), Some((48, 53)));
    assert!(
        !instrument
            .regions_matching(48, 100)
            .expect("查询")
            .is_empty()
    );
}
