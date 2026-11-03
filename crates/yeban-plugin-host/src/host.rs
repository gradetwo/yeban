//! 宿主入口：扫描、加载与类选择（格式无关层；VST3 细节在 `vst3.rs`）。
//!
//! 扫描口径（ADR-0064 D12，类比 Logic 的 auval 准入）：
//! - 发现候选：`.vst3` 包内平台动态库＋散动态库（扩展名按平台取，
//!   Linux `.so`、macOS `.dylib`、Windows `.dll`／`.vst3`）。
//! - 逐个尝试加载与枚举类：失败记原因、继续扫下一个，⛔ 不中断扫描；
//!   失败模块不进可用列表（`descriptor.usable()` 为假），但仍列出供诊断。
//! - 扫描结果按路径排序（确定性；模型层坚持 `BTreeMap` 的同款纪律）。

use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::stub::{StubEffect, StubInstrument};
use crate::types::{PluginDescriptor, PluginInstance, PluginKind, tuid_hex};
use crate::vst3::{Vst3Instance, Vst3Module, inspect_module};

/// `stub:` 桩的前缀（仅测试夹具与演示工程用，⛔ 不是真实插件路径）。
pub const STUB_PREFIX: &str = "stub:";

/// 平台动态库扩展名（扫描候选过滤用）。
#[cfg(target_os = "windows")]
pub const DYLIB_EXTENSION: &str = "dll";
/// 平台动态库扩展名（扫描候选过滤用）。
#[cfg(target_os = "macos")]
pub const DYLIB_EXTENSION: &str = "dylib";
/// 平台动态库扩展名（扫描候选过滤用）。
#[cfg(all(unix, not(target_os = "macos")))]
pub const DYLIB_EXTENSION: &str = "so";

/// 环境变量 `YEBAN_VST3_PATHS` 的分隔符（Unix `:`、Windows `;`）。
#[cfg(windows)]
const PATH_LIST_SEP: char = ';';
/// 环境变量 `YEBAN_VST3_PATHS` 的分隔符（Unix `:`、Windows `;`）。
#[cfg(not(windows))]
const PATH_LIST_SEP: char = ':';

/// 宿主（无状态；扫描与加载均为关联函数，便于 MCP 层直接调用）。
pub struct Host;

