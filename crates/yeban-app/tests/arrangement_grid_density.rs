//! ⭐ **编曲网格密度自适应** —— 五版判据（Wang 2026-11-17 14:37 按 Logic Pro
//! 实测规格定）。
//!
//! ## 规格来源（父 agent 对 Wang 所发两张 Logic 截图逐像素实测，Retina 2x，
//! 以下均为逻辑像素）
//!
//! * 图一（bar 宽 48px）：标尺刻度全 1px 宽；小节刻度满刻度行高、拍刻度约
//!   半行（下对齐）；编号每 2 小节一个；车道只有小节线（1px），无拍线。
//! * 图二（bar 宽约 7.5px，极端缩小）：编号每 16 小节；线条单位粗化为每 2
//!   小节一条（线距 15px），仍 1px；无更细刻度。
//! * 拍内细分刻度在 Logic 高缩放时出现（Wang 另行明确）。
//!
//! ## 被钉的读数（`host::grid_density`，纯函数）
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① 三档实测复现 | 48px ⇒ 编号步长 2／单位 1／拍刻度在／车道无拍线／无细分；192px ⇒ 步长 1／拍线在／细分 4；7.5px ⇒ 步长 16／单位 2／无拍级 |
//! | ② 阈值边界 | 96px 编号间距、15px 线距、每拍 8px（拍刻度与细分距）、每拍 24px（车道拍线，推定值）四条边界的内外各一档 |
//! | ③ 非 4/4 推广 | 3/4 与 6/8 按拍号分子算每拍宽，⛔ 不写死 4 |
//! | ④ 退化输入 | 小节宽 0／负／NaN／无穷 ⇒ 最保守读数（步长／单位 1、无拍刻度、无细分），不 panic |
//! | ⑤ 接线与色值 | 宿主五写入点与 `.slint` 五属性消费点都在；网格五色齐备、金色已清零；五色对各自背景的 WCAG 实算比值不低于各自下限 |
//!
//! 像素侧的线宽（全 1px）由 1:1 渲染帧逐像素实测看守（五版对照帧
//! `compare5-*`），本文件只钉**函数读数与接线**，⛔ 不冒充像素证据。
//!
//! ## 六版增补（Wang 2026-11-17 15:06 终判 + 15:09 追加）
//!
//! * 车道拍线再淡一档：`#687685` → `#5d6a76`，对最不利行底 `#181719`
//!   实算 3.22:1，钉在目标窗 3.15–3.30 内（判据 ⑤ 同步换钉 + 窗断言；
//!   硬下限 ≥3.0 不放宽）。
//! * 标尺编号净空：编号只占标尺带上部 9px 净空区，编号区底与每档刻度
//!   顶之间 >= 2px（判据 ⑥ 钉 `.slint` 几何常量；刻度行 12px 与三档
//!   分级 12/6/3 不动）。
//! * 播放头竖线改纯白 `#ffffff`（落点在 `arrangement.slint` 播放头
//!   覆盖层，不在本文件覆盖面）。
//!
//! ## 七版增补（Wang 2026-11-17 15:49 + 15:51 亲定）
//!
//! * 钢琴卷帘播放头同步改纯白 `#ffffff`（判据 ⑦ 钉 `piano_roll.slint`
//!   落点与对最深行底 `#161b26` 的比值）。
//! * 编曲区行底分界线提亮定值 `#5a6672`：对最不利行底 `#181719`
//!   实算 3.04:1，钉在目标窗 3.0–3.1 内，且相对亮度须低于拍线
//!   `#5d6a76`（判据 ⑦）。
//! * 轨道选中边框统一纯白、选中底改定值（行底 `#131a20`／头底
//!   `#16232b`，判据在 `selection_highlight_uses_accent.rs` 换钉）；
//!   本文件判据 ⑦ 另钉小节线/拍线对新选中行底的比值不破既有下限。
//!
//! ## 八版增补（Wang 2026-11-17 16:08 + 16:09 亲定，照 Logic Pro）
//!
//! * 选中改 Logic 式：纯白边框作废，选中只由提亮承载——行底
//!   `#0f1218`、头底 `#293645`（Logic 亮度比 1.80 线性提亮；判据在
//!   `selection_highlight_uses_accent.rs` 换钉）；本文件判据 ⑦ 另钉
//!   小节线/拍线对新选中行底的实算比值（小节线 4.83、拍线 3.38）。
//! * 行底分界线再深一档：`#5a6672` → `#4e5964`，对最不利行底
//!   `#181719` 实算 2.50:1，钉在目标窗 2.3–2.7 内，仍暗于拍线
//!   `#5d6a76`（判据 ⑦）。`#181719` 不再是选中行底，八版起只作
//!   保守最亮参考底保留（实算比值取最不利情形）。
//! * 片段底改纯色：编曲区片段块外层底改为不透明 `#090b10`（判据 ⑦
//!   后新立判据钉住）；Clip 矩阵槽底、下坞轨道徽章底范围外，只列
//!   清单不动。
//! * 片段内标记改白色系实色（16:23 追加）：波形峰值柱与 MIDI 音符条
//!   改纯白 `#ffffff`（对最不利片段实色底实算 14.71:1），波形基准线
//!   与 MIDI 分界线改灰实色 `#748392`（弱于纯白、分主次）。
//! * 和弦块底实色化（16:23 追加）：五块和弦底（原 α≈0.09）与边框
//!   （原 α≈0.21）全换合成等效实色（叠和弦轨带底 `#0d1017` 的呈现
//!   色），hover 同法；和弦名文字对新实底实算 7.56–8.79:1。
//!
//! ## 九版增补（Wang 2026-11-17 16:43 亲定，照 Logic Pro 区域）
//!
//! * 片段底改轨道色纯色本体：八版「合成等效实色」= 0.12×轨道色 + 0.88×近黑，呈现为轨道色混黑、暗且脏，废。底由投影层 `bridge::clip_base_color` 算（`blk.base-color`）：纯色不混黑；仅当白色标记对纯色底不足 3.05 时按统一规则只降 HSL 明度（每步 0.02，色相/饱和不动）到达标。判据 ⑧ 换钉落点 + 全调色板普查（产品自带/自分派轨道色 + 默认色 + 白，逐色钉落码值并实算三项下限）。
//! * 同色边框去除：未选中片段 `trk.color` α0.55 边框整圈去掉（宽 0）；选中片段 2px 纯白边框保留（判据 ⑧）。
//! * 片段内文字/导线改投影推导：文字 `blk.text-color`（近白→纯白→纯黑，第一达 4.5 者）、导线 `blk.guide-color`（灰 `#748392` 达 3.0 则灰，否则黑/白取高者）；纯白标记沿用八版（判据 ⑨）。
//! * 和弦块同口径改纯色：五块底 = 和弦色本体，hover = 同色明度 ±0.04（方向取文字仍 >= 4.5 的一侧），边框 = 同色更深一档实色（-0.10 / hover -0.05）；文字按底色选近白/纯白（判据 ⑨）。
//!
//! ## 十版增补（Wang 2026-11-17 18:28 亲定，照 Logic Pro 截图逐像素实测）
//!
//! * 标记改同色相派生色：纯白 `#FFFFFF` 标记太刺眼（Logic 波形色
//!   不是纯白也不是固定色）。波形峰值柱/MIDI 音符条/波形基准线/
//!   MIDI 分界线改由投影层 `bridge::clip_marker_color` 派生：轨道色
//!   HSV（H 不变、V = 0.99、S × 1/3），即底色的浅色版（Logic 实测
//!   对：蓝 `#4A80CE`→`#C6DEFC`、紫 `#6F3CCC`→`#D7C4FB`、品红
//!   `#9724C2`→`#E6B3FB`，对底 2.9–5.2:1）。判据 ⑧ 普查逐色钉
//!   派生落码值（精确值断言取代九版导线 >= 3.0 地板——派生规则本身
//!   不承诺该地板，Logic 实测下限 2.9）；判据 ⑨ 纯白落点 4 → 2
//!   （只剩播放头线/菱形头，Wang 亲定纯白不动）。片段名文字维持
//!   九版规则（近白优先，>= 4.5），与 Logic 分工一致。和弦块内无
//!   波形/音符类标记（只有名称文字），本项不波及。
//! * 车道底抬升：未选中车道底 `#090b10`（相对亮度 0.0033）→
//!   `#2e2e2e`（0.0273，Logic 编曲区底实测同值），只落车道容器，
//!   `Theme.bg-canvas` 本体与标尺带 `Theme.bg-panel` 不动（标尺带
//!   与车道底不共用令牌）。线色按新底重算、比率目标不变：拍线
//!   `#828f9c`（对最不利底 = 选中行 3.21，窗 3.15–3.30；对未选中
//!   行 4.11）、小节线 `#a7b0ba`（4.82 / 6.18，下限 4.5）、分界线
//!   `#6d7d8c`（2.50，窗 2.3–2.7；对未选中行 3.21，仍暗于拍线）。
//!   选中行底维持八版相对提亮比 1.80：`#383f4c`（相对亮度
//!   0.0492 = 1.80 × 0.0273）；选中头底 `#293645` 不动（头底基色
//!   `#1d2733` 未变）。九版起充当保守最亮参考底的 `#181719` 退役
//!   ——新底组（选中行）本身即最不利底。
//!
//! ## 十一版增补（Wang 2026-11-17 18:51 + 18:52 + 18:53 亲定）
//!
//! * 车道底回蓝调抬亮：十版中性灰 `#2e2e2e` 与全应用深蓝色系
//!   （轨道头 `#1d2733`、标尺带）差异太大 ⇒ 改回九版 `#090b10`
//!   的色相/饱和（HSV 222.9° / 0.4375）只抬明度：定值 `#1d2333`
//!   （相对亮度 0.0170，目标带 0.014–0.020，仍低于十版 0.0273）。
//!   落点仍只落车道容器，`Theme.bg-canvas` 本体与标尺带不动。
//! * 选中行底维持相对提亮比 1.80 按新基底重算：`#273146`
//!   （相对亮度 0.0307 = 1.80 × 0.0170）；选中头底 `#293645`
//!   不动。线色按新底重算、比率目标不变：拍线 `#74808b`（对最
//!   不利底 = 选中行 3.22，窗 3.15–3.30；对未选中行 3.88）、
//!   小节线 `#979fa8`（4.86 / 5.85，下限 4.5）。
//! * 分界线改暗线（18:52，推翻八版以来亮线方向）：取比车道底
//!   更暗的同色系深色 `#040508`（相对亮度 0.0015 < 两底），暗线
//!   对两底实算 1.30（未选中）/ 1.57（选中）。⚠ Wang 原目标窗
//!   2.3–2.7 在暗方向物理不可达：WCAG 对比含 +0.05 眩光项，
//!   车道底亮度 0.0170 时纯黑线对未选中底上限仅 1.34:1；判据按
//!   实算紧窗换钉（1.28–1.32 / 1.54–1.59）并断言方向（分界线
//!   相对亮度低于两底）。层级按绝对对比仍 分界线 < 拍线 < 小节线。
//! * 十版派生标记（`bridge::clip_marker_color`）原样保留，判据
//!   ⑧ 不动；片段名文字规则、播放头纯白、选中片段 2px 纯白边
//!   均不动。
//!
//! ## 合入裁决甲（Wang 2026-11-17 19:58）
//!
//! * 编曲区走质感三元：十一版全部字面定值只落扁平支，拟物支用
//!   main 的令牌形态。本文件受影响的两钉改钉三元全串：⑦ 车道底
//!   （`background: Tokens.texture-physical ? transparent :
//!   #1d2333;`——拟物支 transparent 复现 main 的无底色形态）与
//!   ⑦ 分界线落点（`Tokens.texture-physical ? Tokens.divider-row
//!   : #040508`）。色值本身的亮度/比值断言（⑤⑦ 的实算窗）不变，
//!   强度不降；⑤ 的五色存在钉在扁平支原样成立。
//!
//! ## 重设计补丁包第四轮改钉（Wang 2026-11-18 定：判据按本补丁包修改）
//!
//! * 网格色全部迁入 `tokens.slint` 的 `skin-grid-*`（车道）/
//!   `ruler-tick-*`（标尺）令牌，双主题各一份值；`arrangement.slint`
//!   只消费令牌。⑤ 的存在钉改钉令牌定义，⑥ 的刻度定位改按令牌串。
//! * 网格线改 Logic 式低反差：十一版线色 `#979fa8` / `#74808b`
//!   删除，拍线 ≥3:1 / 小节线 ≥4.5:1 的下限对本包网格线不再适用；
//!   ⑤⑦ 的比值断言改按补丁令牌实测值钉紧窗（车道小节 1.31 / 拍
//!   1.16 对扁平车道底；标尺小节 4.43 / 拍 3.50 / 细分 3.18 对扁平
//!   标尺带底）。层级方向（分界线 < 拍线 < 小节线）与几何断言
//!   （⑥ 刻度 12/6/3 与净空）强度不降。
//! * 车道底改走 `Tokens.bg-lane`（扁平 `#2b3036` / 拟物 `#1d2126`）；
//!   选中行改 accent α0.06 透叠（落点在 `selection_highlight` 判据），
//!   本文件按 sRGB 合成有效色（扁平 `#2f363f` / 拟物 `#222a2e`）实算。
//! * 分界线改走 `Tokens.skin-grid-row`，暗线方向保留（两主题均暗于
//!   车道底），比值改实测窗（扁平 1.13 / 拟物 1.09）。
//!   （2026-11-18 夜：Logic13 强度层落地，使用处改走 `grid-*`，
//!   `skin-grid-*` 退为基色；⑦⑨ 落点钉随架构改钉，见各判据内注。）
//! * 播放头（编曲 + 卷帘）改走 `Tokens.skin-text-strong`，纯白清零；
//!   ⑦ 卷帘播放头比值改实测窗（扁平 11.63 / 拟物 13.75），⑨ 纯白
//!   计数 2 → 0、skin-text-strong 计数钉 2。
//! * 和弦块底改走 `Theme.track-*` 本体 + `.brighter(0.18)` hover、
//!   边框 `.brighter(0.08)` / `.darker(0.12)`；和弦名文字
//!   （`skin-text-strong`）对常态底的 ≥4.5 下限按 Wang 口径以补丁
//!   取值合入，⑨ 改钉实测窗（扁平 3.93/4.17/3.65/3.70、拟物
//!   2.51/3.16/2.79/2.79）。
//!
//! ## 判据卫生（2026-11-19）
//!
//! * ⑤⑦ 静态实算块普查：⑦ 分界线块写死的 `#23272c` / `#16191d`
//!   与网格线退役色 `#3c424a` / `#343a42` / `#2a3037` /
//!   `#242930`、⑤ 标尺带底 `#262a2f`、⑦ 拟物车道底 `#1d2126`
//!   与卷帘底 `#20242a` 均已与现行令牌脱钩（只算字面值、不验
//!   产品），属死块；其余块字面值与现行令牌逐一相符或由产品
//!   代码派生，属活断言，不动。
//! * 死块按现行强度链重算并改真断言：新增 `token_pair` 按令牌名
//!   从 `tokens.slint` 解析双主题现行值；分界线/网格线按默认
//!   强度 0（grid-fade 0.55）算有效色。分界线方向随现行基色
//!   翻转：`skin-grid-row` 基色本比车道底亮半档，rev11 暗线
//!   方向作废，改断言亮线方向与层级（分界线 < 拍线 < 小节线，
//!   两主题）。实算要点：标尺三色对现行 bg-shell 扁平 `#30343b`
//!   为 3.84 / 3.03 / 2.75（旧窗按退役底空算 4.43 / 3.50 /
//!   3.18）；选中行拟物合成改 `#222829`（现行 bg-lane 拟物
//!   `#1d1f21`）；播放头拟物比值改 14.08（现行 bg-panel-alt
//!   拟物 `#202224`）。

