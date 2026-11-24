//! 插件与 ARA 面板的**纯 Rust 侧**（GUI 补齐批 2，T-FIX-83）。
//!
//! 本模块只做三件事：
//!
//! 1. 把 `yeban_plugin_host` 的扫描 / 验证加载 / ARA 会话包成 GUI 可消费的
//!    结果行 —— 扫描失败是**行上的数据**，不是把整个面板打断；
//! 2. 把 MCP 工具现有的**校验口径与错误原文**原样接过来（路径准入门、
//!    `kind` 词表、`tempoBpm` / `loopIterations` / `blockFrames` 值域），
//!    ⛔ 不另造第二套规则；
//! 3. 保存 ARA 会话 registry（最多 64 条，与 MCP 同一个上限）。
//!
//! 界面侧只读本模块产出的行文本；Slint 写属性全在 [`crate::host::wire_plugin_panel`]。
//! 本模块不碰 Slint、不碰工程文件；插件装载的 `Op::InsertDevice` 由宿主提交。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_decode::{DecodeOptions, decode_path};
use yeban_model::{AssetHash, DeviceDefinition, DeviceKind, EntityId, PluginBinding};
use yeban_plugin_host::ara::{AraClip, AraClipKind, AraPlacement, AraSession};
use yeban_plugin_host::{Host, PluginInstance, PluginKind, PluginState};

use crate::save::write_file_atomically;

/// ARA 会话同时存活的上限；与 MCP `MAX_ARA_SESSIONS` 同一个数。
pub const MAX_ARA_SESSIONS: usize = 64;

/// 插件装载的执行口径（T-FIX-338）：装载只做验证并写设备定义（“仅登记”）。
///
/// 实时播放不执行插件（实时回调内禁分配／锁／IO，见 AGENTS.md §2 红线 7，
/// 引擎没有插件实例的无锁生命周期）；离线导出与整曲预览执行首台已装载的
/// 插件（与 MCP 离线渲染同一套装配）。本常量是面板提示的**唯一**来源：
/// 打开面板时与装载成功后都显示它，⛔ 不在 Slint 里另写一份（两份迟早漂移）。
pub const PLUGIN_REGISTRATION_NOTE: &str =
    "仅登记：实时播放不执行该插件；离线导出与整曲预览执行首台已装载的插件。";

/// 扫描结果的一行：一个类一行；模块级失败时类名为空单独一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRow {
    /// 模块路径。
    pub path: String,
    /// 厂商名（读不到为空串）。
    pub vendor: String,
    /// 类名；模块级失败时为空串。
    pub class_name: String,
    /// 类标识（十六进制；模块级失败时为空串）。
    pub class_id: String,
    /// `instrument` / `effect` / `unknown`。
    pub kind: String,
    /// 该类探针是否可用。
    pub usable: bool,
    /// 失败原因原文；可用时为 `None`。
    pub error: Option<String>,
    /// 模块是否带 ARA factory。
    pub ara: bool,
}

impl PluginRow {
    /// 行标题：类名优先，模块级失败时退到文件名。
    #[must_use]
    pub fn title(&self) -> String {
        if self.class_name.is_empty() {
            Path::new(&self.path).file_name().map_or_else(
                || self.path.clone(),
                |name| name.to_string_lossy().into_owned(),
            )
        } else {
            self.class_name.clone()
        }
    }

    /// 行副标题：厂商 / 类别 / ARA 能力。
    #[must_use]
    pub fn meta(&self) -> String {
        let mut parts = Vec::new();
        if !self.vendor.is_empty() {
            parts.push(self.vendor.clone());
        }
        parts.push(match self.kind.as_str() {
            "instrument" => "乐器".to_owned(),
            "effect" => "效果器".to_owned(),
            _ => "类型未知".to_owned(),
        });
        if self.ara {
            parts.push("ARA".to_owned());
        }
        parts.join(" · ")
    }

    /// 行状态：可用 / 失败原文。
    #[must_use]
    pub fn state(&self) -> String {
        if self.usable {
            "可用".to_owned()
        } else {
            format!(
                "失败: {}",
                self.error
                    .as_deref()
                    .unwrap_or("没有失败原因（探针未给出）")
            )
        }
    }
}

/// 一次扫描的汇总。
#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    /// 逐类行。
    pub rows: Vec<PluginRow>,
    /// 可用行数。
    pub usable: usize,
    /// 失败行数。
    pub failed: usize,
}

