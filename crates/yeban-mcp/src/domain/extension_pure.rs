//! 三个扩展工具的**零依赖纯逻辑**（`ADR-0001` **D46**：自动化泳道 / 设备与引擎 / 音频导入）。
//!
//! ## 为什么单独一层
//!
//! 本文件**只依赖 `core` / `std`**：不 import `serde_json`、不 import `yeban_model`。
//! 因此它可以被**裸 `rustc` 独立跑判据**（本机 yeban-mcp 含重依赖 ⇒ `cargo test -p yeban-mcp`
//! 本机跳过，见 `AGENTS.md` §5.2 与 `scripts/dev/heavy-deps.py`）：
//!
//! ```text
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/src/domain/extension_pure.rs -o /tmp/x && /tmp/x
//! ```
//!
//! 与 `section_build.rs` / `render_math.rs` / `render_clip_math.rs` 同一条纪律：把
//! "**能纯函数表达的判断**"从"需要 I/O 与领域状态的接线"里切出来，于是它在本机就是
//! **真跑过的**，而不是"等 CI 编译完才知道"。
//!
//! ## 这一层判定什么（都是"参数形状"，不是领域合法性）
//!
//! | 纯函数 | 判定 |
//! | :--- | :--- |
//! | [`LaneKind::parse`] | `lane` 实参是否是一个**存在**的目标变体（拼错即拒绝，不静默取默认） |
//! | [`LaneKind::needs_edge_id`] 等 | 该目标**必须**携带哪个附加实参（`SendGain` ⇒ `edgeId`） |
//! | [`target_label`] / [`point_label`] / [`clip_label`] | 确定性身份标签：同一请求 ⇒ 同一 `EntityId`（`dryRun` 预览与真做逐字节一致的前提） |
//! | [`select_import_source`] | `assetHash` / `path` **恰好给一个**（两个都不给或都给都是非法形状） |
//! | [`canonical_curve`] | `curve` 实参是否是模型 `CurveType` 的**规范名**（不发明别名） |
//!
//! 领域合法性（音轨是否存在、参数下标是否越界、值是否在取值域内、预算是否超限）**不在**这里 ——
//! 那些由 `yeban-model` / `yeban-decode` 的既有入口判定，本层绝不复制第二份。
//!
//! ## 词汇表**只有一份**
//!
//! 所有名字都与 `project.json` 里 serde 写出的变体名**逐字相同**
//! （`TrackVolume` / `DeviceParam` / `Linear` / `SCurve`…）。刻意**不**接受
//! `trackVolume` / `linear` 这类"友好别名"：那会立刻产生第二份词汇表，而
//! `yeban_query_project` 读回来的 JSON 用的是这一份。

/// 自动化目标变体（与 `yeban_model::AutomationTarget` 的变体名**逐字相同**）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LaneKind {
    /// `AutomationTarget::TrackVolume`：音轨音量（只按 `trackId` 寻址）。
    TrackVolume,
    /// `AutomationTarget::TrackPan`：音轨声相（只按 `trackId` 寻址）。
    TrackPan,
    /// `AutomationTarget::SendGain`：发送增益（额外需要 `edgeId`）。
    SendGain,
    /// `AutomationTarget::DeviceParam`：设备参数（额外需要 `slotIndex` / `paramIndex`）。
    DeviceParam,
    /// `AutomationTarget::Macro`：宏（额外需要 `macroIndex`）。
    Macro,
}

/// 全部目标变体名，**规范顺序**（与 `AutomationTarget` 的声明顺序一致）。
pub const LANE_NAMES: [&str; 5] = [
    "TrackVolume",
    "TrackPan",
    "SendGain",
    "DeviceParam",
    "Macro",
];

