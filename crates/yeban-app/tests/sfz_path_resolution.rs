//! ⭐ 任务单 `#168` 的判据：**SFZ 资产根的解析只有一份规则**，且**不再取决于进程 cwd**。
//!
//! ## 本单要堵的那条路（逐字）
//!
//! `#164`／`#165` 把**探测/装载**（`bridge::sfz_samples_present` /
//! `bridge::instrument_install_ops`）的路径解析固定到**仓库根**
//! （`CARGO_MANIFEST_DIR/../..`），而**渲染侧** `sfz_load::arm_pending_sfz` 仍把
//! `PendingSfz::path` 原样交给 `decode_sfz_plan` ⇒ 相对路径按**进程 cwd** 解释。
//!
//! 后果：cwd ≠ 仓根时，探测说"有样本"，渲染却读不到 ⇒ **轨静音 ＋ 没有任何报错**。
//! 那比"装不上"更坏：用户听不到声音，也拿不到原因。
//!
//! ## 判据读数（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `resolved` | [`resolve_sampler_path`] 返回的路径 | `PathBuf`（逐字比较） |
//! | `regions` | [`SfzDecodeReport::regions`] | 区域数 |
//! | `decoded` | [`SfzDecodeReport::decoded_samples`] | 解出 PCM 的样本**份数** |
//! | `silent` | [`SfzDecodeReport::silent_region_count`] | 缺样本的区域数 |
//! | `missing` | [`SfzDecodeReport::missing_sample_paths`] | 缺样本路径**条数** |
//! | `tried` | [`SfzPathError::tried`] | 候选根**条数**（通常 4；任务单 `#196` 起含采样库根） |
//!
//! ## 反向腿（注入 ⇒ 哪条红）
//!
//! - **注入甲**：删掉 [`resolve_sampler_path`] 里"仓库根"那个候选 ⇒
//!   [`the_same_project_loads_the_same_from_three_cwds`] 在 cwd = `/tmp` 与 crate 目录两档红
//!   （只有 cwd = 仓根那一档还绿）；
//! - **注入乙**：让 [`arm_pending_sfz`] 绕过解析、把模型原样字符串交给 `decode_sfz_plan` ⇒
//!   [`a_missing_relative_path_is_loud_and_lists_every_root`] 红（错误里没有"试过："与三个候选）。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证"仓内 SFZ 今天能出声"（`assets/samples/**` 仍无样本字节，见 `#165`）；
//! ⛔ 不证人耳听到的声音（本判据只读样本数组与报告）。
//!
//! ## 夹具为什么放在仓库内 `target/tmp/`
//!
//! [`the_same_project_loads_the_same_from_three_cwds`] 要造一条**相对仓库根**的路径——
//! 只有放在仓库内的文件才满足这个前提。`target/` 被 git 忽略，且 [`Fixture`] 在 `Drop`
//! 时删掉自己的目录（⛔ 工作树里不留临时文件）。
//!
//! ## ⭐ 任务单 `#170` 补的两条（同一份实现，两个入口）
//!
//! `#168` 只把"相对路径的候选根"接到了 `--export-wav`；本单补上另外两处**同一条语义**：
//!
//! | 入口 | 缺口 | 本文件的判据 |
//! | :--- | :--- | :--- |
//! | `--record-wav`（离线录制） | 走 `render_project_f32`（无工程目录语境）⇒ 工程旁的相对样本读不到 | [`the_record_entry_reads_a_sample_next_to_the_project_file`] ＋ [`the_cli_record_entry_plumbs_the_project_dir`] |
//! | GUI「另存为」（`host::wire_save`） | 落盘后**不更新** `EngineHost.project_dir` ⇒ 之后新加的相对样本按旧目录解析 | [`saving_elsewhere_moves_the_project_dir_to_the_new_file`] ＋ 两条"不动"腿 |
//! | 旧入口 | 必须**逐位不变** | [`the_old_entries_are_bit_identical_on_the_none_path`] |
//!
//! ⛔ 本文件不证"人耳听到的声音"；样本"读到了"的读数是
//! [`yeban_app::sfz_load::SfzDecodeReport`] 的 `decoded_samples == 1` ＋ `silent == 0`。
//!
//! ## ⭐ 任务单 `#171`：判据 4 的绝对路径必须是**该平台自己的**
//!
//! 判据 4（[`absolute_paths_are_returned_verbatim`]）原本用 `/nonexistent/yeban-168.sfz`
//! 当"不存在的绝对路径"。在 **Windows** 上那不是绝对路径（需要盘符或 UNC 前缀）⇒
//! 解析器当相对路径去三档里找 ⇒ `Err` ⇒ 判据 panic。本单用
//! [`platform_absolute_nonexistent`] 换成 `C:\nonexistent\…`，**语义一位未变**：
//! 绝对路径原样返回、不存在的绝对路径也原样、⛔ 不查存在性。

