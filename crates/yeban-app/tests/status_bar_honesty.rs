//! 状态栏三格读数的**诚实性**看守 —— 台账 `R9`（`line/app-r9pix` 关闭那一票）。
//!
//! ## 本文件在 2026-10-08 被**重写**（不是删除，也不是放宽）
//!
//! 上一版（`line/app-r9`）守的是"三条默认值就是基准帧里那三格的内容，且三条都**没有**
//! 宿主写者"。本票把那三格里的假话清掉了：
//!
//! | 属性 | 改前（假） | 改后 | 宿主写者 |
//! | :--- | :--- | :--- | :--- |
//! | `selection` | `选区 1.1.000 – 5.4.480` | `选区 无` | `host::publish_status_bar` ← `ViewState::selection_tick_span` |
//! | `chord` | `Cmaj7` | `(none)` | **没有**（能力不存在：`yeban-theory` 不在依赖图上） |
//! | `device` | `48 kHz / 24-bit · DSP 3.2% · 目标 120 FPS` | `—` | `host::publish_status_bar` ← `ViewState::audio_config_display` |
//!
//! ⇒ 上一版判据 ①（默认值 == 旧字面量）与判据 ③（断言**没有**写者）在新状态下**必然为假**。
//! 本文件按上一版留下的指示（"若这三条已经被真接上，请在本提交里**重写**登记，而不是删掉"）
//! 换成**同向更严**的一组：
//!
//! - 判据 ① 钉住三条默认值是**"还没有读数"的形态**，不是任何一个具体读数；
//! - 判据 ② 钉住登记还在，且点名两条真实数据源与 `chord` 的能力缺口；
//! - 判据 ③ 钉住**写者真的存在、且只有一个**（`src/host.rs`），`chord` 仍然**没有**写者；
//! - 判据 ④ 钉住 `chord` 的能力缺口没被"顺手加依赖"绕过（`yeban-theory` 边仍不存在）；
//! - 判据 ⑤ 钉住"没选区时显示什么"只有**一个**口径（`.slint` 默认值 == `host.rs` 常量）；
//! - 判据 ⑥ 钉住删掉 `DSP …%` 那一格的**理由**仍然成立（app 里没有任何负载读数）。
//!
//! ⚠ 本文件**不**证明界面是诚实的。它证明"登记、写者、数据源名与无读数形态"这四件事
//! 没有被静默改掉。像素那一半由 `src/test_port_adapter.rs` 的 `[UI-MCP-003]` 逐字节比对守。
//!
//! ⚠ **像素代价（必写）**：这三格在**默认帧里可见**，且 5 张 Linux 基准**没有遮罩**它们
//! （矩形 `(8,1056,220,24)` / `(236,1056,96,24)` / `(1652,1056,256,24)`）
//! ⇒ 本票改文本 = 5 张基准**确定性过期**。重录只许走手动档 `gates-manual.yml` 的
//! `gate=goldens`（`YEBAN_WRITE_GOLDEN=1 cargo test -p yeban-app --locked --test real_ui_tier1`）。
//! **本票未重录**（本机是 macOS：`PlatformTag::current()` 会写到 `golden/macos/`，
//! 而仓库里只有 `golden/linux/`；且 `AGENTS.md` §5.2 禁止在本机编 slint）。

/// 状态栏组件源码（`include_str!`：文件被删 / 改名 ⇒ **编译期**就红，判据不会"读不到就跳过"）。
const STATUS_BAR_SLINT: &str = include_str!("../ui/status_bar.slint");
/// 主窗口源码：它必须把两格**转发**给 `StatusBar`，否则状态栏会静默回落到默认值。
const APP_SLINT: &str = include_str!("../ui/app.slint");
/// 宿主源码：两个 setter 的**唯一**写者在这里。
const HOST_RS: &str = include_str!("../src/host.rs");
/// 本 crate 的清单：判据 ④ 的观测面。
const APP_MANIFEST: &str = include_str!("../Cargo.toml");