/// 扫描本机插件；`extra_paths` 是 GUI 的“额外目录”文本，多个用 `;` 分隔。
///
/// 不存在的目录静默跳过（与 [`Host::scan`] / MCP `yeban_scan_plugins` 同口径）。
#[must_use]
pub fn scan(extra_paths: &str) -> ScanReport {
    let mut paths = Host::default_search_paths();
    for piece in extra_paths.split(';') {
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            paths.push(PathBuf::from(trimmed));
        }
    }
    scan_paths(&paths)
}

/// 扫描**给定路径集合**，不叠加 [`Host::default_search_paths`]。
///
/// 生产入口 [`scan`] 负责合并默认搜索路径与 GUI 的额外目录；单元测试用本函数
/// 只扫临时目录，避免开发机默认路径下的真插件把行数断言污染成偶发失败。
fn scan_paths(paths: &[PathBuf]) -> ScanReport {
    let descriptors = Host::scan(paths);
    let mut rows = Vec::new();
    for descriptor in descriptors {
        if descriptor.classes.is_empty() {
            rows.push(PluginRow {
                path: descriptor.path,
                vendor: descriptor.vendor,
                class_name: String::new(),
                class_id: String::new(),
                kind: "unknown".to_owned(),
                usable: false,
                error: descriptor.error,
                ara: descriptor.ara.is_some(),
            });
            continue;
        }
        for class in descriptor.classes {
            rows.push(PluginRow {
                path: descriptor.path.clone(),
                vendor: descriptor.vendor.clone(),
                class_name: class.name,
                class_id: tuid_hex(&class.id),
                kind: match class.kind {
                    PluginKind::Instrument => "instrument",
                    PluginKind::Effect => "effect",
                    PluginKind::Unknown => "unknown",
                }
                .to_owned(),
                usable: class.error.is_none(),
                error: class.error,
                ara: descriptor.ara.is_some(),
            });
        }
    }
    let usable = rows.iter().filter(|row| row.usable).count();
    let failed = rows.len() - usable;
    ScanReport {
        rows,
        usable,
        failed,
    }
}

/// 插件路径准入门：与 MCP `guard_plugin_path` 同一条 [`Host::validate_plugin_path`]。
///
/// # Errors
///
/// 形态不对、路径不存在或不在任何插件根内；返回的字符串以
/// `插件路径校验失败，拒绝加载:` 开头（与 MCP 原文一致）。
pub fn guard_plugin_path(path: &str) -> Result<(), String> {
    Host::validate_plugin_path(path)
        .map(|_| ())
        .map_err(|error| format!("插件路径校验失败，拒绝加载: {error}"))
}

/// JSFX 选择入口的路径门（GUI 专用；MCP 不接 VST3，见 T-FIX-362 的关单裁决）。
///
/// 规则：必须存在、必须是文件、后缀必须是 `.jsfx`（大小写不敏感）；通过后返回
/// **规范化绝对路径**（ysfx 的 `path` 属性要求完整路径）。
///
/// # Errors
///
/// 不存在 / 不是文件 / 后缀不对 / 规范化失败 ⇒ 带原文的响亮错误。
pub fn guard_jsfx_path(path: &str) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("JSFX 路径为空：请选择一个 .jsfx 文件".to_owned());
    }
    let candidate = Path::new(path);
    if !candidate.exists() {
        return Err(format!("JSFX 文件不存在: {path}"));
    }
    if !candidate.is_file() {
        return Err(format!("JSFX 路径不是文件: {path}"));
    }
    let is_jsfx = candidate
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsfx"));
    if !is_jsfx {
        return Err(format!("不是 .jsfx 文件: {path}"));
    }
    std::fs::canonicalize(candidate)
        .map(|absolute| absolute.to_string_lossy().into_owned())
        .map_err(|error| format!("JSFX 路径规范化失败: {error}"))
}

/// 写一个 JUCE `ValueTree` 字符串：UTF-8 后跟 NUL（无长度前缀）。
fn vt_write_string(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
    out.push(0);
}

/// JUCE `OutputStream::writeCompressedInt`（非负值；7 字节上限内）。
///
/// 低字节在前，首字节是字节数（负数会置 0x80；本处只写非负值）。
fn vt_write_compressed_int(out: &mut Vec<u8>, value: i32) {
    let mut un = value.unsigned_abs();
    let mut data = [0u8; 5];
    let mut num = 0usize;
    while un > 0 && num < 4 {
        num += 1;
        data[num] = (un & 0xFF) as u8;
        un >>= 8;
    }
    let mut first = num as u8;
    if value < 0 {
        first |= 0x80;
    }
    out.push(first);
    out.extend_from_slice(&data[1..=num]);
}

