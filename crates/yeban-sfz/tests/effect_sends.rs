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
    // 四条总线都声明了（否则 `effect3/4` 会触发孤儿发送告警，那是另一条判据）。
    let text = "<effect> bus=fx1 type=reverb\n<effect> bus=fx2 type=reverb\n\
                <effect> bus=fx3 type=reverb\n<effect> bus=fx4 type=reverb\n\
                <region> sample=a.wav effect1=50 effect2=0 effect3=100 effect4=25";
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

// ---- 作用域链更深的两格（`global` / 嵌套 group / region 覆盖） ---------------

/// `global` 级的发送量对 region 生效（作用域链的底）。
#[test]
fn global_scope_feeds_the_region() {
    let text = "<global>effect3=30\n<region>sample=a.wav lokey=60 hikey=60";
    let instrument = load(text);
    assert_eq!(instrument.regions()[0].effect_sends[2], 30.0);
}

/// region 的显式值**覆盖** group 的值（近者优先）。
#[test]
fn region_overrides_the_group_send() {
    let text = "<group>effect1=40\n<region>sample=a.wav effect1=90";
    let instrument = load(text);
    assert_eq!(instrument.regions()[0].effect_sends[0], 90.0);
}

/// 嵌套 group：内层覆盖外层（逐层逼近）。
#[test]
fn the_inner_group_wins() {
    let text = "<group>effect2=10\n<group>effect2=70\n<region>sample=a.wav";
    let instrument = load(text);
    assert_eq!(instrument.regions()[0].effect_sends[1], 70.0);
}

// ---- 告警口径（`ADR-0077` D3 / **SFX-11**） ----------------------------------

/// `bus=fx5`（不在 `fx1..fx4` 射程）⇒ **仍告警**，且 bus 文本如实写 `fx5`。
///
/// 防的是"把 `fx5` 当 `fx4` 静默吞掉"：出了射程就必须响亮。
#[test]
fn an_out_of_range_fx_bus_still_warns_with_its_own_name() {
    let text = "<effect> bus=fx5 type=reverb\n<region> sample=a.wav effect1=100";
    let instrument = load(text);
    let notes: Vec<_> = instrument
        .warnings()
        .iter()
        .filter_map(|warning| match warning {
            Warning::EffectNotProcessed { bus, kind, .. } => Some((bus.clone(), kind.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(notes.len(), 1, "一条 `<effect>` 只许一条告警：{notes:?}");
    assert_eq!(notes[0].0, "fx5", "bus 文本必须如实");
    assert_eq!(notes[0].1.as_deref(), Some("reverb"));
}

/// 一条不支持的 `<effect>` 面对**多条 region** 时，告警仍然只有**一条**
/// （按声明去重，不按引用它的 region 数放大）。
#[test]
fn one_unsupported_effect_warns_once_no_matter_how_many_regions() {
    let text = "<effect> bus=main type=chorus\n\
                <region> sample=a.wav lokey=60 hikey=60 effect1=100\n\
                <region> sample=a.wav lokey=62 hikey=64 effect1=100\n\
                <region> sample=a.wav lokey=66 hikey=68 effect1=50";
    let instrument = load(text);
    let count = instrument
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::EffectNotProcessed { .. }))
        .count();
    assert_eq!(count, 1, "三条 region 不能让同一条声明告警三次");
}

/// 已接线的组合（`fx1..fx4` ＋ `type` 含 `reverb`）⇒ **零告警**。
///
/// 与上一条成对：证明"零告警"不是"什么都没读"的副作用。
#[test]
fn a_supported_reverb_bus_is_silent() {
    let text = "<effect> bus=fx2 type=Reverb\n<region> sample=a.wav effect2=100";
    let instrument = load(text);
    assert!(
        instrument.warnings().is_empty(),
        "已接线的总线必须零告警：{:?}",
        instrument.warnings()
    );
}

// ---- 孤儿发送（`Warning::EffectSendWithoutBus`） ---------------------------

fn orphan_buses(instrument: &yeban_sfz::Instrument<'_>) -> Vec<u8> {
    instrument
        .warnings()
        .iter()
        .filter_map(|warning| match warning {
            Warning::EffectSendWithoutBus { bus, .. } => Some(*bus),
            _ => None,
        })
        .collect()
}

/// 写了 `effect1=100` 但整份文件没有 `bus=fx1` 的 `<effect>` ⇒ **告警**
/// （否则用户以为自己在用混响，实际纯干声、无声无息）。
#[test]
fn a_send_without_a_declared_bus_warns() {
    let instrument = load("<region> sample=a.wav effect1=100");
    assert_eq!(orphan_buses(&instrument), vec![1], "必须点名缺 fx1");
}

/// 声明齐了 ⇒ 零孤儿告警（与上一条成对）。
#[test]
fn a_send_with_a_declared_bus_stays_silent() {
    let instrument = load("<effect> bus=fx1 type=reverb\n<region> sample=a.wav effect1=100");
    assert!(orphan_buses(&instrument).is_empty());
    assert!(
        instrument.warnings().is_empty(),
        "{:?}",
        instrument.warnings()
    );
}

/// 只有 `fx1` 声明，却往 `fx2` 发 ⇒ 点名缺 `fx2`（⛔ 不因为"有 fx1"就放过）。
#[test]
fn the_warning_names_the_actually_missing_bus() {
    let instrument = load("<effect> bus=fx1 type=reverb\n<region> sample=a.wav effect2=60");
    assert_eq!(orphan_buses(&instrument), vec![2]);
}

/// 多条 region 都缺同一条总线 ⇒ 只报**一条**（按总线号去重）。
#[test]
fn many_regions_share_one_orphan_warning() {
    let instrument = load(
        "<region> sample=a.wav lokey=60 hikey=62 effect3=40\n\
         <region> sample=a.wav lokey=64 hikey=66 effect3=80",
    );
    assert_eq!(orphan_buses(&instrument), vec![3], "同一总线只许一条");
}

/// `bus=fx1` 声明了但类型不支持（`chorus`）⇒ 已有 `EffectNotProcessed`，
/// ⛔ 不再叠一条孤儿告警（避免同一个问题报两次）。
#[test]
fn a_declared_but_unsupported_bus_is_not_also_an_orphan() {
    let instrument = load("<effect> bus=fx1 type=chorus\n<region> sample=a.wav effect1=100");
    assert!(
        orphan_buses(&instrument).is_empty(),
        "{:?}",
        instrument.warnings()
    );
    let processed = instrument
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::EffectNotProcessed { .. }))
        .count();
    assert_eq!(processed, 1);
}

/// 孤儿发送的判定吃**继承值**：group 写 `effect1=100`、region 不写、文件里也
/// 没有 `fx1` 声明 ⇒ 照样告警（否则"group 上写的发送"会漏报）。
#[test]
fn a_group_level_orphan_send_is_still_reported() {
    let instrument = load("<group>effect1=100\n<region>sample=a.wav");
    assert_eq!(orphan_buses(&instrument), vec![1]);
}

/// 空 `<effect bus=fx4>`（没有 `type`）⇒ 走 `EffectNotProcessed`，
/// ⛔ 不算孤儿发送（总线"声明过了"，只是没有器件）。
#[test]
fn an_empty_effect_is_not_an_orphan() {
    let instrument = load("<effect> bus=fx4\n<region> sample=a.wav effect4=100");
    assert!(
        orphan_buses(&instrument).is_empty(),
        "{:?}",
        instrument.warnings()
    );
    let processed = instrument
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::EffectNotProcessed { .. }))
        .count();
    assert_eq!(processed, 1, "空 type 仍要响亮告警一次");
}
