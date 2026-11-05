//! Anatomy 底鼓：Groove `AnatomyKickEngine` 的三层体感分解合成移植。
//!
//! 准绳是 `groove/src/audio/AnatomyKickEngine.ts` 的
//! [`synthesizeAnatomyKickVoice`](https://github.com/gradetwo/groove/blob/main/src/audio/AnatomyKickEngine.ts)
//!（该文件第 813 行起，含 Q8 速度-音色映射）。类版
//! `AnatomyKickEngine::trigger` 是旧的 UI 预览路径，**不是**准绳：它没有
//! 速度-音色映射、thump 层带失真器、总线失真量是 `grit`（准绳是
//! `effectiveGrit * 0.4`）、过采样 `2x`（准绳是 `4x`）；`exportWav`
//! 是简化版，也不是准绳。
//!
//! ## 1. 三层算法（准绳原文逐项，行号指 `AnatomyKickEngine.ts`）
//!
//! | 层 | 振荡器 | 滤波 | 包络（`t` = 触发后秒数） | 失真 |
//! | :--- | :--- | :--- | :--- | :--- |
//! | Sub | 正弦，`start = base·(1 + 2.2·(1 − soft))` 指数滑到 `base`，耗时 `0.025 + soft·0.04` | 低通 140 Hz（Q 缺省 1） | `0.0001 →` 线性到峰值（+2 ms）`→` 指数到峰值 `·0.45`（+80 ms）`→` 指数到 `0.0001`（+`subDecay`）；停在 `t + subDecay + 0.05` | 无 |
//! | Rumble（`rumble > 0.2` 时才有） | 正弦，`base·0.75`，无滑频 | 无 | `0.0001 →` 线性到幅值（+40 ms）`→` 指数到 `0.0001`（+`rumbleDecay = subDecay·1.1`）；停在 `t + rumbleDecay + 0.05` | 无 |
//! | Thump | 三角波，`380·(1 − soft·0.3)` 指数滑到 `110`，22 ms | 带通 160 Hz、Q 1.8 | `0.0001 →` 线性到峰值（+1 ms）`→` 指数到 `0.0001`（+`thumpDecay`）；停在 `t + thumpDecay + 0.05` | 无（失真器只在类版里，准绳没有） |
//! | Click（`clickAmount > 0.01` 时才有） | 方波，1800 Hz 指数滑到 750 Hz，8 ms | 带通中心 `1600 + (1 − tameHighs)·1200`、Q 4.5 | `0.0001 →` 线性到峰值（+0.3 ms）`→` 指数到 `0.0001`（+`clickDecay`）；停在 `t + clickDecay + 0.02` | 无 |
//!
//! 峰值（`vel` 是这一击的速度，`volume` 是主增益；人化恒等时
//! `levelScale = pitchRatio = decayScale_人化 = 1`）：
//!
//! | 层 | 峰值 |
//! | :--- | :--- |
//! | Sub | `vel · 1.25 · volume` |
//! | Rumble | `vel · rumble · 0.4 · volume` |
//! | Thump | `vel · (0.8 + hitSkin · 0.5) · volume` |
//! | Click | `vel · clickAmount · 0.95 · volume` |
//!
//! 衰减：`subDecay = (0.2 + boomToWhere · 0.9)`；
//! `thumpDecay = (0.05 + hitSkin · 0.18)`；
//! `clickDecay = (0.008 + (1 − effectiveSoftness) · 0.007) · decayScale`。
//! 四层两两汇入（`sumLower` ← sub + rumble、`sumUpper` ← thump + click，
//! 单位增益）再进总线。
//!
//! ## 2. 速度-音色映射（Q8，准绳第 860 行起）
//!
//! `velNorm = clamp(vel, 0, 1)`；
//! `gripScale = 0.55 + 0.45 · velNorm^0.6`（ppp 0.55 → ff 1.0）；
//! `decayScale = 1 + 0.25 · (1 − velNorm)`（只进 click 衰减）；
//! `effectiveGrit = grit · gripScale`；
//! `effectiveSoftness = min(0.95, softness / gripScale)`。
//! 总线失真器：`effectiveGrit > 0.05` 时才接，量
//! `effectiveGrit · 0.4`，曲线 `y = tanh(k·x) / tanh(k)`、
//! `k = max(0.01, amount · 25)`（`makeDistortionCurve`，第 288 行起），
//! 过采样 `4x`。
//!
//! ## 3. 独奏 / 静音（准绳第 883 行起，触发时快照）
//!
//! `anySolo = subSolo ‖ thumpSolo ‖ clickSolo`；
//! `playX = !muteX && (!anySolo ‖ soloX)`。Rumble 没有自己的开关，
//! 随 sub 层走（它没有滤波、直接进 `sumLower`）。
//!
//! ## 4. 人化钉在恒等
//!
//! 准绳用 `noise.ts` 的 `hitVariation(noisePosition)` 给每击加
//! `±` 音分 / `±0.28 dB` 电平 / 衰减变化；`position = 0`（或缺省）时
//! 三项恒等（`pitchRatio = levelScale = decayScale = 1`）。Yeban 鼓机是
//! 确定性器件 [ARCH-DET-001]：同一串触发必须逐位复现，移植钉在
//! `position 0`，不引入随机人化。
//!
//! ## 5. 与准绳的偏离（逐条编号，判据里对账）
//!
//! - **D1 电平**：准绳的 `vel` 同时进峰值与音色映射；Yeban 的电平由
//!   `DrumMachine` 的 `slot.gain` 在 `render` 末尾一次性给出，因此移植
//!   的峰值公式里 `vel` 取 1、音色映射仍用 `velNorm = gain`。两者乘积与
//!   准绳逐项相同；`gain = 0` 时 Yeban 是真静音，准绳是 `safeVelocity`
//!   的 `1e-4` 底噪（−80 dBFS，`dspGuards.ts` 的防崩 guards）。
//! - **D2 总线失真器无过采样**：Web Audio 的 `oversample` 只降混叠、
//!   不改传递函数；移植按 `y = tanh(k·x) / tanh(k)` 逐样本算，混叠产物
//!   见 `tests/anatomy_kick_render.rs` 的频谱量测。Thump 层在准绳里本来
//!   就没有失真器（只在类版里有），移植不加。
//! - **D3 停止时间**：准绳的 `stopTime` 取
//!   `max(subDecay, thumpDecay, 0.4) + 0.05`，未含 rumble
//!   （`subDecay × 1.1`）；移植的 `is_finished` 取四层实际停振的最大值，
//!   否则 rumble 尾巴会被槽位回收截断。
//! - **D4 `safeGain` 的 `1e-4` 下限不移植**：那是 Web Audio 指数斜坡的
//!   防崩 guards，不是音乐意图；Yeban 里 `volume = 0` 就是静音，与经典
//!   声部口径一致。
//! - **D5 朴素波形**：三角波与方波是朴素波形（与踩镲的 6 条方波同款做法）；
//!   click 方波经 Q 4.5 带通，通带外的混叠产物被带通压掉，通带内的谐波
//!   是真实谐波（非混叠），见判据。
//! - **D6 直流阻隔**：准绳的 voice fn 路径里没有 DC blocker（只在类版
//!   master 链里有）；四层信号本身无直流分量（正弦 / 三角 / 方波经带通 /
//!   低通），移植不加。
//! - **D7 汇入顺序**：准绳用 `sumLower` / `sumUpper` 两两汇入（Chrome 逐渲染
//!   哈希不稳定的实测对策）；Rust 里浮点加法顺序是确定的，移植直接按
//!   sub、rumble、thump、click 的顺序相加 —— 与准绳在实数意义下相同，
//!   浮点舍入只差末位。
//! - **D8 解析包络**：准绳的 `linearRamp` / `exponentialRamp` 本身就是解析
//!   插值，移植按同一公式逐样本算（`t` 是触发后秒数），因此与块切分无关；
//!   代价是逐样本路径上多了 `expf`（包络）与 `tanhf`（总线失真器），
//!   见 `voice.rs` §2 的注记 —— 跨平台一致性口径与既有的 `sinf` 相同。
//!
//! ## 6. 预设
//!
//! 前 6 套是 Groove `KICK_PRESETS`（第 122 行起）**原样**搬：id、名称、分类、
//! 参数全保留，philosophy 中英文随预设带过来作说明。后 3 套是本移植新增的
//! 摇滚 / 金属取向预设（理由见各条目；与原 6 套的重合关系在
//! `ANATOMY_KICK_PRESETS` 的条目注释里逐套写明）。
//!
//! ## 7. 接线
//!
//! `DrumKitParams::kick_style` 选底鼓声部（`Classic` = 既有的 808 式下滑正弦，
//! `Anatomy` = 本模块）；`DrumKitParams::anatomy` 是本模块的 9 个连续参数 +
//! 6 个独奏 / 静音开关。引擎侧（`yeban-engine/src/drums.rs`）用
//! `kick_style` / `anatomy_*` 两组旋钮名投影。MCP 侧**没有**鼓机参数工具
//! （`yeban-mcp` 里的 `drum*` 键全是段落生成的鼓网格参数），按任务口径不
//! 新建工具，只留引擎层接口。

