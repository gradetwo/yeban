//! 插件效果器包装源（`PluginEffectSource`）：内层源 → 宿主效果器 → 输出。
//!
//! 效果器线 v1 的信号位置（见 MCP 渲染装配）：
//! - 本源包在**轨源输出**之外：内层是轨的复合源（MIDI／插件乐器／音频片段＋
//!   轨音量／声相自动化），效果器处理它的求和输出；内置插入链
//!   （`render_node` 的 `inserts.process`）在**本源之后**才跑，因此效果器
//!   在内置链之前 —— 设备顺序跨家族时以此为准，⛔ 不是设备表顺序。
//! - 每轨只接**首个**非旁通带绑定效果器（与乐器线的“首个胜出”同一条
//!   first-source 语义；其余带绑定效果器仍进 `externalPlugins` 收据）。
//!   多实例效果器链是 slotlists 折中的后续项，不在本源里顺带做。
//! - 无源总线轨（只有入边求和）不走本源：效果器线 v1 只覆盖有源轨；
//!   总线／主总线上的效果器仍进收据，节点级效果器链是后续项。
//!
//! 推进契约与插件源同款：`first_frame` 必须等于本源播放头，否则响亮失败；
//! 每块恰好推进一次（内层与效果器各一次）。
//!
//! ⚠ 本文件不含 `unsafe`（本 crate `#![forbid(unsafe_code)]`）：插件交互只走
//! `PluginInstance` 的安全 trait；`Send + Sync` 由 trait 约束保证。

use yeban_model::EntityId;
use yeban_plugin_host::PluginInstance;

use crate::render::{AudioSource, BlockContext, RenderError};

/// 插件效果器包装源。
pub struct PluginEffectSource {
    /// 内层源（轨的复合源）。
    inner: Box<dyn AudioSource>,
    /// 已激活的效果器实例（构造期激活，见 [`Self::new`]）。
    plugin: Box<dyn PluginInstance>,
    /// 本源负责的轨道（仅错误报文用）。
    track: EntityId,
    /// 本源播放头（绝对帧；与 `BlockContext::first_frame` 对齐）。
    position: u64,
    /// 内层输出暂存（交错缓冲；调用方保证 `out` 已清零，但内层源是累加语义，
    /// 故本暂存每块先清零再交给内层 —— 与 `render_node` 对源的契约同款）。
    tmp: Vec<f32>,
    /// 左声道暂存（拆交错 → 效果器输入；复用，⛔ 不每块重分容量）。
    in_l: Vec<f32>,
    /// 右声道暂存。
    in_r: Vec<f32>,
    /// 左声道暂存（效果器输出 → 合成交错）。
    out_l: Vec<f32>,
    /// 右声道暂存。
    out_r: Vec<f32>,
}

impl PluginEffectSource {
    /// 构造并激活效果器源。
    ///
    /// `max_block` 是插件 `setupProcessing` 的块上限：必须 ≥ 渲染计划的块大小
    /// （调用方保证；超限时 `process_audio` 响亮失败，不静默截断）。
    /// 激活失败 ⇒ [`RenderError::Source`]（响亮失败，与插件乐器源同款）。
    ///
    /// # Errors
    ///
    /// 插件激活失败 ⇒ [`RenderError::Source`]。
    pub fn new(
        mut plugin: Box<dyn PluginInstance>,
        inner: Box<dyn AudioSource>,
        track: EntityId,
        sample_rate: f64,
        max_block: usize,
    ) -> Result<Self, RenderError> {
        plugin
            .activate(sample_rate, max_block)
            .map_err(|error| RenderError::Source {
                node: track,
                message: format!("效果器激活失败: {error}"),
            })?;
        Ok(Self {
            inner,
            plugin,
            track,
            position: 0,
            tmp: Vec::new(),
            in_l: Vec::new(),
            in_r: Vec::new(),
            out_l: Vec::new(),
            out_r: Vec::new(),
        })
    }

    /// 本源负责的轨道 ID。
    #[must_use]
    pub const fn track(&self) -> EntityId {
        self.track
    }

    /// 本源播放头（绝对帧；判据用它证明“每块恰好推进一次”）。
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }
}

