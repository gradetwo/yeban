//! `yeban_auto_align_tracks` —— 自动相位对齐（手动相位对齐第二层）。
//!
//! 手动层（`setTrackDelay` / `setTrackPolarity` / Sample Delay / 片段偏移）
//! 只给“把一条轨推后 N 采样”的执行机构，不回答“该推多少”。本工具回答后者：
//! 对基准轨与目标轨各 solo 渲染一遍，用互相关量出目标轨相对基准轨的到达
//! 时差，再把补偿写成 `Op::SetTrackDelay`（+ 极性反转时
//! `Op::SetTrackPolarity`），走提案路径（`Plan::PhaseAlignAuto`，
//! `OtcpDeliver` 同款：`data` + 可选 `draft`）。
//!
//! ## 设计来源
//!
//! Logic Pro **没有**自动互相关对齐功能（只有轨延时与 Sample Delay 插件，
//! 手动层已照抄），因此测量设计不抄 Logic，抄常见自动对齐插件的做法
//! （Sound Radix Auto-Align 式）：归一化互相关 + 峰值搜索 + 抛物线插值
//! （实现在 `yeban-dsp` 的 `phase_align_auto`，渲染入口在 `yeban-render`
//! 的 `phase_align_auto`）。补偿侧抄 Logic 的轨延时语义（正值 = 推后）。
//!
//! ## 架构落点：`PhaseAlignAutoProvider` trait + 宿主注入
//!
//! 与 `get_song_overview` 的 `SongOverviewProvider` 同一条纪律：分析住在
//! `yeban-dsp`，`yeban-mcp` **不**依赖 `yeban-dsp`（守卫
//! `check_mcp_dependency_direction.py`），真实实现由 `yeban-app` 在挂载时
//! 注入；未注入 ⇒ `CONFLICT` + `providerUnavailable`，⛔ 不返回假测量。
//!
//! ## 符号约定与补偿规则
//!
//! `lag_samples > 0` = 目标轨比基准轨**晚到** `lag_samples` 采样。补偿只延时
//! **先到的那条轨**（正延时，不用前瞻）：`lag > 0` 延基准轨 `+lag`，
//! `lag < 0` 延目标轨 `|lag|`；新值 = 旧值 + 补偿，超 ±4800 采样
//! （`PHASE_ALIGN_MAX_ABS_SAMPLES`）响亮拒绝（`compensationOutOfRange`），
//! 不部分施加。极性反转检出时翻转**目标轨**的 `invert_polarity`。
//! `lag == 0` 且无反转时无事可做：即使 `apply: true` 也不建空提案。
//!
//! ## 参数与失败语义
//!
//! - `referenceTrackId` / `targetTrackId`：必填 ULID；不存在 ⇒
//!   `TRACK_NOT_FOUND`；两者相同 ⇒ `INVALID_PARAMETER_RANGE`
//!   （`reason: "sameReferenceAndTarget"`）。
//! - `maxLagMs`：可选，缺省 50；须有限且 > 0，换算采样后 ≤ 4800，否则
//!   `INVALID_PARAMETER_RANGE`（`reason: "maxLagOutOfRange"`，带换算值与上限）。
//! - `apply`：可选，缺省 `true`；`false` = 只测量、不建提案。
//! - `autoMerge` / `mergeCommitMessage` / `includeOps`：与提案工具同一口径。
//! - 测量相关系数 |ρ| < 0.3 时拒绝施加（`CONFLICT` + `lowCorrelation`）；
//!   只测量（`apply: false`）时如实返回低相关系数，不拒绝。
//! - 没有活跃工程 ⇒ `NO_ACTIVE_PROJECT`；宿主没有注入 provider ⇒
//!   `CONFLICT` + `providerUnavailable`。
//! - `dryRun`：只校验参数与存在性，不调 provider、不测量、不建提案，
//!   返回 `{wouldMeasure: true, …}` 预览。
//!
//! ## 校验顺序
//!
//! 实参类型 → `maxLagMs` 值域 → 两轨存在性 → 两轨相同否 → `dryRun` 短路
//! → provider → 测量 → 相关系数门 → 补偿值域。顺序是承诺：`dryRun` 仍会
//! 报出参数与存在性错误，但不花一次渲染。

