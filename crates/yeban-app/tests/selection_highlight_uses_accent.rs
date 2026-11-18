//! **选中轨道高亮不得与轨道身份色撞色**（用户 2026-11-16 反馈）。
//!
//! 修前：选中态的行底色/边框用的是 `trk.color`（轨道自身颜色），而轨道色条/片段
//! 也是同一个色 ⇒ 「选中」与「这条轨是什么色」混在一起，看不出哪条被选中。
//!
//! 第一代修法（2026-11-16）：选中态统一用 `Theme.accent` 做底色与边框；
//! 轨道色只保留在左侧色条上（身份标识）。
//!
//! 第二代修法（七版，Wang 2026-11-17 15:51 亲定）：选中边框一律纯白
//! `#ffffff`（行边框与左右轨道头边框三处同改），选中底色改定值——行底
//! `#131a20`、头底 `#16232b`（均由现行 accent 合成帧上实测色向白混合
//! 3% 而来），与轨道色、accent **双脱钩**。
//!
//! 第三代修法（八版，Wang 2026-11-17 16:08 亲定，照 Logic Pro）：纯白
//! 边框作废——选中行不再有任何边框信号，三处边框回归未选中行同款
//! （行边框 `#ffffff0a` 0px、头边框 `Theme.border-subtle`）；选中只由
//! 提亮承载：行底 `#0f1218`、头底 `#293645`，均由未选中底（行
//! `#090b10`、头 `#1d2733`）按 Logic 截图选中/未选中头底相对亮度比
//! 1.80 在线性空间同比例提亮而来（头对比度 1.23:1、行 1.05:1；
//! Logic 自身车道对比度亦仅 1.11:1）。「不得用 trk.color」与「色条
//! 保留轨道色」两条三代原样保留（不放宽）。
//!
//! 第四代（十版，Wang 2026-11-17 18:28）：车道底抬到 `#2e2e2e`
//! （Logic 编曲区底实测同值）后，选中行底按同一相对亮度比 1.80
//! 重算为 `#383f4c`；头底基色 `#1d2733` 未变，选中头底 `#293645`
//! 与提亮法原样保留。
//!
//! 第五代（十一版，Wang 2026-11-17 18:51）：车道底改回蓝调
//! `#1d2333`（`#090b10` 色相/饱和只抬明度，相对亮度 0.0170）后，
//! 选中行底按同一相对亮度比 1.80 重算为 `#273146`；头底基色
//! `#1d2733` 未变，选中头底 `#293645` 与提亮法原样保留。
//!
//! 合入裁决甲（Wang 2026-11-17 19:58）：编曲区走质感三元——十一版
//! 定值只落扁平支，拟物支恢复 main 的 accent 选中形态（行底 α0.08、
//! 行边 α0.70、头底 α0.10、头边 α0.80）。本判据原「全文件禁
//! `selected-track-id ? Theme.accent`」与裁决互斥，改钉为：四个三元
//! 全串逐一钉 + accent 选中形出现次数恰为 6（行底/行边/左头底/
//! 左头边/右头底/右头边，全部位于三元拟物支内）；扁平支的十一版
//! 定值断言（`#273146` / `#293645`）原样保留、强度不降。
//!
//! 第六代（重设计补丁包第四轮，Wang 2026-11-18 定判据按补丁包
//! 修改）：行选中改独立透叠层（`visible` 门控 + accent α0.06，
//! 双主题同形，压网格上、片段下）；轨道头底扁平支改 accent
//! α0.12 / 未选中 `Tokens.bg-panel`，头边三元串不变。改钉为：
//! 透叠层两串 + 头底/头边三元全串（左右各一份）+ accent 选中形
//! 恰 4 处（左头底/左头边/右头底/右头边）；`#273146` / `#293645`
//! 字面钉撤销（两值在文件他处仍有无关出现，字面钉已成空钉）。
//!
//! ⚠ 纯 `include_str!` 判据；像素级效果由对照帧看守（八版 compare8-*），
//! 本改动会触发 golden 基准重录（`gates-manual` 的 `goldens` 档）。

const ARR: &str = include_str!("../ui/arrangement.slint");

#[test]
fn selected_track_highlight_uses_brightening_not_white_border() {
    assert!(
        !ARR.contains("selected-track-id ? trk.color"),
        "选中高亮不得再用 trk.color（与轨道身份色撞色 ⇒ 看不出选中）"
    );
    // 补丁包第四轮 (Wang 2026-11-18) 改钉: 行选中 = 独立透叠层
    // (visible 门控 + accent α0.06, 双主题同形); 头底/头边三元
    // 全串逐一钉 (左右两头各一份, 下方另钉计数 2)。
    assert!(
        ARR.contains("visible: _sel.id == root.selected-track-id;"),
        "行选中透叠层必须由 visible 门控（只画选中行）"
    );
    assert!(
        ARR.contains("background: Theme.accent.with-alpha(0.06);"),
        "行选中必须靠 accent α0.06 透叠层承载（压网格上、片段下）"
    );
    for full in [
        "background: Tokens.texture-physical ? (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.10) : Tokens.track-head-brush) : (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.12) : Tokens.bg-panel);",
        "border-color: Tokens.texture-physical ? (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.80) : Theme.border-subtle) : Theme.border-subtle;",
    ] {
        assert!(ARR.contains(full), "头底/头边三元全串缺失: {full}");
    }
    assert_eq!(
        ARR.matches("background: Tokens.texture-physical ? (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.10)").count(),
        2,
        "头底三元必须左右两头各一份"
    );
    assert_eq!(
        ARR.matches("border-color: Tokens.texture-physical ? (trk.id == root.selected-track-id ? Theme.accent.with-alpha(0.80)").count(),
        2,
        "头边三元必须左右两头各一份"
    );
    assert_eq!(
        ARR.matches("selected-track-id ? Theme.accent").count(),
        6,
        "accent 选中形恰 6 处 (头底每头两支: 拟物 α0.10 + 扁平 α0.12; 头边每头一支 α0.80; 行选中已改独立透叠层, 不计入)"
    );
    assert!(
        !ARR.contains("border-color: trk.id == root.selected-track-id ? #ffffff"),
        "选中边框不得为 #ffffff（八版口径：纯白选中边框作废，回归未选中行同款；轨道名选中变白是原有文字行为，不在本条范围）"
    );
}

#[test]
fn track_color_strip_is_preserved_as_identity() {
    assert!(
        ARR.contains("background: trk.color;"),
        "左侧色条必须保留轨道色（身份标识），否则选中后认不出是哪条轨"
    );
}
