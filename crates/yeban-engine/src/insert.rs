//! **每轨插入器件**的引擎侧接线：把 `TrackV3.devices` 里的内置效果器投影成
//! 音频线程可执行的**压缩器**（`yeban_dsp::compressor`）。
//! [ARCH-RT-001, ARCH-DET-001]
//!
//! ## 1. 为什么是这一件器件（规范出处）
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:411` 把**内部 DSP 拓扑调度**的
//!   `1.00 ms` 预算写成"声部合成、**通道条 EQ/压缩**与 PDC 延迟线插入计算"；
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:109` 把通道条算法的目的地
//!   写成 `crates/yeban-dsp/src/channel_strip.rs`（EQ、滤波、动态旁通链）；
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:108` 把压缩器算法的目的地
//!   写成 `crates/yeban-dsp/src/compressor.rs`（带侧链输入的 SSL 总线压缩器模型）；
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:347` 描述参考工程 A 时写
//!   "每轨挂载 4-Band EQ 与压缩器"。
//!
//! 器件**早就实现并自带判据**，缺的只有一件事：**引擎侧没有任何调用点**
//! （核实方式：`grep -rn 'use yeban_dsp' crates/yeban-engine/src` 在接线前只命中
//! `envelope` / `filter` / `math` / `oscillator` / `polysynth` / `limiter` / `meter`）。
//! 本模块补的就是那条调用点。
//!
//! ## 2. 为什么不是"器件一存在就生效"
//!
//! 模型层的 `DeviceDefinition` 只有 `kind`（`InternalEffect`）说明"这是一个内置效果器"，
//! **没有**"这是压缩器"的类型级信息（该缺口是 `docs/ledger/engine-mix-notes.md` §8.2 的
//! **N5**：`DeviceDefinition::params` 没有参数名规范）。
//! 因此"有 `InternalEffect` ⇒ 挂压缩器"会让**任何**内置效果器（例如只为上报延迟而存在的
//! 那一台）突然开始压限 —— 那是把未知器件猜成压缩器。
//!
//! 本模块沿用**同一仓库已有的**投影规则（[`crate::synth::ToneParams::from_devices`] 的
//! 形状：只认 `InternalInstrument` 设备的 `cutoff_hz`/`resonance`/`drive` 三个约定名）：
//! **只有当设备的 `params` 里真的出现压缩器参数名时，它才是压缩器来源**。
//! 于是"没有参数的效果器"与"没有效果器"在音频上同解 ⇒ 既有工程逐位不变。
//!
//! ## 3. 投影规则（完全确定、无猜测）
//!
//! 1. 只看 [`DeviceKind::InternalEffect`] 的设备（外部效果器由插件宿主负责，
//!    `External*` 一律忽略；内置乐器是**音源**，由 [`crate::synth`] 处理）；
//! 2. `bypassed` 的设备整体忽略（旁通就是旁通，不"取默认参数偷偷接回来"）；
//! 3. 在**第一个**含**至少一个**已识别参数名的设备上取值；更早的、一个已识别参数都没有的
//!    效果器不是来源 ⇒ 继续往下找；
//! 4. 未出现的字段保持 [`CompressorParams::DEFAULT`] 的值；
//! 5. 同一设备内同名参数**后者胜**（与 `ToneParams::from_devices` 同口径）；
//! 6. 参数名大小写不敏感（`to_ascii_lowercase` 后比较）；
//! 7. 全部取值经 [`CompressorParams::sanitised`] 钳到合法域
//!    ⇒ 快照里存的就是音频线程将要用的那一份数（`NaN` 也有定义）；
//! 8. **一个已识别参数都没有 ⇒ [`InsertParams::is_empty`]**，实时侧整段跳过。
//!
//! 已识别的名字（每个字段两个别名）：
//!
//! | 字段 | 主名 | 别名 |
//! | :--- | :--- | :--- |
//! | `threshold_db` | `threshold_db` | `threshold` |
//! | `ratio` | `ratio` | —— |
//! | `knee_db` | `knee_db` | `knee` |
//! | `detector_s` | `detector_s` | `detector` |
//! | `attack_s` | `attack_s` | `attack` |
//! | `release_s` | `release_s` | `release` |
//! | `makeup_db` | `makeup_db` | `makeup` |
//!
//! ## 4. 这是**临时形状**，不是模型层的第二份定义
//!
//! 与 [`crate::synth::ToneParams`] 一样：`yeban-model` 补齐"效果器参数 → 音频线程"的
//! 投影（N5 的裁决）之后，本模块的投影部分应当**整体删除**，只保留 `pub use` 与
//! `CompressorParams` 的再导出。删除前它必须保持**唯一实现**：
//! 本文件不定义压缩器算法、不定义第二份参数类型 —— 判据
//! `engine_insert_module_has_no_second_compressor_implementation` 用源码级检查钉住它。
//!
//! ## 5. 实时分类（[ADR-0001 D32]）
//!
//! - **构造期**（控制线程，[`InsertParams::from_devices`]）：字符串比较、`Vec` 扫描、
//!   `sanitised` 的钳制 —— 允许分配，允许超越函数。
//! - **快照边界**（音频线程，每个修订一次）：[`Compressor::set_params`] /
//!   [`Compressor::set_sample_rate`] 会重算三个一阶低通系数（含 `exp`）⇒ **超越函数类**。
//!   它与引擎里既有的"武装"步骤同一条口径（`crate::rt` 在同一个分支里调
//!   [`crate::mixer::pan_gains`] 的 `cos`/`sin`）。**不在逐样本路径上**。
//! - **逐样本**（[`Compressor::process_mono`]）：乘加、除法、`log10` 不在路径上
//!   （器件内部按模块注释 §3 在均方域工作）⇒ IEEE 精确类；**零分配、零锁、零 I/O**。
//!
//! ## 6. 与"接线会改变渲染输出"的关系（本模块的边界）
//!
//! 接线的**默认口径**是"没有已识别的效果器设备 ⇒ 不挂压缩器"：
//! 实时侧对此**整段跳过**（不是"参数取成透明"）⇒ 这类工程的输出与接线前**逐位相同**。
//! 只有**显式**携带压缩器参数的工程会改变输出 —— 那正是本模块的目的。
//! 逐位一致的**实测**证据（原始样本落盘 + `shasum -a 256`）见
//! `crates/yeban-engine/tests/compressor_insert.rs` 的 C0。