use crate::envelope::{Adsr, STEAL_RELEASE_SECONDS};
use crate::math::sanitise_sample_rate;
use crate::shaping::Biquad;

use super::voice::{increment_for, sine_from_phase};

/// Sub 低通拐点（Hz，准绳 `subFilter.frequency = 140`）。
const SUB_LOWPASS_HZ: f32 = 140.0;
/// Sub 低通 Q（Web Audio `BiquadFilterNode` 的缺省值；准绳未设置）。
const SUB_LOWPASS_Q: f32 = 1.0;
/// Thump 带通中心（Hz）与 Q（准绳 `thumpFilter`）。
const THUMP_BAND_HZ: f32 = 160.0;
const THUMP_BAND_Q: f32 = 1.8;
/// Click 带通 Q（准绳 `clickFilter.Q = 4.5`）。
const CLICK_BAND_Q: f32 = 4.5;
/// 指数斜坡的底（准绳全部四层包络的起点与终点，`0.0001` = −80 dBFS）。
const ENVELOPE_FLOOR: f32 = 0.0001;
/// Sub 包络的中段拐点：`0.0001 → 峰值（+2 ms）→ 峰值·0.45（+80 ms）`。
const SUB_KNEE_S: f32 = 0.08;
const SUB_KNEE_RATIO: f32 = 0.45;
/// 各层线性起振时间（秒）：sub 2 ms、rumble 40 ms、thump 1 ms、click 0.3 ms。
const SUB_ATTACK_S: f32 = 0.002;
const RUMBLE_ATTACK_S: f32 = 0.04;
const THUMP_ATTACK_S: f32 = 0.001;
const CLICK_ATTACK_S: f32 = 0.0003;
/// 各层停振的拖尾（秒）：sub / rumble / thump 50 ms、click 20 ms。
const LONG_STOP_TAIL_S: f32 = 0.05;
const CLICK_STOP_TAIL_S: f32 = 0.02;

/// 底鼓声部的选择。缺省是 [`KickStyle::Classic`]：已有工程与冻结交付物
/// 不动声色地保持经典声部。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KickStyle {
    /// 既有的 808 式单振荡器音高下滑（[`super::KickParams`]）。
    #[default]
    Classic,
    /// Anatomy 三层体感分解（本模块）。
    Anatomy,
}

impl KickStyle {
    /// `f32` 旋钮值 → 声部：`>= 0.5`（且有限）为 Anatomy，其余为 Classic
    ///（含 `NaN`：缺省行为是既有的经典声部）。
    #[must_use]
    pub const fn from_f32(value: f32) -> Self {
        if value.is_finite() && value >= 0.5 {
            Self::Anatomy
        } else {
            Self::Classic
        }
    }
}

/// Anatomy 底鼓的基频合法域下界（Hz，准绳 `SomaticKickParams` 的注释）。
pub const ANATOMY_BASE_PITCH_MIN_HZ: f32 = 32.0;
/// Anatomy 底鼓的基频合法域上界（Hz）。
pub const ANATOMY_BASE_PITCH_MAX_HZ: f32 = 65.0;
/// Anatomy 底鼓的主增益合法域上界（准绳 `volume` 的注释：`0` 到 `1.5`）。
pub const ANATOMY_VOLUME_MAX: f32 = 1.5;

/// Anatomy 底鼓参数（准绳 `SomaticKickParams`，字段名与量纲一一对齐）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnatomyKickParams {
    /// 柔软度（`0..=1`）：`0` = 剃刀般锋利的音高下滑，`1` = 丝绒般柔和的曲线。
    pub softness: f32,
    /// Click 量（`0..=1`）：`0` = 静默，`1` = 极度强烈的神经尖刺。
    pub click_amount: f32,
    /// 高频驯服（`0..=1`）：`0` = 明亮 / 原始，`1` = 变暗 / 收敛。
    pub tame_highs: f32,
    /// 失真度（`0..=1`）：`0` = 干净的纯正弦 / 三角，`1` = 过载失真。
    pub grit: f32,
    /// 低频延展（`0..=1`）：`0` = 极紧的 punch，`1` = 地下无尽的隆隆声。
    pub boom_to_where: f32,
    /// 击皮厚度（`0..=1`）：`0` = 空洞，`1` = 厚实的肌肉声学质量。
    pub hit_skin: f32,
    /// 次谐波隆隆（`0..=1`）：`0` = 静态，`1` = 次谐波二次振荡。
    /// `<= 0.2` 时 rumble 层不存在（准绳 `if (p.rumble > 0.2)`）。
    pub rumble: f32,
    /// 基频（Hz，`32..=65`，默认 `48`）。
    pub base_pitch_hz: f32,
    /// 主增益（`0..=1.5`）。
    pub volume: f32,
    /// Sub 层静音。
    pub sub_mute: bool,
    /// Sub 层独奏。
    pub sub_solo: bool,
    /// Thump 层静音。
    pub thump_mute: bool,
    /// Thump 层独奏。
    pub thump_solo: bool,
    /// Click 层静音。
    pub click_mute: bool,
    /// Click 层独奏。
    pub click_solo: bool,
}

impl AnatomyKickParams {
    /// Groove 的出厂默认（`DEFAULT_SOMATIC_PARAMS` 原样）。
    pub const DEFAULT: Self = Self {
        softness: 0.35,
        click_amount: 0.65,
        tame_highs: 0.3,
        grit: 0.25,
        boom_to_where: 0.55,
        hit_skin: 0.6,
        rumble: 0.4,
        base_pitch_hz: 48.0,
        volume: 1.0,
        sub_mute: false,
        sub_solo: false,
        thump_mute: false,
        thump_solo: false,
        click_mute: false,
        click_solo: false,
    };

    /// 九个连续参数 → 一份预设参数（六个独奏 / 静音开关恒为 `false`：
    /// 准绳的预设表只登记连续参数，开关是演奏时的实时状态）。
    /// 参数顺序 = 准绳 `SomaticKickParams` 的字段顺序。9 个形参是故意的：
    /// 这 9 个数就是预设在 Groove 里的一行，写成 9 元组是为了让预设表
    /// 逐行对照源文件，不拆成结构体。
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn from_nine(
        softness: f32,
        click_amount: f32,
        tame_highs: f32,
        grit: f32,
        boom_to_where: f32,
        hit_skin: f32,
        rumble: f32,
        base_pitch_hz: f32,
        volume: f32,
    ) -> Self {
        Self {
            softness,
            click_amount,
            tame_highs,
            grit,
            boom_to_where,
            hit_skin,
            rumble,
            base_pitch_hz,
            volume,
            sub_mute: false,
            sub_solo: false,
            thump_mute: false,
            thump_solo: false,
            click_mute: false,
            click_solo: false,
        }
    }