/// JUCE `var` 的整数编码：长度 5（压缩串 + marker + int32），小端 `int32`。
fn vt_write_var_int(out: &mut Vec<u8>, value: i32) {
    vt_write_compressed_int(out, 5);
    out.push(1); // varMarker_Int
    out.extend_from_slice(&value.to_le_bytes());
}

/// JUCE `var` 的字符串编码：压缩长度 = UTF-8 字节数 + 2（含 NUL 与 marker），
/// marker 5，随后 UTF-8 与 NUL。
fn vt_write_var_string(out: &mut Vec<u8>, text: &str) {
    let len = text.len() + 1;
    vt_write_compressed_int(out, i32::try_from(len + 1).unwrap_or(i32::MAX));
    out.push(5); // varMarker_String
    vt_write_string(out, text);
}

/// 构造 ysfx 的最小 VST3 状态：`ValueTree("ysfx")` 只带 `version=1` 与 `path`。
///
/// 这正是 ysfx `setStateInformation` 认的格式（`plugin/processor.cpp`）：
/// 没有 `state` 子节点时按路径重新加载 JSFX 本体，滑块取默认值。
#[must_use]
pub fn ysfx_state_blob(jsfx_path: &str) -> Vec<u8> {
    let mut out = Vec::new();
    vt_write_string(&mut out, "ysfx");
    vt_write_compressed_int(&mut out, 2); // 两个属性：version / path
    vt_write_string(&mut out, "version");
    vt_write_var_int(&mut out, 1);
    vt_write_string(&mut out, "path");
    vt_write_var_string(&mut out, jsfx_path);
    vt_write_compressed_int(&mut out, 0); // 没有子节点
    out
}

/// 装载 ysfx 并把所选 JSFX 以 VST3 `setState` 注入，返回**规范状态**的设备定义。
///
/// 只有 GUI 侧会调用；MCP 的 `yeban_load_plugin` 保持 `state: Vec::new()` 不动
/// （T-FIX-362 已关单，MCP 不接 VST3）。注入后立刻 `save_state`，把插件自己写回的
/// 状态（而不是我们手搓的 blob）落进 [`PluginBinding::state`]，保证重载一致。
///
/// # Errors
///
/// 路径门失败、所选插件不是 ysfx、`setState`/`getState` 失败；⛔ 不静默回退到空宿主。
pub fn load_device_with_jsfx(
    path: &str,
    class: Option<&str>,
    kind: &str,
    jsfx_path: &str,
) -> Result<DeviceDefinition, String> {
    guard_plugin_path(path)?;
    let absolute_jsfx = guard_jsfx_path(jsfx_path)?;
    let class_name = class.unwrap_or_default();
    if !class_name.to_ascii_lowercase().contains("ysfx") {
        return Err(format!(
            "所选插件不是 ysfx（class={class_name}），拒绝注入 JSFX"
        ));
    }
    let (mut instance, device_kind): (Box<dyn PluginInstance>, DeviceKind) = match kind {
        "instrument" => (
            Host::load(path, class).map_err(|error| format!("插件验证加载失败: {error}"))?,
            DeviceKind::ExternalInstrument,
        ),
        "effect" => (
            Host::load_effect(path, class).map_err(|error| format!("插件验证加载失败: {error}"))?,
            DeviceKind::ExternalEffect,
        ),
        other => {
            return Err(format!(
                "`kind` 只能是 instrument 或 effect，实为 `{other}`"
            ));
        }
    };
    let blob = ysfx_state_blob(&absolute_jsfx);
    instance
        .restore_state(&PluginState {
            processor: blob,
            controller: Vec::new(),
        })
        .map_err(|error| format!("ysfx 注入 JSFX 失败（{absolute_jsfx}）: {error}"))?;
    let state = instance
        .save_state()
        .map_err(|error| format!("ysfx 状态回读失败（注入未落盘）: {error}"))?
        .encode();
    let instance_name = instance.name().to_owned();
    Ok(DeviceDefinition {
        id: EntityId::new(),
        name: instance_name.clone(),
        kind: device_kind,
        bypassed: false,
        params: Vec::new(),
        latency_samples: instance.latency_samples(),
        plugin: Some(PluginBinding {
            path: path.to_owned(),
            class: Some(class.map_or(instance_name, str::to_owned)),
            state,
        }),
    })
}

