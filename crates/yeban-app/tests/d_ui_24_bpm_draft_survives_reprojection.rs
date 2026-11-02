//! **正在敲的 BPM 草稿不再被重投影抹掉** —— `D-UI-24` 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 用户在走带条的速度框里敲到一半时，**任何一次与本框无关的重投影**（画布上点一下
//! 剪辑造成的那次提交、会话侧 `tools/call` 之后 `reproject.rs` 那一次注入）都
//! ⛔ 不许把草稿盖成权威读数；而**焦点离开那一句**与**回车那一句**照常把框里
//! 写成权威读数（既有口径"⛔ 不留下一个冒充成权威值的草稿"一位不动）。
//!
//! ## 缺陷的现读链条（修前）
//!
//! 1. `host::apply_view` 是 `bpm-display` 的**唯一**写点，修前那一句**无条件**写；
//! 2. 画布（`ui/workspace/arrangement_view.slint` / `ui/console/piano_roll.slint`）
//!    **没有** `FocusScope`（判据 ⑥ 现读数着这一条 ⇒ 它是本票的前提，不是猜测）⇒
//!    点剪辑**不会**让焦点离开 `bpm-input`；
//! 3. 而那一次点击会提交一条 `Op` ⇒ `refresh_undo_window` ⇒ 第 1 步那一句跑起来 ⇒
//!    用户敲了一半的数字被 `128.00` 盖掉（会话侧改动经 `reproject.rs` 同理）。
//!
//! ## 修法（一句话）
//!
//! `bpm-input` 的 `changed has-focus` 发出的**那条既有事件**（`[UI-A11Y-002]` §7.2 拿来
//! 喂 `InputContext::set_focus` 的同一次通知）顺手镜像进 `MainWindow.bpm-field-editing`；
//! `apply_view` 只在它为假时写 `bpm-display`。⛔ 不新造事件源、⛔ 不轮询焦点系统、
//! ⛔ 不在界面里存第二份"我以为焦点在哪"。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（对照臂：没在编辑 ⇒ 照常写） | 焦点事件**没发过**（默认假）时，注入一份"速度 140 ＋ 标题另写"的投影 ⇒ 框里变成 `140.00`。这条是 ② 的**分母**：没有它，② 的"没变"可能只是"注入整体没跑" |
//! | ②（编辑中 ⇒ 草稿活着，而注入**真的**跑了） | 焦点进入 ⇒ 打草稿 ⇒ 同一份注入 ⇒ 框里**仍是草稿**，但**窗口标题**已经换成那份投影的标题 ⇒ 保护只咬住这一格，⛔ 不是"跳过整次投影" |
//! | ③（真编辑走的也是这一条路） | 焦点进入 ＋ 草稿 ⇒ 按 `Cmd/Ctrl+Alt+D`（预备拍键，`D-UI-23` 落的那条**文档**臂）⇒ 文档里 `count_in_bars` 变了（证明重投影确实发生）而框里**仍是草稿** |
//! | ④（失焦就归位，不留永久分叉） | 焦点退出 ⇒ 再注入同一份投影 ⇒ 框里变回权威读数 `140.00` |
//! | ⑤（回车**解决**草稿：两条腿） | 被拒（`"9999"`）⇒ 文档一位不动、提交条数不变、框里写回权威 `128.00`；合法（`140`）⇒ 恰好 `+1` 条、文档 `140.0`、框里归一化成 `140.00`。两条都在**焦点仍在框里**的状态下做 ⇒ 证明清零发生在提交入口，⛔ 不是靠失焦事件 |
//! | ⑥（前提取证：画布够不到焦点） | `arrangement_view.slint` / `piano_roll.slint` / `session_view.slint` 里 `FocusScope` 与 `.focus()` **各 0 处**（代码行口径）⇒ 点画布不可能把焦点从文本框移走 |
//! | ⑦（出处纪律） | `host.rs` 代码行里：`set_bpm_display(` **恰好 1 处**（唯一写点 ⇒ 保护也只有一个落点）、`get_bpm_field_editing(` **1 处**、`set_bpm_field_editing(` **2 处**（`wire_input` 镜像 ＋ `wire_bpm_commit` 清零）；`app.slint` 代码行里 `bpm-field-editing` **1 处**（那一格声明） |
//!
//! ## 如实登记的边界（⛔ 不假装六格文本框都保护了）
//!
//! 1. **只有 BPM 一格**接了这条焦点通道：`ui/sidebar.slint` 的文件头逐字登记着
//!    "本框**不接** `[UI-A11Y-002]` §7.2 那条输入法通道（全仓库只有 `transport.slint`）"。
//!    ⚠ 因此调音台那格 `sampler-path-field` 同样会被重投影抹掉，本票⛔ 不顺手修它 ——
//!    它的取焦镜像要新开一条回调链（`mixer_console.slint` → `app.slint`），
//!    那一条留作下一票（登记在 `docs/ledger/app-zimmer-ui-notes.md`）。
//! 2. **导出弹窗那四格没有暴露面**：`apply_view` 一位不碰它们（现读：弹窗四格只在
//!    打开时被播种一次 `set_export_wav_path`），所以它们不是本缺陷的受害者。
//! 3. 回车之后焦点**仍在**框里而镜像已被清零 ⇒ 若用户**继续**敲第二串草稿，
//!    下一次重投影仍会盖掉它。如实登记：本票的口径是"一次回车 = 一次草稿有了裁决"，
//!    ⛔ 不是"焦点在框里就永远不写"（那会让非法输入的写回失效，见 ⑤）。
//! 4. 判据 ③ 用快捷键注入是因为**无头形态只有这一条真编辑路**可走：真窗口里焦点在
//!    `TextInput` 上时那一带修饰的键会被文本控件吃掉（`input.rs` 的既有策略），
//!    本条证的是"**一次改了文档并重投影的动作**不再抹草稿"这个漏斗，⛔ 不是"焦点在
//!    文本框里还能按动 DAW 快捷键"（那不是本票的命题，也不是产品行为）。
//!
//! ## 像素
//!
//! 本票在 `.slint` 侧只加了**一个属性声明**（`in property <bool> bpm-field-editing`）。
//! ⛔ 没有新增控件、⛔ 没有改任何几何 / 颜色 ⇒ `[UI-MCP-003]` 的基准帧逐位不变，
//! `[UI-TEST-001]` 的控件树覆盖也不变（属性不是节点）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部且每线程只能装配一次；每例一新线程由
//! libtest 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/d_ui_23_count_in_writes_the_document.rs`
//! 同一做法）。⑥ 与 ⑦ 不建窗口（只读源码）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与判决交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// `-D warnings` 会判红（与 `tests/d_ui_23_count_in_writes_the_document.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveControlPlane, LiveWiringOptions, build_live_ui_with};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 夹具速度（`yeban_model::samples::filled_project`）。
const FIXTURE_BPM: f64 = 128.0;

