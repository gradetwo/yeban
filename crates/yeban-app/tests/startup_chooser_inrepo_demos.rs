//! T-FIX-291：CI 可跑的**仓库内合成夹具**判据 —— 文件型 demo 的扫描 /
//! 标题 / 轨道数 / BPM / 小节 / 封面 / 详情行 / 打开归档，全部不依赖
//! 仓库外 `exp-demo-cover-assets`。
//!
//! 旧的 `startup_chooser_all_demos.rs` / `startup_chooser_demo_cover.rs`
//! 是**手工/证据档**的活窗口截图判据（见那些文件的 `#[ignore]` 理由），
//! 本文件把其数据层覆盖搬进正常 `cargo test`：合成两个极小 `.yeban`
//! 容器 + 同目录小图 + 旁挂 meta，钉住 `gather` / `demo_rows` /
//! `load_file_demo_archive` 的真实读数，⛔ 不引入真实 demo 素材。

#![allow(unused)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::cli::Language;
use yeban_app::settings::Settings;
use yeban_app::startup_chooser::{
    DEMOS_DIR_ENV, DemoSource, demo_rows, gather, load_file_demo_archive,
};
use yeban_app::templates::TemplateRoots;
use yeban_model::ids::{AssetHash, PPQ};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-inrepo-demos-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // 先撤环境变量，免得后续判据扫到已删的临时目录。
        // SAFETY: 本测试二进制里只有这一条判据碰这个环境变量。
        unsafe { std::env::remove_var(DEMOS_DIR_ENV) };
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 写一份合成工程容器（真实容器格式，极小资产池）。
fn write_synthetic_demo(path: &Path, title: &str, author: &str, bpm: f64) {
    let mut project = yeban_model::samples::filled_project();
    project.title = title.to_owned();
    project.author = author.to_owned();
    project.bpm = bpm;
    let assets: BTreeMap<AssetHash, Vec<u8>> = BTreeMap::new();
    let bytes =
        yeban_model::container::write_project_container(&project, b"inrepo-fixture", &assets)
            .expect("写合成容器");
    std::fs::write(path, bytes).expect("写合成 demo 文件");
}

/// 写一张 2×2 的合成封面 PNG（有真实字节，供 `Image::load_from_path` 类路径读取）。
fn write_synthetic_cover(path: &Path) {
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::image::Rgb8Image;
    let mut image = Rgb8Image::new(Size::new(2, 2));
    for y in 0..2 {
        for x in 0..2 {
            image.set_pixel(x, y, [24, 48, 96]);
        }
    }
    std::fs::write(path, yeban_ui_test_port::png::encode_rgb8(&image)).expect("写合成封面");
}

/// 按工程自身口径独立算小节/时长（⛔ 不复用被测模块的 `demo_span`）。
fn expected_span(project: &yeban_model::YebanProjectV1) -> (usize, f64) {
    let end_tick = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .map(|clip| clip.start_tick.saturating_add(clip.duration_ticks))
        .max()
        .unwrap_or(0);
    let numerator = u64::from(project.time_signature.numerator.max(1));
    let denominator = u64::from(project.time_signature.denominator.max(1));
    let ticks_per_bar = PPQ.saturating_mul(4).saturating_mul(numerator) / denominator;
    let bars = usize::try_from(end_tick.div_ceil(ticks_per_bar)).unwrap_or(usize::MAX);
    let seconds = end_tick as f64 / PPQ as f64 / project.bpm * 60.0;
    (bars, seconds)
}