/// 验证加载一个插件并返回将落进工程的设备定义。
///
/// `kind` 只能是 `instrument` / `effect`；其它值在路径门之后响亮拒绝，
/// 错误原文与 MCP `yeban_load_plugin` 的 `kind` 校验一致。
///
/// # 执行口径（T-FIX-338）
///
/// 本函数只做**验证加载**：实例在返回前被丢弃，落进工程的只有设备定义
/// （名、延迟、插件绑定与装载时刻的状态快照）。实时播放不执行该插件
/// （见 [`PLUGIN_REGISTRATION_NOTE`]）；离线导出与整曲预览执行首台已装载
/// 的插件（与 MCP 离线渲染同一套装配）。
///
/// # Errors
///
/// 路径门失败、`kind` 非法、宿主验证加载失败；加载失败返回
/// `插件验证加载失败: {HostError}`，⛔ 不回退到任何默认插件。
pub fn load_device(
    path: &str,
    class: Option<&str>,
    kind: &str,
) -> Result<DeviceDefinition, String> {
    guard_plugin_path(path)?;
    let (mut instance, device_kind): (Box<dyn PluginInstance>, DeviceKind) = match kind {
        "instrument" => (
            Host::load(path, class).map_err(|error| format!("插件验证加载失败: {error}"))?,
            DeviceKind::ExternalInstrument,
        ),
        "effect" => (
            Host::load_effect(path, class).map_err(|error| format!("插件验证加载失败: {error}"))?,
            DeviceKind::ExternalEffect,
        ),
        other => {
            return Err(format!(
                "`kind` 只能是 instrument 或 effect，实为 `{other}`"
            ));
        }
    };
    let instance_name = instance.name().to_owned();
    // ⭐ T-FIX-345：装载即抓一次插件自己的状态（处理器 + 控制器 blob），随设备
    // 绑定一起进工程。当前 GUI 还没有常驻插件编辑器 ⇒ 这里抓到的是**装载时刻**
    // 的状态；将来编辑器持有活实例后，保存工程前应再调用同一个 `save_state`。
    // 抓取失败不让"装载"变成失败（插件本身可用），但**响亮**登记，⛔ 静默吞。
    let state = match instance.save_state() {
        Ok(state) => state.encode(),
        Err(error) => {
            eprintln!("[yeban-app] 插件状态抓取失败（工程将保存为空状态，重开回默认）: {error}");
            Vec::new()
        }
    };
    Ok(DeviceDefinition {
        id: EntityId::new(),
        name: instance_name.clone(),
        kind: device_kind,
        bypassed: false,
        params: Vec::new(),
        latency_samples: instance.latency_samples(),
        plugin: Some(PluginBinding {
            path: path.to_owned(),
            class: Some(class.map_or(instance_name, str::to_owned)),
            state,
        }),
    })
}

/// ARA 会话 registry（GUI 面板持有的那一份；与 MCP `Domain` 的 registry 互不共享）。
pub struct AraRegistry {
    sessions: BTreeMap<String, AraSession>,
    next_id: u64,
}