use serde_json::Value;

use yeban_model::music::PHASE_ALIGN_MAX_ABS_SAMPLES;
use yeban_model::{EntityId, Op, YebanProjectV1};

use crate::tools::{ErrorCode, ToolCall, ToolResponse};

use super::error::Fault;
use super::{
    Domain, ProposalDraft, arg_bool, arg_str, include_ops, parse_optional_entity_id, require_active,
};

/// 宿主没有注入 provider 时的 `data.reason`（与 `song_overview` 同一个串）。
pub const PROVIDER_UNAVAILABLE_REASON: &str = super::song_overview::PROVIDER_UNAVAILABLE_REASON;

/// `maxLagMs` 缺省值（毫秒）：50 ms @ 48 kHz = 2400 采样，覆盖常见话筒间距
/// （鼓组 overhead 与近场话筒的到达差一般 < 20 ms），又留一半裕量给
/// ±4800 采样的补偿窗。
pub const DEFAULT_MAX_LAG_MS: f64 = 50.0;

/// 施加补偿的相关系数下界（与 `yeban-dsp` 的 `APPLY_MIN_CORRELATION` 同值；
/// 两处各守一端：dsp 只报告相关系数，是否施加由本层决定）。
pub const APPLY_MIN_CORRELATION: f32 = 0.3;

/// 测量结果的置信度分档（按 |ρ|，与 dsp 的 `Confidence` 同值域）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MeasurementConfidence {
    /// |ρ| ≥ 0.7：峰值清晰，可放心施加。
    High,
    /// 0.4 ≤ |ρ| < 0.7：峰值可辨，施加前宜人耳复核。
    Medium,
    /// |ρ| < 0.4：峰值不可信，不应施加。
    Low,
}

impl MeasurementConfidence {
    /// 按 |ρ| 划档（阈值与 dsp 的 `Confidence::from_correlation` 同值）。
    #[must_use]
    pub fn from_correlation(correlation: f32) -> Self {
        let magnitude = correlation.abs();
        if magnitude >= 0.7 {
            Self::High
        } else if magnitude >= 0.4 {
            Self::Medium
        } else {
            Self::Low
        }
    }
}

/// 一次两轨相位测量的结果（provider 的返回值；响应 `data` 的测量部分）。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct PhaseDelayMeasurement {
    /// 目标轨相对基准轨的到达时差（采样，亚采样插值；> 0 = 目标轨晚到）。
    pub lag_samples: f64,
    /// `lag_samples` 四舍五入到整数采样（补偿真正写入的值）。
    pub lag_samples_rounded: i32,
    /// 峰值归一化相关系数（有符号：负值 = 极性反转）。
    pub correlation: f32,
    /// 两轨极性是否相反（负峰胜出）。
    pub polarity_inverted: bool,
    /// 置信度分档（按 |ρ|）。
    pub confidence: MeasurementConfidence,
    /// 分析窗起点（帧，相对渲染起点）。
    pub window_start_frame: usize,
    /// 分析窗长度（帧）。
    pub window_frames: usize,
    /// 渲染采样率（Hz）。
    pub sample_rate_hz: u32,
}

impl PhaseDelayMeasurement {
    /// 零时差、高相关的定值测量（stub 与夹具用）。
    #[must_use]
    pub const fn zero_aligned(sample_rate_hz: u32) -> Self {
        Self {
            lag_samples: 0.0,
            lag_samples_rounded: 0,
            correlation: 1.0,
            polarity_inverted: false,
            confidence: MeasurementConfidence::High,
            window_start_frame: 0,
            window_frames: 0,
            sample_rate_hz,
        }
    }
}

