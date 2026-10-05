//! 键盘快捷键 + 输入法 (IME) 合成态屏蔽的**纯 Rust 状态机**。
//!
//! 规范来源 (Normative):
//! - `[UI-A11Y-001]` UI/UX 规范 §7.1「全键盘热键映射规范 (Physical Scancode Binding)」:
//!   绑定基于**物理扫描码**, 避免输入法与键盘布局切换产生键位漂移。因此本模块的输入是
//!   [`PhysicalKey`] 而不是"字符" —— 按 `Shift+2` 得到的 `@` 不该被当成 `@` 热键。
//! - `[UI-A11Y-002]` §7.2「输入法 IME 候选词保护规范」: 合成态 (`is_composing == true`) 下
//!   **必须彻底拦截 `Space`/`B`/`Z` 等全部单键快捷键的冒泡分发**。
//! - 规范 §2.2「`Tab` 键焦点隔离 (MUST)」: 焦点在主工作区画布时 `Tab` 切视图;
//!   焦点在文本输入框内时 `Tab` **严格保留**标准文本表单焦点轮转语义。
//! - `[UI-A11Y-003]` §7.3: 焦点流转遵循严密 Tab 序列 —— 所以非画布焦点下
//!   `Tab` 一律 `PassThrough`, 由焦点系统决定去哪。
//!
//! ## 为什么是纯 Rust, 不依赖 Slint
//!
//! 1. 本机纪律禁止编译 Slint, 而这条规范是最容易写错、最值得有测试的一条;
//!    把它做成零依赖状态机, 测试就能在任何机器上跑 (CI 的 `-p yeban-app` 腿)。
//! 2. Slint 侧的 `FocusScope` / `TextInput` 绑定只是**事件来源**。真正的"哪个键在当前
//!    IME 状态下应该做什么"是一个策略表, 策略表属于业务逻辑, 不属于表现层。
//!    宿主把 Slint 的按键事件翻译成 [`PhysicalKey`] + [`Modifiers`], 再问 [`InputContext::resolve`]。
//!
//! ## 三条被显式编码进来的策略决定 (规范没写的部分)
//!
//! | 情形 | 决定 | 理由 |
//! | :-- | :--- | :--- |
//! | 文本输入框聚焦 + **非**合成态, 按 `Cmd/Ctrl+Z` | `PassThrough` (给文本框做文本撤销) | 文本框里的撤销语义应当是"撤销这次输入"。规范只说了 `Tab` 与单键, 没规定组合键; 取"不劫持文本编辑"的方向。 |
//! | 文本输入框聚焦, 按 `F5`/`F6` | 仍然切视图 | 规范称 `F5`/`F6` 为"全局无冲突备选快捷键", 且输入法不消费功能键。 |
//! | 合成态, 按任何非 `F5`/`F6` 的键 | `ConsumedByIme` | 规范要求"彻底拦截"; 与其逐个键列白名单, 不如整体拦截 —— 漏掉一个键的代价是误播走带。 |
//!
//! 第二条与第三条属于对规范的解释, 已登记在 `docs/ledger/ui-shell-notes.md` 的 needs 里。

/// 键盘焦点当前落在哪一类控件上。
///
/// 只需要三类: 键盘策略只关心"是不是文本域"与"是不是主工作区画布"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// 主工作区画布 (Session 矩阵 / Arrangement 时间轴)。规范 §2.2 里 `Tab` 唯一生效的地方。
    MainCanvas,
    /// 文本输入框 (音轨重命名 / 搜索框 / 时间码数值敲入 / 歌词)。`[UI-A11Y-002]` 的防护对象。
    TextInput,
    /// 其余控件 (推子、旋钮、标签)。单键热键仍生效, 但 `Tab` 交给焦点系统。
    Other,
}

/// 修饰键状态。基于物理修饰键, 与扫描码绑定配套。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// 左/右 `Ctrl`。
    pub ctrl: bool,
    /// 左/右 `Shift`。
    pub shift: bool,
    /// 左/右 `Alt` (macOS 上为 `Option`)。
    pub alt: bool,
    /// `Cmd` (macOS) / `Super` / `Win`。
    pub meta: bool,
}