impl Default for AraRegistry {
    fn default() -> Self {
        Self {
            sessions: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl AraRegistry {
    /// 已登记会话数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// 已登记会话 id 清单（按 id 升序）。
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// ⭐ 清空全部会话（T-FIX-349 的工程边界；与 MCP 的 `ara_sessions.clear()` 同义）。
    /// 旧 sessionId 从此不再存在 ⇒ 查询/渲染都会明确失败，⛔ 不跨工程复用。
    pub fn clear(&mut self) {
        self.sessions.clear();
    }

    /// 从 registry 取出一个会话（渲染后台线程移动用）。
    #[must_use]
    pub fn take(&mut self, id: &str) -> Option<AraSession> {
        self.sessions.remove(id)
    }

    /// 把会话放回 registry（渲染结束 / 失败恢复用）。
    pub fn restore(&mut self, id: String, session: AraSession) {
        self.sessions.insert(id, session);
    }

    /// 绑定一段音频 wav 到 ARA 插件；返回新会话摘要。
    ///
    /// # Errors
    ///
    /// 会话上限、路径门、wav 解码、速度 / 循环值域或插件 ARA 会话打开失败。
    #[allow(clippy::too_many_arguments)]
    pub fn bind(
        &mut self,
        plugin_path: &str,
        plugin_id: Option<&str>,
        wav_path: &str,
        tempo_bpm: f64,
        loop_iterations: usize,
        muted: bool,
    ) -> Result<AraSummary, String> {
        if self.sessions.len() >= MAX_ARA_SESSIONS {
            return Err(format!(
                "ARA 会话数已达上限 {MAX_ARA_SESSIONS}，请先解绑（yeban_ara_unbind）再绑定"
            ));
        }
        guard_plugin_path(plugin_path)?;
        if !tempo_bpm.is_finite() || tempo_bpm <= 0.0 {
            return Err("`tempoBpm` 须为有限正数".to_owned());
        }
        if loop_iterations == 0 {
            return Err("`loopIterations` 须 ≥ 1".to_owned());
        }
        let wav = PathBuf::from(wav_path);
        let pcm = decode_wav_to_pcm(&wav)?;
        let frames = pcm.frames();
        let sample_rate = pcm.sample_rate;
        let id = format!("ara-{}", self.next_id);
        let clip_name = wav
            .file_stem()
            .map_or_else(|| id.clone(), |stem| stem.to_string_lossy().into_owned());
        let clip = AraClip {
            name: clip_name,
            persistent_id: format!("{id}-clip"),
            kind: AraClipKind::Audio,
            pcm,
            tempo_bpm,
        };
        let session = AraSession::open(
            plugin_path,
            plugin_id,
            clip,
            AraPlacement {
                loop_iterations,
                muted,
            },
        )
        .map_err(|error| format!("ARA 操作失败: {error}"))?;
        let factory = session.factory_info().clone();
        self.next_id = self.next_id.saturating_add(1);
        self.sessions.insert(id.clone(), session);
        Ok(AraSummary {
            id,
            factory_name: factory.plug_in_name,
            factory_version: factory.version,
            frames,
            sample_rate,
            loop_iterations,
            muted,
        })
    }

    /// 查询会话分析与内容状态；会话不存在时错误原文与 MCP 一致（带已登记 id）。
    ///
    /// # Errors
    ///
    /// 会话不存在，或插件分析状态读取失败。
    pub fn query(&self, id: &str) -> Result<String, String> {
        let session = self.sessions.get(id).ok_or_else(|| self.not_found(id))?;
        let available = session
            .analysis_available()
            .map_err(|error| format!("ARA 操作失败: {error}"))?;
        let (notes, notes_error) = match session.notes() {
            Ok(notes) => (Some(notes.len()), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let (head_seconds, tail_seconds) = session
            .head_tail()
            .map_err(|error| format!("ARA 操作失败: {error}"))?;
        let notes_label = match notes {
            Some(count) => format!("{count} 个音符"),
            None => format!(
                "音符不可用: {}",
                notes_error.unwrap_or_else(|| "插件未返回原因".to_owned())
            ),
        };
        Ok(format!(
            "{} v{} · 分析 {} · region {} · head {:.3}s / tail {:.3}s · {}",
            session.factory_info().plug_in_name,
            session.factory_info().version,
            if available { "可用" } else { "未完成" },
            session.region_count(),
            head_seconds,
            tail_seconds,
            notes_label,
        ))
    }

    fn not_found(&self, id: &str) -> String {
        format!(
            "没有 ARA 会话 `{id}`（已登记：{}）",
            if self.sessions.is_empty() {
                "无".to_owned()
            } else {
                self.sessions.keys().cloned().collect::<Vec<_>>().join(", ")
            }
        )
    }
}

/// 绑定成功后的会话摘要（进 UI 行 / 回执）。
#[derive(Clone, Debug, PartialEq)]
pub struct AraSummary {
    /// 会话 id。
    pub id: String,
    /// factory 名。
    pub factory_name: String,
    /// factory 版本。
    pub factory_version: String,
    /// 片段帧数。
    pub frames: u64,
    /// 片段采样率。
    pub sample_rate: f64,
    /// 循环次数。
    pub loop_iterations: usize,
    /// 是否静音。
    pub muted: bool,
}

/// 渲染结果摘要。
#[derive(Clone, Debug, PartialEq)]
pub struct AraRenderSummary {
    /// 会话 id。
    pub session_id: String,
    /// 输出路径。
    pub out_path: String,
    /// 帧数。
    pub frames: usize,
    /// 峰值。
    pub peak: f32,
    /// 非零帧数。
    pub nonzero_frames: usize,
    /// WAV 字节数。
    pub bytes: usize,
    /// 文档 blob 字节数。
    pub blob_bytes: usize,
    /// 文档 blob SHA-256。
    pub blob_sha256: String,
}

/// 校验 `blockFrames` 文本：缺省 1024，范围 1..=4096；错误原文与 MCP 一致。
///
/// # Errors
///
/// 不是整数、为 0 或超过 4096。
pub fn parse_block_frames(text: &str) -> Result<usize, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(1024);
    }
    let value: u64 = text
        .parse()
        .map_err(|_| format!("`blockFrames` 必须是非负整数, 实际收到 {text:?}"))?;
    if !(1..=4096).contains(&value) {
        return Err("`blockFrames` 须在 1..=4096 内".to_owned());
    }
    usize::try_from(value).map_err(|_| "`blockFrames` 超出范围".to_owned())
}

/// 解析 ARA 速度文本：缺省 120，须为有限正数；错误原文与 MCP 一致。
///
/// # Errors
///
/// 不是数字、非有限或 ≤ 0。
pub fn parse_tempo(text: &str) -> Result<f64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(120.0);
    }
    let value: f64 = text.parse().map_err(|_| "`tempoBpm` 须为数字".to_owned())?;
    if !value.is_finite() || value <= 0.0 {
        return Err("`tempoBpm` 须为有限正数".to_owned());
    }
    Ok(value)
}

/// 解析 ARA 循环次数文本：缺省 1，须 ≥ 1；错误原文与 MCP 一致。
///
/// # Errors
///
/// 不是整数、为 0 或超出 `usize`。
pub fn parse_loop_iterations(text: &str) -> Result<usize, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(1);
    }
    let value: u64 = text
        .parse()
        .map_err(|_| format!("`loopIterations` 必须是非负整数, 实际收到 {text:?}"))?;
    if value == 0 {
        return Err("`loopIterations` 须 ≥ 1".to_owned());
    }
    let value = usize::try_from(value).map_err(|_| "`loopIterations` 超出范围".to_owned())?;
    // ⭐ T-FIX-351：与 MCP / 宿主共用同一个上限常量（`yeban_plugin_host::ara::MAX_LOOP_ITERATIONS`）。
    // 超限在这里就报错（状态栏可见），⛔ 不要放进后台再让宿主 OOM。
    if value > yeban_plugin_host::ara::MAX_LOOP_ITERATIONS {
        return Err(format!(
            "`loopIterations` 须 ≤ {}（收到 {value}）",
            yeban_plugin_host::ara::MAX_LOOP_ITERATIONS
        ));
    }
    Ok(value)
}

