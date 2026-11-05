//! 插件轨音源（`PluginTrackSource`）：轨 MIDI → 宿主乐器 → 音频。
//!
//! 一期范围（ADR-0069 D5／D7／D10）：
//! - 音源优先级由调用方（MCP 渲染装配）裁决：本源只负责“插件已选定”后的渲染。
//! - 事件：只发音符 on／off（帧精确，`note_id`＝调度下标；同块同偏移先 off 后 on）。
//!   CC／弯音／滑音一期不发（VST3 无通用 CC 输入事件；`ScheduledNote` 也不带通道）。
//! - 声相与音量：插件立体声 × `schedule.track_gain()`（静态推子，与引擎路径同一口径）
//!   → 声相增益逐声道；居中 `(1.0, 1.0)` 时声相不改值；单声道导出 `(L+R)*0.5`
//!   （声相无意义）；＞2 声道时 L／R 进前两声道、其余 0。
//! - 推进契约与引擎源同款：`first_frame` 必须等于本源播放头，否则响亮失败；
//!   每块恰好推进一次；调度表为 `None` 的轨也推进（但不调插件，直接静音——
//!   无音符的轨不应让自振荡插件出声）。
//!
//! 事件发射的游标设计（⛔ 不用 `first_sounding_index`）：
//! 调度表按 `start_sample` 排序，但 `end_sample` 不单调（长音符的终点晚于
//! 后续短音符），`first_sounding_index` 的 `partition_point` 前提（`end` 单调）
//! 在此不成立，用它会漏掉关音 ⇒ 粘音。因此本源用双游标：
//! - `cursor`：下一个待发 on 的调度下标（按 `start` 单调推进，块连续时不回退）；
//! - `active`：已发 on、待发 off 的 `(end_sample, note_id)` 表（每块筛一次）。
//!
//! 块总是从第 0 帧起连续渲染（渲染计划的契约），故每个音符的 on／off 各发恰好一次。
//!
//! ⚠ 本文件不含 `unsafe`（本 crate `#![forbid(unsafe_code)]`）：插件交互只走
//! `PluginInstance` 的安全 trait；`Send + Sync` 由 trait 约束保证（见 trait 文档）。

use yeban_engine::synth::NoteSchedule;
use yeban_model::EntityId;
use yeban_plugin_host::{MidiEvent, MidiEventKind, PluginInstance};

use crate::render::{AudioSource, BlockContext, RenderError};

/// 插件轨音源。
pub struct PluginTrackSource {
    /// 本源负责的轨道。
    track: EntityId,
    /// 该轨的调度表（`None` ⇒ 该轨静音，但仍推进播放头）。
    schedule: Option<NoteSchedule>,
    /// 已激活的插件实例（构造期激活，见 [`Self::new`]）。
    plugin: Box<dyn PluginInstance>,
    /// 轨声相（`(左, 右)`；默认 `(1.0, 1.0)`＝居中，与引擎源同款）。
    pan: (f32, f32),
    /// 本源播放头（绝对帧；与 `BlockContext::first_frame` 对齐）。
    position: u64,
    /// 下一个待发 on 的调度下标。
    cursor: usize,
    /// 待发 off 的音符（`(end_sample, note_id, pitch)`；关音带原音高，
    /// 插件按 `note_id` 匹配声部，音高是辅助信息）。
    active: Vec<(u64, i32, u8)>,
    /// 左声道暂存（插件输出）。
    left: Vec<f32>,
    /// 右声道暂存（插件输出）。
    right: Vec<f32>,
    /// 块内事件暂存（复用，⛔ 不每块分配新 `Vec` 的容量，只复用同一块缓冲）。
    events: Vec<MidiEvent>,
}

impl PluginTrackSource {
    /// 构造并激活插件源。
    ///
    /// `max_block` 是插件 `setupProcessing` 的块上限：必须 ≥ 渲染计划的块大小
    /// （调用方保证；超限时 `process` 响亮失败，不静默截断）。
    /// 激活失败 ⇒ [`RenderError::Source`]（响亮失败，见 ADR-0069 D6）。
    ///
    /// # Errors
    ///
    /// 插件激活失败 ⇒ [`RenderError::Source`]。
    pub fn new(
        mut plugin: Box<dyn PluginInstance>,
        track: EntityId,
        schedule: Option<NoteSchedule>,
        sample_rate: f64,
        max_block: usize,
        pan: (f32, f32),
    ) -> Result<Self, RenderError> {
        plugin
            .activate(sample_rate, max_block)
            .map_err(|error| RenderError::Source {
                node: track,
                message: format!("插件激活失败: {error}"),
            })?;
        Ok(Self {
            track,
            schedule,
            plugin,
            pan,
            position: 0,
            cursor: 0,
            active: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
            events: Vec::new(),
        })
    }