#[test]
fn inrepo_synthetic_demos_cover_the_chooser_data_layer() {
    let scratch = Scratch::new();
    let demos = scratch.dir.join("demos");
    std::fs::create_dir_all(&demos).expect("建演示目录夹具");

    // 路径序：01 在前、02 在后（扫描按路径排序，⛔ 不依赖 read_dir 顺序）。
    let first = demos.join("01-synth.yeban");
    write_synthetic_demo(&first, "合成曲一", "作者甲 · 注记", 120.0);
    write_synthetic_cover(&demos.join("01-synth-cover.png"));
    std::fs::write(demos.join("01-synth.meta.json"), r#"{"genre":"合成曲风"}"#)
        .expect("写旁挂 meta");

    let second = demos.join("02-synth.yeban");
    write_synthetic_demo(&second, "合成曲二", "", 90.0);
    // 同名封面（无 `-cover` 后缀）另一条约定也要覆盖。
    write_synthetic_cover(&demos.join("02-synth.png"));
    // 故意不放 meta ⇒ 曲风必须留空、无数据源的行不出现。

    // SAFETY: 本测试二进制里只有这一条判据碰这个环境变量。
    unsafe { std::env::set_var(DEMOS_DIR_ENV, demos.as_os_str()) };

    let roots = TemplateRoots {
        factory: scratch.dir.join("factory"),
        user: scratch.dir.join("user"),
    };
    let data = gather(&Settings::default(), &roots);
    // T-FIX-951 起 gather 还并入 8 条官方 demo（带下载态）；本判据只认
    // 合成目录里的两条本地 demo（无下载态）。
    let synthetic: Vec<_> = data
        .demos
        .iter()
        .filter(|entry| entry.download.is_none())
        .collect();
    assert_eq!(synthetic.len(), 2, "两条合成 demo 必须都被扫到");
    assert!(
        synthetic
            .iter()
            .all(|entry| matches!(entry.source, DemoSource::File(_))),
        "演示清单只能是文件型"
    );

    let a = synthetic[0];
    let b = synthetic[1];
    let pa = yeban_model::samples::filled_project();
    let mut pb = pa.clone();
    pb.bpm = 90.0;
    let expected_tracks = pa
        .tracks
        .keys()
        .filter(|id| **id != pa.master_bus_track_id)
        .count();
    let (bars_a, secs_a) = expected_span(&pa);
    let (bars_b, secs_b) = expected_span(&pb);

    assert_eq!(a.title, "合成曲一", "标题从容器实读");
    assert_eq!(a.author.as_deref(), Some("作者甲"), "作者取首个 · 之前的段");
    assert_eq!(a.genre.as_deref(), Some("合成曲风"), "曲风从旁挂 meta 实读");
    assert_eq!(a.bpm, 120.0);
    assert_eq!(a.track_count, expected_tracks);
    assert_eq!(a.bars, bars_a);
    assert!(
        (a.duration_secs - secs_a).abs() < 0.5,
        "时长读数 {} 与独立实算 {} 不一致",
        a.duration_secs,
        secs_a
    );
    assert!(
        a.cover.path().is_some_and(|p| p.is_file()),
        "同目录合成封面必须被认出"
    );

    assert_eq!(b.title, "合成曲二");
    assert!(b.author.is_none(), "空作者 ⇒ None，不编造");
    assert!(b.genre.is_none(), "无旁挂 ⇒ 曲风留空");
    assert_eq!(b.bpm, 90.0);
    assert_eq!(b.track_count, expected_tracks);
    assert_eq!(b.bars, bars_b);
    assert!((b.duration_secs - secs_b).abs() < 0.5);
    assert!(
        b.cover.path().is_some_and(|p| p.is_file()),
        "同名封面约定也必须认出"
    );

    // 行模型：详情行按专辑页顺序，有数据才有行。
    let synthetic_entries: Vec<_> = synthetic.iter().map(|entry| (*entry).clone()).collect();
    let rows = demo_rows(&synthetic_entries, Language::ZhCn);
    assert_eq!(rows.len(), 2);
    let labels_a: Vec<&str> = rows[0]
        .detail_rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert!(
        labels_a.contains(&"轨道数"),
        "文件型 demo 必须有实算轨道数行: {labels_a:?}"
    );
    assert!(
        labels_a.contains(&"小节数"),
        "文件型 demo 必须有实算小节行: {labels_a:?}"
    );
    assert!(
        labels_a.contains(&"时长"),
        "文件型 demo 必须有实算时长行: {labels_a:?}"
    );
    assert!(
        labels_a.contains(&"曲风"),
        "旁挂有曲风 ⇒ 曲风行出现: {labels_a:?}"
    );
    assert!(rows[0].genre.contains("合成曲风"));
    assert!(rows[0].bpm_label.contains("120"));
    assert!(rows[0].cover.is_some());
    let labels_b: Vec<&str> = rows[1]
        .detail_rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert!(
        !labels_b.contains(&"曲风"),
        "无旁挂 ⇒ 曲风行不出现: {labels_b:?}"
    );
    assert!(rows[1].genre.is_empty());
    assert!(rows[1].bpm_label.contains("90"));

    // 打开语义的数据层：从磁盘真实加载归档（未命名副本的源），
    // 标题与容器实读一致，源文件字节不变。
    let archive = load_file_demo_archive(&first).expect("合成 demo 必须能真实加载");
    assert_eq!(archive.project.title, "合成曲一");
    let before = std::fs::read(&first).expect("读源文件字节");
    let _ = load_file_demo_archive(&first).expect("再次加载仍成功");
    assert_eq!(
        std::fs::read(&first).expect("再读源文件字节"),
        before,
        "打开不得改动源文件"
    );
}
