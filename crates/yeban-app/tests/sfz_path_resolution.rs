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
//! | `tried` | [`SfzPathError::tried`] | 候选根**条数**（通常 3） |
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
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    rust_sources(&manifest.join("src"), &mut sources);

    // ① 全 crate 只有**一个** `pub fn resolve_sampler_path` 定义。
    let definitions: usize = sources
        .iter()
        .map(|(_, text)| text.matches("pub fn resolve_sampler_path").count())
        .sum();
    assert_eq!(
        definitions, 1,
        "⭐ 解析函数必须只有一份定义（实测 {definitions}）"
    );

    // ② 旧的两处：探测侧（bridge）没有本地解析器，改成调它。
    let bridge = sources
        .iter()
        .find(|(name, _)| name == "bridge.rs")
        .map(|(_, text)| text)
        .expect("bridge.rs 必须在 src/");
    assert!(
        !bridge.contains("fn resolve_builtin_instrument_path"),
        "⛔ `bridge.rs` 不得再有自己的解析函数"
    );
    assert!(
        bridge.contains("resolve_sampler_path("),
        "⭐ 探测侧必须调 `resolve_sampler_path`"
    );

    // ③ 旧的第二处：渲染侧（engine_host）先解析再进缓存/摊平。
    let engine_host = sources
        .iter()
        .find(|(name, _)| name == "engine_host.rs")
        .map(|(_, text)| text)
        .expect("engine_host.rs 必须在 src/");
    assert!(
        engine_host.contains("resolve_sampler_path("),
        "⭐ 渲染侧必须调 `resolve_sampler_path`"
    );

    println!(
        "[#168 判据1] 源文件={}；`pub fn resolve_sampler_path` 定义={definitions}；\
         bridge 本地解析器=0；bridge 调用={}；engine_host 调用={}",
        sources.len(),
        bridge.matches("resolve_sampler_path(").count(),
        engine_host.matches("resolve_sampler_path(").count(),
    );
}

