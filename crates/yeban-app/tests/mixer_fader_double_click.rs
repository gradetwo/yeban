//! **双击复位**（推子 → `0 dB`、声相 → 中央）—— **文本层**判据。
//!
//! ## 本文件要证的句子
//!
//! 调音台的四块命中面（通道条推子 / 通道条声相 / 主控推子 / 主控声相）上那个
//! `double-clicked` **不是一个画了但不动的手势**（[`ADR-0052`] 那一类）：
//! 每一面各自绑**自己**的复位回调，回调一路转发到 `MainWindow`，宿主四条臂都汇到
//! **同一个**落点 [`host::commit_mixer_reset`]，而且**先**清掉在手的拖拽手势**再**提交。
//!
//! 仓内已有同族先例：底部控制台 splitter 的双击回默认档位（`ui/app.slint` 的
//! `double-clicked => { … root.console-height-reset(); }`，宿主 `host.rs` 的
//! `ui.on_console_height_reset`）。本票把同一条手势给混音台的推子 / 声相。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① | `mixer_console.slint` 里**恰好**四块命中面带 `double-clicked`；每一块**往上**读得到 `TouchArea {` 与它**自己**的 `x/y/width/height`（几何一位没动），**往下**读得到**它自己**那条回调（⛔ 串面：把声相臂贴到推子矩形上 ⇒ 红） |
//! | ② | 四个回调在 `mixer_console` / `console_tabs` / `app` **三个**文件里各**声明一次**，并在 `console_tabs` / `app` 各**转发一次**（少一段 ⇒ 手势落在没人听的回调上；多一段 ⇒ 有第二条路径绕过宿主） |
//! | ③ | 宿主四条 `on_*_reset` 臂各**恰好一次**，每臂里 `end_mixer_gesture` 出现在 `commit_mixer_reset` **之前**，且写的是**自己**那一格（`MixerResetField::Volume` / `::Pan`） |
//! | ④ | 中性值**只有一处**定义：`commit_mixer_reset` 定义一次、被调用四次；`new_val: 0.0` 在整个 `host.rs` 里**恰好一次** ⇒ 界面上不存在第二份"默认值"常量与它分叉 |
//!
//! ## ⛔ 本文件不证的东西（正对照在哪）
//!
//! 文本层证的是"接线存在且唯一"，**不证**"双击真的改了工程"。后者由
//! `tests/live_ui_mcp.rs` 的两条活体判据负责（`a_double_click_reset_moves_the_fader_and_pan_to_neutral`
//! / `a_double_click_reset_leaves_no_sticky_gesture`）：它们驱动**活的** `MainWindow`
//! 从权威工程读回数值。两半不可互相代替 —— 只看文本会在"回调声明了但宿主没接"这种
//! 状态下全绿，只看活体则钉不住"中性值只有一处定义"。
//!
//! ## 变异表（每条都实测过能变红）
//!
//! | 注入 | 文件 | 预期 |
//! | :--- | :--- | :--- |
//! | 把声相命中面的 `root.mixer-pan-reset(track_index);` 换成 `root.mixer-fader-reset(track_index);` | `ui/console/mixer_console.slint` | ① 红（第 2 面的调用不符） |
//! | 删掉 `console_tabs.slint` 里 `mixer-master-pan-reset()` 的转发臂 | `ui/console/console_tabs.slint` | ② 红（`root.mixer-master-pan-reset();` 计数 2 → 1） |
//! | 删掉某一臂里的 `end_mixer_gesture(&ui, &gesture);` | `src/host.rs` | ③ 红（找不到清零点，`commit` 之前无 `end`） |
//! | 把某一臂的 `MixerResetField::Pan` 改成 `::Volume` | `src/host.rs` | ③ 红（写错格）＋活体判据 ② 红（声相没动） |
//! | 在 `commit_mixer_reset` 之外再写一处 `new_val: 0.0` | `src/host.rs` | ④ 红（计数 1 → 2） |

/// 返回 `source` 里含 `needle` 的**行号**（1 基，按文档顺序）。
///
/// 数行而不是数子串：命中面与回调的**配对**关系要看"哪一块矩形上面写着什么"，
/// 全文 `matches()` 会把四块矩形混成一堆无上下文的计数。
fn line_numbers(source: &str, needle: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(nth, _)| nth + 1)
        .collect()
}

/// 命中面 `(x, y, width, height)` 四行 ＋ 它应当调用的那条回调。
type HitRect<'a> = ([&'a str; 4], &'a str);

