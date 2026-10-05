//! **IME 合成态（`is_composing`）** 在 UI 控制面上的可观测位 —— `[UI-A11Y-002]`。
//!
//! ## 规范原文（UI/UX §7.2，逐字）
//!
//! > `[UI-A11Y-002]` **输入法候选词防护 (IME Composition Guard)**：
//! > 1. **合成态检测**：在所有文本输入框（音轨命名、歌词标注、标记备注、参数敲入）内，
//! >    Slint 控件层必须严格监听输入法状态标志 `is_composing`；
//! > 2. **单键快捷键完全屏蔽 (MUST)**：当 `is_composing == true` 时，**彻底拦截并屏蔽
//! >    `Space`（空格走带）、`B`（笔刷切换）、`Z`（缩放）等全部单键快捷键的冒泡分发**……
//!
//! ## 真实载体（**不是**影子变量）
//!
//! 这个位的载体是 `yeban-app` 侧**真的**那个 IME 状态机：`yeban_app::input::InputContext`
//! （`crates/yeban-app/src/input.rs:290`）的 `composing` 字段 —— 也就是
//! `InputContext::resolve` 用来判定 `Resolution::ConsumedByIme` 的**同一个**字段
//! （`crates/yeban-app/src/input.rs:313,319,327,332,357-364`）。
//!
//! 接线在 `crates/yeban-app/src/live_surface.rs`：`LiveAdminSurface` 持
//! `Rc<RefCell<InputContext>>`，**同一份**状态同时喂给
//! [`crate::surface::UiSurface::ime_state`]（观测）与
//! [`crate::surface::UiSurface::preview_effect`]（"这一键会被怎么处置"）。
//! 本模块只定义**词表**与**只读读数**，不持有状态 —— 因此它零 Slint、可在本机真跑。
//!
//! ## 线格式名字的映射（一处，可判定）
//!
//! | 出处 | 名字 |
//! | :--- | :--- |
//! | 规范 §7.2 原文（Rust 侧字段） | [`IME_SPEC_FIELD`] = `is_composing` |
//! | UI 控制面线格式（camelCase，与 `ui/*` 其余参数同风格） | [`IME_FIELD`] = `isComposing` |
//!
//! 两者的对应关系由判据 `ime_field_is_the_wire_form_of_the_spec_field` 机械钉住
//! （`snake_case → camelCase` 的推导，而不是"我记得我改过"）。
//!
//! ## 它怎么被观测到
//!
//! 走**既有的只读方法** `ui/property`（不新增方法：`methods.rs` 的 14 条方法是
//! `scripts/gates/check_feature_alignment.py` 与三方对齐矩阵的点名对象，新增一条会让
//! 那份表立刻不一致）。调用形状：
//!
//! ```text
//! ui/property {"elementId": "transport-bpm-field", "name": "isComposing"}
//!   -> {"id":"transport-bpm-field","name":"isComposing","value":false,
//!       "focus":"main-canvas","specId":"UI-A11Y-002"}
//! ```
//!
//! `elementId` 仍然必须**真的在控件树里**（§12.2：只能按语义 ID 寻址，找不到就是找不到）；
//! `value` 是**原生 JSON 布尔**（不是字符串）—— 它的载体是进程内的 IME 状态机，
//! 不是 Slint 的响应式属性，因此没有"UI 框架类型泄漏"的问题，也不该让 AI 去解析 `"false"`。

/// `[UI-A11Y-002]` 规范里点名的字段名（**逐字**：`is_composing`）。
pub const IME_SPEC_FIELD: &str = "is_composing";

/// 线格式字段名（camelCase）：`ui/property` 的 `name` 取值。
pub const IME_FIELD: &str = "isComposing";

/// `ui/property` 的 `name` 描述里要列出的虚拟属性（能力发现用）。
pub const VIRTUAL_PROPERTIES: [&str; 1] = [IME_FIELD];

/// 本模块挂的规范 ID（判据与文档引用）。
pub const SPEC_ID: &str = "UI-A11Y-002";

/// 一次注入按键的处置：**被输入法吞掉**（`Resolution::ConsumedByIme`，§7.2 的 MUST）。
pub const RESOLUTION_CONSUMED_BY_IME: &str = "consumed-by-ime";
/// 一次注入按键的处置：命中一条规范快捷键（`Resolution::Action`）⇒ 会触发 DAW 动作。
pub const RESOLUTION_ACTION: &str = "action";
/// 一次注入按键的处置：不归 DAW 管（`Resolution::PassThrough`，或根本不在
/// `[UI-A11Y-001]` 的扫描码表里）—— 窗口仍然会收到它，只是热键表对它没有意见。
pub const RESOLUTION_PASS_THROUGH: &str = "pass-through";

/// 全部处置取值（判据用它做"词表闭合"）。
///
/// **为什么这段词表住在本 crate**：`ui/dispatch_key_press` 的 `dryRun` 预览会报
/// `preview.effect.resolution`，而报它的人可能是**任何一个**执行面（真执行面
/// `LiveAdminSurface` 用 `InputContext::resolve`；零 Slint 假面只能给出两档）。
/// 取值若各自发明，同一个键在两个执行面上就会有两个名字 —— 那正是"同一个词必须
/// 同一个意思"要禁止的。因此词表只有这一份，两侧都引用常量。
pub const RESOLUTIONS: [&str; 3] = [
    RESOLUTION_CONSUMED_BY_IME,
    RESOLUTION_ACTION,
    RESOLUTION_PASS_THROUGH,
];