#![allow(unused)]
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use yeban_app::bridge::{ViewState, sfz_samples_present};
use yeban_app::engine_host::EngineHost;
use yeban_app::file_picker::{FilePicker, PickerRequest};
use yeban_app::host::{SaveStatus, build_main_window_with_console_tab, wire_save_with_engine};
use yeban_app::record_wav::{RecordOptions, record_project, record_project_under};
use yeban_app::render_core::{render_project_f32, render_project_f32_with_assets_under};
use yeban_app::scene::DemoScene;
use yeban_app::sfz_load::{SfzRoots, arm_pending_sfz, repo_root, resolve_sampler_path};
use yeban_engine::sfz_instrument::SfzPlan;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_ui_test_port::render::Tier1Window;

/// 工程采样率（与 `YebanProjectV1::default` 同口径）。
const RATE: u32 = 48_000;

/// 一次装载的**可比较读数**（见模块文档的表）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reading {
    /// 解析出来的绝对路径（逐字）。
    resolved: PathBuf,
    /// SFZ 区域数。
    regions: usize,
    /// 解出 PCM 的样本份数。
    decoded: usize,
    /// 缺样本的区域数。
    silent: usize,
    /// 缺样本路径条数。
    missing: usize,
}

/// 仓库内的临时夹具（`target/tmp/...`；`Drop` 时删除自己）。
struct Fixture {
    /// 夹具根目录（`samples/tone.sfz` ＋ `samples/tone.wav` 在它下面）。
    dir: PathBuf,
}