// ===========================================================================
// 判据 2：cwd 无关（仓根 / /tmp / crate 目录三档装载读数相同）
// ===========================================================================
#[test]
fn the_same_project_loads_the_same_from_three_cwds() {
    let fixture = Fixture::new("cwd");
    let relative = fixture.repo_relative_sfz();
    let expected = fixture.sfz();
    let cwds = [
        repo_root().to_path_buf(),
        PathBuf::from("/tmp"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    ];
    let mut readings = Vec::new();
    for cwd in &cwds {
        let roots = SfzRoots::none().with_cwd(cwd);
        let resolved = resolve_sampler_path(&relative, roots).expect("仓库相对路径必须解析得到");
        assert_eq!(
            resolved, expected,
            "解析结果必须就是那份夹具（cwd={cwd:?}）"
        );
        let reading = load_reading(&relative, roots);
        println!(
            "[#168 判据2] cwd={} ⇒ resolved={} regions={} decoded={} silent={} missing={}",
            cwd.display(),
            reading.resolved.display(),
            reading.regions,
            reading.decoded,
            reading.silent,
            reading.missing
        );
        readings.push(reading);
    }
    assert_eq!(
        readings[0], readings[1],
        "⭐ /tmp 与仓根的装载读数必须相同（旧行为下这里会读不到样本）"
    );
    assert_eq!(
        readings[0], readings[2],
        "⭐ crate 目录与仓根的装载读数必须相同（`cargo test` 的 cwd 就是 crate 目录）"
    );
    assert_eq!(
        readings[0],
        Reading {
            resolved: expected,
            regions: 1,
            decoded: 1,
            silent: 0,
            missing: 0,
        },
        "三档的共同读数：1 region、1 份 PCM、0 静音、0 缺样本"
    );
}

// ===========================================================================
// 判据 3：找不到 ⇒ 响亮（逐字含"试过：⟨根1⟩／⟨根2⟩／⟨根3⟩"）＋ 不静音
// ===========================================================================
#[test]
fn a_missing_relative_path_is_loud_and_lists_every_root() {
    let fixture = Fixture::new("missing");
    let project_dir = fixture.dir.clone();
    let cwd = PathBuf::from("/tmp/yeban-168-missing-cwd");
    let roots = SfzRoots::under_project(&project_dir).with_cwd(&cwd);
    let missing = "samples/definitely-missing.sfz";

    // ① 解析层：错误里逐字列出三个候选。
    let path_error = resolve_sampler_path(missing, roots).expect_err("找不到必须报错");
    let text = path_error.to_string();
    assert!(text.contains("试过："), "错误必须逐字说试过哪些根：{text}");
    let expected_candidates = [
        project_dir.join(missing),
        repo_root().join(missing),
        cwd.join(missing),
    ];
    assert_eq!(
        path_error.tried(),
        expected_candidates.as_slice(),
        "候选顺序必须是 工程目录 ⇒ 仓库根 ⇒ cwd"
    );
    for candidate in &expected_candidates {
        assert!(
            text.contains(&candidate.display().to_string()),
            "错误原文必须包含候选 `{}`：{text}",
            candidate.display()
        );
    }

    // ② 渲染层：`arm_pending_sfz` 必须把同一条错误原文抛出来（⛔ 不静默静音）。
    let (project, _track) = project_with_sampler(missing);
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("投影快照");
    let arm_error = arm_pending_sfz(
        snapshot,
        RATE as f32,
        BTreeMap::<EntityId, SfzPlan>::new(),
        roots,
    )
    .expect_err("渲染侧必须失败");
    let text = arm_error.to_string();
    assert!(
        text.contains("试过：") && text.contains("SFZ 路径"),
        "渲染侧错误必须带解析原文：{text}"
    );

    // ③ 宿主层（生产 GUI 的入口）：`reload` 必须失败，且**不留一代装作在播的引擎**。
    let mut host = EngineHost::new();
    host.set_project_dir(Some(project_dir.clone()));
    let host_error = host.reload(&project, 0).expect_err("reload 必须失败");
    let text = host_error.to_string();
    assert!(
        text.contains("试过："),
        "宿主错误必须带试过的根（⛔ 不静默静音）：{text}"
    );
    assert!(!host.has_engine(), "失败后⛔ 不得留下一代替身引擎");

    println!(
        "[#168 判据3] path={missing}；tried={:?}；resolve=Err(含试过)；\
         arm_pending_sfz=Err(含试过)；EngineHost::reload=Err(含试过) has_engine={}",
        path_error.tried(),
        host.has_engine()
    );
    println!("[#168 判据3 原文] {text}");
}

// ===========================================================================
// 判据 4：绝对路径原样
// ===========================================================================
#[test]
fn absolute_paths_are_returned_verbatim() {
    let fixture = Fixture::new("absolute");
    let absolute = fixture.sfz();
    assert!(
        absolute.is_absolute(),
        "夹具路径必须是本平台的绝对路径（`target/tmp` 在仓库内）"
    );
    let text = absolute.to_string_lossy().into_owned();
    // 两个根都指向**别处**，且（`#171`）各自是**本平台的绝对路径** ⇒
    // 若实现"改写绝对路径"，这里必然红。
    let elsewhere = platform_absolute_nonexistent("yeban-171-elsewhere");
    let elsewhere_cwd = platform_absolute_nonexistent("yeban-171-cwd");
    let roots = SfzRoots::under_project(Path::new(&elsewhere)).with_cwd(Path::new(&elsewhere_cwd));
    let resolved = resolve_sampler_path(&text, roots).expect("绝对路径必须原样返回");
    assert_eq!(
        resolved,
        PathBuf::from(&text),
        "⭐ 绝对路径必须逐字原样（⛔ 不查根、⛔ 不改写）"
    );
    let reading = load_reading(&text, roots);
    assert_eq!(reading.decoded, 1, "绝对路径装载出 1 份 PCM");
    assert_eq!(
        reading.resolved,
        PathBuf::from(&text),
        "报告里的路径也必须原样"
    );

    // ⭐ `#171`：不存在的绝对路径也**原样**返回（缺件由 `decode_sfz_plan` 的 Read 错误
    // 响亮兜住）。这条路径必须是**本平台**的绝对路径 —— `/nonexistent/…` 在 Windows 上
    // `is_absolute() == false`，会被当相对路径去三档里找，判据就丢了平台语义。
    let nonexistent = platform_absolute_nonexistent("yeban-171.sfz");
    #[cfg(windows)]
    assert_eq!(
        nonexistent, r"C:\nonexistent\yeban-171.sfz",
        "Windows 的绝对路径形状必须带盘符（`C:\\…`；⛔ 不是 `/…`）"
    );
    #[cfg(not(windows))]
    assert_eq!(
        nonexistent, "/nonexistent/yeban-171.sfz",
        "Unix 的绝对路径形状必须是 `/…`（⛔ 不是 `C:\\…`）"
    );
    assert!(
        Path::new(&nonexistent).is_absolute(),
        "⭐ 判据前提：`{nonexistent}` 必须在本平台读作**绝对**路径；\
         `#171` 的红正是因为 `/nonexistent/…` 在 Windows 上读作 `false`"
    );
    assert_eq!(
        resolve_sampler_path(&nonexistent, roots).expect("原样返回"),
        PathBuf::from(&nonexistent),
        "⭐ **不存在**的绝对路径也必须原样返回（⛔ 不查根、⛔ 不查存在性）"
    );
    println!(
        "[#168/#171 判据4] abs={} is_absolute={} ⇒ resolved={} decoded={}；\
         不存在的绝对路径={nonexistent} is_absolute={} ⇒ 原样；elsewhere={elsewhere}",
        absolute.display(),
        absolute.is_absolute(),
        reading.resolved.display(),
        reading.decoded,
        Path::new(&nonexistent).is_absolute()
    );
}

// ===========================================================================
// 判据 5：相对工程目录（样本放工程旁 ⇒ 任意 cwd 都读得到）
// ===========================================================================
#[test]
fn a_project_adjacent_sfz_is_readable_from_any_cwd() {
    let fixture = Fixture::new("adjacent");
    let expected = fixture.sfz();
    let relative = "samples/tone.sfz";
    for cwd in [Path::new("/tmp"), Path::new(env!("CARGO_MANIFEST_DIR"))] {
        let roots = SfzRoots::under_project(&fixture.dir).with_cwd(cwd);
        let resolved = resolve_sampler_path(relative, roots).expect("工程旁的相对路径必须解析得到");
        assert_eq!(
            resolved, expected,
            "工程目录必须是第一个候选（cwd={cwd:?}）"
        );
        let reading = load_reading(relative, roots);
        assert_eq!(reading.decoded, 1, "工程旁的样本必须真的解出 PCM");
        assert_eq!(reading.silent, 0);
        println!(
            "[#168 判据5] cwd={} project_dir={} ⇒ resolved={} decoded={} silent={}",
            cwd.display(),
            fixture.dir.display(),
            reading.resolved.display(),
            reading.decoded,
            reading.silent
        );
    }

    // 生产宿主：`set_project_dir` 之后 `reload` 直接读得到（缓存 miss=1 ⇒ 真解了一次）。
    let (project, track) = project_with_sampler(relative);
    let mut host = EngineHost::new();
    host.set_project_dir(Some(fixture.dir.clone()));
    host.reload(&project, 0).expect("宿主 reload 必须成功");
    let snapshot = host.current_snapshot().expect("reload 后有快照");
    assert!(
        snapshot.sfz_plan(&track).is_some(),
        "⭐ 工程旁的 SFZ 必须被摊平成快照里的表"
    );
    assert!(snapshot.sfz_pending().is_empty(), "摊平表必须取代待摊平项");
    assert_eq!(host.sfz_cache_readings().misses, 1, "恰好解码一次");
    println!(
        "[#168 判据5 宿主] project_dir={}；reload=Ok；摊平表=Some；cache misses={}",
        fixture.dir.display(),
        host.sfz_cache_readings().misses
    );
}

// ===========================================================================
// 判据 1（旁证）：探测与渲染共用同一个解析 ⇒ 仓库相对路径的探测读数不变
// ===========================================================================
#[test]
fn the_probe_and_the_renderer_use_the_same_resolver() {
    let fixture = Fixture::new("probe");
    let relative = fixture.repo_relative_sfz();
    // 探测（`bridge`）没有工程目录：按仓库根解析（`#164`／`#165` 的契约）⇒ 1 份样本。
    assert_eq!(
        sfz_samples_present(&relative),
        Ok(()),
        "仓库相对路径的夹具必须被探测判为'有样本'"
    );
    // 渲染（同一个解析器，cwd 注入为 crate 目录）⇒ 同一份文件、同样有样本。
    let roots = SfzRoots::none().with_cwd(Path::new(env!("CARGO_MANIFEST_DIR")));
    let reading = load_reading(&relative, roots);
    assert_eq!(
        reading.resolved,
        fixture.sfz(),
        "探测与渲染必须解析到同一份文件"
    );
    assert_eq!((reading.decoded, reading.silent), (1, 0));
    println!(
        "[#168 判据1 旁证] path={relative}；探测={:?}；渲染 resolved={} decoded={} silent={}",
        sfz_samples_present(&relative),
        reading.resolved.display(),
        reading.decoded,
        reading.silent
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
    let fixture = Fixture::new("process-cwd");
    let relative = fixture.repo_relative_sfz();
    let cwd = std::env::current_dir().expect("进程 cwd");
    let reading = load_reading(&relative, SfzRoots::none());
    assert_eq!(
        reading.resolved,
        fixture.sfz(),
        "真实进程 cwd 不得改变解析结果"
    );
    assert_eq!(
        (
            reading.regions,
            reading.decoded,
            reading.silent,
            reading.missing
        ),
        (1, 1, 0, 0),
        "真实进程 cwd 不得改变装载读数"
    );
    println!(
        "[#168 判据2 进程cwd] cwd={} resolved={} regions={} decoded={} silent={} missing={}",
        cwd.display(),
        reading.resolved.display(),
        reading.regions,
        reading.decoded,
        reading.silent,
        reading.missing
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
    let old = Fixture::new("170-save-old");
    let new = Fixture::new("170-save-new");
    let target = new.dir.join("moved.yeban");
    let (_tier1, ui) = save_window();
    let (project, track) = project_with_sampler("samples/tone.sfz");
    let engine = Rc::new(RefCell::new(EngineHost::new()));
    engine.borrow_mut().set_project_dir(Some(old.dir.clone()));
    let before = engine.borrow().project_dir().map(Path::to_path_buf);
    let picker = Rc::new(ScriptedSave::answering(target.clone()));
    // 装配期**没有**目标路径 ⇒ 保存会问一次对话框 = 产品的「另存为」形态。
    wire_save_with_engine(
        &ui,
        SaveStatus {
            target: None,
            project: project.clone(),
        },
        None,
        Rc::clone(&picker) as Rc<dyn FilePicker>,
        None,
        Some(Rc::clone(&engine)),
    );

    ui.invoke_save_project();
    let after = engine.borrow().project_dir().map(Path::to_path_buf);

    assert_eq!(picker.calls.get(), 1, "另存为必须问一次对话框");
    assert!(ui.get_save_succeeded(), "另存为必须成功落盘");
    assert_eq!(
        before.as_deref(),
        Some(old.dir.as_path()),
        "另存前：工程目录必须还是**旧**文件所在目录"
    );
    assert_eq!(
        after.as_deref(),
        Some(new.dir.as_path()),
        "⭐ 另存后：工程目录必须跟到**新**文件所在目录"
    );
    assert!(target.exists(), "另存的文件必须真的在磁盘上");

    // 随后加载相对的 `samples/tone.sfz` ⇒ 必须按**新**目录解析并摊平。
    let mut host = engine.borrow_mut();
    host.reload(&project, 0)
        .expect("另存后，工程旁的相对样本必须按新目录解析得到");
    let snapshot = host.current_snapshot().expect("reload 后有快照");
    let plan_present = snapshot.sfz_plan(&track).is_some();
    let pending_empty = snapshot.sfz_pending().is_empty();
    let misses = host.sfz_cache_readings().misses;
    assert!(plan_present, "⭐ 摊平表里必须有那条轨（样本真的读到了）");
    assert!(pending_empty, "摊平表必须取代待摊平项");
    assert_eq!(misses, 1, "恰好解码一次");

    println!(
        "[#170 判据1] before={} after={} target_exists={} save_status={:?} \
         reload=Ok sfz_plan={plan_present} pending_empty={pending_empty} misses={misses}",
        before.as_deref().unwrap_or(Path::new("<none>")).display(),
        after.as_deref().unwrap_or(Path::new("<none>")).display(),
        target.exists(),
        ui.get_save_status().to_string(),
    );
}

/// ⭐ 判据 ②a：用户在「另存为」对话框里按**取消** ⇒ `project_dir` **一位不动**
/// （与 `wire_save` 的既有取消口径一致：⛔ 不写状态、⛔ 不写文件）。
#[test]
fn a_cancelled_save_leaves_the_project_dir_alone() {
    let fixture = Fixture::new("170-cancel");
    let (_tier1, ui) = save_window();
    let (project, _track) = project_with_sampler("samples/tone.sfz");
    let engine = Rc::new(RefCell::new(EngineHost::new()));
    engine
        .borrow_mut()
        .set_project_dir(Some(fixture.dir.clone()));
    let before = engine.borrow().project_dir().map(Path::to_path_buf);
    let picker = Rc::new(ScriptedSave::cancelling());
    wire_save_with_engine(
        &ui,
        SaveStatus {
            target: None,
            project,
        },
        None,
        Rc::clone(&picker) as Rc<dyn FilePicker>,
        None,
        Some(Rc::clone(&engine)),
    );

    ui.invoke_save_project();
    let after = engine.borrow().project_dir().map(Path::to_path_buf);

    assert_eq!(picker.calls.get(), 1, "取消也要先开一次对话框");
    assert_eq!(
        ui.get_save_status().to_string(),
        "",
        "⛔ 取消不是失败：不得留下一条错误文案"
    );
    assert!(!ui.get_save_succeeded(), "取消没保存成功");
    assert_eq!(
        before, after,
        "⭐ 取消 ⇒ `project_dir` 必须**一位不动**（没有新工程文件就没有可跟的目录）"
    );
    println!(
        "[#170 判据2a] calls={} project_dir={} save_status(空)={:?} succeeded={}",
        picker.calls.get(),
        after.as_deref().unwrap_or(Path::new("<none>")).display(),
        ui.get_save_status().to_string(),
        ui.get_save_succeeded()
    );
}

/// ⭐ 判据 ②b：落盘**失败**（目标目录不存在）⇒ `project_dir` **一位不动**，
/// 且界面如实报"保存失败"。
#[test]
fn a_failed_save_leaves_the_project_dir_alone() {
    let fixture = Fixture::new("170-failure");
    let (_tier1, ui) = save_window();
    let (project, _track) = project_with_sampler("samples/tone.sfz");
    let engine = Rc::new(RefCell::new(EngineHost::new()));
    engine
        .borrow_mut()
        .set_project_dir(Some(fixture.dir.clone()));
    let before = engine.borrow().project_dir().map(Path::to_path_buf);
    // 父目录不存在 ⇒ 本地原子写必然被拒（⛔ 不建目录）。
    let target = fixture.dir.join("missing-dir").join("moved.yeban");
    let picker = Rc::new(ScriptedSave::answering(target.clone()));
    wire_save_with_engine(
        &ui,
        SaveStatus {
            target: None,
            project,
        },
        None,
        Rc::clone(&picker) as Rc<dyn FilePicker>,
        None,
        Some(Rc::clone(&engine)),
    );

    ui.invoke_save_project();
    let after = engine.borrow().project_dir().map(Path::to_path_buf);
    let status = ui.get_save_status().to_string();

    assert!(!target.exists(), "失败时不得写出任何文件");
    assert!(!ui.get_save_succeeded(), "失败不得报成功");
    assert!(
        status.starts_with("保存失败"),
        "界面必须如实报失败（实得 {status:?}）"
    );
    assert_eq!(
        before, after,
        "⭐ 落盘失败 ⇒ `project_dir` 必须**一位不动**"
    );
    println!(
        "[#170 判据2b] calls={} target_exists={} project_dir={} save_succeeded={} status={status:?}",
        picker.calls.get(),
        target.exists(),
        after.as_deref().unwrap_or(Path::new("<none>")).display(),
        ui.get_save_succeeded(),
    );
}

/// ⭐ 判据 ③（直接入口）：录制入口**拿得到工程目录** ⇒ 工程旁的相对样本读得到；
/// 反之（旧入口 `None`）**响亮**失败且⛔ 不写文件。
#[test]
fn the_record_entry_reads_a_sample_next_to_the_project_file() {
    let fixture = Fixture::new("170-record");
    let (project, _track) = project_with_sampler("samples/tone.sfz");
    let options = RecordOptions {
        seconds: 0.25,
        ..RecordOptions::default()
    };
    let out = fixture.dir.join("recorded.wav");

    // 反向腿：旧入口（没有工程目录）⇒ 解析必须**响亮**失败（⛔ 不静默静音）。
    let error =
        record_project(&project, &out, &options, None).expect_err("没有工程目录时必须读不到");
    let text = error.to_string();
    assert!(text.contains("试过："), "错误必须列出试过的根：{text}");
    assert!(!out.exists(), "失败时⛔ 不得写任何文件");

    // 正向腿：工程目录 = 夹具目录 ⇒ 工程旁的相对样本读得到。
    let report = record_project_under(&project, &out, &options, None, Some(&fixture.dir))
        .expect("工程旁的相对样本必须读得到");
    let bytes = std::fs::metadata(&out).expect("写出的 WAV 必须在").len();
    assert_eq!(bytes, report.bytes, "报告的字节数必须与磁盘一致");
    assert!(report.bytes > 44, "WAV 必须比 44 字节的头部长");
    assert_eq!(report.frames, 12_000, "0.25 s × 48 kHz = 12000 帧");

    // 同一条渲染入口的装载读数：样本**真的**被解成 PCM（⛔ 不是静音兜底）。
    let rendered = render_project_f32_with_assets_under(
        &project,
        record_wav_options(&options),
        None,
        &[],
        Some(&fixture.dir),
    )
    .expect("渲染入口必须成功");
    assert_eq!(rendered.sfz_decoded.len(), 1, "恰好一条轨有 SFZ");
    let decode = &rendered.sfz_decoded[0].1;
    assert_eq!((decode.decoded_samples, decode.silent_region_count), (1, 0));

    println!(
        "[#170 判据3] record(project_dir=Some) ⇒ frames={} bytes={} source={:?}；\
         old_entry(None)=Err(含试过) bytes_on_disk={bytes}；sfz decoded={} silent={} regions={}",
        report.frames,
        report.bytes,
        report.source,
        decode.decoded_samples,
        decode.silent_region_count,
        decode.regions,
    );
    println!("[#170 判据3 反向腿原文] {text}");
}

/// ⭐ 判据 ③（生产接线）：**真二进制** `--open … --record-wav …` 也拿得到工程目录
/// （证明 `cli` 那一跳真的把 `Loaded::project_dir()` 交给了录制入口，⛔ 不只测内部函数）。
#[test]
fn the_cli_record_entry_plumbs_the_project_dir() {
    let fixture = Fixture::new("170-cli");
    let (project, _track) = project_with_sampler("samples/tone.sfz");
    let container = fixture.dir.join("project.yeban");
    yeban_app::save::save_project_file(&project, &container).expect("写夹具工程文件");
    let out = fixture.dir.join("cli-recorded.wav");
    let run = Command::new(env!("CARGO_BIN_EXE_yeban-app"))
        .args([
            "--open",
            container.to_str().expect("utf8 路径"),
            "--record-wav",
            out.to_str().expect("utf8 路径"),
            "--record-seconds",
            "0.2",
        ])
        .output()
        .expect("跑 yeban-app --record-wav");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        run.status.success(),
        "退出码={:?} stderr={stderr}",
        run.status.code()
    );
    assert!(
        stdout.contains("recorded-wav:"),
        "必须打印录制回执；stdout={stdout}"
    );
    assert!(
        stdout.contains("frames=9600"),
        "0.2 s × 48 kHz = 9600 帧必须逐字出现在回执里；stdout={stdout}"
    );
    let bytes = std::fs::metadata(&out)
        .expect("CLI 录出的 WAV 必须在")
        .len();
    println!(
        "[#170 判据3 CLI] rc={:?} bytes={bytes}；回执行={:?}",
        run.status.code(),
        stdout
            .lines()
            .find(|line| line.starts_with("recorded-wav:"))
            .unwrap_or("<没有 recorded-wav 行>"),
    );
}

/// ⭐ 判据 ④：旧入口（`project_dir = None` 的委托路径）**逐位不变**。
///
/// 读数：旧入口与 `_under(None)` 写出的 WAV **字节相等**（逐字节比较）、
/// 渲染 `samples` 相等、`digest` 十六进制相等。
#[test]
fn the_old_entries_are_bit_identical_on_the_none_path() {
    let fixture = Fixture::new("170-none");
    // 相对**仓库根**的路径：`None` 口径（仓库根 ⇒ cwd）必须仍解析得到。
    let relative = fixture.repo_relative_sfz();
    let (project, _track) = project_with_sampler(&relative);
    let options = RecordOptions {
        seconds: 0.25,
        ..RecordOptions::default()
    };
    let old_wav = fixture.dir.join("old-entry.wav");
    let new_wav = fixture.dir.join("under-none.wav");
    let old = record_project(&project, &old_wav, &options, None).expect("旧入口必须成功");
    let new = record_project_under(&project, &new_wav, &options, None, None)
        .expect("`_under(None)` 必须成功");
    let old_bytes = std::fs::read(&old_wav).expect("旧入口写出的 WAV");
    let new_bytes = std::fs::read(&new_wav).expect("`_under(None)` 写出的 WAV");
    assert_eq!(old.frames, new.frames, "帧数必须相同");
    assert_eq!(old.bytes, new.bytes, "字节数必须相同");
    assert_eq!(
        old_bytes, new_bytes,
        "⭐ 旧入口与 `_under(None)` 写出的 WAV 必须**逐位相同**"
    );

    let old_render = render_project_f32(&project, record_wav_options(&options), None)
        .expect("旧渲染入口必须成功");
    let new_render = render_project_f32_with_assets_under(
        &project,
        record_wav_options(&options),
        None,
        &[],
        None,
    )
    .expect("`_under(None)` 渲染入口必须成功");
    assert_eq!(old_render.0, new_render.samples, "samples 必须逐位相同");
    assert_eq!(old_render.1, new_render.frames, "帧数必须相同");
    assert_eq!(
        old_render.3, new_render.digest,
        "⭐ digest 必须相同（同一份逐位样本）"
    );

    println!(
        "[#170 判据4] wav_bytes_old={} wav_bytes_new={} byte_equal={} frames={} digest={}",
        old_bytes.len(),
        new_bytes.len(),
        old_bytes == new_bytes,
        old.frames,
        new_render.digest,
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
    }
}
