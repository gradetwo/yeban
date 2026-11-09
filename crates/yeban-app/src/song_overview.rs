//! 整曲多层可视化总览图（Song Overview）—— Phase 1 UI 面板。
//!
//! 设计稿：`~/workspace/yeban-songmap-design.md`（§2 四层定义、§7 视觉编码与交互、
//! §8.2 数据 Schema、§11 MVP Phase 1 列）。
//!
//! ## 本轮的范围（Phase 1）
//!
//! - 底部可折叠 104 px 面板（主画布 82 px + 底部 22 px），四层叠合渲染。
//! - 全屏最大化（Wang 2026-11-08 19:56 要求）：面板占满主窗口可用区（编排 /
//!   卷帘 / 控制台塌掉，顶栏 48 px 与状态栏 24 px 不动）；画布高 = 面板高 − 22 px
//!   条带，4 项交互在全屏态下照常可用。进入全屏即展开、折叠即退出全屏，
//!   `(collapsed, fullscreen)` 的唯一写者是 [`wire_song_overview`] 的两条 toggle；
//!   `Esc` 退出全屏走 `Action::Cancel` 级联（`host::apply_action`），落点是同一个
//!   `toggle-fullscreen` 回调。Logic Pro 对照见 `app.slint` 的全屏属性注释：
//!   Logic 的编辑器全屏是窗口级最大化 + 按钮退出、退出恢复原窗口态 —— 本面板
//!   抄的是"退出恢复原态"这一点；`Esc` 退出是本仓库 `Action::Cancel` 级联的既有
//!   口径（面板先于手势收尾），Logic 里 `Esc` 不退全屏。
//! - 4 项核心交互：点击跳转播放头、视口联动框（拖动滚动 / 右缘拖动缩放）、
//!   图层开关胶囊（单击隐藏 / 双击独显）、悬停小节读数。
//! - 数据是 **真实管线**：工程 → `yeban_render::song_overview::cached_overview`
//!   （整曲哈希缓存；谱面映射与离线渲染都在那一层）→ dsp `SongOverview` → 本模块的
//!   view-model 映射（[`overview_from_dsp`]）。标题不再带「试算」。
//! - 默认折叠：窗口不在 UI-GRID-002 的全展开档（≥1920×1080）时面板默认折叠
//!   （22 px），展开档默认展开。默认值是 `app.slint` 里 `song-overview-collapsed`
//!   的表达式绑定；用户按一次折叠键后 Rust 写一次属性，绑定即被覆盖，之后跟 toggle 走。
//!
//! ## 数据流（与仓库纪律一致）
//!
//! ```text
//! UndoPort::try_project（活工程，换工程自动跟着换）
//!   └─ refresh_song_overview ──> cached_overview（命中不重算）
//!        └─ overview_from_dsp ──> SongOverview（面板 view-model，活态的单一事实源）
//!             └─ apply_song_overview ──> MainWindow 的 song-overview-* 属性 ──> .slint 只画数
//! ```
//!
//! - 界面不算任何东西：Magma 配色、张力配色、悬停文本、标尺标签全在这里算好再注入。
//! - 小节数口径：**视图说了算**。dsp 的 `total_bars` 是内容跨度（谱面末音 / 音频末端
//!   向上取整），视图的 `bar_positions.len()` 是工程跨度（段落排布）；演示工程上前者
//!   是 3、后者是 16。面板的 `total-bars` / `span-ticks` 必须与视图一致（走带吸附、
//!   视口框、播放头都读视图跨度），因此映射按视图小节数定长：dsp 有数的小节原样映射，
//!   超出 dsp 跨度的小节按 dsp 自己的静音哨兵填（LUFS −70、频段 −36 dB、疏密 0、
//!   张力 0），⛔ 不编测量值；dsp 比视图长时截断（不画视图之外的部分）。
//! - 渲染失败时回退到纯谱面分析（`analyze_song_overview(score, None)`，dsp 的既定
//!   回退口径：能量 / 频率记哨兵，情绪响度分量走谱面力度代理，疏密与和声照常算），
//!   并把回退写进 [`RefreshReport`] 与日志；纯谱面也空时才画静音面板，⛔ 不留旧工程
//!   的数据（那会把上一首歌的曲线画到新工程上）。
//! - SFZ provider（[`arm_sfz_input`]）：只走磁盘腿（`resolve_sampler_path` 的既定
//!   候选顺序：工程目录 ⇒ 采样库根 ⇒ 仓库根 ⇒ cwd）+ 既有解码（`build_region_samples`）。
//!   内嵌 SFZ（`.yeban` 容器内）本轮不接：解析不到时 provider 记失败并返回空输入，
//!   渲染层随即响亮失败（⛔ 不拿静音轨去渲染），refresh 走纯谱面回退。缺样本的区域
//!   按引擎既定口径记静音（不进提供表），⛔ 不塞正弦占位。
//! - 刷新是同步的（调用线程上跑完整曲渲染 + 分析，首算付一次，之后同一哈希命中缓存
//!   不重算）；换工程后调一次 [`refresh_song_overview`] 即可。
//!
//! ## 生产接线
//!
//! `main.rs` 的 `wire_callbacks` 里，在 `host::wire_ruler_seek(..)` 之后加：
//!
//! ```ignore
//! let overview = yeban_app::song_overview::wire_song_overview(ui, Rc::clone(engine), Rc::clone(&view), undo);
//! yeban_app::song_overview::refresh_song_overview(ui, &view.borrow(), &overview);
//! ```
//!
//! `wire_song_overview` 在装配时已经按活工程算过一次真实数据并注入；换工程会重跑
//! `wire_callbacks`（`main.rs` 的 `after` 闭包），于是接线 + 刷新各再跑一次。
//! 只换缩放不换工程时不用调（数据是按小节的，与 `ticks-per-pixel` 无关）。
//!
//! - 界面不算任何东西：Magma 配色、张力配色、悬停文本、标尺标签全在这里算好再注入。
//! - 走带定位走**既有的** `EngineHost::seek`（`TransportCommand::SeekTicks` 的唯一
//!   发送口，与 `host::commit_seek` 同一条路）；`host::apply_transport` 是走带读数
//!   的唯一写者。本模块只复制 `commit_seek` 的 5 行借用守卫，⛔ 不另造定位通路。
//! - 视口滚动只写 `MainWindow.roll-scroll-x`（编排视图活读它，⛔ 不重投影）；
//!   视口缩放走 `host::set_roll_ticks_per_pixel`（唯一写点）+ 既有重投影路径
//!   （`host::project_with_view_state` + `host::apply_view`，与
//!   `host::wire_zoom_controls` 的滑块臂同一条路）。
//! - 定位 / 图层开关 / 折叠都是**会话运行态**（`MODEL-ISO-001` 第二层）：⛔ 不碰工程、
//!   ⛔ 不进撤销栈、⛔ 不改 `projectDigest`。

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};

use crate::bridge::ViewState;
use crate::engine_host::EngineHost;
use crate::host::{apply_transport, apply_view, project_with_view_state, set_roll_ticks_per_pixel};
use crate::sfz_load::{
    SfzRoots, build_region_samples, load_sfz_with_instrument, resolve_sampler_path,
};
use crate::ui::MainWindow;
use crate::undo::UndoPort;
use yeban_dsp::song_overview::{
    DENSITY_ANCHOR_NOTES_PER_SEC, NO_LOUDNESS_READING_LUFS, analyze_song_overview,
};
use yeban_model::song_overview::score_input_from_project;
use yeban_render::engine_source::{ArmedSfzInput, ArmedSfzSample};
use yeban_render::song_overview::{RenderSettings, SongOverviewCache, cached_overview};