impl Host {
    /// 环境变量 `YEBAN_VST3_PATHS` 列出的目录（空串项跳过）。
    #[must_use]
    pub fn env_search_paths() -> Vec<PathBuf> {
        std::env::var("YEBAN_VST3_PATHS")
            .unwrap_or_default()
            .split(PATH_LIST_SEP)
            .filter(|item| !item.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    /// 平台标准 VST3 目录（Linux `~/.vst3`＋`/usr/lib/vst3`；
    /// macOS 用户＋系统 `Library/Audio/Plug-Ins/VST3`；
    /// Windows `%COMMONPROGRAMFILES%\VST3`）。
    ///
    /// ⚠ Windows 上非 ASCII 路径经 `LoadLibraryA` 有损转换可能打不开
    /// （`dylib.rs` 的已知一期限缩）；标准路径本身多为 ASCII。
    #[must_use]
    pub fn default_search_paths() -> Vec<PathBuf> {
        let mut out = Vec::new();
        #[cfg(target_os = "linux")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                out.push(PathBuf::from(home).join(".vst3"));
            }
            out.push(PathBuf::from("/usr/lib/vst3"));
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                out.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
            }
            out.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(common) = std::env::var("COMMONPROGRAMFILES") {
                out.push(PathBuf::from(common).join("VST3"));
            }
        }
        out
    }

    /// 全部扫描根（环境变量在前，平台标准路径在后；调用方可再追加 `paths`）。
    #[must_use]
    pub fn all_search_paths(extra: &[PathBuf]) -> Vec<PathBuf> {
        let mut out = Self::env_search_paths();
        out.extend(Self::default_search_paths());
        out.extend(extra.iter().cloned());
        out
    }

    /// 扫描目录：发现候选、逐个加载验证、失败记原因继续。
    ///
    /// 不存在的目录静默跳过（⛔ 不是失败：标准路径在很多机器上本来就不存在）。
    /// 同一模块被多个根命中时各列一条（不去重：诊断时要看清是哪个路径的副本）。
    #[must_use]
    pub fn scan(dirs: &[PathBuf]) -> Vec<PluginDescriptor> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(candidate) = resolve_candidate(&path) {
                    candidates.push(candidate);
                }
            }
        }
        candidates.sort();
        let mut out = Vec::new();
        for candidate in candidates {
            let path_string = candidate.to_string_lossy().into_owned();
            match inspect_module(&path_string) {
                Ok((vendor, classes)) => out.push(PluginDescriptor {
                    path: path_string,
                    vendor,
                    classes,
                    error: None,
                }),
                Err(error) => out.push(PluginDescriptor {
                    path: path_string,
                    vendor: String::new(),
                    classes: Vec::new(),
                    error: Some(error.to_string()),
                }),
            }
        }
        out
    }

    /// 加载一个乐器插件并做验证激活循环（激活 → 停用），返回未激活实例。
    ///
    /// - `stub:` 路径走进程内桩（`stub:sine`）。
    /// - `plugin_id` 指名：精确匹配类名，否则按 CID 十六进制匹配；
    ///   不指名时取首个乐器类；无乐器类 ⇒ `NotAnInstrument`。
    /// - 验证激活用 48 kHz／4096 帧：只证明整链可激活，渲染时由渲染源
    ///   按实际采样率重激活（`activate` 已激活时先停用再重建）。
    ///
    /// # Errors
    ///
    /// 路径非法、模块失败、类找不到、不是乐器、验证激活失败 ⇒ [`HostError`]。
    pub fn load(path: &str, plugin_id: Option<&str>) -> Result<Box<dyn PluginInstance>, HostError> {
        Self::load_kind(path, plugin_id, PluginKind::Instrument)
    }

    /// 加载一个效果器插件并做验证激活循环（激活 → 停用），返回未激活实例。
    ///
    /// 与 [`Self::load`] 同款流程，只把 kind 换成效果器：`stub:` 走 `stub:gain`；
    /// 不指名时取首个效果器类；无效果器类 ⇒ `NotAnEffect`；指名了乐器类 ⇒
    /// `NotAnEffect`（不是“不是乐器”的反向口径 —— 拒绝的理由就是 kind 不对）。
    ///
    /// # Errors
    ///
    /// 路径非法、模块失败、类找不到、不是效果器、验证激活失败 ⇒ [`HostError`]。
    pub fn load_effect(
        path: &str,
        plugin_id: Option<&str>,
    ) -> Result<Box<dyn PluginInstance>, HostError> {
        Self::load_kind(path, plugin_id, PluginKind::Effect)
    }

    /// 按 kind 加载（`load`／`load_effect` 的共享实现）。
    fn load_kind(
        path: &str,
        plugin_id: Option<&str>,
        want_kind: PluginKind,
    ) -> Result<Box<dyn PluginInstance>, HostError> {
        if let Some(stub_name) = path.strip_prefix(STUB_PREFIX) {
            return Self::load_stub(stub_name, want_kind);
        }
        let module = Vst3Module::load(path)?;
        let raw = module.classes_raw();
        if raw.is_empty() {
            return Err(HostError::ClassNotFound("工厂没有登记任何类".to_owned()));
        }
        // 类选择：指名优先（类名精确，或 CID 十六进制）；不指名取首个目标 kind 类。
        let chosen = if let Some(want) = plugin_id {
            raw.iter()
                .find(|(cid, name, _)| name == want || tuid_hex(cid).eq_ignore_ascii_case(want))
                .ok_or_else(|| {
                    HostError::ClassNotFound(format!("没有类叫 `{want}`（或 CID 不是它）"))
                })?
                .clone()
        } else {
            // 不指名：逐个实例化探针，首个目标 kind 类胜出（探针失败的类跳过）。
            let mut found: Option<(crate::types::Tuid, String, String)> = None;
            for (cid, name, category) in &raw {
                match Vst3Instance::open(&module, cid, name) {
                    Ok(instance) if instance.kind() == want_kind => {
                        found = Some((*cid, name.clone(), category.clone()));
                        break;
                    }
                    _ => continue,
                }
            }
            found.ok_or_else(|| match want_kind {
                PluginKind::Instrument => HostError::NotAnInstrument(format!(
                    "模块 `{path}` 没有乐器类（共 {} 个类）",
                    raw.len()
                )),
                PluginKind::Effect => HostError::NotAnEffect(format!(
                    "模块 `{path}` 没有效果器类（共 {} 个类）",
                    raw.len()
                )),
                PluginKind::Unknown => {
                    HostError::Other("load_kind 不接受 Unknown 目标 kind".to_owned())
                }
            })?
        };
        let mut instance = Vst3Instance::open(&module, &chosen.0, &chosen.1)?;
        if instance.kind() != want_kind {
            return Err(match want_kind {
                PluginKind::Instrument => HostError::NotAnInstrument(format!(
                    "类 `{}` 不是乐器（kind＝{:?}）",
                    chosen.1,
                    instance.kind()
                )),
                PluginKind::Effect => HostError::NotAnEffect(format!(
                    "类 `{}` 不是效果器（kind＝{:?}）",
                    chosen.1,
                    instance.kind()
                )),
                PluginKind::Unknown => {
                    HostError::Other("load_kind 不接受 Unknown 目标 kind".to_owned())
                }
            });
        }
        // 验证激活循环：证明整链可激活，再停用交还（渲染时重激活）。
        instance.activate(48_000.0, 4096)?;
        instance.deactivate();
        Ok(Box::new(instance))
    }

    /// `stub:` 桩分发（`stub:sine` 乐器／`stub:gain` 效果器；kind 不对或未知桩 ⇒ 响亮失败）。
    fn load_stub(name: &str, want_kind: PluginKind) -> Result<Box<dyn PluginInstance>, HostError> {
        match (name, want_kind) {
            ("sine", PluginKind::Instrument) => Ok(Box::new(StubInstrument::new())),
            ("gain", PluginKind::Effect) => Ok(Box::new(StubEffect::new())),
            ("sine", PluginKind::Effect) => Err(HostError::NotAnEffect(
                "测试桩 `stub:sine` 是乐器，不是效果器".to_owned(),
            )),
            ("gain", PluginKind::Instrument) => Err(HostError::NotAnInstrument(
                "测试桩 `stub:gain` 是效果器，不是乐器".to_owned(),
            )),
            _ => Err(HostError::InvalidPluginPath(format!(
                "未知测试桩 `stub:{name}`（目前仅 `stub:sine`／`stub:gain`）"
            ))),
        }
    }
}