impl Modifiers {
    /// 无任何修饰键 —— 即规范里说的"单键快捷键"。
    #[must_use]
    pub const fn none() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        }
    }

    /// 仅 `Ctrl`。
    #[must_use]
    pub const fn ctrl() -> Self {
        Self {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        }
    }

    /// 仅 `Cmd`/`Super`。
    #[must_use]
    pub const fn meta() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            meta: true,
        }
    }

    /// `Ctrl` + `Shift`。
    #[must_use]
    pub const fn ctrl_shift() -> Self {
        Self {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        }
    }

    /// `Ctrl` + `Alt`。
    #[must_use]
    pub const fn ctrl_alt() -> Self {
        Self {
            ctrl: true,
            shift: false,
            alt: true,
            meta: false,
        }
    }

    /// 仅 `Shift`。
    #[must_use]
    pub const fn shift() -> Self {
        Self {
            ctrl: false,
            shift: true,
            alt: false,
            meta: false,
        }
    }

    /// 仅 `Alt`。
    #[must_use]
    pub const fn alt() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: true,
            meta: false,
        }
    }

    /// 规范表格里的 `Cmd / Ctrl` —— macOS 用 `Cmd`, 其余平台用 `Ctrl`。
    #[must_use]
    pub const fn command(self) -> bool {
        self.ctrl || self.meta
    }

    /// 一个修饰键都没按。
    #[must_use]
    pub const fn is_bare(self) -> bool {
        !self.ctrl && !self.shift && !self.alt && !self.meta
    }
}

/// 物理按键 (扫描码语义)。
///
/// 只列 `[UI-A11Y-001]` 表格里真实出现过的键, 加上 `Delete`/`Backspace`
/// (规范写在同一格里)。没有映射的键由宿主直接丢弃, 不进本状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalKey {
    /// `Space` —— 播放 / 暂停。
    Space,
    /// `Tab` —— 视图切换 (仅画布聚焦)。
    Tab,
    /// `Esc` —— 放弃 AI 建议 / 取消拖拽。
    Escape,
    /// `Enter` —— 与 `Shift` 组合才是 AI 采纳。
    Enter,
    /// `Backspace` —— 删除选区。
    Backspace,
    /// `Delete` —— 删除选区。
    Delete,
    /// `F5` —— 直达 Session 视图。
    F5,
    /// `F6` —— 直达 Arrangement 视图。
    F6,
    /// `B` —— 选择/铅笔工具快速切换。
    KeyB,
    /// `Z` —— 缩放聚焦 / `Shift+Z` 恢复全曲总览。
    KeyZ,
    /// `H` —— 与 `Cmd/Ctrl+Shift` 组合呼出时光机。
    KeyH,
    /// `D` —— 与 `Cmd/Ctrl` 组合做原位复制。
    KeyD,
    /// `M` —— 与 `Cmd/Ctrl+Alt` 组合最大化/还原底部控制台。
    KeyM,
    /// `1`–`5` —— 卷帘工具矩阵。
    Digit(u8),
    /// `[` —— 切到主线版本试听。
    BracketLeft,
    /// `]` —— 切到 AI 提案分支试听。
    BracketRight,
}

/// `[UI-NOTE-003]` 左键**单击**在该工具下的语义（规范矩阵的"左键单击"列）。
///
/// 只做**分类**：真正作用到模型的编辑要经过撤销与 MCP, 属于后续切片。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClick {
    /// 选中音符 / 点空白清除选区。
    SelectOrClear,
    /// 在吸附网格处画出音符（配合 [`crate::bridge::snap_tick`]）。
    DrawNote,
    /// 沿网格竖线切分。
    SplitNote,
    /// 选中该音符的力度柱。
    SelectVelocity,
    /// 删除光标下的音符。
    DeleteNote,
}

impl Tool {
    /// 光标样式 —— **逐字**取自规范矩阵的"光标样式"列。
    #[must_use]
    pub fn cursor(self) -> &'static str {
        match self {
            Self::Select => "default",
            Self::Pencil => "crosshair",
            Self::Knife => "col-resize",
            Self::Velocity => "ns-resize",
            Self::Eraser => "cell",
        }
    }

    /// 左键单击的语义（矩阵的"左键单击"列）。
    #[must_use]
    pub fn click(self) -> ToolClick {
        match self {
            Self::Select => ToolClick::SelectOrClear,
            Self::Pencil => ToolClick::DrawNote,
            Self::Knife => ToolClick::SplitNote,
            Self::Velocity => ToolClick::SelectVelocity,
            Self::Eraser => ToolClick::DeleteNote,
        }
    }

    /// 五个工具的**全部**取值, 顺序与规范矩阵的行顺序一致（快捷键 `1`..`5`）。
    #[must_use]
    pub fn all_in_matrix_order() -> [Self; 5] {
        [
            Self::Select,
            Self::Pencil,
            Self::Knife,
            Self::Velocity,
            Self::Eraser,
        ]
    }
}