    /// 把 9 个连续参数钳到合法域后的副本。**全定义**：任何输入（含 `NaN`/`∞`）
    /// 都有返回；非有限值一律归**下界**（口径与 [`super::KickParams::sanitised`]
    /// 相同：下界在每个字段上都是保守的；有限越界值则钳到最近的界）。
    ///
    /// | 字段 | 合法域 | 非有限 |
    /// | :--- | :--- | :--- |
    /// | `softness` / `click_amount` / `tame_highs` / `grit` / `boom_to_where` / `hit_skin` / `rumble` | `[0, 1]` | **`0`** |
    /// | `base_pitch_hz` | `[32, 65]` | **`32`** |
    /// | `volume` | `[0, 1.5]` | **`0`** |
    #[must_use]
    pub fn sanitised(self) -> Self {
        Self {
            softness: clamp_unit(self.softness),
            click_amount: clamp_unit(self.click_amount),
            tame_highs: clamp_unit(self.tame_highs),
            grit: clamp_unit(self.grit),
            boom_to_where: clamp_unit(self.boom_to_where),
            hit_skin: clamp_unit(self.hit_skin),
            rumble: clamp_unit(self.rumble),
            base_pitch_hz: if self.base_pitch_hz.is_finite() {
                self.base_pitch_hz
                    .clamp(ANATOMY_BASE_PITCH_MIN_HZ, ANATOMY_BASE_PITCH_MAX_HZ)
            } else {
                ANATOMY_BASE_PITCH_MIN_HZ
            },
            volume: if self.volume.is_finite() {
                self.volume.clamp(0.0, ANATOMY_VOLUME_MAX)
            } else {
                0.0
            },
            sub_mute: self.sub_mute,
            sub_solo: self.sub_solo,
            thump_mute: self.thump_mute,
            thump_solo: self.thump_solo,
            click_mute: self.click_mute,
            click_solo: self.click_solo,
        }
    }

    /// 按 id 取预设参数（`None` = 没有这个 id）。线性扫描 9 项常量表：
    /// 构造期调用，不在实时路径上。
    #[must_use]
    pub fn preset(id: &str) -> Option<AnatomyKickParams> {
        ANATOMY_KICK_PRESETS
            .iter()
            .find(|preset| preset.id == id)
            .map(|preset| preset.params)
    }
}

impl Default for AnatomyKickParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// `0..=1` 钳制：`NaN` ⇒ `0`（下界，保守）。
#[must_use]
fn clamp_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Anatomy 底鼓预设的分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnatomyKickCategory {
    /// 柏林奥菲斯巨柱（Groove 原预设）。
    Orphic,
    /// 底特律机械脉冲（Groove 原预设）。
    Club,
    /// 躯体 808 内脏引力 / Anatomy 默认（Groove 原预设）。
    Somatic,
    /// 工业阶级抵抗（Groove 原预设）。
    Industrial,
    /// 真皮鼓锤敲击（Groove 原预设）。
    Acoustic,
    /// 神经时钟微瞬态（Groove 原预设）。
    Neural,
    /// punchy 摇滚底鼓（本移植新增）。
    Rock,
    /// 紧实金属底鼓（本移植新增）。
    Metal,
}

impl AnatomyKickCategory {
    /// Groove 的分类名（新预设取同形小写名）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Orphic => "orphic",
            Self::Club => "club",
            Self::Somatic => "somatic",
            Self::Industrial => "industrial",
            Self::Acoustic => "acoustic",
            Self::Neural => "neural",
            Self::Rock => "rock",
            Self::Metal => "metal",
        }
    }
}

/// 一套 Anatomy 底鼓预设：身份 + 双语名 + 分类 + 双语 philosophy + 参数。
#[derive(Clone, Copy, Debug)]
pub struct AnatomyKickPreset {
    /// 预设 id（Groove 里是 `kick:<id>` 的后半段）。
    pub id: &'static str,
    /// 中文名。
    pub name_zh: &'static str,
    /// 英文名。
    pub name_en: &'static str,
    /// 分类。
    pub category: AnatomyKickCategory,
    /// philosophy（中文，Groove 原文照录；新预设为本移植所写）。
    pub philosophy_zh: &'static str,
    /// philosophy（英文，Groove 原文照录；新预设为本移植所写）。
    pub philosophy_en: &'static str,
    /// 参数（六个独奏 / 静音开关恒为 `false`，见
    /// [`AnatomyKickParams::from_nine`]）。
    pub params: AnatomyKickParams,
}

/// Anatomy 底鼓预设表：前 6 套是 Groove `KICK_PRESETS` **原样**搬
///（id、名称、分类、参数、philosophy 全保留），后 3 套是本移植新增的
///摇滚 / 金属取向预设。
pub const ANATOMY_KICK_PRESETS: [AnatomyKickPreset; 9] = [
    AnatomyKickPreset {
        id: "berlin-orphic",
        name_zh: "柏林奥菲斯巨柱",
        name_en: "Berlin Orphic Pillar",
        category: AnatomyKickCategory::Orphic,
        philosophy_zh: "受 Koçer《Dub Techno: The Orphic Experience of Sound》启发。深沉内向的次低频回响，空间化耳机内向声场，次低频如液态金属缓缓沉降。",
        philosophy_en: "Inspired by Koçer's 'Dub Techno: The Orphic Experience of Sound'. Subterranean, introspective cavernous resonance designed for inner-ear architecture.",
        params: AnatomyKickParams::from_nine(0.2, 0.4, 0.55, 0.35, 0.8, 0.5, 0.7, 42.0, 1.05),
    },
    AnatomyKickPreset {
        id: "detroit-mechanical",
        name_zh: "底特律机械脉冲",
        name_en: "Detroit Mechanical Pulse",
        category: AnatomyKickCategory::Club,
        philosophy_zh: "致敬 TR-909 与底特律极简主义。强劲的高频瞬态击皮，神经脑干高度锁相，定义不可撼动的机械时间秩序。",
        philosophy_en: "Tribute to TR-909 and Detroit minimalism. Crisp high-frequency beater slap, high PLV coherence, establishing strict chronological discipline.",
        params: AnatomyKickParams::from_nine(0.15, 0.85, 0.15, 0.4, 0.32, 0.8, 0.2, 52.0, 1.0),
    },
    AnatomyKickPreset {
        id: "somatic-808-gravity",
        name_zh: "躯体 808 内脏引力",
        name_en: "Visceral 808 Gravity",
        category: AnatomyKickCategory::Somatic,
        philosophy_zh: "36 Hz 极低基频纯正弦波。跳过耳蜗基底膜直接激发生理机械感受器，内脏与重力的深层共振。",
        philosophy_en: "Pure 36 Hz sub-bass sine wave. Bypasses the cochlea to activate visceral mechanoreceptors; gravity transformed into biological acoustic mass.",
        params: AnatomyKickParams::from_nine(0.6, 0.2, 0.8, 0.05, 0.92, 0.3, 0.85, 36.0, 1.15),
    },
    AnatomyKickPreset {
        id: "industrial-revolt",
        name_zh: "工业阶级抵抗",
        name_en: "Industrial Class Revolt",
        category: AnatomyKickCategory::Industrial,
        philosophy_zh: "反对商业预制菜的平庸抛光。非线性饱和过载与硬质削波，让音色承载阶级摩擦力与叛逆张力。",
        philosophy_en: "Refusal of the over-polished McPulse. Severe non-linear wavefolding and tanh saturation; timbre carrying socio-political friction.",
        params: AnatomyKickParams::from_nine(0.05, 0.9, 0.05, 0.92, 0.5, 0.95, 0.45, 46.0, 0.95),
    },
    AnatomyKickPreset {
        id: "acoustic-beater-skin",
        name_zh: "真皮鼓锤敲击",
        name_en: "Acoustic Beater Skin",
        category: AnatomyKickCategory::Acoustic,
        philosophy_zh: "还原 19 世纪军乐队大军鼓向现代爵士演进的物理击打感。饱满的木腔共振与羊毛鼓锤撞击皮膜的弹性质感。",
        philosophy_en: "Anatomy of the 19th-century marching drum evolved into jazz. Organic wooden shell resonance and elastic felt-beater contact with drumhead.",
        params: AnatomyKickParams::from_nine(0.45, 0.5, 0.45, 0.2, 0.4, 0.85, 0.15, 58.0, 1.0),
    },
    AnatomyKickPreset {
        id: "neural-click-clock",
        name_zh: "神经时钟微瞬态",
        name_en: "Neural Click Clock",
        category: AnatomyKickCategory::Neural,
        philosophy_zh: "极端微雕的高频脉冲，剥离大部分低频质量，只保留神经锁相所必需的数毫秒尖刺，使时间结晶化。",
        philosophy_en: "Micro-sculpted high-frequency transient. Strips excessive sub mass, leaving only the microsecond spike required for neural phase-locking.",
        params: AnatomyKickParams::from_nine(0.1, 0.98, 0.05, 0.1, 0.15, 0.2, 0.05, 50.0, 0.95),
    },
    // --- 以下 3 套是本移植新增的摇滚 / 金属取向预设 ---
    AnatomyKickPreset {
        id: "tight-metal-kick",
        name_zh: "紧实金属底鼓",
        name_en: "Tight Metal Kick",
        category: AnatomyKickCategory::Metal,
        philosophy_zh: "为 dense 吉他墙里的双踩设计：基频坐 50 Hz 不探 40 Hz 以下，鼓锤瞬态亮而短，低频紧不拖尾，次谐波基本关掉，失真只给中度饱和。",
        philosophy_en: "Built for double-kick inside a dense guitar wall: base at 50 Hz without dipping below 40 Hz, a bright short beater transient, a tight sub that never drags, rumble nearly off, and only moderate saturation.",
        // 理由：50 Hz（金属底鼓坐 50–60 Hz，不探 40 Hz 以下以免与贝斯低频打架）；
        // softness 0.10（音高下滑快，起振锋利）；click 0.90 + tameHighs 0.10
        //（鼓锤瞬态亮、切得进 dense 吉他墙）；grit 0.50（总线失真量 0.5·grip，
        // 中度饱和，不糊）；boom 0.25（sub 衰减 0.425 s，低频紧）；
        // hitSkin 0.90（thump 厚，衰减 0.212 s）；rumble 0.10（基本关掉次谐波，
        // 避免低频浑浊）。
        // 与原 6 套的重合关系：与 industrial-revolt 同取向（高 click、高 grit、
        // 高 hitSkin），但 9 项参数全不相同（industrial：46 Hz / grit 0.92 /
        // boom 0.5 / rumble 0.45 / volume 0.95）；本套是 industrial 的收紧版，
        // 不是复制。
        params: AnatomyKickParams::from_nine(0.10, 0.90, 0.10, 0.50, 0.25, 0.90, 0.10, 50.0, 1.0),
    },
    AnatomyKickPreset {
        id: "punchy-rock-kick",
        name_zh: "punchy 摇滚底鼓",
        name_en: "Punchy Rock Kick",
        category: AnatomyKickCategory::Rock,
        philosophy_zh: "摇滚底鼓的中间态：鼓锤清晰但不刺，轻度模拟饱和，低频有 body 但不拖，留一点次谐波厚度。",
        philosophy_en: "The middle of the rock kick: a beater that is clear but never harsh, light analogue-style saturation, a sub with body that never drags, and a touch of rumble thickness.",
        // 理由：48 Hz 默认基频（摇滚底鼓标准位置）；click 0.75 + tameHighs 0.25
        //（鼓锤清晰但不刺）；grit 0.30（轻度饱和，贴合摇滚的模拟味）；
        // boom 0.45（sub 衰减 0.605 s，有 body 不拖）；hitSkin 0.75（thump 饱满）；
        // rumble 0.25（一点次谐波厚度）。
        // 与原 6 套的重合关系：与 detroit-mechanical 同取向（高 click、短 boom），
        // 但 9 项参数全不相同（detroit：52 Hz / click 0.85 / tame 0.15 / grit 0.4 /
        // boom 0.32 / hitSkin 0.8 / rumble 0.2）；本套比 detroit 更暖（tame 更高、
        // grit 更低、boom 更长）。
        params: AnatomyKickParams::from_nine(0.30, 0.75, 0.25, 0.30, 0.45, 0.75, 0.25, 48.0, 1.0),
    },
    AnatomyKickPreset {
        id: "anatomy-default",
        name_zh: "Anatomy 默认",
        name_en: "Anatomy Default",
        category: AnatomyKickCategory::Somatic,
        philosophy_zh: "Groove 的出厂默认（DEFAULT_SOMATIC_PARAMS 原样），作新预设的参照锚点。",
        philosophy_en: "Groove's factory default (DEFAULT_SOMATIC_PARAMS verbatim), kept as the reference anchor for the new presets.",
        // 与原 6 套的重合关系：无一相同。逐项比对最接近的 detroit-mechanical：
        // basePitch 48 vs 52、softness 0.35 vs 0.15、click 0.65 vs 0.85、
        // tameHighs 0.3 vs 0.15、grit 0.25 vs 0.4、boom 0.55 vs 0.32、
        // hitSkin 0.6 vs 0.8、rumble 0.4 vs 0.2、volume 1.0 vs 1.0 ——
        // 除 volume 外 8 项全不同。
        params: AnatomyKickParams::DEFAULT,
    },
];

