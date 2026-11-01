//! 缺陷 **D-UI-3** 的判据：控制面能不能注入**带修饰键的组合**（chord）。
//!
//! 一句话的问题陈述（现读，⛔ 不采信报告措辞）：`[UI-TEST-002]` §12.4 要求
//! `dispatch_key_press(key_code)` 分发 `Shift+Enter`，但那条签名只有**一个键名**、
//! 没有修饰位，`slint::platform::WindowEvent::KeyPressed` 本身也**不带**修饰字段
//! ⇒ 平台事件那条路永远表达不了 `Shift+Z`（全曲总览）、`Shift+↑/↓`（整八度）、
//! `Cmd/Ctrl+*` 全家。本文件证明新的 `ui/dispatch_key_chord` 这一格**真的通到界面**：
//!
//! | 判据 | 证明 |
//! | :--- | :--- |
//! | `every_port_chord_key_reaches_the_single_parse_point_of_the_host` | **词表同源**：端口 `ChordKey::text()` 出来的那一个码位，宿主的唯一解析点 `LogicalKey::from_text` 认得，且解析出的**就是**那一个逻辑键（词表漂移会红在这里，而不是红在"按了没反应"） |
//! | `injecting_shift_z_through_the_port_fits_the_whole_project_into_the_viewport` | **真的改了投影**：经端口注入 `Shift+Z` 之后，卷帘缩放 == 模型侧独立算出的 `ceil(内容跨度 / 视口宽)`，滚动回到 0，回执 `consumed == true`，且第二次注入幂等 |
//! | `an_unbound_chord_reports_not_consumed_and_leaves_everything_alone` | **不假装成功**：策略表明写「未被点名 ⇒ 放行」的那一格（`Cmd/Ctrl+Alt+Shift+↑`）返回 `consumed == false`，缩放/滚动/工具/播放四位读数一位没动 |
//!
//! ## 为什么落点是 `invoke_key_action` 而不是平台事件
//!
//! 见 `crates/yeban-app/src/live_surface.rs` 的 `dispatch_key_chord_impl` 文档与
//! `docs/adr/ADR-0061_ui_dispatch_key_chord.md`（`Proposed`）。要点只有一句：用户真键盘上的
//! `WindowEvent::KeyPressed` → `.slint` 的 `key-handler` → `key_action(text, shift, ctrl, alt, meta)`
//! → `host::wire_keys` 的 `on_key_action` 这条链里，`invoke_key_action` 就是它的**入口**，
//! 因此这条注入与用户键走**同一个回调、同一张策略表、同一个 UndoPort** ⇒ 不是旁路。
//!
//! ## 每个 `#[test]` 只装一次 Tier-1 平台
//!
//! 同 `live_ui_mcp.rs`：`slint::platform::set_platform` 是线程局部的（libtest 每测试一线程），
//! 同一个测试里**不能**建第二个 `LiveUi`。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分（三条 chord 判据 + 一个 `LiveUi`）⇒ 其余项在
// 本测试目标里是 dead_code（`-D warnings` 会判红）。同款见 `tests/ui_bpm_field.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;
use yeban_app::input::LogicalKey;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::{ChordKey, KeyChord, KeyChordModifiers, Permission};
use yeban_ui_test_port::render::report_line;

/// 缩放的上下界（`bridge::MIN/MAX_TICKS_PER_PIXEL`）—— 写成**字面量**做独立 oracle；
/// 常量本身由 `bridge` 的单元判据钉住。单位：tick / 逻辑像素。
const MIN_TPP: u64 = 1;
const MAX_TPP: u64 = 960;

const NOW: u64 = 1_760_000_000_000;

/// 内容的末端（tick）：剪辑右缘与段落右缘里更远的那个。
///
/// 这是**模型侧**的独立算法（与 `host::zoom_action` 读的投影 `content_end_tick()` 不同源），
/// 用它做 oracle 才有意义 —— 拿投影验投影等于什么都没验。口径与 `tests/live_ui_mcp.rs`
/// 的同名助手逐字一致（那条判据已在 CI 绿过，这里不另造第三份口径）。
fn model_content_end(project: &yeban_model::project::YebanProjectV1) -> u64 {
    let clip_end = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .map(|placement| placement.start_tick + placement.duration_ticks)
        .max()
        .unwrap_or(0);
    let section_end = project
        .sections
        .values()
        .map(|section| section.end_tick)
        .max()
        .unwrap_or(0);
    clip_end.max(section_end)
}

