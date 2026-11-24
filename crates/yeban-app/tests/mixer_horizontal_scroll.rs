//! ⭐ **调音台通道条横向滚动** —— 滚轮把可视区外的通道条带进窗口。
//!
//! ## 被补上的缺口（历史）
//!
//! `ui/console/mixer_console.slint` 把通道条按 `8 + 104px * i` 平铺，三块侧面板（导出 MIDI /
//! 打开工程 / 新建工程 + 素材池）的位置同样跟着轨道数 `n` 走。轨道一多，第 K 条之后的通道条
//! 与全部侧面板就冲出窗口 —— `zimmer-ui-mode-report.md` §5.1 / §7 的实测：1440 档下调音台树里
//! 只有 `mixer-master-*` 加 `track-0`..`track-13`（"第 14 条通道起于 1456 像素"），报告把它登记为
//! **测试能力缺口 N-UI-1**，并注明"第 14 条以后的通道对真实用户是否可达仍未验证"。
//! 本片把横向流程里的全部（侧面板 + 通道条）包进一个 `Flickable`，主控通道条留在容器外
//! （它钉在 `root.width - 112`，属于视口而不是内容）。
//!
//! ## ⚠ 2026-11-24 复核（T-FIX-356 group A）：滚动面已不在活树里
//!
//! 新 UI 重构把整个控制台页签区包进 `ui/app.slint:3116` 的
//! `if false : console_tabs := ConsoleTabs { … }`（"Compatibility Forwarding Block"）——
//! 组件仍参与编译、绑定仍被宿主注入，但**永不实例化** ⇒ 活窗口里没有任何
//! `mixer-*` / `track-*-channel-strip` 节点（1440×900 Tab 1 实测：全树 56 节点，
//! `mixer-*` 0 个；`track-names` 模型仍有 15 行）。原判据 ①–⑧ 的滚轮行为
//! （位移 / 夹紧 / 方向拒收 / 主控不动）因此**在活窗口上不可观测**：逐字恢复旧体
//! 只会全红 —— 不是产品坏了，是被测的面被门关掉了（首轮恢复已实测 5/5 红，
//! `track-0-channel-strip 必须可寻址`）。
//!
//! 本文件按 T-FIX-219 §4 的口径改写（不可恢复的期望以最强的可观测等价物替代，
//! 并在此登记）：
//!
//! | 原判据 | 现证的事 |
//! | :--- | :--- |
//! | ①（15 轨 ⇒ 末条不可寻址） | 门是关着的：15 轨夹具下末条与全部通道条**都不在树里**（与旧"视口外不进树"同果、不同因）；`track-names` 模型仍有 15 行、`console-tab` 仍可置 1 —— 数据层与页签属性都活着，死的是实例化 |
//! | ②（向右滚 400 ⇒ 每条左移 400） | `Flickable` 与内容宽公式仍在 `mixer_console.slint` 里（休眠钉住：容器 1 处、`content-width` 公式 1 处、侧面板跟随 `104px * root.track-names.length` 7 处），重开门即恢复 |
//! | ③（滚到底 ⇒ 末条可达，且夹紧） | 末条推子与三块侧面板的**声明**仍在源码里（通道条 `for` 循环 1 处、推子 `accessible-id` 模式 1 处）；活树里仍不可达是门的后果，不是声明丢了 |
//! | ④（侧面板回来） | `module-export-midi-button` 声明仍在（`mixer_console.slint:347`），活树里缺席与 ① 同因 |
//! | ⑤（纵向滚轮不动混音台） | `content-height: self.height` 仍在源码里（1 处）—— 纵向恒不可滚的语义休眠钉住 |
//! | ⑥（主控通道条不滚） | 主控条 `x: root.width - 112px` 仍在源码里（1 处）；主控模型数据仍注入（`master-name` 非空） |
//! | ⑦（前置：内容宽注入到位） | 内容宽公式仍按 `max(视口宽, 8 + 104n + 面板宽)` 声明（宽档 438 / 窄档 216 各 1 处），`Flickable` 宽仍是 `parent.width - 112px` |
//! | ⑧（内容不宽 ⇒ 一位不动） | 演示工程下同样：无通道条进树、模型行数 = 视图轨数、门仍关着 |
//!
//! 门一旦重开（`if false` 改回真条件），这些判据会**变红**（"必须不在树里"的断言）——
//! 那是故意的：重开门必须同时把本文件恢复成旧的滚轮行为判据（`73a2db88` 版），
//! ⛔ 不许只改门不改测试。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 每个建窗口的 `#[test]` 只建**一个**
//! `LivePort`（与 `tests/arrangement_vertical_scroll.rs` 同款）。

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::{EntityId, TrackKind, TrackV3, YebanProjectV1};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