impl LaneKind {
    /// 解析 `lane` 实参；未知名字返回 `None`（调用方负责给出契约错误码，**绝不**回退到默认）。
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "TrackVolume" => Some(Self::TrackVolume),
            "TrackPan" => Some(Self::TrackPan),
            "SendGain" => Some(Self::SendGain),
            "DeviceParam" => Some(Self::DeviceParam),
            "Macro" => Some(Self::Macro),
            _ => None,
        }
    }

    /// 规范名（解析的逆）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrackVolume => "TrackVolume",
            Self::TrackPan => "TrackPan",
            Self::SendGain => "SendGain",
            Self::DeviceParam => "DeviceParam",
            Self::Macro => "Macro",
        }
    }

    /// 该目标是否**必须**携带 `edgeId`（发送增益寻址的是路由边，不是音轨）。
    #[must_use]
    pub const fn needs_edge_id(self) -> bool {
        matches!(self, Self::SendGain)
    }

    /// 该目标是否**必须**携带 `slotIndex` / `paramIndex`。
    #[must_use]
    pub const fn needs_device_slot(self) -> bool {
        matches!(self, Self::DeviceParam)
    }

    /// 该目标是否**必须**携带 `macroIndex`。
    #[must_use]
    pub const fn needs_macro_index(self) -> bool {
        matches!(self, Self::Macro)
    }
}

/// 目标的确定性标签（点身份的派生输入；同一目标 ⇒ 同一标签 ⇒ 同一点身份）。
///
/// 只把**该变体实际使用的**分量写进标签：`TrackVolume` 的标签里不会出现
/// `slotIndex`，否则"同一个目标用两种写法传实参"会派生出两个不同的点身份。
#[must_use]
pub fn target_label(
    kind: LaneKind,
    track_id: &str,
    edge_id: Option<&str>,
    slot_index: u64,
    param_index: u64,
    macro_index: u64,
) -> String {
    match kind {
        // `format!("{track_id}")`（单变量、无其他文本）会被 clippy 判成 `useless_format`。
        LaneKind::TrackVolume | LaneKind::TrackPan => track_id.to_string(),
        LaneKind::SendGain => format!("{track_id}->{}", edge_id.unwrap_or("")),
        LaneKind::DeviceParam => format!("{track_id}#{slot_index}:{param_index}"),
        LaneKind::Macro => format!("{track_id}@{macro_index}"),
    }
}

/// 自动化的**唯一求值入口名**（审计与响应都引用这一个常量，避免两处硬编码漂移）。
pub const AUTOMATION_ENTRY: &str = "automation_value_at";

/// 一个自动化点的确定性标签：`(目标, tick)` ⇒ 同一 tick 重复写入**更新同一个点**。
///
/// 这是"重复导入/重复设点不产生第二条点"的机械来源，也是 `dryRun` 的预览身份与
/// 真做身份一致的原因（见 `ids.rs`）。
#[must_use]
pub fn point_label(target_label: &str, tick: u64) -> String {
    format!("automation-point:{target_label}:{tick}")
}

/// `curve` 实参的规范名全集（`yeban_model::CurveType` 的四个变体，**逐字相同**）。
pub const CURVE_NAMES: [&str; 4] = ["Linear", "Exponential", "Logarithmic", "SCurve"];

/// `curve` 实参 → 规范名；未知/别名写法返回 `None`。
#[must_use]
pub fn canonical_curve(text: &str) -> Option<&'static str> {
    CURVE_NAMES.iter().copied().find(|name| *name == text)
}

/// 音频导入的来源（**恰好二选一**）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSource {
    /// 资产**已经在**会话 CAS 池里（`assets/{sha256}`），按哈希引用。
    AssetPool,
    /// 磁盘上的一个文件路径，需要解码并登记进 CAS 池。
    DiskPath,
}

/// 来源选择失败（参数形状不合法，映射到 `INVALID_PARAMETER_RANGE`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceSelectionError {
    /// `assetHash` 与 `path` 都没给：不知道要导入什么。
    Neither,
    /// 两个都给了：无法判断以哪一份字节为准（把它们当成"互为校验"是**猜**，不是契约）。
    Both,
}

impl SourceSelectionError {
    /// 稳定的机器可读原因名（响应 `data.reason`）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neither => "sourceMissing",
            Self::Both => "sourceAmbiguous",
        }
    }

    /// 人话说明。
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Neither => "`assetHash` 与 `path` 必须**恰好**给一个（两个都没给）",
            Self::Both => "`assetHash` 与 `path` 必须**恰好**给一个（两个都给了）",
        }
    }
}

