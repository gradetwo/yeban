//! 键位层：**数据表与状态机分离**。
//!
//! 口径（Wang 2026-11-18 定）：凡 GUI 与 TUI 都有的动作（走带、保存、打开、撤销、
//! 静音、独奏、混音微调、选轨等），TUI 一律沿用 GUI 的键 —— 机械事实源是
//! `yeban_app::cli::shortcut_rows()`（GUI 帮助速查与 `--print-shortcuts` 同一张表，
//! `help.rs` 明令 ⛔ 不手抄第二份键位表），解析走 `yeban_app::input::InputContext::resolve`，
//! 本模块**不**复制那张表。Vi 模态键（hjkl、gg/G、计数前缀、模式切换）只加在
//! 导航与模式层，由本模块的状态机管辖。
//!
//! # 映射表（动作｜GUI 键｜TUI 键｜TUI 状态）
//!
//! 完整版见 crate 的 `README.md`；本表是随代码走的摘要（状态：已接 / 未接 v1）。
//!
//! | 动作 | GUI 键 | TUI 键 | 状态 |
//! | --- | --- | --- | --- |
//! | 播放/暂停 | `Space` | `Space`（同） | 已接 |
//! | 音符全选 | `Cmd/Ctrl+A` | `Ctrl+A`（同，仅音符视图当前片段） | 已接（T-FIX-801） |
//! | 回到起点 | `Enter` | `Enter`（同） | 已接 |
//! | 保存工程 | `Cmd/Ctrl+S` | `Ctrl+S`（同）＋ `:w` | 已接 |
//! | 打开工程 | `Cmd/Ctrl+O` | `Ctrl+O` 提示＋ `:open <path>` | 已接（命令行形态） |
//! | 撤销 / 重做 | `Cmd/Ctrl+Z` / `Cmd/Ctrl+Shift+Z` | 同键＋ Vi `u` / `Ctrl+R` | 已接（二期，走 `UndoPort`） |
//! | 选中轨静音 | `M` | `M`（同） | 已接 |
//! | 选中轨独奏 | `S` | `S`（同） | 已接 |
//! | 选中轨音量 ±1 dB | `Cmd/Ctrl+Alt+↑/↓` | `Ctrl+Alt+↑/↓`（同） | 已接 |
//! | 选中轨声相 ±0.1 | `Cmd/Ctrl+Alt+←/→` | `Ctrl+Alt+←/→`（同） | 已接 |
//! | 选上一/下一轨 | `Alt+↑/↓` | `Alt+↑/↓`（同），另加 Vi `j/k` | 已接 |
//! | 删除轨道 | GUI `track-delete`（轨头/菜单） | 轨道焦点 `Delete` / `:track delete` | 已接（T-FIX-799，`bridge::track_delete_ops_on`） |
//! | 视图切换 | `Tab` | `Tab`（同）→ TUI 里循环切换窗格焦点 | 已接（形态适配） |
//! | 混音台 / 编辑器 | `X` / `E`·`P` | 同键 → 切焦点到检查器 / 片段区 | 已接（形态适配） |
//! | 时间轴缩放 | `Cmd/Ctrl+=/-` | `Ctrl+=/-`（同）→ 网格每小节列宽 | 已接 |
//! | 轨道导航 | （GUI 用方向键微调选区） | Vi `h/l` 移小节光标、`j/k` 移轨、`gg/G` 首/尾 | TUI 新增（导航层） |
//! | 计数前缀 | 数字 `1..5` = 选工具 | 数字 = Vi 计数前缀（见冲突 K1） | TUI 新增（导航层） |
//! | 命令行 | 无 | `:` 进入（`q/w/wq/play/stop/rewind/export/open[!]/demo[!]`） | TUI 新增（模式层） |
//! | 轨道改名 | 双击轨道头 | `i` 进入 Insert 改名，`Enter` 提交、`Esc` 取消 | TUI 新增（模式层） |
//! | 退出 | 关窗 | `:q`（脏时拦）、`:q!` 强退 | TUI 新增（模式层） |
//! | 片段切分 | `Cmd/Ctrl+T`（播放头处） | `Ctrl+T`（同）→ 切分光标处片段 | 已接（三期，走 `host::clip_split_ops_for`） |
//! | 片段复制/剪切/粘贴 | `Cmd/Ctrl+C/X/V` | 同键 → 光标处片段；粘贴落选中轨光标处 | 已接（三期，组合照 GUI `clipboard_*`） |
//! | 片段改名 | （GUI 未接，MCP 有） | 片段网格焦点 `i` → Insert 改名（`Op::SetClipName`） | TUI 新增（三期，模式层） |
//! | 音符视图 | 钢琴卷帘（GUI） | 片段网格焦点 `n` 或 `:notes` 开、Esc 关 | TUI 新增（三期，视图层） |
//! | 音符选择/移动 | 卷帘选区 + 方向键微移 | 视图内 `j/k` 选、`h/l` 移时、`J/K` 移高、方向键同 GUI 微移 | 已接（三期，走 `nudge_delta_for`） |
//! | 音符增/删/时值/力度 | 卷帘工具 | 视图内 `a` 加、`Delete` 删、`<`/`>` 时值、`-`/`+` 力度 | TUI 新增（三期，视图层） |
//! | 片段池视图 | （GUI 无池管理面，MCP 有） | `:pool` 开、Esc 关；视图内 `Delete` 删条目 | TUI 新增（三期，视图层） |
//! | 移轨（显示顺序） | 轨头拖拽（无键） | `J`/`K`（计数 = 格数）＋ `:move <位置>` | TUI 新增（T-FIX-76，`Op::MoveTrack`） |
//! | 轨道高度 | 轨头拖拽 + 全局缩放 | `Alt+Shift+↑/↓` 每轨 ±16 px、`{`/`}` 全局 ±10%＋ `:height` / `:height%` | TUI 新增（T-FIX-76，`TrackHeightLayout`） |
//! | 主视图 Session / Arrangement | `F5` / `F6` | 同键＋ `:view session\|arrangement` | 已接（T-FIX-76，同一个 `View` 枚举） |
//! | 自动化泳道 | `Ctrl+Shift+A`（开关）+ 鼠标画点 | `Ctrl+Shift+A` 开关音量泳道、`P` 声相、`:auto`；视图内 `a`/`-`/`+`/`Delete` | 已接（T-FIX-76，唯一求值入口 + `Op::SetAutomationPoint`） |
//!
//! # 冲突清单（按「共享动作从 GUI、导航从 Vi」解）
//!
//! - **K1 数字键**：GUI 裸数字 `1..5` 是选工具（共享动作族），Vi 要数字作计数前缀。
//!   TUI v1 没有工具系统，选工具无落点 ⇒ 数字归 Vi 导航层作计数前缀；GUI 的选工具
//!   在 TUI 里记为「未接」。将来 TUI 若加工具，须另行裁决，不许默默改回。
//! - **K2 `+/-`**：本线原 brief 曾定「检查器聚焦时 +/- 调音量」。GUI 的音量键是
//!   `Cmd/Ctrl+Alt+↑/↓`（共享动作）⇒ 按口径从 GUI，`+/-` 不作音量键（未绑定）。
//! - **K3 `u` / `Ctrl+R`**：一期裁决是「撤销从 GUI 键，`u` 不绑定」（当时无
//!   撤销栈）。二期（Wang 2026-11-18 令推进完善）改判：GUI 键仍是基础
//!   （`Ctrl+Z` / `Ctrl+Shift+Z` 照走 GUI 解析表），TUI **再加自己的** Vi 键
//!   —— `u` = 撤销（可带计数前缀，`3u` 撤三步）、`Ctrl+R` = 重做。两路都落到
//!   同一个 `yeban_app::undo::UndoPort`，不另造撤销栈。
//! - **K4 `Enter`**：Vi 习惯 Enter 无动作，GUI `Enter` = 回到起点（共享动作）⇒ 从 GUI。
//!   Insert/Command 模式里的 Enter 是模式内提交，不在此列。
//! - **K5 `q`**：Vi 习惯裸 `q` 退出。TUI 只认 `:q`（模式层），裸 `q` 不绑定，防误触。
//! - **K6 `Tab`**：GUI `Tab`（画布聚焦）= 视图切换；TUI 无多视图，适配为窗格焦点
//!   循环 —— 同一个「视图层切换」语义，不新占别的键。
//! - **K7 列表视图（三期）**：音符/片段池视图是片段网格窗格的**内容变体**，
//!   不是新窗格（Tab 循环仍三格，K6 不变）。视图内的删除与微移不新占键：
//!   复用共享表的 `DeleteSelection` 与 `NudgeSelection`，在分发层按视图定
//!   落点（与二期 `DeleteSelection` 按焦点分流同一手法）；裸 `-`/`+` 在共享
//!   表是缩放，只在音符视图内分流为力度，视图外行为不变。新占的键只有
//!   裸 `n`（共享表 PassThrough、Vi 层未占）与视图内的 `a`/`<`/`>`/`J`/`K`
//!   （均为 PassThrough 或未占，视图外不拦截）。
//! - **K8 四方对齐补齐（T-FIX-76）**：新占键**逐个实测过解析器**（⛔ 不靠猜）——
//!   裸 `J`/`K`、`P`、`{`/`}`、`a` 在画布上下文都是 `PassThrough`；而裸 `p` 是
//!   `ToggleEditor`、裸 `[`/`]` 是试听、`Ctrl+Alt+A` 是 AI 抽屉 ⇒ 这三格不抢。
//!   `Ctrl+Shift+A` 是 GUI 的 `ToggleAutomation`（⚠ 不是 `Ctrl+Alt+A`，实测如此），
//!   在 TUI 落成"打开/关闭音量自动化视图"——同一个"显示自动化"语义的形态适配。
//!   `J`/`K` 的双重身份按视图分流：音符视图内移音高（三期既有），视图外移轨。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yeban_app::input::{LogicalKey, Modifiers};

