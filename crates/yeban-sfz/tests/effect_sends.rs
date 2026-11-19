//! ⭐ **`effect1..4` 解析判据**（`ADR-0077` D2 的装载半边）。
//!
//! # 本判据在防什么
//!
//! `<effect>` 的**声明**此前已建模（`bus` / `type` / `dsp_order`），但 region 侧的
//! **发送量** `effect1..4` 一直没有解析 —— 它会被当成未知 opcode 吞掉，
//! 于是"作者写了 50% 发送"与"没写"在模型层**无法区分**。本片把四个数搬进
//! [`yeban_sfz::instrument::Region::effect_sends`] 与
//! [`yeban_sfz::playback::PlaybackSpec::effect_sends`]：
//!
//! * 缺省 `[0.0; 4]`（没有声明的 region ⇒ 一切既有行为逐位不变）；
//! * 作用域链同 `pan` / `width`（region → group → master → global）；
//! * 越界（`<0` / `>100` / 非有限）⇒ 装载期钳位 ＋ [`Warning::ClampedStaticValue`]
//!   （与声像族同一口径，⛔ 不静默改写、⛔ 不拒载）。
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | 1 | `effect1=50 effect2=0 effect3=100 effect4=25` 原样搬进 `Region` | `effect_sends` |
//! | 2 | 没写 ⇒ 四项全 0、无告警 | `effect_sends` ＋ `warnings()` |
//! | 3 | `group` 级写的发送量对 region 生效（作用域链） | `effect_sends` |
//! | 4 | `effect2=150` 钳到 100 并点名告警；`effect3=-5` 钳到 0 | 存储值 ＋ `Warning::ClampedStaticValue` |
//! | 5 | `PlaybackSpec` 与 `Region` 同值（消费方只拿 spec 也能读到） | 两处读数 |

use yeban_sfz::{ParseLimits, Warning, parse_text};

fn load(text: &str) -> yeban_sfz::Instrument<'_> {
    parse_text(text, &ParseLimits::default())
        .unwrap_or_else(|error| panic!("`{text}` 必须可装载：{error}"))
}

#[test]
fn four_sends_are_carried_into_the_region_verbatim() {
    let text = "<region> sample=a.wav effect1=50 effect2=0 effect3=100 effect4=25";
    let instrument = load(text);
    assert_eq!(
        instrument.regions()[0].effect_sends,
        [50.0, 0.0, 100.0, 25.0]
    );
    assert!(
        instrument.warnings().is_empty(),
        "域内发送量必须零告警：{:?}",
        instrument.warnings()
    );
}

#[test]
fn without_the_opcodes_every_send_is_zero() {
    let instrument = load("<region> sample=a.wav");
    assert_eq!(instrument.regions()[0].effect_sends, [0.0; 4]);
}

#[test]
fn group_scope_feeds_the_region() {
    let text = "<group>effect1=40\n<region>sample=a.wav lokey=60 hikey=60";
    let instrument = load(text);
    assert_eq!(instrument.regions()[0].effect_sends[0], 40.0);
}

#[test]
fn out_of_range_sends_are_clamped_and_named() {
    let instrument = load("<region> sample=a.wav effect2=150 effect3=-5");
    let sends = instrument.regions()[0].effect_sends;
    assert_eq!((sends[1], sends[2]), (100.0, 0.0));
    let notes: Vec<_> = instrument
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::ClampedStaticValue { .. }))
        .collect();
    assert_eq!(notes.len(), 2, "两条越界各一条告警：{notes:?}");
    assert!(matches!(
        notes[0],
        Warning::ClampedStaticValue { opcode, value, clamped, .. }
            if opcode == "effect2" && value == "150" && clamped == "100"
    ));
    assert!(matches!(
        notes[1],
        Warning::ClampedStaticValue { opcode, value, clamped, .. }
            if opcode == "effect3" && value == "-5" && clamped == "0"
    ));
}

#[test]
fn the_playback_spec_carries_the_same_four_numbers() {
    let instrument = load("<region> sample=a.wav effect1=30 effect4=70");
    let region = &instrument.regions()[0];
    let spec = region.playback_spec(60, 100, Default::default());
    assert_eq!(spec.effect_sends, [30.0, 0.0, 0.0, 70.0]);
}
