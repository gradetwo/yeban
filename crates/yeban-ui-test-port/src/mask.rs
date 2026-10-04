//! 动态区域遮罩 —— `[UI-MCP-002]` 的机械实现。
//!
//! 规范原文（UI/UX §12.5）：走带光标位置、VU 电平表跳变、RTA 频谱柱与微秒级时间码在播放时
//! **每帧变化**；直接全屏比对会因时间抖动产生 100% 假阳性。因此比对算法在执行 SSIM 前，
//! 必须根据**元素树元数据**取得这些高频刷新组件的矩形包围盒，并在比对矩阵中把该区域
//! **强制置为纯黑 `#000000`**。
//!
//! ## 为什么置黑而不是"排除"
//!
//! 规范给了两种等价手段（置黑 / 完全排除）。这里选**置黑**，因为它让两张图仍然拥有
//! **逐字节可比较的同一维度**：`masked(a) == masked(b)` 是普通字节相等，
//! 不需要在 SSIM 里维护一份"有效像素掩码"的第二套坐标逻辑（那正是最容易写错的地方）。
//!
//! ## 与 `tree.rs` 的接口
//!
//! 遮罩矩形**只能**来自元素树（[`mask_rects_from_tree`]），不允许调用方手写坐标：
//! `[UI-TEST-001]` §12.2 明文禁止绝对坐标寻址，而"动态区清单写在测试里"正是那种硬编码的变体。

use crate::image::{MASK_COLOR, Rect, Rgb8Image};
use crate::tree::{ControlTree, TreeError};

/// 把若干矩形区域置为纯黑，返回**累计**写黑的像素数。
///
/// 矩形会被夹到图像内（[`Rect::intersect`]），因此来自元素树的"部分在屏幕外"的包围盒
/// 不会越界；完全在图像外的矩形不贡献任何像素。
/// 重叠区域会被重复计入返回值（返回值是"写入次数"，不是"被遮罩的像素集合大小"）——
/// 这一点写在文档里而不是靠调用方猜，避免把返回值当成覆盖率用。
pub fn apply_masks(image: &mut Rgb8Image, rects: &[Rect]) -> usize {
    let mut written = 0usize;
    for rect in rects {
        written = written.saturating_add(image.fill_rect(*rect, MASK_COLOR));
    }
    written
}

/// 返回遮罩后的**副本**（原图不动）。比对路径应当用这个，避免把遮罩写进被断言的黄金内存。
#[must_use]
pub fn masked(image: &Rgb8Image, rects: &[Rect]) -> Rgb8Image {
    let mut copy = image.clone();
    apply_masks(&mut copy, rects);
    copy
}

/// 从控件树取出 `[UI-MCP-002]` 要求的遮罩矩形。
///
/// 直接转发到 [`ControlTree::mask_rects`]，后者的判据拒绝"登记成动态区却没有包围盒"的节点。
pub fn mask_rects_from_tree(tree: &ControlTree) -> Result<Vec<Rect>, TreeError> {
    tree.mask_rects()
}

/// 两张图在遮罩后是否**逐字节相同**。
///
/// 尺寸不同一律返回 `false`（尺寸本身就是 `[UI-MCP-003]` 要抓的视觉回归）。
#[must_use]
pub fn equal_after_masking(a: &Rgb8Image, b: &Rgb8Image, rects: &[Rect]) -> bool {
    if a.size() != b.size() {
        return false;
    }
    masked(a, rects).pixels() == masked(b, rects).pixels()
}