/// 由两个可选实参**唯一**确定导入来源。
///
/// # Errors
///
/// 两个都没给、或两个都给了（见 [`SourceSelectionError`]）。
pub fn select_import_source(
    asset_hash: Option<&str>,
    path: Option<&str>,
) -> Result<ImportSource, SourceSelectionError> {
    match (asset_hash, path) {
        (Some(_), None) => Ok(ImportSource::AssetPool),
        (None, Some(_)) => Ok(ImportSource::DiskPath),
        (None, None) => Err(SourceSelectionError::Neither),
        (Some(_), Some(_)) => Err(SourceSelectionError::Both),
    }
}

/// 一个导入片段的确定性标签：`(来源, 名字, 增益)` ⇒ 同一请求 ⇒ 同一片段身份。
///
/// 增益进标签是因为"同一个文件、同一个名字、不同增益"是**两条不同的片段意图**；
/// 但重复导入**同一意图**必须命中同一条记录（因此它是参数的纯函数）。
#[must_use]
pub fn clip_label(source: &str, name: &str, gain_db: f32) -> String {
    // `f32` 的十进制写法在 Rust 里是确定性的最短往返表示，因此标签逐字节稳定。
    format!("audio-clip:{source}:{name}:{gain_db}")
}

/// 音轨身份之外的**可选**寻址分量（`deviceParam` / `macro` 用），默认 0。
#[must_use]
pub fn optional_index(raw: Option<u64>) -> u64 {
    raw.unwrap_or(0)
}