/// 渲染一个已绑定会话到 f32 立体声 WAV（原子落盘），返回摘要。
///
/// 会话必须已在 registry 中；本函数不移动会话（GUI 的异步腿先 `take` 再在
/// 后台线程调用 [`render_taken`]）。
///
/// # Errors
///
/// 会话不存在、`blockFrames` 越界、渲染 / 编码 / 落盘失败；错误原文前缀
/// 与 MCP `yeban_ara_render` 的 `ARA 操作失败:` 一致。
pub fn render(
    registry: &mut AraRegistry,
    id: &str,
    out_path: &str,
    block_frames: usize,
) -> Result<AraRenderSummary, String> {
    if !registry.sessions.contains_key(id) {
        return Err(registry.not_found(id));
    }
    let session = registry
        .sessions
        .get_mut(id)
        .expect("会话存在性上一行已校验");
    render_session(session, id, out_path, block_frames)
}

/// 后台渲染腿：会话已被 [`AraRegistry::take`] 取出，渲染后由调用方放回。
///
/// # Errors
///
/// 与 [`render`] 相同。
pub fn render_taken(
    session: &mut AraSession,
    id: &str,
    out_path: &str,
    block_frames: usize,
) -> Result<AraRenderSummary, String> {
    render_session(session, id, out_path, block_frames)
}