/// `[UI-NOTE-003]` 卷帘的**选区**（选择工具的左键单击语义）。
///
/// 用 `BTreeSet` 而不是 `HashSet`：迭代顺序**确定**，判据与界面都不依赖哈希随机性（与红线 4 的取向一致）。
/// 选区是**视图状态**，不是模型改动 ⇒ 不需要撤销记录；但选中态最终要能被 MCP 读到, 那是后续切片。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Selection {
    ids: std::collections::BTreeSet<String>,
}

impl Selection {
    /// 空选区。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// 选中数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// 某个音符是否被选中。
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// 迭代（**升序**，因此确定）。
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.ids.iter()
    }

    /// 选择工具**单击一个音符**：替换选区（单击不是累加 —— 累加需要 Shift, 那是矩阵里的"辅助键"列）。
    pub fn select_only(&mut self, id: &str) {
        self.ids.clear();
        self.ids.insert(id.to_string());
    }

    /// 选择工具**单击空白**：清除选区。
    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// 追加选中且**不重复**（框选 / Shift 累加会用；本身幂等）。
    pub fn insert(&mut self, id: &str) {
        self.ids.insert(id.to_string());
    }

    /// 从选区移除（若不在其中则无变化）。
    pub fn remove(&mut self, id: &str) {
        self.ids.remove(id);
    }
}

/// 卷帘工具矩阵 (规范 §3.3 / `[UI-NOTE-003]`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// `1` 选择箭头。
    Select,
    /// `2` 铅笔。
    Pencil,
    /// `3` 剪刀。
    Knife,
    /// `4` 力度。
    Velocity,
    /// `5` 橡皮擦。
    Eraser,
}

impl Tool {
    /// 由数字键得到工具; `1`–`5` 之外返回 `None`。
    #[must_use]
    pub const fn from_digit(digit: u8) -> Option<Self> {
        match digit {
            1 => Some(Self::Select),
            2 => Some(Self::Pencil),
            3 => Some(Self::Knife),
            4 => Some(Self::Velocity),
            5 => Some(Self::Eraser),
            _ => None,
        }
    }
}

/// 双视图 (`Session` 触发矩阵 / `Arrangement` 线性编曲)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Session 触发矩阵。
    Session,
    /// Arrangement 线性编曲时间轴。
    Arrangement,
}

/// 一次按键解析出的**领域动作**。
///
/// 刻意不是"直接改 UI 状态": 同一个动作在不同上下文 (例如正在播放 vs 暂停) 的结果不同,
/// 那属于宿主/引擎的职责。本状态机只回答"这个键在当前焦点与 IME 状态下代表哪个动作"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `Space`: 播放 / 暂停切换 (暂停时播放头回起始点)。
    PlayPause,
    /// `Shift+Space`: 从当前光标停留处继续播放。
    ResumeFromCursor,
    /// `Tab` (仅画布): 在两个视图之间切换。
    ToggleView,
    /// `F5`/`F6`/`Alt+1`/`Alt+2`: 直达某个视图。
    ShowView(View),
    /// `Cmd/Ctrl+Z`: 基于领域操作日志逆向回滚。
    Undo,
    /// `Cmd/Ctrl+Shift+Z`: 重做。
    Redo,
    /// `Cmd/Ctrl+Shift+H`: 呼出全屏 DAG 时光机。
    OpenTimeMachine,
    /// `Cmd/Ctrl+D`: 原位智能复制。
    Duplicate,
    /// `Delete`/`Backspace`: 删除选中的音符 / 选区 / 音轨。
    DeleteSelection,
    /// `B`: 在箭头与铅笔之间快速切换。
    TogglePencilTool,
    /// `Cmd/Ctrl+Alt+B`: 展开 / 收起左侧资源抽屉。
    ToggleSidebar,
    /// `Z`: 把选区撑满视口。
    ZoomToSelection,
    /// `Shift+Z`: 恢复全曲总览缩放。
    ZoomToFit,
    /// `Cmd/Ctrl+Alt+M`: 最大化 / 还原底部控制台。
    ToggleConsoleMaximize,
    /// `1`–`5`: 切到某个卷帘工具。
    SelectTool(Tool),
    /// `Shift+Enter`: 原子采纳当前轨道浮现的 AI 建议。
    AcceptAiSuggestion,
    /// `Esc`: 放弃 AI 建议, 或取消进行中的拖拽手势。
    Cancel,
    /// `[`: 试听主线版本。
    AuditionMain,
    /// `]`: 试听 AI 提案分支。
    AuditionProposal,
}

