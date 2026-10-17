//! ⭐ 任务单 `#157` 的判据：**设备效果链是真实投影 ＋ 最小可编辑**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §3）
//!
//! | §3 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 假数据已删 | `ui/console/device_rack.slint` 的**源码**里没有旧设备名 / `band-gains` / `for device_index in 3`；行循环是 `for … in root.device-names` | 证"界面不再自带一份列表"。⛔ 不证历史（旧字面量在 `scene.rs` 的删除登记里点名，不在本文件里复读） |
//! | ② 投影是派生的 | 同一活窗口上把投影从 1 台设备换到 3 台 ⇒ 运行时 `device-*-card` 节点 1 → 3；`device-rack-count` 的逐字标签 `1 台设备` → `3 台设备` | 证"条目数随投影变"。⛔ 不证像素（见 ⑦） |
//! | ③ 空态可判 | 空工程 ⇒ `device-rack-empty` 的**逐字**文案 `无设备（点上方「装 EQ」或「装混响」）`、卡片 0 个、`0 台设备` | 证"没有设备时不是一片空白，也不是假条目" |
//! | ④ 旁通手势 = 恰好一次提交 | `invoke_device_bypass_toggle(0)` ⇒ `commit_count +1`、`undoable +1`、`TrackV3::devices[0].bypassed` 真的翻转、界面 `device-bypassed[0]` 跟着变 | 证"点击 → 一个 `Op::SetDeviceBypassed` → 一步撤销" |
//! | ⑤ EQ 滑块 = 恰好一次提交 | `invoke_device_eq_grab(1.0)` ＋ `invoke_device_eq_release()` ⇒ `commit_count +1`、`undoable +1`，**读回的模型值** = `18.0`（值域上界来自契约）；再在同位置按下 / 松手 ⇒ `+0`（没变就不提交） | 证"拖动一次 = 一步撤销，且改的值进了 `TrackV3::devices`" |
//! | ⑥ 预算三件套 | 不在本文件：读数在 `tests/perf_draw_budget.rs`（52 写点 / 38 数组，常量与原因同处） | — |
//! | ⑦ 无头可判 | Tier-1 窗口（`LivePort`）＋ 运行时控件树 ＋ 一张 Tab 3 截图（非黑 / 多色） | 证"真的有一个窗口在画它"；⛔ 不证墙钟成本（本机不许跑 GPU 档） |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把 `device-names` 的默认值改回三条字面量 ⇒ ① 红（源码探针）；
//! - 把 `host::apply_view` 里的四个 `set_device_*` 数组删掉 ⇒ ②③ 的运行时读数红；
//! - 把 `for … in root.device-names` 改回 `for device_index in 3` ⇒ ② 红（1 台设备的夹具
//!   会画出 3 张卡）且 ③ 红（空态不可见）；
//! - 摘掉 `host::wire_device_rack` 里的那次 `commit_ops` ⇒ ④⑤ 的 `commit_count` 差 0；
//! - 让 `device-eq-release` 在拖动期也提交（每次 move 一次 `commit_ops`）⇒ ⑤ 的 `+1` 变 `+k`。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"拖 EQ 旋钮就能听到声音"：本判据只证工程内容真的变了。实时侧的插入链是
//! `yeban-engine` 的事 —— 单轨插入会响、**aux 总线上的插入今天还不会**（工单 §0 第 5 条）。

use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ViewState, device_eq_low_gain_target, device_rack_track};
use yeban_app::elements::ElementRegistry;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::{DeviceDefinition, DeviceKind, ParameterValue, YebanProjectV1};
use yeban_model::samples::{default_project, demo_project, filled_project};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, golden_evidence, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸（与既有 Tier-1 判据同款；非零即合法）。
const WINDOW: Size = Size::new(1440, 900);