/// 装配一条**测试模式 + `Interactive`** 的真实控制面。
///
/// ⚠ `undo` 必须是 `Some`：`ZoomToFit` 的前置条件就是撤销端口（`host.rs` 里
/// `Action::ZoomToSelection | Action::ZoomToFit` 那一支，`undo` 为 `None` 时**直接不消费**）——
/// 那正是缺陷 `D-UI-1` 修掉的那一格。装配与判据读的是**同一个** `project`。
fn assemble_interactive(project: &yeban_model::project::YebanProjectV1) -> live::LiveUi {
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据:D-UI-3 chord 注入>",
            "yeban-app",
            project.clone(),
            NOW,
        )
        .expect("打开撤销会话"),
    ));
    let wiring = live::LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        save_path: None,
        engine_quanta: 0,
        undo: Some(Rc::clone(&port)),
        proposal_source: yeban_app::proposals::ProposalSource::none(),
    };
    live::build_live_ui_with(project, &wiring).expect("真实界面 + Tier-1 执行面")
}

/// 判据 ①（**默认构建，无需装配窗口**）：端口词表与宿主唯一解析点同源。
#[test]
fn every_port_chord_key_reaches_the_single_parse_point_of_the_host() {
    // 数什么：**15 个具名 chord 键**，每一个都要满足三件事 ——
    //   `text()` 恰一个码位、`LogicalKey::from_text` 认得它、`name()` 能解析回同一个键。
    // 期望的逻辑键逐条写死（⛔ 不从端口代码推），这样两侧任何一侧改动都会被这格抓住。
    let expected: [(ChordKey, LogicalKey); 15] = [
        (ChordKey::Space, LogicalKey::Space),
        (ChordKey::Tab, LogicalKey::Tab),
        (ChordKey::Escape, LogicalKey::Escape),
        (ChordKey::Enter, LogicalKey::Enter),
        (ChordKey::Backspace, LogicalKey::Backspace),
        (ChordKey::Delete, LogicalKey::Delete),
        (ChordKey::F5, LogicalKey::F5),
        (ChordKey::F6, LogicalKey::F6),
        (ChordKey::F7, LogicalKey::F7),
        (ChordKey::F8, LogicalKey::F8),
        (ChordKey::F9, LogicalKey::F9),
        (ChordKey::UpArrow, LogicalKey::UpArrow),
        (ChordKey::DownArrow, LogicalKey::DownArrow),
        (ChordKey::LeftArrow, LogicalKey::LeftArrow),
        (ChordKey::RightArrow, LogicalKey::RightArrow),
    ];

    assert_eq!(
        expected.len(),
        ChordKey::NAMED.len(),
        "端口新增具名键时必须在这里也写一条期望（穷尽，⛔ 不靠通配）"
    );
    for (named, want) in ChordKey::NAMED.into_iter().zip(expected) {
        assert_eq!(named, want.0, "NAMED 的顺序与本表的顺序必须一致");
        let text = named.text();
        assert_eq!(
            text.chars().count(),
            1,
            "「{}」的 `text` 必须**恰好一个**码位（Slint 的 `KeyEvent.text` 就是这个形状）",
            named.name()
        );
        assert_eq!(
            LogicalKey::from_text(&text),
            Some(want.1),
            "宿主唯一解析点必须把「{}」认成「{want:?}」（词表漂移就是在这里红）",
            named.name()
        );
        assert_eq!(
            ChordKey::parse(&named.name()).expect("具名键的规范名必须能解析回来"),
            named,
            "`name()` 与 `parse()` 必须互逆"
        );
    }

    // 可打印键：**大小写由解析点统一**，端口不猜（`Shift+Z` 到达宿主时 `text == "Z"`，
    // 而策略表用 `Character('z')` ⇒ 修饰位与键位是两件事，这一点必须两侧同口径）。
    let upper = ChordKey::parse("Z").expect("「Z」是一个可打印键");
    assert_eq!(
        upper,
        ChordKey::Character('Z'),
        "端口**原样**保留输入的大小写"
    );
    assert_eq!(
        LogicalKey::from_text(&upper.text()),
        Some(LogicalKey::Character('z')),
        "「Shift+Z」与裸「z」的区分只能靠修饰位，⛔ 不能靠 `text` 的大小写"
    );

    // 多字符 / 组合写法**不是**一个键：必须报错，⛔ 不得静默当成某个字符。
    assert!(
        ChordKey::parse("shift+z").is_err(),
        "「keyName」只收**一个键**，修饰位在 `shift` / `ctrl` / `alt` / `meta` 那四个布尔里"
    );
    assert!(
        LogicalKey::from_text("shift+z").is_none(),
        "宿主那侧同样把多字符判成「不是一次按键」"
    );
}