/// 另一份投影的速度（与夹具**不同值** ⇒ 注入才有判别力，⛔ 不是自比较）。
const OTHER_BPM: f64 = 140.0;

/// 用户敲到一半的草稿（单位：**屏幕上的那串字符**）。⚠ 它是**非法提交**的串，
/// 所以 ②/③/④ 里没有任何一条靠"这串恰好能被解析"成立。
const DRAFT: &str = "13";

/// 预备拍键的字面（逻辑键，与 `D-UI-23` 同一注入形状：`text, shift, control, alt, meta`）。
const COUNT_IN_KEY: (&str, bool, bool, bool, bool) = ("d", false, true, true, false);

fn displayed(bpm: f64) -> String {
    format!("{bpm:.2}")
}

/// 一台带权威会话、且**输入法那一条腿已经接好**的活窗口
/// （`live_surface.rs` 在装配时调 `host::wire_input` ⇒ 本文件的焦点事件走的是
/// **产品那一个**注册者，⛔ 判据自己不再注册第二个 —— `on_*` 是覆盖语义，
/// 再注册一次会把执行面持有的那份 `InputContext` 悄悄换掉）。
struct Harness {
    window: MainWindow,
    /// 执行面活着才有"活窗口"这件事；本文件不读它的任何方法。
    _plane: LiveControlPlane,
    port: Rc<UndoPort>,
    /// 注入用的视口宽度（单位：**逻辑像素**），取装配时那一份，⛔ 不另造一个数。
    width: f32,
}

