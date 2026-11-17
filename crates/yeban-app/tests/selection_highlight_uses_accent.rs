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
//! ⚠ 纯 `include_str!` 判据；像素级效果由对照帧看守（八版 compare8-*），
//! 本改动会触发 golden 基准重录（`gates-manual` 的 `goldens` 档）。

const ARR: &str = include_str!("../ui/arrangement.slint");

#[test]
fn selected_track_highlight_uses_brightening_not_white_border() {
    assert!(
        !ARR.contains("selected-track-id ? trk.color"),
        "选中高亮不得再用 trk.color（与轨道身份色撞色 ⇒ 看不出选中）"
    );
    assert!(
        !ARR.contains("selected-track-id ? Theme.accent"),
        "选中高亮不得再用 Theme.accent（七版口径：与 accent 亦脱钩）"
    );
    assert!(
        !ARR.contains("border-color: trk.id == root.selected-track-id ? #ffffff"),
        "选中边框不得为 #ffffff（八版口径：纯白选中边框作废，回归未选中行同款；轨道名选中变白是原有文字行为，不在本条范围）"
    );
    assert!(
        ARR.contains("selected-track-id ? #383f4c"),
        "选中行底必须用十版定值 #383f4c（八版 1.80 提亮比按新车道底重算）"
    );
    assert!(
        ARR.contains("selected-track-id ? #293645"),
        "选中轨道头底必须用八版定值 #293645"
    );
}

#[test]
fn track_color_strip_is_preserved_as_identity() {
    assert!(
        ARR.contains("background: trk.color;"),
        "左侧色条必须保留轨道色（身份标识），否则选中后认不出是哪条轨"
    );
}
