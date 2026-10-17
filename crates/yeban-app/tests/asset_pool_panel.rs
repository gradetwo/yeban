//! ⭐ 任务单 `#133` 的判据：**素材池在界面里可见**（`clip_pool` → 控制台 Tab 1 的面板）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §4）
//!
//! | §4 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 面板存在且可无障碍访问 | `.slint` 源里的 `accessible-id: "asset-pool-panel"` 可由**源码派生**的计数（1 条行循环 / 4 个 `accessible-id:`）对上；注册表与运行时控件树两侧都有它 | 证"面板是一个可寻址的语义区域"。不证 Linux 像素（本机 macOS，无法重录基准） |
//! | ② 空态可判 | 0 素材的工程（`new_project_archive()`）⇒ `asset-pool-empty` 的**逐字**标签 `尚无素材`、条目数 `素材 0 条`、行数 0 | 证"空池有明确文案，不是空白" |
//! | ③ 录音 clip 可见 | `plan_recording` ＋ **一次** `Op::AddClip` 造出的那条 clip ⇒ 面板出现一行（逐字标签 / `素材 1 条` / `asset` 前缀） | 证"`#132` 落进来的音频片段真的被投影到界面"。不证 L5 的摆放／发声 |
//! | ④ 派生不是硬编码 | 两个不同 WAV ⇒ `clip_pool` 2 条 ⇒ 面板 2 行；同一窗口改注入 1 条的投影 ⇒ 1 行（`row-1` 消失） | 证"行数随投影变" |
//!
//! ## 无头可判（工单 §4 第 7 条）
//!
//! 窗口用 Tier-1 无头平台（`yeban_ui_test_port::render::LivePort`），WAV 用**既有**的
//! `record_wav::record_frames` 真落盘（⛔ 手写假字节），产物落 `std::env::temp_dir()`。
//! 纯投影的那一半（`ViewState::asset_pool`）连窗口都不需要。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 删掉 `AssetPoolPanel` 的 `accessible-id` ⇒ ① 的源码派生计数与运行时树两侧都红；
//! - 删掉 `host::apply_view` 里的三个 `set_asset_pool_*` ⇒ ②③④ 的界面读数全红
//!   （模型数组空 ⇒ `asset-pool-names.length` 0 ⇒ 面板显示空态）；
//! - 把 `asset_pool_rows` 改成只认 MIDI / 只认音频 ⇒ ③④ 红（录音那条不在/多出一条）；
//! - 把行循环改成写死条数 ⇒ ④ 红（两个夹具的行数不再跟着 `clip_pool` 变）。

use std::path::{Path, PathBuf};

use slint::ComponentHandle as _;
use yeban_app::bridge::ViewState;
use yeban_app::elements::ElementRegistry;
use yeban_app::host;
use yeban_app::record_landing::plan_recording;
use yeban_app::record_wav::{RecordOptions, record_frames};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::samples::{default_project, filled_project};
use yeban_model::{Op, YebanProjectV1};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸（与既有 Tier-1 判据同款；非零即合法）。
const WINDOW: Size = Size::new(1440, 900);

/// 面板源码（**另一个文件**，不是本判据自己）—— 第 293 条：判据钉住的字符串必须从
/// 别处读，或用"锚点 ＋ 计算窗口"的派生形，⛔ 不许从判据所在文件读（那是恒真）。
const PANEL_SOURCE: &str = include_str!("../ui/console/asset_pool_panel.slint");

/// 一个临时目录，`Drop` 时清理（**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-asset-pool-{tag}-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 建环参数（与 `record_lands_in_pools.rs` 同一条；⛔ 不为本文件另发明一套）。
fn record_options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: 48_000,
        channels: 2,
        capacity_frames: 8192,
        drain_batch: 512,
        ..RecordOptions::default()
    }
}

/// 用**既有**的落盘器造一段真 WAV（`seed` 不同 ⇒ 字节不同 ⇒ CAS 哈希不同）。
fn write_wav(path: &Path, seed: u64) {
    record_frames(path, &record_options(), |frame| {
        (((frame + seed) % 64) as f32) / 128.0
    })
    .expect("落一段真 WAV");
}