use yeban_app::host::{GridDensity, grid_density};

/// 一行断言一组读数（五个字段逐项比，失败时整组打印便于定位）。
fn expect(bar_width_px: f32, beats_per_bar: i32, want: GridDensity) {
    let got = grid_density(bar_width_px, beats_per_bar);
    assert_eq!(
        got, want,
        "grid_density({bar_width_px}, {beats_per_bar}) 读数不符"
    );
}

/// ① 两张 Logic 截图的三档缩放必须逐项复现。
#[test]
fn logic_measured_tiers_are_reproduced() {
    // 图一档：bar 48px、每拍 12px。编号每 2 小节；车道无拍线；标尺拍刻度在；
    // 细分距 12/4 = 3px < 8px ⇒ 无细分。
    expect(
        48.0,
        4,
        GridDensity {
            label_stride_bars: 2,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: false,
            subdiv_per_beat: 0,
        },
    );
    // 高缩放档：bar 192px、每拍 48px。编号每小节；车道拍线在；细分 4 份
    // （细分距 12px >= 8px）。
    expect(
        192.0,
        4,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 4,
        },
    );
    // 图二档：bar 7.5px。编号每 16 小节（16 × 7.5 = 120 >= 96，8 × 7.5 = 60
    // 不够）；线条单位 2 小节（线距 15px）；无拍刻度、无车道拍线、无细分。
    expect(
        7.5,
        4,
        GridDensity {
            label_stride_bars: 16,
            line_unit_bars: 2,
            show_beat_ticks: false,
            show_lane_beats: false,
            subdiv_per_beat: 0,
        },
    );
    // 默认帧档：bar 128px（默认缩放 30 tick/px）、每拍 32px。细分 4 份
    // （32/4 = 8 >= 8 恰达标）。
    expect(
        128.0,
        4,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 4,
        },
    );
}

