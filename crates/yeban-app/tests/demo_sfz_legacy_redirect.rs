//! ⭐ T-FIX-723：历史 demo 工程写死的 `_suite/emily_basic.sfz` 必须能解析到
//! 现行资源 `karoryfer-emilyguitar/Emilyguitar/emily_basic.sfz`。
//!
//! ## 为什么在解析收口修
//!
//! 远程 demo（《雨声太吵》，从 `gradetwo/yeban-extras` 下载）的工程文件把 emily_basic
//! 写死成 `assets/samples/_suite/emily_basic.sfz`（早期 bundling 布局残留），但本仓
//! 现行资源的登记路径是 `assets/samples/karoryfer-emilyguitar/Emilyguitar/emily_basic.sfz`。
//! demo 工程文件**不随本仓分发**，无法改源引用，故在唯一解析收口 `resolve_sampler_path`
//! 做精确重定向 —— 打开校验、渲染导出、缺音源救援三处同源。
//!
//! ## 判据读数（真行为，⛔ 非 grep）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `resolved` | `resolve_sampler_path` 返回 | `PathBuf`（逐字） |
//! | `decoded` | `SfzDecodeReport::decoded_samples` | 解出 PCM 的样本份数 |
//! | `silent` | `SfzDecodeReport::silent_region_count` | 缺样本区域数 |
//! | `path()` | `SfzPathError::path` | 用户写的原样字符串 |
//! | `tried()` | `SfzPathError::tried` | 真正找过的候选路径 |
//!
//! ## 反向腿（注入 ⇒ 哪条红）
//!
//! 删掉 `resolve_sampler_path` 里的 `legacy_sfz_redirect(...)` 映射 ⇒
//! [`a_redirected_demo_path_resolves_to_the_current_library_asset`] 与
//! [`the_export_entry_decodes_the_redirected_demo_asset`] 红（历史路径找不到现行资源）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::sfz_load::{
    SfzRoots, arm_pending_sfz, legacy_sfz_redirect, repo_root, resolve_sampler_path,
};
use yeban_engine::sfz_instrument::SfzPlan;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// 历史 demo 工程里写死的那条路径。
const LEGACY: &str = "assets/samples/_suite/emily_basic.sfz";
/// 本仓现行资源的登记路径（manifest 与 builtin_sounds 同口径）。
const CURRENT: &str = "assets/samples/karoryfer-emilyguitar/Emilyguitar/emily_basic.sfz";
/// 库内布局：剥掉 `assets/samples/` 前缀。
const CURRENT_IN_LIBRARY: &str = "karoryfer-emilyguitar/Emilyguitar/emily_basic.sfz";
const RATE: u32 = 48_000;

/// 临时库根 + 空 cwd；`Drop` 时删目录（⛔ 工作树不留临时文件）。
struct LibRoot {
    dir: PathBuf,
    empty_cwd: PathBuf,
}

impl LibRoot {
    /// 默认在库根里放好现行资源（剥掉 `assets/samples/` 前缀的库内布局）。
    fn with_current_asset(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "yeban-723-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let dir = base.join("library");
        let empty_cwd = base.join("cwd");
        let asset_dir = dir.join("karoryfer-emilyguitar/Emilyguitar");
        std::fs::create_dir_all(&asset_dir).expect("建库内乐器目录");
        std::fs::create_dir_all(&empty_cwd).expect("建空 cwd");
        // 1 region + 1 份 0.1 s 的 24-bit 单声道 WAV（静音即可，判据读「解出」不是「好听」）。
        let frames = (RATE / 10) as usize;
        write_plain_wav(
            asset_dir.join("tone.wav"),
            &PcmFormat::integer(1, RATE, 24),
            &PcmBuffer::Int24(vec![0; frames]),
        )
        .expect("写样本 WAV");
        std::fs::write(
            asset_dir.join("emily_basic.sfz"),
            "<region> sample=tone.wav key=60 pitch_keycenter=60\n",
        )
        .expect("写现行 SFZ");
        Self { dir, empty_cwd }
    }

    /// 造一个**没有任何资源**的库根（验证缺失仍响亮）。
    fn empty(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "yeban-723-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        let dir = base.join("library");
        let empty_cwd = base.join("cwd");
        std::fs::create_dir_all(&dir).expect("建空库根");
        std::fs::create_dir_all(&empty_cwd).expect("建空 cwd");
        Self { dir, empty_cwd }
    }

    fn roots(&self) -> SfzRoots<'_> {
        SfzRoots::none()
            .with_library_root(&self.dir)
            .with_cwd(&self.empty_cwd)
    }

    fn current_asset_path(&self) -> PathBuf {
        self.dir.join(CURRENT_IN_LIBRARY)
    }
}