/// 触发时解析出的系数（`reconfigure` 算好；逐样本路径只做乘加与查表）。
///
/// 全部包络都是解析式（`t` = 触发后秒数），与准绳的 `linearRamp` /
/// `exponentialRamp` 同一公式：线性段 `v0 + (v1 − v0)·(t − t0)/(t1 − t0)`；
/// 指数段 `v1 · (v2/v1)^((t − t1)/(t2 − t1))` =
/// `v1 · exp(ln(v2/v1)·(t − t1)/(t2 − t1))`。`_log_per_s` 存的就是
/// `ln(比值) / 段长`，逐样本只需一次 `expf`。
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnatomyCoef {
    /// 采样率（Hz，解析包络 `t = frame / sample_rate` 的分母）。
    sample_rate: f32,
    /// Sub 起始频率（Hz）。
    sub_start_hz: f32,
    /// Sub 终止频率（Hz，= `basePitch`）。
    sub_end_hz: f32,
    /// Sub 音高滑频的指数系数：`ln(end/start) / pitch_drop_s`（/秒）。
    sub_pitch_log_per_s: f32,
    /// Sub 音高下滑耗时（秒）。
    sub_pitch_drop_s: f32,
    /// Sub 峰值（`1.25 · volume`；`vel` 由 `slot.gain` 给，见模块文档 D1）。
    sub_peak: f32,
    /// Sub 包络中段的指数系数：`ln(0.45) / 0.078`（/秒）。
    sub_knee_log_per_s: f32,
    /// Sub 包络尾段的指数系数：`ln(0.0001/(peak·0.45)) / (sub_decay − 0.08)`。
    sub_tail_log_per_s: f32,
    /// Sub 衰减时间（秒）。
    sub_decay_s: f32,
    /// Rumble 峰值（`rumble · 0.4 · volume`）。
    rumble_peak: f32,
    /// Rumble 包络的指数系数：`ln(0.0001/peak) / (rumble_decay − 0.04)`。
    rumble_log_per_s: f32,
    /// Rumble 衰减时间（秒，= `sub_decay · 1.1`）。
    rumble_decay_s: f32,
    /// Thump 起始频率（Hz）。
    thump_start_hz: f32,
    /// Thump 终止频率（Hz，恒 `110`）。
    thump_end_hz: f32,
    /// Thump 音高滑频的指数系数：`ln(end/start) / 0.022`（/秒）。
    thump_pitch_log_per_s: f32,
    /// Thump 峰值（`(0.8 + hitSkin·0.5) · volume`）。
    thump_peak: f32,
    /// Thump 包络的指数系数：`ln(0.0001/peak) / (thump_decay − 0.001)`。
    thump_log_per_s: f32,
    /// Thump 衰减时间（秒）。
    thump_decay_s: f32,
    /// Click 起始频率（Hz，恒 `1800`）。
    click_start_hz: f32,
    /// Click 终止频率（Hz，恒 `750`）。
    click_end_hz: f32,
    /// Click 音高滑频的指数系数：`ln(750/1800) / 0.008`（/秒）。
    click_pitch_log_per_s: f32,
    /// Click 峰值（`clickAmount · 0.95 · volume`）。
    click_peak: f32,
    /// Click 包络的指数系数：`ln(0.0001/peak) / (click_decay − 0.0003)`。
    click_log_per_s: f32,
    /// Click 衰减时间（秒）。
    click_decay_s: f32,
    /// 总线失真器是否接入（`effectiveGrit > 0.05`）。
    bus_shaper_enabled: bool,
    /// 总线失真器的 `k = max(0.01, effectiveGrit · 0.4 · 25)`。
    bus_shaper_k: f32,
    /// `tanh(k)`（失真归一化分母，`reconfigure` 时算好）。
    bus_shaper_norm: f32,
    /// 发声总长（帧）：**在响**的层实际停振的最大值（模块文档 D3）。
    total_frames: u64,
}