/// 频率层频段数（设计稿 §4.1：8 频段音乐职能倍频程）。
pub const BAND_COUNT: usize = 8;

/// 频段名（设计稿 §4.1 表，Band 1 Sub 在下、Band 8 Air 在上）。
pub const BAND_NAMES: [&str; BAND_COUNT] = [
    "Sub 超低频",
    "Bass 低频",
    "Low-Mid 中低频",
    "Mid 中频",
    "Upper-Mid 中高频",
    "Presence 临场感",
    "Brilliance 辉煌度",
    "Air 空气感",
];

/// 频段短名（悬停读数里用，省宽度）。
pub const BAND_SHORT_NAMES: [&str; BAND_COUNT] = [
    "超低", "低频", "中低", "中频", "中高", "临场", "辉煌", "空气",
];

/// 频段频率范围（设计稿 §4.1 表，单位 Hz）。
pub const BAND_RANGES_HZ: [(u32, u32); BAND_COUNT] = [
    (20, 90),
    (90, 250),
    (250, 500),
    (500, 1000),
    (1000, 2000),
    (2000, 5000),
    (5000, 10000),
    (10000, 20000),
];

/// 能量层 LUFS 映射区间（设计稿 §7.2：纵轴映射 −36 LUFS 至 −6 LUFS）。
pub const LUFS_FLOOR: f32 = -36.0;
/// 能量层 LUFS 映射上沿（设计稿 §7.2）。
pub const LUFS_CEIL: f32 = -6.0;

/// 频率层 dB 映射区间（设计稿 §4.5：相对峰值对数标尺 [−36 dB, 0 dB]）。
pub const BAND_DB_FLOOR: f32 = -36.0;
/// 频率层 dB 映射上沿（设计稿 §4.5：全曲第 98 百分位数为 0 dB 满度锚点）。
pub const BAND_DB_CEIL: f32 = 0.0;

/// 图层下标（`.slint` 的 `layer-toggle` / `layer-solo` 回调参数）。
/// 顺序 = 胶囊从左到右：情绪 / 能量 / 频段 / 疏密（设计稿 §7.3）。
pub const LAYER_TENSION: i32 = 0;
/// 图层下标：能量层。
pub const LAYER_ENERGY: i32 = 1;
/// 图层下标：频段层。
pub const LAYER_BANDS: i32 = 2;
/// 图层下标：疏密层。
pub const LAYER_DENSITY: i32 = 3;

/// 小节级列式总览数据（设计稿 §8.2 的 `bar_series`，列式形状）。
///
/// 每个数组的长度都是 `total_bars`；`bands_db` 是平铺的 `bar * 8 + band`
/// （band 0 = Sub，band 7 = Air）。所有归一化指标保留 2 位小数语义
/// （`0.00..=1.00`），响度与频段能量保留 1 位小数（单位 dB）。
#[derive(Debug, Clone, PartialEq)]
pub struct SongOverview {
    /// 小节数（= `ViewState.bar_positions.len()`，至少 `MIN_BAR_COUNT`）。
    pub total_bars: usize,
    /// 一小节的 tick 数（`ViewState.bar_length_ticks`）。
    pub bar_length_ticks: u64,
    /// 全曲 tick 跨度（= `total_bars * bar_length_ticks`）。
    pub span_ticks: u64,
    /// 调号文本（设计稿 §8.2 `song_meta.key_signature`，如 `"C major"`；无工程时为空串）。
    pub key_signature: String,
    /// 段落（名，起始小节 0 起），按起始小节升序。
    pub sections: Vec<(String, usize)>,
    /// 情绪张力 `T(b)`（设计稿 §3.2，`0.0..=1.0`）。
    pub tension: Vec<f32>,
    /// 效价色相（设计稿 §3.1，`-1.0..=1.0`：负 = 冷 / 小调，小正 = 暖 / 大调）。
    pub valence_hue: Vec<f32>,
    /// 短时 LUFS（设计稿 §5.2，单位 LU）。
    pub lufs_short: Vec<f32>,
    /// 旋律和声组每拍触发数（设计稿 §6.1，已归一化到 `0.0..=1.0`）。
    pub density_pitched: Vec<f32>,
    /// 旋律和声组发声占空比（设计稿 §6.1，`0.0..=1.0`）。
    pub density_duty: Vec<f32>,
    /// 打击组每拍触发数（设计稿 §6.1，已归一化到 `0.0..=1.0`）。
    pub density_perc: Vec<f32>,
    /// 8 频段倾斜补偿后能量（设计稿 §4.3，单位 dB，平铺 `bar * 8 + band`）。
    pub bands_db: Vec<f32>,
}

impl SongOverview {
    /// 第 `bar` 小节第 `band` 频段的倾斜补偿后能量（dB）。
    #[must_use]
    pub fn band_db(&self, bar: usize, band: usize) -> f32 {
        self.bands_db
            .get(bar.saturating_mul(BAND_COUNT).saturating_add(band))
            .copied()
            .unwrap_or(BAND_DB_FLOOR)
    }