impl Drop for LibRoot {
    fn drop(&mut self) {
        if let Some(base) = self.dir.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

#[test]
fn legacy_redirect_maps_only_the_known_demo_path() {
    assert_eq!(legacy_sfz_redirect(LEGACY), Some(CURRENT));
    // 现行路径本身不受影响（幂等，不二次重定向）。
    assert_eq!(legacy_sfz_redirect(CURRENT), None);
    // 同目录的其它文件不在表里 ⇒ ⛔ 不误伤。
    assert_eq!(legacy_sfz_redirect("assets/samples/_suite/other.sfz"), None);
    assert_eq!(legacy_sfz_redirect("assets/samples/_suite/"), None);
    assert_eq!(legacy_sfz_redirect(""), None);
}

#[test]
fn windows_backslashes_are_normalized_before_matching() {
    let with_backslash = LEGACY.replace('/', "\\");
    assert_eq!(legacy_sfz_redirect(&with_backslash), Some(CURRENT));
}

#[test]
fn a_redirected_demo_path_resolves_to_the_current_library_asset() {
    let lib = LibRoot::with_current_asset("resolve");
    let resolved = resolve_sampler_path(LEGACY, lib.roots()).expect("历史路径经重定向必须能解析");
    assert_eq!(
        resolved,
        lib.current_asset_path(),
        "必须解析到库根下的现行资源（剥掉 assets/samples/ 前缀的布局）"
    );
}

#[test]
fn a_redirected_path_falls_back_to_the_repo_root_asset() {
    // 空库根：现行资源在仓库根真实存在（SFZ 文本在仓里，仅无样本字节）。
    // 原始 `_suite` 路径在仓里零命中 —— 能解析成功本身就证明重定向生效。
    let lib = LibRoot::empty("repo-fallback");
    let resolved = resolve_sampler_path(LEGACY, lib.roots()).expect("仓库根兜底必须解析成功");
    assert_eq!(
        resolved,
        repo_root().join(CURRENT),
        "必须回落到仓库根下的现行资源（不是空库根里的路径）"
    );
}

#[test]
fn a_still_missing_redirected_path_is_loud() {
    // 造一条重定向表里没有的历史路径 ⇒ 重定向不命中，仍走原样路径 ⇒ 必然找不到。
    // 这条验证重定向机制没有破坏既有的「响亮失败」语义（⛔ 不静默静音）。
    let lib = LibRoot::empty("missing");
    let ghost = "assets/samples/_suite/ghost_missing.sfz";
    assert_eq!(legacy_sfz_redirect(ghost), None, "幽灵路径不在重定向表里");
    let err = resolve_sampler_path(ghost, lib.roots()).expect_err("资源缺失必须响亮失败");
    assert_eq!(
        err.path(),
        ghost,
        "错误里的 path 必须是用户写的原样字符串（响亮、不骗人）"
    );
    assert!(
        !err.tried().is_empty(),
        "tried 必须列出真正找过的候选位置：{:?}",
        err.tried()
    );
    let msg = err.to_string();
    assert!(msg.contains("找不到"), "错误文案要明说找不到：{msg}");
}

#[test]
fn absolute_paths_are_never_redirected() {
    // 绝对路径规则 1 优先，⛔ 不重定向、⛔ 不查存在性，逐字返回。
    let absolute = if cfg!(windows) {
        r"C:\nonexistent\emily_basic.sfz".to_owned()
    } else {
        "/nonexistent/emily_basic.sfz".to_owned()
    };
    assert_eq!(legacy_sfz_redirect(&absolute), None);
    let resolved =
        resolve_sampler_path(&absolute, SfzRoots::none()).expect("绝对路径原样返回（不查存在性）");
    assert_eq!(resolved, Path::new(&absolute));
}

/// 工程里第一条非主总线轨（与 `sfz_path_resolution.rs` 同口径）。
fn non_master_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .keys()
        .copied()
        .find(|id| *id != project.master_bus_track_id)
        .expect("filled_project 必须有非主总线轨")
}

#[test]
fn the_export_entry_decodes_the_redirected_demo_asset() {
    let lib = LibRoot::with_current_asset("export");
    let mut project = filled_project();
    let track = non_master_track(&project);
    project.tracks.get_mut(&track).expect("轨必须在").sampler = Some(SamplerBinding {
        note_octave_override: None,
        path: LEGACY.to_owned(),
    });
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影快照");
    let (_snapshot, reports) = arm_pending_sfz(
        snapshot,
        RATE as f32,
        BTreeMap::<EntityId, SfzPlan>::new(),
        lib.roots(),
    )
    .expect("导出侧走同一解析器，历史路径必须能装载（不再 RENDER_FAILED）");
    assert_eq!(reports.len(), 1, "恰好一条轨有 SFZ");
    let report = &reports[0].1;
    assert_eq!(report.decoded_samples, 1, "现行资源必须解出 1 份 PCM");
    assert_eq!(report.silent_region_count, 0, "⛔ 不许静音");
    assert_eq!(
        report.path,
        lib.current_asset_path(),
        "装载的必须是重定向后的现行资源"
    );
}