fn assemble() -> Harness {
    let project = yeban_model::samples::filled_project();
    assert_eq!(
        project.bpm, FIXTURE_BPM,
        "夹具速度变了：本判据的读数要一起改"
    );
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<d-ui-24-bpm-draft>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开会话"),
    ));
    let ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面 + 输入法接线");
    let width = f64::from(ui.scene.viewport_width) as f32;
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    Harness {
        window,
        _plane: plane,
        port,
        width,
    }
}

/// 一份"速度与标题都不同于夹具"的投影（②/④ 用的那一次注入）。
///
/// 为什么连**标题**一起改：标题是 `apply_view` 的第二格注入（在 BPM 那一句**之前**），
/// 于是"标题变了"就是"这一次注入真的跑了"的现场证据 —— ⛔ 不能让"草稿没被盖掉"
/// 混成"整次注入没执行"那种假绿。
fn other_projection() -> (yeban_model::YebanProjectV1, ViewState) {
    let mut project = yeban_model::samples::filled_project();
    project.bpm = OTHER_BPM;
    project.title = "另一版工程".to_owned();
    let view = ViewState::from_project(&project).expect("另一份工程必须能投影");
    assert_eq!(
        view.bpm_display,
        displayed(OTHER_BPM),
        "投影的读数格式是 `{{:.2}}`"
    );
    (project, view)
}

/// 权威文档的一份快照（读不到 ⇒ 前提不成立，直接判据红）。
fn live_project(port: &UndoPort) -> yeban_model::YebanProjectV1 {
    port.try_project()
        .expect("端口必须有一份权威工程（没有 ⇒ 本文件的每一条都无从判起）")
}

/// 把源码压成**只有代码行**：注释里出现的字面量⛔ 不算实现站点
/// （口径实测于 run `41516839490`，注释把方法名提一句就把"唯一实现"数成两处）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read(relative: &str) -> String {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("读 {}: {err}", path.display()))
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// ①（对照臂）：焦点事件从未发过 ⇒ 保护门是假的 ⇒ 注入照常写。
#[test]
fn a_re_projection_writes_the_field_when_nothing_is_being_edited() {
    let h = assemble();
    assert!(
        !h.window.get_bpm_field_editing(),
        "刚装配好的窗口⛔ 不该已经在编辑态（那条焦点事件的默认值必须是假）"
    );
    assert_eq!(
        h.window.get_bpm_display().as_str(),
        displayed(FIXTURE_BPM),
        "装配后的读数必须来自权威工程"
    );

    let (_project, view) = other_projection();
    host::apply_view(&h.window, &view, h.width, 0.0);

    assert_eq!(
        h.window.get_bpm_display().as_str(),
        displayed(OTHER_BPM),
        "没在编辑 ⇒ 重投影必须把权威读数写进框里（②的保护⛔ 不许外溢成整次注入不跑）"
    );
    assert_eq!(
        h.window.get_window_title().as_str(),
        "另一版工程",
        "标题那一格必须跟着这次注入走"
    );
}