/// 两轨相位测量的**唯一**入口。
///
/// 实现方（`yeban-app` 的真实 provider，或测试里的 stub）只做一件事：对
/// 当前活跃工程里的两条轨做一次测量。`Debug` 是 trait 约束的一部分 ——
/// [`Domain`] 派生 `Debug`，provider 以 `Arc<dyn PhaseAlignAutoProvider>`
/// 住在会话运行态里。
pub trait PhaseAlignAutoProvider: std::fmt::Debug + Send + Sync {
    /// 自述名（进响应 `data.provider`，供调用方与判据区分 stub 与真实实现）。
    fn name(&self) -> &'static str;

    /// 对给定的活跃工程测两轨的到达时差。
    ///
    /// `project` 是 [`Domain`] 当前的活跃工程（调用那一刻的版本，见
    /// `song_overview` 模块头的接线口径）—— provider 不缓存它、不回头读它。
    /// `max_lag_samples` 是搜索窗（±，采样；调用方保证 ≤ 4800）。
    ///
    /// # Errors
    ///
    /// 测量不可用（轨不是源节点 / 任一轨静音 / 信号太短）时响亮失败，
    /// ⛔ 不返回假 lag。
    fn measure(
        &self,
        project: &YebanProjectV1,
        reference: EntityId,
        target: EntityId,
        max_lag_samples: usize,
    ) -> Result<PhaseDelayMeasurement, Fault>;
}

/// 定值 stub provider（测试用：不渲染、不读工程，只返回构造时给定的测量）。
#[derive(Debug)]
pub struct StubPhaseAlignAutoProvider {
    /// 每次 `measure` 返回的测量（与轨 id 无关）。
    measurement: PhaseDelayMeasurement,
}

impl StubPhaseAlignAutoProvider {
    /// 造一个返回给定测量的 stub。
    #[must_use]
    pub const fn new(measurement: PhaseDelayMeasurement) -> Self {
        Self { measurement }
    }

    /// 零时差 stub（lag 0、ρ = 1、不反转）。
    #[must_use]
    pub const fn zero_aligned(sample_rate_hz: u32) -> Self {
        Self::new(PhaseDelayMeasurement::zero_aligned(sample_rate_hz))
    }
}

impl PhaseAlignAutoProvider for StubPhaseAlignAutoProvider {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn measure(
        &self,
        _project: &YebanProjectV1,
        _reference: EntityId,
        _target: EntityId,
        _max_lag_samples: usize,
    ) -> Result<PhaseDelayMeasurement, Fault> {
        Ok(self.measurement)
    }
}

/// 必填 ULID 实参（缺键 / 非字符串 / 非 ULID 都 `INVALID_PARAMETER_RANGE`）。
fn parse_required_entity_id(call: &ToolCall, key: &str) -> Result<EntityId, Fault> {
    match parse_optional_entity_id(&call.arguments, key) {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("`{key}` 必填，必须是 26 字符 ULID"),
        )),
        Err(fault) => Err(fault),
    }
}

/// `maxLagMs`：有限且 > 0，换算采样后 ≤ ±4800；返回搜索窗（采样）。
fn parse_max_lag_samples(call: &ToolCall, sample_rate_hz: u32) -> Result<usize, Fault> {
    let value = call
        .arguments
        .get("maxLagMs")
        .and_then(Value::as_f64)
        .unwrap_or(DEFAULT_MAX_LAG_MS);
    if !value.is_finite() || value <= 0.0 {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`maxLagMs` 须为有限正数，实际收到 {value}"),
            serde_json::json!({
                "reason": "maxLagOutOfRange",
                "maxLagMs": value,
            }),
        ));
    }
    let samples = (value * f64::from(sample_rate_hz) / 1000.0).round();
    if samples > f64::from(PHASE_ALIGN_MAX_ABS_SAMPLES) {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`maxLagMs` {value} ms 在 {sample_rate_hz} Hz 下是 {samples} 采样，超出补偿窗 ±{PHASE_ALIGN_MAX_ABS_SAMPLES} 采样"
            ),
            serde_json::json!({
                "reason": "maxLagOutOfRange",
                "maxLagMs": value,
                "maxLagSamples": samples,
                "limitSamples": PHASE_ALIGN_MAX_ABS_SAMPLES,
                "sampleRateHz": sample_rate_hz,
            }),
        ));
    }
    Ok(samples as usize)
}