fn render_session(
    session: &mut AraSession,
    id: &str,
    out_path: &str,
    block_frames: usize,
) -> Result<AraRenderSummary, String> {
    if !(1..=4096).contains(&block_frames) {
        return Err("`blockFrames` 须在 1..=4096 内".to_owned());
    }
    let out = PathBuf::from(out_path);
    if !out.parent().is_some_and(Path::is_dir) {
        return Err(format!(
            "目标父目录不存在: {}",
            out.parent().unwrap_or(Path::new("")).display()
        ));
    }
    let total_frames = session.clip_frames();
    let rendered = session
        .render(total_frames, block_frames)
        .map_err(|error| format!("ARA 操作失败: {error}"))?;
    let sample_rate = session.clip_sample_rate() as u32;
    let format = yeban_render::rf64::PcmFormat::float(2, sample_rate, 32);
    if rendered.left.len() != rendered.right.len() {
        return Err(format!(
            "左右声道不等长（{} vs {}），无法编码 WAV",
            rendered.left.len(),
            rendered.right.len()
        ));
    }
    let mut interleaved = Vec::with_capacity(rendered.left.len().saturating_mul(2));
    for (left, right) in rendered.left.iter().zip(rendered.right.iter()) {
        interleaved.push(*left);
        interleaved.push(*right);
    }
    let temp = out.with_file_name(format!(
        ".{}.tmp-{}",
        out.file_name().map_or_else(
            || "ara-render".to_owned(),
            |name| name.to_string_lossy().into_owned()
        ),
        EntityId::new().to_canonical_string()
    ));
    if let Err(error) = yeban_render::wav::write_plain_wav(
        &temp,
        &format,
        &yeban_render::dither::PcmBuffer::Float32(interleaved),
    ) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("ARA 渲染编码失败: {error}"));
    }
    let bytes = match std::fs::read(&temp) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("ARA 渲染临时文件读取失败: {error}"));
        }
    };
    if let Err(error) = std::fs::remove_file(&temp) {
        eprintln!("[yeban-app] ARA 渲染临时文件清理失败: {error}");
    }
    write_file_atomically(&bytes, &out)
        .map_err(|error| format!("ARA 渲染结果落盘失败: {error}"))?;
    let blob = session
        .store_blob()
        .map_err(|error| format!("ARA 操作失败: {error}"))?;
    Ok(AraRenderSummary {
        session_id: id.to_owned(),
        out_path: out.display().to_string(),
        frames: rendered.left.len(),
        peak: rendered.peak(),
        nonzero_frames: rendered.nonzero_frames(),
        bytes: bytes.len(),
        blob_bytes: blob.len(),
        blob_sha256: AssetHash::of_bytes(&blob).as_str().to_owned(),
    })
}

fn decode_wav_to_pcm(path: &Path) -> Result<yeban_plugin_host::PcmData, String> {
    let asset = decode_path(path, &DecodeOptions::default())
        .map_err(|error| format!("音频解码失败 {}: {error}", path.display()))?;
    let channels = asset.channels() as usize;
    let frames = asset.frame_count() as usize;
    let samples = asset.samples();
    if channels == 0 || samples.len() < frames.saturating_mul(channels) {
        return Err(format!(
            "解码产物自相矛盾 {}: 声道 {} 帧 {} 样本 {}",
            path.display(),
            channels,
            frames,
            samples.len()
        ));
    }
    let mut columns: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); channels];
    for frame in 0..frames {
        for (channel, column) in columns.iter_mut().enumerate() {
            column.push(samples[frame * channels + channel]);
        }
    }
    Ok(yeban_plugin_host::PcmData {
        channels: columns,
        sample_rate: f64::from(asset.sample_rate()),
    })
}