use yeban_model::{DeviceDefinition, DeviceKind};

// 压缩器的唯一实现住在 `yeban-dsp`（`crates/yeban-dsp/src/compressor.rs`，规范出处见
// `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:108`）。本模块只做转发：
// 类型 + 参数类型。⚠ 引擎侧的既有命名先例是 `mixer.rs` 的 `Limiter as BusLimiter`
// —— 那是为了不改动既有调用点而保留旧名；本模块是**新**调用点，故直接用 dsp 的名字。
pub use yeban_dsp::compressor::{Compressor, CompressorParams};

/// 一条轨的**插入链**投影（当前只有一件器件：压缩器）。
///
/// 结构体保留一个"链"的形状而不是裸 `Option<CompressorParams>`：`ChannelStrip`
/// （`crates/yeban-dsp/src/channel_strip.rs`，规范出处
/// `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:109`）是同一层的下一件器件，
/// 接线时在这里**加字段**即可，实时侧的"逐轨查找 + 整段跳过"骨架不必改。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InsertParams {
    /// 本轨的压缩器参数：`None` = 本轨**没有**压缩器（实时侧整段跳过）。
    compressor: Option<CompressorParams>,
}

impl InsertParams {
    /// **空链**：实时侧对这条轨整段跳过 ⇒ 输出逐位不变。
    #[must_use]
    pub const fn empty() -> Self {
        Self { compressor: None }
    }

