//! 状态栏三条默认读数的**像素代价**与诚实性登记的机械看守 —— 台账 `R9`（`line/app-r9`）。
//!
//! ## 这四条判据在守什么（以及不守什么）
//!
//! `crates/yeban-app/ui/status_bar.slint` 的 `selection` / `chord` / `device` 三条 `in property
//! <string>` **没有任何宿主写者**（实测：`git grep -n 'set_selection\|set_chord\|set_device'
//! crates/yeban-app/src` 命中 **0**），而 `ui/app.slint` 的 `StatusBar { … }` 实例只传
//! `save-status` / `save-succeeded` ⇒ 用户看到的就是这三条**默认值**，且它们在**默认帧里可见**。
//!
//! 因此"改这三条里的任何一条"都**不是**一次普通文案改动：`crates/yeban-app/tests/golden/linux/**`
//! 的 5 张基准是**逐字节**比对的（`src/test_port_adapter.rs:174` 的 `assert_matches_golden`），
//! 而这三格在基准里**没有被遮罩** —— 本票实测这三格的非黑像素是 5280/5280、2304/2304、6144/6144
//! （矩形 `(8,1056,220,24)` / `(236,1056,96,24)` / `(1652,1056,256,24)`，出处见
//! `status_bar.slint` 的登记块 ①）。动态区遮罩只走 MCP 预览那条路
//! （`crates/yeban-ui-mcp/src/surface.rs:631`），不进 golden 比对。
//!
//! 判据 ① 把"三条默认值 == 基准里那三格的内容"钉成一条**可在任何平台跑**的红线：改文案的人会在
//! 本地（而不是只在 Linux CI 上）看到"这不是文案改动，是基准过期"。
//!
//! ⚠ **它不证明界面是诚实的**。它只证明"当前登记的那三条默认值没有被静默改掉"。
//! 真正的关闭动作（把三条接上真数据 / 删掉编造的读数）**必然改可见文本 ⇒ 必须重录基准**，
//! 那一步等负责人排重录（登记块里写着）。判据 ② 守的是**登记本身**不被悄悄删掉。
//!
//! 判据 ③ / ④ 补的是**登记里那两条事实断言**（"没有宿主写者" / "`yeban-theory` 不在依赖图上"）：
//! 判据 ② 只查关键词**在不在这段注释里**，它**不查断言本身真假** —— 谁在 `src/` 里写一句
//! `ui.set_device(…)`，登记当场变成新的假话，而 ② 仍然绿。③ / ④ 就是把这个缺口补上。
//!
//! 走**源码文本**而不是渲染像素，理由与 `tests/theme_selection.rs` 的 ⑧b 同款：观测面是文本时
//! 判据不依赖 Tier-1 渲染，也能在**没有基准**的平台（本机 macOS 只有 Linux 基准）给出确定判决。

/// `ui/status_bar.slint` 的源码文本。
///
/// 用 `include_str!` 而不是运行时读文件：文件被删 / 被改名 ⇒ **编译期**就红（判据不会"因为读不到
/// 文件而跳过"）；并且改 `.slint` 会自动让这个测试目标重建。
const STATUS_BAR_SLINT: &str = include_str!("../ui/status_bar.slint");

/// 取 `in property <string> <name>: "<literal>";` 的默认值（**去掉**首尾引号）。
///
/// 只认**行首**的声明（`trim_start` 后以 `in property <string> <name>:` 开头）：内联在别处的
/// 同名子串（例如注释里的 `selection`）不会被误认。找不到声明本身即判据失败 ——
/// "属性被改名/被删"是**比文案漂移更严重**的漂移，不能当"没找到就跳过"。
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

/// **判据 ①**：三条默认值必须**逐字符**等于基准帧里渲染的那三条（基线 `18a8874` 的值）。
///
/// 红了怎么办（写在断言消息里，别去猜）：这三条是**基准图的内容**，不是普通文案 ——
/// 先按手动档 `gates-manual.yml` 的 `gate=goldens` 重录 `crates/yeban-app/tests/golden/linux/**`
/// 的 5 张并复核，**同一个提交里**再更新这里的期望值。只改一边 = 让 Linux CI 红或者让判据撒谎。
#[test]
fn status_bar_three_defaults_are_the_golden_coupled_literals() {
    let expected = [
        ("selection", "选区 1.1.000 – 5.4.480"),
        ("chord", "Cmaj7"),
        ("device", "48 kHz / 24-bit · DSP 3.2% · 目标 120 FPS"),
    ];
    for (property, want) in expected {
        let got = default_literal(STATUS_BAR_SLINT, property);
        assert_eq!(
            got, want,
            "`status_bar.slint` 的 `{property}` 默认值变了。这三条默认值**就是基准帧里那三格的内容**\
             （`tests/golden/linux/**` 的 5 张，逐字节比对，且这三格在基准里未被遮罩）\
             ⇒ 改它 = 基准确定性过期。正确顺序：先 `gates-manual.yml` 的 `gate=goldens` 重录 + 人复核，\
             再在同一个提交里更新本判据的期望值。`{property}`: want {want:?}, got {got:?}"
        );
    }
}