/// 文档顺序：通道条推子 → 通道条声相 → 主控推子 → 主控声相
/// （与 `mixer_console.slint` 里两个组件的排布同序）。
const TRACK_AND_MASTER_HIT_RECTS: [HitRect<'_>; 4] = [
    (
        ["x: 22px;", "y: 54px;", "width: 48px;", "height: 116px;"],
        "root.mixer-fader-reset(track_index);",
    ),
    (
        [
            "x: Tokens.space-3;",
            "y: 18px;",
            "width: 36px;",
            "height: 18px;",
        ],
        "root.mixer-pan-reset(track_index);",
    ),
    (
        ["x: 24px;", "y: 54px;", "width: 48px;", "height: 116px;"],
        "root.mixer-master-fader-reset();",
    ),
    (
        [
            "x: Tokens.space-3;",
            "y: 18px;",
            "width: 36px;",
            "height: 18px;",
        ],
        "root.mixer-master-pan-reset();",
    ),
];

/// ① 四块命中面各自绑对自己的回调，几何一位没动。
#[test]
fn each_mixer_hit_rect_binds_its_own_double_click_reset() {
    let console = include_str!("../ui/console/mixer_console.slint");
    let lines: Vec<&str> = console.lines().collect();
    let arms = line_numbers(console, "double-clicked => {");
    assert_eq!(
        arms.len(),
        4,
        "调音台必须**恰好**四块带 `double-clicked` 的命中面（推子 / 声相 × 通道条 / 主控）: {arms:?}"
    );

    for (arm_line, (geometry, call)) in arms.iter().zip(TRACK_AND_MASTER_HIT_RECTS.iter()) {
        // `double-clicked` 那一行的 0 基下标。命中面头部（`TouchArea {` + 四行几何）
        // 必须在它**上面**、间隔不超过一行注释。
        let index = *arm_line - 1;
        let head = &lines[index.saturating_sub(8)..=index];
        let head_joined = head.join("\n");
        assert!(
            head_joined.contains("TouchArea {"),
            "第 {} 块命中面的 `double-clicked` 必须挂在 `TouchArea` 上: {head_joined}",
            arm_line
        );
        for &row in geometry.iter() {
            assert!(
                head_joined.contains(row),
                "第 {arm_line} 行的命中面几何必须逐行读得到 `{row}`（本票只**加**双击臂，\
                 ⛔ 挪动矩形 —— 推子帽 / 声相旋钮的位置与 `host` 的行程常量都按它算）: \
                 {head_joined}"
            );
        }
        // 调用在 `double-clicked => {` 之后的两行内（中间只有一行回调名）。
        let body = lines[index..(index + 3).min(lines.len())].join("\n");
        assert!(
            body.contains(*call),
            "第 {arm_line} 行的双击必须落到 `{call}`（⛔ 串到另一面上）: {body}"
        );
    }
}

/// ② 四个回调从组件一路转发到 `MainWindow`，每一段**恰好**一次。
#[test]
fn the_four_reset_callbacks_forward_to_the_main_window_exactly_once_each() {
    // (声明文本, 转发文本)：主控那两条**不带下标**（主总线不在 `track-ids` 里）。
    const CHAIN: [(&str, &str); 4] = [
        (
            "callback mixer-fader-reset(int);",
            "root.mixer-fader-reset(index);",
        ),
        (
            "callback mixer-pan-reset(int);",
            "root.mixer-pan-reset(index);",
        ),
        (
            "callback mixer-master-fader-reset();",
            "root.mixer-master-fader-reset();",
        ),
        (
            "callback mixer-master-pan-reset();",
            "root.mixer-master-pan-reset();",
        ),
    ];

    // 组件与两个转发层都必须**声明**这四个回调（`MixerConsole` 的声明是给 `TouchArea` 调的）。
    for (file, source) in [
        (
            "ui/console/mixer_console.slint",
            include_str!("../ui/console/mixer_console.slint"),
        ),
        (
            "ui/console/console_tabs.slint",
            include_str!("../ui/console/console_tabs.slint"),
        ),
        ("ui/app.slint", include_str!("../ui/app.slint")),
    ] {
        for (declaration, _) in CHAIN {
            assert_eq!(
                source.matches(declaration).count(),
                1,
                "`{file}` 必须**恰好**声明一次 `{declaration}`"
            );
        }
    }

    // 两级转发：`MixerConsole → ConsoleTabs → MainWindow`。
    for (file, source) in [
        (
            "ui/console/console_tabs.slint",
            include_str!("../ui/console/console_tabs.slint"),
        ),
        ("ui/app.slint", include_str!("../ui/app.slint")),
    ] {
        for (_, forwarding) in CHAIN {
            assert_eq!(
                source.matches(forwarding).count(),
                1,
                "`{file}` 必须把 `{forwarding}` 原样上抛**恰好**一次（0 次 ⇒ 双击落在没人听的\
                 回调上 = `ADR-0052` 那一类画了不动的控件；2 次 ⇒ 有第二条路径绕过宿主）"
            );
        }
    }
}