/// ②（本票的正面）：编辑中 ⇒ 草稿活着，而同一次注入的**别的**格子照常落地。
#[test]
fn the_draft_survives_a_re_projection_while_the_field_is_being_edited() {
    let h = assemble();
    // 事件源 = `bpm-input` 的 `changed has-focus`（与 §7.2 喂 `InputContext` 同一次通知）。
    h.window.invoke_ime_focus_changed(true);
    assert!(
        h.window.get_bpm_field_editing(),
        "焦点进入必须把编辑门置起来（`wire_input` 里那一句镜像）"
    );
    // 双向绑定 ⇒ 用户敲的字就是这一格（与 `tests/ui_bpm_field.rs` ③ 同一注入形状）。
    h.window.set_bpm_display(DRAFT.into());

    let (_project, view) = other_projection();
    host::apply_view(&h.window, &view, h.width, 0.0);

    assert_eq!(
        h.window.get_bpm_display().as_str(),
        DRAFT,
        "编辑中 ⇒ 重投影⛔ 不许盖掉草稿（修前这一条就是红的：草稿被写成 `140.00`）"
    );
    // 同一次注入的**另一格**必须已经落地 ⇒ 保护只咬住 BPM 这一格。
    assert_eq!(
        h.window.get_window_title().as_str(),
        "另一版工程",
        "标题没跟着变 ⇒ 说明整次注入没跑，那上面那条\"草稿还在\"就是假绿"
    );
    // 文档⛔ 不因为"这次不写界面"而少走一步：注入是**读**权威，写不写框与工程无关。
    assert!(
        (live_project(&h.port).bpm - FIXTURE_BPM).abs() < f64::EPSILON,
        "一次注入⛔ 不是一次编辑：权威速度必须原封不动"
    );
}

/// ③（真编辑的那一次重投影）：按一次**改文档**的键，草稿仍在，而文档真的改了。
#[test]
fn a_document_edit_that_re_projects_does_not_lose_the_draft() {
    let h = assemble();
    h.window.invoke_ime_focus_changed(true);
    h.window.set_bpm_display(DRAFT.into());

    let bars_before = live_project(&h.port).transport.count_in_bars;
    let commits_before = h.port.display().commit_count;
    // 预备拍键：`Cmd/Ctrl+Alt+D` ⇒ 一条 `Op::SetTransportConfig` ⇒ `refresh_undo_window`。
    assert!(
        h.window.invoke_key_action(
            COUNT_IN_KEY.0.into(),
            COUNT_IN_KEY.1,
            COUNT_IN_KEY.2,
            COUNT_IN_KEY.3,
            COUNT_IN_KEY.4,
        ),
        "预备拍键必须被消费（否则本条没有发生重投影，②的\"没变\"就成了无源之水）"
    );
    let after = live_project(&h.port).transport.count_in_bars;
    assert_ne!(
        after, bars_before,
        "文档里的预备拍必须被这一次按键改掉（单位：小节）—— 没改 ⇒ 没有重投影，本条无从判"
    );
    assert_eq!(
        h.port.display().commit_count,
        commits_before + 1,
        "一次按键恰好一条提交（`D-UI-23` ② 的口径，这里只复核\"真的提交了\"）"
    );
    assert_eq!(
        h.window.get_bpm_display().as_str(),
        DRAFT,
        "由这一次**文档编辑**触发的重投影⛔ 不许盖掉正在敲的草稿"
    );
}

/// ④（不失永久分叉）：焦点退出后，下一次注入把框里写回权威读数。
#[test]
fn leaving_the_field_restores_the_authority_reading() {
    let h = assemble();
    h.window.invoke_ime_focus_changed(true);
    h.window.set_bpm_display(DRAFT.into());
    let (_project, view) = other_projection();
    host::apply_view(&h.window, &view, h.width, 0.0);
    assert_eq!(h.window.get_bpm_display().as_str(), DRAFT, "先确认保护生效");

    h.window.invoke_ime_focus_changed(false);
    assert!(
        !h.window.get_bpm_field_editing(),
        "焦点退出必须把编辑门清零（否则框会**永远**不跟随权威）"
    );
    host::apply_view(&h.window, &view, h.width, 0.0);
    assert_eq!(
        h.window.get_bpm_display().as_str(),
        displayed(OTHER_BPM),
        "失焦之后必须回到权威读数 ⇒ 本票⛔ 造出第二个事实源"
    );
}