/// 判据 ②（**默认构建**）：经**端口**注入 `Shift+Z` ⇒ 卷帘真的缩放到全曲。
#[test]
fn injecting_shift_z_through_the_port_fits_the_whole_project_into_the_viewport() {
    let project = yeban_model::samples::filled_project();
    let mut ui = assemble_interactive(&project);

    let viewport_px = slint::ComponentHandle::window(ui.ui()).size().width;
    assert_eq!(viewport_px, 1920, "Tier-1 装配的视口宽 = DemoScene 的 1920");
    let vw = u64::from(viewport_px);
    let fit = |span: u64| span.div_ceil(vw).clamp(MIN_TPP, MAX_TPP);

    let before = ui.ui().get_roll_ticks_per_pixel();
    assert_eq!(
        before, 30,
        "起点缩放 = `DEFAULT_TICKS_PER_PIXEL`（属性默认值）"
    );
    assert_eq!(ui.ui().get_roll_scroll_x(), 0.0, "起点偏移 = 0");

    let chord = KeyChord::new(
        ChordKey::parse("z").expect("「z」是一个可打印键"),
        KeyChordModifiers {
            shift: true,
            ..KeyChordModifiers::default()
        },
    );
    report_line(&format!(
        "[d-ui-3] 注入 chord: {}（`text` 码位数 {}，shift={}）",
        chord.name(),
        chord.text().chars().count(),
        chord.modifiers.shift
    ));

    let consumed = ui
        .dispatch_key_chord(&chord)
        .expect("`Interactive` 权限下 chord 注入必须被放行");
    assert!(
        consumed,
        "「Shift+Z」（全曲总览）在策略表里有点名 ⇒ 界面必须消费它；`false` 意味着注入落到了没人接的地方"
    );

    let content_end = model_content_end(&project);
    assert!(
        content_end > 0,
        "夹具必须有内容（否则 ZoomToFit 的前置条件不成立，这条判据会假绿）: {content_end}"
    );
    let expected = fit(content_end);
    assert_ne!(
        expected, before as u64,
        "夹具的总览缩放必须**不等于**默认缩放，否则这条判据什么都没证: 两者都是 {expected}"
    );
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        i32::try_from(expected).expect("tpp 落在 i32 内"),
        "注入「{}」之后缩放应为 ceil(内容跨度 {content_end} / 视口宽 {vw}) = {expected}",
        chord.name()
    );
    assert_eq!(
        ui.ui().get_roll_scroll_x(),
        0.0,
        "全曲总览必须回到原点（滚动 0）—— `zoom_action` 对 ZoomToFit 这一支写死 0"
    );

    // **幂等**：同一个 chord 再注入一次，仍然消费，但不动投影（`host.rs` 的
    // "已经在这个缩放上"那一格）。这条防止"每次都重投影"被当成正常行为。
    let consumed_again = ui.dispatch_key_chord(&chord).expect("第二次注入同样被放行");
    assert!(consumed_again, "语义已达成 ⇒ 仍应报告消费");
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        i32::try_from(expected).expect("tpp 落在 i32 内"),
        "第二次注入不得再改缩放"
    );

    report_line(&format!(
        "[d-ui-3] 「Shift+Z」之后: roll_ticks_per_pixel {} → {}（模型侧 oracle = {expected}），scroll_x = 0",
        before,
        ui.ui().get_roll_ticks_per_pixel()
    ));
}

/// 判据 ③（**默认构建**）：策略表里"未被点名 ⇒ 放行"的那一格必须报 `consumed == false`，
/// 并且**一位状态都不动**。
#[test]
fn an_unbound_chord_reports_not_consumed_and_leaves_everything_alone() {
    let project = yeban_model::samples::filled_project();
    let mut ui = assemble_interactive(&project);

    let before = (
        ui.ui().get_roll_ticks_per_pixel(),
        ui.ui().get_roll_scroll_x(),
        ui.ui().get_active_tool(),
        ui.ui().get_playing(),
    );

    // `Cmd/Ctrl + Alt + Shift + ↑`：`input.rs` 的 `resolve_logical` 第 3 步里
    // `NudgeMixerChannel` 的四条臂**都要求 `shift == false`**，注释明写
    // "「Cmd/Ctrl+Alt+Shift+方向」未被点名 ⇒ 放行" ⇒ 这一格是**表上可证**的未绑定组合。
    let unbound = KeyChord::new(
        ChordKey::UpArrow,
        KeyChordModifiers {
            shift: true,
            ctrl: true,
            alt: true,
            ..KeyChordModifiers::default()
        },
    );
    let consumed = ui
        .dispatch_key_chord(&unbound)
        .expect("`Interactive` 权限下注入被放行（被放行 ≠ 被消费）");
    assert!(
        !consumed,
        "未绑定的组合必须如实报 `consumed == false` —— 报 `true` 就是把「没接线的键」写成「生效的快捷键」"
    );

    let after = (
        ui.ui().get_roll_ticks_per_pixel(),
        ui.ui().get_roll_scroll_x(),
        ui.ui().get_active_tool(),
        ui.ui().get_playing(),
    );
    assert_eq!(
        before, after,
        "未消费的 chord 不该留下任何痕迹（缩放 / 滚动 / 工具 / 播放 四位读数）: {before:?} → {after:?}"
    );
    report_line(&format!(
        "[d-ui-3] 未绑定组合「{}」: consumed = false，四位读数一位未动",
        unbound.name()
    ));
}