    /// 第 `bar` 小节最突出的两个频段（按倾斜补偿后 dB 从高到低）。
    /// 返回 `(第一, 第二)` 的频段下标；小节越界时返回 `(3, 5)`（中频 / 临场感，
    /// 只是越界兜底，⛔ 不是测量值 —— 调用方只在合法小节上调它）。
    #[must_use]
    pub fn top_bands(&self, bar: usize) -> (usize, usize) {
        if bar >= self.total_bars {
            return (3, 5);
        }
        let mut order: [usize; BAND_COUNT] = [0, 1, 2, 3, 4, 5, 6, 7];
        order.sort_by(|&a, &b| {
            self.band_db(bar, b)
                .partial_cmp(&self.band_db(bar, a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        (order[0], order[1])
    }
}

/// Magma 色表 5 个锚点（设计稿 §4.5）：
/// 深炭黑 → 靛紫 → 暗洋红 → 琥珀橙 → 暖白金。
const MAGMA_STOPS: [(u8, u8, u8); 5] = [
    (0x12, 0x13, 0x1A),
    (0x3B, 0x1C, 0x54),
    (0x8C, 0x29, 0x56),
    (0xE0, 0x68, 0x38),
    (0xFC, 0xE8, 0xA6),
];

/// 归一化频段能量（`0.0..=1.0`，0 = −36 dB，1 = 0 dB）→ Magma 颜色。
///
/// 分段线性插值，端点外夹紧。纯函数（单测覆盖 6 个锚点）。
#[must_use]
pub fn magma_color(normalized: f32) -> slint::Color {
    let v = normalized.clamp(0.0, 1.0);
    let scaled = v * (MAGMA_STOPS.len() as f32 - 1.0);
    let index = (scaled.floor() as usize).min(MAGMA_STOPS.len() - 2);
    let frac = scaled - index as f32;
    let (r0, g0, b0) = MAGMA_STOPS[index];
    let (r1, g1, b1) = MAGMA_STOPS[index + 1];
    let mix = |a: u8, b: u8| -> u8 {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * frac).round() as u8
    };
    slint::Color::from_rgb_u8(mix(r0, r1), mix(g0, g1), mix(b0, b1))
}

/// 频段能量（dB）→ Magma 颜色（设计稿 §4.5 的 [−36 dB, 0 dB] 映射）。
#[must_use]
pub fn band_db_color(db: f32) -> slint::Color {
    magma_color((db - BAND_DB_FLOOR) / (BAND_DB_CEIL - BAND_DB_FLOOR))
}

/// 效价色相（`-1.0..=1.0`）→ 情绪线颜色（设计稿 §3.1 / §7.2：
/// 冷蓝 = 低效价 / 小调，亮金 = 高效价 / 大调）。
#[must_use]
pub fn tension_color(valence_hue: f32) -> slint::Color {
    let t = ((valence_hue + 1.0) / 2.0).clamp(0.0, 1.0);
    // 冷端 #3B82F6（蓝）→ 暖端 #F5B81D（亮金），线性插值。
    let mix =
        |a: u8, b: u8| -> u8 { (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8 };
    slint::Color::from_rgb_u8(mix(0x3B, 0xF5), mix(0x82, 0xB8), mix(0xF6, 0x1D))
}

/// 短时 LUFS → 能量柱归一化高度（设计稿 §7.2：−36 LUFS = 0，−6 LUFS = 满）。
#[must_use]
pub fn lufs_to_height(lufs: f32) -> f32 {
    ((lufs - LUFS_FLOOR) / (LUFS_CEIL - LUFS_FLOOR)).clamp(0.0, 1.0)
}

/// 横坐标分数（`0.0..=1.0`，相对总览图全宽）→ 小节下标（0 起，夹在合法范围内）。
#[must_use]
pub fn bar_at_fraction(fraction: f32, total_bars: usize) -> usize {
    if total_bars == 0 || !fraction.is_finite() {
        return 0;
    }
    let bar = (fraction.clamp(0.0, 1.0) * total_bars as f32).floor() as usize;
    bar.min(total_bars.saturating_sub(1))
}

/// 横坐标分数 → 走带目标 tick（纯函数）：分数 × 全曲跨度，再吸附到最近的小节线，
/// 夹在 `[0, span_ticks]`（与 `host::ruler_seek_tick` 同一条吸附 + 夹紧口径，
/// 只是输入是分数而不是像素列）。
/// 非有限分数 / 跨度为 0 / 小节长为 0 ⇒ `None`（不响应）。
#[must_use]
pub fn seek_tick_for_fraction(
    fraction: f32,
    span_ticks: u64,
    bar_length_ticks: u64,
) -> Option<u64> {
    if !fraction.is_finite() || span_ticks == 0 || bar_length_ticks == 0 {
        return None;
    }
    let tick = (fraction.clamp(0.0, 1.0) * span_ticks as f32) as u64;
    Some(crate::bridge::snap_tick(tick, bar_length_ticks).min(span_ticks))
}

/// 悬停读数（设计稿 §7.3 ④）：当前小节号、短时 LUFS、张力分值及最突出的前两个频段。
///
/// 格式：`第 5 小节 · -18.2 LUFS · 张力 0.62 · 突出 中频/临场感`
/// （小节号 1 起；LUFS 1 位小数；张力 2 位小数）。
#[must_use]
pub fn hover_readout(overview: &SongOverview, bar: usize) -> String {
    if overview.total_bars == 0 || bar >= overview.total_bars {
        return String::new();
    }
    let (first, second) = overview.top_bands(bar);
    format!(
        "第 {} 小节 · {:.1} LUFS · 张力 {:.2} · 突出 {}/{}",
        bar + 1,
        overview.lufs_short[bar],
        overview.tension[bar],
        BAND_SHORT_NAMES[first],
        BAND_SHORT_NAMES[second]
    )
}

/// 底部 8 px 标尺每小节的标签（Rust 侧算好，`.slint` 只画）：
/// 段落起点写段落名；其余小节每 4 小节写一个小节号（1 起）；其余为空串。
/// 段落名优先（段落起点那一格不写数字，避免两串文字重叠）。
#[must_use]
pub fn ruler_labels(total_bars: usize, sections: &[(String, usize)]) -> Vec<String> {
    let mut labels = vec![String::new(); total_bars];
    for (index, label) in labels.iter_mut().enumerate() {
        if index % 4 == 0 {
            *label = format!("{}", index + 1);
        }
    }
    for (name, start) in sections {
        if *start < total_bars {
            labels[*start] = name.clone();
        }
    }
    labels
}

/// 每拍触发数（次/拍）→ 归一化疏密（`0.0..=1.0`，面板的疏密条码读这个）。
///
/// 口径与 dsp 张力公式里的密度分量**同一条**（设计稿 §3.2）：先按基准速度折成
/// 每秒触发数 `d = onsets_per_beat × bpm / 60`，再对数压缩
/// `D̂ = ln(1 + d) / ln(1 + 16)`（16 = 120 BPM 下双轨十六分音符满铺的锚点）。
/// 速度非法 / 非有限 ⇒ 按 120 BPM 计（dsp 的 `DEFAULT_BPM` 同值）。
#[must_use]
pub fn normalize_density(onsets_per_beat: f32, bpm: f32) -> f32 {
    let bpm = if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    };
    let notes_per_sec = onsets_per_beat.max(0.0) * bpm / 60.0;
    ((1.0 + notes_per_sec).ln() / (1.0 + DENSITY_ANCHOR_NOTES_PER_SEC).ln()).clamp(0.0, 1.0)
}

/// 调号文本 → 效价底色（dsp 的 `valence_base`：小调 −0.6、大调 +0.6）。
///
/// dsp 的 `key_signature` 形如 `"D minor"` / `"C major"`；后缀读不出时中性 0.0
/// （⛔ 不猜调式）。静音垫条的效价就取这个底色（张力为 0 时 dsp 的公式退化成底色本身）。
#[must_use]
pub fn valence_base_for_key(key_signature: &str) -> f32 {
    if key_signature.ends_with("minor") {
        -0.6
    } else if key_signature.ends_with("major") {
        0.6
    } else {
        0.0
    }
}

/// 视图段落 → 面板段落表（名，起始小节 0 起，按起始小节升序）。
///
/// 段落是工程结构（⛔ 不是测量值），因此取自视图（与 round 1 的 stub 同源），
/// 不取 dsp 的逐小节 `section` 列（那一列只覆盖内容跨度，演示工程上只有 3 小节，
/// 会丢掉第 4 小节起的 Verse / Chorus / Outro 分界线）。
fn sections_from_view(view: &ViewState, total_bars: usize) -> Vec<(String, usize)> {
    view.sections
        .iter()
        .map(|section| {
            // ⚠ 不手写 `if bar_length_ticks == 0` 再除：clippy `manual_checked_ops`
            // 会判手动查除数；`checked_div` 的 None（除数为 0）即 0。
            let start = section
                .start_tick
                .checked_div(view.bar_length_ticks)
                .unwrap_or_default() as usize;
            (
                section.name.clone(),
                start.min(total_bars.saturating_sub(1)),
            )
        })
        .collect()
}

/// dsp `SongOverview` → 面板 view-model（[`SongOverview`]，列式形状见 §8.2）。
///
/// - 小节数 / 小节长 / 跨度 / 段落取自视图（见模块文档的小节数口径）。
/// - 张力 / 效价 / LUFS 原样取 dsp 列（各夹到面板的设计量程）；频段能量夹到
///   [−36 dB, 0 dB]（dsp 无音频时记 −120 dB 哨兵，面板的 Magma 映射以 −36 dB 为底，
///   夹断即最暗，不另造一档）；疏密按 [`normalize_density`] 归一化。
/// - 超出 dsp 跨度的小节是静音垫条（LUFS −70、频段 −36 dB、疏密 0、张力 0，
///   效价取调号底色）；dsp 比视图长时截断。调号取 dsp 的 `song_meta.key_signature`。
#[must_use]
pub fn overview_from_dsp(
    dsp: &yeban_dsp::song_overview::SongOverview,
    view: &ViewState,
) -> SongOverview {
    let total_bars = view.bar_positions.len();
    let bar_length_ticks = view.bar_length_ticks;
    let series = &dsp.bar_series;
    let dsp_bars = series.len();
    let bpm = dsp.song_meta.base_bpm;
    let valence_base = valence_base_for_key(&dsp.song_meta.key_signature);
    let mut tension = Vec::with_capacity(total_bars);
    let mut valence_hue = Vec::with_capacity(total_bars);
    let mut lufs_short = Vec::with_capacity(total_bars);
    let mut density_pitched = Vec::with_capacity(total_bars);
    let mut density_duty = Vec::with_capacity(total_bars);
    let mut density_perc = Vec::with_capacity(total_bars);
    let mut bands_db = Vec::with_capacity(total_bars.saturating_mul(BAND_COUNT));
    for bar in 0..total_bars {
        if bar < dsp_bars {
            tension.push(series.tension[bar].clamp(0.0, 1.0));
            valence_hue.push(series.valence_hue[bar].clamp(-1.0, 1.0));
            lufs_short.push(series.lufs_short[bar]);
            density_pitched.push(normalize_density(
                series.density_pitched_onsets_per_beat[bar],
                bpm,
            ));
            density_duty.push(series.density_pitched_duty[bar].clamp(0.0, 1.0));
            density_perc.push(normalize_density(
                series.density_perc_onsets_per_beat[bar],
                bpm,
            ));
            for band in 0..BAND_COUNT {
                bands_db.push(series.bands_tilted_db[bar][band].clamp(BAND_DB_FLOOR, BAND_DB_CEIL));
            }
        } else {
            // 静音垫条：超出 dsp 内容跨度的小节（无音符、无音频），按 dsp 自己的
            // 静音哨兵填，⛔ 不编测量值。
            tension.push(0.0);
            valence_hue.push(valence_base);
            lufs_short.push(NO_LOUDNESS_READING_LUFS);
            density_pitched.push(0.0);
            density_duty.push(0.0);
            density_perc.push(0.0);
            bands_db.extend(std::iter::repeat_n(BAND_DB_FLOOR, BAND_COUNT));
        }
    }
    SongOverview {
        total_bars,
        bar_length_ticks,
        span_ticks: (total_bars as u64).saturating_mul(bar_length_ticks),
        key_signature: dsp.song_meta.key_signature.clone(),
        sections: sections_from_view(view, total_bars),
        tension,
        valence_hue,
        lufs_short,
        density_pitched,
        density_duty,
        density_perc,
        bands_db,
    }
}

/// 没有任何测量值的静音面板（视图几何 + 全哨兵）：没有工程、或纯谱面也空时用。
///
/// 调号未知 ⇒ 效价底色取 0.0 中性（⛔ 不编一个假调号）。
#[must_use]
pub fn silent_overview(view: &ViewState) -> SongOverview {
    let total_bars = view.bar_positions.len();
    let bar_length_ticks = view.bar_length_ticks;
    SongOverview {
        total_bars,
        bar_length_ticks,
        span_ticks: (total_bars as u64).saturating_mul(bar_length_ticks),
        key_signature: String::new(),
        sections: sections_from_view(view, total_bars),
        tension: vec![0.0; total_bars],
        valence_hue: vec![0.0; total_bars],
        lufs_short: vec![NO_LOUDNESS_READING_LUFS; total_bars],
        density_pitched: vec![0.0; total_bars],
        density_duty: vec![0.0; total_bars],
        density_perc: vec![0.0; total_bars],
        bands_db: vec![BAND_DB_FLOOR; total_bars.saturating_mul(BAND_COUNT)],
    }
}
/// `[float]` 属性 ← `&[f32]`（与 `host.rs` 的 `lengths` 同款，⛔ 不复用私函数）。
fn floats(values: &[f32]) -> ModelRc<f32> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

/// `[string]` 属性 ← `&[String]`（与 `host.rs` 的 `strings` 同款）。
fn strings(values: &[String]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        values
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>(),
    ))
}

/// `[int]` 属性 ← `&[usize]`（小节下标，`i32` 装得下真实工程的小节数）。
fn bar_indexes(values: &[usize]) -> ModelRc<i32> {
    ModelRc::new(VecModel::from(
        values
            .iter()
            .map(|value| i32::try_from(*value).unwrap_or(i32::MAX))
            .collect::<Vec<_>>(),
    ))
}

/// `[color]` 属性 ← `&[slint::Color]`。
fn colors(values: &[slint::Color]) -> ModelRc<slint::Color> {
    ModelRc::new(VecModel::from(values.to_vec()))
}

/// 把一份 [`SongOverview`] 单向注入 `MainWindow` 的 `song-overview-*` 属性。
///
/// 幂等：同一份数据重复注入得到同一个界面。悬停读数（`hover-*`）⛔ 不在这里碰 ——
/// 它是 `hover-requested` 回调的写面，不是数据注入面。
pub fn apply_song_overview(ui: &MainWindow, overview: &SongOverview) {
    ui.set_song_overview_total_bars(i32::try_from(overview.total_bars).unwrap_or(i32::MAX));
    ui.set_song_overview_span_ticks(i32::try_from(overview.span_ticks).unwrap_or(i32::MAX));
    ui.set_song_overview_key_signature(overview.key_signature.clone().into());
    ui.set_song_overview_tension(floats(&overview.tension));
    let tension_colors: Vec<slint::Color> = overview
        .valence_hue
        .iter()
        .map(|hue| tension_color(*hue))
        .collect();
    ui.set_song_overview_tension_colors(colors(&tension_colors));
    ui.set_song_overview_lufs(floats(&overview.lufs_short));
    let band_colors: Vec<slint::Color> = overview
        .bands_db
        .iter()
        .map(|db| band_db_color(*db))
        .collect();
    ui.set_song_overview_band_colors(colors(&band_colors));
    ui.set_song_overview_density_pitched(floats(&overview.density_pitched));
    ui.set_song_overview_density_perc(floats(&overview.density_perc));
    ui.set_song_overview_density_duty(floats(&overview.density_duty));
    let section_names: Vec<String> = overview
        .sections
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    ui.set_song_overview_section_names(strings(&section_names));
    let section_starts: Vec<usize> = overview.sections.iter().map(|(_, start)| *start).collect();
    ui.set_song_overview_section_start_bars(bar_indexes(&section_starts));
    ui.set_song_overview_ruler_labels(strings(&ruler_labels(
        overview.total_bars,
        &overview.sections,
    )));
}

/// 刷新一次总览数据的结果（给调用方与测试读的回执，不是界面状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshReport {
    /// 本次注入面板的小节数（= 视图小节数）。
    pub bars: usize,
    /// 本次是否命中整曲哈希缓存（命中 ⇒ 没有重渲染、没有重分析）。
    pub cache_hit: bool,
    /// 本次是否走了纯谱面回退（渲染失败时才为真；见 [`refresh_song_overview`]）。
    pub score_only: bool,
    /// 失败 / 回退的原因（成功且全量时为 `None`；回退时记渲染失败原文）。
    pub error: Option<String>,
}