/// TUI 的模态（Vi 式三态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 普通模式：GUI 共享键 + Vi 导航。
    Normal,
    /// 插入模式：轨道改名文本编辑。
    Insert,
    /// 命令模式：`:` 命令行输入。
    Command,
}

/// 平铺窗格焦点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// 左栏轨道列表。
    Tracks,
    /// 中栏片段网格。
    Grid,
    /// 右栏检查器。
    Inspector,
}

impl Pane {
    /// `Tab` 焦点循环的下一格。
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Tracks => Self::Grid,
            Self::Grid => Self::Inspector,
            Self::Inspector => Self::Tracks,
        }
    }

    /// 窗格的中文名（状态行显示）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tracks => "轨道",
            Self::Grid => "片段",
            Self::Inspector => "检查器",
        }
    }
}

/// Vi 导航动作（状态机吃掉一个键后吐出的导航意图；计数已折算进 `count`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViMotion {
    /// `h`：小节光标左移。
    BarLeft,
    /// `l`：小节光标右移。
    BarRight,
    /// `j`：选下一轨。
    TrackDown,
    /// `k`：选上一轨。
    TrackUp,
    /// `gg`：跳到首（轨或小节，随焦点）。
    First,
    /// `G`：跳到尾（轨或小节，随焦点）。
    Last,
}

