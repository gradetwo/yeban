//! **"自动化求值没有第二份实现"的机械审计**（`ADR-0001` **D46** 判据 ⑥）。
//!
//! ## 为什么需要一条独立审计
//!
//! 模型层把"目标 + tick ⇒ 此刻该用哪个值"收敛成**唯一**入口
//! `YebanProjectV1::automation_value_at`（`crates/yeban-model/src/automation.rs` 的模块文档
//! 明文："下游三条线（离线/实时渲染、音频引擎的音符调度与参数平滑、界面曲线绘制）
//! **不得**各自再写一份插值"）。但"不得"是一句话，不是机制 —— 三天后有人在 MCP 侧写
//! 一个 `t as f32 / span` 的线性插值，没有任何东西会红，而且它会**看起来是对的**
//! （线性曲线下逐位相同，只有 `SCurve` / `Exponential` 上才分叉）。
//!
//! 本模块把它变成机制：**扫描生产源码**，任何"看起来像自己算插值"的调用点都是违规。
//! 同一族做法在本仓库已有先例（`crate::undo_session::scan_second_undo_implementations`
//! 扫 `apply_inverse`/`.undo(`），本模块是它的**第二个实例**，扫描对象是自动化求值。
//!
//! ## 判据（`grep` 级，纯函数）
//!
//! | 规则 | 违规 | 为什么 |
//! | :--- | :--- | :--- |
//! | 生产代码里出现 `value_at(` 但**不是** `automation_value_at` | [`Violation::SecondEvaluation`] | 直接调泳道的纯计算版本，绕开了 `read_enabled` 开关与目标对账 |
//! | 生产代码里出现 `.ease(` | [`Violation::SecondInterpolation`] | 自己拿曲线形状插值 = 第二份口径（`CurveType::ease` 的唯一调用者必须是模型） |
//! | 生产代码里出现 `interpolate` | [`Violation::SecondInterpolation`] | 自己写插值函数 |
//!
//! **刻意不扫** `points_in_tick_order()`：它是模型公开的"按 tick 列出采样点"入口，
//! **不含插值**，界面画折线用它（`crates/yeban-app/src/automation.rs`）。
//! 把它也算违规会让审计变成"禁止读点"，那不是本判据要管的事。
//!
//! ## 为什么它能被裸 `rustc` 独立跑
//!
//! 本文件**只依赖 `std`**（连 `serde_json` 都不用），因此本机（yeban-mcp 含重依赖 ⇒
//! `cargo test -p yeban-mcp` 在本机跳过）也能真跑：
//!
//! ```text
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/src/domain/automation_audit.rs -o /tmp/x && /tmp/x
//! ```
//!
//! 真实的仓库源码由判据喂进来：`yeban_mcp::undo_session::read_rust_sources(
//! yeban_mcp::undo_session::production_source_roots())` —— 文件遍历器**复用**那一份，
//! 本模块不复制它。唯一复制的是 4 行的 `production_region`，并且有一条判据
//! （`production_region_agrees_with_undo_session`) 断言两份实现**逐字节相同**。
//!
//! ## 与 `extension_pure` 共用 `automation_value_at` 这个名字
//!
//! [`UNIQUE_EVALUATION_ENTRY`] 与 `crate::domain::extension_pure::AUTOMATION_ENTRY`
//! 都必须是 `"automation_value_at"`，判据 `entry_name_matches_the_pure_module` 钉住这点。

/// 模型层的**唯一求值入口**（下游必须调它）。
pub const UNIQUE_EVALUATION_ENTRY: &str = "automation_value_at";

/// "有人在读泳道值"的调用形状（合法与否取决于同一行有没有出现 [`UNIQUE_EVALUATION_ENTRY`]）。
pub const EVALUATION_CALL_NEEDLE: &str = "value_at(";

/// "有人在算插值"的调用形状。
pub const INTERPOLATION_NEEDLES: [&str; 2] = [".ease(", "interpolate"];

/// 一条违规。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Violation {
    /// 绕开唯一入口直接求值（`xxx.value_at(`）。
    SecondEvaluation,
    /// 自己插值（`.ease(` / `interpolate`）。
    SecondInterpolation,
}

impl Violation {
    /// 稳定的机器可读名（判据输出用）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SecondEvaluation => "secondEvaluation",
            Self::SecondInterpolation => "secondInterpolation",
        }
    }

    /// 人话说明（判据失败信息里直接可用）。
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::SecondEvaluation => "生产代码绕开唯一求值入口 `automation_value_at` 直接读泳道值",
            Self::SecondInterpolation => {
                "生产代码自己算插值（`ease` / `interpolate`）—— 自动化曲线因此有第二份口径"
            }
        }
    }
}

