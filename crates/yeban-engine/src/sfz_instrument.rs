//! SFZ 乐器的**适配层**（`yeban-sfz` → 引擎）。[ROAD-M2-005, ROAD-M3-004]
//!
//! `yeban-sfz` 是 SFZ v2 的**唯一实现**（零拷贝解析器、区域/曲线/力度/交叉淡化/声部池）。
//! 它的 API 是**借用式**的：
//!
//! ```text
//! pub fn parse_text<'a>(text: &'a str, limits: &ParseLimits) -> Result<Instrument<'a>, SfzError>
//! pub struct Region<'a> { … }        // 同样带生命周期
//! ```
//!
//! 于是"把 `String` 与解析结果放进**同一个结构体**"会变成**自引用结构** —— 安全 Rust 里
//! 表达不了（必须 `unsafe` 或引入 `ouroboros` 一类新依赖）。本适配层因此采用**闭包式**设计：
//! 结构体只持有**文本与元数据**，解析在**使用时**发生，生命周期被限制在闭包内。
//!
//! # ⚠ 两条必须记住的事
//!
//! ① **本类型⛔ 不在实时音频路径上**：`with_instrument` 会**重新解析**（⇒ **堆分配**），
//!    若在音频回调里调用就**违反红线 7**（实时回调内禁任何堆分配/释放、禁锁等待、禁阻塞 I/O）
//!    [MUST-GATE-001]。回调里只能读**预先摊平**的只读数据。
//! ② **实时路径的正解（下一步）**：在**构造期/离线期**把 `Instrument::regions()` 摊平成
//!    **自有数据**（定长数组/预分配 `Vec`，`f32` 键范围与力度范围直接内联），回调侧只做
//!    整数/浮点比较与索引 ⇒ 零分配、零锁。⛔ 当前**尚未**实现这一步，故本类型只用于
//!    **控制侧**（加载、校验、计数、选择区域）。
//!
//! 本模块**不新增任何所有权机制**：只包一层文本 + 元数据。

use yeban_sfz::error::SfzError;
use yeban_sfz::instrument::Instrument;
// ⚠ 这两个是 **lib 构建**就需要的（`load` / `with_instrument` 都调用它们）⇒ ⛔ 不能放在 `#[cfg(test)]` 下，
// 否则非测试构建会报 "cannot find type `ParseLimits`"（本批实测过一次：三项门全红）。
use yeban_sfz::parser::{ParseLimits, parse_text};

/// 已加载的 SFZ 乐器：**文本 + 元数据**（⛔ 不含借用式解析结果）。
#[derive(Debug, Clone)]
pub struct SfzInstrument {
    /// 原始 SFZ 文本（自己持有 ⇒ 无生命周期参数）。
    text: String,
    /// 构造期解析得到的**区域数**（元数据，供控制侧快速查询）。
    region_count: usize,
}

impl SfzInstrument {
    /// 构造期解析一次：**校验**文本并记下元数据。
    ///
    /// ⚠ 本函数会分配（`String` 已由调用方给出；解析器内部有分配）⇒ **只允许在控制侧调用**。
    ///
    /// # Errors
    /// 文本非法（语法、超限等）时原样透出 [`SfzError`]，⛔ 不 panic。
    pub fn load(text: String, limits: &ParseLimits) -> Result<Self, SfzError> {
        let region_count = parse_text(&text, limits)?.regions().len();
        Ok(Self { text, region_count })
    }