/// 一次自动化编辑请求里"要读哪些 tick"（空数组按"没要求读"处理）。
///
/// 返回的 tick 序列**保持调用方顺序**：响应里的 `values` 因此与实参一一对应，
/// 调用方不需要自己对齐（排序会让"第 i 个值对应哪个 tick"变成一个隐式约定）。
#[must_use]
pub fn read_ticks(raw: &[u64]) -> Vec<u64> {
    raw.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_names_round_trip_and_reject_aliases() {
        for name in LANE_NAMES {
            let kind = LaneKind::parse(name).unwrap_or_else(|| panic!("{name} 必须可解析"));
            assert_eq!(kind.as_str(), name, "解析与写回必须互逆");
        }
        // 别名一律拒绝: 静默接受 `trackVolume` 会造出第二份词汇表。
        for alias in [
            "trackVolume",
            "track_volume",
            "volume",
            "",
            "Trackvolume",
            "DeviceParams",
        ] {
            assert!(LaneKind::parse(alias).is_none(), "别名必须被拒绝: {alias}");
        }
    }

    #[test]
    fn lane_kind_requirements_are_explicit() {
        assert!(!LaneKind::TrackVolume.needs_edge_id());
        assert!(!LaneKind::TrackVolume.needs_device_slot());
        assert!(!LaneKind::TrackVolume.needs_macro_index());
        assert!(LaneKind::SendGain.needs_edge_id());
        assert!(LaneKind::DeviceParam.needs_device_slot());
        assert!(LaneKind::Macro.needs_macro_index());
        // 恰好三种"需要附加寻址"的变体, 且互不重叠。
        let extras: Vec<&str> = LANE_NAMES
            .iter()
            .copied()
            .filter(|name| {
                let kind = LaneKind::parse(name).expect("已知变体");
                kind.needs_edge_id() || kind.needs_device_slot() || kind.needs_macro_index()
            })
            .collect();
        assert_eq!(extras, vec!["SendGain", "DeviceParam", "Macro"]);
    }

    #[test]
    fn target_labels_only_use_the_components_the_variant_addresses() {
        let volume = target_label(LaneKind::TrackVolume, "T1", None, 3, 4, 5);
        assert_eq!(volume, "T1", "音量泳道不该把 slot/macro 写进身份");
        let pan = target_label(LaneKind::TrackPan, "T1", None, 3, 4, 5);
        assert_eq!(pan, volume, "两种推子类目标按同一音轨寻址");
        assert_eq!(
            target_label(LaneKind::SendGain, "T1", Some("E9"), 0, 0, 0),
            "T1->E9"
        );
        assert_eq!(
            target_label(LaneKind::DeviceParam, "T1", None, 1, 2, 0),
            "T1#1:2"
        );
        assert_eq!(target_label(LaneKind::Macro, "T1", None, 0, 0, 7), "T1@7");
        // 设备参数的两个下标都必须进身份, 否则 (1,2) 与 (2,1) 会撞成同一个点。
        assert_ne!(
            target_label(LaneKind::DeviceParam, "T1", None, 1, 2, 0),
            target_label(LaneKind::DeviceParam, "T1", None, 2, 1, 0)
        );
        // 缺 edgeId 的 SendGain 标签仍是确定的（空串占位），不会 panic。
        assert_eq!(
            target_label(LaneKind::SendGain, "T1", None, 0, 0, 0),
            "T1->"
        );
    }

    #[test]
    fn point_label_is_stable_and_tick_sensitive() {
        let target = target_label(LaneKind::TrackVolume, "T1", None, 0, 0, 0);
        assert_eq!(point_label(&target, 960), point_label(&target, 960));
        assert_ne!(point_label(&target, 960), point_label(&target, 961));
        assert_eq!(point_label(&target, 0), "automation-point:T1:0");
    }

    #[test]
    fn curve_names_are_the_model_vocabulary_only() {
        assert_eq!(CURVE_NAMES.len(), 4);
        for name in CURVE_NAMES {
            assert_eq!(canonical_curve(name), Some(name));
        }
        for alias in ["linear", "sCurve", "s_curve", "SCurve ", ""] {
            assert!(canonical_curve(alias).is_none(), "别名必须被拒绝: {alias}");
        }
    }

    #[test]
    fn import_source_must_be_exactly_one_of_two() {
        assert_eq!(
            select_import_source(Some("ab"), None),
            Ok(ImportSource::AssetPool)
        );
        assert_eq!(
            select_import_source(None, Some("/tmp/a.wav")),
            Ok(ImportSource::DiskPath)
        );
        assert_eq!(
            select_import_source(None, None),
            Err(SourceSelectionError::Neither)
        );
        assert_eq!(
            select_import_source(Some("ab"), Some("/tmp/a.wav")),
            Err(SourceSelectionError::Both)
        );
        assert_eq!(SourceSelectionError::Neither.as_str(), "sourceMissing");
        assert_eq!(SourceSelectionError::Both.as_str(), "sourceAmbiguous");
        assert!(!SourceSelectionError::Neither.message().is_empty());
    }

    #[test]
    fn clip_label_is_a_pure_function_of_its_inputs() {
        assert_eq!(
            clip_label("disk:/tmp/a.wav", "Kick", 0.0),
            clip_label("disk:/tmp/a.wav", "Kick", 0.0)
        );
        assert_ne!(
            clip_label("disk:/tmp/a.wav", "Kick", 0.0),
            clip_label("disk:/tmp/a.wav", "Kick", -1.5),
            "增益是片段意图的一部分"
        );
        assert_ne!(
            clip_label("asset:ab", "Kick", 0.0),
            clip_label("disk:/tmp/a.wav", "Kick", 0.0)
        );
    }

    #[test]
    fn optional_index_and_read_ticks_preserve_the_caller_contract() {
        assert_eq!(optional_index(None), 0);
        assert_eq!(optional_index(Some(0)), 0);
        assert_eq!(optional_index(Some(9)), 9);
        // 读 tick 顺序必须保持（不排序）: 否则 "第 i 个值" 与实参的对应关系变成隐式约定。
        assert_eq!(read_ticks(&[3840, 0, 960]), vec![3840, 0, 960]);
        assert!(read_ticks(&[]).is_empty());
    }

    #[test]
    fn automation_entry_name_is_the_model_symbol() {
        // 审计模块与本文件共用同一个常量; 这里只钉住它不是随手写的字符串。
        assert_eq!(AUTOMATION_ENTRY, "automation_value_at");
        assert!(!AUTOMATION_ENTRY.contains("?"));
    }
}