/// 取一份源码的**生产区**（`#[cfg(test)]` 属性**行**之前的全部内容）。
///
/// ⚠ 与 `crate::undo_session::production_region` 是**同一条口径**（整行以 `#[cfg(test)]`
/// 开头，而不是 `find("#[cfg(test)]")` —— 后者会把文档注释里提到这个词的地方当成测试区起点）。
/// 判据 `production_region_agrees_with_undo_session` 断言两份实现逐字节相同。
#[must_use]
pub fn production_region(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 去掉一行的**字符串字面量**与**行尾注释**，只留代码骨架。
///
/// 为什么必须先去掉字面量：判据自己的常量定义（`pub const …: &str = "value_at(";`）
/// 与失败文案（`"… \`ease\` …"`）**字面上**含有被禁的模式，但它们不是调用。
/// 不去掉的话，本模块会**把自己判红**（实测：第一版就是这样，见
/// `the_audit_source_is_clean_by_its_own_rules`）。
///
/// 口径是 grep 级：`'"'` 这类**字符字面量**里的引号会把这个小状态机带偏。
/// 本仓库的生产代码里没有这种写法；真出现了，它会让判据偏**严**（多报），不会漏报。
#[must_use]
pub fn code_without_literals(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<char> = chars.clone();
    let mut index = 0usize;
    while index < chars.len() {
        // ⓐ 行尾注释：从 `//` 到**下一个换行**（⛔ 不是"到输入末尾" —— 多行输入下
        //    后者会把**后续正文**一起抹掉 ⇒ **假阴性**，R214① 实测）。
        if chars[index] == '/' && chars.get(index + 1) == Some(&'/') {
            let mut cursor = index;
            while cursor < chars.len() && chars[cursor] != '\n' {
                out[cursor] = ' ';
                cursor += 1;
            }
            index = cursor;
            continue;
        }
        // ⓐ' **块**注释 `/* … */`：里面可能同时含 `"` 与 `//`（R214① 的对抗样本 ④）——
        //    不掩它就会让 `"` **翻转字符串状态** ⇒ 掩码失同步 ⇒ 掩盖真违规。换行保留。
        if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
            let mut cursor = index;
            out[cursor] = ' ';
            out[cursor + 1] = ' ';
            cursor += 2;
            while cursor < chars.len() {
                if chars[cursor] == '*' && chars.get(cursor + 1) == Some(&'/') {
                    out[cursor] = ' ';
                    out[cursor + 1] = ' ';
                    cursor += 2;
                    break;
                }
                if chars[cursor] != '\n' {
                    out[cursor] = ' ';
                }
                cursor += 1;
            }
            index = cursor;
            continue;
        }
        // ⓑ 字符字面量 `'x'` / `'\n'` / `'"'`（⛔ 不是生命周期 `'a`）。
        if chars[index] == '\'' {
            let (inner_start, inner_end) = if chars.get(index + 1) == Some(&'\\') {
                (index + 2, index + 3)
            } else {
                (index + 1, index + 2)
            };
            if chars.get(inner_end) == Some(&'\'') {
                for cell in out.iter_mut().take(inner_end).skip(inner_start) {
                    *cell = ' ';
                }
                index = inner_end + 1;
                continue;
            }
        }
        // ⓒ 字符串字面量：**只掩内部**，开/闭引号留着（R94：掩内部，不掩界符）。
        if chars[index] == '"' {
            let mut cursor = index + 1;
            while cursor < chars.len() {
                if chars[cursor] == '\\' {
                    out[cursor] = ' ';
                    if cursor + 1 < chars.len() {
                        out[cursor + 1] = ' ';
                    }
                    cursor += 2;
                    continue;
                }
                if chars[cursor] == '"' {
                    break;
                }
                out[cursor] = ' ';
                cursor += 1;
            }
            index = cursor + 1;
            continue;
        }
        index += 1;
    }
    out.into_iter().collect()
}

