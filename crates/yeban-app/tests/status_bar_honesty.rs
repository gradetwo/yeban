//! 状态栏三条默认读数的**像素代价**与诚实性登记的机械看守 —— 台账 `R9`（`line/app-r9`）。
//!
//! ## 这两条判据在守什么（以及不守什么）
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
