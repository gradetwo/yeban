//! ⭐ ⭐ **发布装载的后台线程**（CODE_AUDIT_REPORT P0-7）：把 SFZ 摊平与音频
//! 解码移出 UI 线程。
//!
//! ## 它解决的那一个问题
//!
//! `EngineHost::publish_project` 由 60 Hz UI 心跳驱动。缓存未命中的 SFZ 要
//! 读盘＋解析＋解码它引用的全部样本，音频片段要解码＋重采样 —— 原本全在
//! **主线程同步**发生：挂载大音源库后第一次发布冻结界面数秒（P0-7）。
//!
//! ## 模型（照仓内既有 `waveform_cache` 的后台线程模式，但所有权不同）
//!
//! * **每台宿主一条**：worker 住在 [`crate::engine_host::EngineHost`] 的私有
//!   字段里（与 `sfz_cache` 同一条所有权裁决：生命周期＝这一代宿主、单写者、
//!   判据可隔离），⛔ 不是全局。`waveform_cache` 的值是可丢弃的派生数据
//!   （波形峰值），丢了重算即可；装载结果**不可丢弃**（发布在等它），因此
//!   不用全局表＋轮询的形态，而用宿主私有的请求/结果通道。
//! * **懒启动**：第一件未命中的工作才拉起线程（名为 `yeban-arm`）；没有
//!   未命中 ⇒ 线程永不存在，发布路径与改动前逐位相同。
//! * **结果经通道回主线程**：worker 只做纯解码（SFZ 文件 ⇒ `SfzPlan`、
//!   字节 ⇒ 定形 PCM），不碰缓存、不碰快照；主线程在下一次发布时收走结果
//!   并入缓存（`SfzPlanCache::store_prepared`／`AudioPcmCache::absorb`）。
//! * **失败响亮**：解码失败是结果的一种形态，经通道原样回主线程，由发布
//!   路径以既有错误形态报出（SFZ ⇒ `EngineHostError::Sfz`；音频 ⇒ 注入
//!   报告的跳过原因），⛔ 不在 worker 里吞掉。
//! * **线程起不来不许卡死发布**：`spawn` 失败 ⇒ 该件工作**当场在主线程
//!   内联执行**、结果立即可取（退化成旧行为，但发布永不挂死）；解码在
//!   worker 里 panic ⇒ 被 `catch_unwind` 拦下转成响亮失败结果（在飞集合
//!   照常清掉，⛔ 不让发布永远等）；通道断裂 ⇒ 在飞集合清掉，后续发布
//!   重新派发（解码是纯函数，重做安全），⛔ 不静默丢件。
//!
//! ## 线程边界
//!
//! worker 线程只做**控制侧纯计算**（读盘/解码/分配都允许），⛔ 不在任何
//! 实时回调路径上；音频线程仍只读快照里已装好的表/PCM [MUST-GATE-001]。
//! 资产字节由主线程从 `AudioAssetSource` 取好后随工作交给 worker ——
//! 那个来源是 `Box<dyn Fn>`、**不是 `Send`**，⛔ 不能搬进 worker。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use yeban_engine::sfz_instrument::SfzPlan;
use yeban_model::{AssetHash, EntityId};

use crate::audio_inject::DecodeOutcome;
use crate::sfz_load::SfzLoadError;

/// 一件后台工作的**键**（去重与"在飞"判定的单位）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArmJobKey {
    /// SFZ 摊平：轨＋缓存键（解析后路径＋八度覆盖后缀）＋采样率位型。
    Sfz {
        /// 轨。
        track: EntityId,
        /// 缓存键（与 `SfzPlanCache` 的键逐字相同）。
        key: String,
        /// 引擎采样率位型（`f32::to_bits`）。
        hz_bits: u32,
    },
    /// 音频解码：素材哈希＋目标采样率。
    Audio {
        /// 素材哈希。
        hash: AssetHash,
        /// 目标采样率（快照率）。
        target_rate: u32,
    },
}

/// 交给 worker 的一件工作（纯数据，`Send`）。
pub enum ArmJob {
    /// 解码一份 SFZ（路径已在主线程解析完毕）。
    Sfz {
        /// 轨。
        track: EntityId,
        /// 缓存键。
        key: String,
        /// 引擎采样率位型。
        hz_bits: u32,
        /// 解析后的 SFZ 文件路径。
        resolved: PathBuf,
        /// 引擎采样率。
        hz: f32,
        /// 八度覆盖（半音；随项收集，与同步管线同一口径）。
        octave_override: Option<i32>,
    },
    /// 解码一份音频素材的字节（字节已在主线程从资产来源取好）。
    Audio {
        /// 素材哈希。
        hash: AssetHash,
        /// 目标采样率。
        target_rate: u32,
        /// 素材字节。
        bytes: Vec<u8>,
    },
}