    /// 从模型层的设备链投影（**构造期**；见模块文档 §3 的规则表）。
    #[must_use]
    pub fn from_devices(devices: &[DeviceDefinition]) -> Self {
        let mut found: Option<CompressorParams> = None;
        for device in devices {
            // 规则 1 + 2：只看非旁通的内置效果器。
            if device.bypassed || device.kind != DeviceKind::InternalEffect {
                continue;
            }
            // 规则 3：`None` = 本设备一个已识别参数都没有 ⇒ 它不是压缩器来源。
            found = Self::compressor_of(device);
            if found.is_some() {
                break;
            }
        }
        Self {
            compressor: found.map(CompressorParams::sanitised),
        }
    }

    /// 单台设备 → 压缩器参数（`None` = 一个已识别名字都没有）。
    ///
    /// 基值是 [`CompressorParams::DEFAULT`]；只有**出现**的名字才覆盖它（规则 4）。
    fn compressor_of(device: &DeviceDefinition) -> Option<CompressorParams> {
        let mut params = CompressorParams::DEFAULT;
        let mut recognised = false;
        for param in &device.params {
            let name = param.name.to_ascii_lowercase();
            let value = param.value;
            // 规则 5：同名后者胜（逐个赋值）。规则 6：名字已转小写。
            match name.as_str() {
                "threshold_db" | "threshold" => {
                    params.threshold_db = value;
                    recognised = true;
                }
                "ratio" => {
                    params.ratio = value;
                    recognised = true;
                }
                "knee_db" | "knee" => {
                    params.knee_db = value;
                    recognised = true;
                }
                "detector_s" | "detector" => {
                    params.detector_s = value;
                    recognised = true;
                }
                "attack_s" | "attack" => {
                    params.attack_s = value;
                    recognised = true;
                }
                "release_s" | "release" => {
                    params.release_s = value;
                    recognised = true;
                }
                "makeup_db" | "makeup" => {
                    params.makeup_db = value;
                    recognised = true;
                }
                _ => {}
            }
        }
        recognised.then_some(params)
    }

    /// 本轨的压缩器参数；`None` = 整段跳过。
    #[must_use]
    pub const fn compressor(&self) -> Option<CompressorParams> {
        self.compressor
    }

    /// 本链是否为空（没有任何激活的器件）。
    ///
    /// 快照只收录**非空**的链 ⇒ "不在表里"与"表里是空链"在实时侧同解。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.compressor.is_none()
    }
}