/// ② 四条阈值的边界内外。
#[test]
fn density_threshold_boundaries() {
    // 编号间距 96px：恰达标 ⇒ 步长 1；差 1px ⇒ 步长 2。
    expect(
        96.0,
        4,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 2,
        },
    );
    assert_eq!(grid_density(95.0, 4).label_stride_bars, 2);
    // 线距 15px：恰达标 ⇒ 单位 1；不足 ⇒ 单位 2。
    assert_eq!(grid_density(15.0, 4).line_unit_bars, 1);
    assert_eq!(grid_density(14.9, 4).line_unit_bars, 2);
    // 每拍 8px（拍刻度）：恰达标在场；不足退场且无细分。
    let at = grid_density(32.0, 4);
    assert!(at.show_beat_ticks && at.subdiv_per_beat == 0);
    let below = grid_density(31.9, 4);
    assert!(!below.show_beat_ticks && below.subdiv_per_beat == 0);
    // 每拍 24px（车道拍线）：bar 96px 时每拍恰 24px ⇒ 在场（上已断言）；
    // bar 95px 时每拍 23.75px ⇒ 退场。
    assert!(!grid_density(95.0, 4).show_lane_beats);
    // 细分逐档退：每拍 16px ⇒ 2 份（16/2 = 8 达标、16/4 = 4 不达标）；
    // 每拍 12px ⇒ 0 份（12/2 = 6 < 8）。
    assert_eq!(grid_density(64.0, 4).subdiv_per_beat, 2);
    assert_eq!(grid_density(48.0, 4).subdiv_per_beat, 0);
    // 单位 >= 2 时拍级全退（即使每拍宽本身达标也不适用——单位由小节宽定）。
    let coarse = grid_density(14.0, 4);
    assert_eq!(coarse.line_unit_bars, 2);
    assert!(!coarse.show_beat_ticks && !coarse.show_lane_beats);
    assert_eq!(coarse.subdiv_per_beat, 0);
}

/// ③ 拍号分子推广：每拍宽 = 小节宽 / 分子，⛔ 不写死 4。
#[test]
fn density_generalizes_beyond_four_four() {
    // 3/4、bar 48px：每拍 16px ⇒ 拍刻度在、车道拍线不在、细分 2 份。
    expect(
        48.0,
        3,
        GridDensity {
            label_stride_bars: 2,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: false,
            subdiv_per_beat: 2,
        },
    );
    // 6/8、bar 192px：每拍 32px ⇒ 与 4/4 默认帧同读数（细分 4 份）。
    expect(
        192.0,
        6,
        GridDensity {
            label_stride_bars: 1,
            line_unit_bars: 1,
            show_beat_ticks: true,
            show_lane_beats: true,
            subdiv_per_beat: 4,
        },
    );
    // 步长与单位只由小节宽定，与拍号无关：7.5px 在 3/4 下同为 16 / 2。
    let narrow = grid_density(7.5, 3);
    assert_eq!(narrow.label_stride_bars, 16);
    assert_eq!(narrow.line_unit_bars, 2);
    // 分子异常（0）夹到 1 处理，不 panic、不除零。
    assert_eq!(grid_density(48.0, 0), grid_density(48.0, 1));
}

/// ④ 退化输入：小节宽无读数时退回最保守读数。
#[test]
fn density_degenerate_widths_fall_back() {
    let fallback = GridDensity {
        label_stride_bars: 1,
        line_unit_bars: 1,
        show_beat_ticks: false,
        show_lane_beats: false,
        subdiv_per_beat: 0,
    };
    for width in [0.0_f32, -5.0, f32::NAN, f32::INFINITY] {
        expect(width, 4, fallback);
    }
}