/// 取 `in property <string> <name>: "<literal>";` 的默认值（**去掉**首尾引号）。
///
/// 只认**行首**的声明（`trim_start` 后以 `in property <string> <name>:` 开头）：注释里
/// 内联的同名子串（例如登记块里那句"原文 ⇒ 改后"）不会被误认。找不到声明本身即判据失败
/// —— "属性被改名/被删"是**比文案漂移更严重**的漂移，不能当"没找到就跳过"。
fn default_literal(source: &str, property: &str) -> String {
    let needle = format!("in property <string> {property}:");
    let line = source
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with(&needle))
        .unwrap_or_else(|| {
            panic!("`ui/status_bar.slint` 里找不到 `{needle}` —— 属性被改名或删掉了")
        });
    let value = line
        .split_once(&needle)
        .expect("上面刚用同一个 needle 匹配过")
        .1
        .trim()
        .trim_end_matches(';')
        .trim();
    let literal = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("`{needle}` 的默认值必须是双引号字符串字面量；实际 {value:?}"));
    literal.to_owned()
}

/// **判据 ①**：三条默认值必须是**"还没有读数"的形态**，不是任何一个具体读数。
///
/// 口径（三个值各自都是决定，不是随手挑的）：
///   - `选区 无` —— 空选区的**唯一**文本，与 `src/host.rs` 的 `STATUS_SELECTION_EMPTY`
///     逐字符相同（判据 ⑤ 机械对账）；
///   - `(none)` —— `chord` 的能力不存在，所以这一格永远停在这里（判据 ③/④ 守着）；
///   - `—` —— "还没有读数"。它**不是**任何一个采样率 / 位深。
///
/// ⚠ 这里断言的是**解析出来的默认值**，不是整份文件不含某个词：登记的"原文 ⇒ 改后"
/// 必须能引用旧字面量（否则留痕本身会被判据禁止），所以不能对整个文件做子串检查。
#[test]
fn status_bar_defaults_are_no_reading_forms_not_fabricated_values() {
    let expected = [
        ("selection", "选区 无"),
        ("chord", "(none)"),
        ("device", "—"),
    ];
    for (property, want) in expected {
        let got = default_literal(STATUS_BAR_SLINT, property);
        assert_eq!(
            got, want,
            "`status_bar.slint` 的 `{property}` 默认值必须是**无读数形态**（{want:?}）。\
             它落在默认帧的可见像素里（三格都未被遮罩）⇒ 改动它 = `tests/golden/linux/**` \
             的 5 张基准确定性过期。正确顺序：按手动档 `gates-manual.yml` 的 `gate=goldens` \
             重录 + 人复核，再在同一个提交里更新本判据。{property}: want {want:?}, got {got:?}"
        );
    }
}

/// **判据 ②**：R9 登记还在，而且点名两条真实数据源与 `chord` 的能力缺口。
///
/// 口径（刻意只要"关键词还在"，不比对整段文字 —— 那是文案的判据，不是事实的判据）：
///   - `publish_status_bar`：唯一的宿主写者（判据 ③ 查它真的存在）；
///   - `selection_tick_span` / `audio_config_display` / `timecode_for_ticks`：
///     两条数据源的**名字**，登记必须点得出来；
///   - `yeban-theory`：`chord` 的能力缺口（判据 ④ 查这条边真的不存在）。
///
/// ⚠ 这条**不**证明界面诚实；它证明"这笔债/这份出处还记在账上，没被顺手删掉"。
#[test]
fn status_bar_honesty_registration_names_the_real_sources_and_the_chord_gap() {
    for token in [
        "publish_status_bar",
        "selection_tick_span",
        "audio_config_display",
        "timecode_for_ticks",
        "yeban-theory",
    ] {
        assert!(
            STATUS_BAR_SLINT.contains(token),
            "`ui/status_bar.slint` 的 R9 登记里必须仍然出现 `{token}` —— 这条判据守的是\
             **登记本身**（两条真实数据源 + `chord` 的能力缺口）。若接线方式换了，\
             请在本提交里**重写**登记（并重录基准），而不是删掉它。"
        );
    }
}

