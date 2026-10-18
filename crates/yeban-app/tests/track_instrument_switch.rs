//! ⭐ 任务单 `#210`：**GUI 里能切换乐器并试听** —— 库内全部可装乐器的判据。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | 工单 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 列表内容 | 假库根里放 2 份 manifest 乐器的 SFZ ＋ 全部样本 ⇒ `library_instruments()` 长 **2**、前两条名字逐字是 `Cowsynth` / `Karoryfer Pastabass (linguine)`；投影 `instrument_names` 与控件树按钮**同序同长** | 证"列表 ＝ 库里有 SFZ 的那些、源自 manifest"；⛔ 不证真库的 20+ 条（那一条在报告里给读数） |
//! | ② 点击换轨（同一条轨） | 点 0 ⇒ `sampler.path == A`；点 1 ⇒ `B`；两次都在**同一条**选中轨上，`commit_count +2`、`undoable +2` | 证"一次点击 ＝ 恰好一次 `Op::SetTrackSampler` 提交" |
//! | ③ 换到另一条轨 | 选中轨换成第 2 条 ⇒ 点 0 ⇒ **只有**第 2 条变 A，第 1 条仍是 B | 证"主语是选中轨（`#192`），不是兜底" |
//! | ④ 撤销 | `UiAction::Undo` 两次 ⇒ 第 2 条回到 `None`、第 1 条从 B 回到 A | 证"进了撤销栈、且回到上一个值"（这是本单对"要不要进栈"的裁决证据） |
//! | ⑤ 能出声 | CLI `--track-sampler … --export-wav` ⇒ `sfz(tracks=1 regions=… samples=… silent=0 missing=[])` ＋ `source=sfz(file(…)` | ⛔ 不用 `probe` 的能量；用导出链路的样本来源读数 |
//!
//! ## 假库根（为什么不是真库）
//!
//! 判据必须在**任何机器**上测同一件事：真库里有没有 20+ 份乐器是产品状态，不是判据前提。
//! 本文件因此把库根指向临时目录，只放两份 manifest 乐器（SFZ 文本从仓库读，样本按
//! `sample=` 逐条造 16 帧 16-bit WAV）⇒ 列表长度、名字、点击、撤销都可机械断言。
//! ⚠ 这两份 SFZ 是**仓库资产**（`assets/samples/**`，只读）：判据读它们，⛔ 不写它们。
//!
//! ## 它**不**证什么
//!
//! - ⛔ 没人耳听过：本机判据只读导出链路的 `source=` / `missing=` 读数；
//! - ⛔ 不证 GUI 里的**鼠标点击**能触发：Tier-1 端口没有 `send_mouse_click`（上游 1.18.1 不存在），
//!   本文件调的是生产同一个回调 `invoke_instrument_install`（与 `#164`/`#165` 的既有判据同款）；
//! - ⛔ 不证 1920 基准帧：那是 Tab 0（本单只动 Tab 3）；读数见交付报告 §3。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use slint::Model as _;

use yeban_app::bridge::{ViewState, library_instrument_names, library_instruments};
use yeban_app::host;
use yeban_app::samples::SAMPLE_LIBRARY_ENV;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, RoutingEdge, RoutingKind, TrackKind,
    TrackV3, YebanProjectV1,
};
use yeban_model::{EntityId, MidiNote, SamplerBinding};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸（与 `instrument_install.rs` 同款：同一块 Tab 3 面板需要高度余量）。
const WINDOW: Size = Size::new(1440, 1000);

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 判据用的 manifest 乐器（路径逐字取自 `assets/samples/manifest.json`）。
const COWSYNTH_SFZ: &str = "assets/samples/karoryfer-cowsynth/cowsynth_baggy.sfz";
const PASTABASS_SFZ: &str = "assets/samples/karoryfer-pastabass/linguine.sfz";
/// 第三份只有 **SFZ 文本**、没有样本字节 ⇒ 它必须出现在**列表**里（库里有 SFZ），
/// 但点它会被既有字节谓词**响亮拒绝**（本文件不点它，只用它把按钮行推过一页）。
const SQUIDPIPES_SFZ: &str = "assets/samples/karoryfer-squidpipes/Programs/01-squidpipes.sfz";