/// 设备机架源码（**另一个文件**，不是本判据自己）—— 第 293 条：判据钉住的字符串必须从
/// 别处读，⛔ 不许从判据所在文件读（那是恒真）。
const RACK_SOURCE: &str = include_str!("../ui/console/device_rack.slint");

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 内嵌 MCP 契约里的低架增益值域（**从契约现读**，⛔ 不手抄第二个 -18/18）。
fn contract_eq_range() -> (f32, f32) {
    let (_, spec) =
        yeban_app::effect_params::recognise("eq_low_gain").expect("契约里必须有 eq_low_gain");
    #[allow(clippy::cast_possible_truncation)]
    (spec.min as f32, spec.max as f32)
}

/// 一台设备：`params` 逐条给 `(名字, 值, 单位)`。
fn device(name: &str, bypassed: bool, params: &[(&str, f32, Option<&str>)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed,
        params: params
            .iter()
            .map(|(param, value, unit)| ParameterValue {
                name: (*param).to_owned(),
                value: *value,
                unit: unit.map(str::to_owned),
            })
            .collect(),
        latency_samples: 0,
    }
}

/// 一台真的带 `eq_low_gain` 的 EQ：两个已识别参数 ＋ 一个未识别参数。
fn eq_device() -> DeviceDefinition {
    device(
        "Yeban EQ",
        false,
        &[
            ("eq_low_gain", 0.0, Some("dB")),
            ("eq_low_freq", 200.0, Some("Hz")),
            ("mystery", 1.0, None),
        ],
    )
}