/// 相对亮度（WCAG 2.1 定义，与 `tests/ui_contrast_tokens.rs` 同式）。
fn relative_luminance(hex: &str) -> f32 {
    let h = hex.trim_start_matches('#');
    let channel = |i: usize| {
        let v = u8::from_str_radix(&h[i..i + 2], 16).expect("合法十六进制") as f32 / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/// 两色对比度（WCAG：`(L1+0.05)/(L2+0.05)`，亮者作分子）。
fn contrast_ratio(a: &str, b: &str) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// sRGB 空间 alpha 合成（与 Slint 渲染一致）：`fg` 以 `alpha` 叠在
/// `bg` 上，返回合成色 hex（逐通道四舍五入）。选中行有效底实算用。
fn alpha_over(fg: &str, alpha: f32, bg: &str) -> String {
    let parse = |hex: &str| {
        let h = hex.trim_start_matches('#');
        [
            u8::from_str_radix(&h[0..2], 16).expect("合法十六进制") as f32,
            u8::from_str_radix(&h[2..4], 16).expect("合法十六进制") as f32,
            u8::from_str_radix(&h[4..6], 16).expect("合法十六进制") as f32,
        ]
    };
    let (f, b) = (parse(fg), parse(bg));
    let mut out = String::from("#");
    for i in 0..3 {
        let v = (f[i] * alpha + b[i] * (1.0 - alpha)).round() as u8;
        out.push_str(&format!("{v:02x}"));
    }
    out
}

/// 从 `tokens.slint` 源码按令牌名读出双主题现行值（拟物，扁平）：
/// 定位 `property <color> {name}:` 的定义段（到第一个 `;`），取
/// 第一个 `texture-physical ?` 之后的两枚 `#rrggbb`（依次为拟物、
/// 扁平）。判据按令牌名取值后，令牌值一改实算即跟随，不再出现
/// 「字面值与令牌脱钩、只算不验」的死块（2026-11-19 判据卫生）。
fn token_pair(tokens: &str, name: &str) -> (String, String) {
    let marker = format!("property <color> {name}:");
    let at = tokens
        .find(&marker)
        .unwrap_or_else(|| panic!("令牌 {name} 定义必须在 tokens.slint"));
    let seg = &tokens[at..];
    let seg = &seg[..seg.find(';').expect("令牌定义以 ; 收尾")];
    let tp = seg
        .find("texture-physical ?")
        .unwrap_or_else(|| panic!("令牌 {name} 须含双主题支"));
    let rest = &seg[tp..];
    let mut found: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rest.len() && found.len() < 2 {
        if rest.as_bytes()[i] == b'#' {
            let hex: String = rest[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if hex.len() == 6 {
                found.push(format!("#{hex}"));
                i += 7;
                continue;
            }
        }
        i += 1;
    }
    assert!(found.len() == 2, "令牌 {name} 双主题值须齐备");
    (found[0].clone(), found[1].clone())
}

/// ⑤ 接线与色值：宿主写入点、`.slint` 消费点、五色与比值下限。
#[test]
fn density_wiring_and_colors_are_in_place() {
    let host = include_str!("../src/host.rs");
    for setter in [
        "ui.set_grid_label_stride(",
        "ui.set_grid_line_unit(",
        "ui.set_grid_show_beat_ticks(",
        "ui.set_grid_show_lane_beats(",
        "ui.set_grid_subdiv_per_beat(",
    ] {
        assert!(host.contains(setter), "host::apply_view 缺写入点 {setter}");
    }
    assert!(
        host.contains("grid_density(bar_width_px"),
        "密度读数必须由 grid_density 按小节宽算出"
    );

    let arrangement = include_str!("../ui/arrangement.slint");
    for property in [
        "root.grid-label-stride",
        "root.grid-line-unit",
        "root.grid-show-beat-ticks",
        "root.grid-show-lane-beats",
        "root.grid-subdiv-per-beat",
    ] {
        assert!(
            arrangement.contains(property),
            "arrangement.slint 未消费 {property}"
        );
    }
    // 网格色齐备（补丁包第四轮改钉：色值迁入 tokens.slint 的
    // ruler-tick-* / skin-grid-* 令牌，存在钉改钉令牌定义文件；
    // 扁平支：标尺三色 + 车道小节/拍两色）；四版以前的金色
    // （#a3804f / #c6a47c）清零；十版起旧车道线色（#748392 /
    // #5d6a76）清零（已按新底重算）；十一版起十版底与线色全组
    // 清零（底回蓝调重算）；十一版线色 #979fa8 / #74808b 随补丁
    // 在令牌与落点两处清零。
    let tokens = include_str!("../ui/tokens.slint");
    for color in ["#83909e", "#6e7f8f", "#66788a", "#3d444d", "#363c45"] {
        assert!(
            tokens.contains(color),
            "网格色 {color} 缺失（tokens.slint）"
        );
    }
    for gone in ["#979fa8", "#74808b"] {
        assert!(
            !tokens.contains(gone) && !arrangement.contains(gone),
            "十一版线色 {gone} 应随补丁清零"
        );
    }
    for gold in ["#a3804f", "#c6a47c"] {
        assert!(!arrangement.contains(gold), "金色 {gold} 仍在网格里");
    }
    for retired in [
        "#748392", "#5d6a76", "#2e2e2e", "#383f4c", "#a7b0ba", "#828f9c", "#6d7d8c",
    ] {
        assert!(
            !arrangement.contains(retired),
            "旧底/旧线色 {retired} 仍在（十一版已按新底重算）"
        );
    }

    // 比值改钉（Wang 2026-11-18 口径：判据按补丁包第四轮修改）：
    // 网格线改 Logic 式低反差，十一版的拍线 >= 3:1 / 小节线
    // >= 4.5:1 下限对本包网格线不再适用；改按补丁令牌实测值钉
    // 紧窗（本文件 WCAG 式实算，窗只容舍入）。标尺三色对标尺带
    // 底（扁平 bg-shell）：2026-11-19 判据卫生重算——原写死底
    // #262a2f 已随重设计退役（现行 bg-shell 扁平 #30343b），三窗
    // 此前按退役底空算；改按令牌名解析现行值实算：小节 3.84 /
    // 拍 3.03 / 细分 2.75。车道两色对车道底（扁平 bg-lane
    // #2b3036）：小节 1.35 / 拍 1.20（f7c5192f 改钉：车道小节/拍线色 #3c424a/#343a42 → #3d444d/#363c45，判据跟进 Wang 取值）。
    let (_, shell_flat) = token_pair(tokens, "bg-shell");
    let (_, tick_bar_flat) = token_pair(tokens, "ruler-tick-bar");
    let (_, tick_beat_flat) = token_pair(tokens, "ruler-tick-beat");
    let (_, tick_sub_flat) = token_pair(tokens, "ruler-tick-sub");
    let ruler_bar = contrast_ratio(&tick_bar_flat, &shell_flat);
    assert!(
        (3.79..=3.89).contains(&ruler_bar),
        "标尺小节刻度比值 {ruler_bar:.3} 不在实测窗 3.79–3.89"
    );
    let ruler_beat = contrast_ratio(&tick_beat_flat, &shell_flat);
    assert!(
        (2.98..=3.08).contains(&ruler_beat),
        "标尺拍刻度比值 {ruler_beat:.3} 不在实测窗 2.98–3.08"
    );
    let ruler_sub = contrast_ratio(&tick_sub_flat, &shell_flat);
    assert!(
        (2.70..=2.80).contains(&ruler_sub),
        "标尺细分刻度比值 {ruler_sub:.3} 不在实测窗 2.70–2.80"
    );
    let lane_bar = contrast_ratio("#3d444d", "#2b3036");
    assert!(
        (1.30..=1.40).contains(&lane_bar),
        "车道小节线比值 {lane_bar:.3} 不在实测窗 1.30–1.40"
    );
    let lane_beat = contrast_ratio("#363c45", "#2b3036");
    assert!(
        (1.15..=1.25).contains(&lane_beat),
        "车道拍线比值 {lane_beat:.3} 不在实测窗 1.15–1.25"
    );
    // 层级方向不降：小节 > 拍 > 细分（标尺）、小节 > 拍（车道）。
    assert!(ruler_bar > ruler_beat && ruler_beat > ruler_sub);
    assert!(lane_bar > lane_beat);
}

/// 从 `src[..end]` 里最后一个 `{marker}` 之后读出 px 数值（`.slint`
/// 几何常量提取，无正则，手写扫描）。
fn last_px_before(src: &str, end: usize, marker: &str) -> f32 {
    let head = &src[..end];
    let idx = head.rfind(marker).expect("标记必须存在");
    let rest = &head[idx + marker.len()..];
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().expect("标记之后必须是数值")
}

/// 从 `src[from..]` 里第一个 `{marker}` 之后读出 px 数值。
fn first_px_after(src: &str, from: usize, marker: &str) -> f32 {
    let rest = &src[from..];
    let idx = rest.find(marker).expect("标记必须存在");
    let tail = &rest[idx + marker.len()..];
    let num: String = tail
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().expect("标记之后必须是数值")
}

/// ⑥ 编号净空（六版，Wang 15:06 终判）：编号区与刻度行分区，编号区底
/// 与每档刻度顶之间 >= 2px 净空；三档刻度分级比例（满行 : 1/2 : 1/4）
/// 不变。钉的是 `arrangement.slint` 里的几何常量本身，⛔ 不靠目视。
#[test]
fn ruler_numbers_keep_a_clear_zone_above_the_ticks() {
    let arrangement = include_str!("../ui/arrangement.slint");

    // 标尺带高：`// Ruler (24px)` 注释之后第一个 height。
    let ruler_at = arrangement.find("// Ruler (24px)").expect("标尺注释在");
    let band = first_px_after(arrangement, ruler_at, "height: ");

    // 编号区高：编号 Text 的 text 行之前最近的 height。
    let text_at = arrangement
        .find("text: \"\" + (root.bar-start-index + i + 1);")
        .expect("编号 Text 在");
    let number_zone = last_px_before(arrangement, text_at, "height: ");

    // 三档刻度：各自 background 行之前最近的 height（刻度高）与
    // `parent.height - Npx`（刻度底距带底的距离，即刻度行上沿位置）。
    // 补丁包第四轮起刻度色走 Tokens.ruler-tick-* 令牌，定位串改
    // 令牌名；几何断言（分级比例 + 净空）强度不降。
    let tick = |color: &str| {
        let at = arrangement
            .find(&format!("background: {color};"))
            .unwrap_or_else(|| panic!("刻度落点 {color} 在"));
        (
            last_px_before(arrangement, at, "height: "),
            last_px_before(arrangement, at, "parent.height - "),
        )
    };
    let (bar_h, bar_from_bottom) = tick("Tokens.ruler-tick-bar");
    let (beat_h, beat_from_bottom) = tick("Tokens.ruler-tick-beat");
    let (sub_h, sub_from_bottom) = tick("Tokens.ruler-tick-sub");

    // 分级比例不变：小节满刻度行、拍 1/2、细分 1/4。
    assert_eq!(beat_h * 2.0, bar_h, "拍刻度须为小节刻度高的一半");
    assert_eq!(sub_h * 4.0, bar_h, "细分刻度须为小节刻度高的四分之一");

    // 净空不变量：每档刻度顶（自带顶起算）与编号区底之间 >= 2px。
    for (name, from_bottom) in [
        ("小节", bar_from_bottom),
        ("拍", beat_from_bottom),
        ("细分", sub_from_bottom),
    ] {
        let tick_top = band - from_bottom;
        assert!(
            tick_top - number_zone >= 2.0,
            "{name}刻度顶 {tick_top} 与编号区底 {number_zone} 之间净空不足 2px"
        );
    }
}

/// ⑦ 分界线与选中行（十一版原题；补丁包第四轮 2026-11-18 改钉；
/// 2026-11-18 夜随 Logic13 网格强度架构再改钉；2026-11-19 判据
/// 卫生按现行令牌链重算静态实算块）：
/// 车道底改走 `Tokens.bg-lane`、分界线改走 `Tokens.grid-row`
/// （强度层令牌，基色 `skin-grid-row`；现行基色比车道底亮半档，
/// rev11 暗线方向随退役字面值 `#23272c` 作废，现断言亮线方向）、
/// 选中行改 accent α0.06 透叠（按 sRGB 合成有效色实算）、网格线
/// 比值改按默认强度有效色实测窗、卷帘播放头改走
/// `Tokens.skin-text-strong`（比值实测窗）。色值一律经
/// `token_pair` 按令牌名解析，⛔ 不再写死字面值空算。
#[test]
fn rev11_divider_and_selected_row() {
    let arrangement = include_str!("../ui/arrangement.slint");

    // 车道底落点：背景层 Rectangle 走 Tokens.bg-lane 平色（令牌内
    // 含双主题支，相对亮度实测窗；色值经 token_pair 解析现行值：
    // 扁平 #2b3036 / 拟物 #1d1f21——原写死拟物 #1d2126 已退役）。
    assert!(
        arrangement.contains("background: Tokens.bg-lane;"),
        "车道容器底必须走 Tokens.bg-lane"
    );
    let tokens = include_str!("../ui/tokens.slint");
    let (lane_phys, lane_flat) = token_pair(tokens, "bg-lane");
    let (accent_phys, accent_flat) = token_pair(tokens, "accent");
    let base_lum = relative_luminance(&lane_flat);
    assert!(
        (0.027..=0.031).contains(&base_lum),
        "扁平车道底相对亮度 {base_lum:.4} 不在实测窗 0.027–0.031"
    );
    let phys_lum = relative_luminance(&lane_phys);
    assert!(
        (0.012..=0.016).contains(&phys_lum),
        "拟物车道底相对亮度 {phys_lum:.4} 不在实测窗 0.012–0.016"
    );
    // 选中行有效底：accent α0.06 透叠车道底的 sRGB 合成色（扁平
    // → #2f363f；拟物 → #222829，按现行车道底重算）。
    // 落点形态在 selection_highlight_uses_accent 判据钉。
    let sel_flat = alpha_over(&accent_flat, 0.06, &lane_flat);
    assert_eq!(sel_flat, "#2f363f", "扁平选中行有效底合成不符");
    let sel_phys = alpha_over(&accent_phys, 0.06, &lane_phys);
    assert_eq!(sel_phys, "#222829", "拟物选中行有效底合成不符");

    // 分界线落点：「行底分隔线」注释之后第一个 background 必须是
    // Tokens.grid-row（2026-11-18 夜改钉：Logic13 强度层落地后使用
    // 处一律走 grid-*，skin-grid-* 退为基色、只许经 tokens 派生；
    // 决策依据 Logic Pro——网格是背景、默认要轻，强度层即调轻
    // 机制，使用处直用基色会绕过它并破坏线层级）。
    let divider_at = arrangement.find("行底分隔线").expect("分界线注释在");
    let rest = &arrangement[divider_at..];
    let bg_idx = rest.find("background: ").expect("分界线 background 在");
    let bg_val: String = rest[bg_idx + "background: ".len()..]
        .chars()
        .take_while(|c| *c != ';')
        .collect();
    assert_eq!(
        bg_val.trim(),
        "Tokens.grid-row",
        "分界线必须走 Tokens.grid-row（强度层；基色 skin-grid-row）"
    );

    // 派生链：grid-row 必须在 tokens.slint 里由基色 skin-grid-row
    // 向车道底按 grid-fade 混合派生（强度 2 时 fade = 0 即基色原样）。
    assert!(
        tokens.contains("grid-row: skin-grid-row.mix(bg-lane, grid-fade)"),
        "Tokens.grid-row 必须由 skin-grid-row 经强度层派生"
    );

    // 强度层锚钉：默认强度 0（轻）⇒ grid-fade 0.55，grid-* 有效色
    // = 基色向车道底混合 0.55（Slint mix 的 sRGB 逐通道插值，与
    // 本文件 alpha_over(base, 0.45, bg) 同式）。
    assert!(
        tokens.contains("grid-strength: 0"),
        "网格默认强度须为 0（轻）"
    );
    assert!(
        tokens.contains("grid-strength == 0 ? 0.55"),
        "强度 0 的 grid-fade 须为 0.55"
    );
    let (row_base_phys, row_base_flat) = token_pair(tokens, "skin-grid-row");
    let (bar_base_phys, bar_base_flat) = token_pair(tokens, "skin-grid-bar");
    let (beat_base_phys, beat_base_flat) = token_pair(tokens, "skin-grid-beat");
    let row_flat = alpha_over(&row_base_flat, 0.45, &lane_flat);
    let row_phys = alpha_over(&row_base_phys, 0.45, &lane_phys);
    let bar_flat = alpha_over(&bar_base_flat, 0.45, &lane_flat);
    let bar_phys = alpha_over(&bar_base_phys, 0.45, &lane_phys);
    let beat_flat = alpha_over(&beat_base_flat, 0.45, &lane_flat);
    let beat_phys = alpha_over(&beat_base_phys, 0.45, &lane_phys);

    // 分界线方向（2026-11-19 判据卫生重算）：现行基色 skin-grid-row
    // 本就比车道底亮半档（令牌注释明示），rev11 暗线方向随退役
    // 字面值 #23272c / #16191d 一并作废；现断言亮线方向：两主题
    // 有效色相对亮度均高于各自车道底、低于各自拍线有效色。
    let divider_lum = relative_luminance(&row_flat);
    assert!(
        divider_lum > base_lum && divider_lum < relative_luminance(&beat_flat),
        "扁平分界线有效色 {row_flat} 须亮于车道底、暗于拍线（亮线方向）"
    );
    let divider_phys_lum = relative_luminance(&row_phys);
    assert!(
        divider_phys_lum > phys_lum && divider_phys_lum < relative_luminance(&beat_phys),
        "拟物分界线有效色 {row_phys} 须亮于车道底、暗于拍线（亮线方向）"
    );
    // 分界线比值（默认强度实测窗）：扁平对车道底 1.06、拟物 1.04。
    let divider_lane = contrast_ratio(&row_flat, &lane_flat);
    assert!(
        (1.01..=1.11).contains(&divider_lane),
        "扁平分界线对车道底比值 {divider_lane:.3} 不在实测窗 1.01–1.11"
    );
    let divider_phys = contrast_ratio(&row_phys, &lane_phys);
    assert!(
        (0.99..=1.09).contains(&divider_phys),
        "拟物分界线对车道底比值 {divider_phys:.3} 不在实测窗 0.99–1.09"
    );

    // 网格线对选中行有效底（默认强度有效色实测窗，2026-11-19
    // 重算；原窗按 f7c5192f 前的退役线色 #3c424a / #343a42 与
    // 拟物退役色 #2a3037 / #242930 空算）：扁平小节 1.05 / 拍
    // 1.01；拟物小节 1.01 / 拍 1.04。
    let bar_sel = contrast_ratio(&bar_flat, &sel_flat);
    assert!(
        (1.00..=1.10).contains(&bar_sel),
        "扁平小节线对选中行比值 {bar_sel:.3} 不在实测窗 1.00–1.10"
    );
    let beat_sel = contrast_ratio(&beat_flat, &sel_flat);
    assert!(
        (0.96..=1.06).contains(&beat_sel),
        "扁平拍线对选中行比值 {beat_sel:.3} 不在实测窗 0.96–1.06"
    );
    let bar_sel_p = contrast_ratio(&bar_phys, &sel_phys);
    assert!(
        (0.96..=1.06).contains(&bar_sel_p),
        "拟物小节线对选中行比值 {bar_sel_p:.3} 不在实测窗 0.96–1.06"
    );
    let beat_sel_p = contrast_ratio(&beat_phys, &sel_phys);
    assert!(
        (0.99..=1.09).contains(&beat_sel_p),
        "拟物拍线对选中行比值 {beat_sel_p:.3} 不在实测窗 0.99–1.09"
    );
    // 层级（对车道底的绝对对比，默认强度有效色）：分界线 < 拍线
    // < 小节线（两主题）。
    let beat_lane = contrast_ratio(&beat_flat, &lane_flat);
    let bar_lane = contrast_ratio(&bar_flat, &lane_flat);
    assert!(
        divider_lane < beat_lane && beat_lane < bar_lane,
        "扁平层级须 分界线 < 拍线 < 小节线（实算 {divider_lane:.3} / {beat_lane:.3} / {bar_lane:.3}）"
    );
    let beat_lane_p = contrast_ratio(&beat_phys, &lane_phys);
    let bar_lane_p = contrast_ratio(&bar_phys, &lane_phys);
    assert!(
        divider_phys < beat_lane_p && beat_lane_p < bar_lane_p,
        "拟物层级须 分界线 < 拍线 < 小节线（实算 {divider_phys:.3} / {beat_lane_p:.3} / {bar_lane_p:.3}）"
    );

    // 卷帘播放头：`piano-roll-playhead` 元素向上的最近一处 background
    // 必须走 Tokens.skin-text-strong（补丁包第四轮，取代纯白）；
    // 对卷帘最深行底（bg-panel-alt）实算：扁平 11.63、拟物 14.08
    // （2026-11-19 判据卫生：色值经 token_pair 解析，原写死拟物
    // 底 #20242a 已退役、现行 #202224），钉实测窗。
    let piano_roll = include_str!("../ui/console/piano_roll.slint");
    let ph_at = piano_roll
        .find("accessible-id: \"piano-roll-playhead\"")
        .expect("卷帘播放头在");
    let head = &piano_roll[..ph_at];
    let bg_rel = head.rfind("background: ").expect("播放头 background 在");
    let ph_val: String = head[bg_rel + "background: ".len()..]
        .chars()
        .take_while(|c| *c != ';')
        .collect();
    assert_eq!(
        ph_val.trim(),
        "Tokens.skin-text-strong",
        "卷帘播放头必须走 Tokens.skin-text-strong"
    );
    let (panel_alt_phys, panel_alt_flat) = token_pair(tokens, "bg-panel-alt");
    let (strong_phys, strong_flat) = token_pair(tokens, "skin-text-strong");
    let ph_flat = contrast_ratio(&strong_flat, &panel_alt_flat);
    assert!(
        (11.4..=11.9).contains(&ph_flat),
        "扁平播放头对卷帘最深行底比值 {ph_flat:.3} 不在实测窗 11.4–11.9"
    );
    let ph_phys = contrast_ratio(&strong_phys, &panel_alt_phys);
    assert!(
        (13.8..=14.3).contains(&ph_phys),
        "拟物播放头对卷帘最深行底比值 {ph_phys:.3} 不在实测窗 13.8–14.3"
    );
}

/// RGB → HSL（与 `bridge::clip_base_color` 的压暗规则对账用：
/// 压暗只许降明度，色相/饱和须守恒）。
fn rgb_to_hsl(hex: &str) -> (f32, f32, f32) {
    let h = hex.trim_start_matches('#');
    let chan =
        |i: usize| u8::from_str_radix(&h[i..i + 2], 16).expect("合法十六进制") as f32 / 255.0;
    let (r, g, b) = (chan(0), chan(2), chan(4));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let hue = if (max - r).abs() < f32::EPSILON {
        let mut x = (g - b) / d;
        if g < b {
            x += 6.0;
        }
        x / 6.0
    } else if (max - g).abs() < f32::EPSILON {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (hue, s, l)
}

/// ⑧ 十版片段底纯色 + 去同色边框 + 标记同色相派生（底/边框为九版
/// Wang 2026-11-17 16:43 亲定，标记为十版 18:28 亲定）：
/// 片段块底必须是投影层算好的轨道色纯色 `blk.base-color`（八版
/// 「近黑底 + 0.12 叠层」混黑做法清零）；未选中片段的同色边框
/// （`trk.color` α0.55）去掉（宽 0），选中片段 2px 纯白边框保留；
/// 文字/导线/标记走投影字段。调色板普查逐色钉落码值：底/文字
/// 沿九版下限实算（白对底 >= 3.0、文字 >= 4.5），标记/导线改钉
/// 派生精确值（十版规则不承诺九版导线 >= 3.0 地板，精确值断言
/// 取代之，强度不降）。
#[test]
fn rev10_clip_background_is_pure_track_color() {
    use yeban_app::bridge::{
        RgbColor, clip_base_color, clip_marker_color, clip_text_color, track_color_or_default,
    };

    let arrangement = include_str!("../ui/arrangement.slint");
    // 锚点含下标变量：框选线给循环加了 [blk_idx]（选中标志同序同长）。
    let clips_at = arrangement
        .find("for blk[blk_idx] in root.timeline-blocks")
        .expect("片段循环在");
    let rest = &arrangement[clips_at..];
    let bg_idx = rest.find("background: ").expect("片段块 background 在");
    let bg_val: String = rest[bg_idx + "background: ".len()..]
        .chars()
        .take_while(|c| *c != ';')
        .collect();
    assert_eq!(
        bg_val.trim(),
        "blk.base-color",
        "片段块底必须是轨道色纯色 blk.base-color（九版），不得混黑/透底"
    );
    // 八版混黑两件套清零：0.12 叠层与 α0.55 同色边框。
    for old in ["trk.color.with-alpha(0.12)", "trk.color.with-alpha(0.55)"] {
        assert!(!arrangement.contains(old), "八版片段做法 {old} 仍在");
    }
    // 边框：未选中宽 0（无边框）、选中 2px；边框色只剩纯白（选中指示）。
    assert!(
        arrangement.contains("? 2px : 0px"),
        "未选中片段边框宽必须为 0（同色边框已去除）"
    );
    // 文字与导线走投影字段（名字/增益/标注/空片段 4 处文字、2 处导线）。
    assert_eq!(
        arrangement.matches("color: blk.text-color;").count(),
        4,
        "片段内文字应为 4 处走 blk.text-color"
    );
    assert_eq!(
        arrangement.matches("background: blk.guide-color;").count(),
        2,
        "片段内导线应为 2 处走 blk.guide-color"
    );
    assert_eq!(
        arrangement.matches("background: blk.marker-color;").count(),
        2,
        "片段内标记应为 2 处走 blk.marker-color（峰值柱/音符条）"
    );

    // 调色板普查：产品自带/自分派的全部轨道色（自动分派 6 色、夹具
    // 3 色）、默认色与白色（最不利），逐色按 bridge 规则钉落码值。
    // （原色、落码底、文字、标记）——标记与导线同出
    // `clip_marker_color`（导线字段由 host 同值填充，接线计数在上），
    // 数值由派生式离线算出后钉死，bridge 规则若漂移此处即红。
    // 下限实算只保留仍成立的两项：白对底 >= 3.0（底压暗规则的
    // 兜底性质，十版不变）、文字 >= 4.5；标记改精确值断言（十版
    // 派生规则对底实算 2.22–7.96，与 Logic 实测带 2.9–5.2 同量级，
    // 不再钉 >= 3.0 地板）。
    let census: [(&str, &str, &str, &str); 11] = [
        ("#2C3A63", "#2C3A63", "#F1F5F9", "#CEDAFC"),
        ("#FF8800", "#E07800", "#000000", "#FCD5A8"),
        ("#33AAFF", "#0095FF", "#000000", "#B9E0FC"),
        ("#66DD88", "#24A449", "#000000", "#CFFCDC"),
        ("#DD66CC", "#DD66CC", "#000000", "#FCCFF6"),
        ("#FFD166", "#C28800", "#000000", "#FCEDCA"),
        ("#8C8CFF", "#8282FF", "#000000", "#D7D7FC"),
        ("#3366FF", "#3366FF", "#FFFFFF", "#B9CAFC"),
        ("#22AA88", "#20A281", "#000000", "#B9FCEC"),
        ("#F7E6B0", "#B48E12", "#000000", "#FCF7E4"),
        ("#FFFFFF", "#8F8F8F", "#000000", "#FCFCFC"),
    ];
    for (orig, want_base, want_text, want_marker) in census {
        let track = track_color_or_default(Some(orig));
        let base = clip_base_color(track);
        assert_eq!(base.to_hex(), want_base, "{orig} 的落码底色不符");
        assert_eq!(
            clip_text_color(base).to_hex(),
            want_text,
            "{orig} 的文字色不符"
        );
        assert_eq!(
            clip_marker_color(track).to_hex(),
            want_marker,
            "{orig} 的标记派生色不符"
        );
        // 下限用本文件的 WCAG 式实算（不靠 bridge 自证）。
        let base_hex = base.to_hex();
        assert!(
            contrast_ratio("#ffffff", &base_hex) >= 3.0,
            "{orig} 底 {base_hex} 上白色对比不足 3.0（底压暗规则兜底）"
        );
        assert!(
            contrast_ratio(want_text, &base_hex) >= 4.5,
            "{orig} 底 {base_hex} 上文字对比不足 4.5"
        );
        // 压暗守恒：落码与原色同色相、同饱和（只降明度）；未压暗者
        // 落码 == 原色。
        let (h0, s0, l0) = rgb_to_hsl(orig);
        let (h1, s1, l1) = rgb_to_hsl(&base_hex);
        assert!(
            (h0 - h1).abs() < 0.01 && (s0 - s1).abs() < 0.02 && l1 <= l0 + 0.004,
            "{orig} → {base_hex} 不是只降明度（色相 {h0:.3}→{h1:.3}、饱和 {s0:.3}→{s1:.3}）"
        );
    }
    // 默认色走同一入口（缺失/非法回退色也是调色板成员）。
    assert_eq!(
        clip_base_color(RgbColor::new(0x2c, 0x3a, 0x63)).to_hex(),
        "#2C3A63"
    );
}

/// ⑨ 片段内标记与和弦块（标记为十版 Wang 2026-11-17 18:28；
/// 和弦块与播放头落点为补丁包第四轮 2026-11-18 改钉）：波形峰值
/// 柱/MIDI 音符条走投影派生色 `blk.marker-color`、片段内导线走
/// `blk.guide-color`（同出派生函数）；播放头线/菱形头改走
/// `Tokens.skin-text-strong` 2 处、纯白清零；网格线改走
/// `grid-*` 强度令牌（背景层各 1 处，基色 `skin-grid-*` 不得直用；
/// 2026-11-18 夜随 Logic13 强度架构改钉）、十一版字面线色清零；
/// 五块和弦底改走 `Theme.track-*` 本体 + `.brighter(0.18)` hover、
/// 边框 `.brighter(0.08)` / `.darker(0.12)`，和弦名文字对常态底
/// 比值改钉实测窗；八版合成等效实色与更早的 alpha 字面量全清零
/// （沿九版）。
#[test]
fn rev11_clip_markers_and_chord_blocks_are_pure() {
    let arrangement = include_str!("../ui/arrangement.slint");

    // 片段内旧透画标记清零（本文件内三处原为轨道色 α0.85/0.20/0.15）。
    for old in [
        "trk.color.with-alpha(0.85)",
        "trk.color.with-alpha(0.20)",
        "trk.color.with-alpha(0.15)",
    ] {
        assert!(!arrangement.contains(old), "片段内透画标记 {old} 仍在");
    }
    // 播放头落点计数（补丁包第四轮改钉）：纯白 #ffffff 清零，
    // 编曲播放头线/菱形头 2 处改走 Tokens.skin-text-strong
    // （峰值柱/音符条走 blk.marker-color，判据 ⑧ 钉计数 2）。
    assert_eq!(
        arrangement.matches("background: #ffffff;").count(),
        0,
        "纯白实色落点应清零（播放头已改走 skin-text-strong）"
    );
    assert_eq!(
        arrangement
            .matches("background: Tokens.skin-text-strong;")
            .count(),
        2,
        "skin-text-strong 落点应为 2 处（编曲播放头线/菱形头）"
    );
    // 网格线落点改走 grid-* 强度令牌（背景层各 1 处）；基色
    // skin-grid-* 不得在编曲区直接落点（只许经 tokens.slint 的
    // 强度派生消费）。十一版字面线色 #979fa8 / #74808b 与更早的
    // #748392 / #a7b0ba 清零。
    for tok in ["grid-bar", "grid-beat", "grid-sub"] {
        assert_eq!(
            arrangement
                .matches(&format!("background: Tokens.{tok};"))
                .count(),
            1,
            "{tok} 落点应为 1 处（背景层）"
        );
    }
    for base in [
        "skin-grid-row",
        "skin-grid-bar",
        "skin-grid-beat",
        "skin-grid-sub",
    ] {
        assert_eq!(
            arrangement
                .matches(&format!("background: Tokens.{base};"))
                .count(),
            0,
            "{base} 是基色，不得在编曲区直接落点（须走 grid-* 强度层）"
        );
    }
    for gone in ["#979fa8", "#74808b", "#748392", "#a7b0ba"] {
        assert!(!arrangement.contains(gone), "旧线色 {gone} 应清零");
    }
    // 派生标记对最不利落码底（白色轨道：派生 #fcfcfc 对压暗底
    // #8f8f8f）实算 3.15 >= 3.0；其余调色板底由判据 ⑧ 的普查
    // 逐色钉精确值看守。
    assert!(contrast_ratio("#fcfcfc", "#8f8f8f") >= 3.0);

    // 和弦块：旧 alpha 字面量（底 α0x18/0x35、边框 α0x35/0x70）清零。
    for old in [
        "#1b283818",
        "#1b283835",
        "#2d203818",
        "#2d203835",
        "#182c2618",
        "#182c2635",
        "#34261618",
        "#34261635",
        "#38769135",
        "#38769170",
        "#705e8835",
        "#705e8870",
        "#34786435",
        "#34786470",
        "#9a6a3835",
        "#9a6a3870",
    ] {
        assert!(!arrangement.contains(old), "和弦块 alpha 字面量 {old} 仍在");
    }
    // 八版合成等效实色（混黑）清零。
    for old in [
        "#0e121a", "#10151e", "#162530", "#203d4d", "#10121a", "#14131e", "#22202e", "#383249",
        "#0e1318", "#0f161a", "#152627", "#1e3e39", "#111217", "#151517", "#2a231e", "#4b3825",
    ] {
        assert!(!arrangement.contains(old), "八版和弦块混黑实色 {old} 仍在");
    }
    // 和弦块底形（补丁包第四轮改钉）：底走 Theme.track-* 本体
    // （chord-1/5 同 synth），hover = .brighter(0.18)，边框 =
    // hover .brighter(0.08) / 常态 .darker(0.12)，逐形钉。
    for form in [
        "Theme.track-synth.brighter(0.18) : Theme.track-synth;",
        "Theme.track-pad.brighter(0.18) : Theme.track-pad;",
        "Theme.track-vocal.brighter(0.18) : Theme.track-vocal;",
        "Theme.track-drum.brighter(0.18) : Theme.track-drum;",
    ] {
        assert!(arrangement.contains(form), "和弦块底形 {form} 缺失");
    }
    assert_eq!(
        arrangement
            .matches("Theme.track-synth.brighter(0.18) : Theme.track-synth;")
            .count(),
        2,
        "synth 和弦块应为 2 块（chord-1/5）"
    );
    for edge in [
        "Theme.track-synth.brighter(0.08)",
        "Theme.track-synth.darker(0.12)",
        "Theme.track-pad.brighter(0.08)",
        "Theme.track-pad.darker(0.12)",
        "Theme.track-vocal.brighter(0.08)",
        "Theme.track-vocal.darker(0.12)",
        "Theme.track-drum.brighter(0.08)",
        "Theme.track-drum.darker(0.12)",
    ] {
        assert!(arrangement.contains(edge), "和弦块边框形 {edge} 缺失");
    }
    // 和弦名文字（skin-text-strong）对常态底实算（Wang 2026-11-18
    // 口径：以补丁包取值合入，原 >= 4.5 下限对本包和弦文字不再
    // 适用，改钉实测窗）：扁平 #f4f7fa 对 synth 3.93 / pad 4.17 /
    // vocal 3.65 / drum 3.70；拟物 #eef1f4 对 synth 2.51 / pad
    // 3.16 / vocal 2.79 / drum 2.79。
    for (text, base, lo, hi) in [
        ("#f4f7fa", "#5c7f9c", 3.78_f32, 4.08_f32),
        ("#f4f7fa", "#7d7291", 4.02, 4.32),
        ("#f4f7fa", "#5b8a78", 3.50, 3.80),
        ("#f4f7fa", "#9a7a52", 3.55, 3.85),
        ("#eef1f4", "#6f9fbf", 2.36, 2.66),
        ("#eef1f4", "#8a80b8", 3.01, 3.31),
        ("#eef1f4", "#4fa07a", 2.64, 2.94),
        ("#eef1f4", "#b8883f", 2.64, 2.94),
    ] {
        let ratio = contrast_ratio(text, base);
        assert!(
            (lo..=hi).contains(&ratio),
            "和弦文字 {text} 对底 {base} 比值 {ratio:.3} 不在实测窗 {lo}–{hi}"
        );
    }
}