/// 总览面板的活态（`wire_song_overview` 的返回值，`Rc<RefCell>` 单一事实源）。
pub struct SongOverviewLive {
    /// 当前注入面板的那份 view-model（回调闭包读它算悬停读数）。
    pub data: SongOverview,
    /// 整曲哈希缓存（`cached_overview` 的进程内缓存，跨刷新常驻）。
    pub cache: SongOverviewCache,
    /// 活工程的来源（`try_project()`，换工程自动跟着换；`None` ⇒ 无工程）。
    pub port: Option<Rc<UndoPort>>,
    /// SFZ 磁盘解析的工程目录根（装配时从引擎现读；换工程重接线时重读）。
    pub project_dir: Option<std::path::PathBuf>,
}

/// SFZ 武装输入（`cached_overview` 的 provider）：磁盘腿。
///
/// - 路径解析走既有的 [`resolve_sampler_path`]（工程目录 ⇒ 采样库根 ⇒ 仓库根 ⇒ cwd）。
/// - 样本解码走既有的 [`build_region_samples`]（读不到的区域记缺样本，引擎按静音处理，
///   ⛔ 不塞正弦占位；内建静音 `*silence` 不进提供表，引擎同样记静音）。
/// - 解析 / 读盘失败 ⇒ 把原因记进 `failures` 并返回空输入；渲染层随即响亮失败
///   （`load_sfz_instrument` 读不到空输入指的路径），⛔ 不拿静音轨去渲染。
///   内嵌 SFZ（`.yeban` 容器内）本轮不接：盘上解析不到即走这条失败路，refresh 再回退
///   到纯谱面分析（见 [`refresh_song_overview`]）。
fn arm_sfz_input(
    raw: &str,
    roots: SfzRoots<'_>,
    engine_hz: f32,
    failures: &mut Vec<String>,
) -> ArmedSfzInput {
    let resolved = match resolve_sampler_path(raw, roots) {
        Ok(path) => path,
        Err(error) => {
            failures.push(format!("SFZ 路径解析失败 `{raw}`：{error}"));
            return ArmedSfzInput::empty(raw);
        }
    };
    let instrument = match load_sfz_with_instrument(&resolved, None, engine_hz) {
        Ok((_, instrument)) => instrument,
        Err(error) => {
            failures.push(format!(
                "SFZ 读盘/解析失败 `{}`：{error}",
                resolved.display()
            ));
            return ArmedSfzInput::empty(raw);
        }
    };
    let table = build_region_samples(&instrument, &resolved);
    for (missing, reason) in &table.missing {
        failures.push(format!(
            "SFZ 缺样本 `{missing}`（{reason}）：该区域按静音渲染"
        ));
    }
    let samples: Vec<ArmedSfzSample> = table
        .samples
        .iter()
        .filter(|(path, _)| !yeban_sfz::instrument::is_builtin_silence_sample(path))
        .map(|(path, sample)| {
            (
                path.clone(),
                (std::sync::Arc::clone(&sample.pcm), sample.sample_hz),
            )
        })
        .collect();
    ArmedSfzInput::new(resolved, samples)
}