/// 走 `#132` 的落盘路径：纯函数 `plan_recording` ＋ **一次** `Op::AddClip`（每条 WAV 一次）。
fn project_with_recordings(
    paths: &[PathBuf],
) -> (
    YebanProjectV1,
    Vec<yeban_app::record_landing::RecordingPlan>,
) {
    let mut project = default_project();
    let mut plans = Vec::new();
    for path in paths {
        let plan = plan_recording(path, &project).expect("纯规划（不碰会话）");
        assert!(
            plan.creates_clip(),
            "空 clip_pool 里每条新 WAV 都必须新建一条 clip"
        );
        for op in &plan.ops {
            assert!(
                matches!(op, Op::AddClip { .. }),
                "录音落点只允许 AddClip（L5 的摆放不归本单）"
            );
            op.apply(&mut project)
                .expect("AddClip 必须能作用在空工程上");
        }
        plans.push(plan);
    }
    (project, plans)
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 模型里的数组（`asset-pool-*`）逐项读出 —— 与 `.slint` 消费的是同一份 `ModelRc`。
fn model_rows(window: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    window
        .get_asset_pool_names()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

fn model_kinds(window: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    window
        .get_asset_pool_kinds()
        .iter()
        .map(|kind| kind.to_string())
        .collect()
}

fn model_details(window: &MainWindow) -> Vec<String> {
    use slint::Model as _;
    window
        .get_asset_pool_details()
        .iter()
        .map(|detail| detail.to_string())
        .collect()
}

/// 一条**生产形态**的 Tier-1 活窗口（`build_main_window_with_console_tab` 是唯一的注入实现）。
///
/// ⚠ 返回值里的 `LivePort` **必须被持有到判据结束**：它拥有 `set_platform` 装好的
/// 每线程一份的 Slint 平台与活窗口。同一测试里**只能建一个**（第二次 `install` 会
/// 返回 `PlatformUnavailable`）。
fn live_window(view: &ViewState, console_tab: i32) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::ReadOnly, None, || {
        host::build_main_window_with_console_tab(view, &scene, console_tab)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

// ===========================================================================
// 判据 ①：面板存在且可无障碍访问（源码派生 ＋ 注册表 ＋ 运行时树）
// ===========================================================================
#[test]
fn the_asset_pool_panel_is_declared_and_addressable() {
    // ---- 源码派生（⛔ 不钉手抄条数）：行循环 1 条、`accessible-id:` 4 条 ----
    let row_loops = PANEL_SOURCE
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("for ") && line.contains("in root.asset-pool-names"))
        .count();
    assert_eq!(
        row_loops, 1,
        "素材池面板必须**恰好**一条行循环（`for … in root.asset-pool-names`）；\
         现在 {row_loops} 条 ⇒ `asset-pool-row-{{row_index}}-row` 的索引含义变了"
    );
    let id_declarations = PANEL_SOURCE
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("accessible-id:"))
        .count();
    assert_eq!(
        id_declarations, 4,
        "素材池面板的 `accessible-id:` 必须恰好 4 条（panel / count / empty / row）；\
         现在 {id_declarations} 条 ⇒ 有节点没登记或被删了"
    );

    // ---- 注册表：条目数由**投影**决定（filled_project 的 clip_pool 有 2 条） ----
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let registry = ElementRegistry::from_view(&view);
    for id in ["asset-pool-panel", "asset-pool-count", "asset-pool-empty"] {
        assert!(registry.contains(id), "注册表缺少 `{id}`");
    }
    assert_eq!(
        registry.with_prefix("asset-pool-row-").count(),
        view.asset_pool.len(),
        "注册表的行数必须等于投影的行数（派生，⛔ 不是写死的）"
    );

    // ---- 运行时树：Tab 0（默认帧）里没有它；Tab 1 里有它且可寻址 ----
    let port = live_window(&view, 0);
    let tab0 = runtime_tree(port.ui());
    report_line(&format!(
        "[#133 ①] Tab 0 运行时树 {} 条；含 asset-pool-panel = {}",
        tab0.len(),
        tab0.contains("asset-pool-panel")
    ));
    assert!(
        !tab0.contains("asset-pool-panel"),
        "⛔ 默认帧（Tab 0 = 卷帘）里不得出现素材池面板 —— 它是 `[UI-MCP-003]` 基准\
         覆盖的那一帧；面板必须住在 Tab 1（本机无法重录 Linux 基准）"
    );

    port.ui().set_console_tab(1);
    let tab1 = runtime_tree(port.ui());
    let panel = tab1.find_by_id("asset-pool-panel").unwrap_or_else(|| {
        panic!(
            "切到 Tab 1 之后 `asset-pool-panel` 必须在树里（树里 {} 条）",
            tab1.len()
        )
    });
    assert_eq!(panel.role.as_str(), "region", "面板角色必须是 region");
    assert_eq!(panel.label, "素材池", "面板标签必须是 `素材池`");
    let count = tab1
        .find_by_id("asset-pool-count")
        .expect("`asset-pool-count` 必须在树里");
    assert_eq!(count.role.as_str(), "text");
    assert_eq!(
        count.label,
        format!("素材 {} 条", view.asset_pool.len()),
        "条目数必须来自投影（filled_project 的 clip_pool 条数）"
    );
    // 音频那一行携带**工程里登记的字节数**（filled_project 的 assets[hash].byte_len = 44100）。
    let audio = view
        .asset_pool
        .iter()
        .find(|row| row.kind == "音频")
        .expect("filled_project 必须有一条音频素材");
    assert!(
        audio.detail.contains("44100 字节"),
        "音频行的字节数必须来自 `project.assets[asset].byte_len`；实际 {}",
        audio.detail
    );
    let rows: Vec<String> = tab1
        .with_prefix("asset-pool-row-")
        .map(|node| node.id.clone())
        .collect();
    report_line(&format!(
        "[#133 ①] Tab 1 运行时树 {} 条；面板节点 id={} label={:?}；行节点 {rows:?}",
        tab1.len(),
        panel.id,
        panel.label
    ));
    assert_eq!(
        rows.len(),
        view.asset_pool.len(),
        "运行时行数必须等于投影行数"
    );
}

// ===========================================================================
// 判据 ②：空态可判（0 素材 ⇒ 逐字文案 ＋ 条目数 0）
// ===========================================================================
#[test]
fn an_empty_pool_shows_the_empty_state_and_zero_rows() {
    // 新工程的**规范来源**是 `#129` 的 `new_project_archive()`
    // （`#161` 起：0 条音轨 ＋ 1 条主总线 / 0 片段 / 0 资产）。
    let project = host::new_project_archive().project;
    let view = ViewState::from_project(&project).expect("空工程必须能投影");
    assert_eq!(
        project.clip_pool.len(),
        0,
        "夹具必须是 0 素材（否则这条判据测的不是空态）"
    );
    assert_eq!(view.asset_pool.len(), 0, "空池必须投影成空 Vec");

    let port = live_window(&view, 1);
    assert!(
        model_rows(port.ui()).is_empty(),
        "界面上的 `asset-pool-names` 必须为空"
    );
    assert!(model_kinds(port.ui()).is_empty());
    assert!(model_details(port.ui()).is_empty());

    let tree = runtime_tree(port.ui());
    assert!(tree.contains("asset-pool-panel"), "空池时面板本体必须在");
    let empty = tree
        .find_by_id("asset-pool-empty")
        .expect("空池必须显示 `asset-pool-empty`（⛔ 不许一片空白）");
    assert_eq!(empty.label, "尚无素材", "空态文案必须逐字是 `尚无素材`");
    let count = tree
        .find_by_id("asset-pool-count")
        .expect("条目数节点必须在");
    assert_eq!(count.label, "素材 0 条", "空池的条目数必须是 `素材 0 条`");
    assert_eq!(
        tree.with_prefix("asset-pool-row-").count(),
        0,
        "空池不得有行节点"
    );
    report_line(&format!(
        "[#133 ②] 空池：clip_pool=0 / view.asset_pool=0 / 面板文案={:?} / 条目数={:?} / 行节点=0",
        empty.label, count.label
    ));
}

// ===========================================================================
// 判据 ③：录音 clip 可见（plan_recording ＋ 一次 AddClip ⇒ 面板里出现那一行）
// ===========================================================================
#[test]
fn a_recorded_clip_appears_in_the_panel_with_its_asset_prefix() {
    let scratch = Scratch::new("one");
    let wav = scratch.join("take.wav");
    write_wav(&wav, 7);

    let (project, plans) = project_with_recordings(std::slice::from_ref(&wav));
    let plan = &plans[0];
    assert_eq!(
        project.clip_pool.len(),
        1,
        "一次 AddClip ⇒ clip_pool 恰好 1 条"
    );
    let view = ViewState::from_project(&project).expect("投影");
    assert_eq!(view.asset_pool.len(), 1, "素材池必须多出 1 行");

    let row = &view.asset_pool[0];
    let asset_prefix: String = plan.asset.to_string().chars().take(8).collect();
    assert_eq!(row.name, plan.clip.name, "行名必须来自 ClipPoolEntry::name");
    assert_eq!(row.kind, "音频", "录音片段必须是音频类型");
    assert!(
        row.detail.contains(&asset_prefix),
        "行里必须出现 asset 哈希前缀 `{asset_prefix}`；实际 {}",
        row.detail
    );
    assert!(
        row.detail.contains("0.0 dB"),
        "录音落点的单位增益必须如实显示；实际 {}",
        row.detail
    );
    // ⭐ 如实登记：`#132` 只提交 `Op::AddClip`，**不**写 `project.assets` 索引
    // ⇒ 字节数未知（⛔ 不按 WAV 头推算）。
    assert!(
        row.detail.contains("字节数未知"),
        "未登记资产元数据时必须写「字节数未知」而不是编一个数；实际 {}",
        row.detail
    );
    assert_eq!(
        row.label,
        format!("{} · 音频 · {}", plan.clip.name, row.detail),
        "整行标签必须由名字 / 类型 / 字段拼成"
    );

    let port = live_window(&view, 1);
    assert_eq!(
        model_rows(port.ui()),
        vec![row.name.clone()],
        "界面行名必须等于投影"
    );
    assert_eq!(model_kinds(port.ui()), vec!["音频".to_owned()]);
    assert_eq!(model_details(port.ui()), vec![row.detail.clone()]);
    let tree = runtime_tree(port.ui());
    let count = tree.find_by_id("asset-pool-count").expect("条目数节点");
    assert_eq!(count.label, "素材 1 条", "录音落进来之后条目数必须是 1");
    let node = tree
        .find_by_id("asset-pool-row-0-row")
        .expect("录音 clip 必须在面板里有一行");
    assert_eq!(node.role.as_str(), "list-item");
    assert_eq!(node.label, row.label, "运行时行标签必须与投影逐字相同");
    report_line(&format!(
        "[#133 ③] 录音行：clip={} name={:?} asset={} 前缀={} 逐字标签={:?} 条目数={:?}",
        plan.clip.id.to_canonical_string(),
        row.name,
        plan.asset,
        asset_prefix,
        node.label,
        count.label
    ));
}

// ===========================================================================
// 判据 ④：投影是派生不是硬编码（两个不同素材 ⇒ 2 行；改注入 1 条 ⇒ 1 行）
// ===========================================================================
#[test]
fn the_row_count_follows_the_clip_pool_instead_of_a_hardcoded_number() {
    let scratch = Scratch::new("two");
    let first = scratch.join("take-a.wav");
    let second = scratch.join("take-b.wav");
    write_wav(&first, 3);
    write_wav(&second, 41);

    let (two_clips, plans) = project_with_recordings(&[first.clone(), second.clone()]);
    assert_eq!(
        two_clips.clip_pool.len(),
        2,
        "两条不同 WAV ⇒ clip_pool 2 条"
    );
    assert_ne!(
        plans[0].asset, plans[1].asset,
        "两条 WAV 的内容哈希必须不同"
    );
    let view_two = ViewState::from_project(&two_clips).expect("投影");
    assert_eq!(
        view_two.asset_pool.len(),
        2,
        "两档夹具：clip_pool 2 ⇒ 面板 2 行"
    );

    // 同一窗口先注入 2 条的投影，再注入 1 条的投影（走的是**唯一**注入实现 `apply_view`）。
    let port = live_window(&view_two, 1);
    assert_eq!(model_rows(port.ui()).len(), 2);
    let tree_two = runtime_tree(port.ui());
    assert_eq!(tree_two.with_prefix("asset-pool-row-").count(), 2);
    assert!(
        tree_two.contains("asset-pool-row-1-row"),
        "两行时 `asset-pool-row-1-row` 必须在"
    );

    let (one_clip, _) = project_with_recordings(std::slice::from_ref(&first));
    assert_eq!(one_clip.clip_pool.len(), 1);
    let view_one = ViewState::from_project(&one_clip).expect("投影");
    assert_eq!(
        view_one.asset_pool.len(),
        1,
        "一档夹具：clip_pool 1 ⇒ 面板 1 行"
    );
    let width = port.ui().window().size().width as f32;
    host::apply_view(port.ui(), &view_one, width, 0.0);

    let rows_one = model_rows(port.ui());
    let tree_one = runtime_tree(port.ui());
    let count_one = tree_one.find_by_id("asset-pool-count").expect("条目数节点");
    assert_eq!(rows_one.len(), 1, "改注入 1 条之后界面必须只剩 1 行");
    assert_eq!(tree_one.with_prefix("asset-pool-row-").count(), 1);
    assert!(
        !tree_one.contains("asset-pool-row-1-row"),
        "1 行时 `asset-pool-row-1-row` 必须消失（行数不是写死的）"
    );
    assert_eq!(count_one.label, "素材 1 条");
    report_line(&format!(
        "[#133 ④] 两档读数：clip_pool 2 ⇒ 面板 {} 行；clip_pool 1 ⇒ 面板 {} 行（条目数 {:?}）",
        tree_two.with_prefix("asset-pool-row-").count(),
        tree_one.with_prefix("asset-pool-row-").count(),
        count_one.label
    ));
}