    /// 写入轨声相（`(左, 右)`；构造期一次，与引擎源同款）。
    #[must_use]
    pub const fn with_pan(mut self, pan: (f32, f32)) -> Self {
        self.pan = pan;
        self
    }

    /// 本源负责的轨道 ID。
    #[must_use]
    pub const fn track(&self) -> EntityId {
        self.track
    }

    /// 从调度表来的已排程音符数（`sources[].notes` 的插件分支来源）。
    #[must_use]
    pub fn scheduled_notes(&self) -> usize {
        self.schedule.as_ref().map_or(0, NoteSchedule::len)
    }

    /// 本源播放头（绝对帧；判据用它证明“每块恰好推进一次”）。
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// 本块的事件发射（窗口 `[window_start, window_end)`；调用方保证块连续）。
    fn emit_events(&mut self, window_start: u64, window_end: u64) {
        self.events.clear();
        let Some(schedule) = &self.schedule else {
            return;
        };
        let notes = schedule.notes();
        // 关音：`active` 里终点落在本窗口的（终点早于窗口起点的防御分支：
        // 块连续时不存在，万一出现也发在偏移 0，⛔ 不丢关音粘住插件）。
        let mut kept: Vec<(u64, i32, u8)> = Vec::with_capacity(self.active.len());
        for (end, note_id, pitch) in self.active.drain(..) {
            if end < window_end {
                let offset = end.saturating_sub(window_start);
                self.events.push(MidiEvent {
                    sample_offset: offset as u32,
                    kind: MidiEventKind::NoteOff {
                        channel: 0,
                        pitch,
                        velocity: 0.0,
                        note_id,
                    },
                });
            } else {
                kept.push((end, note_id, pitch));
            }
        }
        self.active = kept;
        // 开音：`cursor` 起、起点落在本窗口的（起点早于窗口起点的防御分支同上）。
        while self.cursor < notes.len() {
            let note = &notes[self.cursor];
            if note.start_sample() >= window_end {
                break;
            }
            let offset = note.start_sample().saturating_sub(window_start);
            let note_id = self.cursor as i32;
            self.events.push(MidiEvent {
                sample_offset: offset as u32,
                kind: MidiEventKind::NoteOn {
                    channel: 0,
                    pitch: note.pitch(),
                    velocity: f32::from(note.velocity()) / 127.0,
                    note_id,
                },
            });
            self.active.push((note.end_sample(), note_id, note.pitch()));
            self.cursor += 1;
        }
        // 同块同偏移先 off 后 on（ADR-0069 D7）；稳定排序保同组内调度顺序。
        // 弯音排在开音之后：宿主把弯音拆进参数变化通道（与音符事件无关），
        // 此顺序只影响桩路径，插件侧的弯音生效点只看参数点的采样偏移。
        self.events.sort_by_key(|event| {
            (
                event.sample_offset,
                match event.kind {
                    MidiEventKind::NoteOff { .. } => 0,
                    MidiEventKind::NoteOn { .. } => 1,
                    MidiEventKind::PitchBend { .. } => 2,
                },
            )
        });
    }
}