/// 假库根（进程内建一次）：两份 SFZ ＋ 它们引用的全部样本（16 帧 16-bit WAV）。
///
/// ⚠ `set_var` 在 Rust 2024 是 `unsafe`（要求没有并发读者）：写发生在 `OnceLock` 的
/// 初始化里，任何读都发生在本函数返回之后。
fn fake_library() -> &'static Path {
    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("yeban-210-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // (仓库内 SFZ, 库内相对目录, 是否造样本)
        for (repository_path, (library_dir, sfz_relative), with_samples) in [
            (
                COWSYNTH_SFZ,
                ("karoryfer-cowsynth", "cowsynth_baggy.sfz"),
                true,
            ),
            (PASTABASS_SFZ, ("karoryfer-pastabass", "linguine.sfz"), true),
            (
                SQUIDPIPES_SFZ,
                ("karoryfer-squidpipes", "Programs/01-squidpipes.sfz"),
                false,
            ),
        ] {
            let source = yeban_app::sfz_load::repo_root().join(repository_path);
            let text = std::fs::read_to_string(&source).expect("读仓库里的 SFZ 文本");
            let instrument_dir = dir.join(library_dir);
            let target_sfz = instrument_dir.join(sfz_relative);
            let sfz_dir = target_sfz.parent().expect("SFZ 有父目录");
            std::fs::create_dir_all(sfz_dir).expect("建库内乐器目录");
            std::fs::write(&target_sfz, &text).expect("写库内 SFZ");
            if !with_samples {
                continue;
            }
            for sample in sample_references(&text) {
                // 仓库里的 `sample=` 用 Windows 分隔符（`#205` 的归一化在读取层）；
                // 夹具必须落在同一个位置，否则这条判据会在"归一已生效"时假红。
                let relative = sample.replace('\\', "/");
                let target = sfz_dir.join(&relative);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).expect("建样本子目录");
                }
                write_plain_wav(
                    &target,
                    &PcmFormat::integer(1, 48_000, 16),
                    &PcmBuffer::Int16(vec![0_i16; 16]),
                )
                .expect("写夹具 WAV");
            }
        }
        // SAFETY: 见函数文档 —— 这次写发生在任何读之前（`OnceLock` 的 happens-before）。
        unsafe { std::env::set_var(SAMPLE_LIBRARY_ENV, &dir) };
        dir
    })
}

/// 从 SFZ 文本里取全部 `sample=` 的**原文**（与 `sample_library.rs` 同一条最小解析）。
fn sample_references(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|token| token.strip_prefix("sample="))
        .map(|reference| {
            reference
                .trim_matches(|character: char| {
                    character.is_whitespace() || character == '"' || character == '\''
                })
                .to_owned()
        })
        .filter(|reference| !reference.is_empty())
        .collect()
}