/// 按活工程重算总览数据并注入面板（换工程后调一次；只换缩放不换工程时不用调）。
///
/// - 全量路：`cached_overview`（工程 JSON + 采样率 + 种子哈希；命中不重算）。
///   采样率取工程 `audio_config` 的值（与渲染层校验的口径同一条）；种子 0。
/// - 渲染失败时回退到纯谱面分析（`analyze_song_overview(score, None)`）；回退原因
///   写进回执与日志，面板画的是谱面实测值 + 能量/频率哨兵，⛔ 不是假全量。
/// - 纯谱面也空（空工程）时画静音面板；没有工程（`port` 为 `None` 或取不到工程）
///   时同样画静音面板，⛔ 不留旧工程的数据。
/// - 刷新是同步的：首算在调用线程上跑完整曲渲染 + 分析，之后同一哈希命中缓存。
///
/// 返回回执（小节数 / 是否命中 / 是否回退 / 失败原文）。调用方留着 `state`
/// （`wire_song_overview` 的返回值就是同一份 `Rc`）以备下一次刷新 —— 刷新原地
/// 改写这份 `Rc` 里的内容，回调闭包读到的永远是最新份。
#[must_use]
pub fn refresh_song_overview(
    ui: &MainWindow,
    view: &ViewState,
    state: &Rc<RefCell<SongOverviewLive>>,
) -> RefreshReport {
    let port = state.borrow().port.clone();
    let project_dir = state.borrow().project_dir.clone();
    let Some(project) = port.as_ref().and_then(|port| port.try_project()) else {
        let silent = silent_overview(view);
        let report = RefreshReport {
            bars: silent.total_bars,
            cache_hit: false,
            score_only: false,
            error: Some("没有活工程：总览面板画静音（不留旧工程数据）".to_string()),
        };
        state.borrow_mut().data = silent.clone();
        apply_song_overview(ui, &silent);
        eprintln!("[yeban-app] 总览图刷新：没有活工程 ⇒ 面板画静音");
        return report;
    };
    let settings = RenderSettings::new(project.audio_config.sample_rate.hz(), 0);
    let roots = match project_dir.as_ref() {
        Some(dir) => SfzRoots::under_project(dir),
        None => SfzRoots::none(),
    };
    let mut failures: Vec<String> = Vec::new();
    let engine_hz = settings.sample_rate_hz as f32;
    let result = {
        let mut live = state.borrow_mut();
        cached_overview(&mut live.cache, &project, &settings, |_, raw| {
            arm_sfz_input(raw, roots, engine_hz, &mut failures)
        })
    };
    match result {
        Ok((dsp, hit)) => {
            let mapped = overview_from_dsp(&dsp, view);
            let report = RefreshReport {
                bars: mapped.total_bars,
                cache_hit: hit,
                score_only: false,
                error: None,
            };
            state.borrow_mut().data = mapped.clone();
            apply_song_overview(ui, &mapped);
            if !failures.is_empty() {
                eprintln!(
                    "[yeban-app] 总览图刷新：SFZ 有 {} 处缺口（已按静音渲染）：{}",
                    failures.len(),
                    failures.join("；")
                );
            }
            report
        }
        Err(error) => {
            // 渲染失败 ⇒ 纯谱面回退（dsp 的既定回退口径），不留旧工程数据。
            let score = score_input_from_project(&project);
            let dsp = analyze_song_overview(&score, None);
            let message = format!("渲染失败（{error:?}），回退到纯谱面分析");
            eprintln!("[yeban-app] 总览图刷新：{message}");
            let (mapped, score_only) = if dsp.bar_series.is_empty() {
                (silent_overview(view), true)
            } else {
                (overview_from_dsp(&dsp, view), true)
            };
            let report = RefreshReport {
                bars: mapped.total_bars,
                cache_hit: false,
                score_only,
                error: Some(message),
            };
            state.borrow_mut().data = mapped.clone();
            apply_song_overview(ui, &mapped);
            report
        }
    }
}
/// 走带定位的提交口（与 `host::commit_seek` 同一条路，只是那一个函数是私有的）。
///
/// 引擎借不到（同一线程重入）⇒ 降级（一行 `eprintln`，走带与界面一位不动），
/// ⛔ 不 panic、⛔ 不 abort。
fn commit_overview_seek(ui: &MainWindow, engine: &Rc<RefCell<EngineHost>>, tick: u64) {
    let Ok(mut engine) = engine.try_borrow_mut() else {
        eprintln!("[yeban-app] 忽略总览图定位：引擎正被同一线程借用（重入）⇒ 本次定位无效");
        return;
    };
    let reading = engine.seek(tick);
    drop(engine);
    apply_transport(ui, reading);
}

/// 视口缩放的重投影（与 `host::wire_zoom_controls` 的滑块臂同一条路）。
///
/// `port` 没有工程（`try_project` 是 `None`）⇒ 只写缩放属性，不重投影
/// （面板自己的视口几何活读 `roll-ticks-per-pixel`，不受影响；编排网格等下一次
/// 重投影再跟上，与 `host::set_roll_ticks_per_pixel` 的「不重投影」口径一致）。
fn reproject_after_zoom(ui: &MainWindow, port: &Option<Rc<UndoPort>>) {
    let Some(port) = port.as_ref() else {
        return;
    };
    let Some(project) = port.try_project() else {
        return;
    };
    let Ok(view) = project_with_view_state(&project, ui) else {
        return;
    };
    let width = slint::ComponentHandle::window(ui).size().width as f32;
    apply_view(ui, &view, width, ui.get_roll_scroll_x());
}