/// 把 `filled_project()` 的**展示轨**（身份序第一条非主总线轨）的设备链换成 `devices`。
///
/// 为什么直接改模型而不是提交 `Op::InsertDevice`：夹具是**判据的输入**，不是被测行为；
/// 本单被测的是"投影 ＋ 两个手势"，插入设备那条 `Op` 在 UI 侧今天**没有**构造点
/// （本单不造，见交付报告）。用完的工程只活在内存里，⛔ 不落盘。
fn project_with(devices: Vec<DeviceDefinition>) -> YebanProjectV1 {
    let mut project = filled_project();
    let master = project.master_bus_track_id;
    let track = project
        .tracks
        .values_mut()
        .find(|track| track.id != master)
        .expect("filled_project 必须有非主总线轨");
    track.devices = devices;
    project
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 ＋ JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 模型数组：设备名（`ModelRc<SharedString>`）。
fn model_names(window: &MainWindow) -> Vec<String> {
    window
        .get_device_names()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

/// 模型数组：旁通位。
fn model_bypassed(window: &MainWindow) -> Vec<bool> {
    window.get_device_bypassed().iter().collect()
}

/// `device-{i}-card` 的形态判定（⚠ `device-plugin-host-placeholder-card` 也以 `-card`
/// 结尾，所以不能只按后缀数：否则空链也会数出 1）。
fn is_device_card(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("device-") else {
        return false;
    };
    let Some(index) = rest.strip_suffix("-card") else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

/// 运行时树里 `device-{i}-card` 的条数。
fn card_count(tree: &ControlTree) -> usize {
    tree.iter().filter(|node| is_device_card(&node.id)).count()
}

/// 一条**生产形态**的 Tier-1 活窗口（`build_main_window_with_console_tab` 是唯一的注入实现）。
///
/// ⚠ `LivePort` 必须被持有到判据结束；同一测试里只能建一个（Tier-1 平台每线程一份）。
fn live_window(view: &ViewState, console_tab: i32) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(view, &scene, console_tab)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// 一条带撤销端口的 Tab 3 活窗口（接线走**生产同一个** `host::wire_device_rack`）。
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
    let live = live_window(&view, 2);
    host::wire_device_rack(live.ui(), &port);
    (live, port)
}

// ===========================================================================
// 判据 ①：假数据已删（**源码派生**，不需要窗口）
// ===========================================================================
#[test]
fn the_fake_rack_data_is_gone_and_the_chain_follows_the_projection() {
    // 工单 §3.1 的**逐字**判据：两个旧设备名在本文件里 0 命中。
    for forbidden in ["EQ 4 段", "Space Reverb"] {
        assert!(
            !RACK_SOURCE.contains(forbidden),
            "`device_rack.slint` 里仍有假设备名 `{forbidden}` —— 工单 §3.1 要求 0 命中"
        );
    }
    // `band-gains` 必须**不再是声明 / 不再被消费**（历史登记里点名它不算复活）。
    for forbidden in [
        "in property <[float]> band-gains",
        "root.band-gains",
        "device-names: [\"",
    ] {
        assert!(
            !RACK_SOURCE.contains(forbidden),
            "`device_rack.slint` 里仍有 `{forbidden}` —— 工单 §1.1 要求删掉死列表与\
             无写者的 `band-gains`（它们没有任何宿主写者，看起来有设备架、实际什么都没接）"
        );
    }
    // 循环条数必须由投影决定（旧版是写死的 `for device_index in 3`）。
    assert!(
        !RACK_SOURCE.contains("for device_index in 3"),
        "条目数仍被写死成 3 ⇒ 夹具换成 1 台设备时界面会画 3 张卡"
    );
    let loops = RACK_SOURCE
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("for ") && line.contains("in root.device-names"))
        .count();
    assert_eq!(
        loops, 1,
        "设备链必须**恰好**一条 `for … in root.device-names` 循环（现在 {loops} 条）"
    );
    // 值域必须来自注入（⛔ 界面上没有第二份范围表）。
    assert!(
        RACK_SOURCE.contains("accessible-value-minimum: root.eq-min-db")
            && RACK_SOURCE.contains("accessible-value-maximum: root.eq-max-db"),
        "滑块值域必须绑定注入的 `eq-min-db` / `eq-max-db`，⛔ 不许写字面量"
    );
    // 空态文案逐字固定（判据 ③ 在运行时再断一次）。
    assert!(
        RACK_SOURCE.contains("无设备（点上方「装 EQ」或「装混响」）"),
        "空态文案缺失"
    );

    report_line(&format!(
        "[#157 ①] device_rack.slint 源码探针：假名字/band-gains 命中 0；行循环 1 条；\
         值域绑定注入 = true；源码 {} 字节",
        RACK_SOURCE.len()
    ));
}

// ===========================================================================
// 判据 ②：投影是派生的（两个不同设备数的夹具 ⇒ 面板条目数随之变）
// ===========================================================================
#[test]
fn the_rack_follows_the_projected_device_chain() {
    // 夹具 A：`demo_project()` 的展示轨挂 1 台设备（`Yeban PolySynth`，参数 `cutoff`）。
    let view_a = ViewState::from_project(&demo_project()).expect("演示工程投影");
    assert_eq!(view_a.devices.len(), 1, "夹具 A 必须只有 1 台设备");
    assert_eq!(view_a.device_names(), vec!["Yeban PolySynth".to_owned()]);
    // `cutoff` 不在引擎白名单里 ⇒ 它必须是**未识别**，而"已识别"是空串（⛔ 不猜）。
    assert_eq!(view_a.devices[0].recognised, "");
    assert_eq!(view_a.devices[0].unrecognised, "未识别 1 · cutoff= 1200 Hz");

    // 夹具 B：同一功能位置挂 3 台设备（1 台 EQ ＋ 2 台 Compressor）。
    let fixtures = vec![
        eq_device(),
        device("Compressor", false, &[("threshold_db", -18.0, Some("dB"))]),
        device("Space Echo", true, &[("delay_ms", 320.0, Some("ms"))]),
    ];
    let project_b = project_with(fixtures);
    let view_b = ViewState::from_project(&project_b).expect("三设备工程投影");
    assert_eq!(view_b.devices.len(), 3, "夹具 B 必须有 3 台设备");
    // "已识别 / 未识别"按契约分档（同一台可以同时有两档）。
    assert_eq!(
        view_b.devices[0].recognised,
        "已识别 2 · EQ：eq_low_gain= 0 dB, eq_low_freq= 200 Hz"
    );
    assert_eq!(view_b.devices[0].unrecognised, "未识别 1 · mystery= 1");
    // `threshold_db` 是压缩器契约里的名字；`delay_ms` 不是 ⇒ 后者未识别。
    assert!(
        view_b.devices[1]
            .recognised
            .contains("threshold_db= -18 dB")
    );
    assert_eq!(view_b.devices[1].unrecognised, "");
    assert_eq!(
        view_b.devices[2].unrecognised,
        "未识别 1 · delay_ms= 320 ms"
    );
    // 旁通位来自模型，⛔ 不是界面状态。
    assert_eq!(view_b.device_bypassed(), vec![false, false, true]);
    assert_eq!(
        view_b.device_track_label,
        format!(
            "轨 {} · 设备链",
            view_b.tracks.first().expect("filled_project 有轨道").name
        ),
        "展示轨的标题必须来自投影（同一规则 `device_rack_track`）"
    );

    // 注册表同样派生：卡片数 = 投影设备数。
    let registry = ElementRegistry::from_view(&view_b);
    assert_eq!(
        registry
            .iter()
            .filter(|meta| is_device_card(&meta.id))
            .count(),
        3,
        "注册表的设备卡数必须等于投影设备数"
    );

    // 运行时：同一个活窗口先画夹具 A，再注入夹具 B。
    let port = live_window(&view_a, 2);
    let tree_a = runtime_tree(port.ui());
    let cards_a = card_count(&tree_a);
    let count_a = tree_a
        .find_by_id("device-rack-count")
        .expect("条目数节点必须在 Tab 3 里")
        .label
        .clone();
    assert_eq!(cards_a, 1, "夹具 A 必须画出 1 张设备卡");
    assert_eq!(count_a, "1 台设备");
    assert_eq!(model_names(port.ui()), view_a.device_names());
    assert!(tree_a.contains("device-rack-track-label"));
    assert!(
        !tree_a.contains("device-rack-empty"),
        "有设备时不得显示空态"
    );

    // 换投影（同一个窗口、同一个 Tab）——条目数必须跟着变。
    host::apply_view(port.ui(), &view_b, WINDOW.width as f32, 0.0);
    let tree_b = runtime_tree(port.ui());
    let cards_b = card_count(&tree_b);
    let count_b = tree_b
        .find_by_id("device-rack-count")
        .expect("条目数节点必须在")
        .label
        .clone();
    assert_eq!(cards_b, 3, "夹具 B 必须画出 3 张设备卡");
    assert_eq!(count_b, "3 台设备");
    assert_eq!(model_names(port.ui()), view_b.device_names());
    assert_eq!(
        model_bypassed(port.ui()),
        vec![false, false, true],
        "旁通位必须来自 `TrackV3::devices[i].bypassed`"
    );
    // 已识别 / 未识别读数进了控件树（不是只活在 Rust 里）。
    let recognised = tree_b
        .find_by_id("device-0-recognised")
        .expect("EQ 的已识别参数行必须在树里");
    assert_eq!(recognised.label, view_b.devices[0].recognised);
    let unrecognised = tree_b
        .find_by_id("device-0-unrecognised")
        .expect("未识别参数行必须在树里");
    assert_eq!(unrecognised.label, "未识别 1 · mystery= 1");

    report_line(&format!(
        "[#157 ②] 夹具 A: 卡片 {cards_a} / {count_a:?}；夹具 B: 卡片 {cards_b} / {count_b:?}；\
         设备名 = {:?}；设备 0 已识别 = {:?}、未识别 = {:?}",
        model_names(port.ui()),
        recognised.label,
        unrecognised.label
    ));
}

// ===========================================================================
// 判据 ③：空态可判（无设备 ⇒ 逐字文案 ＋ 条目数 0）
// ===========================================================================
#[test]
fn an_empty_chain_shows_the_empty_state_and_zero_rows() {
    let project = default_project();
    let view = ViewState::from_project(&project).expect("空工程必须能投影");
    assert!(
        view.devices.is_empty(),
        "空工程必须投影成空设备链（⛔ 不是三条演示设备）"
    );
    assert_eq!(view.device_track_label, "没有可显示的音轨");

    let port = live_window(&view, 2);
    assert!(model_names(port.ui()).is_empty(), "`device-names` 必须为空");
    assert!(model_bypassed(port.ui()).is_empty());
    assert!(!port.ui().get_device_eq_available());

    let tree = runtime_tree(port.ui());
    assert!(tree.contains("device-rack"), "空链时面板本体必须在");
    let empty = tree
        .find_by_id("device-rack-empty")
        .expect("空链必须显示 `device-rack-empty`（⛔ 不许一片空白）");
    assert_eq!(
        empty.label, "无设备（点上方「装 EQ」或「装混响」）",
        "空态文案必须逐字一致"
    );
    let count = tree
        .find_by_id("device-rack-count")
        .expect("条目数节点必须在");
    assert_eq!(count.label, "0 台设备");
    assert_eq!(card_count(&tree), 0, "空链不得有设备卡节点");
    let eq_value = tree
        .find_by_id("device-eq-value")
        .expect("不可编辑时也必须有一格如实文案");
    assert_eq!(eq_value.label, "本轨没有可编辑的 eq_low_gain");

    report_line(&format!(
        "[#157 ③] 空链：devices=0 / 文案={:?} / 条目数={:?} / 卡片=0 / EQ={:?}",
        empty.label, count.label, eq_value.label
    ));
}

// ===========================================================================
// 判据 ④：旁通手势 = 恰好一次提交
// ===========================================================================
#[test]
fn the_bypass_gesture_commits_exactly_once() {
    let project = project_with(vec![eq_device(), device("Compressor", false, &[])]);
    let (live, port) = live_window_with_undo(&project, "#157 旁通");
    let before = port.display();

    // 初始读数：两台都未旁通。
    assert_eq!(model_bypassed(live.ui()), vec![false, false]);

    live.ui().invoke_device_bypass_toggle(0);
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(
        after.undoable,
        before.undoable + 1,
        "一次点击必须恰好留下一步可撤销的编辑"
    );

    // 值真的进了 `TrackV3::devices`（读回权威工程，⛔ 不信界面）。
    let committed = port.project();
    let track = device_rack_track(&committed).expect("展示轨");
    assert!(
        track.devices[0].bypassed,
        "`old_bypassed=false → new_bypassed=true` 必须落盘"
    );
    assert!(!track.devices[1].bypassed, "⛔ 不得顺手改第二台");
    // 界面跟着重投影（`refresh_undo_window` 的唯一出口）。
    assert_eq!(model_bypassed(live.ui()), vec![true, false]);
    let tree = runtime_tree(live.ui());
    assert_eq!(
        tree.find_by_id("device-0-card").expect("设备 0 卡片").label,
        "设备 Yeban EQ（已旁通）"
    );

    report_line(&format!(
        "[#157 ④] 旁通：commit_count {} → {}（Δ1）；undoable {} → {}；\
         devices[0].bypassed={} / devices[1].bypassed={}；界面旁通位={:?}",
        before.commit_count,
        after.commit_count,
        before.undoable,
        after.undoable,
        track.devices[0].bypassed,
        track.devices[1].bypassed,
        model_bypassed(live.ui())
    ));
}

// ===========================================================================
// 判据 ⑤：EQ 滑块 = 恰好一次提交，且值真的进了 `TrackV3::devices`
// ===========================================================================
#[test]
fn the_eq_slider_commits_exactly_once_and_writes_the_model() {
    let project = project_with(vec![eq_device()]);
    let (live, port) = live_window_with_undo(&project, "#157 EQ");
    let (min_db, max_db) = contract_eq_range();
    assert_eq!(
        (min_db, max_db),
        (-18.0, 18.0),
        "契约里的低架增益值域必须是 ±18 dB"
    );

    let view = ViewState::from_project(&project).expect("投影");
    assert!(
        view.device_eq_available(),
        "夹具必须有一格可编辑的 eq_low_gain"
    );
    assert_eq!(
        (view.device_eq_min_db(), view.device_eq_max_db()),
        (min_db, max_db)
    );
    assert_eq!(view.device_eq_gain_db(), 0.0);

    // ---- 一次拖动（按下在最右端 ⇒ +18 dB，松手提交一次）----
    let before = port.display();
    live.ui().invoke_device_eq_grab(1.0);
    live.ui().invoke_device_eq_release();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次拖动必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);
    let (_, _slot, _param, value, lo, hi) =
        device_eq_low_gain_target(&port.project()).expect("提交之后目标仍在");
    assert_eq!((lo, hi), (min_db, max_db), "值域来自契约");
    assert_eq!(
        value.to_bits(),
        max_db.to_bits(),
        "拖到最右端必须写进值域上界（读回 {value}）"
    );
    assert!((live.ui().get_device_eq_gain_db() - value).abs() < f32::EPSILON);

    // ---- 没变的第二次手势：按下再松手 ⇒ **不提交**（⛔ 不留一步什么都没改的撤销）----
    let quiet = port.display();
    live.ui().invoke_device_eq_grab(1.0);
    live.ui().invoke_device_eq_release();
    let still = port.display();
    assert_eq!(
        still.commit_count, quiet.commit_count,
        "同一个位置按下再松手不得产生提交"
    );
    assert_eq!(still.undoable, quiet.undoable);

    // ---- 单击中点 ⇒ 0 dB，仍是一次提交 ----
    live.ui().invoke_device_eq_grab(0.5);
    live.ui().invoke_device_eq_release();
    let clicked = port.display();
    assert_eq!(clicked.commit_count, still.commit_count + 1);
    let (_, _, _, mid, _, _) = device_eq_low_gain_target(&port.project()).expect("目标仍在");
    assert_eq!(
        mid.to_bits(),
        0.0_f32.to_bits(),
        "中点必须写 0 dB（读回 {mid}）"
    );

    report_line(&format!(
        "[#157 ⑤] EQ：拖动 commit_count {} → {}（Δ1），读回 value={value}（上界 {max_db}）；\
         同位置再按一次 Δ0（{}）；单击中点读回 {mid} dB（Δ1）",
        before.commit_count, after.commit_count, still.commit_count
    ));
}

// ===========================================================================
// 判据 ⑦：无头可判 —— Tab 3 真有一张非黑、多色的截图
// ===========================================================================
#[test]
fn the_device_tab_renders_a_non_black_frame() {
    let project = project_with(vec![
        eq_device(),
        device("Compressor", false, &[("threshold_db", -18.0, Some("dB"))]),
    ]);
    let view = ViewState::from_project(&project).expect("投影");
    let port = live_window(&view, 2);
    let image = port.window().capture().expect("Tier-1 截图");
    let evidence = golden_evidence(&image).expect("[MUST-GATE-015] 尺寸非零且非全黑");
    assert_eq!(evidence.size, WINDOW, "截图尺寸必须等于窗口尺寸");
    assert!(evidence.non_black_pixels > 0, "全黑截图: {evidence:?}");
    assert!(
        evidence.distinct_colors >= 8,
        "颜色过少, 疑似只画了背景: {evidence:?}"
    );

    report_line(&format!("[#157 ⑦] Tab 3 截图：{}", evidence.summary()));
}