/// 一个最小合法工程：主总线 ＋ 两条 MIDI 轨（无音符 —— 本判据只测"装到哪条轨"）。
fn project_with_two_tracks() -> (YebanProjectV1, EntityId, EntityId) {
    let master = EntityId::new();
    let lead = EntityId::new();
    let bass = EntityId::new();
    let mut project = YebanProjectV1 {
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    for (id, name) in [(lead, "Lead"), (bass, "Bass")] {
        project.tracks.insert(
            id,
            TrackV3 {
                id,
                name: name.to_owned(),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
    }
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut clip_notes = BTreeMap::new();
    let note = MidiNote::new(EntityId::new(), 0, 60, 480);
    clip_notes.insert(note.id, note);
    let clip_id = EntityId::new();
    project.clip_pool.insert(
        clip_id,
        ClipPoolEntry {
            id: clip_id,
            name: "clip".to_owned(),
            content: ClipContent::Midi { notes: clip_notes },
        },
    );
    let placement = ClipPlacement {
        id: EntityId::new(),
        clip_id,
        start_tick: 0,
        duration_ticks: 480,
        loop_config: LoopConfig::default(),
        muted: false,
    };
    project
        .tracks
        .get_mut(&lead)
        .expect("Lead 在表里")
        .clips
        .insert(placement.id, placement);
    project.routing_graph.nodes = vec![lead, bass, master];
    for source in [lead, bass] {
        let edge = EntityId::new();
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: source,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    project.validate().expect("夹具工程必须合法");
    (project, lead, bass)
}

/// 一条生产形态的 Tab 3 活窗口（接线走**生产同一个** `host::wire_device_rack`）。
fn live_window_with_undo(
    project: &YebanProjectV1,
    tag: &str,
) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_device_rack(live.ui(), &port);
    (live, port)
}

/// 运行时控件树（`[UI-DoD §3.6]` 的唯一载体）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 选中一条轨（生产同一个窗口属性；`host::display_track` 的唯一输入）。
fn select_track(window: &MainWindow, track: EntityId) {
    window.set_selected_track_id(track.to_canonical_string().into());
}

// ===========================================================================
// ① 列表内容：长度 / 名字 / 投影 / 控件树
// ===========================================================================
#[test]
fn the_list_is_what_the_library_has_and_the_view_and_tree_follow_it() {
    let _library = fake_library();
    let (project, lead, _bass) = project_with_two_tracks();

    let expected_paths = [COWSYNTH_SFZ, PASTABASS_SFZ, SQUIDPIPES_SFZ];
    let instruments = library_instruments();
    assert_eq!(
        instruments.len(),
        3,
        "假库根放了三份 manifest 乐器（两份带样本 + 一份只有 SFZ），实测 {:?}",
        instruments
            .iter()
            .map(|instrument| instrument.path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        instruments
            .iter()
            .map(|instrument| instrument.path.as_str())
            .collect::<Vec<_>>(),
        expected_paths
            .iter()
            .map(|path| (*path).to_owned())
            .collect::<Vec<String>>(),
        "清单顺序必须与 manifest 一致（cowsynth 在 pastabass 之前、squidpipes 之后）"
    );
    assert_eq!(
        library_instrument_names(),
        vec![
            "Cowsynth".to_owned(),
            "Karoryfer Pastabass (linguine)".to_owned(),
            "Squidpipes".to_owned()
        ],
        "前三条名字逐字来自 manifest 的 `name`"
    );

    let (live, port) = live_window_with_undo(&project, "#210 列表");
    assert_eq!(
        live.ui().get_instrument_names().row_count(),
        3,
        "属性的按钮行"
    );
    assert!(
        live.ui().get_instrument_install_available(),
        "有轨 ＋ 清单非空"
    );
    assert_eq!(live.ui().get_instrument_install_reason().to_string(), "");

    let tree = runtime_tree(live.ui());
    // 每页 2 条 ⇒ 第 0 页只有第 0/1 条按钮；第 2 条在下一页（报告里给读数）。
    for index in 0..2 {
        let node = tree
            .find_by_id(&format!("instrument-install-{index}-button"))
            .unwrap_or_else(|| panic!("控件树里必须有第 {index} 个装乐器按钮"));
        let name = library_instruments()[index].name.clone();
        assert_eq!(
            node.label,
            format!("装乐器 {name}"),
            "无障碍标签必须逐字对上第 {index} 条"
        );
    }
    assert!(
        tree.find_by_id("instrument-install-2-button").is_none(),
        "每页 2 条 ⇒ 第 2 条按钮在第 0 页不得出现（它在下一页）"
    );
    assert!(
        tree.find_by_id("instrument-page-next-button").is_some(),
        "还有第 3 条 ⇒ 下一页键必须在场"
    );
    assert!(
        tree.find_by_id("instrument-page-prev-button").is_none(),
        "第 0 页 ⇒ 上一页键不得出现"
    );
    assert!(
        tree.find_by_id("sample-library-download-button").is_none(),
        "清单非空 ⇒ 下载入口收起（与 `#196` 同一口径）"
    );

    // ⭐ 控件行**两两不相交**（本单新增的翻页键 ＋ 缩窄的读数格都在这一行里）。
    let ids = [
        "instrument-label",
        "instrument-page-next-button",
        "instrument-install-0-button",
        "instrument-install-1-button",
    ];
    for (left_index, left) in ids.iter().enumerate() {
        for right in &ids[left_index + 1..] {
            let a = bounds_of(&tree, left);
            let b = bounds_of(&tree, right);
            assert!(
                !intersects(a, b),
                "[#210 装乐器行] `{left}` {a:?} ⟂ `{right}` {b:?} 不得相交"
            );
        }
    }
    assert_eq!(
        port.project().tracks.get(&lead).expect("轨在").sampler,
        None
    );

    report_line(&format!(
        "[#210 ①] 假库根 instruments={} names={:?} view_names={} tree_buttons=2/3（第 3 条在下一页）next=true prev=false",
        instruments.len(),
        library_instrument_names(),
        live.ui().get_instrument_names().row_count(),
    ));
}

/// 运行时节点包围盒（`(x, y, w, h)`；与 `pane_no_overlap` 同一口径）。
fn bounds_of(tree: &ControlTree, id: &str) -> (i64, i64, i64, i64) {
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    let bounds = node.bounds.expect("运行时节点必须有包围盒");
    (
        i64::from(bounds.x),
        i64::from(bounds.y),
        i64::from(bounds.width),
        i64::from(bounds.height),
    )
}

/// 半开区间相交（边贴边**不算**相交）。
fn intersects(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

// ===========================================================================
// ② + ③ + ④ 点击 / 换轨 / 撤销
// ===========================================================================
#[test]
fn clicking_instruments_switches_the_selected_track_only_and_undo_restores() {
    let _library = fake_library();
    let (project, lead, bass) = project_with_two_tracks();
    let (live, port) = live_window_with_undo(&project, "#210 点击");

    let before = port.display();

    // ---- ② 同一条选中轨：点 0 ⇒ A，点 1 ⇒ B（`sampler.path` 逐字）。
    select_track(live.ui(), lead);
    live.ui().invoke_instrument_install(0);
    let after_a = port.project();
    assert_eq!(
        after_a.tracks.get(&lead).expect("Lead 在").sampler,
        Some(SamplerBinding {
            path: COWSYNTH_SFZ.to_owned()
        }),
        "点第 0 条 ⇒ 选中轨的 `sampler.path` 必须是 A"
    );
    assert_eq!(
        after_a.tracks.get(&bass).expect("Bass 在").sampler,
        None,
        "没点到的那条轨必须一位没动"
    );
    assert_eq!(
        live.ui().get_instrument_label().to_string(),
        format!("乐器：{COWSYNTH_SFZ}")
    );

    live.ui().invoke_instrument_install(1);
    let after_b = port.project();
    assert_eq!(
        after_b.tracks.get(&lead).expect("Lead 在").sampler,
        Some(SamplerBinding {
            path: PASTABASS_SFZ.to_owned()
        }),
        "再点第 1 条 ⇒ **同一条**选中轨变成 B"
    );
    let after_two = port.display();
    assert_eq!(
        after_two.commit_count,
        before.commit_count + 2,
        "两次点击 = 恰好两次提交（{} → {}）",
        before.commit_count,
        after_two.commit_count
    );
    assert_eq!(
        after_two.undoable,
        before.undoable + 2,
        "两次点击 = 两步可撤销"
    );

    // ---- ③ 换到另一条轨：只有那条变。
    select_track(live.ui(), bass);
    live.ui().invoke_instrument_install(0);
    let after_third = port.project();
    assert_eq!(
        after_third.tracks.get(&bass).expect("Bass 在").sampler,
        Some(SamplerBinding {
            path: COWSYNTH_SFZ.to_owned()
        }),
        "选中 Bass 后点第 0 条 ⇒ **Bass** 装上 A"
    );
    assert_eq!(
        after_third.tracks.get(&lead).expect("Lead 在").sampler,
        Some(SamplerBinding {
            path: PASTABASS_SFZ.to_owned()
        }),
        "前一条轨必须保持 B（只有选中轨变）"
    );
    assert_eq!(port.display().undoable, before.undoable + 3);

    let tree = runtime_tree(live.ui());
    assert_eq!(
        tree.find_by_id("instrument-label")
            .expect("乐器读数行")
            .label,
        format!("乐器：{COWSYNTH_SFZ}"),
        "读数跟着**选中轨**（Bass）走"
    );

    // ---- ④ 撤销：Bass 回到 None，再撤一次 Lead 从 B 回到 A。
    assert!(
        port.perform(UiAction::Undo).changed(),
        "第 1 次撤销必须改变工程"
    );
    assert_eq!(
        port.project().tracks.get(&bass).expect("Bass 在").sampler,
        None,
        "撤销后 Bass 必须回到装之前的 `None`"
    );
    assert_eq!(
        port.project().tracks.get(&lead).expect("Lead 在").sampler,
        Some(SamplerBinding {
            path: PASTABASS_SFZ.to_owned()
        }),
        "撤销 Bass 不得动 Lead"
    );
    assert!(port.perform(UiAction::Undo).changed(), "第 2 次撤销");
    assert_eq!(
        port.project().tracks.get(&lead).expect("Lead 在").sampler,
        Some(SamplerBinding {
            path: COWSYNTH_SFZ.to_owned()
        }),
        "再撤一次 ⇒ Lead 从 B 回到 A（上一个值）"
    );

    report_line(&format!(
        "[#210 ②③④] click0={COWSYNTH_SFZ} click1={PASTABASS_SFZ}；\
         commit {}→{}；undoable {}→{}；撤销后 Lead=A / Bass=None",
        before.commit_count,
        port.display().commit_count,
        before.undoable,
        port.display().undoable,
    ));
}

// ===========================================================================
// ⑤ 能出声的证据：CLI `--track-sampler` ＋ `--export-wav` 的样本来源读数
// ===========================================================================
#[test]
fn the_cli_equivalent_installs_the_instrument_and_the_export_reads_real_samples() {
    let _library = fake_library();
    let out = std::env::temp_dir().join(format!(
        "yeban-210-export-{}-{}.wav",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    let options = yeban_app::cli::parse(&[
        "--track-sampler".to_owned(),
        COWSYNTH_SFZ.to_owned(),
        "--export-wav".to_owned(),
        out.display().to_string(),
    ])
    .expect("`--track-sampler … --export-wav …` 必须解析成功");
    let lines = yeban_app::cli::run_batch(&options).expect("批处理路径");

    let installed = lines
        .iter()
        .find(|line| line.starts_with("track-sampler:"))
        .expect("必须有一行 `track-sampler:`");
    assert!(
        installed.contains("ops=1") && installed.contains(COWSYNTH_SFZ),
        "装乐器必须恰好一条 Op 且路径逐字: {installed}"
    );
    let exported = lines
        .iter()
        .find(|line| line.starts_with("exported-wav:"))
        .expect("必须有一行 `exported-wav:`");
    // ⭐ 判据读数**逐字**（假库根里 cowsynth 的 SFZ：51 个区域、16 个去重样本）。
    // ⛔ 不用 probe 的能量：这一整段是导出链路对**模型 `sampler`** 的解码读数。
    assert!(
        exported.contains("sfz(tracks=1 regions=51 samples=16 silent=0 missing=[]"),
        "导出必须报告模型链条的 SFZ 读数（51 区域 / 16 个真解码样本 / 0 静音 / 0 缺件）: {exported}"
    );
    assert!(
        !exported.contains("回退") && !exported.contains("in-memory-sine"),
        "⛔ 不许回退到内存正弦（那就不是样本出声）: {exported}"
    );
    assert!(
        exported.contains("source=引擎乐器"),
        "导出的音源必须走引擎乐器分支（模型 `sampler` ⇒ SFZ），实测: {exported}"
    );
    assert!(out.is_file(), "WAV 必须真的落盘: {}", out.display());
    let bytes = std::fs::metadata(&out).expect("WAV 元数据").len();
    assert!(bytes > 44, "WAV 必须长于一个空头（实测 {bytes} 字节）");
    let _ = std::fs::remove_file(&out);

    report_line(&format!("[#210 ⑤] {installed}；{exported}"));
}