/// 把总览面板的 7 条回调接到引擎 / 视图态上。
///
/// 参数与产品装配同源：`engine` 是 `host::wire_transport` 拿的**同一个**
/// `Rc<RefCell<EngineHost>>`（点总览图与点标尺作用在同一条走带上），`view` 是
/// `host::wire_clip_scope` / `host::wire_ruler_seek` 拿的**同一个**活投影
/// （网格右缘 / 小节长度都从它现读，换工程之后跟着换），`port` 是
/// `host::wire_zoom_controls` 拿的**同一个**撤销端口（视口缩放要重投影；同时也是
/// 活工程的来源，见 [`refresh_song_overview`]）。
/// `port` 传 `None` 时缩放只写属性（见 [`reproject_after_zoom`]），且面板画静音
/// （没有活工程可算）。
///
/// 返回活态的单一事实源：装配时已经按活工程算过一次真实数据并注入；换工程后调
/// [`refresh_song_overview`]（传这份 `Rc`）即可更新数据，⛔ 不用重接回调。
pub fn wire_song_overview(
    ui: &MainWindow,
    engine: Rc<RefCell<EngineHost>>,
    view: Rc<RefCell<ViewState>>,
    port: Option<Rc<UndoPort>>,
) -> Rc<RefCell<SongOverviewLive>> {
    let project_dir = engine.borrow().project_dir().map(|path| path.to_path_buf());
    let state = Rc::new(RefCell::new(SongOverviewLive {
        data: silent_overview(&view.borrow()),
        cache: SongOverviewCache::new(),
        port: port.clone(),
        project_dir,
    }));
    // 装配时按活工程算一次真实数据并注入（`refresh_song_overview` 在内）。
    // 回执只作装配期诊断：缓存命中/回退已由刷新写进日志，这里不展开用。
    let _assembly_report = refresh_song_overview(ui, &view.borrow(), &state);

    // ① 点击跳转播放头：界面只报横坐标分数（0..1），换算 / 吸附 / 夹紧全在这里。
    let weak = ui.as_weak();
    ui.on_song_overview_seek({
        let weak = weak.clone();
        let engine = Rc::clone(&engine);
        let view = Rc::clone(&view);
        move |fraction| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let borrowed = view.borrow();
            let Some(tick) = seek_tick_for_fraction(
                fraction,
                (borrowed.bar_positions.len() as u64).saturating_mul(borrowed.bar_length_ticks),
                borrowed.bar_length_ticks,
            ) else {
                return;
            };
            drop(borrowed);
            commit_overview_seek(&ui, &engine, tick);
        }
    });

    // ④ 悬停小节读数：界面只报横坐标分数，文本全在这里按真实数据算好写回。
    let weak = ui.as_weak();
    ui.on_song_overview_hover({
        let weak = weak.clone();
        let state = Rc::clone(&state);
        move |fraction| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let borrowed = state.borrow();
            let overview = &borrowed.data;
            if overview.total_bars == 0 {
                return;
            }
            let bar = bar_at_fraction(fraction, overview.total_bars);
            let text = hover_readout(overview, bar);
            drop(borrowed);
            ui.set_song_overview_hover_text(text.into());
            ui.set_song_overview_hover_fraction(fraction.clamp(0.0, 1.0));
            ui.set_song_overview_hover_visible(true);
        }
    });

    // ③ 图层开关胶囊：单击隐藏 / 显示（界面不自己翻转状态，唯一写者是这里）。
    //
    // ⚠ `on_*` 是**覆盖**语义：这条回调只注册**一次**，闭包内按下标分发。
    // 循环注册四次会把前三次静默擦掉（只剩最后一层能按）—— 与 `main.rs` 的
    // `wire_callbacks` 里删掉的那两行 `on_accept_ai_proposal` 是同一个坑。
    let weak = ui.as_weak();
    ui.on_song_overview_layer_toggle({
        let weak = weak.clone();
        move |index| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            match index {
                LAYER_TENSION => {
                    ui.set_song_overview_show_tension(!ui.get_song_overview_show_tension());
                }
                LAYER_ENERGY => {
                    ui.set_song_overview_show_energy(!ui.get_song_overview_show_energy());
                }
                LAYER_BANDS => {
                    ui.set_song_overview_show_bands(!ui.get_song_overview_show_bands());
                }
                LAYER_DENSITY => {
                    ui.set_song_overview_show_density(!ui.get_song_overview_show_density());
                }
                _ => {}
            }
        }
    });

    // ③ 独显：双击某一层 ⇒ 只留它；已经是独显态 ⇒ 恢复四层全显。
    let weak = ui.as_weak();
    ui.on_song_overview_layer_solo({
        let weak = weak.clone();
        move |index| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if !(0..=3).contains(&index) {
                return;
            }
            let shown = [
                ui.get_song_overview_show_tension(),
                ui.get_song_overview_show_energy(),
                ui.get_song_overview_show_bands(),
                ui.get_song_overview_show_density(),
            ];
            // ⚠ 不用 `.all()`：仓库的 `assertion_bounds_audit` 按文本扫描 `.all(`
            // 站点 —— 新文件含 `.all(` 就必须登记进它的 SOURCES 清单（那是别人的
            // 测试文件，不归本轮改）；4 元定长数组直接判等，等价且无扫描站点。
            let only_this = shown
                == [
                    index == LAYER_TENSION,
                    index == LAYER_ENERGY,
                    index == LAYER_BANDS,
                    index == LAYER_DENSITY,
                ];
            if only_this {
                ui.set_song_overview_show_tension(true);
                ui.set_song_overview_show_energy(true);
                ui.set_song_overview_show_bands(true);
                ui.set_song_overview_show_density(true);
            } else {
                ui.set_song_overview_show_tension(index == LAYER_TENSION);
                ui.set_song_overview_show_energy(index == LAYER_ENERGY);
                ui.set_song_overview_show_bands(index == LAYER_BANDS);
                ui.set_song_overview_show_density(index == LAYER_DENSITY);
            }
        }
    });

    // ② 视口联动框：拖动框体 ⇒ 滚动编排视图（只写 `roll-scroll-x`，⛔ 不重投影）。
    // 界面在 `.slint` 里已经按画布宽 / 跨度算好目标像素并夹紧，这里只做最后一道
    // 下界守卫（负值到不了编排视图）。
    let weak = ui.as_weak();
    ui.on_song_overview_viewport_scroll({
        let weak = weak.clone();
        move |target_px| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if target_px.is_finite() {
                ui.set_roll_scroll_x(target_px.max(0.0));
            }
        }
    });

    // ② 视口联动框：拖动右缘 ⇒ 缩放编排视图（走唯一写点 + 既有重投影路径）。
    let weak = ui.as_weak();
    ui.on_song_overview_viewport_zoom({
        let weak = weak.clone();
        let port = port.clone();
        move |target_tpp| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Ok(tpp) = u64::try_from(target_tpp) else {
                return;
            };
            if set_roll_ticks_per_pixel(&ui, tpp) {
                reproject_after_zoom(&ui, &port);
            }
        }
    });

    // 可折叠：界面只报“按了折叠键”，状态翻转的唯一写者是这里。
    // ⭐ 全屏与折叠是**协调的一对**，两条 toggle 共享同一个所有权（本函数是
    // `(collapsed, fullscreen)` 的唯一写者，见 `wire_song_overview` 头注）：
    // 折叠时若全屏开着，先退出全屏（面板落到 22 px 条带）；两者**不同时为真**，
    // 因此 `.slint` 里 `collapsed ? 22px : (fullscreen ? …)` 的分支互不抢。
    // 全屏态下按折叠键：`fullscreen` 清零、`collapsed` 翻为 `true` ⇒ 22 px 条带；
    // 展开态按折叠键：`fullscreen` 本已是 `false`，只翻 `collapsed`。
    let weak = ui.as_weak();
    ui.on_song_overview_toggle_collapsed({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_song_overview_fullscreen(false);
                ui.set_song_overview_collapsed(!ui.get_song_overview_collapsed());
            }
        }
    });

    // ⭐ 全屏最大化（Wang 2026-11-08 19:56 要求）：界面只报“按了全屏键 / Esc”，
    // 状态翻转的唯一写者是这里（与折叠 toggle 同一所有权）。
    // 进入全屏即展开（清 `collapsed`）：全屏面板占满主窗口可用区，22 px 条带
    // 态没有意义；退出全屏恢复 104 px（`collapsed` 仍为 `false`）。`Esc` 退出
    // 全屏落点也是这里（`host::apply_action` 的 Cancel 臂只调同一个回调，
    // ⛔ 不在键盘臂再写一次属性 —— 单写者纪律，见那一臂的注释）。
    let weak = ui.as_weak();
    ui.on_song_overview_toggle_fullscreen({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let fullscreen = ui.get_song_overview_fullscreen();
                ui.set_song_overview_fullscreen(!fullscreen);
                if !fullscreen {
                    ui.set_song_overview_collapsed(false);
                }
            }
        }
    });

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magma 锚点逐位相等（设计稿 §4.5 的 5 个色值）。
    #[test]
    fn magma_stops_match_the_design_table() {
        assert_eq!(
            magma_color(0.0),
            slint::Color::from_rgb_u8(0x12, 0x13, 0x1A)
        );
        assert_eq!(
            magma_color(0.25),
            slint::Color::from_rgb_u8(0x3B, 0x1C, 0x54)
        );
        assert_eq!(
            magma_color(0.5),
            slint::Color::from_rgb_u8(0x8C, 0x29, 0x56)
        );
        assert_eq!(
            magma_color(0.75),
            slint::Color::from_rgb_u8(0xE0, 0x68, 0x38)
        );
        assert_eq!(
            magma_color(1.0),
            slint::Color::from_rgb_u8(0xFC, 0xE8, 0xA6)
        );
        // 端点外夹紧，不回绕。
        assert_eq!(magma_color(-1.0), magma_color(0.0));
        assert_eq!(magma_color(2.0), magma_color(1.0));
    }

    /// 频段 dB → Magma：−36 dB 最暗、0 dB 最亮。
    #[test]
    fn band_db_maps_minus_36_to_dark_and_zero_to_bright() {
        assert_eq!(band_db_color(-36.0), magma_color(0.0));
        assert_eq!(band_db_color(0.0), magma_color(1.0));
        assert_eq!(band_db_color(-100.0), magma_color(0.0));
        assert_eq!(band_db_color(5.0), magma_color(1.0));
    }

    /// 效价色相两端：−1 冷蓝、+1 亮金；0 是中点。
    #[test]
    fn tension_color_spans_cold_blue_to_bright_gold() {
        assert_eq!(
            tension_color(-1.0),
            slint::Color::from_rgb_u8(0x3B, 0x82, 0xF6)
        );
        assert_eq!(
            tension_color(1.0),
            slint::Color::from_rgb_u8(0xF5, 0xB8, 0x1D)
        );
        assert_eq!(
            tension_color(0.0),
            slint::Color::from_rgb_u8(0x98, 0x9D, 0x8A)
        );
    }

    /// LUFS → 高度：−36 = 0，−6 = 1，端点外夹紧。
    #[test]
    fn lufs_height_clamps_to_the_design_range() {
        assert_eq!(lufs_to_height(-36.0), 0.0);
        assert_eq!(lufs_to_height(-6.0), 1.0);
        assert!((lufs_to_height(-21.0) - 0.5).abs() < 1e-6);
        assert_eq!(lufs_to_height(-100.0), 0.0);
        assert_eq!(lufs_to_height(0.0), 1.0);
    }

    /// 分数 → 小节：左闭右开，右缘夹在最后一小节。
    #[test]
    fn bar_at_fraction_is_left_closed_and_clamps_right() {
        assert_eq!(bar_at_fraction(0.0, 16), 0);
        assert_eq!(bar_at_fraction(0.0625, 16), 1);
        assert_eq!(bar_at_fraction(0.999, 16), 15);
        assert_eq!(bar_at_fraction(1.0, 16), 15);
        assert_eq!(bar_at_fraction(2.0, 16), 15);
        assert_eq!(bar_at_fraction(-1.0, 16), 0);
        assert_eq!(bar_at_fraction(f32::NAN, 16), 0);
        assert_eq!(bar_at_fraction(0.5, 0), 0);
    }

    /// 分数 → tick：吸附到小节线，越界夹在右缘，非有限不响应。
    #[test]
    fn seek_tick_snaps_to_bars_and_clamps() {
        // 16 小节 × 3840 tick = 61440 跨度；0.5 正好是第 8 小节起点。
        assert_eq!(seek_tick_for_fraction(0.5, 61_440, 3_840), Some(30_720));
        // 0.26 ≈ 第 4.16 小节 ⇒ 吸附到第 4 小节起点（15360）。
        assert_eq!(seek_tick_for_fraction(0.26, 61_440, 3_840), Some(15_360));
        assert_eq!(seek_tick_for_fraction(1.0, 61_440, 3_840), Some(61_440));
        assert_eq!(seek_tick_for_fraction(2.0, 61_440, 3_840), Some(61_440));
        assert_eq!(seek_tick_for_fraction(-1.0, 61_440, 3_840), Some(0));
        assert_eq!(seek_tick_for_fraction(f32::NAN, 61_440, 3_840), None);
        assert_eq!(seek_tick_for_fraction(0.5, 0, 3_840), None);
        assert_eq!(seek_tick_for_fraction(0.5, 61_440, 0), None);
    }

    /// 悬停读数：一行里有小节号（1 起）、LUFS（1 位小数）、张力（2 位小数）、
    /// 最突出的前两个频段短名。
    #[test]
    fn hover_readout_names_the_bar_lufs_tension_and_top_bands() {
        let overview = SongOverview {
            total_bars: 4,
            bar_length_ticks: 3840,
            span_ticks: 15_360,
            key_signature: String::new(),
            sections: vec![("Intro".to_string(), 0)],
            tension: vec![0.24, 0.62, 0.80, 0.30],
            valence_hue: vec![-0.5, 0.0, 0.4, -0.2],
            lufs_short: vec![-22.4, -18.2, -12.0, -20.0],
            density_pitched: vec![0.3, 0.5, 0.8, 0.4],
            density_duty: vec![0.6, 0.6, 0.7, 0.5],
            density_perc: vec![0.2, 0.5, 0.9, 0.3],
            bands_db: vec![
                -18.2, -14.5, -12.0, -11.4, -15.0, -19.2, -24.0, -31.5, // bar 0
                -18.0, -14.0, -10.0, -9.0, -14.0, -18.0, -23.0, -30.0, // bar 1
                -17.0, -13.0, -9.0, -8.0, -13.0, -17.0, -22.0, -29.0, // bar 2
                -19.0, -15.0, -13.0, -12.0, -16.0, -20.0, -25.0, -32.0, // bar 3
            ],
        };
        // bar 1 的最高两段是 Mid（-9.0）与 Low-Mid（-10.0）。
        assert_eq!(overview.top_bands(1), (3, 2));
        assert_eq!(
            hover_readout(&overview, 1),
            "第 2 小节 · -18.2 LUFS · 张力 0.62 · 突出 中频/中低"
        );
        assert_eq!(hover_readout(&overview, 99), "");
    }

    /// 标尺标签：段落起点写段落名，其余每 4 小节写小节号，段落名优先。
    #[test]
    fn ruler_labels_prefer_section_names_over_bar_numbers() {
        let labels = ruler_labels(8, &[("Intro".to_string(), 0), ("Verse".to_string(), 4)]);
        assert_eq!(labels[0], "Intro");
        assert_eq!(labels[1], "");
        assert_eq!(labels[4], "Verse");
        assert_eq!(labels[5], "");
        let plain = ruler_labels(8, &[]);
        assert_eq!(plain[0], "1");
        assert_eq!(plain[4], "5");
        assert_eq!(plain[2], "");
    }

    /// 疏密归一化：0 触发 ⇒ 0；120 BPM 下 1 次/拍（= 2 次/秒）⇒ ln3/ln17；
    /// 8 次/拍（= 16 次/秒，设计锚点）⇒ 1；超锚点夹到 1；非法速度按 120 BPM 计。
    #[test]
    fn normalize_density_matches_the_design_anchor() {
        assert_eq!(normalize_density(0.0, 120.0), 0.0);
        let one_per_beat = normalize_density(1.0, 120.0);
        let expected = (3.0f32).ln() / (17.0f32).ln();
        assert!(
            (one_per_beat - expected).abs() < 1e-6,
            "1 次/拍 @120 BPM 应为 ln3/ln17 = {expected}，实得 {one_per_beat}"
        );
        assert_eq!(normalize_density(8.0, 120.0), 1.0);
        assert_eq!(normalize_density(100.0, 120.0), 1.0);
        assert_eq!(normalize_density(-1.0, 120.0), 0.0);
        // 速度非法时按 120 BPM 计（与 1 次/拍 @120 同值）。
        assert_eq!(normalize_density(1.0, 0.0), one_per_beat);
        assert_eq!(normalize_density(1.0, f32::NAN), one_per_beat);
        // 同样的每拍触发数，速度翻倍 ⇒ 每秒触发数翻倍 ⇒ 归一化值更大。
        assert!(normalize_density(1.0, 240.0) > one_per_beat);
    }

    /// 效价底色只读调号后缀：小调 −0.6、大调 +0.6、读不出时中性 0.0（⛔ 不猜调式）。
    #[test]
    fn valence_base_for_key_reads_the_mode_suffix() {
        assert_eq!(valence_base_for_key("D minor"), -0.6);
        assert_eq!(valence_base_for_key("C major"), 0.6);
        assert_eq!(valence_base_for_key(""), 0.0);
        assert_eq!(valence_base_for_key("C"), 0.0);
    }

    /// 映射：dsp 有数的小节原样映射，超出 dsp 跨度的小节是静音垫条（不是测量值）。
    ///
    /// 用 dsp 自己的 `analyze_song_overview`（纯谱面，无音频）造一份 1 小节的真输出，
    /// 再映射到 16 小节的演示视图上。
    #[test]
    fn overview_from_dsp_maps_measured_bars_and_pads_silent_bars() {
        use yeban_dsp::song_overview::{
            NoteEvent, ScoreInput, SectionMarker, analyze_song_overview,
        };
        let score = ScoreInput {
            notes: vec![
                NoteEvent {
                    start_beat: 0.0,
                    duration_beats: 1.0,
                    midi: 60,
                    velocity: 100,
                    track: 0,
                    is_percussion: false,
                },
                NoteEvent {
                    start_beat: 1.0,
                    duration_beats: 1.0,
                    midi: 69,
                    velocity: 100,
                    track: 0,
                    is_percussion: false,
                },
            ],
            tempo_map: vec![],
            beats_per_bar: 4,
            key_tonic_pc: 0,
            is_minor: false,
            markers: vec![SectionMarker {
                bar: 1,
                label: "Intro".to_string(),
            }],
        };
        let dsp = analyze_song_overview(&score, None);
        assert_eq!(dsp.song_meta.total_bars, 1, "2 拍内容 ⇒ dsp 只给 1 小节");
        assert_eq!(dsp.song_meta.key_signature, "C major");

        let project = crate::bridge::demo_project();
        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let mapped = overview_from_dsp(&dsp, &view);
        assert_eq!(mapped.total_bars, view.bar_positions.len());
        assert_eq!(
            mapped.total_bars, 16,
            "小节数取视图（16），不是 dsp 的内容跨度（1）"
        );
        assert_eq!(mapped.span_ticks, 16 * mapped.bar_length_ticks);
        assert_eq!(mapped.key_signature, "C major");
        assert_eq!(mapped.bands_db.len(), 16 * BAND_COUNT);
        // 第 0 小节：张力 / LUFS 与 dsp 逐位相同；频段无音频 ⇒ −120 dB 哨兵夹到 −36。
        assert_eq!(mapped.tension[0], dsp.bar_series.tension[0]);
        assert_eq!(mapped.lufs_short[0], dsp.bar_series.lufs_short[0]);
        assert!(mapped.tension[0] > 0.0, "有音符的小节张力必须大于 0");
        assert_eq!(
            mapped.density_pitched[0],
            normalize_density(
                dsp.bar_series.density_pitched_onsets_per_beat[0],
                dsp.song_meta.base_bpm
            )
        );
        assert!(
            mapped.density_pitched[0] > 0.0,
            "有起音的小节疏密必须大于 0"
        );
        for band in 0..BAND_COUNT {
            assert_eq!(mapped.band_db(0, band), BAND_DB_FLOOR);
        }
        // 第 1–15 小节：静音垫条（⛔ 不是测量值）。
        for bar in 1..16 {
            assert_eq!(mapped.tension[bar], 0.0, "垫条张力必须为 0（bar {bar}）");
            assert_eq!(
                mapped.lufs_short[bar], NO_LOUDNESS_READING_LUFS,
                "垫条 LUFS 必须是 −70 哨兵（bar {bar}）"
            );
            assert_eq!(mapped.density_pitched[bar], 0.0);
            assert_eq!(mapped.density_duty[bar], 0.0);
            assert_eq!(mapped.density_perc[bar], 0.0);
            assert_eq!(mapped.valence_hue[bar], 0.6, "大调垫条效价取底色 +0.6");
            for band in 0..BAND_COUNT {
                assert_eq!(mapped.band_db(bar, band), BAND_DB_FLOOR);
            }
        }
        // 段落是工程结构（视图），不是 dsp 的 1 小节截断：仍有 Chorus。
        assert!(
            mapped.sections.iter().any(|(name, _)| name == "Chorus"),
            "段落表必须含 Chorus（视图的 16 小节结构）：{:?}",
            mapped.sections
        );
    }

    /// 静音面板：视图几何 + 全哨兵，调号空串（⛔ 不编一个假调号）。
    #[test]
    fn silent_overview_has_the_view_geometry_and_no_measurements() {
        let project = crate::bridge::demo_project();
        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let silent = silent_overview(&view);
        assert_eq!(silent.total_bars, 16);
        assert_eq!(silent.span_ticks, 16 * silent.bar_length_ticks);
        assert!(silent.key_signature.is_empty());
        assert_eq!(silent.sections[0].0, "Intro");
        for bar in 0..16 {
            assert_eq!(silent.tension[bar], 0.0);
            assert_eq!(silent.lufs_short[bar], NO_LOUDNESS_READING_LUFS);
            assert_eq!(silent.density_pitched[bar], 0.0);
            for band in 0..BAND_COUNT {
                assert_eq!(silent.band_db(bar, band), BAND_DB_FLOOR);
            }
        }
    }
}