/// 扫描一批源码，返回违规清单（空 = 干净）。
///
/// 入参与 `crate::undo_session::scan_second_undo_implementations` 同型
/// （`&[(路径, 源码)]`），因此判据可以用**同一批**源码跑两条审计。
///
/// 规则：
///
/// 1. 注释行（`//` 开头）不算生产代码 —— 文档里提到 `lane.value_at` 是**说明**，
///    不是调用；
/// 2. 判定前先用 [`code_without_literals`] 去掉字符串与行尾注释（判据自己的常量
///    定义与文案就在字符串里）；
/// 3. 剩下的代码里出现 `value_at(` **且**不出现 `automation_value_at`
///    ⇒ [`Violation::SecondEvaluation`]；
/// 4. 出现 `.ease(` 或 `interpolate` ⇒ [`Violation::SecondInterpolation`]。
///
/// 返回的字符串形如 `crates/yeban-mcp/src/x.rs:12 [secondEvaluation] …`，可直接当失败信息。
#[must_use]
pub fn scan_second_automation_evaluations(sources: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (path, text) in sources {
        let production = production_region(text);
        for (lineno, line) in production.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let skeleton = code_without_literals(line);
            let mut record = |violation: Violation| {
                violations.push(format!(
                    "{path}:{} [{}] {}: {}",
                    lineno + 1,
                    violation.as_str(),
                    violation.message(),
                    code
                ));
            };
            for needle in INTERPOLATION_NEEDLES {
                if skeleton.contains(needle) {
                    record(Violation::SecondInterpolation);
                }
            }
            if skeleton.contains(EVALUATION_CALL_NEEDLE)
                && !skeleton.contains(UNIQUE_EVALUATION_ENTRY)
            {
                record(Violation::SecondEvaluation);
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn a_call_to_the_unique_entry_is_clean() {
        let batch = sources(&[(
            "crates/yeban-mcp/src/domain/automation.rs",
            "fn f(project: &YebanProjectV1, target: &AutomationTarget, tick: u64) {\n    \
             let value = project.automation_value_at(target, tick).expect(\"x\");\n}\n",
        )]);
        assert_eq!(
            scan_second_automation_evaluations(&batch),
            Vec::<String>::new()
        );
    }

    #[test]
    fn calling_the_lane_evaluator_directly_is_flagged() {
        // 注入: 绕开唯一入口, 直接调泳道的纯计算版本。
        let batch = sources(&[(
            "crates/yeban-mcp/src/domain/rogue.rs",
            "fn f(lane: &AutomationLane, tick: u64) -> Option<f32> {\n    lane.value_at(tick)\n}\n",
        )]);
        let found = scan_second_automation_evaluations(&batch);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("[secondEvaluation]"), "{found:?}");
        assert!(found[0].contains("rogue.rs:2"), "{found:?}");
    }

    #[test]
    fn a_hand_rolled_interpolation_is_flagged() {
        // 注入: 自己算 S 曲线 —— 线性下与模型逐位相同, 只有 SCurve 上才分叉。
        let batch = sources(&[(
            "crates/yeban-mcp/src/domain/rogue.rs",
            "fn interp(low: &AutomationPoint, high: &AutomationPoint, t: f32) -> f32 {\n    \
             let u = low.curve.ease(t);\n    low.value + (high.value - low.value) * u\n}\n",
        )]);
        let found = scan_second_automation_evaluations(&batch);
        assert!(
            found
                .iter()
                .any(|line| line.contains("[secondInterpolation]")),
            "{found:?}"
        );
    }

    #[test]
    fn listing_points_in_tick_order_is_not_a_violation() {
        // 界面画折线要用它; 它不含插值, 因此**刻意不扫**。
        let batch = sources(&[(
            "crates/yeban-app/src/automation.rs",
            "fn vertices(lane: &AutomationLane) -> Vec<AutomationPoint> {\n    \
             lane.points_in_tick_order()\n}\n",
        )]);
        assert_eq!(
            scan_second_automation_evaluations(&batch),
            Vec::<String>::new()
        );
    }

    #[test]
    fn comments_and_test_regions_are_not_production_code() {
        let batch = sources(&[(
            "crates/yeban-mcp/src/domain/notes.rs",
            "//! 说明: 采样点上的顶点用 `lane.value_at(point.tick)` 逐位精确。\n\
             // 注释里提到 .ease( 与 interpolate 也不算调用\n\
             fn f() {}\n\
             #[cfg(test)]\n\
             mod tests {\n    \
                 fn t(lane: &AutomationLane) { let _ = lane.value_at(3); }\n    \
                 fn u(p: &AutomationPoint) { let _ = p.curve.ease(0.5); }\n\
             }\n",
        )]);
        assert_eq!(
            scan_second_automation_evaluations(&batch),
            Vec::<String>::new(),
            "注释与 #[cfg(test)] 之后都不算生产代码"
        );
    }

    #[test]
    fn production_region_stops_at_the_attribute_line_only() {
        // `find("#[cfg(test)]")` 会被文档注释里的同名字符串骗到 —— 这条判据钉住"整行开头"口径。
        let text = "// 文档里提到 #[cfg(test)] 这个词\nfn f() {}\n#[cfg(test)]\nmod tests {}\n";
        let region = production_region(text);
        assert!(region.contains("fn f() {}"));
        assert!(!region.contains("mod tests"));
    }

    #[test]
    fn literals_and_trailing_comments_are_stripped_before_judging() {
        // ⭐ R94 口径：**掩码等长**（按字符数）—— 只标区间、不删字符，列号也不漂。
        let cases = [
            "let x = \"value_at(\";",
            "let y = 1; // .ease( 只是注释",
            "f(\"a\\\"b\")",
            "let c = '\"'; let d = lane.value_at(tick);",
        ];
        for case in cases {
            let masked = code_without_literals(case);
            assert_eq!(
                masked.chars().count(),
                case.chars().count(),
                "掩码必须等长: {case:?} → {masked:?}"
            );
        }
        // 掩掉的是**字面量与注释**：针不得留下。
        assert!(!code_without_literals("let x = \"value_at(\";").contains("value_at("));
        assert!(!code_without_literals("let y = 1; // .ease( 只是注释").contains(".ease("));
        // 真调用必须留下（这正是要被抓到的形状）。
        assert!(code_without_literals("lane.value_at(tick)").contains("value_at("));
        // ⛔ 生命周期 `'a` 不是字符字面量，不得被掩。
        assert_eq!(
            code_without_literals("fn f<'a>(x: &'a str) -> &'a str { x }"),
            "fn f<'a>(x: &'a str) -> &'a str { x }"
        );
    }

    #[test]
    fn the_audit_source_is_clean_by_its_own_rules() {
        // 判据自己的常量定义与失败文案里含有被禁模式；这条判据钉住"它们不算调用"。
        let own = include_str!("automation_audit.rs");
        let batch = vec![(
            "crates/yeban-mcp/src/domain/automation_audit.rs".to_owned(),
            own.to_owned(),
        )];
        assert_eq!(
            scan_second_automation_evaluations(&batch),
            Vec::<String>::new()
        );
    }

    #[test]
    fn entry_name_is_the_model_symbol() {
        assert_eq!(UNIQUE_EVALUATION_ENTRY, "automation_value_at");
        assert!(EVALUATION_CALL_NEEDLE.starts_with("value_at"));
        assert_eq!(Violation::SecondEvaluation.as_str(), "secondEvaluation");
        assert_eq!(
            Violation::SecondInterpolation.as_str(),
            "secondInterpolation"
        );
        assert!(!Violation::SecondEvaluation.message().is_empty());
    }
    /// `interpolate` 那条针**真的会咬人**（不是死条目）。
    ///
    /// 第二轮注入实测：`AA-interp`（把 [`INTERPOLATION_NEEDLES`] 的第二项改成
    /// `"interpolateX"`）**全绿** —— 既有判据只喂过 `.ease(`（`low.curve.ease(t)`），
    /// 第二项从来没有被任何输入触发过。一个从没被触发过的针等于没有针：
    /// 谁把插值函数命名成 `interpolate`，审计**不会**红。
    #[test]
    fn the_interpolate_needle_really_bites() {
        // 只含 `interpolate`、**不含** `.ease(` —— 恰好把两条针分开。
        let batch = sources(&[(
            "crates/yeban-mcp/src/domain/rogue.rs",
            "fn interpolate(low: f32, high: f32, t: f32) -> f32 {\n    \
             low + (high - low) * t\n}\n",
        )]);
        let found = scan_second_automation_evaluations(&batch);
        assert_eq!(found.len(), 1, "`interpolate` 必须被抓到: {found:?}");
        assert!(found[0].contains("[secondInterpolation]"), "{found:?}");
        assert!(found[0].contains("rogue.rs:1"), "{found:?}");
        // 阴性对照: 把同一个词切掉一个字母 ⇒ 干净（证明上面红的是那个**词**）。
        let clean = sources(&[(
            "crates/yeban-mcp/src/domain/rogue.rs",
            "fn interpolat(low: f32, high: f32, t: f32) -> f32 {\n    \
             low + (high - low) * t\n}\n",
        )]);
        assert_eq!(
            scan_second_automation_evaluations(&clean),
            Vec::<String>::new()
        );
    }
}
