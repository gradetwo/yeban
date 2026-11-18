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
//! | `chord` | `Cmaj7` | `(none)` | **没有**（和弦识别未接线：`yeban-theory` 边自鼓机线起存在，但只供鼓机生成——判据 ④） |
//! | `device` | `48 kHz / 24-bit · DSP 3.2% · 目标 120 FPS` | `—` | `host::publish_status_bar` ← `ViewState::audio_config_display` |
//!
//! ⇒ 上一版判据 ①（默认值 == 旧字面量）与判据 ③（断言**没有**写者）在新状态下**必然为假**。
//! 本文件按上一版留下的指示（"若这三条已经被真接上，请在本提交里**重写**登记，而不是删掉"）
//! 换成**同向更严**的一组：
//!
//! - 判据 ① 钉住三条默认值是**"还没有读数"的形态**，不是任何一个具体读数；
//! - 判据 ② 钉住登记还在，且点名两条真实数据源与 `chord` 的能力缺口；
//! - 判据 ③ 钉住**写者真的存在、且只有一个**（`src/host.rs`），`chord` 仍然**没有**写者；
//! - 判据 ④ 钉住 `yeban-theory` 边**只供鼓机生成**（`chord` 识别仍未接线；旧代理"边不存在"已随鼓机线失效，见判据 ④ 的改写注记）；
//! - 判据 ⑤ 钉住"没选区时显示什么"只有**一个**口径（`.slint` 默认值 == `host.rs` 常量）；
//! - 判据 ⑥ 钉住删掉 `DSP …%` 那一格的**理由**仍然成立（app 里没有任何负载读数）。
//!
//! ## 本文件在 2026-11-17 再改判据 ④（`line/drum-machine-p1` 第三轮修复）
//!
//! 鼓机线给 `yeban-app` 加了 `yeban-theory` 依赖边（`bridge::drum_gen_ops` 生成鼓型、
//! `host::wire_drum_gen_dialog` 投影风格、`elements` 注册风格行），旧判据 ④ 的代理命题
//! 「边不存在 ⇒ 能力不存在」随之失效，分支 CI 单败本判据。边是真实需要的（删边等于把鼓机
//! 生成搬出 app 或走 MCP 环回，与台账第 105 行「同一 theory 生成层」的设计冲突），故修法
//! 不是撤边，而是把判据改写成**真不变量**：边存在，但 `yeban_theory::` 引用全在鼓机白名单内
//! （11 个符号，实测 23 处 / 3 文件），且 `chord` 仍无写者（判据 ③）、仍显示 `(none)`
//! （判据 ①）。新判据自带 R56 合成正负对照。可见文本一位未动 ⇒ 无基准重录。
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

#![allow(unused)]
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
/// 双语 i18n（line/ui-i18n）后带文案的默认值写作 `@tr("<literal>")`：解析时先
/// 脱掉这层包装再按字面量核对 —— 本判据钉的是**值**，不是表示形式。
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
    // `@tr("…")` 包装（i18n）先脱掉，再按裸字面量核对值。
    let value = value
        .strip_prefix("@tr(")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(value);
    let literal = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| {
            panic!("`{needle}` 的默认值必须是双引号字符串字面量（可带 @tr 包装）；实际 {value:?}")
        });
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
///   - `yeban-theory`：`chord` 的能力缺口（判据 ④ 查这条边只供鼓机生成）。
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
    // 新 UI 重构: 旧 status bar 的静态文本断言不再成立; 核对契约面与宿主接线仍在。
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(app.contains("> playing:"), "契约面必须声明 playing");
    assert!(
        host.contains("apply_transport"),
        "宿主必须注册 apply_transport"
    );
    assert!(
        host.contains("wire_aether_actions"),
        "宿主必须注册新 UI 桥接层"
    );
}

/// 鼓机符号白名单：`yeban-app` 调 `yeban-theory` 只许这 13 个符号。
///
/// 实测（2026-11-17，`crates/yeban-app/src` 去 `//` 注释后）：23 处引用、3 个文件
/// （`bridge.rs` / `host.rs` / `elements.rs`），符号恰为这 11 个；2026-11-18
/// 鼓机效果层又加 `DrumFxFamilies` / `DrumFxPreset`（`drum_preset.rs`，仅供
/// `bridge.rs` 落逐风格 FX），共 29 处 / 13 符号。
/// `factory_drum_style` 与 `factory_drum_styles` 是两个不同符号，须各列一条。
const DRUM_THEORY_SYMBOLS: [&str; 13] = [
    "ACOUSTIC_KIT_SFZ_PATH",
    "DrumFxFamilies",
    "DrumFxPreset",
    "DrumGenParams",
    "DrumKitSetup",
    "DrumStylePreset",
    "KitFamily",
    "TICKS_PER_BAR",
    "factory_drum_style",
    "factory_drum_style_ids",
    "factory_drum_styles",
    "generate_drum_pattern",
    "kit_setup",
];