/// **判据 ②**：诚实性登记必须还在，而且必须还点着这三条属性的**宿主写者**与 `chord` 的**能力缺口**。
///
/// 口径（刻意只要"关键词还在"，不比对整段文字 —— 那是文案的判据，不是事实的判据）：
///   - `set_selection` / `set_chord` / `set_device`：登记引用的那三条 `git grep` 判据（命中 0）；
///   - `yeban-theory`：`chord` 的能力缺口（本票实测 `cargo tree -p yeban-app -e normal --locked`
///     里命中 **0**）—— 一旦这条边被加上，登记就过期了，必须重写而不是让它悬着。
///
/// ⚠ 这条判据**不**证明界面诚实；它只证明"这笔债还记在账上，没被顺手删掉"。
#[test]
fn status_bar_honesty_registration_still_names_the_three_unwired_properties() {
    for token in ["set_selection", "set_chord", "set_device", "yeban-theory"] {
        assert!(
            STATUS_BAR_SLINT.contains(token),
            "`ui/status_bar.slint` 的诚实性登记里必须仍然出现 `{token}` —— \
             这条判据守的是**登记本身**（`selection` / `chord` / `device` 三条没有宿主写者、\
             `chord` 的能力缺口）。若这三条已经被真接上，请在本提交里**重写**登记\
             （并重录基准），而不是删掉这段。"
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

/// **判据 ③**：登记里那句"`selection` / `chord` / `device` **没有任何宿主写者**"必须仍然成立。
///
/// 观测面 = `crates/yeban-app/src` 下**全部** `.rs` 的**代码文本**（去掉 `//` 注释后）里有没有
/// `set_selection` / `set_chord` / `set_device`（Slint 为这三条 `in property` 生成的三个 setter 名）。
///
/// 为什么要去注释（与 `tests/undo_tree_honesty.rs` 的 `code_only` 同款）：登记与留痕本身就会
/// 引用这些名字（"我们没有 `ui.set_device(…)`"）。把注释也算进来，判据就会逼着作者**不写留痕**，
/// 与本仓库的纪律相反。代价是 `/* … */` 块注释不在口径内（`src/` 里没有这种注释）。
///
/// 属性本身被改名 ⇒ 本判据会漏，但那种改名已经由判据 ①（按属性名寻址）抓住。
///
/// ⚠ 判据 ② 只查这些名字**在不在这段注释里**；本判据查的是**断言本身**。
/// 两者一起才是"登记没有变成新的假话"。
#[test]
fn status_bar_three_properties_still_have_no_host_writer() {
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in app_source_files() {
        for (index, line) in text.lines().enumerate() {
            let code = match line.split_once("//") {
                Some((code, _comment)) => code,
                None => line,
            };
            for setter in ["set_selection", "set_chord", "set_device"] {
                if code.contains(setter) {
                    offenders.push(format!("{}:{}: {}", path.display(), index + 1, code.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "登记（`ui/status_bar.slint`）写着这三条属性**没有任何宿主写者**，但 `crates/yeban-app/src` \
         里出现了写者调用：\n  {}\n\
         为什么这不是一次普通接线：这三条默认值**就是基准帧里那三格的内容**，而 5 张基准是\
         **整帧逐字节**比对的（`src/test_port_adapter.rs` 的 `assert_matches_golden`；被比对的那一帧\
         由 `host::build_main_window` 构造 —— 与生产路径是**同一个**构造函数，见\
         `src/test_port_adapter.rs` 的 `build_demo_main_window`）⇒ 把真值写进去必然改默认帧像素。\n\
         同一个提交里按顺序做完三件事：① 手动档 `gates-manual.yml` 的 `gate=goldens` 重录\
         `crates/yeban-app/tests/golden/linux/**` 的 5 张 + 人复核；② 重写 `ui/status_bar.slint` 的\
         诚实性登记（它现在写着「没有宿主写者」，加了写者就成了新的假话）；③ 把本判据改成断言\
         「写者存在、值来自哪个读数」—— 不要直接删掉这一条，那等于把判据换成一个没人看的空洞。",
        offenders.join("\n  ")
    );
}

/// `crates/yeban-app/Cargo.toml` 的源码文本（**编译期**读入：清单被删 / 被改名 ⇒ 判据编译不过，
/// 不会"因为读不到文件而跳过"）。
const APP_MANIFEST: &str = include_str!("../Cargo.toml");

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
             改这一格要先给它换一个对照。\n\
             出处：`docs/ledger/integration-rulings-notes.md` 的 R9 行 ——「修，但先处理 D24 对照样本」。"
        );
    }
}