/// 一次按键的处置结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// 命中一条规范快捷键。
    Action(Action),
    /// 不归 DAW 管: 交给焦点系统 / 文本控件 (例如文本框里的 `Tab` 焦点轮转)。
    PassThrough,
    /// `[UI-A11Y-002]`: 被输入法合成态吞掉, **绝不允许**冒泡成 DAW 快捷键。
    ConsumedByIme,
}

/// 键盘焦点 + 输入法状态的组合。
///
/// 宿主在每个事件到来前更新它 (焦点变化、`is_composing` 变化), 然后调用
/// [`InputContext::resolve`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputContext {
    focus: Focus,
    composing: bool,
}

impl InputContext {
    /// 创建一个"画布聚焦、非合成态"的上下文 —— 启动时的状态。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            focus: Focus::MainCanvas,
            composing: false,
        }
    }

    /// 当前焦点分类。
    #[must_use]
    pub const fn focus(&self) -> Focus {
        self.focus
    }

    /// 当前是否处于输入法合成态。
    #[must_use]
    pub const fn is_composing(&self) -> bool {
        self.composing
    }

    /// 切换焦点。焦点离开文本域时**自动结束合成态** —— 焦点都不在文本框上了,
    /// 还留着 `composing = true` 只会把后续所有单键热键永久吞掉。
    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        if focus != Focus::TextInput {
            self.composing = false;
        }
    }

    /// `is_composing` 置位 (输入法开始合成)。
    pub fn begin_composition(&mut self) {
        self.composing = true;
    }

    /// `is_composing` 清零 (候选词上屏或取消)。
    pub fn end_composition(&mut self) {
        self.composing = false;
    }

    /// 解析一次按键。
    ///
    /// 判定顺序 (顺序本身就是规范优先级):
    /// 1. `F5`/`F6` —— 全局直达, 连合成态都放行;
    /// 2. 文本域策略 —— 合成态整体拦截, 非合成态整体放行;
    /// 3. `Cmd`/`Ctrl` 组合;
    /// 4. `Alt` 组合 (备选视图切换);
    /// 5. `Shift` 组合;
    /// 6. 无修饰单键。
    #[must_use]
    pub fn resolve(&self, key: PhysicalKey, modifiers: Modifiers) -> Resolution {
        use PhysicalKey as Key;

        // 1. F5 / F6: 「全局无冲突备选快捷键」。
        match key {
            Key::F5 => return Resolution::Action(Action::ShowView(View::Session)),
            Key::F6 => return Resolution::Action(Action::ShowView(View::Arrangement)),
            _ => {}
        }

        // 2. 文本输入框: 规范 §7.2 与 §2.2 的硬约束。
        if self.focus == Focus::TextInput {
            return if self.composing {
                Resolution::ConsumedByIme
            } else {
                // `Tab` 在这里同样是 PassThrough —— 标准文本表单焦点轮转。
                Resolution::PassThrough
            };
        }

        // 3. Cmd / Ctrl 组合。
        if modifiers.command() {
            return match (key, modifiers.shift, modifiers.alt) {
                (Key::KeyZ, false, false) => Resolution::Action(Action::Undo),
                (Key::KeyZ, true, false) => Resolution::Action(Action::Redo),
                (Key::KeyH, true, false) => Resolution::Action(Action::OpenTimeMachine),
                (Key::KeyD, false, false) => Resolution::Action(Action::Duplicate),
                (Key::KeyB, false, true) => Resolution::Action(Action::ToggleSidebar),
                (Key::KeyM, false, true) => Resolution::Action(Action::ToggleConsoleMaximize),
                _ => Resolution::PassThrough,
            };
        }

        // 4. Alt 组合: 规范 §2.2 的 `Alt + 1` / `Alt + 2` 备选视图切换。
        if modifiers.alt {
            return match key {
                Key::Digit(1) => Resolution::Action(Action::ShowView(View::Session)),
                Key::Digit(2) => Resolution::Action(Action::ShowView(View::Arrangement)),
                _ => Resolution::PassThrough,
            };
        }

        // 5. Shift 组合。
        if modifiers.shift {
            return match key {
                Key::Space => Resolution::Action(Action::ResumeFromCursor),
                Key::Enter => Resolution::Action(Action::AcceptAiSuggestion),
                Key::KeyZ => Resolution::Action(Action::ZoomToFit),
                _ => Resolution::PassThrough,
            };
        }

        // 6. 无修饰单键。
        match key {
            // `Tab` 只在主工作区画布聚焦时切视图 [规范 §2.2 MUST]。
            // 其余焦点 (推子 / 旋钮 / 标签) 一律放行, 交给焦点系统。
            Key::Tab => {
                if self.focus == Focus::MainCanvas {
                    Resolution::Action(Action::ToggleView)
                } else {
                    Resolution::PassThrough
                }
            }
            Key::Space => Resolution::Action(Action::PlayPause),
            Key::Escape => Resolution::Action(Action::Cancel),
            Key::Delete | Key::Backspace => Resolution::Action(Action::DeleteSelection),
            Key::KeyB => Resolution::Action(Action::TogglePencilTool),
            Key::KeyZ => Resolution::Action(Action::ZoomToSelection),
            Key::BracketLeft => Resolution::Action(Action::AuditionMain),
            Key::BracketRight => Resolution::Action(Action::AuditionProposal),
            Key::Digit(digit) => match Tool::from_digit(digit) {
                Some(tool) => Resolution::Action(Action::SelectTool(tool)),
                None => Resolution::PassThrough,
            },
            _ => Resolution::PassThrough,
        }
    }
}