impl Default for InsertParams {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_model::{EntityId, ParameterValue};

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
        }
    }

    /// **默认口径**：没有设备 / 没有已识别参数 ⇒ 空链（实时侧整段跳过）。
    #[test]
    fn devices_without_recognised_params_do_not_arm_a_compressor() {
        assert!(InsertParams::from_devices(&[]).is_empty());
        // 只上报延迟的效果器（既有夹具的形状）：参数为空。
        assert!(
            InsertParams::from_devices(&[device(DeviceKind::InternalEffect, false, &[])])
                .is_empty()
        );
        // 参数名不认识的第三方效果器：不许猜成压缩器。
        assert!(
            InsertParams::from_devices(&[device(
                DeviceKind::InternalEffect,
                false,
                &[("reverb_mix", 0.3), ("filter_cutoff_hz", 800.0)]
            )])
            .is_empty()
        );
        // 内置乐器（音源）不是插入器件 —— 同一个名字也不认。
        assert!(
            InsertParams::from_devices(&[device(
                DeviceKind::InternalInstrument,
                false,
                &[("ratio", 4.0)]
            )])
            .is_empty()
        );
        // 旁通的效果器就是旁通：不许"取默认参数偷偷接回来"。
        assert!(
            InsertParams::from_devices(&[device(
                DeviceKind::InternalEffect,
                true,
                &[("threshold_db", -20.0), ("ratio", 8.0)]
            )])
            .is_empty()
        );
    }

    /// **命中**：出现的名字覆盖默认值，未出现的字段保持 [`CompressorParams::DEFAULT`]。
    #[test]
    fn recognised_params_override_the_defaults_field_by_field() {
        let insert = InsertParams::from_devices(&[device(
            DeviceKind::InternalEffect,
            false,
            &[("threshold", -20.0), ("RATIO", 8.0), ("release", 0.25)],
        )]);
        let params = insert.compressor().expect("必须武装压缩器");
        assert_eq!(params.threshold_db.to_bits(), (-20.0f32).to_bits());
        assert_eq!(params.ratio.to_bits(), 8.0f32.to_bits());
        assert_eq!(params.release_s.to_bits(), 0.25f32.to_bits());
        // 未出现的字段 = 默认值（逐位）。
        let default = CompressorParams::DEFAULT;
        assert_eq!(params.knee_db.to_bits(), default.knee_db.to_bits());
        assert_eq!(params.detector_s.to_bits(), default.detector_s.to_bits());
        assert_eq!(params.attack_s.to_bits(), default.attack_s.to_bits());
        assert_eq!(params.makeup_db.to_bits(), default.makeup_db.to_bits());
    }

    /// 规则 3：第一条**没有**已识别参数的效果器不是来源，继续找下一条。
    /// 规则 5：同一设备内同名参数**后者胜**。
    /// 规则 7：非有限值经 `sanitised` 有定义（不许 `NaN` 进快照）。
    #[test]
    fn source_selection_is_ordered_and_values_are_sanitised() {
        let chain = [
            device(DeviceKind::InternalEffect, false, &[("mix", 0.5)]),
            device(
                DeviceKind::InternalEffect,
                false,
                &[("ratio", 2.0), ("ratio", 3.0), ("threshold_db", f32::NAN)],
            ),
            device(DeviceKind::InternalEffect, false, &[("ratio", 99.0)]),
        ];
        let params = InsertParams::from_devices(&chain)
            .compressor()
            .expect("第二条必须命中");
        assert_eq!(params.ratio.to_bits(), 3.0f32.to_bits(), "同名后者胜");
        assert!(params.threshold_db.is_finite(), "非有限值必须被钳掉");
        assert_eq!(
            params.threshold_db.to_bits(),
            yeban_dsp::compressor::MIN_LEVEL_DB.to_bits(),
            "NaN 归到下界（与 sanitised 同口径）"
        );
    }

    /// 判据：**engine 侧没有第二份压缩器实现**（源码级机械检查）。
    ///
    /// 与 `mixer.rs` 的 `engine_mixer_module_has_no_second_limiter_implementation`
    /// 同款。记号用 `concat!` 拼出来，避免判据自己的字面量命中自己。
    ///
    /// 注入：把 `yeban_dsp::compressor` 的**类型定义**复制进本文件（`struct` + 类型名），
    /// 或在本文件里新增一个 `process_mono` 方法 ⇒ 本判据立即变红。
    /// ⚠ 源码级检查对**本判据自己的注释**也生效：注释里不许出现被禁的字面量
    /// （第一版就是这样自我命中，实测变红后改写了本条注释）。
    #[test]
    fn engine_insert_module_has_no_second_compressor_implementation() {
        let source = include_str!("insert.rs");
        let forbidden = [
            concat!("struct", " Compressor", " {"),
            concat!("impl", " Compressor"),
            concat!("struct", " CompressorParams", " {"),
            concat!("impl", " CompressorParams"),
            concat!("fn", " process_mono("),
            concat!("fn", " gain_db_for("),
            concat!("fn", " output_db_for("),
            concat!("const", " DEFAULT_DETECTOR_S"),
            concat!("const", " POWER_FLOOR"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "engine 的 insert.rs 里出现了实现记号 `{needle}` —— 这里只允许投影 + `pub use`"
            );
        }
        assert!(
            source.contains("pub use yeban_dsp::compressor::{Compressor, CompressorParams};"),
            "engine 的 insert.rs 必须是 dsp 压缩器的再导出"
        );
    }
}