/// ③ 宿主四条臂：先清手势、后提交，而且写的是自己那一格。
#[test]
fn every_host_reset_arm_clears_the_gesture_before_committing_its_own_field() {
    let host = include_str!("../src/host.rs");
    const ARMS: [(&str, &str, &str); 4] = [
        (
            "ui.on_mixer_fader_reset(",
            "end_mixer_gesture(&ui, &gesture);",
            "MixerResetField::Volume,",
        ),
        (
            "ui.on_mixer_pan_reset(",
            "end_mixer_gesture(&ui, &gesture);",
            "MixerResetField::Pan,",
        ),
        (
            "ui.on_mixer_master_fader_reset(",
            "end_mixer_gesture(&ui, &gesture);",
            "MixerResetField::Volume,",
        ),
        (
            "ui.on_mixer_master_pan_reset(",
            "end_mixer_gesture(&ui, &gesture);",
            "MixerResetField::Pan,",
        ),
    ];

    for (arm, clear, field) in ARMS {
        assert_eq!(
            host.matches(arm).count(),
            1,
            "宿主必须**恰好**接一次 `{arm}`（`.slint` 声明了四个复位回调 ⇒ 少一条就是画了不动）"
        );
        let start = host.find(arm).expect("上面已断言存在");
        // 臂体：从这一臂到它自己的 `});`（下一处 `});` 之后即整臂结束）。
        let tail = &host[start..];
        let body = &tail[..tail.find("});").expect("臂必须以 `});` 收尾")];
        let clear_at = body.find(clear).unwrap_or_else(|| {
            panic!(
                "`{arm}` 必须在提交**之前**调用 `{clear}`：带着在手的拖拽记录提交 ⇒\
                    重投影丢掉正在拖的 `TouchArea` ⇒ `mixer-drag-active` 黏住。\n臂体: {body}"
            )
        });
        let commit_at = body
            .find("commit_mixer_reset(")
            .unwrap_or_else(|| panic!("`{arm}` 必须落到 `commit_mixer_reset`（唯一落点）: {body}"));
        assert!(
            clear_at < commit_at,
            "`{arm}` 的清零必须**先于**提交（{clear_at} < {commit_at} 不成立）: {body}"
        );
        assert!(
            body.contains(field),
            "`{arm}` 必须复位**自己**那一格（期望 `{field}`）: {body}"
        );
    }

    // 主控那两条按**身份**寻址（不下标）：取身份的唯一入口是 `master_track_id`。
    for arm in [
        "ui.on_mixer_master_fader_reset(",
        "ui.on_mixer_master_pan_reset(",
    ] {
        let start = host.find(arm).expect("上面已断言存在");
        let tail = &host[start..];
        let body = &tail[..tail.find("});").expect("臂必须以 `});` 收尾")];
        assert!(
            body.contains("master_track_id(&port)"),
            "`{arm}` 必须从权威工程取主总线身份（⛔ 猜下标）: {body}"
        );
        assert!(
            !body.contains("track_id_at("),
            "`{arm}` 与下标无关（主总线不在 `track-ids` 里）: {body}"
        );
    }
}

/// ④ 中性值只有一处定义：落点唯一、`0.0` 只出现一次。
#[test]
fn the_neutral_value_is_defined_in_exactly_one_place() {
    let host = include_str!("../src/host.rs");
    assert_eq!(
        host.matches("fn commit_mixer_reset(").count(),
        1,
        "复位的落点必须**只有一个**函数"
    );
    // 1 处定义 + 4 处调用。
    assert_eq!(
        host.matches("commit_mixer_reset(").count(),
        5,
        "四条复位臂必须全部汇到 `commit_mixer_reset`（不能有的臂自己拼 `Op`）"
    );
    assert_eq!(
        host.matches("new_val: 0.0").count(),
        1,
        "`0.0`（`0 dB` = unity、声相 = 中央）必须**恰好**出现一次：出现第二处就说明界面上\
         有了一份会与它分叉的\"默认值\""
    );
    // 已经是中性值 ⇒ 不提交（否则撤销栈里多一步什么都没改的历史）。
    assert_eq!(
        host.matches("current.to_bits() == 0.0_f32.to_bits()")
            .count(),
        1,
        "`commit_mixer_reset` 必须**先**位比较当前值（与拖动收尾同口径：`-0.0` 不算中性）"
    );
}