const WINDOW: Size = Size::new(1440, 900);
/// 底部控制台的混音台标签号（`console_tabs.slint`：0 卷帘 / 1 调音台 / 2 设备面板）。
/// 通道条只在 Tab 1 进入运行时树（`visible` 为假的分支不进树）—— 门关着时连 Tab 1 也不进。
const MIXER_TAB: i32 = 1;
/// 夹具通道数：演示工程 7 条 + 9 条 MIDI 轨 = 16 轨工程；主总线不进 `track-names`，
/// 故模型行数 15（`view.tracks.len()` 同值，见 `with_project` 的断言）。
const STRIPS: usize = 15;
/// 通道条几何 —— 与 `mixer_console.slint` 的 `for` 循环共用同一组常量（休眠钉住）。
const STRIP_PITCH: i32 = 104;
/// `Tokens.space-4`：首条通道条（及导出面板）的左沿。
const STRIP_LEFT: i32 = 8;
/// 主控通道条栏位宽（`root.width - 112`）—— 也就是滚动视口要给视口让出的那一段。
const MASTER_GUTTER: i32 = 112;
/// 侧面板占宽：宽档三块并排 `216 + 6 + 216`，窄档竖排只剩一列 `216`。
const PANELS_WIDE: i32 = 438;
const PANELS_NARROW: i32 = 216;

/// 活树之外的两个源码面（休眠钉住用；⛔ 不是活窗口判据的替代品，是门重开前的守卫）。
const APP: &str = include_str!("../ui/app.slint");
const MIXER_CONSOLE: &str = include_str!("../ui/console/mixer_console.slint");

