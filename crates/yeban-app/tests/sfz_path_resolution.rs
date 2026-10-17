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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::bridge::sfz_samples_present;
use yeban_app::engine_host::EngineHost;
use yeban_app::sfz_load::{SfzRoots, arm_pending_sfz, repo_root, resolve_sampler_path};
use yeban_engine::sfz_instrument::SfzPlan;
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

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
    let text = absolute.to_string_lossy().into_owned();
    // 两个根都指向别处 ⇒ 若实现"改写绝对路径"，这里必然红。
    let roots =
        SfzRoots::under_project(Path::new("/tmp/yeban-168-elsewhere")).with_cwd(Path::new("/tmp"));
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

    // 不存在的绝对路径也**原样**返回（缺件由 `decode_sfz_plan` 的 Read 错误响亮兜住）。
    let nonexistent = "/nonexistent/yeban-168.sfz";
    assert_eq!(
        resolve_sampler_path(nonexistent, roots).expect("原样返回"),
        PathBuf::from(nonexistent)
    );
    println!(
        "[#168 判据4] abs={} ⇒ resolved={} decoded={}；不存在绝对路径也原样={nonexistent}",
        absolute.display(),
        reading.resolved.display(),
        reading.decoded
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