/// `crates/yeban-app/src` 下的全部 `.rs` 源码：`(路径, 文本)`，按路径排序。
///
/// 读目录的返回顺序不确定 ⇒ 排序，好让判据的失败消息在两次运行之间可比。
///
/// 用**运行时递归读目录**而不是逐个 `include_str!`：判据必须看得见**新增**的源文件 ——
/// 逐个列文件的话，"把写者放进一个新文件"就是绕过判据的最短路径。
fn app_source_files() -> Vec<(std::path::PathBuf, String)> {
    /// 递归收集 `dir` 下的 `.rs`。
    fn walk(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("读目录 `{}` 失败: {err}", dir.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|err| panic!("读目录项失败: {err}"))
                .path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("读源文件 `{}` 失败: {err}", path.display()));
                out.push((path, text));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(root.is_dir(), "`{}` 必须是目录", root.display());
    let mut out = Vec::new();
    walk(&root, &mut out);
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

/// 去掉 `//` 之后的行注释，返回"代码文本"。
///
/// 为什么要去注释（与 `tests/undo_tree_honesty.rs` 的 `code_only` 同款）：登记与留痕
/// 本身就会引用这些名字（"写者是谁"这类句子）。把注释也算进来，判据就会逼着作者
/// **不写留痕**，与本仓库的纪律相反。代价是 `/* … */` 块注释不在口径内
/// （`src/` 里没有这种注释）。
fn code_only(line: &str) -> &str {
    match line.split_once("//") {
        Some((code, _comment)) => code,
        None => line,
    }
}

/// **判据 ③**：两条已接线的属性**各有且只有一个**宿主写者，`chord` 仍然**没有**写者。
///
/// 观测面 = `crates/yeban-app/src` 下**全部** `.rs` 的**代码文本**（去掉 `//` 注释）里
/// Slint 为 `status-selection` / `status-device` 生成的 setter 名。
///
/// 三条断言：
///   1. 两个 setter **都出现**（接线真的落了地）；
///   2. 两个 setter **只在 `src/host.rs` 里出现** —— 写者唯一（`publish_status_bar`），
///      不存在"第二个地方也写状态栏"的暗路；
///   3. `set_chord` **不出现** —— `chord` 没有能力也没有写者，登记那句仍然成立。
///
/// 另加一条转发断言：`ui/app.slint` 必须把两格传给 `StatusBar`。少传一处，
/// 状态栏就静默回落到默认值 —— 那正是本票修掉的那类"假话"。
#[test]
fn the_two_wired_cells_have_exactly_one_host_writer_and_chord_still_has_none() {
    let wired = ["set_status_selection", "set_status_device"];
    let mut hits: Vec<(String, usize, String)> = Vec::new();
    let mut chord_hits: Vec<String> = Vec::new();
    for (path, text) in app_source_files() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        for (index, line) in text.lines().enumerate() {
            let code = code_only(line);
            for setter in wired {
                if code.contains(setter) {
                    hits.push((name.clone(), index + 1, code.trim().to_owned()));
                }
            }
            if code.contains("set_chord") {
                chord_hits.push(format!("{}:{}: {}", path.display(), index + 1, code.trim()));
            }
        }
    }

    for setter in wired {
        assert!(
            hits.iter().any(|(_, _, code)| code.contains(setter)),
            "`{setter}` 在 `crates/yeban-app/src` 里一次都没被调用 ⇒ 那一格又回到\
             「界面显示的是一条默认值」的状态（台账 R9 修掉的正是这个）。\
             写者是 `src/host.rs` 的 `publish_status_bar`。"
        );
    }
    let strays: Vec<String> = hits
        .iter()
        .filter(|(name, _, _)| name != "host.rs")
        .map(|(name, line, code)| format!("{name}:{line}: {code}"))
        .collect();
    assert!(
        strays.is_empty(),
        "状态栏两格只允许**一个**写者（`src/host.rs` 的 `publish_status_bar`），\
         但 `src/` 的其它文件里也出现了 setter 调用：\n  {}\n\
         两个写者 = 两个真相源：换工程那条路与点击那条路会各写一份，谁后写谁赢。",
        strays.join("\n  ")
    );
    assert!(
        chord_hits.is_empty(),
        "登记写着 `chord` **没有宿主写者**（app 侧没有和弦识别能力），但 `src/` 里出现了\
         `set_chord`：\n  {}\n\
         ⇒ 要么这条能力真的接上了（那要先把 `yeban-theory` 依赖边过裁决、重写登记并重录基准），\
         要么这是写错了地方。⛔ 本票的红线是**不为 `chord` 加依赖边**。",
        chord_hits.join("\n  ")
    );

    for binding in [
        "selection: root.status-selection;",
        "device: root.status-device;",
    ] {
        assert!(
            APP_SLINT.contains(binding),
            "`ui/app.slint` 里必须把 `StatusBar` 的那一格接上宿主属性（`{binding}`）—— \
             不接就是把 `status_bar.slint` 的默认值当读数显示。"
        );
    }
}