/// Vi 状态机吃键的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViOutcome {
    /// 键被状态机消费，但还没凑成动作（数字累积、`g` 等待第二下）。
    Pending,
    /// 凑成一个导航动作（附计数，缺省 1）。
    Motion(ViMotion, usize),
    /// `:` —— 进命令模式。
    EnterCommand,
    /// `i` —— 进 Insert（改名）。
    EnterInsert,
    /// `u` —— 撤销（附计数，缺省 1；二期 K3 改判，见模块文档）。
    Undo(usize),
    /// 不是 Vi 键，交还给上层（上层也不认就忽略）。
    NotVi,
}

/// Vi 导航状态机：只管计数前缀与 `gg` 这类两键序列，与键位数据（GUI 表）分离。
#[derive(Debug, Default)]
pub struct ViMachine {
    count: Option<usize>,
    pending_g: bool,
}

impl ViMachine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前累积的计数（状态行显示用）。
    #[must_use]
    pub const fn count(&self) -> Option<usize> {
        self.count
    }

    /// 清空计数与半成品序列（动作执行后 / Esc）。
    pub fn reset(&mut self) {
        self.count = None;
        self.pending_g = false;
    }

    /// 取走计数（缺省 1）。
    fn take_count(&mut self) -> usize {
        let count = self.count.unwrap_or(1);
        self.count = None;
        count
    }

    /// 喂一个**已小写化**的字符键（无 Ctrl/Alt 修饰的普通模式按键）。
    pub fn feed_char(&mut self, ch: char) -> ViOutcome {
        if self.pending_g {
            self.pending_g = false;
            if ch == 'g' {
                let count = self.take_count();
                let _ = count; // gg 的计数在 v1 不改变语义（恒跳首），保留读数防误吞。
                return ViOutcome::Motion(ViMotion::First, 1);
            }
            // `g` 后面不是 `g`：半成品作废，这个键按它自己重新解释。
        }
        match ch {
            '0'..='9' => {
                let digit = usize::from(ch as u8 - b'0');
                // Vi 规矩：裸 `0` 不是计数起始（v1 不给它绑动作），计数中可作尾数。
                if digit == 0 && self.count.is_none() {
                    return ViOutcome::NotVi;
                }
                let next = self
                    .count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit);
                self.count = Some(next.min(999));
                ViOutcome::Pending
            }
            'h' => ViOutcome::Motion(ViMotion::BarLeft, self.take_count()),
            'l' => ViOutcome::Motion(ViMotion::BarRight, self.take_count()),
            'j' => ViOutcome::Motion(ViMotion::TrackDown, self.take_count()),
            'k' => ViOutcome::Motion(ViMotion::TrackUp, self.take_count()),
            'g' => {
                self.pending_g = true;
                ViOutcome::Pending
            }
            'G' => {
                self.count = None;
                ViOutcome::Motion(ViMotion::Last, 1)
            }
            ':' => {
                self.reset();
                ViOutcome::EnterCommand
            }
            'u' => ViOutcome::Undo(self.take_count()),
            'i' => {
                self.reset();
                ViOutcome::EnterInsert
            }
            _ => ViOutcome::NotVi,
        }
    }
}