impl AudioSource for PluginEffectSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        if context.first_frame != self.position {
            return Err(RenderError::Source {
                node: self.track,
                message: format!(
                    "效果器包装源的推进契约被违反: `first_frame` = {}，播放头 = {} \
                     （调用方必须从第 0 帧起按连续升序的块渲染，每块恰好一次）",
                    context.first_frame, self.position
                ),
            });
        }
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        let width = frames * channels;
        if self.tmp.len() < width {
            self.tmp.resize(width, 0.0);
        }
        if self.in_l.len() < frames {
            self.in_l.resize(frames, 0.0);
            self.in_r.resize(frames, 0.0);
            self.out_l.resize(frames, 0.0);
            self.out_r.resize(frames, 0.0);
        }
        self.tmp[..width].fill(0.0);
        self.inner.render_block(context, &mut self.tmp[..width])?;
        // 拆交错：单声道复制到双声道（与插件乐器源的单声道口径对称：
        // 效果器是立体声，单声道输入先扩成双声道，输出再混回单声道）。
        if channels == 1 {
            for (index, sample) in self.tmp[..frames].iter().enumerate() {
                self.in_l[index] = *sample;
                self.in_r[index] = *sample;
            }
        } else {
            for (index, frame) in self.tmp[..width].chunks_exact(channels).enumerate() {
                self.in_l[index] = frame[0];
                self.in_r[index] = frame[1];
            }
        }
        self.plugin
            .process_audio(
                &self.in_l[..frames],
                &self.in_r[..frames],
                &mut self.out_l[..frames],
                &mut self.out_r[..frames],
            )
            .map_err(|error| RenderError::Source {
                node: self.track,
                message: format!("效果器渲染失败: {error}"),
            })?;
        self.position += frames as u64;
        out.fill(0.0);
        if channels == 1 {
            for (index, sample) in out.iter_mut().take(frames).enumerate() {
                *sample = (self.out_l[index] + self.out_r[index]) * 0.5;
            }
        } else {
            for (index, frame) in out[..width].chunks_exact_mut(channels).enumerate() {
                frame[0] = self.out_l[index];
                frame[1] = self.out_r[index];
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_plugin_host::{StubEffect, StubInstrument};

    /// 测试轨 ID（`EntityId` 的零值构造与插件源测试同款口径）。
    fn track() -> EntityId {
        EntityId::default()
    }

    /// 常量内层源：每块写固定立体声值（左 0.4、右 -0.8），推进播放头。
    struct ConstantSource {
        position: u64,
    }

    impl AudioSource for ConstantSource {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            if context.first_frame != self.position {
                return Err(RenderError::Source {
                    node: track(),
                    message: "内层源跳块".to_owned(),
                });
            }
            let channels = context.channels.max(1);
            let frames = context.frames.min(out.len() / channels);
            out.fill(0.0);
            if channels == 1 {
                for sample in out.iter_mut().take(frames) {
                    *sample = 0.4;
                }
            } else {
                for index in 0..frames {
                    let base = index * channels;
                    out[base] = 0.4;
                    out[base + 1] = -0.8;
                }
            }
            self.position += frames as u64;
            Ok(())
        }
    }

    fn context(first_frame: u64, frames: usize, channels: usize) -> BlockContext {
        BlockContext {
            block_index: 0,
            first_frame,
            frames,
            channels,
            sample_rate: 48_000,
            seed: 0,
        }
    }

    fn wrapped(gain: f64) -> PluginEffectSource {
        let mut effect = StubEffect::new();
        effect.set_parameter(0, gain).expect("设增益");
        PluginEffectSource::new(
            Box::new(effect),
            Box::new(ConstantSource { position: 0 }),
            track(),
            48_000.0,
            1024,
        )
        .expect("构造")
    }

    #[test]
    fn effect_source_scales_inner_output_and_advances() {
        let mut source = wrapped(0.5);
        let mut out = vec![0.0_f32; 64 * 2];
        source
            .render_block(context(0, 64, 2), &mut out)
            .expect("渲染");
        assert_eq!(source.position(), 64);
        assert!((out[0] - 0.2).abs() < 1e-6, "左声道应为 0.4 × 0.5");
        assert!((out[1] + 0.4).abs() < 1e-6, "右声道应为 -0.8 × 0.5");
        let mut out = vec![0.0_f32; 64 * 2];
        source
            .render_block(context(64, 64, 2), &mut out)
            .expect("渲染");
        assert_eq!(source.position(), 128);
        assert!((out[0] - 0.2).abs() < 1e-6, "第二块也应减半");
    }

    #[test]
    fn effect_source_unity_gain_passes_through() {
        let mut source = wrapped(1.0);
        let mut out = vec![0.0_f32; 64 * 2];
        source
            .render_block(context(0, 64, 2), &mut out)
            .expect("渲染");
        assert_eq!(out[0], 0.4);
        assert_eq!(out[1], -0.8);
    }

    #[test]
    fn effect_source_mono_mixes_down() {
        let mut source = wrapped(0.5);
        let mut out = vec![0.0_f32; 64];
        source
            .render_block(context(0, 64, 1), &mut out)
            .expect("渲染");
        // 单声道内层写 0.4 → 扩双声道 → ×0.5 → 混回 (0.2+0.2)*0.5 = 0.2。
        assert!(
            (out[0] - 0.2).abs() < 1e-6,
            "单声道应为双声道输出均值，实为 {}",
            out[0]
        );
    }

    #[test]
    fn effect_source_rejects_skipped_blocks() {
        let mut source = wrapped(0.5);
        let mut out = vec![0.0_f32; 64 * 2];
        let error = source
            .render_block(context(128, 64, 2), &mut out)
            .expect_err("跳块必须失败");
        assert!(matches!(error, RenderError::Source { .. }));
        assert_eq!(source.position(), 0, "失败块不推进");
    }

    #[test]
    fn effect_source_with_instrument_instance_fails_loudly_on_first_block() {
        // 乐器实例没有音频输入总线：构造期激活成功（乐器激活路径），
        // 首块 process_audio 响亮失败 —— 坏接线不配静默过关。
        let mut source = PluginEffectSource::new(
            Box::new(StubInstrument::new()),
            Box::new(ConstantSource { position: 0 }),
            track(),
            48_000.0,
            1024,
        )
        .expect("构造期只激活，不验 kind");
        let mut out = vec![0.0_f32; 64 * 2];
        let error = source
            .render_block(context(0, 64, 2), &mut out)
            .expect_err("乐器实例走效果器路径必须失败");
        assert!(matches!(error, RenderError::Source { .. }));
    }
}