/// **判据 ④**：登记里那句"`chord` 的能力不存在（`yeban-theory` 不在 `yeban-app` 的依赖图上）"
/// 必须仍然成立。
///
/// 观测面 = 本 crate 清单里有没有一条**以 `yeban-theory` 开头的依赖键**。先剔掉 `#` 注释行：
/// 本仓清单里大段注释会点名别的 crate，把注释也当成依赖会把判据自己变成假红源。
#[test]
fn chord_capability_gap_is_still_missing_no_yeban_theory_edge() {
    let edge = APP_MANIFEST
        .lines()
        .map(str::trim)
        .find(|line| !line.starts_with('#') && line.starts_with("yeban-theory"));
    if let Some(edge) = edge {
        panic!(
            "登记写着 `chord` 的能力不存在（`yeban-theory` 不在 `yeban-app` 的依赖图上），但 \
             `crates/yeban-app/Cargo.toml` 里出现了依赖边 `{edge}`。\n\
             本票的红线是**不为 `chord` 加依赖边**。若确实要接上和弦识别：先把登记重写成\
             「能力已接线」，并在同一个提交里处理基准 —— 这一格 `status-bar-chord` 同时是\
             ADR-0001 D24 的**对照样本**（`src/test_port_adapter.rs` 的 D24 墨迹判据），\
             改这一格要先确认新文本仍含 ASCII 墨迹。\n\
             出处：`docs/ledger/integration-rulings-notes.md` 的 R9 行。"
        );
    }
}

/// **判据 ⑤**：空选区的文本只有**一个**口径。
///
/// 两个事实源必须逐字符相同：
///   - `ui/status_bar.slint` 的 `selection` 默认值（写者跑之前显示的那一条）；
///   - `src/host.rs` 的 `pub const STATUS_SELECTION_EMPTY`（写者跑之后空选区写的那一条）。
///
/// 两者不同 ⇒ "没选区时显示什么"有两个答案，而这正是本票修掉的那类缺陷的形态
/// （两处各写一份，谁也不知道哪一份是真的）。
#[test]
fn the_empty_selection_text_has_exactly_one_wording() {
    let default = default_literal(STATUS_BAR_SLINT, "selection");
    let declaration = format!("pub const STATUS_SELECTION_EMPTY: &str = \"{default}\";");
    assert!(
        HOST_RS.contains(&declaration),
        "`src/host.rs` 里必须有一条 `{declaration}`（与 `ui/status_bar.slint` 的 `selection` \
         默认值逐字符相同）。当前 `.slint` 的默认值是 {default:?}；两者不同就是\
         「没选区时显示什么」有两个口径。"
    );
}

/// **判据 ⑥**：删掉 `DSP …%` 那一格的**理由**必须仍然成立 —— app 侧没有任何负载读数。
///
/// 口径（先说单位再量）：对象 = `crates/yeban-app/src` 全部 `.rs` 的**代码文本**
/// （`//` 注释已被 `code_only` 去掉）；量 = 出现 `dsp_load` / `cpu_load` / `load_percent`
/// 的**行数**；单位 = 行。当前实测 **0**。
///
/// ⚠ 必须去掉注释再量：登记与注释**自身**就会写下这三个词（本文件与
/// `ui/status_bar.slint`、`src/bridge.rs` 都写了），不过滤注释的 grep 会得到非零值，
/// 那是量法错误，不是"有了负载读数"。
///
/// 为什么把它写成判据而不是注释：那条 `"DSP 3.2%"` 是一个**编造的读数**。只要没有人
/// 真的算负载，这一格就**必须**是空的。若将来真有了负载读数，这条判据会红 —— 那时
/// 正确的动作是**先**把读数的出处写清楚、再决定要不要放回界面（并重录基准），
/// 而不是默默把它变回一个数字。
#[test]
fn no_dsp_load_reading_exists_in_the_app_source() {
    let mut hits: Vec<String> = Vec::new();
    for (path, text) in app_source_files() {
        for (index, line) in text.lines().enumerate() {
            let code = code_only(line);
            for token in ["dsp_load", "cpu_load", "load_percent"] {
                if code.contains(token) {
                    hits.push(format!("{}:{}: {}", path.display(), index + 1, code.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "`crates/yeban-app/src` 里出现了负载读数：\n  {}\n\
         台账 R9 删除 `device` 那格的 `DSP 3.2%` 的理由就是「没有真实读数」。\
         现在有读入了 ⇒ 请先把它接到界面并重录基准，同时**重写**这条判据与\
         `ui/status_bar.slint` 的登记（不要删掉这条判据）。",
        hits.join("\n  ")
    );
}
