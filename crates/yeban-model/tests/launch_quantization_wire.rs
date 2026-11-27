//! ⭐ T-FIX-824：`LaunchQuantization` 的线协议名必须与 `project.schema.json` 逐字一致。
//!
//! 旧五档是 `.yeban` 的既有词汇，⛔ 不许改名；新增四档只在末尾追加，
//! 让旧文档读回 `Bar` / `TwoBars` / `FourBars` / `EightBars` / `Off` 仍逐字节相同。

use yeban_model::LaunchQuantization;

#[test]
fn launch_quantization_wire_names_are_stable_and_cover_sub_beat_tiers() {
    let cases = [
        (LaunchQuantization::Off, "Off"),
        (LaunchQuantization::Bar, "Bar"),
        (LaunchQuantization::TwoBars, "TwoBars"),
        (LaunchQuantization::FourBars, "FourBars"),
        (LaunchQuantization::EightBars, "EightBars"),
        (LaunchQuantization::HalfBeat, "HalfBeat"),
        (LaunchQuantization::QuarterBeat, "QuarterBeat"),
        (LaunchQuantization::EighthBeat, "EighthBeat"),
        (LaunchQuantization::SixteenthBeat, "SixteenthBeat"),
    ];
    for (quantization, name) in cases {
        let json = serde_json::to_string(&quantization).expect("序列化");
        assert_eq!(json, format!("\"{name}\""), "线协议名不得漂移: {name}");
        let back: LaunchQuantization = serde_json::from_str(&json).expect("合法线协议名必须可读回");
        assert_eq!(back, quantization);
    }
    assert_eq!(cases.len(), 9, "旧五档 + 新四档必须全部覆盖");
}