impl ArmJob {
    /// 这件工作的键。
    #[must_use]
    pub fn key(&self) -> ArmJobKey {
        match self {
            Self::Sfz {
                track,
                key,
                hz_bits,
                ..
            } => ArmJobKey::Sfz {
                track: *track,
                key: key.clone(),
                hz_bits: *hz_bits,
            },
            Self::Audio {
                hash, target_rate, ..
            } => ArmJobKey::Audio {
                hash: hash.clone(),
                target_rate: *target_rate,
            },
        }
    }
}

/// worker 交回的一件结果（与 [`ArmJob`] 一一对应）。
pub enum ArmOutcome {
    /// SFZ 摊平结果（成功 = 摊平表；失败 = 原始错误，⛔ 不吞）。
    Sfz {
        /// 轨。
        track: EntityId,
        /// 缓存键。
        key: String,
        /// 引擎采样率位型。
        hz_bits: u32,
        /// 解码结果（`SfzPlan` 较大 ⇒ 装箱过通道，⛔ 不让枚举体积被它撑大）。
        result: Result<Box<SfzPlan>, SfzLoadError>,
    },
    /// 音频解码结果（定形产物／形状拒绝／解码失败，见 [`DecodeOutcome`]）。
    Audio {
        /// 素材哈希。
        hash: AssetHash,
        /// 目标采样率。
        target_rate: u32,
        /// 解码结果。
        outcome: DecodeOutcome,
    },
}

/// 一件工作的**失败形态**（worker panic 时用；文本如实说是内部失败，⛔ 不伪装）。
fn failure_outcome(job: &ArmJob) -> ArmOutcome {
    match job {
        ArmJob::Sfz {
            track,
            key,
            hz_bits,
            ..
        } => ArmOutcome::Sfz {
            track: *track,
            key: key.clone(),
            hz_bits: *hz_bits,
            result: Err(SfzLoadError::Parse(
                "后台解码线程内部失败（panic 已被拦下并转为本错误）".to_owned(),
            )),
        },
        ArmJob::Audio {
            hash, target_rate, ..
        } => ArmOutcome::Audio {
            hash: hash.clone(),
            target_rate: *target_rate,
            outcome: DecodeOutcome::Failed(
                "后台解码线程内部失败（panic 已被拦下并转为本错误）".to_owned(),
            ),
        },
    }
}

/// 执行一件工作（worker 线程与"起不来时内联"共用同一份，⛔ 不写两份解码）。
fn run_job(job: ArmJob) -> ArmOutcome {
    match job {
        ArmJob::Sfz {
            track,
            key,
            hz_bits,
            resolved,
            hz,
            octave_override,
        } => ArmOutcome::Sfz {
            track,
            key,
            hz_bits,
            result: crate::sfz_load::decode_sfz_plan_with_override(&resolved, hz, octave_override)
                .map(|(plan, _report)| Box::new(plan)),
        },
        ArmJob::Audio {
            hash,
            target_rate,
            bytes,
        } => ArmOutcome::Audio {
            hash: hash.clone(),
            target_rate,
            outcome: crate::audio_inject::decode_and_shape(&hash, &bytes, target_rate),
        },
    }
}

/// ⭐ 宿主私有的后台装载线程（懒启动、单线程、通道回结果）。
pub struct ArmWorker {
    /// 工作通道发送端（`None` = 线程还没起，或起不来 ⇒ 内联模式）。
    job_sender: Option<Sender<ArmJob>>,
    /// 结果通道接收端。
    outcome_receiver: Receiver<ArmOutcome>,
    /// 结果通道发送端（`new` 时建对，`spawn` 时克隆一份给线程）。
    outcome_sender: Sender<ArmOutcome>,
    /// 已派发、结果还没收走的工作键。
    in_flight: BTreeSet<ArmJobKey>,
    /// 内联执行（线程起不来）的结果暂存（下一次 `drain` 立即可取）。
    inline_done: Vec<ArmOutcome>,
    /// `spawn` 已失败过 ⇒ 不再重试起线程（内联模式是终态，⛔ 不每跳重试）。
    spawn_failed: bool,
}

impl Default for ArmWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl ArmWorker {
    /// 新 worker（线程未启动；第一件工作才拉起）。
    #[must_use]
    pub fn new() -> Self {
        let (outcome_sender, outcome_receiver) = channel();
        Self {
            job_sender: None,
            outcome_receiver,
            outcome_sender,
            in_flight: BTreeSet::new(),
            inline_done: Vec::new(),
            spawn_failed: false,
        }
    }