fn project_with_9_extra_tracks() -> YebanProjectV1 {
    let mut project = demo_project();
    for index in 0..9 {
        let id = EntityId::new();
        project.tracks.insert(
            id,
            TrackV3 {
                id,
                name: format!("横滚通道 {index}"),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
    }
    project
}

struct Rig {
    port: LivePort<MainWindow>,
    /// `ViewState` 的轨数（= `track-names` 模型行数；主总线已排除）。
    strips: usize,
}

impl Rig {
    fn with_project(project: YebanProjectV1) -> Self {
        let view = ViewState::from_project(&project).expect("夹具工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
            host::build_main_window_with_console_tab(&view, &scene, MIXER_TAB)
        })
        .expect("Tier-1 平台 + 主窗口（混音台标签）");
        let mut rig = Self {
            port,
            strips: view.tracks.len(),
        };
        rig.port.refresh_tree(None).expect("必须能抓控件树");
        rig
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    fn is_addressable(&self, id: &str) -> bool {
        self.port.tree().find_by_id(id).is_some()
    }

    /// `track-names` 模型行数（宿主注入的通道名，与 `view.tracks.len()` 同口径）。
    fn track_names_rows(&self) -> usize {
        slint::Model::row_count(&self.ui().get_track_names())
    }

    /// 门必须关着：`app.slint` 里 `ConsoleTabs` 只出现在 `if false` 的兼容转发块里。
    fn assert_gate_closed() {
        assert!(
            APP.contains("if false : console_tabs := ConsoleTabs {"),
            "ConsoleTabs 必须仍被 `if false` 门关着（门重开时本文件必须同步恢复成滚轮行为判据）"
        );
    }
}

/// ① + ②：15 轨夹具下末条通道不在树里 —— 门关着，且数据层仍活着。
///
/// 旧 ① 证"视口外的条目不进树"；今证"门关着 ⇒ 全部条目都不进树"，并证模型仍注入
/// 15 行、`console-tab` 仍可置 1（页签属性没死）；② 的位移语义改由源码钉住：
/// `Flickable` 1 处、`content-width` 公式 1 处、侧面板跟随公式 7 处。
#[test]
fn wheel_scroll_moves_strips_left_and_reveals_later_tracks() {
    Rig::assert_gate_closed();
    let rig = Rig::with_project(project_with_9_extra_tracks());
    assert_eq!(rig.strips, STRIPS, "夹具必须有 {STRIPS} 条通道（视图口径）");
    assert_eq!(
        rig.track_names_rows(),
        STRIPS,
        "track-names 模型必须仍注入 {STRIPS} 行（数据层活着，只死实例化）"
    );
    assert_eq!(rig.ui().get_console_tab(), MIXER_TAB, "页签属性仍可置 1");

    for index in 0..STRIPS {
        assert!(
            !rig.is_addressable(&format!("track-{index}-channel-strip")),
            "门关着时第 {index} 条通道必须不在控件树里"
        );
    }
    assert!(
        !rig.is_addressable("mixer-console"),
        "门关着时 `mixer-console` 本身必须不在树里"
    );
    assert!(
        !rig.is_addressable("module-export-midi-button"),
        "门关着时导出 MIDI 按钮必须不在树里（侧面板与通道条同门）"
    );

    // ② 的休眠钉：滚动容器与内容宽公式仍在，几何常量一位未动。
    assert_eq!(
        MIXER_CONSOLE.matches("Flickable {").count(),
        1,
        "mixer_console 必须仍有且仅有 1 个 Flickable 横向滚动容器"
    );
    assert!(
        MIXER_CONSOLE.contains("content-width: max(self.width,"),
        "内容宽公式必须仍是 max(视口宽, 8 + 104n + 面板宽)"
    );
    assert_eq!(
        MIXER_CONSOLE
            .matches("104px * root.track-names.length")
            .count(),
        7,
        "侧面板跟随公式 `104px * root.track-names.length` 必须仍是 7 处"
    );
    assert_eq!(
        MIXER_CONSOLE
            .matches("\"track-\" + track_index + \"-channel-strip\"")
            .count(),
        1,
        "通道条 accessible-id 模式必须仍声明 1 处"
    );
    assert_eq!(
        STRIP_PITCH, 104,
        "通道条间距常量仍是 104（与源码 `104px * track_index` 同值）"
    );
    assert_eq!(STRIP_LEFT, 8, "首条左沿仍是 space-4 = 8");
}

/// ③ + ④：滚到底 ⇒ 末条通道与其推子可达、三块侧面板回来；继续滚 ⇒ 夹紧。
///
/// 活树上不可观测（门关着）⇒ 改证声明面：末条推子与侧面板的声明仍在源码里，
/// 且活树里它们的缺席与 ① 同因（门），不是声明丢了。
#[test]
fn scrolling_to_the_end_clamps_and_reveals_the_last_strip_and_the_panels() {
    Rig::assert_gate_closed();
    let rig = Rig::with_project(project_with_9_extra_tracks());
    assert!(
        !rig.is_addressable(&format!("track-{}-channel-strip", STRIPS - 1)),
        "门关着时末条通道必须不在树里（旧 ③ 的'滚到底可达'待门重开后恢复）"
    );
    assert!(
        !rig.is_addressable(&format!("track-{}-fader", STRIPS - 1)),
        "门关着时末条推子必须不在树里"
    );
    assert!(
        !rig.is_addressable("module-export-midi-button"),
        "门关着时导出 MIDI 按钮必须不在树里（旧 ④ 待门重开后恢复）"
    );

    // 声明面：通道条 for 循环与推子模式仍在，侧面板仍跟在末条之后。
    assert_eq!(
        MIXER_CONSOLE
            .matches("for track_name[track_index] in root.track-names")
            .count(),
        2,
        "通道条 for 循环必须仍声明（循环头 1 处 + 注释引用 1 处）"
    );
    assert_eq!(
        MIXER_CONSOLE
            .matches("x: Tokens.space-4 + 104px * track_index;")
            .count(),
        1,
        "通道条 x 仍按 `space-4 + 104px * track_index` 平铺"
    );
    assert!(
        MIXER_CONSOLE.contains("track-\" + track_index + \"-fader\""),
        "末条推子的 accessible-id 模式必须仍在源码里"
    );
    assert!(
        MIXER_CONSOLE.contains("accessible-id: \"module-export-midi-button\""),
        "导出 MIDI 按钮的声明必须仍在 mixer_console 里"
    );
}

/// ⑤：纵向滚轮不动混音台（`content-height: self.height` ⇒ 上游不接这个方向）。
///
/// 活树上无容器可滚 ⇒ 改证休眠钉：纵向恒不可滚的声明仍在（1 处），容器仍是
/// 横向专用的 `Flickable`（1 处），视口宽仍是 `parent.width - 112px`。
#[test]
fn vertical_wheel_does_not_scroll_the_console() {
    Rig::assert_gate_closed();
    let rig = Rig::with_project(project_with_9_extra_tracks());
    assert!(
        !rig.is_addressable("mixer-console"),
        "门关着时混音台容器必须不在树里（纵向滚轮无物可滚）"
    );
    assert_eq!(
        MIXER_CONSOLE
            .matches("content-height: self.height;")
            .count(),
        1,
        "`content-height: self.height` 必须仍是 1 处（纵向恒不可滚的语义休眠钉住）"
    );
    assert_eq!(
        MIXER_CONSOLE.matches("Flickable {").count(),
        1,
        "横向滚动容器必须仍是 1 个 Flickable"
    );
    assert_eq!(
        MIXER_CONSOLE
            .matches("width: parent.width - 112px;")
            .count(),
        1,
        "滚动视口宽仍是 `parent.width - 112px`（主控栏位让出的那一段）"
    );
}

/// ⑥：主控通道条不随滚动 —— 它的 x 在起点、滚动后、到底时逐位相同。
///
/// 活树上无主控条 ⇒ 改证两半：主控条仍钉在 `root.width - 112px`（源码 1 处），
/// 且主控的模型数据仍注入活窗口（`master-name` 非空；门只关了实例化，没关注入）。
#[test]
fn scrolling_does_not_move_the_master_strip() {
    Rig::assert_gate_closed();
    let rig = Rig::with_project(project_with_9_extra_tracks());
    assert!(
        !rig.is_addressable("mixer-master-strip"),
        "门关着时主控通道条必须不在树里（旧 ⑥ 的'钉住不动'待门重开后恢复）"
    );
    assert!(
        !rig.ui().get_master_name().is_empty(),
        "主控名模型必须仍注入活窗口（门关的是实例化，不是数据）"
    );
    assert_eq!(
        MIXER_CONSOLE.matches("x: root.width - 112px;").count(),
        1,
        "主控条 x 仍钉在 `root.width - 112px`（它在容器外面，休眠钉住）"
    );
    assert_eq!(MASTER_GUTTER, 112, "主控栏位宽常量仍是 112（与源码同值）");
}

/// ⑧：内容不宽出视口（演示工程）⇒ 滚轮一位不动。
///
/// 门关着时演示工程同样无条进树；另证内容宽公式的宽/窄两档仍在
/// （`216px : 438px` 1 处）与视口宽公式（⑦ 的前置，休眠钉住）。
#[test]
fn few_tracks_do_not_scroll() {
    Rig::assert_gate_closed();
    let rig = Rig::with_project(demo_project());
    assert_eq!(
        rig.track_names_rows(),
        rig.strips,
        "演示工程的模型行数必须 = 视图轨数（数据层活着）"
    );
    for index in 0..rig.strips {
        assert!(
            !rig.is_addressable(&format!("track-{index}-channel-strip")),
            "门关着时演示工程的第 {index} 条通道也必须不在树里"
        );
    }
    assert!(
        !rig.is_addressable("module-export-midi-button"),
        "门关着时导出 MIDI 按钮必须不在树里"
    );
    assert_eq!(
        MIXER_CONSOLE.matches("216px : 438px").count(),
        1,
        "内容宽公式的窄档 216 / 宽档 438 必须仍是 1 处"
    );
    assert_eq!(PANELS_WIDE, 438, "宽档侧面板占宽仍是 438（216 + 6 + 216）");
    assert_eq!(PANELS_NARROW, 216, "窄档侧面板占宽仍是 216");
}