/// `is_composing` 是不是这个属性名。
#[must_use]
pub fn is_ime_property(name: &str) -> bool {
    name == IME_FIELD
}

/// IME 合成态的**只读读数**（执行面交给控制面的事实）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImeState {
    /// `is_composing`：输入法是否正在合成候选词。
    pub composing: bool,
    /// 当前键盘焦点落在哪一类控件上（`main-canvas` / `text-input` / `other`）。
    ///
    /// 为什么一起报：`[UI-A11Y-002]` 的防护只在**文本输入框**里有意义，而
    /// `InputContext::set_focus` 会在焦点离开文本域时**自动结束合成态**
    /// （`crates/yeban-app/src/input.rs:319-324`）。只报一个裸布尔，调用方无从判断
    /// "合成态为假"是"没在输入"还是"焦点跑了"。
    pub focus: ImeFocus,
}

impl ImeState {
    /// 焦点字面名（进 JSON）。
    #[must_use]
    pub const fn focus_name(&self) -> &'static str {
        self.focus.as_str()
    }
}

/// 焦点分类（与 `yeban_app::input::Focus` 一一对应；本模块不依赖那个 crate，见文件头）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImeFocus {
    /// 主工作区画布（`Tab` 唯一生效的地方）。
    #[default]
    MainCanvas,
    /// 文本输入框（`[UI-A11Y-002]` 的防护对象）。
    TextInput,
    /// 其余控件（推子 / 旋钮 / 标签）。
    Other,
}

impl ImeFocus {
    /// 线格式字面名（kebab-case，与 `ui/tree` 的角色名同风格）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MainCanvas => "main-canvas",
            Self::TextInput => "text-input",
            Self::Other => "other",
        }
    }
}

/// `snake_case` → `camelCase`（只处理本项目用得上的形状：单下划线、无数字边界）。
///
/// 写成一个函数而不是在判据里手写一对字面量：这样"线格式名是规范名的 camelCase"
/// 是**算出来的**，规范改名（或线格式改名）时判据会红。
#[must_use]
pub fn camel_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper_next = false;
    for ch in snake.chars() {
        if ch == '_' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 1: 线格式名必须是规范名的 camelCase（**算出来**的，不是手写的对子）。
    ///
    /// 注入验证（把 [`IME_FIELD`] 改成 `"is_composing"` 或 `"isComposingX"`）会让本判据变红。
    #[test]
    fn ime_field_is_the_wire_form_of_the_spec_field() {
        assert_eq!(IME_SPEC_FIELD, "is_composing");
        assert_eq!(camel_case(IME_SPEC_FIELD), IME_FIELD);
        assert_eq!(IME_FIELD, "isComposing");
        assert!(is_ime_property(IME_FIELD));
        assert!(!is_ime_property(IME_SPEC_FIELD), "线格式不是规范原文");
        assert!(!is_ime_property("iscomposing"), "大小写敏感");
        assert!(!is_ime_property("isComposing "), "不做 trim: 拼错就是拼错");
        assert_eq!(VIRTUAL_PROPERTIES, [IME_FIELD]);
    }

    /// 判据 2: `camel_case` 的形状（用于把"两个名字的对应关系"变成机械事实）。
    #[test]
    fn camel_case_covers_the_shapes_we_use() {
        assert_eq!(camel_case("is_composing"), "isComposing");
        assert_eq!(camel_case("already"), "already");
        assert_eq!(camel_case("a_b_c"), "aBC");
        assert_eq!(camel_case(""), "");
        assert_eq!(camel_case("x_"), "x");
    }

    /// 判据 3: 焦点字面名与 `ImeFocus` 的三个变体一一对应（穷举，不抽样）。
    #[test]
    fn focus_names_cover_every_variant() {
        let all = [ImeFocus::MainCanvas, ImeFocus::TextInput, ImeFocus::Other];
        let mut names: Vec<&str> = all.iter().copied().map(ImeFocus::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["main-canvas", "other", "text-input"]);
        assert_eq!(ImeFocus::default(), ImeFocus::MainCanvas);
        for (focus, name) in all.into_iter().zip(["main-canvas", "text-input", "other"]) {
            assert_eq!(focus.as_str(), name);
            assert_eq!(
                ImeState {
                    composing: true,
                    focus
                }
                .focus_name(),
                name
            );
        }
    }

    /// 判据 4: **按键处置的词表是闭合的、唯一的**。
    ///
    /// 两个执行面（真执行面 / 零 Slint 假面）都会报 `preview.effect.resolution`；
    /// 各自发明取值会让同一个键有两个名字。因此词表只有这一份，取值集合恰好是那三个。
    #[test]
    fn key_resolution_vocabulary_is_closed_and_unique() {
        assert_eq!(
            RESOLUTIONS,
            [
                RESOLUTION_CONSUMED_BY_IME,
                RESOLUTION_ACTION,
                RESOLUTION_PASS_THROUGH
            ]
        );
        let unique: std::collections::BTreeSet<&str> = RESOLUTIONS.iter().copied().collect();
        assert_eq!(unique.len(), RESOLUTIONS.len(), "取值不得重复");
        for value in RESOLUTIONS {
            assert!(!value.is_empty());
            assert!(
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-'),
                "线格式取值必须是小写 kebab-case: {value}"
            );
        }
        // §7.2 的 MUST 用一个专门的取值表达，绝不与"普通动作"混用。
        assert_eq!(RESOLUTION_CONSUMED_BY_IME, "consumed-by-ime");
        assert_ne!(RESOLUTION_CONSUMED_BY_IME, RESOLUTION_ACTION);
    }
}