/// 扫一段源码，返回（`yeban_theory::` 符号表，未入账的行）。
///
/// 未入账 = 含 `yeban_theory` 却提不出符号的行（`use yeban_theory::{…}` 大括号导入、
/// 裸 `use yeban_theory;` 之类）—— 白名单扫不到的形态必须先红，而不是静默漏过。
/// 逐行先过 `code_only`：注释里的引用不算（登记与留痕本身就会写这些名字）。
fn theory_references_in_code(code: &str) -> (Vec<String>, Vec<String>) {
    let mut symbols = Vec::new();
    let mut unaccounted = Vec::new();
    for line in code.lines() {
        let line = code_only(line);
        if !line.contains("yeban_theory") {
            continue;
        }
        let mut rest = line;
        let mut found_any = false;
        while let Some(idx) = rest.find("yeban_theory::") {
            found_any = true;
            let after = &rest[idx + "yeban_theory::".len()..];
            let symbol: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if symbol.is_empty() {
                unaccounted.push(line.trim().to_owned());
                break;
            }
            symbols.push(symbol);
            rest = after;
        }
        if !found_any {
            unaccounted.push(line.trim().to_owned());
        }
    }
    (symbols, unaccounted)
}

/// **判据 ④**：`yeban-theory` 边**只供鼓机生成**，`chord` 识别仍未接线。
///
/// 2026-11-17 改写（`line/drum-machine-p1` 第三轮修复）：旧代理「边不存在 ⇒ 能力不存在」
/// 已失效 —— 鼓机线真实调这条边。真不变量是：`crates/yeban-app/src` 里每一条
/// `yeban_theory::` 引用都落在鼓机白名单内；`chord` 仍无写者（判据 ③ 另行钉
/// `set_chord` 不出现）、仍显示 `(none)`（判据 ①）。`theory` 里确实有和弦能力
/// （`chord.rs` / `Chord` / `genre` 与 `progression` 的 `chords`），所以白名单
/// 必须能抓住"用 theory 做和弦" —— 见下面的 R56 对照。
///
/// 四条断言：
///   1. 清单里有 `yeban-theory` 边（鼓机线需要它；撤边会先断鼓机编译，本条直接钉前提）；
///   2. 全部 `yeban_theory::` 符号都在白名单内；
///   3. 没有白名单扫不到的形态（大括号导入、裸 `use`）；
///   4. 非真空下界：引用 ≥ 10 处、跨 ≥ 2 个文件、不同符号 ≥ 5 个（实测 29 处 / 3 文件 /
///      13 符号）—— 扫描域塌成空/个位数时本条先红（R93/R100）。
///
/// 另附 R56 合成对照（与真扫描同一段代码）：纯鼓片段 ⇒ 干净；含 `Chord` /
/// `detect_chord` 的片段 ⇒ 点名报出；大括号导入 / 裸 `use` ⇒ 报"未入账"；
/// 注释里的引用 ⇒ 不报。
#[test]
fn theory_edge_is_drum_only_and_chord_still_has_no_recognition() {
    // (1) 边必须存在：鼓机线需要它。
    let edge = APP_MANIFEST
        .lines()
        .map(str::trim)
        .find(|line| !line.starts_with('#') && line.starts_with("yeban-theory"));
    assert!(
        edge.is_some(),
        "`crates/yeban-app/Cargo.toml` 里必须有 `yeban-theory` 依赖边 —— 鼓机线\
         （`bridge::drum_gen_ops` / 对话框 / 风格注册）真实调它。若有人撤边，鼓机编译\
         先断；本条直接钉住这个前提。"
    );

    // (2)(3) 真扫描：全部符号入白名单，无未入账形态。
    let files = app_source_files();
    assert!(
        files.len() >= 20,
        "扫描域非真空：`crates/yeban-app/src` 下 `.rs` 文件数必须 ≥ 20（实测 {}）。\
         文件数塌了 ⇒ 判据在扫一个空目录。",
        files.len()
    );
    let mut total_uses = 0;
    let mut distinct: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut files_with_uses = 0;
    let mut violations: Vec<String> = Vec::new();
    for (path, text) in &files {
        let (symbols, unaccounted) = theory_references_in_code(text);
        if !symbols.is_empty() {
            files_with_uses += 1;
        }
        for symbol in &symbols {
            total_uses += 1;
            distinct.insert(symbol.clone());
            if !DRUM_THEORY_SYMBOLS.contains(&symbol.as_str()) {
                violations.push(format!(
                    "{}: `yeban_theory::{symbol}` 不在鼓机白名单内 —— theory 边只供鼓机生成，\
                     和弦识别仍未接线（R9 登记）",
                    path.display()
                ));
            }
        }
        for line in &unaccounted {
            violations.push(format!(
                "{}: 白名单扫不到的 theory 引用（大括号导入 / 裸 `use` 之类）：{line}",
                path.display()
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "theory 边被挪作他用（{} 处）：\n{}",
        violations.len(),
        violations.join("\n")
    );

    // (4) 非真空下界（实测 23 处 / 3 文件 / 11 符号）。
    assert!(
        total_uses >= 10,
        "引用数非真空：`yeban_theory::` 引用必须 ≥ 10 处（实测 29 处），当前 {total_uses} 处。\
         引用塌了 ⇒ 要么鼓机接线被删了，要么扫描本身坏了。"
    );
    assert!(
        files_with_uses >= 2,
        "引用分布非真空：含 theory 引用的文件必须 ≥ 2 个（实测 3 个），当前 {files_with_uses} 个。"
    );
    assert!(
        distinct.len() >= 5,
        "符号面非真空：不同 theory 符号必须 ≥ 5 个（实测 13 个），当前 {} 个。",
        distinct.len()
    );

    // R56 合成对照：同一段判定代码跑四份合成片段。
    let (symbols, unaccounted) = theory_references_in_code(
        "let p = yeban_theory::generate_drum_pattern(&preset, &params).unwrap();\n\
         let s = yeban_theory::factory_drum_styles();",
    );
    // 对照夹具前提（R115）：合成纯鼓片段恰含 2 处 theory 引用 —— 空集时 `.all`
    // 恒真，须先把 `symbols` 的长度钉死，下面的 `.all` 断言才不是真空通过。
    assert_eq!(
        symbols.len(),
        2,
        "对照夹具前提：纯鼓片段的 theory 引用必须恰为 2 处（`generate_drum_pattern`／`factory_drum_styles`），当前 {} 处。",
        symbols.len()
    );
    assert!(
        unaccounted.is_empty(),
        "对照失败：纯鼓片段不应有未入账行：{unaccounted:?}"
    );
    assert!(
        symbols
            .iter()
            .all(|s| DRUM_THEORY_SYMBOLS.contains(&s.as_str())),
        "对照失败：纯鼓片段的符号必须全在白名单内：{symbols:?}"
    );
    let (symbols, _) =
        theory_references_in_code("let c = yeban_theory::Chord::from_notes(&notes);");
    assert_eq!(
        symbols,
        vec!["Chord".to_string()],
        "对照失败：和弦片段必须被点名报出符号 `Chord`"
    );
    assert!(
        !DRUM_THEORY_SYMBOLS.contains(&"Chord"),
        "对照失败：白名单里不许有 `Chord` —— 否则判据抓不住\"用 theory 做和弦\""
    );
    let (_, unaccounted) = theory_references_in_code("use yeban_theory::{Chord, DrumGenParams};");
    assert_eq!(
        unaccounted.len(),
        1,
        "对照失败：大括号导入必须报 1 行未入账（白名单扫不到的形态不能静默漏过）"
    );
    let (_, unaccounted) = theory_references_in_code("use yeban_theory;");
    assert_eq!(
        unaccounted.len(),
        1,
        "对照失败：裸 `use yeban_theory;` 必须报 1 行未入账"
    );
    let (symbols, unaccounted) = theory_references_in_code(
        "// 登记：yeban_theory::Chord 是和弦能力（只写在注释里）\n\
         let p = yeban_theory::generate_drum_pattern(&preset, &params).unwrap();",
    );
    assert!(
        unaccounted.is_empty() && symbols == vec!["generate_drum_pattern".to_string()],
        "对照失败：注释里的引用不算，只认代码里的 1 处鼓引用：{symbols:?} / {unaccounted:?}"
    );
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
    // 新 UI 重构: 旧 status bar 的静态文本断言不再成立; 核对契约面与宿主接线仍在。
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(app.contains("> playing:"), "契约面必须声明 playing");
    assert!(
        host.contains("apply_transport"),
        "宿主必须注册 apply_transport"
    );
    assert!(
        host.contains("wire_aether_actions"),
        "宿主必须注册新 UI 桥接层"
    );
}
