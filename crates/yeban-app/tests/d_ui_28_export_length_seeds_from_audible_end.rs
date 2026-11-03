//! **导出 WAV 的时长缺省 = 工程可闻末端**（`D-UI-28`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 打开「导出 WAV」弹窗时，**时长那一格自动填上整首曲子的长度**（而不是写死 8 秒），
//! 于是"按缺省点导出"产出的是**全曲**；用户改过的那一格重开时逐字保留；清空它 ⇒ 重新
//! 播种；工程变长 ⇒ 重新播种；没有内容的工程 ⇒ ⛔ 不编一个假数。
//!
//! ## 为什么缺口是真的（不是审美问题）
//!
//! `ui/dialogs/export_wav_dialog.slint` 原来在**两处**写死 `seconds: "8"`（根属性与对话框
//! 自己的 `in-out`）。一份几分钟的工程，创作者点开弹窗、什么都不改、按「导出」⇒ 拿到
//! 8 秒的片段，**界面上没有任何地方说这件事**。这是"UI 看起来能用、实际会静默毁掉产物"
//! 的那一类缺口。
//!
//! ## 为什么补在宿主、不补在 `resolve`
//!
//! [`yeban_app::export_wav::WavExportOptions::resolve`] 的 8 秒缺省是 **GUI / CLI / MCP
//! 三方共用**的那一份，五处既有判据把 `frames=384000` 钉在上面（判据 ① 复钉一次）。
//! 播种落在"打开弹窗"这一刻 ⇒ 共用口径一位不改，缺省行为逐字节不变。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（未打开） | 时长格初始为**空串**（⛔ 不再是 `"8"`）；此时直接导出仍 `frames=384000` ⇒ 共用的 8 秒缺省一位没动 |
//! | ②（打开播种） | `open-export-wav` ⇒ 该格变成 `"7.387"` = `filled_project` 可闻末端（段末 15360 tick @ 128 → 132 的**分段**换算） |
//! | ③（不截断） | 按播好的值导出 ⇒ `frames=354576` **≥** 内容末端帧数 `354545` ⇒ 最后一段一个样本不丢，且字节数与磁盘一致 |
//! | ④（用户值保留） | 该格填 `0.5` ⇒ 关 → 重开，**仍是** `0.5`（⛔ 不被宿覆盖） |
//! | ⑤（清空重播） | 该格清成空串 ⇒ 关 → 重开，回到 `"7.387"`（⇒ "保留用户值"⛔ 不是"永远不再播种"） |
//! | ⑥（工程变长 ⇒ 刷新） | 提交 `Op::SetTempo { 128 → 20 }` ⇒ 重开变 `"27.637"`（旧缺省 8 秒在这一格上会**静默切掉 19.6 秒**） |
//! | ⑦（空工程不编数） | 换成 `default_project()`（只有母线轨 ⇒ 末端 0）⇒ 重开该格**仍为空**，⛔ 不播一个算出来的假值 |
//! | ⑧（上限） | 把末端推到 4735 秒 ⇒ 播 `"3600.000"`（与 MCP `MAX_RENDER_FRAMES` 同一小时上限），⛔ 不播一个会挂死 UI 线程的数（`D-EX-2` 登记的同步导出今天仍在） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_export_dialog.rs` 同款）。
//! 判据 ③ 之外**不做真导出**（⑥⑧ 只读那一格）：一次 3600 秒的导出是 172.8 M 帧，
//! CI 里既没必要也没有意义。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/a11y_screen_reader.rs`。
#[allow(dead_code)]
mod live;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 测试产物落在临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-d-ui-28-{tag}-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 取回执里的一个顶层字段（`key=value` 以 `:` 分隔；只做**前缀**匹配）。
///
/// 本判据只读 `action` / `outcome` / `frames` / `bytes`，回执里可能含 `:` 的 `path=`
/// 与 `clipping_note=` 都不取值 ⇒ 不需要引号感知（同款见 `tests/export_dialog_button_focus.rs`）。
fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    for part in receipt.split(':') {
        if let Some(value) = part.strip_prefix(needle.as_str()) {
            return value.to_owned();
        }
    }
    panic!("回执里没有字段 `{key}`: {receipt}");
}