impl Default for InputContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use Action::{
        AcceptAiSuggestion, AuditionMain, AuditionProposal, Cancel, DeleteSelection, Duplicate,
        OpenTimeMachine, PlayPause, Redo, ResumeFromCursor, SelectTool, ShowView,
        ToggleConsoleMaximize, TogglePencilTool, ToggleSidebar, ToggleView, Undo, ZoomToFit,
        ZoomToSelection,
    };
    use Focus::{MainCanvas, Other, TextInput};
    use PhysicalKey::{
        Backspace, BracketLeft, BracketRight, Delete, Digit, Enter, Escape, F5, F6, KeyB, KeyD,
        KeyH, KeyM, KeyZ, Space, Tab,
    };
    use Resolution::{Action as Act, ConsumedByIme, PassThrough};

    fn canvas() -> InputContext {
        let mut ctx = InputContext::new();
        ctx.set_focus(MainCanvas);
        ctx
    }

    fn text_field() -> InputContext {
        let mut ctx = InputContext::new();
        ctx.set_focus(TextInput);
        ctx
    }

    // ---------------------------------------------------------------- 判据 1
    /// 判据 1 (SKILL 规则 2): **IME 合成态下 `Space` 不得触发播放**。
    ///
    /// 曾经在把 `if self.composing` 分支删掉之后变红 (得到 `Action(PlayPause)`) ——
    /// 那正是中文输入法选词按空格会误播走带的真实事故。
    #[test]
    fn space_while_composing_never_plays() {
        let mut ctx = text_field();
        ctx.begin_composition();
        assert!(ctx.is_composing());
        assert_eq!(ctx.resolve(Space, Modifiers::none()), ConsumedByIme);
        assert_ne!(ctx.resolve(Space, Modifiers::none()), Act(PlayPause));
    }

    /// 合成态下**所有**单键热键都被吞掉, 不只是 `Space`。
    ///
    /// 曾经在把 `ConsumedByIme` 改成只对 `Space` 生效后变红 (`KeyB` 漏成了动作)。
    #[test]
    fn composing_swallows_every_bare_shortcut() {
        let mut ctx = text_field();
        ctx.begin_composition();
        for key in [
            Space,
            KeyB,
            KeyZ,
            Digit(2),
            BracketLeft,
            BracketRight,
            Escape,
            Enter,
            Tab,
        ] {
            assert_eq!(
                ctx.resolve(key, Modifiers::none()),
                ConsumedByIme,
                "合成态下 {key:?} 必须被输入法吞掉 [UI-A11Y-002]"
            );
        }
        // `Shift+Space` 也不行 —— 它同样是单键 + 修饰键的走带动作。
        assert_eq!(ctx.resolve(Space, Modifiers::shift()), ConsumedByIme);
    }

    // ---------------------------------------------------------------- 判据 2
    /// 判据 2 (SKILL 规则 2): **`Tab` 在文本输入框内不切视图**。
    ///
    /// 曾经在把 `Focus::TextInput` 分支删掉后变红 (得到 `Action(ToggleView)`)。
    #[test]
    fn tab_in_text_input_does_not_switch_view() {
        let ctx = text_field();
        assert_eq!(ctx.resolve(Tab, Modifiers::none()), PassThrough);
        assert_ne!(ctx.resolve(Tab, Modifiers::none()), Act(ToggleView));
    }

    /// 非合成态的文本域里, 单键热键同样被屏蔽 (规范 §7.1 的"文本输入时屏蔽单键热键")。
    #[test]
    fn bare_shortcuts_are_blocked_in_text_input_even_without_composition() {
        let ctx = text_field();
        assert!(!ctx.is_composing());
        assert_eq!(ctx.resolve(Space, Modifiers::none()), PassThrough);
        assert_eq!(ctx.resolve(KeyB, Modifiers::none()), PassThrough);
        assert_eq!(ctx.resolve(Digit(2), Modifiers::none()), PassThrough);
    }

    /// 文本域里的 `Cmd/Ctrl+Z` 归文本框 (文本撤销), 不劫持成 DAW 的历史回滚。
    /// 这是规范没写、由本实现显式决定的策略, 见模块文档与 notes 的 needs。
    #[test]
    fn command_z_in_text_input_belongs_to_the_text_field() {
        let ctx = text_field();
        assert_eq!(ctx.resolve(KeyZ, Modifiers::meta()), PassThrough);
        assert_eq!(ctx.resolve(KeyZ, Modifiers::ctrl_shift()), PassThrough);
    }

    /// `F5`/`F6` 即便在合成态也直达视图 —— 输入法不消费功能键。
    #[test]
    fn f5_f6_stay_global_even_while_composing() {
        let mut ctx = text_field();
        ctx.begin_composition();
        assert_eq!(
            ctx.resolve(F5, Modifiers::none()),
            Act(ShowView(View::Session))
        );
        assert_eq!(
            ctx.resolve(F6, Modifiers::none()),
            Act(ShowView(View::Arrangement))
        );
    }

    // ---------------------------------------------------------------- 画布焦点
    /// `Tab` 在画布聚焦时切视图, 在其它控件上放行 (规范 §2.2 的 MUST)。
    #[test]
    fn tab_switches_view_only_on_the_main_canvas() {
        assert_eq!(canvas().resolve(Tab, Modifiers::none()), Act(ToggleView));

        let mut other = InputContext::new();
        other.set_focus(Other);
        assert_eq!(other.resolve(Tab, Modifiers::none()), PassThrough);
    }

    #[test]
    fn space_plays_on_canvas_and_on_other_widgets() {
        assert_eq!(canvas().resolve(Space, Modifiers::none()), Act(PlayPause));

        let mut other = InputContext::new();
        other.set_focus(Other);
        assert_eq!(other.resolve(Space, Modifiers::none()), Act(PlayPause));
        assert_eq!(
            other.resolve(Space, Modifiers::shift()),
            Act(ResumeFromCursor)
        );
    }

    // ---------------------------------------------------------------- 历史
    #[test]
    fn undo_requires_a_command_modifier() {
        assert_eq!(canvas().resolve(KeyZ, Modifiers::meta()), Act(Undo));
        assert_eq!(canvas().resolve(KeyZ, Modifiers::ctrl()), Act(Undo));
        // 裸 `Z` 是缩放聚焦, 不是撤销 —— 两条动作不能混。
        assert_eq!(
            canvas().resolve(KeyZ, Modifiers::none()),
            Act(ZoomToSelection)
        );
        // 裸 `Shift+Z` 是全曲总览
        assert_eq!(canvas().resolve(KeyZ, Modifiers::shift()), Act(ZoomToFit));
    }

    #[test]
    fn command_shift_z_is_redo() {
        assert_eq!(canvas().resolve(KeyZ, Modifiers::ctrl_shift()), Act(Redo));
    }

    #[test]
    fn time_machine_and_duplicate_need_their_exact_combo() {
        let mut mods = Modifiers::meta();
        mods.shift = true;
        assert_eq!(canvas().resolve(KeyH, mods), Act(OpenTimeMachine));
        // 少了 Shift 就不是时光机
        assert_eq!(canvas().resolve(KeyH, Modifiers::meta()), PassThrough);
        assert_eq!(canvas().resolve(KeyD, Modifiers::meta()), Act(Duplicate));
        // 裸 `D` 不是复制 (规范没有裸 D)
        assert_eq!(canvas().resolve(KeyD, Modifiers::none()), PassThrough);
    }

    #[test]
    fn sidebar_and_console_toggles_are_decoupled_from_bare_letters() {
        assert_eq!(
            canvas().resolve(KeyB, Modifiers::ctrl_alt()),
            Act(ToggleSidebar)
        );
        assert_eq!(
            canvas().resolve(KeyM, Modifiers::ctrl_alt()),
            Act(ToggleConsoleMaximize)
        );
        // 与单键解耦的关键: 裸 `B` 仍然是笔刷切换
        assert_eq!(
            canvas().resolve(KeyB, Modifiers::none()),
            Act(TogglePencilTool)
        );
    }

    // ---------------------------------------------------------------- 工具矩阵
    #[test]
    fn selection_replaces_on_click_and_clears_on_empty() {
        // 判据（规范"选择工具 · 左键单击"列）: 单击音符 ⇒ 选区**只有**它（替换, 不是累加）;
        // 单击空白 ⇒ **清空**; 重复选中不出重复项; 顺序**确定**（BTreeSet ⇒ 升序）。
        let mut sel = Selection::new();
        assert!(sel.is_empty());
        sel.select_only("b");
        assert_eq!(sel.len(), 1);
        assert!(sel.contains("b"));
        sel.select_only("a");
        assert_eq!(sel.len(), 1, "单击是**替换**, 不是累加");
        assert!(
            sel.contains("a") && !sel.contains("b"),
            "旧选中必须被替换掉"
        );
        sel.insert("c");
        sel.insert("a");
        assert_eq!(sel.len(), 2, "重复插入不得产生重复项");
        let ids: Vec<&String> = sel.iter().collect();
        assert_eq!(ids, vec!["a", "c"], "迭代顺序必须确定（升序）");
        sel.remove("a");
        assert_eq!(sel.len(), 1);
        sel.remove("zzz");
        assert_eq!(sel.len(), 1, "移除不存在的 id 不得改变选区");
        sel.clear();
        assert!(sel.is_empty(), "单击空白必须清空选区");
    }

    #[test]
    fn tool_cursor_and_click_match_the_spec_matrix_row_for_row() {
        // 判据: 光标字符串与"左键单击"语义**逐行**对齐规范 `[UI-NOTE-003]` 的矩阵,
        // 且与**既有**快捷键映射（`from_digit`）指向同一行 —— 两处若分叉, 用户按键得到的工具
        // 会与光标/点击语义不符, 而那种错看起来只是"工具怪怪的"。
        let expected = [
            (Tool::Select, "default", ToolClick::SelectOrClear),
            (Tool::Pencil, "crosshair", ToolClick::DrawNote),
            (Tool::Knife, "col-resize", ToolClick::SplitNote),
            (Tool::Velocity, "ns-resize", ToolClick::SelectVelocity),
            (Tool::Eraser, "cell", ToolClick::DeleteNote),
        ];
        let matrix = Tool::all_in_matrix_order();
        assert_eq!(matrix.len(), expected.len(), "工具数必须与矩阵行数一致");
        for (index, (tool, cursor, click)) in expected.iter().enumerate() {
            assert_eq!(matrix[index], *tool, "矩阵第 {} 行的工具不对", index + 1);
            assert_eq!(tool.cursor(), *cursor, "{tool:?} 的光标与规范不一致");
            assert_eq!(tool.click(), *click, "{tool:?} 的单击语义与规范不一致");
            // `from_digit` 收的是**数字值**（1..=5），不是 ASCII 字节 —— 我第一次传 `b'1'`（49）得到 None，判据抓到了。
            let digit = u8::try_from(index).expect("五个工具") + 1;
            assert_eq!(
                Tool::from_digit(digit),
                Some(*tool),
                "快捷键 {} 与矩阵第 {} 行不是同一个工具",
                digit,
                index + 1
            );
        }
        // 光标互不相同 —— 否则用户无法从光标分辨模式。
        let mut cursors: Vec<&str> = matrix.iter().map(|tool| tool.cursor()).collect();
        cursors.sort_unstable();
        cursors.dedup();
        assert_eq!(cursors.len(), 5, "五种工具的光标必须互不相同");
    }

    #[test]
    fn digits_one_to_five_select_the_five_tools() {
        let expected = [
            Tool::Select,
            Tool::Pencil,
            Tool::Knife,
            Tool::Velocity,
            Tool::Eraser,
        ];
        for (offset, tool) in expected.into_iter().enumerate() {
            let digit = u8::try_from(offset).expect("索引 0..5 一定放得进 u8") + 1;
            assert_eq!(
                canvas().resolve(Digit(digit), Modifiers::none()),
                Act(SelectTool(tool))
            );
        }
        // 0 与 6..9 没有绑定
        for digit in [0_u8, 6, 7, 8, 9] {
            assert_eq!(
                canvas().resolve(Digit(digit), Modifiers::none()),
                PassThrough
            );
        }
    }

    #[test]
    fn digits_are_swallowed_in_a_text_field() {
        let ctx = text_field();
        assert_eq!(ctx.resolve(Digit(1), Modifiers::none()), PassThrough);
        let mut composing = text_field();
        composing.begin_composition();
        assert_eq!(
            composing.resolve(Digit(1), Modifiers::none()),
            ConsumedByIme
        );
    }

    // ---------------------------------------------------------------- A/B 盲听
    #[test]
    fn brackets_drive_a_b_audition() {
        assert_eq!(
            canvas().resolve(BracketLeft, Modifiers::none()),
            Act(AuditionMain)
        );
        assert_eq!(
            canvas().resolve(BracketRight, Modifiers::none()),
            Act(AuditionProposal)
        );
    }

    // ---------------------------------------------------------------- AI / 取消
    #[test]
    fn shift_enter_accepts_and_escape_cancels() {
        assert_eq!(
            canvas().resolve(Enter, Modifiers::shift()),
            Act(AcceptAiSuggestion)
        );
        assert_eq!(canvas().resolve(Escape, Modifiers::none()), Act(Cancel));
        // 裸 Enter 不采纳 (避免误固化 AI 建议)
        assert_eq!(canvas().resolve(Enter, Modifiers::none()), PassThrough);
    }

    #[test]
    fn delete_and_backspace_are_equivalent() {
        assert_eq!(
            canvas().resolve(Delete, Modifiers::none()),
            Act(DeleteSelection)
        );
        assert_eq!(
            canvas().resolve(Backspace, Modifiers::none()),
            Act(DeleteSelection)
        );
    }

    // ---------------------------------------------------------------- 备选视图
    #[test]
    fn alt_digits_are_the_conflict_free_view_switch() {
        assert_eq!(
            canvas().resolve(Digit(1), Modifiers::alt()),
            Act(ShowView(View::Session))
        );
        assert_eq!(
            canvas().resolve(Digit(2), Modifiers::alt()),
            Act(ShowView(View::Arrangement))
        );
        assert_eq!(canvas().resolve(Digit(3), Modifiers::alt()), PassThrough);
    }

    // ---------------------------------------------------------------- 焦点转移
    #[test]
    fn leaving_the_text_field_ends_composition() {
        let mut ctx = text_field();
        ctx.begin_composition();
        assert!(ctx.is_composing());
        ctx.set_focus(MainCanvas);
        assert!(
            !ctx.is_composing(),
            "焦点离开文本域后合成态必须结束, 否则热键会被永久吞掉"
        );
        assert_eq!(ctx.resolve(Space, Modifiers::none()), Act(PlayPause));
    }

    #[test]
    fn composition_can_be_ended_explicitly() {
        let mut ctx = text_field();
        ctx.begin_composition();
        ctx.end_composition();
        assert!(!ctx.is_composing());
        // 合成结束后, 文本域仍然是文本域: 单键依旧屏蔽。
        assert_eq!(ctx.resolve(Space, Modifiers::none()), PassThrough);
    }

    #[test]
    fn default_context_is_canvas_and_not_composing() {
        let ctx = InputContext::default();
        assert_eq!(ctx.focus(), MainCanvas);
        assert!(!ctx.is_composing());
        assert_eq!(ctx.resolve(Space, Modifiers::none()), Act(PlayPause));
    }

    /// 修饰键判定本身也要有判据: `command()` 必须覆盖 `Ctrl` 与 `Cmd` 两者,
    /// `is_bare()` 必须对任何修饰键返回 false。
    #[test]
    fn modifier_helpers_are_consistent() {
        assert!(Modifiers::ctrl().command());
        assert!(Modifiers::meta().command());
        assert!(!Modifiers::shift().command());
        assert!(Modifiers::none().is_bare());
        for mods in [
            Modifiers::ctrl(),
            Modifiers::meta(),
            Modifiers::shift(),
            Modifiers::alt(),
            Modifiers::ctrl_shift(),
            Modifiers::ctrl_alt(),
        ] {
            assert!(!mods.is_bare());
        }
    }
}
