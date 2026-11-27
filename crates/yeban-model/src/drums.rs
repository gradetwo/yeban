//! **鼓机识别表**：鼓机键位名、音色旋钮名与它们的投影规则 ——
//! 一台 `InternalInstrument` 设备是不是鼓机、它的每个参数名认不认，
//! **唯一**由这里决定。
//!
//! 本模块是纯名字 / 数值表：只做字符串比较、`f32` 转换与
//! [`DrumKitParams`] 字段赋值，不碰音频线程、不碰 `cpal`、不碰 GUI。
//! 它依赖的只有 `yeban-model` 自身与 `yeban-dsp` 的鼓机参数类型 ——
//! 因此 `yeban-mcp` 可以直接依赖它而不拖进音频栈。
//! （T-FIX-846 自 `yeban-engine/src/drums.rs` 下移；
//! 取向见 `docs/DEVELOPMENT_LEDGER.md` Round 261/273 ——
//! "把共享件**下移到轻量 crate**，而不是加依赖"；
//! `f90a7609` 把 `record.rs` 下移成 `yeban-record` 是同一形态的先例，
//! 引擎侧保留 `pub use` 再导出。）
//!
//! ## 谁消费这张表
//!
//! - 引擎投影：`yeban-engine::drums::DrumsParams::from_devices` /
//!   `from_device` —— 名字 → 槽位、旋钮 → `DrumKitParams` 字段，
//!   再由 `DrumKitParams::sanitised` 收尾钳制；
//! - MCP 诊断：`yeban-mcp` 的 `instrument_param_warnings`
//!   （`unknown_kit_param_names`，T-FIX-746）与键位不全诊断
//!   （`incomplete_drum_note_map`）；
//! - 判据：仓内有判据按源码读 `note_slot` 的 match 臂对账
//!   （改名 ⇒ 判据红，⛔ 不许静默漂移），读的是**本文件** ——
//!   `note_slot` 的签名必须保持 `fn note_slot(name: &str) -> Option<usize> {`
//!   这一字面形状（`pub` 前缀不影响：判据按子串查找）。
//!
//! ## 口径（与引擎 `drums.rs` 模块文档 §3 同款）
//!
//! 名字大小写不敏感（调用方先 `to_ascii_lowercase`）、同名后者胜；
//! 五个键位名必须**全部**写出且取值有限，缺一或非有限 ⇒ 整台设备不是鼓机
//! （规则 5）；音色值先赋原值、由 [`DrumKitParams::sanitised`] 收尾统一钳制
//! （规则 6），本模块不持有第二份值域表。

use yeban_dsp::drums::{ClapParams, DrumKitParams, KickStyle};

use crate::project::{DeviceDefinition, DeviceKind};

/// 五个键位名的**规范拼写**（下标 = 键位槽位，顺序 = `DrumVoice::ALL`）。
///
/// 这是 [`note_slot`] 那张识别名单的**报告用**形状：识别本身仍走 `note_slot`
/// （仓内有判据按源码读它的 match 臂对账），本表供诊断点名缺键时逐字引用。
/// 两份名单的一致性由本模块的判据 `note_names_table_matches_note_slot` 钉住。
pub const DRUM_NOTE_NAMES: [&str; NOTE_SLOTS] = [
    "kick_note",
    "snare_note",
    "closed_hat_note",
    "open_hat_note",
    "clap_note",
];