impl AnatomyCoef {
    /// 空系数：采样率取 48 kHz、其余归零；只在 `AnatomyVoice::IDLE` 里用，
    /// 任何一次真实触发前都会被 `reconfigure` 重写。
    pub(crate) const IDLE: Self = Self {
        sample_rate: 48_000.0,
        sub_start_hz: 0.0,
        sub_end_hz: 0.0,
        sub_pitch_log_per_s: 0.0,
        sub_pitch_drop_s: 0.025,
        sub_peak: 0.0,
        sub_knee_log_per_s: 0.0,
        sub_tail_log_per_s: 0.0,
        sub_decay_s: 0.0,
        rumble_peak: 0.0,
        rumble_log_per_s: 0.0,
        rumble_decay_s: 0.0,
        thump_start_hz: 0.0,
        thump_end_hz: 0.0,
        thump_pitch_log_per_s: 0.0,
        thump_peak: 0.0,
        thump_log_per_s: 0.0,
        thump_decay_s: 0.0,
        click_start_hz: 0.0,
        click_end_hz: 0.0,
        click_pitch_log_per_s: 0.0,
        click_peak: 0.0,
        click_log_per_s: 0.0,
        click_decay_s: 0.0,
        bus_shaper_enabled: false,
        bus_shaper_k: 0.01,
        bus_shaper_norm: 1.0,
        total_frames: 0,
    };
}

/// `u32` 定点相位 → 三角波值。
///
/// 与 [`sine_from_phase`] 同一款 `phase >> 8` 取值：`2²⁴` 以内的整数在 `f32`
/// 里精确 ⇒ 这一步没有舍入。Web Audio 三角波从 0 起、向上走：
/// `x = 0 → 0`、`0.25 → 1`、`0.5 → 0`、`0.75 → −1`、`1 → 0`。
#[inline]
#[must_use]
fn triangle_from_phase(phase: u32) -> f32 {
    let x = (phase >> 8) as f32 * (1.0 / 16_777_216.0);
    if x < 0.25 {
        4.0 * x
    } else if x < 0.75 {
        2.0 - 4.0 * x
    } else {
        -4.0 + 4.0 * x
    }
}

/// `u32` 定点相位 → 方波值（与踩镲的 6 条方波同款：看相位最高位）。
#[inline]
#[must_use]
fn square_from_phase(phase: u32) -> f32 {
    if phase < 0x8000_0000 { 1.0 } else { -1.0 }
}

/// 指数斜坡的对数比值：`ln(target/start)`，钳到 `(0, 1]` 区间。
///
/// `start <= target`（峰值已在底线之下，例如速度极低时）时比值钳到 `1`
/// ⇒ 该段保持在起点值、不反向上升；比值下界 `1e-6` 防止 `ln(0) = −∞`。
#[must_use]
fn ramp_log_ratio(target: f32, start: f32) -> f32 {
    libm::logf((target / start).clamp(1e-6, 1.0))
}

/// Sub 层幅度包络（解析式，`t` = 触发后秒数）。
#[must_use]
fn sub_amp(coef: &AnatomyCoef, t: f32) -> f32 {
    if t < SUB_ATTACK_S {
        ENVELOPE_FLOOR + (coef.sub_peak - ENVELOPE_FLOOR) * (t / SUB_ATTACK_S)
    } else if t < SUB_KNEE_S {
        coef.sub_peak * libm::expf(coef.sub_knee_log_per_s * (t - SUB_ATTACK_S))
    } else if t < coef.sub_decay_s {
        coef.sub_peak * SUB_KNEE_RATIO * libm::expf(coef.sub_tail_log_per_s * (t - SUB_KNEE_S))
    } else {
        ENVELOPE_FLOOR
    }
}

/// Rumble 层幅度包络（解析式）。
#[must_use]
fn rumble_amp(coef: &AnatomyCoef, t: f32) -> f32 {
    if t < RUMBLE_ATTACK_S {
        ENVELOPE_FLOOR + (coef.rumble_peak - ENVELOPE_FLOOR) * (t / RUMBLE_ATTACK_S)
    } else if t < coef.rumble_decay_s {
        coef.rumble_peak * libm::expf(coef.rumble_log_per_s * (t - RUMBLE_ATTACK_S))
    } else {
        ENVELOPE_FLOOR
    }
}

/// Thump 层幅度包络（解析式）。
#[must_use]
fn thump_amp(coef: &AnatomyCoef, t: f32) -> f32 {
    if t < THUMP_ATTACK_S {
        ENVELOPE_FLOOR + (coef.thump_peak - ENVELOPE_FLOOR) * (t / THUMP_ATTACK_S)
    } else if t < coef.thump_decay_s {
        coef.thump_peak * libm::expf(coef.thump_log_per_s * (t - THUMP_ATTACK_S))
    } else {
        ENVELOPE_FLOOR
    }
}

/// Click 层幅度包络（解析式）。
#[must_use]
fn click_amp(coef: &AnatomyCoef, t: f32) -> f32 {
    if t < CLICK_ATTACK_S {
        ENVELOPE_FLOOR + (coef.click_peak - ENVELOPE_FLOOR) * (t / CLICK_ATTACK_S)
    } else if t < coef.click_decay_s {
        coef.click_peak * libm::expf(coef.click_log_per_s * (t - CLICK_ATTACK_S))
    } else {
        ENVELOPE_FLOOR
    }
}

/// Anatomy 底鼓的一个槽位声部（`Generator` 里的 `anatomy` 字段）。
///
/// 容量取并集的取舍与 [`super::voice::Generator`] 相同：整个结构是 `Copy`
/// 值类型，槽位池因此也是定长数组，`render` / `trigger` 零分配。
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnatomyVoice {
    /// 每层相位（`2³²` 一周期）：0 = Sub、1 = Rumble、2 = Thump、3 = Click。
    phases: [u32; 4],
    /// Sub 低通（140 Hz，RBJ，Q 1）。
    sub_lp: Biquad,
    /// Thump 带通（160 Hz，Q 1.8）。
    thump_bp: Biquad,
    /// Click 带通（中心 `1600 + (1 − tameHighs) × 1200`，Q 4.5）。
    click_bp: Biquad,
    /// 抢占淡出包络（attack 1 样本、sustain 1、release 3 ms）——只做淡出乘子，
    /// 不参与"发声结束"判定（那一条由解析包络的帧数判定）。
    fade_env: Adsr,
    /// 触发时解析出的系数。
    coef: AnatomyCoef,
    /// 这一击的速度归一值（`0..=1`，触发时由 `DrumHit::gain` 存入；
    /// `reconfigure` 用它重算速度-音色映射）。
    vel_norm: f32,
    /// 触发时快照的层开关（独奏 / 静音是触发时快照，见模块文档 §3；
    /// click 层还叠加 `clickAmount > 0.01` 的量门，rumble 层还叠加
    /// `rumble > 0.2` 的量门与 sub 层的开关）。
    play_sub: bool,
    play_rumble: bool,
    play_thump: bool,
    play_click: bool,
}

impl AnatomyVoice {
    /// 空声部：全部相位与滤波器状态归零、淡出包络 Idle、层开关全开。
    pub(crate) const IDLE: Self = Self {
        phases: [0; 4],
        sub_lp: Biquad::identity(),
        thump_bp: Biquad::identity(),
        click_bp: Biquad::identity(),
        fade_env: Adsr::new(),
        coef: AnatomyCoef::IDLE,
        vel_norm: 1.0,
        play_sub: true,
        play_rumble: true,
        play_thump: true,
        play_click: true,
    };

    /// 触发：快照层开关与速度（在 `reconfigure` 之前调用）。
    ///
    /// 独奏 / 静音按模块文档 §3 的公式；click 层与 rumble 层的量门
    /// （`clickAmount > 0.01` / `rumble > 0.2`）同样是触发时读 ——
    /// 准绳里这两道门决定振荡器造不造，不是运行时的连续参数。
    pub(crate) fn trigger(&mut self, params: &AnatomyKickParams, gain: f32) {
        let any_solo = params.sub_solo || params.thump_solo || params.click_solo;
        self.play_sub = !params.sub_mute && (!any_solo || params.sub_solo);
        self.play_thump = !params.thump_mute && (!any_solo || params.thump_solo);
        self.play_click =
            !params.click_mute && (!any_solo || params.click_solo) && params.click_amount > 0.01;
        self.play_rumble = self.play_sub && params.rumble > 0.2;
        self.vel_norm = gain.clamp(0.0, 1.0);
    }