/// 验证"遮罩确实生效"：矩形内每个像素都必须等于 [`MASK_COLOR`]。
///
/// 这是 CI 里防止"遮罩静默失效"的自检 —— 如果 [`apply_masks`] 被改成空实现，
/// 它会在**非空矩形**上立刻返回 `false`。
#[must_use]
pub fn mask_is_effective(image: &Rgb8Image, rects: &[Rect]) -> bool {
    let in_bounds: Vec<Rect> = rects
        .iter()
        .filter_map(|rect| rect.intersect(image.size()))
        .collect();
    if in_bounds.is_empty() {
        // 没有任何矩形落在图像内 ⇒ 这次"遮罩生效"无从证明, 不能返回 true 冒充。
        return false;
    }
    in_bounds.iter().all(|rect| {
        (rect.y..rect.bottom()).all(|y| {
            (rect.x..rect.right()).all(|x| image.pixel(x as u32, y as u32) == Some(MASK_COLOR))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::Size;
    use crate::tree::{ControlNode, Role};

    fn scene() -> Rgb8Image {
        let mut image = Rgb8Image::new(Size::new(64, 32));
        image.fill([0x20, 0x30, 0x40]);
        image.fill_rect(Rect::new(4, 4, 20, 10), [0xff, 0x00, 0x00]);
        image
    }

    /// 判据 1: `[UI-MCP-002]` —— 遮罩区必须被**真的**置成纯黑（`mask_is_effective` 是自检）。
    #[test]
    fn mask_writes_pure_black_into_the_region() {
        let image = scene();
        let rects = [Rect::new(10, 6, 8, 6), Rect::new(0, 0, 3, 3)];
        let out = masked(&image, &rects);
        assert!(mask_is_effective(&out, &rects));
        assert_eq!(out.pixel(10, 6), Some(MASK_COLOR));
        assert_eq!(out.pixel(17, 11), Some(MASK_COLOR));
        assert_eq!(
            out.pixel(18, 6),
            Some([0xff, 0x00, 0x00]),
            "矩形外不得被改动"
        );
        // 原图不被改动。
        assert_eq!(image.pixel(10, 6), Some([0xff, 0x00, 0x00]));
    }

    /// 判据 2: **遮罩区内的改动不得影响结论** —— 这是本模块存在的全部理由。
    #[test]
    fn changes_inside_the_mask_do_not_change_the_verdict() {
        let base = scene();
        let mut jittered = base.clone();
        // 模拟 VU 表 / 走带光标每帧跳变。
        jittered.fill_rect(Rect::new(11, 7, 6, 4), [0x00, 0xff, 0x00]);
        let rects = [Rect::new(10, 6, 8, 6)];

        assert_ne!(base.pixels(), jittered.pixels(), "两张图必须真的不同");
        assert!(
            equal_after_masking(&base, &jittered, &rects),
            "遮罩内的差异必须被完全吸收"
        );
        assert!(
            !equal_after_masking(&base, &jittered, &[]),
            "不遮罩时同一个差异必须被检出 —— 否则上一条判据毫无意义"
        );
    }

    /// 判据 3: **遮罩区外的改动必须被检出**（遮罩不能把整幅图变成盲区）。
    #[test]
    fn changes_outside_the_mask_are_still_detected() {
        let base = scene();
        let mut shifted = base.clone();
        shifted.fill_rect(Rect::new(40, 20, 6, 6), [0x00, 0x00, 0xff]);
        let rects = [Rect::new(10, 6, 8, 6)];
        assert!(!equal_after_masking(&base, &shifted, &rects));
    }

    /// 判据 4: 越界矩形被夹取；完全在图像外时"生效"无从证明（返回 `false`，不许冒充）。
    #[test]
    fn out_of_bounds_rects_are_clamped_and_never_fake_effectiveness() {
        let mut image = Rgb8Image::new(Size::new(8, 8));
        image.fill([0xff, 0xff, 0xff]);
        let written = apply_masks(&mut image, &[Rect::new(-4, -4, 8, 8)]);
        assert_eq!(written, 16, "只有交叠的 4x4 被写入");
        assert!(mask_is_effective(&image, &[Rect::new(-4, -4, 8, 8)]));
        assert!(!mask_is_effective(&image, &[Rect::new(100, 100, 4, 4)]));
        assert_eq!(apply_masks(&mut image, &[Rect::new(100, 100, 4, 4)]), 0);
    }

    /// 判据 5: 动态区清单**只能**来自元素树，且动态区缺包围盒必须报错。
    #[test]
    fn mask_rects_come_from_the_tree_only() {
        let mut tree = ControlTree::new();
        let role = Role::parse("image").expect("合法角色");
        tree.insert(
            ControlNode::new("transport-playhead", role.clone(), "走带光标")
                .with_bounds(Rect::new(12, 0, 2, 32))
                .as_dynamic(),
        )
        .expect("插入应当成功");
        tree.insert(
            ControlNode::new("mixer-vu-track-0", role, "轨道 0 VU 电平")
                .with_bounds(Rect::new(48, 8, 4, 16))
                .as_dynamic(),
        )
        .expect("插入应当成功");
        tree.insert(ControlNode::new(
            "track-0-fader",
            Role::parse("slider").expect("合法角色"),
            "推子",
        ))
        .expect("插入应当成功");

        assert_eq!(
            mask_rects_from_tree(&tree),
            Ok(vec![Rect::new(48, 8, 4, 16), Rect::new(12, 0, 2, 32)]),
            "顺序 = ID 升序 (确定性)"
        );

        let mut broken = ControlTree::new();
        broken
            .insert(
                ControlNode::new(
                    "rta-band-0",
                    Role::parse("progress-indicator").expect("合法角色"),
                    "RTA",
                )
                .as_dynamic(),
            )
            .expect("插入应当成功");
        assert!(
            mask_rects_from_tree(&broken).is_err(),
            "动态区缺包围盒必须报错"
        );
    }
}