/// 诊断：这台设备是**鼓机形状但键位不全**时，逐条点名缺的键位名；其余情况 `None`。
///
/// **判别规则**（与 `DrumsParams::from_device`（引擎投影）的识别口径逐条同款）：
/// - `kind != InternalInstrument` ⇒ `None`（不是乐器设备，与鼓机无关）；
/// - 五个键位名**一个都没出现** ⇒ `None`（**不是鼓机**：只写音色旋钮的设备与
///   没有设备链逐位同解，引擎 `drums.rs` 模块文档 §6 —— 诊断不得把普通复音合成器报成坏鼓机）；
/// - 至少一个键位名出现、且至少一个槽位缺失或取值非有限 ⇒ `Some(缺的键位名)`，
///   按槽位顺序、逐字用 [`DRUM_NOTE_NAMES`] 的规范拼写；
/// - 五个槽位全部写出且有限 ⇒ `None`（映射完整，识别走 `DrumsParams::from_devices`（引擎投影））。
///
/// 同名参数**后者胜**、名字大小写不敏感，与识别同一口径。`bypassed` **不在**
/// 本函数的判别内：识别侧跳过旁通设备是"取哪台作音源"的裁决，而本诊断回答的是
/// "这份参数本身是否写全" —— 旁通的不完整映射在取消旁通的那一刻就是静音陷阱，
/// 调用方（写入闸门 / 渲染预警）按各自场景决定是否只看非旁通设备。
#[must_use]
pub fn incomplete_drum_note_map(device: &DeviceDefinition) -> Option<Vec<&'static str>> {
    if device.kind != DeviceKind::InternalInstrument {
        return None;
    }
    let mut seen_any = false;
    let mut notes: [Option<u8>; NOTE_SLOTS] = [None; NOTE_SLOTS];
    for param in &device.params {
        let name = param.name.to_ascii_lowercase();
        if let Some(index) = note_slot(&name) {
            seen_any = true;
            notes[index] = note_from_f32(param.value);
        }
    }
    if !seen_any {
        return None;
    }
    let missing: Vec<&'static str> = DRUM_NOTE_NAMES
        .iter()
        .enumerate()
        .filter(|(index, _)| notes[*index].is_none())
        .map(|(_, name)| *name)
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(missing)
    }
}

/// 五个键位在投影中间态里的下标（顺序 = `DrumVoice::ALL`）。
pub const NOTE_SLOTS: usize = 5;

/// 参数名 → 键位下标（`None` = 不是键位名）。
#[must_use]
pub fn note_slot(name: &str) -> Option<usize> {
    match name {
        "kick_note" => Some(0),
        "snare_note" => Some(1),
        "closed_hat_note" => Some(2),
        "open_hat_note" => Some(3),
        "clap_note" => Some(4),
        _ => None,
    }
}

/// 可选键位槽位数（2026-11-18 八件；逐件＋侧链期 4 加 808 低音为 9）。
pub const EXTRA_NOTE_SLOTS: usize = 9;

/// 可选八件的**规范拼写**（下标 = 可选键位槽位）。
///
/// 与五键的 [`DRUM_NOTE_NAMES`] 分开：五键是识别的**必要条件**，
/// 八件是可选扩展——诊断 `incomplete_drum_note_map` 只点名五键，
/// 缺可选键位不是"不完整"。
pub const DRUM_EXTRA_NOTE_NAMES: [&str; EXTRA_NOTE_SLOTS] = [
    "rim_note",
    "tom_low_note",
    "tom_mid_note",
    "tom_high_note",
    "crash_note",
    "ride_note",
    "cowbell_note",
    "shaker_note",
    "bass808_note",
];

/// 参数名 → **可选**键位下标（`None` = 不是可选键位名）。
///
/// ⛔ 独立于 [`note_slot`]：仓内判据按源码读 `note_slot` 的字符串
/// 字面量并断言恰为五名（"鼓机识别名单"）；可选八件的名单住在
/// 这里，扩容不惊动那条判据。
#[must_use]
pub fn extra_note_slot(name: &str) -> Option<usize> {
    match name {
        "rim_note" => Some(0),
        "tom_low_note" => Some(1),
        "tom_mid_note" => Some(2),
        "tom_high_note" => Some(3),
        "crash_note" => Some(4),
        "ride_note" => Some(5),
        "cowbell_note" => Some(6),
        "shaker_note" => Some(7),
        "bass808_note" => Some(8),
        _ => None,
    }
}

