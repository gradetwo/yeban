//! 宿主入口：扫描、加载与类选择（格式无关层；VST3 细节在 `vst3.rs`）。
//!
//! 扫描口径（ADR-0069 D12，类比 Logic 的 auval 准入）：
//! - 发现候选：`.vst3` 包内平台动态库＋散动态库（扩展名按平台取，
//!   Linux `.so`、macOS `.dylib`、Windows `.dll`／`.vst3`）。
//! - 逐个尝试加载与枚举类：失败记原因、继续扫下一个，⛔ 不中断扫描；
//!   失败模块不进可用列表（`descriptor.usable()` 为假），但仍列出供诊断。
//! - 扫描结果按路径排序（确定性；模型层坚持 `BTreeMap` 的同款纪律）。

use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::stub::{StubEffect, StubInstrument};
use crate::types::{PluginDescriptor, PluginInstance, PluginKind, tuid_hex};
use crate::vst3::{Vst3Instance, Vst3Module, is_non_component_class};

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
    /// Windows 上经 `LoadLibraryW`（UTF-16）打开动态库，支持含非 ASCII 字符的路径。
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

    /// 插件路径校验（P0-5）：`dlopen` 之前的准入门。凡按外部实参指定路径
    /// 加载插件的入口（`yeban_load_plugin`、`yeban_ara_bind` 等）都必须先
    /// 过此门，未过门不得把路径交给 [`Self::load`]／`AraSession::open`。
    ///
    /// 三层校验，任一不过即 [`HostError::InvalidPluginPath`] 响亮拒绝：
    /// 1. **形态**：只接受 `.vst3` 包或平台动态库扩展名（与扫描候选同口径）；
    /// 2. **规范化**：`canonicalize()` 必须成功（路径真实存在）；校验与
    ///    containment 都在规范形态上做，符号链接逃逸无所遁形；
    /// 3. **来源 containment**：规范路径必须落在某个插件根内——插件根 =
    ///    [`Self::all_search_paths`]（`YEBAN_VST3_PATHS`＋平台标准根）。
    ///    操作者要加载根外插件，先把目录写进 `YEBAN_VST3_PATHS`：信任由
    ///    操作者显式授予，不由工具实参自带。
    ///
    /// `stub:` 桩不是文件路径（进程内夹具），原样放行，与 [`Self::load`]
    /// 的分流一致。返回规范化后的路径（`.vst3` 包返回包路径本身，包内
    /// 平台二进制的解析仍由加载层做）。
    ///
    /// # Errors
    ///
    /// 形态不对、路径不存在或不可达、不在任何插件根内 ⇒
    /// [`HostError::InvalidPluginPath`]（消息列出实参路径与允许根清单）。
    pub fn validate_plugin_path(path: &str) -> Result<PathBuf, HostError> {
        if path.starts_with(STUB_PREFIX) {
            return Ok(PathBuf::from(path));
        }
        Self::validate_plugin_path_in_roots(path, &Self::all_search_paths(&[]))
    }

    /// 规范根内包含判定（[`Self::validate_plugin_path`] 与
    /// [`Self::validate_scan_dir`] 共用）：任一根规范化后是 `canonical` 的
    /// 前缀 ⇒ 通过；否则返回规范根清单供报错（规范化失败的根不计入清单）。
    fn canonical_roots_containing(canonical: &Path, roots: &[PathBuf]) -> Result<(), Vec<PathBuf>> {
        let mut canonical_roots: Vec<PathBuf> = Vec::new();
        for root in roots {
            if let Ok(canonical_root) = root.canonicalize() {
                if canonical.starts_with(&canonical_root) {
                    return Ok(());
                }
                canonical_roots.push(canonical_root);
            }
        }
        Err(canonical_roots)
    }

    /// [`Self::validate_plugin_path`] 的可注入根版本（单测用；生产走
    /// 环境变量＋平台标准根）。
    fn validate_plugin_path_in_roots(path: &str, roots: &[PathBuf]) -> Result<PathBuf, HostError> {
        let candidate = Path::new(path);
        let extension_ok = candidate
            .extension()
            .is_some_and(|ext| ext == "vst3" || ext == DYLIB_EXTENSION);
        if !extension_ok {
            return Err(HostError::InvalidPluginPath(format!(
                "`{path}` 不是 `.vst3` 包或平台动态库（扩展名须为 .vst3／.{DYLIB_EXTENSION}）"
            )));
        }
        let canonical = candidate.canonicalize().map_err(|error| {
            HostError::InvalidPluginPath(format!(
                "`{path}` 无法规范化（路径不存在或不可达）: {error}"
            ))
        })?;
        match Self::canonical_roots_containing(&canonical, roots) {
            Ok(()) => Ok(canonical),
            Err(canonical_roots) => Err(HostError::InvalidPluginPath(format!(
                "`{path}`（规范路径 {}）不在任何插件根内；允许根：{}。\
                 要加载根外插件，先把其目录加入环境变量 YEBAN_VST3_PATHS",
                canonical.display(),
                canonical_roots
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join("、"),
            ))),
        }
    }

    /// 扫描目录校验（T-FIX-125）：调用方额外给的扫描目录必须落在某个插件根内。
    ///
    /// 与 [`Self::validate_plugin_path`] 同一条 containment 口径
    /// （`canonicalize`＋规范根前缀），只是不做扩展名检查（目录不是插件文件）。
    /// 不存在的目录 ⇒ [`HostError::InvalidPluginPath`]；调用方
    /// （`yeban_scan_plugins`）按 [`Self::scan`] 的口径静默跳过不存在的目录，
    /// 只对真实存在的目录把关 —— 不存在的目录走不到 dlopen，没有风险。
    ///
    /// # Errors
    ///
    /// 目录不存在／不可达、不在任何插件根内 ⇒
    /// [`HostError::InvalidPluginPath`]（消息列出实参路径与允许根清单）。
    pub fn validate_scan_dir(path: &Path) -> Result<PathBuf, HostError> {
        Self::validate_scan_dir_in_roots(path, &Self::all_search_paths(&[]))
    }

    /// [`Self::validate_scan_dir`] 的可注入根版本（单测用；生产走
    /// 环境变量＋平台标准根）。
    fn validate_scan_dir_in_roots(path: &Path, roots: &[PathBuf]) -> Result<PathBuf, HostError> {
        let canonical = path.canonicalize().map_err(|error| {
            HostError::InvalidPluginPath(format!(
                "`{}` 无法规范化（目录不存在或不可达）: {error}",
                path.display()
            ))
        })?;
        match Self::canonical_roots_containing(&canonical, roots) {
            Ok(()) => Ok(canonical),
            Err(canonical_roots) => Err(HostError::InvalidPluginPath(format!(
                "扫描目录 `{}`（规范路径 {}）不在任何插件根内；允许根：{}。\
                 要扫描根外目录，先把其目录加入环境变量 YEBAN_VST3_PATHS",
                path.display(),
                canonical.display(),
                canonical_roots
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join("、"),
            ))),
        }
    }

    /// 扫描目录：发现候选、逐个加载验证、失败记原因继续。
    ///
    /// 不存在的目录静默跳过（⛔ 不是失败：标准路径在很多机器上本来就不存在）。
    /// 同一模块被多个根命中时各列一条（不去重：诊断时要看清是哪个路径的副本）。
    /// 每行另带 ARA 能力列（模块含 `ARA Main Factory Class` 时 `Some`；
    /// ARA 探针失败 ⇒ `None`，不阻塞扫描、不记入模块 `error`）。
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
            // 每个候选模块只加载一次，同时服务 ARA 探针与类枚举，避免重复 dlopen。
            match Vst3Module::load(&path_string) {
                Ok(module) => {
                    let ara = crate::ara::inspect_ara_factory(&module);
                    match crate::vst3::inspect_loaded_module(&module) {
                        Ok((vendor, classes)) => out.push(PluginDescriptor {
                            path: path_string,
                            vendor,
                            classes,
                            error: None,
                            ara,
                        }),
                        Err(error) => out.push(PluginDescriptor {
                            path: path_string,
                            vendor: String::new(),
                            classes: Vec::new(),
                            error: Some(error.to_string()),
                            ara,
                        }),
                    }
                }
                Err(error) => out.push(PluginDescriptor {
                    path: path_string,
                    vendor: String::new(),
                    classes: Vec::new(),
                    error: Some(error.to_string()),
                    ara: None,
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
        // `.vst3` 包路径先解析到包内平台动态库（与扫描侧同一口径；不解析则 `Is a directory`）。
        let module_path = {
            let candidate = Path::new(path);
            if candidate.extension().is_some_and(|ext| ext == "vst3") {
                resolve_bundle_binary(candidate).ok_or_else(|| {
                    HostError::InvalidPluginPath(format!("`.vst3` 包 `{path}` 内找不到平台动态库"))
                })?
            } else {
                candidate.to_path_buf()
            }
        };
        let module_path = module_path.to_string_lossy().into_owned();
        let module = Vst3Module::load(&module_path)?;
        let raw = module.classes_raw();
        if raw.is_empty() {
            return Err(HostError::ClassNotFound("工厂没有登记任何类".to_owned()));
        }
        // 类选择：指名优先（类名精确，或 CID 十六进制）；不指名取首个目标 kind 类并直接复用探针实例。
        let (chosen_name, mut instance) = if let Some(want) = plugin_id {
            let chosen = raw
                .iter()
                .find(|(cid, name, _)| name == want || tuid_hex(cid).eq_ignore_ascii_case(want))
                .ok_or_else(|| {
                    HostError::ClassNotFound(format!("没有类叫 `{want}`（或 CID 不是它）"))
                })?;
            let inst = Vst3Instance::open(&module, &chosen.0, &chosen.1)?;
            (chosen.1.clone(), inst)
        } else {
            // 不指名：逐个实例化探针，首个目标 kind 类胜出（探针失败的类跳过；
            // 控制器／兼容类不作组件探针，见 `is_non_component_class`）。
            let mut found: Option<(String, Vst3Instance)> = None;
            for (cid, name, category) in &raw {
                if is_non_component_class(category) {
                    continue;
                }
                match Vst3Instance::open(&module, cid, name) {
                    Ok(instance) if instance.kind() == want_kind => {
                        found = Some((name.clone(), instance));
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
        if instance.kind() != want_kind {
            return Err(match want_kind {
                PluginKind::Instrument => HostError::NotAnInstrument(format!(
                    "类 `{}` 不是乐器（kind＝{:?}）",
                    chosen_name,
                    instance.kind()
                )),
                PluginKind::Effect => HostError::NotAnEffect(format!(
                    "类 `{}` 不是效果器（kind＝{:?}）",
                    chosen_name,
                    instance.kind()
                )),
                PluginKind::Unknown => {
                    HostError::Other("load_kind 不接受 Unknown 目标 kind".to_owned())
                }
            });
        }
        let _ = chosen_name;
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

/// 模块路径解析（`.vst3` 包路径或散动态库路径 → 实际动态库路径；
/// ARA 会话打开与 MCP 绑定共用此口径，不另造第二份解析规则）。
///
/// # Examples
///
/// ```ignore
/// // .vst3 包 → 包内平台动态库；散动态库 → 自身；其余 → None。
/// ```
#[must_use]
pub(crate) fn resolve_module_path(path: &Path) -> Option<PathBuf> {
    resolve_candidate(path)
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

/// 按 `(os, arch)` 返回 `.vst3` 包内平台二进制目录候选（Steinberg VST3 规范顺序）。
fn bundle_subdirs_for(os: &str, arch: &str) -> &'static [&'static str] {
    match (os, arch) {
        ("linux", "x86_64") => &["Contents/x86_64-linux"],
        ("linux", "aarch64") => &["Contents/aarch64-linux"],
        ("linux", "x86") => &["Contents/i386-linux"],
        ("macos", _) => &["Contents/MacOS"],
        ("windows", "x86_64") => &["Contents/x86_64-win"],
        ("windows", "aarch64") => &["Contents/arm64x-win", "Contents/arm64-win"],
        ("windows", "x86") => &["Contents/x86-win"],
        _ => &[],
    }
}

/// 在给定候选子目录顺序中解析 `.vst3` 包内平台动态库。
fn resolve_bundle_binary_in(bundle: &Path, subdirs: &[&str]) -> Option<PathBuf> {
    if bundle.is_file() {
        return Some(bundle.to_path_buf());
    }
    for subdir_rel in subdirs {
        let Ok(entries) = std::fs::read_dir(bundle.join(subdir_rel)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && is_bundle_binary(path))
            .collect();
        files.sort();
        if let Some(found) = files.into_iter().next() {
            return Some(found);
        }
    }
    None
}

/// `.vst3` 包内平台动态库解析（支持 Linux `x86_64`/`aarch64`/`i386`、
/// macOS `Contents/MacOS`、Windows `x86_64`/`arm64x`/`arm64`/`x86`）。
/// 包路径本身是文件时（Windows 单文件 `.vst3`）直接用它。
fn resolve_bundle_binary(bundle: &Path) -> Option<PathBuf> {
    let subdirs = bundle_subdirs_for(std::env::consts::OS, std::env::consts::ARCH);
    resolve_bundle_binary_in(bundle, subdirs)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// P0-5 校验判据用的临时目录（pid＋纳秒，互不干扰）。
    fn p05_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-plughost-p05-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("建 P0-5 测试目录");
        dir
    }

    #[test]
    fn validate_plugin_path_accepts_in_root_module_and_bundle() {
        let root = p05_test_dir("accept");
        let module = root.join(format!("ok.{DYLIB_EXTENSION}"));
        std::fs::write(&module, b"stub").expect("写根内模块");
        let bundle = root.join("Bundle.vst3");
        std::fs::create_dir_all(&bundle).expect("建根内包");
        let roots = vec![root.clone()];
        assert_eq!(
            Host::validate_plugin_path_in_roots(module.to_str().unwrap(), &roots).ok(),
            module.canonicalize().ok(),
            "根内散动态库应放行并返回规范路径"
        );
        assert_eq!(
            Host::validate_plugin_path_in_roots(bundle.to_str().unwrap(), &roots).ok(),
            bundle.canonicalize().ok(),
            "根内 .vst3 包应放行并返回规范路径"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn validate_plugin_path_rejects_outside_bad_shape_and_missing() {
        let root = p05_test_dir("root");
        let outside = p05_test_dir("outside");
        let stray = outside.join(format!("stray.{DYLIB_EXTENSION}"));
        std::fs::write(&stray, b"stub").expect("写根外模块");
        let roots = vec![root.clone()];
        // 根外：形态对、文件在，但不在任何插件根内 ⇒ 拒绝。
        assert!(
            matches!(
                Host::validate_plugin_path_in_roots(stray.to_str().unwrap(), &roots),
                Err(HostError::InvalidPluginPath(_))
            ),
            "根外路径必须被拒绝"
        );
        // 形态不对：根内 `.txt` ⇒ 拒绝（不许拿任意文件喂 dlopen）。
        let text = root.join("notes.txt");
        std::fs::write(&text, b"stub").expect("写根内文本文件");
        assert!(
            matches!(
                Host::validate_plugin_path_in_roots(text.to_str().unwrap(), &roots),
                Err(HostError::InvalidPluginPath(_))
            ),
            "非插件扩展名必须被拒绝"
        );
        // 不存在：根内缺席的 `.so` ⇒ canonicalize 失败，响亮拒绝。
        let missing = root.join(format!("missing.{DYLIB_EXTENSION}"));
        assert!(
            matches!(
                Host::validate_plugin_path_in_roots(missing.to_str().unwrap(), &roots),
                Err(HostError::InvalidPluginPath(_))
            ),
            "不存在的路径必须被拒绝"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn validate_plugin_path_rejects_symlink_escape() {
        let root = p05_test_dir("linkroot");
        let outside = p05_test_dir("linkoutside");
        let target = outside.join(format!("target.{DYLIB_EXTENSION}"));
        std::fs::write(&target, b"stub").expect("写根外目标");
        let link = root.join(format!("escape.{DYLIB_EXTENSION}"));
        std::os::unix::fs::symlink(&target, &link).expect("建逃逸符号链接");
        let roots = vec![root.clone()];
        assert!(
            matches!(
                Host::validate_plugin_path_in_roots(link.to_str().unwrap(), &roots),
                Err(HostError::InvalidPluginPath(_))
            ),
            "根内符号链接指向根外必须被拒绝（校验在规范形态上做）"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn validate_plugin_path_passes_stub_prefix() {
        assert_eq!(
            Host::validate_plugin_path("stub:sine").ok(),
            Some(PathBuf::from("stub:sine")),
            "stub: 桩不是文件路径，应原样放行"
        );
    }

    #[test]
    fn validate_scan_dir_accepts_in_root_dir_and_subdir() {
        let root = p05_test_dir("scan-accept");
        let sub = root.join("nested");
        std::fs::create_dir_all(&sub).expect("建根内子目录");
        let roots = vec![root.clone()];
        assert_eq!(
            Host::validate_scan_dir_in_roots(&root, &roots).ok(),
            root.canonicalize().ok(),
            "插件根自身应放行并返回规范路径"
        );
        assert_eq!(
            Host::validate_scan_dir_in_roots(&sub, &roots).ok(),
            sub.canonicalize().ok(),
            "根内子目录应放行并返回规范路径"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn validate_scan_dir_rejects_outside_and_missing_dir() {
        let root = p05_test_dir("scan-root");
        let outside = p05_test_dir("scan-outside");
        let roots = vec![root.clone()];
        assert!(
            matches!(
                Host::validate_scan_dir_in_roots(&outside, &roots),
                Err(HostError::InvalidPluginPath(_))
            ),
            "根外目录必须被拒绝"
        );
        let missing = root.join("does-not-exist");
        assert!(
            matches!(
                Host::validate_scan_dir_in_roots(&missing, &roots),
                Err(HostError::InvalidPluginPath(_))
            ),
            "不存在的目录必须被拒绝（调用方按 scan 口径静默跳过，不把本错误当失败）"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn validate_scan_dir_rejects_symlink_escape() {
        let root = p05_test_dir("scan-linkroot");
        let outside = p05_test_dir("scan-linkoutside");
        let link = root.join("escape");
        std::os::unix::fs::symlink(&outside, &link).expect("建逃逸符号链接");
        let roots = vec![root.clone()];
        assert!(
            matches!(
                Host::validate_scan_dir_in_roots(&link, &roots),
                Err(HostError::InvalidPluginPath(_))
            ),
            "根内符号链接指向根外目录必须被拒绝（校验在规范形态上做）"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn bundle_subdirs_cover_all_supported_platforms() {
        assert_eq!(
            bundle_subdirs_for("linux", "x86_64"),
            &["Contents/x86_64-linux"]
        );
        assert_eq!(
            bundle_subdirs_for("linux", "aarch64"),
            &["Contents/aarch64-linux"]
        );
        assert_eq!(bundle_subdirs_for("linux", "x86"), &["Contents/i386-linux"]);
        assert_eq!(bundle_subdirs_for("macos", "x86_64"), &["Contents/MacOS"]);
        assert_eq!(bundle_subdirs_for("macos", "aarch64"), &["Contents/MacOS"]);
        assert_eq!(
            bundle_subdirs_for("windows", "x86_64"),
            &["Contents/x86_64-win"]
        );
        assert_eq!(
            bundle_subdirs_for("windows", "aarch64"),
            &["Contents/arm64x-win", "Contents/arm64-win"]
        );
        assert_eq!(bundle_subdirs_for("windows", "x86"), &["Contents/x86-win"]);
        assert!(bundle_subdirs_for("freebsd", "riscv64").is_empty());
    }

    #[test]
    fn resolve_bundle_binary_in_respects_subdir_precedence_and_sorting() {
        let root = std::env::temp_dir().join(format!(
            "yeban-bundle-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let bundle = root.join("Multi.vst3");
        let fallback_dir = bundle.join("Contents/fallback");
        let preferred_dir = bundle.join("Contents/preferred");
        std::fs::create_dir_all(&fallback_dir).expect("建回退目录");
        std::fs::create_dir_all(&preferred_dir).expect("建首选目录");

        #[cfg(target_os = "windows")]
        let (b_name, a_name) = ("b.vst3", "a.vst3");
        #[cfg(not(target_os = "windows"))]
        let (b_name, a_name) = (
            format!("b.{DYLIB_EXTENSION}"),
            format!("a.{DYLIB_EXTENSION}"),
        );

        let fallback_file = fallback_dir.join(&a_name);
        std::fs::write(&fallback_file, b"stub").expect("写回退文件");
        // 首选目录尚无文件时，应回退到第二候选目录。
        let found = resolve_bundle_binary_in(&bundle, &["Contents/preferred", "Contents/fallback"]);
        assert_eq!(found.as_deref(), Some(fallback_file.as_path()));

        // 首选目录出现多个文件时，按字典序取第一个并优先于回退目录。
        let pref_b = preferred_dir.join(&b_name);
        let pref_a = preferred_dir.join(&a_name);
        std::fs::write(&pref_b, b"stub").expect("写首选 b");
        std::fs::write(&pref_a, b"stub").expect("写首选 a");
        let found = resolve_bundle_binary_in(&bundle, &["Contents/preferred", "Contents/fallback"]);
        assert_eq!(found.as_deref(), Some(pref_a.as_path()));

        let _ = std::fs::remove_dir_all(&root);
    }
}