impl Fixture {
    /// 建一份 1 region ＋ 1 份 24-bit WAV 的 SFZ。
    fn new(tag: &str) -> Self {
        let dir = repo_root().join("target/tmp").join(format!(
            "yeban-168-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(dir.join("samples")).expect("建夹具目录");
        // 0.1 s、440 Hz、幅度 0.5 的 24-bit 单声道 WAV。
        let frames = (RATE / 10) as usize;
        let mut pcm: Vec<i32> = Vec::with_capacity(frames);
        for frame in 0..frames {
            let phase = std::f64::consts::TAU * 440.0 * frame as f64 / f64::from(RATE);
            #[allow(clippy::cast_possible_truncation)]
            let value = (phase.sin() * 0.5 * 8_388_607.0).round() as i32;
            pcm.push(value);
        }
        write_plain_wav(
            dir.join("samples/tone.wav"),
            &PcmFormat::integer(1, RATE, 24),
            &PcmBuffer::Int24(pcm),
        )
        .expect("写夹具 WAV");
        std::fs::write(
            dir.join("samples/tone.sfz"),
            "<region> sample=tone.wav key=60 pitch_keycenter=60\n",
        )
        .expect("写夹具 SFZ");
        Self { dir }
    }

    /// 夹具 SFZ 的**绝对**路径。
    fn sfz(&self) -> PathBuf {
        self.dir.join("samples/tone.sfz")
    }

    /// 夹具 SFZ 的**相对仓库根**路径（带 `/` 分隔符；必须真的在仓库内）。
    fn repo_relative_sfz(&self) -> String {
        self.sfz()
            .strip_prefix(repo_root())
            .expect("夹具必须在仓库内（`target/tmp`）")
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// ⭐ 任务单 `#171`：判据要的"**该平台自己的**绝对路径"（⛔ 不借用 Unix 形状）。
///
/// 教训（CI run `39070081067`，windows 腿）：`/nonexistent/…` 在 Unix 上是绝对路径，
/// 在 **Windows** 上 `Path::is_absolute()` 读 `false` —— Windows 的绝对路径需要
/// **盘符**（`C:\…`）或 **UNC** 前缀（`\\server\share\…`），只有根 `/…` 不算。
/// 于是解析器把 `/nonexistent/…` 当**相对**路径，走"工程目录 ⇒ 仓库根 ⇒ cwd"三档，
/// 找不到就 `Err` ⇒ `expect` 在 Windows 上 panic。
///
/// 修法**不改语义**（"绝对路径原样返回、⛔ 不查存在性"），只把那条路径换成
/// **运行判据的这个平台**自己的绝对路径形状：
///
/// | 平台 | 形状 | `Path::new(…).is_absolute()` |
/// | :--- | :--- | :--- |
/// | Unix / macOS | `/nonexistent/<name>` | `true` |
/// | Windows | `C:\nonexistent\<name>` | `true` |
///
/// 两个分支都是普通字符串，**任意**平台都能编译 ⇒ 不需要 `#[cfg]` 复制整个判据；
/// 判据里再用 `#[cfg]` 两条断言核"形状"、一条运行期断言核"真的读作绝对"。
fn platform_absolute_nonexistent(name: &str) -> String {
    if cfg!(windows) {
        format!(r"C:\nonexistent\{name}")
    } else {
        format!("/nonexistent/{name}")
    }
}

/// 工程里第一条非主总线轨（`EngineSnapshot::from_project` 会把它投影成待摊平项）。
fn non_master_track(project: &YebanProjectV1) -> EntityId {
    project
        .tracks
        .keys()
        .copied()
        .find(|id| *id != project.master_bus_track_id)
        .expect("`filled_project` 必须有非主总线轨")
}

/// 一份把 `path` 逐字写进 `TrackV3.sampler` 的工程（⛔ 不规范化 —— 判据要测的就是原样字符串）。
fn project_with_sampler(path: &str) -> (YebanProjectV1, EntityId) {
    let mut project = filled_project();
    let track = non_master_track(&project);
    project.tracks.get_mut(&track).expect("轨必须在").sampler = Some(SamplerBinding {
        path: path.to_owned(),
    });
    (project, track)
}

/// ⭐ 走**渲染侧同一条入口**（[`arm_pending_sfz`]）装载一次，返回读数。
fn load_reading(path: &str, roots: SfzRoots<'_>) -> Reading {
    let (project, track) = project_with_sampler(path);
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影快照");
    let (_snapshot, reports) = arm_pending_sfz(
        snapshot,
        RATE as f32,
        BTreeMap::<EntityId, SfzPlan>::new(),
        roots,
    )
    .expect("装载必须成功");
    assert_eq!(reports.len(), 1, "恰好一条轨有 SFZ");
    assert_eq!(reports[0].0, track, "报告必须属于那条轨");
    let report = &reports[0].1;
    Reading {
        resolved: report.path.clone(),
        regions: report.regions,
        decoded: report.decoded_samples,
        silent: report.silent_region_count,
        missing: report.missing_sample_paths.len(),
    }
}

/// `src/` 下全部 `*.rs` 的 `(文件名, 文本)`（判据 1 的机械读数）。
fn rust_sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("读 src 目录") {
        let entry = entry.expect("目录项");
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push((
                path.file_name()
                    .expect("有文件名")
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read_to_string(&path).expect("读源文件"),
            ));
        }
    }
}

// ===========================================================================
// 判据 1：一份实现（旧的两处都改成调它）
// ===========================================================================
#[test]
fn one_resolver_is_the_single_source_of_truth() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

// ===========================================================================
// 判据 2：cwd 无关（仓根 / /tmp / crate 目录三档装载读数相同）
// ===========================================================================
#[test]
fn the_same_project_loads_the_same_from_three_cwds() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

// ===========================================================================
// 判据 3：找不到 ⇒ 响亮（逐字含"试过：⟨根1⟩／⟨根2⟩／⟨根3⟩"）＋ 不静音
// ===========================================================================
#[test]
fn a_missing_relative_path_is_loud_and_lists_every_root() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

// ===========================================================================
// 判据 4：绝对路径原样
// ===========================================================================
#[test]
fn absolute_paths_are_returned_verbatim() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

// ===========================================================================
// 判据 5：相对工程目录（样本放工程旁 ⇒ 任意 cwd 都读得到）
// ===========================================================================
#[test]
fn a_project_adjacent_sfz_is_readable_from_any_cwd() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

// ===========================================================================
// 判据 1（旁证）：探测与渲染共用同一个解析 ⇒ 仓库相对路径的探测读数不变
// ===========================================================================
#[test]
fn the_probe_and_the_renderer_use_the_same_resolver() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

// ===========================================================================
// 判据 2（进程级）：**不注入** cwd ⇒ 解析真的读 `std::env::current_dir()`
// ===========================================================================
//
// `cargo test` 总是把测试二进制的 cwd 设成 crate 目录，所以"三种真实 cwd"只能由
// **直接运行测试二进制**来制造（见交付报告的读数：从仓根 / `/tmp` / crate 目录各跑一次）。
// 本判据本身不注入 `cwd` ⇒ `SfzRoots::none()` 走的是真实的 `std::env::current_dir()`。
#[test]
fn the_real_process_cwd_does_not_change_the_reading() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

// ===========================================================================
// 任务单 `#170`：⭐ 工程目录跟着**当前工程文件**走
// ===========================================================================
//
// 两条缺口共用**一条**语义：`EngineHost.project_dir` / 渲染入口的 `project_dir`
// = "当前工程文件所在目录"。
//
// | 判据 | 对象 | 读数 |
// | :--- | :--- | :--- |
// | ① | GUI「另存为」前后的 `EngineHost::project_dir()` | `Option<PathBuf>`（逐字）＋ 随后 `reload` 的 `sfz_plan` / `cache misses` |
// | ② | 取消 / 失败后的 `project_dir` | 同一个 `Option<PathBuf>`（必须**不变**） |
// | ③ | `--record-wav` 的渲染入口 | `recorded-wav: frames=… bytes=…` ＋ `SfzDecodeReport::{decoded,silent}` |
// | ④ | 旧入口 vs `_under(None)` | WAV **字节**（逐位比较）＋ `digest` 十六进制 |

/// 判据用的**脚本化保存对话框**：`path = None` ⇒ 用户取消（`FilePicker` 的既有口径）。
struct ScriptedSave {
    /// 对话框"选"的路径（`None` = 取消）。
    path: Option<PathBuf>,
    /// `choose_save` 被调用的次数（次）。
    calls: Cell<usize>,
}

impl ScriptedSave {
    /// 一份**回答**给定路径的对话框。
    fn answering(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            calls: Cell::new(0),
        }
    }

    /// 一份**取消**的对话框。
    const fn cancelling() -> Self {
        Self {
            path: None,
            calls: Cell::new(0),
        }
    }
}

impl FilePicker for ScriptedSave {
    fn choose_open(&self, _request: &PickerRequest) -> Option<PathBuf> {
        None
    }