    /// 触发后 gate_on 淡出包络（attack 1 样本到 1，sustain 1 保持）。
    pub(crate) fn gate_on(&mut self) {
        self.fade_env.gate_on();
    }

    /// 进入淡出（窃取 / choke）：淡出包络用 [ARCH-RT-004] 的 3 ms 指数释放。
    pub(crate) fn begin_fade(&mut self) {
        self.fade_env.start_steal_fade();
    }

    /// 按新的参数重算"参数 → 系数"的映射，**保留全部状态值**
    /// （相位、滤波器状态、触发时快照的层开关与速度都不动）。
    ///
    /// 调用方保证参数已由 [`AnatomyKickParams::sanitised`] 钳到合法域
    /// （与 `voice.rs` §3 的口径相同：参数进不了本文件）。
    pub(crate) fn reconfigure(&mut self, params: &AnatomyKickParams, sample_rate: f32) {
        let sample_rate = sanitise_sample_rate(sample_rate);
        // 淡出包络：attack 1 样本、sustain 1（保持）、release 3 ms。
        // `set_sample_rate` 与 `set_params` 各自带去抖，防御性重复调用零成本。
        self.fade_env.set_sample_rate(sample_rate);
        self.fade_env
            .set_params(1.0 / sample_rate, 0.1, 1.0, STEAL_RELEASE_SECONDS);

        // --- 速度-音色映射（Q8）---
        let vel_norm = self.vel_norm;
        let grip_scale = 0.55 + 0.45 * libm::powf(vel_norm, 0.6);
        let decay_scale = 1.0 + 0.25 * (1.0 - vel_norm);
        let effective_grit = params.grit * grip_scale;
        let effective_softness = (params.softness / grip_scale).min(0.95);
        let base_pitch = params.base_pitch_hz;

        // --- Sub ---
        let sub_start_hz = base_pitch * (1.0 + 2.2 * (1.0 - effective_softness));
        let sub_pitch_drop_s = 0.025 + effective_softness * 0.04;
        let sub_decay_s = 0.2 + params.boom_to_where * 0.9;
        let sub_peak = 1.25 * params.volume;
        let sub_knee_log_per_s = libm::logf(SUB_KNEE_RATIO) / (SUB_KNEE_S - SUB_ATTACK_S);
        let sub_tail_log_per_s =
            ramp_log_ratio(ENVELOPE_FLOOR, sub_peak * SUB_KNEE_RATIO) / (sub_decay_s - SUB_KNEE_S);

        // --- Rumble ---
        let rumble_decay_s = sub_decay_s * 1.1;
        let rumble_peak = params.rumble * 0.4 * params.volume;
        let rumble_log_per_s =
            ramp_log_ratio(ENVELOPE_FLOOR, rumble_peak) / (rumble_decay_s - RUMBLE_ATTACK_S);

        // --- Thump ---
        let thump_start_hz = 380.0 * (1.0 - effective_softness * 0.3);
        let thump_end_hz = 110.0;
        let thump_pitch_log_per_s = libm::logf(thump_end_hz / thump_start_hz) / 0.022;
        let thump_decay_s = 0.05 + params.hit_skin * 0.18;
        let thump_peak = (0.8 + params.hit_skin * 0.5) * params.volume;
        let thump_log_per_s =
            ramp_log_ratio(ENVELOPE_FLOOR, thump_peak) / (thump_decay_s - THUMP_ATTACK_S);

        // --- Click ---
        let click_start_hz = 1800.0;
        let click_end_hz = 750.0;
        let click_pitch_log_per_s = libm::logf(click_end_hz / click_start_hz) / 0.008;
        let click_decay_s = (0.008 + (1.0 - effective_softness) * 0.007) * decay_scale;
        let click_peak = params.click_amount * 0.95 * params.volume;
        let click_log_per_s =
            ramp_log_ratio(ENVELOPE_FLOOR, click_peak) / (click_decay_s - CLICK_ATTACK_S);
        let click_band_hz = 1600.0 + (1.0 - params.tame_highs) * 1200.0;

        // --- 总线失真器 ---
        let bus_shaper_enabled = effective_grit > 0.05;
        let bus_shaper_k = (effective_grit * 0.4 * 25.0).max(0.01);
        let bus_shaper_norm = libm::tanhf(bus_shaper_k);

        // --- 滤波器系数（只换系数、保留状态）---
        self.sub_lp
            .lowpass(sample_rate, SUB_LOWPASS_HZ, SUB_LOWPASS_Q);
        self.thump_bp
            .bandpass(sample_rate, THUMP_BAND_HZ, THUMP_BAND_Q);
        self.click_bp
            .bandpass(sample_rate, click_band_hz, CLICK_BAND_Q);

        // --- 发声总长：在响的层实际停振的最大值（模块文档 D3）---
        let mut total_s = 0.0f32;
        if self.play_sub {
            total_s = total_s.max(sub_decay_s + LONG_STOP_TAIL_S);
        }
        if self.play_rumble {
            total_s = total_s.max(rumble_decay_s + LONG_STOP_TAIL_S);
        }
        if self.play_thump {
            total_s = total_s.max(thump_decay_s + LONG_STOP_TAIL_S);
        }
        if self.play_click {
            total_s = total_s.max(click_decay_s + CLICK_STOP_TAIL_S);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let total_frames = (total_s * sample_rate).ceil() as u64;

        self.coef = AnatomyCoef {
            sample_rate,
            sub_start_hz,
            sub_end_hz: base_pitch,
            sub_pitch_log_per_s: libm::logf(base_pitch / sub_start_hz) / sub_pitch_drop_s,
            sub_pitch_drop_s,
            sub_peak,
            sub_knee_log_per_s,
            sub_tail_log_per_s,
            sub_decay_s,
            rumble_peak,
            rumble_log_per_s,
            rumble_decay_s,
            thump_start_hz,
            thump_end_hz,
            thump_pitch_log_per_s,
            thump_peak,
            thump_log_per_s,
            thump_decay_s,
            click_start_hz,
            click_end_hz,
            click_pitch_log_per_s,
            click_peak,
            click_log_per_s,
            click_decay_s,
            bus_shaper_enabled,
            bus_shaper_k,
            bus_shaper_norm,
            total_frames,
        };
    }

    /// 当前**输出幅度包络**（用作窃取优先级里的"振幅能量"）。
    ///
    /// 在响的层里取最大的那一个包络值，再乘淡出包络 —— 与经典声部的
    /// `body_env.value()` 同量纲（`0..=1` 量级）。
    #[must_use]
    pub(crate) fn level(&self, frame: u64) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let t = frame as f32 / self.coef.sample_rate;
        let mut peak = 0.0f32;
        if self.play_sub {
            peak = peak.max(sub_amp(&self.coef, t));
        }
        if self.play_rumble {
            peak = peak.max(rumble_amp(&self.coef, t));
        }
        if self.play_thump {
            peak = peak.max(thump_amp(&self.coef, t));
        }
        if self.play_click {
            peak = peak.max(click_amp(&self.coef, t));
        }
        peak * self.fade_env.value()
    }

    /// 本槽位是否已经发声完毕（可以回收）：经过的帧数到达发声总长。
    #[must_use]
    pub(crate) fn is_finished(&self, frame: u64) -> bool {
        frame >= self.coef.total_frames
    }