impl AudioSource for PluginTrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        if context.first_frame != self.position {
            return Err(RenderError::Source {
                node: self.track,
                message: format!(
                    "插件音源的推进契约被违反: `first_frame` = {}，播放头 = {} \
                     （调用方必须从第 0 帧起按连续升序的块渲染，每块恰好一次）",
                    context.first_frame, self.position
                ),
            });
        }
        let channels = context.channels.max(1);
        let frames = context.frames.min(out.len() / channels);
        if self.left.len() < frames {
            self.left.resize(frames, 0.0);
            self.right.resize(frames, 0.0);
        }
        // 调度表为 `None`：不调插件、直接静音，但仍推进（见模块文档）。
        // （增益先按值取出：`emit_events` 要 `&mut self`，不能与调度表借用共存。）
        let gain = match &self.schedule {
            None => {
                out.fill(0.0);
                self.position += frames as u64;
                return Ok(());
            }
            Some(schedule) => schedule.track_gain(),
        };
        self.emit_events(context.first_frame, context.first_frame + frames as u64);
        self.plugin
            .process(
                &self.events,
                frames,
                &mut self.left[..frames],
                &mut self.right[..frames],
            )
            .map_err(|error| RenderError::Source {
                node: self.track,
                message: format!("插件渲染失败: {error}"),
            })?;
        self.position += frames as u64;
        out.fill(0.0);
        let (gain_l, gain_r) = self.pan;
        if channels == 1 {
            for (sample, (left, right)) in out
                .iter_mut()
                .zip(self.left.iter().zip(self.right.iter()))
                .take(frames)
            {
                *sample = (left + right) * 0.5 * gain;
            }
        } else {
            for index in 0..frames {
                let base = index * channels;
                out[base] = self.left[index] * gain * gain_l;
                out[base + 1] = self.right[index] * gain * gain_r;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_engine::synth::ScheduledNote;
    use yeban_plugin_host::StubInstrument;

    /// 测试轨 ID（`EntityId` 的零值构造与引擎源测试同款口径）。
    fn track() -> EntityId {
        EntityId::default()
    }

    /// 单音符调度表（起点 `start`、终点 `end`、A4、满力度；轨增益 `gain`）。
    fn one_note_schedule(start: u64, end: u64, gain: f32) -> NoteSchedule {
        NoteSchedule::from_sorted(vec![ScheduledNote::new(
            start, end, 69, 127, 440.0, 1.0, 48_000.0,
        )])
        .with_track_gain(gain)
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

    #[test]
    fn plugin_source_renders_stub_sine_and_advances() {
        let mut source = PluginTrackSource::new(
            Box::new(StubInstrument::new()),
            track(),
            Some(one_note_schedule(0, 64, 1.0)),
            48_000.0,
            1024,
            (1.0, 1.0),
        )
        .expect("构造");
        let mut out = vec![0.0_f32; 64 * 2];
        source
            .render_block(context(0, 64, 2), &mut out)
            .expect("渲染");
        assert_eq!(source.position(), 64);
        assert_eq!(out[0], 0.0, "正弦初相为 0");
        // 桩缺省增益 0.8（`StubInstrument::new`，与参数表缺省一致）⇒ 期望值带 0.8。
        let expected = (2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() as f32 * 0.8;
        assert!(
            (out[2] - expected).abs() < 1e-4,
            "第 2 采样应 ≈ sin 步进 × 桩增益，实为 {}",
            out[2]
        );
        assert_eq!(out[1], out[0], "双声道首采样同为 0");
        // 下一块：音符已关（终点 64），应静默且播放头继续推进。
        let mut out = vec![0.0_f32; 64 * 2];
        source
            .render_block(context(64, 64, 2), &mut out)
            .expect("渲染");
        assert_eq!(source.position(), 128);
        assert_eq!(out.len(), 64 * 2);
        assert!(out.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn plugin_source_applies_track_gain_and_pan() {
        let mut source = PluginTrackSource::new(
            Box::new(StubInstrument::new()),
            track(),
            Some(one_note_schedule(0, 64, 0.5)),
            48_000.0,
            1024,
            (0.5, 0.0),
        )
        .expect("构造");
        let mut out = vec![0.0_f32; 64 * 2];
        source
            .render_block(context(0, 64, 2), &mut out)
            .expect("渲染");
        let expected =
            (2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() as f32 * 0.8 * 0.5 * 0.5;
        assert!(
            (out[2] - expected).abs() < 1e-4,
            "左声道应为插件输出 × 桩增益 0.8 × 轨增益 0.5 × 声相 0.5，实为 {}",
            out[2]
        );
        assert_eq!(out[3], 0.0, "右声相 0 应静默");
    }

    #[test]
    fn plugin_source_mono_mixes_down() {
        let mut source = PluginTrackSource::new(
            Box::new(StubInstrument::new()),
            track(),
            Some(one_note_schedule(0, 64, 1.0)),
            48_000.0,
            1024,
            (0.0, 0.0),
        )
        .expect("构造");
        let mut out = vec![0.0_f32; 64];
        source
            .render_block(context(0, 64, 1), &mut out)
            .expect("渲染");
        // 桩缺省增益 0.8；单声道 (L+R)*0.5 在 L=R 时即桩输出本身。
        let expected = (2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() as f32 * 0.8;
        assert!(
            (out[1] - expected).abs() < 1e-4,
            "单声道应为 (L+R)*0.5（声相无意义），实为 {}",
            out[1]
        );
    }

    #[test]
    fn plugin_source_none_schedule_is_silent_but_advances() {
        let mut source = PluginTrackSource::new(
            Box::new(StubInstrument::new()),
            track(),
            None,
            48_000.0,
            1024,
            (1.0, 1.0),
        )
        .expect("构造");
        let mut out = vec![1.0_f32; 64 * 2];
        source
            .render_block(context(0, 64, 2), &mut out)
            .expect("渲染");
        assert_eq!(source.position(), 64);
        assert_eq!(out.len(), 64 * 2);
        assert!(out.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn plugin_source_rejects_skipped_blocks() {
        let mut source = PluginTrackSource::new(
            Box::new(StubInstrument::new()),
            track(),
            Some(one_note_schedule(0, 64, 1.0)),
            48_000.0,
            1024,
            (1.0, 1.0),
        )
        .expect("构造");
        let mut out = vec![0.0_f32; 64 * 2];
        let error = source
            .render_block(context(128, 64, 2), &mut out)
            .expect_err("跳块必须失败");
        assert!(matches!(error, RenderError::Source { .. }));
        assert_eq!(source.position(), 0, "失败块不推进");
    }
}