fn tuid_hex(id: &yeban_plugin_host::types::Tuid) -> String {
    // `Tuid` 是 `[u8; 16]`；类标识按 MCP 的同一十六进制口径展示。
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-tfix83-{tag}-{}",
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("临时目录");
        dir
    }

    #[test]
    fn scan_reports_a_broken_candidate_as_a_row_not_an_error() {
        let dir = temp_dir("scan");
        let broken = dir.join("Broken.vst3");
        std::fs::write(&broken, b"not a real vst3").expect("假插件");
        let report = scan_paths(std::slice::from_ref(&dir));
        assert_eq!(report.rows.len(), 1, "一个坏模块必须有一行");
        assert!(!report.rows[0].usable);
        assert_eq!(report.failed, 1);
        assert!(report.rows[0].error.is_some(), "失败原因必须留在行上");
        assert!(report.rows[0].state().starts_with("失败:"));
    }

    #[test]
    fn guard_plugin_path_keeps_the_mcp_error_prefix() {
        let dir = temp_dir("guard");
        let text = dir.join("not-a-plugin.txt");
        std::fs::write(&text, b"x").expect("临时文件");
        let error = guard_plugin_path(text.to_string_lossy().as_ref()).expect_err("扩展名必须拒绝");
        assert!(error.starts_with("插件路径校验失败，拒绝加载:"), "{error}");
    }

    #[test]
    fn registration_note_states_the_execution_policy() {
        assert!(
            PLUGIN_REGISTRATION_NOTE.contains("仅登记"),
            "{PLUGIN_REGISTRATION_NOTE}"
        );
        assert!(
            PLUGIN_REGISTRATION_NOTE.contains("实时播放不执行"),
            "{PLUGIN_REGISTRATION_NOTE}"
        );
        assert!(
            PLUGIN_REGISTRATION_NOTE.contains("离线导出"),
            "{PLUGIN_REGISTRATION_NOTE}"
        );
        assert!(
            PLUGIN_REGISTRATION_NOTE.contains("首台"),
            "{PLUGIN_REGISTRATION_NOTE}"
        );
    }

    #[test]
    fn load_device_rejects_an_unknown_kind_without_defaulting() {
        let error = load_device("stub:sine", None, "sampler").expect_err("未知 kind 必须拒绝");
        assert_eq!(error, "`kind` 只能是 instrument 或 effect，实为 `sampler`");
    }

    #[test]
    fn load_device_uses_the_host_validation_and_records_the_binding() {
        let device = load_device("stub:sine", None, "instrument").expect("桩必须能验证加载");
        assert_eq!(device.kind, DeviceKind::ExternalInstrument);
        assert_eq!(
            device.plugin.as_ref().map(|binding| binding.path.as_str()),
            Some("stub:sine")
        );
    }

    #[test]
    fn ara_values_keep_the_mcp_error_text() {
        let mut registry = AraRegistry::default();
        assert_eq!(
            registry
                .bind("stub:sine", None, "does-not-exist.wav", 0.0, 1, false)
                .unwrap_err(),
            "`tempoBpm` 须为有限正数"
        );
        assert_eq!(
            registry
                .bind("stub:sine", None, "does-not-exist.wav", 120.0, 0, false)
                .unwrap_err(),
            "`loopIterations` 须 ≥ 1"
        );
        assert_eq!(parse_tempo("0").unwrap_err(), "`tempoBpm` 须为有限正数");
        // T-FIX-351：GUI 侧超大值与 MCP 同一条上限（超限报错而不是交给宿主 OOM）。
        assert!(
            parse_loop_iterations("4097").is_err(),
            "超过 MAX_LOOP_ITERATIONS 必须被 GUI 拒掉"
        );
        assert_eq!(
            parse_loop_iterations("4096").expect("上限值必须放行"),
            yeban_plugin_host::ara::MAX_LOOP_ITERATIONS
        );
        assert_eq!(
            parse_loop_iterations("0").unwrap_err(),
            "`loopIterations` 须 ≥ 1"
        );
        assert_eq!(
            parse_block_frames("0").unwrap_err(),
            "`blockFrames` 须在 1..=4096 内"
        );
        assert_eq!(parse_block_frames("").unwrap(), 1024);
    }

    #[test]
    fn missing_ara_session_names_the_registry() {
        let registry = AraRegistry::default();
        let error = registry.query("ara-99").expect_err("空 registry 必须拒绝");
        assert!(error.contains("没有 ARA 会话 `ara-99`"), "{error}");
        assert!(error.contains("已登记：无"), "{error}");
    }

    #[test]
    fn ysfx_state_blob_carries_the_jsfx_path_in_value_tree_form() {
        let blob = ysfx_state_blob("/tmp/tone.jsfx");
        assert_eq!(&blob[..5], b"ysfx\0");
        // 根节点：2 个属性（version / path），0 个子节点。
        assert_eq!(blob[5], 1, "压缩整数首字节 = 数据字节数");
        assert_eq!(blob[6], 2, "随后是值 2");
        assert!(blob.windows(8).any(|w| w == b"version\0"));
        assert!(blob.windows(5).any(|w| w == b"path\0"));
        assert!(blob.windows(14).any(|w| w == b"/tmp/tone.jsfx"));
        assert_eq!(*blob.last().unwrap(), 0, "根节点必须写明 0 个子节点");
    }

    #[test]
    fn jsfx_path_gate_is_loud_for_every_failure() {
        let dir = std::env::temp_dir().join(format!("yeban-jsfx-gate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("tone.jsfx");
        std::fs::write(&good, "desc:x\n@sample\nspl0=0;\n").unwrap();
        let not_jsfx = dir.join("tone.txt");
        std::fs::write(&not_jsfx, "x").unwrap();

        let canonical = guard_jsfx_path(good.to_string_lossy().as_ref()).unwrap();
        assert!(canonical.ends_with("tone.jsfx"));
        let missing = guard_jsfx_path(&dir.join("missing.jsfx").to_string_lossy()).unwrap_err();
        assert!(missing.contains("不存在"), "{missing}");
        let wrong = guard_jsfx_path(not_jsfx.to_string_lossy().as_ref()).unwrap_err();
        assert!(wrong.contains("不是 .jsfx"), "{wrong}");
        let empty = guard_jsfx_path("  ").unwrap_err();
        assert!(empty.contains("路径为空"), "{empty}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