    /// 推进一个样本并返回它。**实时路径**：零分配、零锁、零 I/O。
    ///
    /// 信号流（与准绳的接线一一对应）：sub 正弦 → 低通 → 包络；
    /// rumble 正弦 → 包络；thump 三角 → 带通 → 包络；
    /// click 方波 → 带通 → 包络；四层相加 → 总线失真器（若接入）
    /// → 淡出乘子。相位取波形值**再**推进 ⇒ 第 0 帧的相位正好是 0。
    #[inline]
    pub(crate) fn next(&mut self, frame: u64) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let t = frame as f32 / self.coef.sample_rate;
        let fade = self.fade_env.process(true);
        let mut mix = 0.0f32;
        if self.play_sub {
            let freq = if t < self.coef.sub_pitch_drop_s {
                self.coef.sub_start_hz * libm::expf(self.coef.sub_pitch_log_per_s * t)
            } else {
                self.coef.sub_end_hz
            };
            let sub = sine_from_phase(self.phases[0]);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let step = increment_for(freq, self.coef.sample_rate) as u32;
            self.phases[0] = self.phases[0].wrapping_add(step);
            mix += self.sub_lp.process_sample(sub) * sub_amp(&self.coef, t);
        }
        if self.play_rumble {
            // Rumble 无滑频、无滤波（准绳原文）。
            let rumble = sine_from_phase(self.phases[1]);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let step = increment_for(self.coef.sub_end_hz * 0.75, self.coef.sample_rate) as u32;
            self.phases[1] = self.phases[1].wrapping_add(step);
            mix += rumble * rumble_amp(&self.coef, t);
        }
        if self.play_thump {
            let freq = if t < 0.022 {
                self.coef.thump_start_hz * libm::expf(self.coef.thump_pitch_log_per_s * t)
            } else {
                self.coef.thump_end_hz
            };
            let thump = triangle_from_phase(self.phases[2]);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let step = increment_for(freq, self.coef.sample_rate) as u32;
            self.phases[2] = self.phases[2].wrapping_add(step);
            mix += self.thump_bp.process_sample(thump) * thump_amp(&self.coef, t);
        }
        if self.play_click {
            let freq = if t < 0.008 {
                self.coef.click_start_hz * libm::expf(self.coef.click_pitch_log_per_s * t)
            } else {
                self.coef.click_end_hz
            };
            let click = square_from_phase(self.phases[3]);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let step = increment_for(freq, self.coef.sample_rate) as u32;
            self.phases[3] = self.phases[3].wrapping_add(step);
            mix += self.click_bp.process_sample(click) * click_amp(&self.coef, t);
        }
        if self.coef.bus_shaper_enabled {
            let k = self.coef.bus_shaper_k;
            mix = libm::tanhf(k * mix) / self.coef.bus_shaper_norm;
        }
        mix * fade
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 敌意输入下每个字段的钳制值（单位：各字段自己的单位）。
    #[test]
    fn sanitised_clamps_every_field_into_its_documented_domain() {
        let hostile = AnatomyKickParams {
            softness: f32::NAN,
            click_amount: f32::INFINITY,
            tame_highs: -1.0,
            grit: 2.0,
            boom_to_where: f32::NEG_INFINITY,
            hit_skin: f32::NAN,
            rumble: 1.5,
            base_pitch_hz: f32::NAN,
            volume: f32::INFINITY,
            sub_mute: true,
            sub_solo: true,
            thump_mute: true,
            thump_solo: true,
            click_mute: true,
            click_solo: true,
        };
        let clean = hostile.sanitised();
        assert_eq!(clean.softness.to_bits(), 0.0f32.to_bits(), "NaN 归下界");
        assert_eq!(
            clean.click_amount.to_bits(),
            0.0f32.to_bits(),
            "非有限（+∞）归下界，与模块口径一致"
        );
        assert_eq!(clean.tame_highs.to_bits(), 0.0f32.to_bits());
        assert_eq!(clean.grit.to_bits(), 1.0f32.to_bits());
        assert_eq!(clean.boom_to_where.to_bits(), 0.0f32.to_bits());
        assert_eq!(clean.hit_skin.to_bits(), 0.0f32.to_bits(), "NaN 归下界");
        assert_eq!(clean.rumble.to_bits(), 1.0f32.to_bits());
        assert_eq!(
            clean.base_pitch_hz.to_bits(),
            ANATOMY_BASE_PITCH_MIN_HZ.to_bits(),
            "NaN 归下界 32 Hz"
        );
        assert_eq!(
            clean.volume.to_bits(),
            0.0f32.to_bits(),
            "非有限（+∞）归下界，与模块口径一致"
        );
        // 开关原样保留（bool 没有合法域问题）。
        assert!(clean.sub_mute && clean.sub_solo);
        assert!(clean.thump_mute && clean.thump_solo);
        assert!(clean.click_mute && clean.click_solo);
        // 幂等：再钳一次不变。
        assert_eq!(clean.sanitised(), clean);
    }

    /// `KickStyle::from_f32` 的阈值：`>= 0.5` 为 Anatomy，其余（含 NaN）为 Classic。
    #[test]
    fn kick_style_from_f32_threshold() {
        assert_eq!(KickStyle::from_f32(0.0), KickStyle::Classic);
        assert_eq!(KickStyle::from_f32(0.49), KickStyle::Classic);
        assert_eq!(KickStyle::from_f32(0.5), KickStyle::Anatomy);
        assert_eq!(KickStyle::from_f32(1.0), KickStyle::Anatomy);
        assert_eq!(KickStyle::from_f32(2.0), KickStyle::Anatomy);
        assert_eq!(KickStyle::from_f32(-1.0), KickStyle::Classic);
        assert_eq!(KickStyle::from_f32(f32::NAN), KickStyle::Classic);
        assert_eq!(KickStyle::from_f32(f32::INFINITY), KickStyle::Classic);
        assert_eq!(KickStyle::default(), KickStyle::Classic);
    }

    /// 预设表完整性：9 套、id 唯一、前 6 套是 Groove 原 id 且顺序与准绳一致、
    /// 每套 philosophy 双语非空、分类名与准绳一致。
    #[test]
    fn preset_table_is_complete_and_verbatim() {
        assert_eq!(ANATOMY_KICK_PRESETS.len(), 9);
        let ids: Vec<&str> = ANATOMY_KICK_PRESETS.iter().map(|p| p.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 9, "id 必须唯一");
        assert_eq!(
            &ids[..6],
            [
                "berlin-orphic",
                "detroit-mechanical",
                "somatic-808-gravity",
                "industrial-revolt",
                "acoustic-beater-skin",
                "neural-click-clock",
            ],
            "前 6 套是 Groove 原预设，按准绳顺序"
        );
        for preset in &ANATOMY_KICK_PRESETS {
            assert!(!preset.name_zh.is_empty() && !preset.name_en.is_empty());
            assert!(
                !preset.philosophy_zh.is_empty() && !preset.philosophy_en.is_empty(),
                "{} 的 philosophy 必须双语齐全",
                preset.id
            );
            assert!(!preset.category.name().is_empty());
        }
        // 逐项核对两套原预设的全部 9 个连续参数（Groove 原文值）。
        let berlin = AnatomyKickParams::preset("berlin-orphic").expect("原预设必须在表");
        assert_eq!(
            berlin,
            AnatomyKickParams::from_nine(0.2, 0.4, 0.55, 0.35, 0.8, 0.5, 0.7, 42.0, 1.05)
        );
        let industrial = AnatomyKickParams::preset("industrial-revolt").expect("原预设必须在表");
        assert_eq!(
            industrial,
            AnatomyKickParams::from_nine(0.05, 0.9, 0.05, 0.92, 0.5, 0.95, 0.45, 46.0, 0.95)
        );
        // 新预设的参照锚点就是出厂默认本身。
        assert_eq!(
            AnatomyKickParams::preset("anatomy-default").expect("新预设必须在表"),
            AnatomyKickParams::DEFAULT
        );
        assert!(AnatomyKickParams::preset("no-such-preset").is_none());
        // 分类名与准绳一致。
        assert_eq!(ANATOMY_KICK_PRESETS[0].category.name(), "orphic");
        assert_eq!(ANATOMY_KICK_PRESETS[6].category.name(), "metal");
        assert_eq!(ANATOMY_KICK_PRESETS[7].category.name(), "rock");
        assert_eq!(ANATOMY_KICK_PRESETS[8].category.name(), "somatic");
    }