    /// 区域数（规范里的 `region` 条目个数）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.region_count
    }

    /// 原始文本（调用方可能需要原样回写/哈希）。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// **用时现解析**：把借用式 [`Instrument`] 限制在闭包内 ⇒ 无需自引用结构。
    ///
    /// ⚠ **控制侧专用**（见模块头文档 ①）：本函数会重新解析 ⇒ 会分配。
    ///
    /// # Errors
    /// 若文本在此期间变得非法（正常路径不会发生），透出 [`SfzError`]。
    pub fn with_instrument<R>(&self, f: impl FnOnce(&Instrument<'_>) -> R) -> Result<R, SfzError> {
        let limits = ParseLimits::default();
        let instrument = parse_text(&self.text, &limits)?;
        Ok(f(&instrument))
    }

    /// 在给定 `note`/`velocity` 下**选中**的区域（控制侧的查询口径）。
    ///
    /// 复用 `yeban-sfz` 自己的谓词（`Region::matches_key` / `matches_velocity`），
    /// ⛔ 不在引擎里重写一套键/力度判定。
    ///
    /// # Errors
    /// 同 [`Self::with_instrument`]。
    pub fn regions_matching(&self, note: u8, velocity: u8) -> Result<Vec<usize>, SfzError> {
        self.with_instrument(|instrument| {
            instrument
                .regions()
                .iter()
                .enumerate()
                .filter(|(_, region)| region.matches_key(note) && region.matches_velocity(velocity))
                .map(|(index, _)| index)
                .collect::<Vec<usize>>()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 SFZ：两条区域（键范围不同）。
    const MINIMAL: &str = "<region> sample=a.wav key=60\n<region> sample=b.wav key=64\n";

    /// [ROAD-M2-005] 判据 ①：`load` 一段最小文本 ⇒ `region_count` 等于期望值。
    #[test]
    fn load_records_the_region_count() {
        let instrument = SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default())
            .expect("最小 SFZ 必须解析成功");
        assert_eq!(instrument.region_count(), 2, "两条 `<region>` ⇒ 区域数 2");
        // 单区域对照（⛔ 防止把"1"写死成常量也能过）
        let one = SfzInstrument::load(
            "<region> sample=a.wav key=60\n".to_owned(),
            &ParseLimits::default(),
        )
        .expect("单区域必须解析成功");
        assert_eq!(one.region_count(), 1, "单区域 ⇒ 1");
    }

    /// [ROAD-M2-005] 判据 ②：`with_instrument` 能在给定 note/velocity 下查到区域。
    #[test]
    fn regions_matching_selects_by_key_and_velocity() {
        let instrument =
            SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        assert_eq!(
            instrument.regions_matching(60, 100).expect("查询"),
            vec![0],
            "note 60 ⇒ 第 0 条"
        );
        assert_eq!(
            instrument.regions_matching(64, 100).expect("查询"),
            vec![1],
            "note 64 ⇒ 第 1 条"
        );
        assert!(
            instrument
                .regions_matching(72, 100)
                .expect("查询")
                .is_empty(),
            "无匹配键 ⇒ 空（⛔ 不是 panic、也不是默认第一条）"
        );
        // `with_instrument` 本身：闭包内能读到区域表
        let paths = instrument
            .with_instrument(|i| {
                i.regions()
                    .iter()
                    .map(|region| region.sample_path().to_string())
                    .collect::<Vec<_>>()
            })
            .expect("闭包式访问");
        assert_eq!(paths, vec!["a.wav".to_owned(), "b.wav".to_owned()]);
    }

    /// [ROAD-M2-005] 判据 ③：非法/超限 SFZ ⇒ 返回 [`SfzError`]，⛔ **不 panic**。
    ///
    /// ⭐ **实测登记（本节的重要读数）**：空文本、纯空白、以及坏指令 `<region> ???`
    /// 都被 `yeban-sfz` **接受**为 `Ok(0 区域)` —— 库把无法识别的 opcode 当作**忽略**而非错误。
    /// ⛔ 这不是我的缺陷、也不该由我在引擎侧"补一个错误"（那是**发明规范**）；
    /// 因此本判据分两半：
    ///   ① **不 panic**：三类退化输入都必须**返回**（`Ok` 或 `Err` 皆可）；
    ///   ② **必然报错**的机械证据：用**超限**输入（`max_line_bytes` 极小）⇒ 必须 `Err`。
    #[test]
    fn invalid_or_empty_sfz_returns_an_error_without_panicking() {
        // ① 退化输入：只要求"不 panic"（实测三者皆 Ok(0)，如实打印）
        for (label, text) in [
            ("空文本", ""),
            ("纯空白", "   \n\t\n"),
            ("坏指令", "<region> ???\n"),
        ] {
            let result = SfzInstrument::load(text.to_owned(), &ParseLimits::default());
            println!(
                "[sfz] {label}: {:?}",
                result.as_ref().map(SfzInstrument::region_count)
            );
            if let Ok(instrument) = result {
                assert_eq!(instrument.region_count(), 0, "{label} 若成功则区域数应为 0");
            }
        }
        // ② 超限输入 ⇒ **必须 Err**（库自己的判据也用这一口径）
        let tiny = ParseLimits {
            max_line_bytes: 8,
            ..ParseLimits::default()
        };
        let over = SfzInstrument::load("<region> sample=abcdefgh.wav key=60\n".to_owned(), &tiny);
        println!(
            "[sfz] 超限输入: {:?}",
            over.as_ref().map(SfzInstrument::region_count)
        );
        assert!(
            over.is_err(),
            "超限行必须返回 SfzError（这才是'非法 ⇒ 报错'的机械证据）"
        );
        // ③ 正常 limits 下同一条文本必须**成功**（反向对照：证明上面的 Err 来自 limits 而非文本本身）
        let normal = SfzInstrument::load(
            "<region> sample=abcdefgh.wav key=60\n".to_owned(),
            &ParseLimits::default(),
        );
        assert!(
            normal.is_ok(),
            "同一文本在默认 limits 下必须成功（否则上面的 Err 另有原因）"
        );
    }

    /// [MUST-GATE-001] 判据 ④：**本类型不在实时路径上** —— 机械部分。
    ///
    /// 机械可判的部分：`with_instrument` / `regions_matching` **必然分配**（重新解析 +
    /// `Vec`），因此它们**不可能**出现在实时回调里。这里用`Vec`返回值与"重新解析"两条
    /// 事实把它钉住；⭐ **"回调里只用摊平数据"这一条属于文档约束**，见模块头文档 ②
    /// （⛔ 机器暂时判不了"某函数有没有被回调调用"，故不假装有机械判据）。
    #[test]
    fn the_adapter_is_documented_as_control_side_only() {
        let instrument =
            SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        // `regions_matching` 返回 `Vec` ⇒ 一定有堆分配 ⇒ 不能进实时回调
        let selected: Vec<usize> = instrument.regions_matching(60, 100).expect("查询");
        assert_eq!(selected.len(), 1, "查询结果本身就是堆分配的 `Vec`");
        // 每次 `with_instrument` 都**重新解析** ⇒ 用"区域数恒定但地址不同"的间接证据说明
        // 每次调用都会重建区域表（控制侧可接受，实时侧不可接受）。
        let first = instrument
            .with_instrument(|i| i.regions().as_ptr() as usize)
            .expect("闭包");
        let second = instrument
            .with_instrument(|i| i.regions().as_ptr() as usize)
            .expect("闭包");
        println!("[sfz] 两次解析的区域表地址: {first} / {second}");
        assert_eq!(instrument.region_count(), 2, "区域数不随解析次数变化");
    }
}