/// ⑤（回车**解决**草稿）：被拒 ⇒ 写回权威；合法 ⇒ 归一化。两条都在焦点仍在框里时做。
#[test]
fn enter_resolves_the_draft_on_both_the_reject_and_the_accept_path() {
    let h = assemble();
    let ui = &h.window;

    // ---- 被拒的那一支（越界：`MAX_BPM = 999.0`）----
    ui.invoke_ime_focus_changed(true);
    ui.set_bpm_display("9999".into());
    let commits_before = h.port.display().commit_count;
    ui.invoke_bpm_commit("9999".into());
    assert_eq!(
        h.port.display().commit_count,
        commits_before,
        "非法输入⛔ 不许提交"
    );
    assert!(
        (live_project(&h.port).bpm - FIXTURE_BPM).abs() < f64::EPSILON,
        "非法输入之后权威工程必须原封不动"
    );
    assert_eq!(
        ui.get_bpm_display().as_str(),
        displayed(FIXTURE_BPM),
        "非法输入之后必须写回权威读数（`tests/ui_bpm_field.rs` ③ 的既有口径，\
         本票的编辑保护⛔ 不许把它挡住）"
    );
    assert!(
        !ui.get_bpm_field_editing(),
        "回车本身就把草稿判成\"已有裁决\"⇒ 门必须被 `wire_bpm_commit` 清零（失焦事件并**没**有发过）"
    );

    // ---- 合法的那一支 ----
    ui.invoke_ime_focus_changed(true);
    ui.set_bpm_display("140".into());
    let before = h.port.display().commit_count;
    ui.invoke_bpm_commit("140".into());
    assert_eq!(
        h.port.display().commit_count,
        before + 1,
        "一次回车 = 恰好一次提交"
    );
    assert!(
        (live_project(&h.port).bpm - OTHER_BPM).abs() < f64::EPSILON,
        "权威速度必须真的变成 {OTHER_BPM}"
    );
    assert_eq!(
        ui.get_bpm_display().as_str(),
        displayed(OTHER_BPM),
        "提交之后必须归一化成 `{{:.2}}` 读数（草稿 `\"140\"` ⛔ 不留在框里）"
    );
}

/// ⑥（前提取证）：画布那三层**够不到**焦点 ⇒ 点剪辑不会让文本框失焦。
#[test]
fn the_canvas_layers_take_no_focus_so_a_click_cannot_leave_the_text_field() {
    for relative in [
        "ui/workspace/arrangement_view.slint",
        "ui/console/piano_roll.slint",
        "ui/workspace/session_view.slint",
    ] {
        let code = code_only(&read(relative));
        assert_eq!(
            count(&code, "FocusScope"),
            0,
            "{relative} 里出现 `FocusScope` ⇒ 本票的前提（点画布不换焦点）不再成立，\
             必须重读这条链再判"
        );
        assert_eq!(
            count(&code, ".focus("),
            0,
            "{relative} 里出现程序化取焦 ⇒ 同上"
        );
    }
}

/// ⑦（出处纪律）：写点唯一、门只有两个写者与一个读者、`.slint` 只有那一格声明。
#[test]
fn the_write_points_stay_singular() {
    let host_code = code_only(&read("src/host.rs"));
    assert_eq!(
        count(&host_code, "set_bpm_display("),
        1,
        "`bpm-display` 必须只有**一个**写点（`apply_view`）：\
         开第二个写点等于把\"什么时候能不写\"这个问题变成两处答案"
    );
    assert_eq!(
        count(&host_code, "get_bpm_field_editing("),
        1,
        "编辑门必须只有**一个**读者（`apply_view` 的那一句保护）"
    );
    assert_eq!(
        count(&host_code, "set_bpm_field_editing("),
        2,
        "编辑门必须只有**两个**写者：`wire_input` 的镜像 ＋ `wire_bpm_commit` 的清零"
    );
    assert_eq!(
        count(&code_only(&read("ui/app.slint")), "bpm-field-editing"),
        1,
        "`app.slint` 里这一格只许出现一次（那条声明）：\
         出现第二处就说明界面里开始自己存第二份焦点状态"
    );
    // 双向绑定与提交回调⛔ 不许被本票换成"单向 ＋ 宿主自己判断"。
    let transport_code = code_only(&read("ui/transport.slint"));
    assert!(
        transport_code.contains("text <=> root.bpm-display;"),
        "输入框必须**双向**绑定（`tests/ui_bpm_field.rs` ⑥ 钉着同一句）"
    );
}
