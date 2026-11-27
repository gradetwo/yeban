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

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

mod common;

use yeban_app::bridge::{InstrumentSampleProbe, ViewState, probe_instrument_samples};
use yeban_app::engine_host::EngineHost;
use yeban_app::file_picker::{FilePicker, PickerRequest};
use yeban_app::host::{
    SaveStatus, apply_language, build_main_window_with_console_tab, wire_save_with_engine,
};
use yeban_app::record_wav::{RecordOptions, record_project, record_project_under};
use yeban_app::render_core::{render_project_f32, render_project_f32_with_assets_under};
use yeban_app::scene::DemoScene;
use yeban_app::sfz_load::{SfzRoots, arm_pending_sfz, repo_root, resolve_sampler_path};
use yeban_engine::sfz_instrument::SfzPlan;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::container::write_project_container;
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

/// 在 `dir/samples` 下写一份 1 region ＋ 1 份 24-bit WAV 的 SFZ（幅度可调）。
fn write_fixture(dir: &Path, amplitude: f64) {
    std::fs::create_dir_all(dir.join("samples")).expect("建夹具目录");
    // 0.1 s、440 Hz、给定幅度的 24-bit 单声道 WAV。
    let frames = (RATE / 10) as usize;
    let mut pcm: Vec<i32> = Vec::with_capacity(frames);
    for frame in 0..frames {
        let phase = std::f64::consts::TAU * 440.0 * frame as f64 / f64::from(RATE);
        #[allow(clippy::cast_possible_truncation)]
        let value = (phase.sin() * amplitude * 8_388_607.0).round() as i32;
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
}

/// 仓库内的临时夹具（`target/tmp/...`；`Drop` 时删除自己）。
struct Fixture {
    /// 夹具根目录（`samples/tone.sfz` ＋ `samples/tone.wav` 在它下面）。
    dir: PathBuf,
}

impl Fixture {
    /// 建一份 1 region ＋ 1 份 24-bit WAV 的 SFZ（幅度 0.5）。
    fn new(tag: &str) -> Self {
        Self::new_with_amplitude(tag, 0.5)
    }

    /// 建一份同形状、但样本幅度可调的 SFZ（判据用它造"同名不同字节"的诱饵）。
    fn new_with_amplitude(tag: &str, amplitude: f64) -> Self {
        let dir = repo_root().join("target/tmp").join(format!(
            "yeban-168-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        write_fixture(&dir, amplitude);
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
        note_octave_override: None,
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

/// 临时目录 + 唯一标签（`Drop` 由调用方删；⛔ 工作树里不留临时文件）。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-168-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

// ===========================================================================
// 判据 1：一份实现（探测与渲染都调同一个解析）
// ===========================================================================
#[test]
fn one_resolver_is_the_single_source_of_truth() {
    let fixture = Fixture::new("one-resolver");
    let literal = fixture.sfz().to_string_lossy().into_owned();
    let resolved = resolve_sampler_path(&literal, SfzRoots::none()).expect("绝对 SFZ 必须解析");
    assert_eq!(resolved, fixture.sfz(), "绝对路径原样就是夹具路径");
    assert_eq!(
        probe_instrument_samples(&literal),
        InstrumentSampleProbe::Installable { decoded_samples: 1 },
        "探测必须认可这份夹具"
    );
    let reading = load_reading(&literal, SfzRoots::none());
    assert_eq!(reading.resolved, resolved, "探测与渲染必须是同一条路径");
    assert_eq!(
        (
            reading.regions,
            reading.decoded,
            reading.silent,
            reading.missing
        ),
        (1, 1, 0, 0),
        "1 region、1 份解码、0 静音、0 缺样本"
    );
    // 仓库相对路径：仓库根只有 `repo_root()` 一份取值。
    let relative = fixture.repo_relative_sfz();
    let expected = repo_root().join(&relative);
    assert_eq!(
        resolve_sampler_path(&relative, SfzRoots::none()).expect("仓库相对路径必须解析"),
        expected,
        "仓库根候选必须来自 `repo_root()`"
    );
    let relative_reading = load_reading(&relative, SfzRoots::none());
    assert_eq!(relative_reading.resolved, expected);
    assert_eq!(relative_reading.decoded, 1);
}

// ===========================================================================
// 判据 2：cwd 无关（仓根 / 临时目录 / crate 目录三档装载读数相同）
// ===========================================================================
#[test]
fn the_same_project_loads_the_same_from_three_cwds() {
    let fixture = Fixture::new("three-cwds");
    let relative = fixture.repo_relative_sfz();
    let temp = scratch_dir("three-cwds-tmp");
    let cwds = [
        repo_root().to_path_buf(),
        temp.clone(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    ];
    let readings: Vec<Reading> = cwds
        .iter()
        .map(|cwd| load_reading(&relative, SfzRoots::none().with_cwd(cwd)))
        .collect();
    for (index, reading) in readings.iter().enumerate() {
        assert_eq!(
            reading, &readings[0],
            "第 {index} 档 cwd 的读数必须与仓根那一档逐项相同"
        );
        assert_eq!(
            reading.resolved,
            fixture.sfz(),
            "第 {index} 档 cwd 必须解析到仓库内同一个文件"
        );
    }
    assert_eq!(
        readings[0],
        Reading {
            resolved: fixture.sfz(),
            regions: 1,
            decoded: 1,
            silent: 0,
            missing: 0
        }
    );
    std::fs::remove_dir_all(&temp).ok();
}

// ===========================================================================
// 判据 3：找不到 ⇒ 响亮（逐字含"试过：⟨根1⟩／⟨根2⟩／⟨根3⟩"）＋ 列出全部候选
// ===========================================================================
#[test]
fn a_missing_relative_path_is_loud_and_lists_every_root() {
    let project_dir = scratch_dir("loud-project");
    let library_root = scratch_dir("loud-library");
    let cwd = scratch_dir("loud-cwd");
    let missing = "assets/samples/t890-missing/tone.sfz";
    let roots = SfzRoots {
        project_dir: Some(&project_dir),
        library_root: Some(&library_root),
        cwd: Some(&cwd),
    };
    let error = resolve_sampler_path(missing, roots).expect_err("这份路径在任何根下都不存在");
    assert_eq!(error.path(), missing, "错误必须回显用户原样写下的路径");
    let expected: Vec<PathBuf> = vec![
        project_dir.join(missing),
        library_root.join("t890-missing/tone.sfz"),
        repo_root().join(missing),
        cwd.join(missing),
    ];
    assert_eq!(
        error.tried(),
        expected.as_slice(),
        "候选必须按规则 2 的顺序列全"
    );
    assert_eq!(
        error.tried().len(),
        4,
        "工程目录 / 库根 / 仓库根 / cwd 四条"
    );
    let text = error.to_string();
    assert!(
        text.starts_with(&format!("SFZ 路径 `{missing}` 找不到；试过：")),
        "错误开头必须点名路径并说试过哪些根: {text}"
    );
    for candidate in &expected {
        assert!(
            text.contains(&candidate.display().to_string()),
            "错误必须逐条列出候选 {}: {text}",
            candidate.display()
        );
    }
    assert_eq!(text.matches('／').count(), 3, "四个候选之间恰好三个分隔符");

    // 渲染侧同一条语义：`arm_pending_sfz` 也必须响亮，而不是静默静音。
    let (project, _track) = project_with_sampler(missing);
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影快照");
    let error = arm_pending_sfz(
        snapshot,
        RATE as f32,
        BTreeMap::<EntityId, SfzPlan>::new(),
        roots,
    )
    .expect_err("渲染侧也必须报错");
    let render_text = error.to_string();
    assert!(
        render_text.contains("试过：") && render_text.contains(missing),
        "渲染侧错误必须带路径与试过的根: {render_text}"
    );
    for dir in [&project_dir, &library_root, &cwd] {
        assert!(
            render_text.contains(&dir.display().to_string()),
            "渲染侧错误也必须列出 {}: {render_text}",
            dir.display()
        );
    }
    std::fs::remove_dir_all(&project_dir).ok();
    std::fs::remove_dir_all(&library_root).ok();
    std::fs::remove_dir_all(&cwd).ok();
}

// ===========================================================================
// 判据 4：绝对路径原样（不查存在性）
// ===========================================================================
#[test]
fn absolute_paths_are_returned_verbatim() {
    let path = platform_absolute_nonexistent("yeban-168.sfz");
    assert!(
        Path::new(&path).is_absolute(),
        "判据用的路径必须是本平台自己的绝对形状: {path}"
    );
    #[cfg(windows)]
    assert!(path.starts_with(r"C:\"), "Windows 形状必须带盘符: {path}");
    #[cfg(not(windows))]
    assert_eq!(path, "/nonexistent/yeban-168.sfz");
    let cwd = scratch_dir("absolute-cwd");
    let roots = SfzRoots::none().with_cwd(&cwd);
    let resolved = resolve_sampler_path(&path, roots).expect("绝对路径必须原样返回，⛔ 不查存在性");
    assert_eq!(resolved, PathBuf::from(&path), "必须逐字相同");
    assert!(!resolved.exists(), "判据故意用一条不存在的绝对路径");
    // 存在的绝对路径同样原样（工程目录在场时也不被改写）。
    let fixture = Fixture::new("absolute-existing");
    let literal = fixture.sfz().to_string_lossy().into_owned();
    assert_eq!(
        resolve_sampler_path(&literal, SfzRoots::under_project(&fixture.dir)).expect("绝对路径"),
        fixture.sfz()
    );
    std::fs::remove_dir_all(&cwd).ok();
}

// ===========================================================================
// 判据 5：相对工程目录（样本放工程旁 ⇒ 任意 cwd 都读得到，且优先于 cwd 同名文件）
// ===========================================================================
#[test]
fn a_project_adjacent_sfz_is_readable_from_any_cwd() {
    let fixture = Fixture::new("adjacent");
    // 诱饵：cwd 下有一条**同名不同字节**的文件 —— 工程目录必须胜出。
    let decoy = scratch_dir("adjacent-decoy");
    std::fs::create_dir_all(decoy.join("samples")).expect("建诱饵目录");
    std::fs::write(
        decoy.join("samples/tone.sfz"),
        "<region> sample=decoy.wav key=60 pitch_keycenter=60\n",
    )
    .expect("写诱饵 SFZ");
    let relative = "samples/tone.sfz";
    for cwd in [decoy.clone(), scratch_dir("adjacent-other")] {
        let reading = load_reading(
            relative,
            SfzRoots::under_project(&fixture.dir).with_cwd(&cwd),
        );
        assert_eq!(
            reading.resolved,
            fixture.sfz(),
            "工程旁的样本必须按工程目录解析，而不是 cwd"
        );
        assert_eq!(
            (
                reading.regions,
                reading.decoded,
                reading.silent,
                reading.missing
            ),
            (1, 1, 0, 0),
            "工程旁样本必须真的解出 PCM"
        );
        std::fs::remove_dir_all(&cwd).ok();
    }
}

// ===========================================================================
// 判据 1（旁证）：探测与渲染共用同一个解析 ⇒ 仓库相对路径的探测读数不变
// ===========================================================================
#[test]
fn the_probe_and_the_renderer_use_the_same_resolver() {
    let fixture = Fixture::new("probe-renderer");
    let relative = fixture.repo_relative_sfz();
    assert_eq!(
        probe_instrument_samples(&relative),
        InstrumentSampleProbe::Installable { decoded_samples: 1 },
        "仓库相对路径的探测必须可装"
    );
    let resolved = resolve_sampler_path(&relative, SfzRoots::none()).expect("唯一解析");
    let reading = load_reading(&relative, SfzRoots::none());
    assert_eq!(reading.resolved, resolved, "探测与渲染必须是同一条解析结果");
    assert_eq!(
        (
            reading.regions,
            reading.decoded,
            reading.silent,
            reading.missing
        ),
        (1, 1, 0, 0)
    );
}

// ===========================================================================
// 判据 2（进程级）：**不注入** cwd ⇒ 解析真的读 `std::env::current_dir()`
// ===========================================================================
#[test]
fn the_real_process_cwd_does_not_change_the_reading() {
    let fixture = Fixture::new("real-cwd");
    let relative = fixture.repo_relative_sfz();
    let real = std::env::current_dir().expect("读进程 cwd");
    assert_eq!(
        real.canonicalize().ok(),
        Some(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .canonicalize()
                .expect("crate 目录")
        ),
        "`cargo test` 的进程 cwd 是 crate 目录"
    );
    let real_reading = load_reading(&relative, SfzRoots::none());
    let other = scratch_dir("real-cwd-other");
    let injected_reading = load_reading(&relative, SfzRoots::none().with_cwd(&other));
    assert_eq!(
        real_reading, injected_reading,
        "真实 cwd 与注入的另一档 cwd 必须给出同一份读数"
    );
    assert_eq!(real_reading.resolved, fixture.sfz());
    assert_eq!(real_reading.decoded, 1);
    std::fs::remove_dir_all(&other).ok();
}

// ===========================================================================
// 任务单 `#170`：⭐ 工程目录跟着**当前工程文件**走
// ===========================================================================

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

/// 装配「保存」回调（旧入口 `wire_save` 的同一条落点）。
fn wire_save(
    ui: &yeban_app::ui::MainWindow,
    target: Option<PathBuf>,
    picker: Rc<dyn FilePicker>,
    engine: Rc<RefCell<EngineHost>>,
) {
    wire_save_with_engine(
        ui,
        SaveStatus {
            target,
            project: Rc::new(RefCell::new(filled_project())),
        },
        None,
        picker,
        None,
        Some(engine),
    );
}

/// 一个装过旧工程路径的宿主（判据要读"目录动没动"）。
fn engine_at(path: &Path) -> Rc<RefCell<EngineHost>> {
    let engine = Rc::new(RefCell::new(EngineHost::new()));
    engine
        .borrow_mut()
        .set_project_path(Some(path.to_path_buf()));
    engine
}

// ===========================================================================
// 判据 ①：**另存为到另一个目录** ⇒ `project_dir` 跟着新文件走，随后相对路径的
//         工程旁样本按**新**目录解析得到。
// ===========================================================================
#[test]
fn saving_elsewhere_moves_the_project_dir_to_the_new_file() {
    let fixture = Fixture::new("save-elsewhere");
    let old_dir = scratch_dir("save-elsewhere-old");
    let old_file = old_dir.join("old.yeban");
    let new_file = fixture.dir.join("saved-as-168.yeban");
    let engine = engine_at(&old_file);
    let (tier1, ui) = save_window();
    let picker = Rc::new(ScriptedSave::answering(new_file.clone()));
    let dyn_picker: Rc<dyn FilePicker> = picker.clone();
    wire_save(&ui, Some(old_file.clone()), dyn_picker, Rc::clone(&engine));
    ui.invoke_save_project_as();
    assert_eq!(picker.calls.get(), 1, "另存为问了一次对话框");
    assert!(
        new_file.is_file(),
        "另存为必须真的落盘: {}",
        new_file.display()
    );
    assert_eq!(
        engine.borrow().project_dir(),
        Some(fixture.dir.as_path()),
        "落盘成功后工程目录必须跟着新文件"
    );
    assert_eq!(
        engine.borrow().project_path(),
        Some(new_file.as_path()),
        "真实文件名也要跟着新文件（导出护栏读它）"
    );
    // 新目录里的相对样本：随后的 reload 必须读得到（目录没跟着走 ⇒ 解析失败）。
    let (project, _track) = project_with_sampler("samples/tone.sfz");
    engine
        .borrow_mut()
        .reload(&project, 1)
        .expect("工程目录已跟着新文件 ⇒ 工程旁样本必须可解析");
    std::fs::remove_dir_all(&old_dir).ok();
    let _ = tier1;
}

// ===========================================================================
// 判据 ②a：用户按**取消** ⇒ `project_dir` **一位不动**（⛔ 不写状态、⛔ 不写文件）
// ===========================================================================
#[test]
fn a_cancelled_save_leaves_the_project_dir_alone() {
    let old_dir = scratch_dir("cancel-old");
    let old_file = old_dir.join("old.yeban");
    let engine = engine_at(&old_file);
    let (tier1, ui) = save_window();
    let picker = Rc::new(ScriptedSave::cancelling());
    let dyn_picker: Rc<dyn FilePicker> = picker.clone();
    let before = ui.get_save_status();
    wire_save(&ui, None, dyn_picker, Rc::clone(&engine));
    ui.invoke_save_project_as();
    assert_eq!(picker.calls.get(), 1, "对话框被问了一次");
    assert_eq!(
        engine.borrow().project_dir(),
        Some(old_dir.as_path()),
        "取消不动工程目录"
    );
    assert_eq!(ui.get_save_status(), before, "取消⛔ 不写保存回执");
    std::fs::remove_dir_all(&old_dir).ok();
    let _ = tier1;
}

// ===========================================================================
// 判据 ②b：落盘**失败**（目标目录不存在）⇒ `project_dir` **一位不动**，
//          且界面如实报"保存失败"
// ===========================================================================
#[test]
fn a_failed_save_leaves_the_project_dir_alone() {
    let old_dir = scratch_dir("failed-old");
    let old_file = old_dir.join("old.yeban");
    let engine = engine_at(&old_file);
    let (tier1, ui) = save_window();
    apply_language(&ui, yeban_app::cli::Language::ZhCn);
    let missing_dir = scratch_dir("failed-target").join("no-such-subdir");
    let bad_target = missing_dir.join("boom.yeban");
    let picker = Rc::new(ScriptedSave::answering(bad_target.clone()));
    let dyn_picker: Rc<dyn FilePicker> = picker.clone();
    wire_save(&ui, Some(old_file), dyn_picker, Rc::clone(&engine));
    ui.invoke_save_project_as();
    assert!(!bad_target.exists(), "落盘失败⛔ 不留文件");
    assert_eq!(
        engine.borrow().project_dir(),
        Some(old_dir.as_path()),
        "失败不动工程目录"
    );
    let status = ui.get_save_status().to_string();
    assert!(
        status.starts_with("保存失败："),
        "界面必须如实报保存失败: {status}"
    );
    assert!(!ui.get_save_succeeded(), "失败必须标成未成功");
    std::fs::remove_dir_all(&old_dir).ok();
    let _ = tier1;
}

// ===========================================================================
// 判据 ③（直接入口）：录制入口**拿得到工程目录** ⇒ 工程旁的相对样本读得到；
//          反之（旧入口 `None`）**响亮**失败且⛔ 不写文件。
// ===========================================================================
#[test]
fn the_record_entry_reads_a_sample_next_to_the_project_file() {
    let fixture = Fixture::new("record-entry");
    let (project, _track) = project_with_sampler("samples/tone.sfz");
    let options = RecordOptions {
        seconds: 0.1,
        channels: 1,
        ..RecordOptions::default()
    };
    let out = fixture.dir.join("take.wav");
    let report = record_project_under(&project, &out, &options, None, Some(fixture.dir.as_path()))
        .expect("工程旁的样本必须读得到");
    assert_eq!(report.frames, 4_800, "0.1 s @ 48 kHz");
    assert_eq!(
        report.bytes,
        std::fs::metadata(&out).expect("WAV 必须落盘").len()
    );
    assert!(
        report
            .source
            .contains("sfz(tracks=1 regions=1 samples=1 silent=0"),
        "来源必须如实报「读到 1 份、0 静音」: {}",
        report.source
    );
    // 旧入口（`None`）：同一条相对路径不认识工程目录 ⇒ 响亮失败，⛔ 静默出静音。
    let old_out = fixture.dir.join("old-take.wav");
    let error = record_project(&project, &old_out, &options, None)
        .expect_err("旧入口读不到工程旁样本时必须报错");
    let text = error.to_string();
    assert!(
        text.contains("试过：") && text.contains("samples/tone.sfz"),
        "旧入口的错误必须响亮列出试过的根: {text}"
    );
    assert!(!old_out.exists(), "失败⛔ 不写半成品 WAV");
}

// ===========================================================================
// 判据 ③（生产接线）：**真二进制** `--open … --record-wav …` 也拿得到工程目录
// ===========================================================================
#[test]
fn the_cli_record_entry_plumbs_the_project_dir() {
    let fixture = Fixture::new("cli-record");
    let (mut project, _track) = project_with_sampler("samples/tone.sfz");
    // `--open` 把缺资产当硬错；本判据只测目录接线 ⇒ 把音频片段换成空 MIDI 片段。
    for entry in project.clip_pool.values_mut() {
        if matches!(entry.content, yeban_model::ClipContent::Audio { .. }) {
            entry.content = yeban_model::ClipContent::Midi {
                notes: BTreeMap::new(),
            };
        }
    }
    let bytes = write_project_container(
        &project,
        b"t890-cli-history",
        &BTreeMap::<yeban_model::AssetHash, Vec<u8>>::new(),
    )
    .expect("写容器工程");
    let project_file = fixture.dir.join("song.yeban");
    std::fs::write(&project_file, &bytes).expect("写工程文件");
    let out = fixture.dir.join("cli-take.wav");
    // cwd 故意放在一个**没有** samples 的目录：只有 `--open` 的父目录能解释这条相对路径。
    let cwd = scratch_dir("cli-record-cwd");
    let bin = common::bin_path();
    let output = Command::new(&bin)
        .current_dir(&cwd)
        .args([
            "--open",
            project_file.to_str().expect("UTF-8 工程路径"),
            "--record-wav",
            out.to_str().expect("UTF-8 录制路径"),
            "--record-seconds",
            "0.2",
        ])
        .output()
        .expect("执行真二进制");
    assert!(
        output.status.success(),
        "`--open … --record-wav` 必须成功: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|line| line.starts_with("recorded-wav:"))
        .unwrap_or_else(|| panic!("stdout 必须有 recorded-wav 行: {stdout}"));
    assert!(
        line.contains("sfz(tracks=1 regions=1 samples=1 silent=0"),
        "CLI 那一跳必须把工程目录交给录制入口（否则样本解析不到）: {line}"
    );
    assert!(out.is_file(), "录制必须落盘: {}", out.display());
    std::fs::remove_dir_all(&cwd).ok();
}

// ===========================================================================
// 判据 ④：旧入口（`project_dir = None` 的委托路径）**逐位不变**
// ===========================================================================
#[test]
fn the_old_entries_are_bit_identical_on_the_none_path() {
    // 题面：没有采样器的工程 + 采样器指向**仓库内相对路径**的工程，两条腿都要逐位相同。
    let plain = filled_project();
    let options = RecordOptions {
        seconds: 0.1,
        channels: 1,
        ..RecordOptions::default()
    };
    let wav_options = record_wav_options(&options);
    let base = render_project_f32(&plain, wav_options, None).expect("旧渲染入口");
    let under = render_project_f32_with_assets_under(&plain, wav_options, None, &[], None)
        .expect("`_under(None)` 渲染入口");
    assert_eq!(base.0, under.samples, "样本必须逐位相同");
    assert_eq!(base.1, under.frames, "帧数必须相同");
    assert_eq!(base.2, under.source_label, "来源文案必须逐字相同");
    assert_eq!(base.3, under.digest, "digest 必须逐位相同");

    let fixture = Fixture::new("old-none-path");
    // 旧入口若偷偷带上项目目录，就会读到这份同名不同字节的诱饵 ⇒ 逐位相同必须变红。
    let decoy =
        std::env::temp_dir().join(format!("yeban-168-none-path-decoy-{}", std::process::id()));
    let relative = fixture.repo_relative_sfz();
    let fixture_root = Path::new(&relative)
        .parent()
        .and_then(Path::parent)
        .expect("诱饵的夹具根");
    write_fixture(&decoy.join(fixture_root), 0.25);
    let (with_sampler, _track) = project_with_sampler(&relative);
    let old_render = render_project_f32(&with_sampler, wav_options, None).expect("旧渲染入口");
    let under_render =
        render_project_f32_with_assets_under(&with_sampler, wav_options, None, &[], None)
            .expect("`_under(None)` 渲染入口");
    assert_eq!(old_render.0, under_render.samples);
    assert_eq!(old_render.2, under_render.source_label);
    assert_eq!(old_render.3, under_render.digest);

    let dir = scratch_dir("old-none-wav");
    let old_path = dir.join("old.wav");
    let under_path = dir.join("under.wav");
    let old_report = record_project(&with_sampler, &old_path, &options, None).expect("旧录制入口");
    let under_report = record_project_under(&with_sampler, &under_path, &options, None, None)
        .expect("`_under(None)` 录制入口");
    assert_eq!(old_report.frames, under_report.frames);
    assert_eq!(
        std::fs::read(&old_path).expect("读旧入口 WAV"),
        std::fs::read(&under_path).expect("读 `_under(None)` WAV"),
        "旧入口与 `_under(None)` 的 WAV 必须逐字节相同"
    );
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&decoy).ok();
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
        loudness: None,
    }
}
