//! # yeban-midi
//!
//! SMF（标准 MIDI 文件）的**唯一**编解码实现：`MidiExport::to_smf_bytes` 导出、
//! `parse_smf` / `track_chunks` 回读，含独立 VLQ/chunk 字节级核验。
//!
//! ## 为什么它是一个独立 crate（账本第 273/283 轮）
//!
//! 它原住在 `yeban-render::midi`。但 `yeban-render` 同时依赖 **`yeban-dsp`** 与 **`hound`**（音频栈），
//! 而 `yeban-mcp` 的依赖方向规则是"**不拖音频栈进 MCP**" ⇒ **MCP 拿不到编码器**。
//! 编解码本身**只**需要 `yeban-model` + `midly`，因此把它下移到这里：
//! `yeban-render` 经再导出继续提供 `yeban_render::midi::*`（调用方一行不改），
//! `yeban-mcp` 则可以直接依赖本 crate ⇒ **两侧共用同一实现**。
pub mod export;
pub mod midi;
/// MIDI 可变长度量（VLQ）的零依赖参考编解码 —— `midi` 的字节级核验依赖它。
pub mod vlq;