/// crossterm 按键 → GUI 解析器的逻辑键 + 修饰位。
///
/// 映射口径与 GUI 的 `LogicalKey::from_text` 一致：可打印字符小写化，Shift 走修饰位；
/// 平台命令键在 Linux 终端里就是 Ctrl ⇒ 用 `Modifiers` 的 ctrl 族构造子。
/// 返回 `None` = 这个键不进 GUI 解析（功能键 F1–F4/F10+、纯修饰键等）。
#[must_use]
pub fn to_logical(event: KeyEvent) -> Option<(LogicalKey, Modifiers)> {
    let mods = event.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);
    let modifiers = match (ctrl, alt, shift) {
        (false, false, false) => Modifiers::none(),
        (false, false, true) => Modifiers::shift(),
        (false, true, false) => Modifiers::alt(),
        (true, false, false) => Modifiers::ctrl(),
        (true, false, true) => Modifiers::ctrl_shift(),
        (true, true, false) => Modifiers::ctrl_alt(),
        (true, true, true) => Modifiers::ctrl_shift_alt(),
        (false, true, true) => return None, // Alt+Shift 无对应构造子, GUI 表里也没有这一格。
    };
    let key = match event.code {
        KeyCode::Char(' ') => LogicalKey::Space,
        KeyCode::Char(ch) => LogicalKey::Character(ch.to_ascii_lowercase()),
        KeyCode::Enter => LogicalKey::Enter,
        KeyCode::Esc => LogicalKey::Escape,
        KeyCode::Tab | KeyCode::BackTab => LogicalKey::Tab,
        KeyCode::Backspace => LogicalKey::Backspace,
        KeyCode::Delete => LogicalKey::Delete,
        KeyCode::Up => LogicalKey::UpArrow,
        KeyCode::Down => LogicalKey::DownArrow,
        KeyCode::Left => LogicalKey::LeftArrow,
        KeyCode::Right => LogicalKey::RightArrow,
        KeyCode::F(5) => LogicalKey::F5,
        KeyCode::F(6) => LogicalKey::F6,
        KeyCode::F(7) => LogicalKey::F7,
        KeyCode::F(8) => LogicalKey::F8,
        KeyCode::F(9) => LogicalKey::F9,
        _ => return None,
    };
    Some((key, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(machine: &mut ViMachine, text: &str) -> Vec<ViOutcome> {
        text.chars().map(|ch| machine.feed_char(ch)).collect()
    }

    #[test]
    fn count_prefix_multiplies_motion() {
        let mut machine = ViMachine::new();
        let outcomes = feed(&mut machine, "12j");
        assert_eq!(
            outcomes,
            vec![
                ViOutcome::Pending,
                ViOutcome::Pending,
                ViOutcome::Motion(ViMotion::TrackDown, 12),
            ]
        );
        assert_eq!(machine.count(), None, "动作后计数必须清空");
    }

    #[test]
    fn bare_zero_is_not_a_count_start() {
        let mut machine = ViMachine::new();
        assert_eq!(machine.feed_char('0'), ViOutcome::NotVi);
        // 但计数中的 0 是尾数: 10k = 上移 10。
        let outcomes = feed(&mut machine, "10k");
        assert_eq!(
            outcomes.last(),
            Some(&ViOutcome::Motion(ViMotion::TrackUp, 10))
        );
    }

    #[test]
    fn gg_and_shift_g_jump_to_ends() {
        let mut machine = ViMachine::new();
        assert_eq!(machine.feed_char('g'), ViOutcome::Pending);
        assert_eq!(
            machine.feed_char('g'),
            ViOutcome::Motion(ViMotion::First, 1)
        );
        assert_eq!(machine.feed_char('G'), ViOutcome::Motion(ViMotion::Last, 1));
    }

    #[test]
    fn mode_entry_keys() {
        let mut machine = ViMachine::new();
        assert_eq!(machine.feed_char(':'), ViOutcome::EnterCommand);
        assert_eq!(machine.feed_char('i'), ViOutcome::EnterInsert);
        assert_eq!(
            machine.feed_char('q'),
            ViOutcome::NotVi,
            "裸 q 不绑定 (冲突 K5)"
        );
    }

    #[test]
    fn u_is_undo_with_count() {
        let mut machine = ViMachine::new();
        assert_eq!(machine.feed_char('u'), ViOutcome::Undo(1));
        let outcomes = feed(&mut machine, "3u");
        assert_eq!(outcomes.last(), Some(&ViOutcome::Undo(3)));
        assert_eq!(machine.count(), None, "动作后计数必须清空");
    }

    #[test]
    fn stray_g_then_motion_still_works() {
        let mut machine = ViMachine::new();
        assert_eq!(machine.feed_char('g'), ViOutcome::Pending);
        assert_eq!(
            machine.feed_char('j'),
            ViOutcome::Motion(ViMotion::TrackDown, 1)
        );
    }
}