    /// 默认预设、速度 1.0、48 kHz 下，解析出的系数与准绳公式手算值一致。
    ///
    /// 手算（`velNorm = 1` ⇒ `gripScale = 1`、`decayScale = 1`、
    /// `effectiveGrit = 0.25`、`effectiveSoftness = 0.35`）：
    /// sub 起始 `48·(1+2.2·0.65) = 116.64` Hz、下滑 `0.025+0.35·0.04 = 0.039` s、
    /// 衰减 `0.2+0.55·0.9 = 0.695` s、峰值 `1.25`；
    /// rumble 36 Hz、衰减 `0.695·1.1 = 0.7645` s、峰值 `0.4·0.4 = 0.16`；
    /// thump 起始 `380·(1−0.35·0.3) = 340.1` Hz、衰减 `0.05+0.6·0.18 = 0.158` s、
    /// 峰值 `0.8+0.6·0.5 = 1.1`；click 带通 `1600+0.7·1200 = 2440` Hz、
    /// 衰减 `0.008+0.65·0.007 = 0.01255` s、峰值 `0.65·0.95 = 0.6175`；
    /// 总线失真 `k = 0.25·0.4·25 = 2.5`；总长 `0.7645+0.05 = 0.8145` s。
    #[test]
    fn default_preset_coefficients_match_the_groove_formulas() {
        let mut voice = AnatomyVoice::IDLE;
        voice.trigger(&AnatomyKickParams::DEFAULT, 1.0);
        voice.reconfigure(&AnatomyKickParams::DEFAULT, 48_000.0);
        let c = voice.coef;
        let close = |got: f32, want: f64| {
            assert!(
                (f64::from(got) - want).abs() < 1e-4,
                "系数 {got} 与手算 {want} 的偏差超过 1e-4"
            );
        };
        close(c.sub_start_hz, 116.64);
        close(c.sub_end_hz, 48.0);
        close(c.sub_pitch_drop_s, 0.039);
        close(c.sub_decay_s, 0.695);
        close(c.sub_peak, 1.25);
        close(c.rumble_peak, 0.16);
        close(c.rumble_decay_s, 0.7645);
        close(c.thump_start_hz, 340.1);
        close(c.thump_end_hz, 110.0);
        close(c.thump_decay_s, 0.158);
        close(c.thump_peak, 1.1);
        close(c.click_peak, 0.6175);
        close(c.click_decay_s, 0.01255);
        close(c.bus_shaper_k, 2.5);
        assert!(
            c.bus_shaper_enabled,
            "effectiveGrit 0.25 > 0.05 ⇒ 失真器接入"
        );
        assert!(
            (c.total_frames as i64 - 39_096).abs() <= 2,
            "总长 {} 帧，与手算 0.8145 s × 48 kHz = 39096 帧偏差超过 2 帧",
            c.total_frames
        );
        // 三段包络的拐点值（解析式在拐点处连续）。
        assert!((sub_amp(&c, 0.0) - 0.0001).abs() < 1e-6);
        assert!((sub_amp(&c, 0.002) - 1.25).abs() < 1e-4, "2 ms 处到峰值");
        assert!(
            (sub_amp(&c, 0.08) - 1.25 * 0.45).abs() < 1e-4,
            "80 ms 处为峰值的 0.45"
        );
        assert!(
            (sub_amp(&c, 0.695) - 0.0001).abs() < 1e-4,
            "衰减终点回到底线"
        );
        assert!((thump_amp(&c, 0.001) - 1.1).abs() < 1e-4);
        assert!((click_amp(&c, 0.0003) - 0.6175).abs() < 1e-4);
        assert!((rumble_amp(&c, 0.04) - 0.16).abs() < 1e-4);
    }

    /// 独奏 / 静音快照逻辑（模块文档 §3 的公式）+ 两道量门。
    #[test]
    fn solo_mute_snapshot_logic() {
        let mut voice = AnatomyVoice::IDLE;
        // 缺省：三层全开、rumble 有（`rumble 0.4 > 0.2`）。
        voice.trigger(&AnatomyKickParams::DEFAULT, 1.0);
        assert!(voice.play_sub && voice.play_rumble && voice.play_thump && voice.play_click);
        // 全静音 ⇒ 全关。
        let mut all_mute = AnatomyKickParams::DEFAULT;
        all_mute.sub_mute = true;
        all_mute.thump_mute = true;
        all_mute.click_mute = true;
        voice.trigger(&all_mute, 1.0);
        assert!(!voice.play_sub && !voice.play_rumble && !voice.play_thump && !voice.play_click);
        // 只独奏 click ⇒ 只有 click（rumble 随 sub 关）。
        let mut click_solo = AnatomyKickParams::DEFAULT;
        click_solo.click_solo = true;
        voice.trigger(&click_solo, 1.0);
        assert!(!voice.play_sub && !voice.play_rumble && !voice.play_thump && voice.play_click);
        // 独奏 sub 但 sub 又静音 ⇒ sub 关（静音优先于独奏）。
        let mut sub_solo_mute = AnatomyKickParams::DEFAULT;
        sub_solo_mute.sub_solo = true;
        sub_solo_mute.sub_mute = true;
        voice.trigger(&sub_solo_mute, 1.0);
        assert!(!voice.play_sub && !voice.play_rumble);
        // 量门：`clickAmount 0.01` 关 click；`rumble 0.2` 关 rumble。
        let mut no_click = AnatomyKickParams::DEFAULT;
        no_click.click_amount = 0.01;
        voice.trigger(&no_click, 1.0);
        assert!(!voice.play_click, "clickAmount <= 0.01 时 click 层不存在");
        let mut no_rumble = AnatomyKickParams::DEFAULT;
        no_rumble.rumble = 0.2;
        voice.trigger(&no_rumble, 1.0);
        assert!(
            voice.play_sub && !voice.play_rumble,
            "rumble <= 0.2 时 rumble 层不存在"
        );
        let mut yes_rumble = AnatomyKickParams::DEFAULT;
        yes_rumble.rumble = 0.21;
        voice.trigger(&yes_rumble, 1.0);
        assert!(voice.play_rumble);
    }

    /// 速度-音色映射：低速击的音色与高速击不同（不只是电平）。
    ///
    /// 手算（`vel = 0.2`）：`gripScale = 0.55+0.45·0.2^0.6 ≈ 0.7213`、
    /// `effectiveSoftness = 0.35/0.7213 ≈ 0.4853` ⇒ sub 起始
    /// `48·(1+2.2·(1−0.4853)) ≈ 102.35` Hz（高速击是 116.64 Hz）；
    /// `decayScale = 1.2` ⇒ click 衰减
    /// `(0.008+(1−0.4853)·0.007)·1.2 ≈ 0.01392` s（高速击是 0.01255 s）。
    #[test]
    fn velocity_mapping_changes_timbre_not_just_level() {
        let mut soft = AnatomyVoice::IDLE;
        soft.trigger(&AnatomyKickParams::DEFAULT, 0.2);
        soft.reconfigure(&AnatomyKickParams::DEFAULT, 48_000.0);
        let mut hard = AnatomyVoice::IDLE;
        hard.trigger(&AnatomyKickParams::DEFAULT, 1.0);
        hard.reconfigure(&AnatomyKickParams::DEFAULT, 48_000.0);
        assert!(
            (f64::from(soft.coef.sub_start_hz) - 102.35).abs() < 0.05,
            "低速击的 sub 起始频率 {} Hz，与手算 102.35 Hz 不符",
            soft.coef.sub_start_hz
        );
        assert!(
            (f64::from(hard.coef.sub_start_hz) - 116.64).abs() < 0.05,
            "高速击的 sub 起始频率 {} Hz，与手算 116.64 Hz 不符",
            hard.coef.sub_start_hz
        );
        assert!(
            (f64::from(soft.coef.click_decay_s) - 0.01392).abs() < 0.00005,
            "低速击的 click 衰减 {} s，与手算 0.01392 s 不符",
            soft.coef.click_decay_s
        );
        assert!(
            (f64::from(hard.coef.click_decay_s) - 0.01255).abs() < 0.00005,
            "高速击的 click 衰减 {} s，与手算 0.01255 s 不符",
            hard.coef.click_decay_s
        );
        // 峰值公式里 vel 取 1（电平由 slot.gain 给，模块文档 D1）⇒
        // 两击的 sub 峰值系数相同，音色差异只来自映射。
        assert_eq!(soft.coef.sub_peak.to_bits(), hard.coef.sub_peak.to_bits());
    }

    /// 三角波 / 方波的相位约定：三角从 0 起向上、方波上半周期为 +1。
    #[test]
    fn triangle_and_square_phase_conventions() {
        assert_eq!(triangle_from_phase(0), 0.0);
        assert!((triangle_from_phase(0x4000_0000) - 1.0).abs() < 1e-6);
        assert!(triangle_from_phase(0x8000_0000).abs() < 1e-6);
        assert!((triangle_from_phase(0xC000_0000) + 1.0).abs() < 1e-6);
        assert_eq!(square_from_phase(0), 1.0);
        assert_eq!(square_from_phase(0x7FFF_FFFF), 1.0);
        assert_eq!(square_from_phase(0x8000_0000), -1.0);
        assert_eq!(square_from_phase(0xFFFF_FFFF), -1.0);
    }
}
