//! **常驻判据（R93／R100）**：`yeban_render_master` **不自己发声**采样器音符。
//!
//! # 为什么要有它（一句话的不变量，撑着两条"退役"）
//!
//! `KNOWN_UNSUPPORTED_KEYS` 里 **没有** `sfzSampleSpan` / `sfzRegionLayering`，
//! 理由是 `#274`：采样器轨改由**引擎的 SFZ 支路**发声，而引擎真的按 `offset`/`end`
//! 切片、真的把**每一条**命中区域都建声部叠加。那两条"我们发现不了"的读数因此变成假话。
//!
//! ⚠ 但这条理由有一个**前提**：`domain/render.rs` 自己**不**走 `sfz::note_wave`
//! （那条路只取第一条区域、不切 `span`）。若有人把 `note_wave` 接回 `render.rs`
//! 去"顺手渲染一下"，`sfzSampleSpan` / `sfzRegionLayering` 就立刻回到"其实没做到却
//! 不报"的状态 —— 而**没有任何现有判据会红**：
//!
//! * `tests/render_master.rs::sfz_region_layering_is_applied_and_no_longer_reported`
//!   只看**输出峰值**，接回 `note_wave` 后它自己的单层/双层夹具仍会走它（峰值仍会变），
//!   而且它测的是"报了没有"，不是"谁在发声"；
//! * `the_known_unsupported_list_is_not_decoration` 只逼"清单里的键都在具体输入下出现"，
//!   反向（不该出现的键不再出现）靠的是渲染路径本身。
//!
//! ⇒ 本文件把"谁在发声"钉成源码级读数。
//!
//! # 口径
//!
//! * `include_str!` **编译期**固定读 `src/domain/render.rs`（R100：⛔ 运行期找夹具，
//!   文件一旦挪走就**编不过**，而不是静默跳过）；
//! * 匹配的是**调用形状** `note_wave(` ／ `sfz::render(`（⚠ 注释里也不要写这两个带括号
//!   的形状，否则同样会红 —— 那不是误伤，是"读起来像调用"的文档必须改写）；
//! * 反向的非真空：`sfz::survey_sampler(` **必须**出现（否则"只取读数"这句话也失去了载体）。
//!
//! # 另一半证据（别处，不在本文件）
//!
//! * 引擎真的应用 `span`：`crates/yeban-engine/tests/sfz_instrument.rs` 的 S13
//!   （`offset=10` ⇒ 输出第 0 帧**逐位等于** `源[10] × gain`，写入帧数 190；`offset=50`
//!   ⇒ 指纹与波形都变）；
//! * 引擎真的叠加多区域：`tests/render_master.rs::sfz_region_layering_is_applied_and_no_longer_reported`
//!   （双层峰值显著高于单层，且 `sfzRegionLayering` 不再出现）。

/// `yeban_render_master` 的实现文件（**编译期**固定）。
const RENDER_SRC: &str = include_str!("../src/domain/render.rs");

#[test]
fn the_master_never_renders_sampler_notes_itself() {
    // ① 非真空：读到的确实是那一份文件（两个只有它会有的记号）。
    assert!(
        RENDER_SRC.contains("KNOWN_UNSUPPORTED_KEYS"),
        "`include_str!` 读到的不是 `domain/render.rs`（缺 `KNOWN_UNSUPPORTED_KEYS`）"
    );
    assert!(
        RENDER_SRC.contains("sfz::survey_sampler("),
        "`domain/render.rs` 不再调 `sfz::survey_sampler(` ⇒ 采样器读数（逐音符计数）没载体了：\
         要么路径改了（请同步改本判据并说明新的读数来源），要么调用被误删"
    );

    // ② 核心不变量：母带**不**自己发声。
    assert!(
        !RENDER_SRC.contains("note_wave("),
        "`domain/render.rs` 出现了 `note_wave(` ⇒ 母带又开始自己选样本发声了。\
         那条路只取**第一条**命中区域、**不**切 `offset`/`end` ⇒ `KNOWN_UNSUPPORTED_KEYS` \
         里退役 `sfzSampleSpan` / `sfzRegionLayering` 就变成假话（用户听到的音频与服务端\
         宣称的「已做到」不符）。⛔ 不要让母带发声：发声走引擎的 SFZ 支路（`#274`）"
    );
    assert!(
        !RENDER_SRC.contains("sfz::render("),
        "`domain/render.rs` 调了 `sfz::render(` ⇒ 那是 `yeban_render_sfz_wav` 的\
         **预览**渲染入口（只取第一条区域、不展开循环），母带用它会得到与引擎不同的音频"
    );
}