    /// 这件工作是否已在飞（已派发、结果未收走）。
    #[must_use]
    pub fn in_flight(&self, key: &ArmJobKey) -> bool {
        self.in_flight.contains(key)
    }

    /// 派发一件工作。线程起不来 ⇒ 当场内联执行，结果进暂存（见模块文档）。
    ///
    /// ⚠ 调用方负责先查 [`Self::in_flight`] 去重；重复派发同一键会让结果
    /// 通道里出现两份同键结果（宿主按"后到覆盖"处理，不出错但白解码一次）。
    pub fn dispatch(&mut self, job: ArmJob) {
        let key = job.key();
        if !self.spawn_failed && self.job_sender.is_none() {
            self.try_spawn();
        }
        if let Some(sender) = &self.job_sender {
            match sender.send(job) {
                Ok(()) => {
                    self.in_flight.insert(key);
                    return;
                }
                Err(error) => {
                    // 通道断裂（worker 已死）：清掉发送端与在飞集合，
                    // 本件改走内联，后续工作重新尝试起线程。⛔ 不静默丢件。
                    self.job_sender = None;
                    self.in_flight.clear();
                    self.inline_done.push(run_job(error.0));
                    return;
                }
            }
        }
        // 内联模式（线程起不来）：当场执行，结果立即可取。
        self.inline_done.push(run_job(job));
    }

    /// 拉起 worker 线程（失败 ⇒ 记 `spawn_failed`，转内联模式）。
    fn try_spawn(&mut self) {
        let (job_sender, job_receiver) = channel::<ArmJob>();
        let outcome_sender = self.outcome_sender.clone();
        let spawned = std::thread::Builder::new()
            .name("yeban-arm".to_owned())
            .spawn(move || {
                while let Ok(job) = job_receiver.recv() {
                    // ⭐ panic 不许楔死发布：拦下并转成响亮失败结果，在飞集合
                    //    才能被宿主正常清掉（解码代码 panic 在旧同步路径下是
                    //    直接崩发布，转成错误只会更诚实）。失败形态先按 job
                    //    算好（run_job 会消费 job）。
                    let fallback = failure_outcome(&job);
                    let outcome =
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run_job(job)
                        })) {
                            Ok(outcome) => outcome,
                            Err(_panic) => {
                                eprintln!("[yeban-app] 后台装载线程拦下一件解码 panic");
                                fallback
                            }
                        };
                    if outcome_sender.send(outcome).is_err() {
                        break;
                    }
                }
            });
        match spawned {
            Ok(_handle) => {
                // ⚠ JoinHandle 直接丢弃（detach）：宿主 Drop 时 job_sender
                //    随之 Drop ⇒ recv 返回 Err ⇒ 线程自己退出，⛔ 不留僵尸。
                self.job_sender = Some(job_sender);
            }
            Err(error) => {
                eprintln!(
                    "[yeban-app] 后台装载线程启动失败（{error}），解码改走主线程内联（发布不挂死）"
                );
                self.spawn_failed = true;
            }
        }
    }

    /// 收走全部已就绪的结果（非阻塞）。在飞集合同步清掉对应键。
    ///
    /// 通道断裂且在飞非空 ⇒ 在飞集合清掉（那些工作不会再有结果，后续
    /// 发布会重新派发；见模块文档），⛔ 不让发布永远等下去。
    pub fn drain(&mut self) -> Vec<ArmOutcome> {
        let mut outcomes = std::mem::take(&mut self.inline_done);
        loop {
            match self.outcome_receiver.try_recv() {
                Ok(outcome) => outcomes.push(outcome),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if !self.in_flight.is_empty() {
                        eprintln!(
                            "[yeban-app] 后台装载通道断裂，{} 件在飞工作将由后续发布重新派发",
                            self.in_flight.len()
                        );
                        self.in_flight.clear();
                        self.job_sender = None;
                    }
                    break;
                }
            }
        }
        for outcome in &outcomes {
            self.in_flight.remove(&outcome_key(outcome));
        }
        outcomes
    }
}

/// 一件结果对应的键（与 [`ArmJob::key`] 同形）。
fn outcome_key(outcome: &ArmOutcome) -> ArmJobKey {
    match outcome {
        ArmOutcome::Sfz {
            track,
            key,
            hz_bits,
            ..
        } => ArmJobKey::Sfz {
            track: *track,
            key: key.clone(),
            hz_bits: *hz_bits,
        },
        ArmOutcome::Audio {
            hash, target_rate, ..
        } => ArmJobKey::Audio {
            hash: hash.clone(),
            target_rate: *target_rate,
        },
    }
}

/// 已就绪的音频结果暂存（宿主侧）：键 = (素材, 目标率)。
pub type AudioOutcomeMap = BTreeMap<(AssetHash, u32), DecodeOutcome>;