/// 键位取值：非有限 ⇒ `None`（规则 5：整台设备不识别）；否则四舍五入并钳到 `0..=127`。
#[must_use]
pub fn note_from_f32(value: f32) -> Option<u8> {
    if !value.is_finite() {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let note = value.round().clamp(0.0, 127.0) as u8;
    Some(note)
}

/// 开关量 `f32 → bool`：非有限 ⇒ `false`；`>= 0.5` ⇒ `true`。
///
/// 与 [`KickStyle::from_f32`] 同一款阈值（`0.5`），理由相同：设备参数只有
/// `f32` 一个通道，开关只能用阈值表达；`NaN` 落到"关"（保守：不静音
/// 也不独奏，声音不断）。
#[must_use]
fn bool_from_f32(value: f32) -> bool {
    value.is_finite() && value >= 0.5
}

/// `clap_bursts` 的 `f32 → u32`：非有限 ⇒ 器件默认值（规则 5）。
///
/// 合法域（`1..=8`）由 [`ClapParams::sanitised`] 在收尾时钳制 —— 这里只做类型转换，
/// 因此**不**在这里重复一份域。
#[must_use]
fn bursts_from_f32(value: f32) -> u32 {
    if !value.is_finite() {
        return ClapParams::DEFAULT.bursts;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bursts = value.round().clamp(0.0, f32::from(u16::MAX)) as u32;
    bursts
}

/// 这台设备里**引擎不认**的鼓机参数名（保持设备 `params` 的原拼写、按出现顺序）。
///
/// 与 [`DrumKitParams::from_device`] 共用**同一份**识别表（键位 `note_slot` / 可选键位
/// `extra_note_slot` / 旋钮 `apply_kit_knob`）⇒ 不可能与渲染侧漂移；工厂预设的每个参数
/// 都会命中，返回空表。MCP 用它把"拼错的音色参数"响亮报出来（T-FIX-746）。
#[must_use]
pub fn unknown_kit_param_names(device: &DeviceDefinition) -> Vec<String> {
    let mut kit = DrumKitParams::DEFAULT;
    let mut unknown = Vec::new();
    for param in &device.params {
        let name = param.name.to_ascii_lowercase();
        if note_slot(&name).is_some() || extra_note_slot(&name).is_some() {
            continue;
        }
        if apply_kit_knob(&mut kit, &name, param.value) {
            continue;
        }
        unknown.push(param.name.clone());
    }
    unknown
}

/// 把一条**音色旋钮**写进投影；返回它是否被认下（名字已转小写）。
///
/// 先赋原值、由 [`DrumKitParams::sanitised`] 在收尾时统一钳制：
/// 这样"值域"只有**一个**事实源（器件自己），本模块不持有第二份域表。
pub fn apply_kit_knob(kit: &mut DrumKitParams, name: &str, value: f32) -> bool {
    match name {
        // --- 底鼓 ---
        "kick_tune_hz" => kit.kick.tune_hz = value,
        "kick_glide_hz" => kit.kick.glide_hz = value,
        "kick_glide_s" => kit.kick.glide_s = value,
        "kick_attack_s" => kit.kick.attack_s = value,
        "kick_decay_s" => kit.kick.decay_s = value,
        "kick_level" => kit.kick.level = value,
        "kick_pan" => kit.kick.pan = value,
        // --- 底鼓声部选择与 Anatomy 参数 ---
        "kick_style" => kit.kick_style = KickStyle::from_f32(value),
        "anatomy_softness" => kit.anatomy.softness = value,
        "anatomy_click_amount" => kit.anatomy.click_amount = value,
        "anatomy_tame_highs" => kit.anatomy.tame_highs = value,
        "anatomy_grit" => kit.anatomy.grit = value,
        "anatomy_boom_to_where" => kit.anatomy.boom_to_where = value,
        "anatomy_hit_skin" => kit.anatomy.hit_skin = value,
        "anatomy_rumble" => kit.anatomy.rumble = value,
        "anatomy_base_pitch_hz" => kit.anatomy.base_pitch_hz = value,
        "anatomy_volume" => kit.anatomy.volume = value,
        "anatomy_sub_mute" => kit.anatomy.sub_mute = bool_from_f32(value),
        "anatomy_sub_solo" => kit.anatomy.sub_solo = bool_from_f32(value),
        "anatomy_thump_mute" => kit.anatomy.thump_mute = bool_from_f32(value),
        "anatomy_thump_solo" => kit.anatomy.thump_solo = bool_from_f32(value),
        "anatomy_click_mute" => kit.anatomy.click_mute = bool_from_f32(value),
        "anatomy_click_solo" => kit.anatomy.click_solo = bool_from_f32(value),
        // --- 军鼓 ---
        "snare_tone_hz" => kit.snare.tone_hz = value,
        "snare_tone_ratio" => kit.snare.tone_ratio = value,
        "snare_tone_decay_s" => kit.snare.tone_decay_s = value,
        "snare_tone_level" => kit.snare.tone_level = value,
        "snare_noise_highpass_hz" => kit.snare.noise_highpass_hz = value,
        "snare_noise_lowpass_hz" => kit.snare.noise_lowpass_hz = value,
        "snare_noise_decay_s" => kit.snare.noise_decay_s = value,
        "snare_noise_level" => kit.snare.noise_level = value,
        "snare_attack_s" => kit.snare.attack_s = value,
        "snare_level" => kit.snare.level = value,
        "snare_pan" => kit.snare.pan = value,
        // --- 踩镲（闭/开共用一套配方）---
        "hihat_base_hz" => kit.hihat.base_hz = value,
        "hihat_highpass_hz" => kit.hihat.highpass_hz = value,
        "hihat_lowpass_hz" => kit.hihat.lowpass_hz = value,
        "hihat_attack_s" => kit.hihat.attack_s = value,
        "hihat_closed_decay_s" => kit.hihat.closed_decay_s = value,
        "hihat_open_decay_s" => kit.hihat.open_decay_s = value,
        "hihat_level" => kit.hihat.level = value,
        "hihat_pan" => kit.hihat.pan = value,
        // --- 拍手 ---
        "clap_highpass_hz" => kit.clap.highpass_hz = value,
        "clap_lowpass_hz" => kit.clap.lowpass_hz = value,
        "clap_bursts" => kit.clap.bursts = bursts_from_f32(value),
        "clap_burst_decay_s" => kit.clap.burst_decay_s = value,
        "clap_burst_spacing_s" => kit.clap.burst_spacing_s = value,
        "clap_attack_s" => kit.clap.attack_s = value,
        "clap_decay_s" => kit.clap.decay_s = value,
        "clap_level" => kit.clap.level = value,
        "clap_pan" => kit.clap.pan = value,
        // --- 桶鼓（低/中/高共用一套配方，音高比例在 dsp 侧）---
        "tom_tune_hz" => kit.tom.tune_hz = value,
        "tom_glide_hz" => kit.tom.glide_hz = value,
        "tom_glide_s" => kit.tom.glide_s = value,
        "tom_attack_s" => kit.tom.attack_s = value,
        "tom_decay_s" => kit.tom.decay_s = value,
        "tom_level" => kit.tom.level = value,
        "tom_pan" => kit.tom.pan = value,
        // --- 边击 ---
        "rim_tone_hz" => kit.rim.tone_hz = value,
        "rim_noise_highpass_hz" => kit.rim.noise_highpass_hz = value,
        "rim_noise_lowpass_hz" => kit.rim.noise_lowpass_hz = value,
        "rim_decay_s" => kit.rim.decay_s = value,
        "rim_attack_s" => kit.rim.attack_s = value,
        "rim_level" => kit.rim.level = value,
        "rim_pan" => kit.rim.pan = value,
        // --- 沙锤 ---
        "shaker_highpass_hz" => kit.shaker.highpass_hz = value,
        "shaker_lowpass_hz" => kit.shaker.lowpass_hz = value,
        "shaker_attack_s" => kit.shaker.attack_s = value,
        "shaker_decay_s" => kit.shaker.decay_s = value,
        "shaker_level" => kit.shaker.level = value,
        "shaker_pan" => kit.shaker.pan = value,
        // --- 牛铃 ---
        "cowbell_freq_a_hz" => kit.cowbell.freq_a_hz = value,
        "cowbell_freq_b_hz" => kit.cowbell.freq_b_hz = value,
        "cowbell_highpass_hz" => kit.cowbell.highpass_hz = value,
        "cowbell_lowpass_hz" => kit.cowbell.lowpass_hz = value,
        "cowbell_attack_s" => kit.cowbell.attack_s = value,
        "cowbell_decay_s" => kit.cowbell.decay_s = value,
        "cowbell_level" => kit.cowbell.level = value,
        "cowbell_pan" => kit.cowbell.pan = value,
        // --- 碎镲 ---
        "crash_base_hz" => kit.crash.base_hz = value,
        "crash_highpass_hz" => kit.crash.highpass_hz = value,
        "crash_lowpass_hz" => kit.crash.lowpass_hz = value,
        "crash_attack_s" => kit.crash.attack_s = value,
        "crash_decay_s" => kit.crash.decay_s = value,
        "crash_level" => kit.crash.level = value,
        "crash_pan" => kit.crash.pan = value,
        // --- 叮叮镲 ---
        "ride_base_hz" => kit.ride.base_hz = value,
        "ride_highpass_hz" => kit.ride.highpass_hz = value,
        "ride_lowpass_hz" => kit.ride.lowpass_hz = value,
        "ride_attack_s" => kit.ride.attack_s = value,
        "ride_decay_s" => kit.ride.decay_s = value,
        "ride_level" => kit.ride.level = value,
        "ride_pan" => kit.ride.pan = value,
        // --- 独立 808 低音（逐件＋侧链期 4） ---
        "bass808_tune_hz" => kit.bass808.tune_hz = value,
        "bass808_glide_hz" => kit.bass808.glide_hz = value,
        "bass808_glide_s" => kit.bass808.glide_s = value,
        "bass808_attack_s" => kit.bass808.attack_s = value,
        "bass808_decay_s" => kit.bass808.decay_s = value,
        "bass808_level" => kit.bass808.level = value,
        "bass808_drive" => kit.bass808.drive = value,
        "bass808_pan" => kit.bass808.pan = value,
        // --- 总线 ---
        "master_level" => kit.master_level = value,
        _ => return false,
    }
    true
}

/// ⭐ G5（审计 2026-10-27）：给定参数名（小写）是否在鼓机识别表里？
///
/// 复用 [`note_slot`] 与 [`apply_kit_knob`] 这两处**已有**的认名逻辑做探针
/// （在 `DEFAULT` 的副本上试写一个无害值），因此**不是**第二张名字表 ——
/// `from_device` 消费什么，这里就承认什么。
#[must_use]
pub fn consumes_drum_param(name_lower: &str) -> bool {
    if note_slot(name_lower).is_some() || extra_note_slot(name_lower).is_some() {
        return true;
    }
    let mut probe = DrumKitParams::DEFAULT;
    apply_kit_knob(&mut probe, name_lower, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::project::{DeviceDefinition, DeviceKind};
    use crate::{EntityId, ParameterValue};

    fn device(kind: DeviceKind, bypassed: bool, params: &[(&str, f32)]) -> DeviceDefinition {
        DeviceDefinition {
            id: EntityId::new(),
            name: "Device".to_owned(),
            kind,
            bypassed,
            params: params
                .iter()
                .map(|(name, value)| ParameterValue {
                    name: (*name).to_owned(),
                    value: *value,
                    unit: None,
                })
                .collect(),
            latency_samples: 0,
            plugin: None,
        }
    }

    /// 完整的五个键位（判据里反复用）。
    const NOTES: [(&str, f32); 5] = [
        ("kick_note", 36.0),
        ("snare_note", 38.0),
        ("closed_hat_note", 42.0),
        ("open_hat_note", 46.0),
        ("clap_note", 39.0),
    ];

    /// 把 [`NOTES`] 与额外参数拼成一台设备。
    fn kit_device(kind: DeviceKind, bypassed: bool, extra: &[(&str, f32)]) -> DeviceDefinition {
        let mut params = NOTES.to_vec();
        params.extend_from_slice(extra);
        device(kind, bypassed, &params)
    }

    /// T-FIX-746：未知旋钮名必须被点名，已知键位/旋钮不得误报。
    #[test]
    fn unknown_kit_params_are_reported_and_known_ones_are_not() {
        let device = DeviceDefinition {
            id: crate::EntityId::new(),
            name: "kit".to_owned(),
            kind: crate::DeviceKind::InternalInstrument,
            bypassed: false,
            params: [
                ("kick_note", 36.0),
                ("kick_tune_hz", 45.0),
                ("kick_tun_hz", 50.0),
            ]
            .into_iter()
            .map(|(name, value)| crate::ParameterValue {
                name: name.to_owned(),
                value,
                unit: None,
            })
            .collect(),
            latency_samples: 0,
            plugin: None,
        };
        assert_eq!(
            unknown_kit_param_names(&device),
            vec!["kick_tun_hz".to_owned()]
        );
    }

    /// **诊断点名**（P2-3）：逐个缺键各点名该键 —— 写四个缺一个时，
    /// [`incomplete_drum_note_map`] 必须恰好返回缺的那个键位名。
    #[test]
    fn incomplete_drum_note_map_names_each_missing_key() {
        for (missing_index, (missing_name, _)) in NOTES.iter().enumerate() {
            let params: Vec<(&str, f32)> = NOTES
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != missing_index)
                .map(|(_, pair)| *pair)
                .collect();
            let found =
                incomplete_drum_note_map(&device(DeviceKind::InternalInstrument, false, &params))
                    .expect("写了四个缺一个必须被诊断点名");
            assert_eq!(
                found,
                vec![*missing_name],
                "缺哪个键就点名哪个键（本次缺 `{missing_name}`）"
            );
        }
    }

    /// **诊断的阴性面**：完整映射、非鼓机设备、种类不对 ⇒ 一律 `None`。
    #[test]
    fn incomplete_drum_note_map_is_none_for_complete_and_non_drum_devices() {
        // 五键齐全 ⇒ 完整，不是诊断对象。
        assert!(
            incomplete_drum_note_map(&kit_device(DeviceKind::InternalInstrument, false, &[]))
                .is_none(),
            "五键齐全不得出诊断"
        );
        // 一个键位都没写（只有音色旋钮）⇒ 不是鼓机（模块文档 §6 的逐位不变面）。
        assert!(
            incomplete_drum_note_map(&device(
                DeviceKind::InternalInstrument,
                false,
                &[("kick_tune_hz", 45.0), ("snare_tone_hz", 200.0)],
            ))
            .is_none(),
            "只写音色旋钮不是坏鼓机"
        );
        // 空设备 ⇒ 不是鼓机。
        assert!(
            incomplete_drum_note_map(&device(DeviceKind::InternalInstrument, false, &[])).is_none()
        );
        // 键位写全但种类是效果器 ⇒ 与鼓机无关。
        assert!(
            incomplete_drum_note_map(&kit_device(DeviceKind::InternalEffect, false, &[])).is_none()
        );
        // 键位名大小写不敏感（与识别同一口径）：全大写写四个、缺 `clap_note`。
        assert_eq!(
            incomplete_drum_note_map(&device(
                DeviceKind::InternalInstrument,
                false,
                &[
                    ("KICK_NOTE", 36.0),
                    ("SNARE_NOTE", 38.0),
                    ("CLOSED_HAT_NOTE", 42.0),
                    ("OPEN_HAT_NOTE", 46.0),
                ],
            )),
            Some(vec!["clap_note"])
        );
    }

    /// 非有限键位 = 缺键（与规则 5 同一口径）：`clap_note = NaN` 时点名 `clap_note`。
    #[test]
    fn incomplete_drum_note_map_treats_a_non_finite_key_as_missing() {
        let mut params = NOTES.to_vec();
        params[4].1 = f32::NAN;
        assert_eq!(
            incomplete_drum_note_map(&device(DeviceKind::InternalInstrument, false, &params)),
            Some(vec!["clap_note"])
        );
    }

    /// 两份名单一致性：[`DRUM_NOTE_NAMES`] 的每个名字经 `note_slot` 必须落回自己的下标。
    #[test]
    fn note_names_table_matches_note_slot() {
        for (index, name) in DRUM_NOTE_NAMES.iter().enumerate() {
            assert_eq!(note_slot(name), Some(index), "`{name}` 的槽位必须自洽");
        }
    }

    /// 判据：`clap_bursts` 的 `f32 → u32` **上界是 u16 全域**，不是 u8。
    ///
    /// `bursts_from_f32` 的文档口径是"非有限 ⇒ 器件默认值；否则四舍五入并钳到
    /// `0..=u16::MAX`"（`drums.rs` 的规则 5）。上界若被写成 `u8::MAX`，那么
    /// 256…65535 这一段配置会被**静默**压成 255 —— 声音于是由"配置"变成了"猜"。
    /// 下界同样要钉住：负值必须落到 `0`（不是回绕成巨大的 u32）。
    ///
    /// **量什么**：`bursts_from_f32(v)` 的返回值（单位：次连击）。
    ///
    /// 注入实测：`clamp(0.0, f32::from(u16::MAX))` → `clamp(0.0, f32::from(u8::MAX))`
    /// （`drums.rs`）⇒ 本判据实测变红（`256` 读到 `255`）。
    #[test]
    fn clap_bursts_keep_the_whole_u16_range() {
        assert_eq!(bursts_from_f32(0.0), 0, "下界");
        assert_eq!(bursts_from_f32(-3.0), 0, "负值必须落到下界（不得回绕）");
        assert_eq!(bursts_from_f32(255.0), 255);
        assert_eq!(bursts_from_f32(256.0), 256, "u8 上界之上必须保留");
        assert_eq!(bursts_from_f32(1_000.0), 1_000);
        assert_eq!(
            bursts_from_f32(f32::from(u16::MAX)),
            u32::from(u16::MAX),
            "u16 上界必须保留"
        );
        assert_eq!(
            bursts_from_f32(1.0e9),
            u32::from(u16::MAX),
            "只有超过 u16 上界才钳住"
        );
    }
}
