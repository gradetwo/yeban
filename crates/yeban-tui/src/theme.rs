//! 主题色：取样自 GUI 的 `crates/yeban-app/ui/tokens.slint`「渔火 (yeban)」flat 主题
//! （本分支基线 6b9240c91 的令牌值），TUI 与 GUI 共用同一套色值，不另调色。
//!
//! 映射表（GUI 令牌 → 取值 → TUI 用途）：
//!
//! | GUI 令牌 | 色值 | TUI 用途 |
//! | --- | --- | --- |
//! | `bg-void` | `#0e1216` | 窗口底色 |
//! | `bg-shell` / `bg-lane` | `#11161c` | 片段车道偶数行底 |
//! | `bg-lane2` | `#0d1116` | 片段车道奇数行底 |
//! | `bg-panel` | `#131a22` | 面板底（顶栏 / 检查器 / 底栏） |
//! | `bg-head` / `bg-panel-alt` | `#161e28` | 轨道头 / 交替面板 |
//! | `bg-raised` / `bg-control` | `#1e2733` | 选中行 / 抬起面 |
//! | `line` | `#212b36` | 边框线 |
//! | `line-strong` | `#2c3948` | 强边框 / 网格分隔 |
//! | `ink-0` | `#d7dee1` | 主文字 |
//! | `ink-1` | `#8d99a5` | 次文字 |
//! | `ink-2` | `#5a6672` | 弱文字 / 空格点 |
//! | `gold-bright` | `#e7dfc8` | 播放头 / 时间码 / 读数 |
//! | `gold` / `accent` | `#c6a47c` | 主强调（片段 / 模式徽标） |
//! | `gold-deep` | `#6d88a1` | 副强调（独奏标记） |
//! | `record-red` | `#ac6e60` | 静音标记 / 警告 |
//! | `playing` | `#6e9488` | 播放状态 |
//! | `selection` | `#8e86a6` | 选中 / 激活 |

use ratatui::style::Color;

/// GUI 令牌 `bg-void`（#0e1216）：窗口底色。
pub const BG_VOID: Color = Color::Rgb(0x0e, 0x12, 0x16);
/// GUI 令牌 `bg-shell` / `bg-lane`（#11161c）：车道偶数行底。
pub const BG_LANE: Color = Color::Rgb(0x11, 0x16, 0x1c);
/// GUI 令牌 `bg-lane2`（#0d1116）：车道奇数行底。
pub const BG_LANE2: Color = Color::Rgb(0x0d, 0x11, 0x16);
/// GUI 令牌 `bg-panel`（#131a22）：面板底。
pub const BG_PANEL: Color = Color::Rgb(0x13, 0x1a, 0x22);
/// GUI 令牌 `bg-head`（#161e28）：轨道头 / 交替面板。
pub const BG_HEAD: Color = Color::Rgb(0x16, 0x1e, 0x28);
/// GUI 令牌 `bg-raised`（#1e2733）：选中行 / 抬起面。
pub const BG_RAISED: Color = Color::Rgb(0x1e, 0x27, 0x33);
/// GUI 令牌 `line`（#212b36）：边框线。
pub const LINE: Color = Color::Rgb(0x21, 0x2b, 0x36);
/// GUI 令牌 `line-strong`（#2c3948）：强边框。
pub const LINE_STRONG: Color = Color::Rgb(0x2c, 0x39, 0x48);
/// GUI 令牌 `ink-0`（#d7dee1）：主文字。
pub const INK0: Color = Color::Rgb(0xd7, 0xde, 0xe1);
/// GUI 令牌 `ink-1`（#8d99a5）：次文字。
pub const INK1: Color = Color::Rgb(0x8d, 0x99, 0xa5);
/// GUI 令牌 `ink-2`（#5a6672）：弱文字。
pub const INK2: Color = Color::Rgb(0x5a, 0x66, 0x72);
/// GUI 令牌 `gold-bright`（#e7dfc8）：播放头 / 时间码。
pub const GOLD_BRIGHT: Color = Color::Rgb(0xe7, 0xdf, 0xc8);
/// GUI 令牌 `gold` / `accent`（#c6a47c）：主强调。
pub const GOLD: Color = Color::Rgb(0xc6, 0xa4, 0x7c);
/// GUI 令牌 `gold-deep`（#6d88a1）：副强调。
pub const GOLD_DEEP: Color = Color::Rgb(0x6d, 0x88, 0xa1);
/// GUI 令牌 `record-red`（#ac6e60）：静音 / 警告。
pub const RECORD_RED: Color = Color::Rgb(0xac, 0x6e, 0x60);
/// GUI 令牌 `playing`（#6e9488）：播放状态。
pub const PLAYING: Color = Color::Rgb(0x6e, 0x94, 0x88);
/// GUI 令牌 `selection`（#8e86a6）：选中 / 激活。
pub const SELECTION: Color = Color::Rgb(0x8e, 0x86, 0xa6);

/// 把工程里的 `#rrggbb` 轨道色标解析成终端真彩色（解析器复用 GUI 的
/// `yeban_app::bridge::parse_hex_color`，不另写一份）。
#[must_use]
pub fn track_color(raw: Option<&str>, slot: usize) -> Color {
    if let Some(parsed) = raw.and_then(yeban_app::bridge::parse_hex_color) {
        return Color::Rgb(parsed.red, parsed.green, parsed.blue);
    }
    // 无色标时的回退调色板（取自演示工程的轨道色与令牌色）。
    const FALLBACK: [Color; 6] = [
        Color::Rgb(0xf7, 0xe6, 0xb0),
        Color::Rgb(0x22, 0xaa, 0x88),
        Color::Rgb(0x33, 0x66, 0xff),
        GOLD,
        GOLD_DEEP,
        PLAYING,
    ];
    FALLBACK[slot % FALLBACK.len()]
}