/// 测量结果的响应 `data`（`applied` / `appliedDelay` / `appliedPolarity` 由
/// 调用方在 plan 里补：`applied == true` ⇔ 本次建了提案）。
fn measurement_data(
    provider_name: &str,
    reference: EntityId,
    target: EntityId,
    max_lag_ms: f64,
    measurement: &PhaseDelayMeasurement,
) -> Value {
    serde_json::json!({
        "referenceTrackId": reference.to_canonical_string(),
        "targetTrackId": target.to_canonical_string(),
        "provider": provider_name,
        "sampleRateHz": measurement.sample_rate_hz,
        "maxLagMs": max_lag_ms,
        "lagSamples": measurement.lag_samples_rounded,
        "lagSamplesFloat": measurement.lag_samples,
        "lagMs": measurement.lag_samples * 1000.0 / f64::from(measurement.sample_rate_hz),
        "correlation": measurement.correlation,
        "polarityInverted": measurement.polarity_inverted,
        "confidence": measurement.confidence,
        "windowStartFrame": measurement.window_start_frame,
        "windowFrames": measurement.window_frames,
    })
}

/// `yeban_auto_align_tracks`。
pub fn plan_auto_align_tracks(domain: &Domain, call: &ToolCall) -> Result<super::Plan, Fault> {
    let project = require_active(domain)?;
    let reference = parse_required_entity_id(call, "referenceTrackId")?;
    let target = parse_required_entity_id(call, "targetTrackId")?;
    let sample_rate_hz = project.audio_config.sample_rate.hz();
    let max_lag_ms = call
        .arguments
        .get("maxLagMs")
        .and_then(Value::as_f64)
        .unwrap_or(DEFAULT_MAX_LAG_MS);
    let max_lag_samples = parse_max_lag_samples(call, sample_rate_hz)?;
    let apply = arg_bool(call, "apply", true);
    let reference_track = project.tracks.get(&reference).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::TrackNotFound,
            format!("基准轨 {reference} 不在工程里"),
            serde_json::json!({
                "reason": "trackNotFound",
                "trackId": reference.to_canonical_string(),
            }),
        )
    })?;
    let target_track = project.tracks.get(&target).ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::TrackNotFound,
            format!("目标轨 {target} 不在工程里"),
            serde_json::json!({
                "reason": "trackNotFound",
                "trackId": target.to_canonical_string(),
            }),
        )
    })?;
    if reference == target {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "`referenceTrackId` 与 `targetTrackId` 不能是同一条轨：自己与自己没有时差可测",
            serde_json::json!({
                "reason": "sameReferenceAndTarget",
                "trackId": reference.to_canonical_string(),
            }),
        ));
    }
    if call.is_dry_run() {
        // dryRun：只校验参数与存在性，不调 provider、不测量、不建提案。
        let mut data = serde_json::json!({
            "wouldMeasure": true,
            "dryRun": true,
            "referenceTrackId": reference.to_canonical_string(),
            "targetTrackId": target.to_canonical_string(),
            "maxLagMs": max_lag_ms,
            "apply": apply,
            "sampleRateHz": sample_rate_hz,
        });
        if let Some(object) = data.as_object_mut() {
            object.insert("applied".to_owned(), Value::from(false));
        }
        return Ok(super::Plan::PhaseAlignAuto {
            data,
            draft: None,
            include_ops: include_ops(call),
            warnings: Vec::new(),
            auto_merge: false,
            merge_commit_message: None,
        });
    }
    let provider = domain.phase_align_auto_provider().ok_or_else(|| {
        Fault::domain_with_data(
            ErrorCode::Conflict,
            "本形态没有相位测量 provider：独立 stdio 进程（形态 B）不托管 yeban-dsp 分析，\
             yeban_auto_align_tracks 无法测量。",
            serde_json::json!({
                "reason": PROVIDER_UNAVAILABLE_REASON,
                "form": "stdio",
                "providerAttached": false,
                "hint": "请在形态 A（yeban-app 的进程内控制面）里调用；\
                         真实 provider 由 yeban-app 挂载时注入",
            }),
        )
    })?;
    let measurement = provider.measure(project, reference, target, max_lag_samples)?;
    let lag = measurement.lag_samples_rounded;
    let mut data = measurement_data(provider.name(), reference, target, max_lag_ms, &measurement);
    let data_object = data.as_object_mut().expect("measurement_data 恒为对象");
    // 无事可做：lag 0 且无反转 ⇒ 即使 apply: true 也不建空提案。
    if lag == 0 && !measurement.polarity_inverted {
        data_object.insert("applied".to_owned(), Value::from(false));
        data_object.insert("appliedDelay".to_owned(), Value::Null);
        data_object.insert("appliedPolarity".to_owned(), Value::Null);
        return Ok(super::Plan::PhaseAlignAuto {
            data,
            draft: None,
            include_ops: include_ops(call),
            warnings: Vec::new(),
            auto_merge: arg_bool(call, "autoMerge", false),
            merge_commit_message: arg_str(call, "mergeCommitMessage").map(str::to_owned),
        });
    }
    if apply && measurement.correlation.abs() < APPLY_MIN_CORRELATION {
        return Err(Fault::domain_with_data(
            ErrorCode::Conflict,
            format!(
                "相关系数 |ρ| = {:.2} < {:.2}，峰值不可信，不施加补偿（apply: false 可只取测量）",
                measurement.correlation.abs(),
                APPLY_MIN_CORRELATION
            ),
            serde_json::json!({
                "reason": "lowCorrelation",
                "correlation": measurement.correlation,
                "threshold": APPLY_MIN_CORRELATION,
                "lagSamples": lag,
            }),
        ));
    }
    // 补偿：只延时先到的那条轨（正延时）。lag > 0 ⇒ 目标轨晚到 ⇒ 延基准轨。
    let (delay_track, delay_track_name, compensation) = if lag > 0 {
        (reference, reference_track.name.clone(), lag)
    } else {
        (target, target_track.name.clone(), -lag)
    };
    let mut ops: Vec<Op> = Vec::new();
    let mut applied_delay = Value::Null;
    if compensation != 0 {
        let old_delay = project
            .tracks
            .get(&delay_track)
            .map(|track| track.delay_samples.unwrap_or(0))
            .unwrap_or(0);
        let new_delay = old_delay + compensation;
        if !(-PHASE_ALIGN_MAX_ABS_SAMPLES..=PHASE_ALIGN_MAX_ABS_SAMPLES).contains(&new_delay) {
            return Err(Fault::domain_with_data(
                ErrorCode::Conflict,
                format!(
                    "补偿后 {delay_track_name} 的轨延时会是 {new_delay} 采样，超出 ±{PHASE_ALIGN_MAX_ABS_SAMPLES} 采样窗"
                ),
                serde_json::json!({
                    "reason": "compensationOutOfRange",
                    "trackId": delay_track.to_canonical_string(),
                    "delaySamplesBefore": old_delay,
                    "compensationSamples": compensation,
                    "delaySamplesAfter": new_delay,
                    "limitSamples": PHASE_ALIGN_MAX_ABS_SAMPLES,
                }),
            ));
        }
        ops.push(Op::SetTrackDelay {
            track_id: delay_track,
            old_delay_samples: old_delay,
            new_delay_samples: new_delay,
        });
        applied_delay = serde_json::json!({
            "trackId": delay_track.to_canonical_string(),
            "delaySamplesBefore": old_delay,
            "delaySamplesAfter": new_delay,
        });
    }
    let mut applied_polarity = Value::Null;
    if measurement.polarity_inverted {
        let old_invert = target_track.invert_polarity.unwrap_or(false);
        ops.push(Op::SetTrackPolarity {
            track_id: target,
            old_invert,
            new_invert: !old_invert,
        });
        applied_polarity = serde_json::json!({
            "trackId": target.to_canonical_string(),
            "invertBefore": old_invert,
            "invertAfter": !old_invert,
        });
    }
    data_object.insert("applied".to_owned(), Value::from(apply));
    data_object.insert("appliedDelay".to_owned(), applied_delay);
    data_object.insert("appliedPolarity".to_owned(), applied_polarity);
    if !apply {
        return Ok(super::Plan::PhaseAlignAuto {
            data,
            draft: None,
            include_ops: include_ops(call),
            warnings: Vec::new(),
            auto_merge: arg_bool(call, "autoMerge", false),
            merge_commit_message: arg_str(call, "mergeCommitMessage").map(str::to_owned),
        });
    }
    // 走提案路径：先由 propose_draft 模拟 + 校验，再把 Propose 变体换成
    // PhaseAlignAuto（测量 data 随身带；apply 时再并进提案响应）。
    let title = format!(
        "自动相位对齐：{} 对 {}",
        target_track.name, reference_track.name
    );
    let mut description = format!(
        "互相关测得 lag {lag} 采样（{:.1} ms），ρ={:.2}，置信度 {}",
        measurement.lag_samples * 1000.0 / f64::from(measurement.sample_rate_hz),
        measurement.correlation,
        match measurement.confidence {
            MeasurementConfidence::High => "high",
            MeasurementConfidence::Medium => "medium",
            MeasurementConfidence::Low => "low",
        },
    );
    if compensation != 0 {
        description.push_str(&format!(
            "；补偿：{delay_track_name} 延时 +{compensation} 采样"
        ));
    }
    if measurement.polarity_inverted {
        description.push_str(&format!("；目标轨 {} 极性翻转", target_track.name));
    }
    let plan = super::propose_draft(domain, project, "notes", title, description, ops, call)?;
    let super::Plan::Propose {
        draft,
        include_ops,
        warnings,
        auto_merge,
        merge_commit_message,
    } = plan
    else {
        unreachable!("propose_draft 恒返回 Plan::Propose");
    };
    Ok(super::Plan::PhaseAlignAuto {
        data,
        draft: Some(draft),
        include_ops,
        warnings,
        auto_merge,
        merge_commit_message,
    })
}

/// `Plan::PhaseAlignAuto` 的施加：有 draft 时先建提案，再把测量数据并进
/// 提案响应；无 draft 时原样返回测量。
pub(crate) fn apply_phase_align_auto(
    domain: &mut Domain,
    data: Value,
    draft: Option<Box<ProposalDraft>>,
    include_ops: bool,
    warnings: &[Value],
    auto_merge: bool,
    merge_commit_message: Option<String>,
) -> Result<ToolResponse, Fault> {
    let Some(draft) = draft else {
        return Ok(ToolResponse::success(data));
    };
    let response = super::apply_propose(
        domain,
        *draft,
        include_ops,
        warnings,
        auto_merge,
        merge_commit_message,
    )?;
    // 提案响应 data 是对象：把测量键并进去（键集合无交集，见模块文档）。
    match response {
        ToolResponse::Success {
            data: proposal_data,
        } => {
            let mut merged = proposal_data;
            if let (Some(target), Value::Object(fields)) = (merged.as_object_mut(), data.clone()) {
                for (key, value) in fields {
                    target.insert(key, value);
                }
            }
            Ok(ToolResponse::success(merged))
        }
        failure => Ok(failure),
    }
}