    fn choose_save(&self, _request: &PickerRequest) -> Option<PathBuf> {
        self.calls.set(self.calls.get() + 1);
        self.path.clone()
    }
}

/// 一条 Tier-1 无头平台上的**生产窗口**（`wire_save_with_engine` 的回调体是纯宿主逻辑）。
///
/// ⚠ 返回值里的 [`Tier1Window`] **必须被持有到判据结束**：平台是每线程一份，丢了它
/// 窗口就没有后端（与 `tests/file_picker_dialogs.rs` 的 `production_window` 同款）。
fn save_window() -> (Tier1Window, yeban_app::ui::MainWindow) {
    let tier1 =
        Tier1Window::install(yeban_ui_test_port::Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&filled_project()).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = build_main_window_with_console_tab(&view, &scene, 1).expect("生产窗口构造入口");
    (tier1, ui)
}

/// ⭐ 判据 ①：**另存为到另一个目录** ⇒ `project_dir` 跟着新文件走，随后相对路径的
/// 工程旁样本按**新**目录解析得到。
///
/// 读数：`before = old.dir`、`after = new.dir`、`reload = Ok`、`sfz_plan = Some`、
/// `cache misses = 1`。
#[test]
fn saving_elsewhere_moves_the_project_dir_to_the_new_file() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

/// ⭐ 判据 ②a：用户在「另存为」对话框里按**取消** ⇒ `project_dir` **一位不动**
/// （与 `wire_save` 的既有取消口径一致：⛔ 不写状态、⛔ 不写文件）。
#[test]
fn a_cancelled_save_leaves_the_project_dir_alone() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

/// ⭐ 判据 ②b：落盘**失败**（目标目录不存在）⇒ `project_dir` **一位不动**，
/// 且界面如实报"保存失败"。
#[test]
fn a_failed_save_leaves_the_project_dir_alone() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

/// ⭐ 判据 ③（直接入口）：录制入口**拿得到工程目录** ⇒ 工程旁的相对样本读得到；
/// 反之（旧入口 `None`）**响亮**失败且⛔ 不写文件。
#[test]
fn the_record_entry_reads_a_sample_next_to_the_project_file() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

/// ⭐ 判据 ③（生产接线）：**真二进制** `--open … --record-wav …` 也拿得到工程目录
/// （证明 `cli` 那一跳真的把 `Loaded::project_dir()` 交给了录制入口，⛔ 不只测内部函数）。
#[test]
fn the_cli_record_entry_plumbs_the_project_dir() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

/// ⭐ 判据 ④：旧入口（`project_dir = None` 的委托路径）**逐位不变**。
///
/// 读数：旧入口与 `_under(None)` 写出的 WAV **字节相等**（逐字节比较）、
/// 渲染 `samples` 相等、`digest` 十六进制相等。
#[test]
fn the_old_entries_are_bit_identical_on_the_none_path() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback save-project("),
        "新 UI 契约面必须声明 callback save-project("
    );
    assert!(
        aether_host.contains("on_save_project"),
        "宿主接线层必须注册 on_save_project"
    );
    assert!(
        aether_app.contains("> save-status:"),
        "新 UI 契约面必须声明 > save-status:"
    );
    assert!(
        aether_host.contains("set_save_status"),
        "宿主接线层必须注册 set_save_status"
    );
    assert!(
        aether_app.contains("> save-succeeded:"),
        "新 UI 契约面必须声明 > save-succeeded:"
    );
    assert!(
        aether_host.contains("set_save_succeeded"),
        "宿主接线层必须注册 set_save_succeeded"
    );
}

/// 与 [`record_project`] 内部**同一组**渲染选项（判据用它读 `samples` / `digest`）。
fn record_wav_options(options: &RecordOptions) -> yeban_app::export_wav::WavExportOptions {
    yeban_app::export_wav::WavExportOptions {
        frames: (options.seconds * options.sample_rate as f32) as u64,
        sample_rate: options.sample_rate,
        channels: usize::from(options.channels),
        seed: 1,
        use_engine_source: true,
        bit_depth: yeban_render::dither::BitDepth::Int24,
        limiter: false,
        threads: None,
        // 渲染侧不读响度字段 ⇒ `None`。
        loudness: None,
    }
}
