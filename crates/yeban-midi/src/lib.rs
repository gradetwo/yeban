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
//!
//! ## 内存安全
//!
//! 本 crate 不出现 `unsafe`，因此与引擎层四个 crate 同口径加 `#![forbid(unsafe_code)]`
//! （先例：`crates/yeban-decode/src/lib.rs:99`）。
#![forbid(unsafe_code)]

pub mod export;
pub mod midi;
/// MusicXML (`.musicxml`) 的**只读**导入 MVP —— 手写 pull parser，零新依赖。
///
/// ⚠️ 规范**未定义** MusicXML（四份规范里命中数为 0）；本模块是工程选择，
/// 出处见 `docs/ledger/integration-rulings-notes.md` 的 R1。
pub mod musicxml;
/// `.mxl`（**压缩** MusicXML，ZIP 容器）的**只读**导入 —— 手写 raw-DEFLATE inflate，
/// 零新依赖（路线 B；裁决出处见 `docs/ledger/integration-rulings-notes.md` 的 R1，
/// 那是"另立票"的下游）。
///
/// ⚠️ 与 [`musicxml`] 同口径：规范**未定义** MusicXML / `.mxl`；本模块是工程选择。
pub mod mxl;
/// MIDI 可变长度量（VLQ）的零依赖参考编解码 —— `midi` 的字节级核验依赖它。
pub mod vlq;