/// 扫描候选解析：`.vst3` 包 → 包内平台动态库；散动态库 → 自身；其余 → 无。
fn resolve_candidate(path: &Path) -> Option<PathBuf> {
    if path.extension().is_some_and(|ext| ext == "vst3") {
        return resolve_bundle_binary(path);
    }
    if path.is_file() && path.extension().is_some_and(|ext| ext == DYLIB_EXTENSION) {
        return Some(path.to_path_buf());
    }
    None
}

/// `.vst3` 包内平台动态库解析（Linux `Contents/x86_64-linux/*.so`；
/// macOS `Contents/MacOS/*`；Windows `Contents/x86_64-win/*.vst3`）。
/// 包路径本身是文件时（Windows 单文件 `.vst3`）直接用它。
fn resolve_bundle_binary(bundle: &Path) -> Option<PathBuf> {
    if bundle.is_file() {
        return Some(bundle.to_path_buf());
    }
    #[cfg(target_os = "linux")]
    let subdir = bundle.join("Contents/x86_64-linux");
    #[cfg(target_os = "macos")]
    let subdir = bundle.join("Contents/MacOS");
    #[cfg(target_os = "windows")]
    let subdir = bundle.join("Contents/x86_64-win");
    let Ok(entries) = std::fs::read_dir(&subdir) else {
        return None;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_bundle_binary(path))
        .collect();
    files.sort();
    files.into_iter().next()
}

/// 包内动态库判定（Windows 要 `.vst3` 后缀；macOS 的 `Contents/MacOS/*`
/// 常无扩展名，无扩展名的文件也算候选；Linux 要 `.so`）。
fn is_bundle_binary(path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        path.extension().is_some_and(|ext| ext == "vst3")
    }
    #[cfg(target_os = "macos")]
    {
        path.extension().is_some_and(|ext| ext == "dylib") || path.extension().is_none()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        path.extension().is_some_and(|ext| ext == "so")
    }
}