fn frames_of(receipt: &str) -> u64 {
    field_of(receipt, "frames")
        .parse()
        .unwrap_or_else(|error| panic!("`frames` 不是整数（{error}）: {receipt}"))
}

fn assert_nonempty_file(path: &Path) {
    let meta = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("导出的产物没落盘 {}: {error}", path.display()));
    assert!(meta.is_file(), "{} 不是文件", path.display());
    assert!(meta.len() > 0, "{} 是 0 字节", path.display());
}

#[test]
fn the_export_length_field_seeds_from_the_audible_end_of_the_project() {
    let scratch = Scratch::new("length");
    let export = scratch.0.join("seeded-length.wav");

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 D-UI-28: 时长播种>", "yeban-app", project.clone(), 1)
            .expect("打开撤销会话"),
    ));

    // 导出落点 = 临时目录里那一个文件（路径格与时长格是同一个"打开时播种"的两半）。
    // SAFETY: 本测试二进制只有一个开窗口的 `#[test]` ⇒ 没有第二个线程读这个变量；
    // 设置发生在任何装配之前。
    unsafe {
        std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", &export);
    }

    let live_ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("Tier-1 平台 + 活窗口 + 模块接线");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());
    let ui = &window;

    // ---- ① 未打开 ⇒ 那一格是空的，且空 = 后端的 8 秒（共用口径一位没动）----
    // 这一条同时是"五处既有判据仍钉住 384000"的复钉：播种只在打开那一刻发生。
    assert_eq!(
        ui.get_export_wav_seconds().to_string(),
        "",
        "① 未打开时时长格必须是空串（⛔ 不再是写死的 \"8\"）"
    );
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "① 空的那一格应按后端缺省导出成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(
        frames_of(&receipt),
        384_000,
        "① 空 = `DEFAULT_WAV_SECONDS` × 48 kHz = 384000 帧（GUI / CLI / MCP 共用那一份）"
    );

    // ---- ② 打开 ⇒ 播成可闻末端 ----
    // filled_project：段末 15360 tick；0..7680 @ 128 BPM、7680..15360 @ 132 BPM ⇒
    // round(180000 + 174545.4545…) = 354545 帧 ÷ 48000 = 7.386354…秒 ⇒ **向上**取整到毫秒。
    ui.invoke_open_export_wav();
    assert!(ui.get_export_wav_open(), "② 前置：弹窗必须开着");
    assert_eq!(
        ui.get_export_wav_seconds().to_string(),
        "7.387",
        "② 打开必须把该格播成整首曲子的秒数（7.387 = 354545 帧 @ 48 kHz 向上取整到毫秒）"
    );

    // ---- ③ 按播好的值导出 ⇒ 覆盖到内容末端，不再静默截断 ----
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "③ 按播好的时长导出应当成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "export-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    let seeded_frames = frames_of(&receipt);
    // 7.387 秒 × 48 kHz（f32 存得下这个数，`resolve` 用 f64 乘）= 354576 帧。
    assert_eq!(seeded_frames, 354_576, "{receipt}");
    // ⭐ 这一条才是本票的**承诺**：产物长度 ≥ 工程可闻末端（7.387 是向上取整 ⇒ 多出 31 帧）。
    assert!(
        seeded_frames >= 354_545,
        "③ 导出帧数必须覆盖内容末端 354545 帧，实际 {seeded_frames}"
    );
    assert_nonempty_file(&export);
    assert_eq!(
        std::fs::metadata(&export)
            .expect("导出文件")
            .len()
            .to_string(),
        field_of(&receipt, "bytes"),
        "③ 回执报的字节数必须等于磁盘上的字节数"
    );

    // ---- ④ 用户填过 ⇒ 重开逐字保留（与路径那一格同一口径）----
    ui.invoke_close_export_wav();
    ui.set_export_wav_seconds("0.5".into());
    ui.invoke_open_export_wav();
    assert_eq!(
        ui.get_export_wav_seconds().to_string(),
        "0.5",
        "④ 用户改过的时长 ⛔ 不许被重开的播种覆盖"
    );

    // ---- ⑤ 清空 ⇒ 重开重新播种（保留 ≠ "从此不再播"）----
    ui.invoke_close_export_wav();
    ui.set_export_wav_seconds("".into());
    ui.invoke_open_export_wav();
    assert_eq!(
        ui.get_export_wav_seconds().to_string(),
        "7.387",
        "⑤ 清空之后再打开必须重新按当前工程播种"
    );

    // ---- ⑥ 工程变长 ⇒ 重开刷新到新末端 ----
    // `Op::SetTempo` 只写顶层 `bpm`（⛔ 不碰 `tempo_map`）⇒ 分段变成 0..7680 @ 20、
    // 7680..15360 @ 132 ⇒ round(1152000 + 174545.4545…) = 1326545 帧 = 27.63635…秒。
    port.commit_ops(
        2,
        "判据 ⑥：把速度放慢到 20 BPM",
        vec![yeban_model::Op::SetTempo {
            old_bpm: 128.0,
            new_bpm: 20.0,
        }],
    )
    .expect("提交 SetTempo");
    ui.invoke_close_export_wav();
    ui.invoke_open_export_wav();
    let lengthened = ui.get_export_wav_seconds().to_string();
    assert_eq!(
        lengthened, "27.637",
        "⑥ 工程变长之后重开必须跟着变（27.637 = 1326545 帧 @ 48 kHz）"
    );
    assert!(
        lengthened.parse::<f32>().expect("⑥ 播的是合法秒数") > 8.0,
        "⑥ 这一条必须落在**旧缺省会截断**的那一侧才有意义（实际 {lengthened} 秒）"
    );

    // ---- ⑦ 空工程 ⇒ ⛔ 不编一个假数（留空 = 后端缺省继续生效）----
    port.open_project_session(
        "<判据 D-UI-28 ⑦: 空工程>",
        yeban_model::samples::default_project(),
        Vec::new(),
        3,
    )
    .expect("换工程（没有内容的工程）");
    ui.invoke_close_export_wav();
    ui.invoke_open_export_wav();
    assert_eq!(
        ui.get_export_wav_seconds().to_string(),
        "",
        "⑦ 没有任何剪辑与段落的工程 ⇒ 末端 = 0 ⇒ ⛔ 播 \"0.000\"、也⛔ 播一个编出来的数，必须留空"
    );

    // ---- ⑧ 上限 ⇒ 播 3600.000，⛔ 播一个会把 UI 线程挂死的数 ----
    // ⑦ 换过工程 ⇒ 现在这份是**原样** filled（`bpm` 回到 128）。挂一条 0..10_000_000 的段落：
    // 0..7680 @ 128 与 7680..10_000_000 @ 132 ⇒ round(180000 + 227098181.81…) = 227278182 帧
    // = 4735.003…秒 ⇒ 超过一小时 ⇒ 封顶。⛔ 本判据**不**导出这一格（3600 秒 = 172.8 M 帧）。
    port.open_project_session("<判据 D-UI-28 ⑧: 超长工程>", project, Vec::new(), 4)
        .expect("换回 filled 工程（用来挂段落）");
    // ⚠ `Op::SetSection` 的前置条件之一是 `new_section.id == section_id`（两处必须是**同一个**
    // ULID）⇒ 先绑一个再克隆，⛔ 不能各调一次 `EntityId::new()`。
    let long_section_id = yeban_model::ids::EntityId::new();
    port.commit_ops(
        5,
        "判据 ⑧：把段落末端推到一千万 tick 之外",
        vec![yeban_model::Op::SetSection {
            section_id: long_section_id.clone(),
            old_section: None,
            new_section: yeban_model::SectionV3 {
                id: long_section_id,
                name: "判据 ⑧ 的超长段落".to_owned(),
                start_tick: 0,
                end_tick: 10_000_000,
                color: None,
            },
        }],
    )
    .expect("提交 SetSection");
    ui.invoke_close_export_wav();
    ui.invoke_open_export_wav();
    assert_eq!(
        ui.get_export_wav_seconds().to_string(),
        "3600.000",
        "⑧ 超过一小时必须**封顶在 3600.000**（与 MCP `MAX_RENDER_FRAMES` 同一上限）"
    );

    report_line(&format!(
        "[d-ui-28] 未打开=空/384000 ⇒ 打开=7.387 ⇒ 导出={seeded_frames} 帧（末端 354545）\
         ⇒ 用户值 0.5 保留 ⇒ 清空重播 7.387 ⇒ 20 BPM 刷成 {lengthened} ⇒ 空工程仍空 ⇒ 超长封顶 3600.000"
    ));
}
