//! 进程内测试桩乐器（`stub:sine`）：正弦振荡器，单 `Gain` 参数。
//!
//! 用途：无真实 VST3 插件时证明整条链路（MCP 工具 → 渲染源 → 宿主 → 音频）。
//! ⛔ 桩不走 VST3 ABI；VST3 ABI 本身由 `tests/vst3_loader.rs` 的现场编译夹具证明。

use std::collections::BTreeMap;

use crate::error::HostError;
use crate::types::{MidiEvent, MidiEventKind, ParamInfo, PluginInstance, PluginState};

/// `stub:sine` 的注册标识（`Host::load` 的 `stub:` 前缀分发用）。
pub const STUB_SINE_ID: &str = "stub:sine";

/// 增益参数标识。
pub const STUB_GAIN_PARAM_ID: u32 = 0;

/// 桩乐器的一个发声音符（相位连续累加，避免块边界咔哒声）。
struct Voice {
    /// 频率（Hz，由音高换算）。
    freq_hz: f64,
    /// 力度（0.0..=1.0）。
    velocity: f32,
    /// 当前相位（弧度）。
    phase: f64,
}

/// 正弦桩乐器。
pub struct StubInstrument {
    /// 激活采样率（未激活时为 0.0，`process` 要求先激活）。
    sample_rate: f64,
    /// 增益（参数 0 的值，也是状态的全部内容）。
    gain: f32,
    /// 发声中的音符（键＝`note_id`，⛔ 不是音高：同音高可复音）。
    voices: BTreeMap<i32, Voice>,
    /// 是否已激活。
    active: bool,
}

impl StubInstrument {
    /// 新桩（缺省增益 0.8，与参数表的 `default_normalized` 一致）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            sample_rate: 0.0,
            gain: 0.8,
            voices: BTreeMap::new(),
            active: false,
        }
    }

    /// 音高 → 频率（平均律，A4＝440 Hz）。
    fn pitch_freq_hz(pitch: u8) -> f64 {
        440.0 * 2_f64.powf((f64::from(pitch) - 69.0) / 12.0)
    }
}

impl Default for StubInstrument {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginInstance for StubInstrument {
    fn name(&self) -> &str {
        "stub:sine"
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn activate(&mut self, sample_rate: f64, _max_block: usize) -> Result<(), HostError> {
        self.sample_rate = sample_rate;
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) {
        self.active = false;
        self.voices.clear();
    }

    fn latency_samples(&self) -> u32 {
        0
    }

    fn parameters(&mut self) -> Result<Vec<ParamInfo>, HostError> {
        Ok(vec![ParamInfo {
            id: STUB_GAIN_PARAM_ID,
            title: "Gain".to_owned(),
            short_title: "Gain".to_owned(),
            units: String::new(),
            step_count: 0,
            default_normalized: 0.8,
            flags: 1, // `kCanAutomate`
        }])
    }

    fn parameter_value(&mut self, id: u32) -> Option<f64> {
        if id == STUB_GAIN_PARAM_ID {
            Some(f64::from(self.gain))
        } else {
            None
        }
    }

    fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), HostError> {
        if id == STUB_GAIN_PARAM_ID {
            self.gain = value.clamp(0.0, 1.0) as f32;
            Ok(())
        } else {
            Err(HostError::Other(format!("桩乐器无参数 {id}")))
        }
    }

    fn save_state(&mut self) -> Result<PluginState, HostError> {
        Ok(PluginState {
            processor: self.gain.to_le_bytes().to_vec(),
            controller: Vec::new(),
        })
    }

    fn restore_state(&mut self, state: &PluginState) -> Result<(), HostError> {
        if state.processor.len() != 4 {
            return Err(HostError::StateCorrupt(format!(
                "桩乐器状态应为 4 字节，实为 {} 字节",
                state.processor.len()
            )));
        }
        self.gain = f32::from_le_bytes(
            state
                .processor
                .as_slice()
                .try_into()
                .map_err(|_| HostError::StateCorrupt("桩乐器状态切不出 f32".to_owned()))?,
        );
        Ok(())
    }

    fn process(
        &mut self,
        events: &[MidiEvent],
        frames: usize,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> Result<(), HostError> {
        if !self.active {
            return Err(HostError::NotActivated);
        }
        if out_l.len() != frames || out_r.len() != frames {
            return Err(HostError::Other(format!(
                "输出片长（{}／{}）≠ 帧数 {frames}",
                out_l.len(),
                out_r.len()
            )));
        }
        // 单遍渲染：每个采样点先应用该偏移的事件（偏移 0 的事件在采样 0 之前生效），
        // 再混出该采样；越界事件（偏移 ≥ frames）丢弃。
        let mut ordered: Vec<&MidiEvent> = events.iter().collect();
        ordered.sort_by_key(|event| event.sample_offset);
        let mut next = 0_usize;
        for sample in 0..frames {
            while next < ordered.len() && (ordered[next].sample_offset as usize) == sample {
                let event = ordered[next];
                match event.kind {
                    MidiEventKind::NoteOn {
                        pitch,
                        velocity,
                        note_id,
                        ..
                    } => {
                        self.voices.insert(
                            note_id,
                            Voice {
                                freq_hz: Self::pitch_freq_hz(pitch),
                                velocity,
                                phase: 0.0,
                            },
                        );
                    }
                    MidiEventKind::NoteOff { note_id, .. } => {
                        self.voices.remove(&note_id);
                    }
                }
                next += 1;
            }
            let mut mixed = 0.0_f32;
            for voice in self.voices.values_mut() {
                mixed += voice.phase.sin() as f32 * voice.velocity;
                voice.phase += 2.0 * std::f64::consts::PI * voice.freq_hz / self.sample_rate;
                if voice.phase >= 2.0 * std::f64::consts::PI {
                    voice.phase %= 2.0 * std::f64::consts::PI;
                }
            }
            let sample_value = mixed * self.gain;
            out_l[sample] = sample_value;
            out_r[sample] = sample_value;
        }
        Ok(())
    }
}
