//! **A/B 试听的控制面落点**（`[ARCH-RT-005]`）：把"主线 vs 提案"两份工程交给引擎的
//! 盲听机制（`attach_audition_branch` ⇒ `begin_audition`）。
//!
//! ## 为什么这份代码住在 `yeban-app`（而不是 `yeban-mcp`）
//!
//! ⭐ `yeban-app` 是**全仓唯一**依赖 `yeban-engine` 的 crate（`yeban-mcp/Cargo.toml` 自己
//! 写明了不加引擎依赖的理由，且加了会变 `mcp-dependency-direction` 守卫的第二条例外）
//! ⇒ 试听这条路只能落在本 crate。
//!
//! ## ⚠ 如实登记的边界（⛔ 不假装已接）
//!
//! * **app 侧今天没有真实提案表示**：`ui/dialogs/musical_pr_drawer.slint` 的
//!   `proposal-count` / `proposal-labels` 等仍是**内联演示常量**，领域侧的
//!   `yeban_propose_section` / `yeban_merge_proposal` 要的是 `proposalId`
//!   ⇒ ⭐ **故本模块把"两份工程"做成**可注入参数**，判据喂**合成工程**；
//!   ⭐ **真实提案接入仍归提案线**（`ADR-0005` 的"界面侧不新造提案状态"约束）。
//! * **真实听感本机无法验证**（无声卡监听回路）⇒ 判据只证"接受/拒绝/回基线"的**结构性**
//!   事实，⛔ 不声称"听过"。
//!
//! ## 一条纪律：`NoMainSnapshot` 也算一种拒绝
//!
//! 引擎的 `AuditionRefusal` 有**五**个值（`NoMainSnapshot` ＋ 四种拓扑不一致）
//! ⇒ 本模块把它们**逐值**转成结构化读数，⛔ 不合并、⛔ 不丢。

use yeban_engine::rt::{AutomationTargets, EngineRuntime};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot};
use yeban_model::{EntityId, YebanProjectV1};

/// ⭐ 一次试听尝试的结果。
///
/// `Err` 的形状**刻意**是结构化的：`reason`（一族）＋ `mismatch`（**哪一个**不一致）
/// ⇒ 界面/控制面可以照它出人话，⛔ 不需要解析字符串。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditionStart {
    /// ⭐ 接受：两份工程拓扑一致，试听已开始。
    Started,
    /// ⛔ 拒绝，并**逐字**带上引擎给的那一个值。
    Refused(AuditionMismatch),
}

/// ⭐ 引擎五种拒绝的**本仓结构化镜像**（⭐ 与 `AuditionRefusal::as_str()` **逐字**同值）。
///
/// ⚠ **单独一个类型**（而不是直接用引擎的枚举）：控制面的读数需要"稳定短名"，而
/// 那条映射必须**被钉住**（判据 `the_refusal_short_names_are_pinned_literals` 逐值断言）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditionMismatch {
    /// 主线侧还没有任何快照。
    NoMainSnapshot,
    /// 音频配置（采样率 / 块长 / 声道数 / 声相律）不一致。
    AudioConfig,
    /// 设备链（轨道集合 / 每轨混音参数 / 插入参数）不一致。
    DeviceChain,
    /// 自动化目标指纹不一致。
    AutomationTargets,
    /// PDC 计划不一致。
    PdcPlan,
}

impl AuditionMismatch {
    /// ⭐ 稳定短名（⭐ **与 `AuditionRefusal::as_str()` 逐字相同** —— 判据逐值钉住）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMainSnapshot => "no-main-snapshot",
            Self::AudioConfig => "audio-config",
            Self::DeviceChain => "device-chain",
            Self::AutomationTargets => "automation-targets",
            Self::PdcPlan => "pdc-plan",
        }
    }

    /// ⭐ 全部五个值（**规范顺序**）：判据与文档共用这一份真相。
    pub const ALL: [Self; 5] = [
        Self::NoMainSnapshot,
        Self::AudioConfig,
        Self::DeviceChain,
        Self::AutomationTargets,
        Self::PdcPlan,
    ];
}

/// ⭐ 结构化读数里 `reason` 那一个字的**唯一**字面（`[UI-A11Y-*]` 同款：错误面要能被断言）。
pub const AUDITION_TOPOLOGY_MISMATCH_REASON: &str = "auditionTopologyMismatch";

impl AuditionStart {
    /// ⭐ `reason`（⭐ `None` ＝ 接受）。
    #[must_use]
    pub const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Started => None,
            Self::Refused(_) => Some(AUDITION_TOPOLOGY_MISMATCH_REASON),
        }
    }

    /// ⭐ `mismatch`（⭐ `None` ＝ 接受）。
    #[must_use]
    pub const fn mismatch(self) -> Option<AuditionMismatch> {
        match self {
            Self::Started => None,
            Self::Refused(which) => Some(which),
        }
    }
}

/// ⭐ **从模型推导自动化目标指纹**（⭐ 宿主侧的定长投影）。
///
/// 口径：每一条**轨道**（按 `EntityId` 升序 ⇒ `from_sorted` 要求的"已排序"）记它的
/// `automation_lanes` **条数**。⛔ 不含 master（它不是被自动化的目标）。
///
/// ⚠ **为什么本模块自己推导**：引擎只提供构造器（`empty` / `from_sorted`），
/// ⛔ **不提供**"从快照导出"的路（`grep` 实测：`crates/yeban-engine/src` 里没有
/// `automation_targets` 的读取面）⇒ 投影责任在**宿主**，也就是这里。
#[must_use]
pub fn automation_targets_for(project: &YebanProjectV1) -> AutomationTargets {
    let mut tracks: Vec<&EntityId> = project.tracks.keys().collect();
    tracks.sort_unstable();
    let pairs: Vec<(EntityId, u16)> = tracks
        .into_iter()
        .map(|id| {
            let lanes = project
                .tracks
                .get(id)
                .map_or(0, |track| track.automation_lanes.len());
            (*id, u16::try_from(lanes).unwrap_or(u16::MAX))
        })
        .collect();
    AutomationTargets::from_sorted(&pairs)
}

/// ⭐ **一份已投影好的试听分支**（主线条 ＋ 提案条 ＋ 各自的快照句柄）。
///
/// ⚠ **为什么自己持有 `Arc<EngineSnapshot>`**：引擎的 `SnapshotSlot` **不暴露**它槽里
/// 那一份（公开面只有 `new` / `publish` / `publish_arc`），而 `begin_audition` 要的正是
/// **那份 `Arc`**（它按值收 ⇒ 试听期间由引擎持有）⇒ 宿主必须在**投影那一刻**把句柄留下来。
///
/// `retire` 必须由持有者**留到停机**（`attach_audition_branch` 的契约）。
pub struct AuditionBranch {
    /// 主线条（引擎侧"当前快照"用它）。
    pub main_slot: std::sync::Arc<SnapshotSlot>,
    /// 提案条（`attach_audition_branch` 用它）。
    pub proposal_slot: std::sync::Arc<SnapshotSlot>,
    /// 主线那一份快照句柄。
    pub main_snapshot: std::sync::Arc<EngineSnapshot>,
    /// 提案那一份快照句柄（`begin_audition` 按值收）。
    pub proposal_snapshot: std::sync::Arc<EngineSnapshot>,
}

/// ⭐ **把两份工程投影成（未挂载的）试听分支**（⭐ 纯构造，⛔ 不碰设备、⛔ 不起线程）。
///
/// # Errors
///
/// 任一份工程投影失败时原样透出**人话原因**（⛔ 不吞、⛔ 不 panic）。
pub fn project_branch(
    main: &YebanProjectV1,
    proposal: &YebanProjectV1,
) -> Result<(AuditionBranch, yeban_engine::snapshot::RetireProducer), String> {
    // ⚠ **每份恰好投影一次**：先建成 `Arc`，槽与"句柄"共用**同一份**（`Arc::clone`）。
    // 早期版本把两份工程各投影了两次（`SnapshotSlot::new` 会吞掉快照）—— 纯浪费，实测记下。
    let main_snapshot = std::sync::Arc::new(
        EngineSnapshot::from_project(main, 1).map_err(|error| format!("主线投影失败: {error}"))?,
    );
    let proposal_snapshot = std::sync::Arc::new(
        EngineSnapshot::from_project(proposal, 2)
            .map_err(|error| format!("提案投影失败: {error}"))?,
    );
    // ⚠ `SnapshotSlot::new` 按值收 ⇒ 槽里那一份是**克隆**（`EngineSnapshot: Clone`）。
    // 克隆比 `from_project` 便宜得多：**投影只发生一次**，克隆只是内存拷贝。
    let main_slot = SnapshotSlot::new((*main_snapshot).clone());
    let proposal_slot = SnapshotSlot::new((*proposal_snapshot).clone());
    // 撤回通道：`retire` **按值**交给 `attach_audition_branch`（它不是 `Clone`）。
    let (retire, _retire_queue) = yeban_engine::snapshot::retire_channel(RETIRE_CAPACITY);
    Ok((
        AuditionBranch {
            main_slot,
            proposal_slot,
            main_snapshot,
            proposal_snapshot,
        },
        retire,
    ))
}

/// ⭐ 本模块自用的撤回通道容量（⭐ 与 `engine_host::RETIRE_CAPACITY` 同量级；本模块
/// ⛔ 不 import 那个私有常量）。
const RETIRE_CAPACITY: usize = 32;

/// ⭐ **在既有 runtime 上开始试听**（⭐ 本模块的唯一"动作"函数）。
///
/// 形状刻意做成"runtime 进、结果出"：判据可以自建 runtime（⛔ 不需要真设备），
/// 产品路径由宿主在它自己的 runtime 上调用。
///
/// 顺序（⭐ 与引擎的纪律一致）：`attach_audition_branch` 只**持有** ⇒ 再
/// `begin_audition` 做拓扑比对 ⇒ 拒绝时**不留半个分支**（`end_audition` 清干净）。
pub fn start(
    runtime: &mut EngineRuntime,
    branch: &AuditionBranch,
    main_project: &YebanProjectV1,
    proposal_project: &YebanProjectV1,
    retire: yeban_engine::snapshot::RetireProducer,
) -> AuditionStart {
    // ⚠ 顺序：先挂分支（引擎要求"只在音频线程启动前"）。
    runtime.attach_audition_branch(&branch.proposal_slot, retire);
    let main_targets = automation_targets_for(main_project);
    let proposal_targets = automation_targets_for(proposal_project);
    match runtime.begin_audition(&branch.proposal_snapshot, main_targets, proposal_targets) {
        Ok(()) => AuditionStart::Started,
        Err(refusal) => {
            // ⭐ 拒绝 ⇒ **不留半个分支**：把刚挂上的那条清掉（`end_audition` 幂等）。
            runtime.end_audition();
            AuditionStart::Refused(mismatch_of(refusal.as_str()))
        }
    }
}

/// ⭐ 把引擎的稳定短名映到本模块的枚举（⭐ **唯一的映射点**）。
///
/// ⚠ 兜底是**显式**的：引擎将来加第六种拒绝时，它落进 `PdcPlan` ⇒ 判据
/// `the_refusal_short_names_are_pinned_literals` 会因 `ALL` 与实测不一致而红
/// （⛔ 不静默错认）。
#[must_use]
pub fn mismatch_of(short_name: &str) -> AuditionMismatch {
    match short_name {
        "no-main-snapshot" => AuditionMismatch::NoMainSnapshot,
        "audio-config" => AuditionMismatch::AudioConfig,
        "device-chain" => AuditionMismatch::DeviceChain,
        "automation-targets" => AuditionMismatch::AutomationTargets,
        _ => AuditionMismatch::PdcPlan,
    }
}

/// ⭐ **结束试听**（幂等）：回到主线。
pub fn stop(runtime: &mut EngineRuntime) {
    runtime.end_audition();
}

/// ⭐ 试听是否在跑（⭐ 读数，⛔ 不改变状态）。
#[must_use]
pub fn is_running(runtime: &EngineRuntime) -> bool {
    runtime.is_auditioning()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_engine::meter::meter_channel;
    use yeban_engine::ring::event_channel;
    use yeban_engine::snapshot::retire_channel;

    /// ⭐ 判据用的 runtime（⛔ 不开设备、⛔ 不起线程）—— 与 `engine_host::fresh_runtime`
    /// **同一批通道**，只是容量写死（本模块 ⛔ 不 import 那些私有常量）。
    fn runtime_for(slot: &std::sync::Arc<SnapshotSlot>) -> EngineRuntime {
        let (retire, _queue) = retire_channel(32);
        let (_events, receiver) = event_channel(256);
        let (publisher, _collector) = meter_channel(256);
        EngineRuntime::new(slot, retire, receiver, publisher)
    }

    /// ⭐ **逐位**指纹：走**公开读取面**，浮点一律 `to_bits()`（⛔ 不用 `==`）。
    ///
    /// 为什么必须是位：`==` 会把 `-0.0 == 0.0` 判等（位不同）、把 `NaN != NaN` 判不等
    /// （位可能相同）⇒ 两种都会让"回基线"的判据说谎。
    ///
    /// ⚠ `EngineSnapshot` ⛔ 不实现 `Serialize` ⇒ 指纹只能从它的**公开读取面**取；
    /// 本函数覆盖：采样率 / 块长 / 声道数 / 声相律 / 轨道表 / 插入表 / 每条轨道的音高增益。
    fn bits_of(snapshot: &EngineSnapshot) -> Vec<u64> {
        let mut out = vec![
            u64::from(snapshot.sample_rate()),
            snapshot.block_frames() as u64,
            u64::from(snapshot.channels()),
            snapshot.pan_law() as u64,
        ];
        for (id, params) in snapshot.tracks() {
            out.push(fnv1a(id.to_canonical_string().as_bytes()));
            // ⚠ `TrackParams` 的字段**全私有**（`snapshot.rs:207`）⇒ 只能读**派生量**
            // （`pan_gains`）与它的 `Debug` 文本。Debug 文本把**每一个**字段都写出来
            // ⇒ 任何一处被就地改写都会让指纹变（这是"回基线"判据的**信号源**）。
            out.push(fnv1a(format!("{params:?}").as_bytes()));
            let (left, right) = params.pan_gains(snapshot.pan_law());
            out.push(left.to_bits().into());
            out.push(right.to_bits().into());
        }
        // ⚠ `inserts()` 的 `InsertParams` **字段全私有**（`insert.rs:412`）⇒ 本指纹
        // 取不到它，如实**不覆盖**那一层（`tracks()` 已覆盖轨道表与混音参数）。
        out
    }

    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    fn fixture() -> YebanProjectV1 {
        yeban_model::samples::filled_project()
    }

    /// ⭐ **正对照（第 42 条）：拓扑相同 ⇒ 必须接受。**
    ///
    /// ⚠ 没有这一条，一个"永远拒绝"的实现可以把四条**负**判据全部骗过。
    #[test]
    fn identical_topologies_are_accepted() {
        let main = fixture();
        let proposal = main.clone();
        let (branch, retire) = project_branch(&main, &proposal).expect("投影");
        let mut runtime = runtime_for(&branch.main_slot);
        let started = start(&mut runtime, &branch, &main, &proposal, retire);
        assert_eq!(started, AuditionStart::Started, "同拓扑必须被接受");
        assert!(is_running(&runtime), "接受之后必须在跑");
    }

    /// ⭐ **五种拒绝各有路径**（⭐ 逐值断言，⛔ 不只测一个）。
    #[test]
    fn each_refusal_has_its_own_path() {
        // ① audio-config：改采样率。
        {
            let main = fixture();
            let mut proposal = main.clone();
            proposal.audio_config.sample_rate = yeban_model::SampleRate::Hz44100;
            let (branch, retire) = project_branch(&main, &proposal).expect("投影");
            let mut runtime = runtime_for(&branch.main_slot);
            assert_eq!(
                start(&mut runtime, &branch, &main, &proposal, retire).mismatch(),
                Some(AuditionMismatch::AudioConfig)
            );
        }
        // ② device-chain：改一条轨道的音量（混音参数属设备链）。
        {
            let main = fixture();
            let mut proposal = main.clone();
            let track_id = *proposal.tracks.keys().next().expect("有音轨");
            if let Some(track) = proposal.tracks.get_mut(&track_id) {
                track.volume_db += 1.5;
            }
            let (branch, retire) = project_branch(&main, &proposal).expect("投影");
            let mut runtime = runtime_for(&branch.main_slot);
            assert_eq!(
                start(&mut runtime, &branch, &main, &proposal, retire).mismatch(),
                Some(AuditionMismatch::DeviceChain)
            );
        }
        // ③ automation-targets：给提案加一条自动化泳道 ⇒ 指纹变。
        {
            let main = fixture();
            let mut proposal = main.clone();
            let track_id = *proposal.tracks.keys().next().expect("有音轨");
            if let Some(track) = proposal.tracks.get_mut(&track_id) {
                let target = yeban_model::AutomationTarget::TrackVolume { track_id };
                track
                    .automation_lanes
                    .insert(target, yeban_model::AutomationLane::implicit(target));
            }
            assert_ne!(
                automation_targets_for(&main),
                automation_targets_for(&proposal),
                "前置：指纹必须真的变了"
            );
            let (branch, retire) = project_branch(&main, &proposal).expect("投影");
            let mut runtime = runtime_for(&branch.main_slot);
            assert_eq!(
                start(&mut runtime, &branch, &main, &proposal, retire).mismatch(),
                Some(AuditionMismatch::AutomationTargets)
            );
        }
        // ④ no-main-snapshot / ⑤ pdc-plan：⭐ **如实登记为"公开面不可构造"**。
        //
        // * `NoMainSnapshot`：`EngineRuntime::new` 一建起来，它的 `SnapshotReader` 就已经
        //   `begin_block` 过一次（槽里有快照）⇒ 一个**良构**的 runtime 上这一支**到不了**。
        // * `PdcPlan`：PDC 计划由**插入参数**的内部字段算出，而 `InsertParams` 的字段
        //   **全私有**（`insert.rs:412`）⇒ 宿主侧没有"改 PDC 而不改设备链"的公开路
        //   （设备链先于 PDC 判 ⇒ 改插入设备会先落 `DeviceChain`）。
        // ⇒ 两值的**短名映射**由 [`the_refusal_short_names_are_pinned_literals`] 逐值钉住
        //   （⭐ 那是能覆盖的唯一面）；⛔ 本判据**不假装**跑过这两条路径。
    }

    /// ⭐ **停止试听后逐字节回基线**（⭐ `to_bits()`，⛔ 不用 `==`）。
    #[test]
    fn stopping_the_audition_returns_to_the_baseline_byte_for_byte() {
        let main = fixture();
        let proposal = main.clone();
        let (branch, retire) = project_branch(&main, &proposal).expect("投影");
        let baseline = bits_of(&branch.main_snapshot);
        // ⚠ **防空转**：指纹必须真的有内容（否则"相等"什么都没证明）。
        assert!(
            baseline
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 3,
            "指纹不许退化（实测 {baseline:?}）"
        );
        let mut runtime = runtime_for(&branch.main_slot);
        assert_eq!(
            start(&mut runtime, &branch, &main, &proposal, retire),
            AuditionStart::Started
        );
        stop(&mut runtime);
        assert!(!is_running(&runtime), "停止之后不许还在跑");
        assert_eq!(
            bits_of(&branch.main_snapshot),
            baseline,
            "停止试听必须让主线快照**逐位**回到基线"
        );
        // 幂等：再停一次不许炸、不许改。
        stop(&mut runtime);
        assert_eq!(bits_of(&branch.main_snapshot), baseline);
    }

    /// ⭐ **宿主面（产品的那个 runtime）也能准入**：用真的
    /// [`crate::engine_host::EngineHost`] 建引擎 ⇒ 经它的判据借入点走**同一条**服务。
    ///
    /// ⚠ 没有这一条，前面四条只能证"我自建的 runtime 能准入"，⛔ 证不了宿主那一份。
    #[test]
    fn the_host_runtime_can_start_and_stop_the_audition() {
        use crate::engine_host::EngineHost;
        let main = fixture();
        let proposal = main.clone();
        let (branch, retire) = project_branch(&main, &proposal).expect("投影");
        let mut host = EngineHost::new();
        host.reload(&main, 0).expect("宿主必须能建起来");
        let runtime = host
            .runtime_for_criteria()
            .expect("reload 之后必须有 runtime");
        let started = start(runtime, &branch, &main, &proposal, retire);
        assert_eq!(started, AuditionStart::Started, "宿主面必须接受同拓扑");
        assert!(is_running(runtime), "宿主面上必须在跑");
        stop(runtime);
        assert!(!is_running(runtime), "宿主面上必须停得掉");
    }

    /// ⭐ **指纹灵敏度（"回基线"判据的牙）**：改一份工程的音量 ⇒ 指纹**必须**变。
    ///
    /// ⚠ 没有这一条，"回基线"可能因为指纹**退化**（字段漏了）而永远相等 ⇒ 判据空转。
    /// 注入实测：把 `bits_of` 打成空 ⇒ **本条与"回基线"两条都红**。
    #[test]
    fn the_snapshot_fingerprint_is_sensitive_to_a_real_change() {
        let main = fixture();
        let mut changed = main.clone();
        let track_id = *changed.tracks.keys().next().expect("有音轨");
        if let Some(track) = changed.tracks.get_mut(&track_id) {
            track.volume_db += 3.0;
        }
        let a = project_branch(&main, &main).expect("投影").0;
        let b = project_branch(&main, &changed).expect("投影").0;
        assert_ne!(
            bits_of(&a.main_snapshot),
            bits_of(&b.proposal_snapshot),
            "音量改了 3 dB ⇒ 指纹必须变（否则'回基线'判据在空转）"
        );
        // 反向：同一份工程两次投影 ⇒ 指纹必须**相同**（确定性）。
        let c = project_branch(&main, &main).expect("投影").0;
        assert_eq!(bits_of(&a.main_snapshot), bits_of(&c.main_snapshot));
    }

    /// ⭐ **`as_str()` 逐值钉住**（⭐ 五值全测，⛔ 不只测一个）。
    ///
    /// ⚠ 与引擎的短名必须**逐字**相同：这条判据就是那次"改一个字符串没人知道"的防线。
    #[test]
    fn the_refusal_short_names_are_pinned_literals() {
        let pinned = [
            (AuditionMismatch::NoMainSnapshot, "no-main-snapshot"),
            (AuditionMismatch::AudioConfig, "audio-config"),
            (AuditionMismatch::DeviceChain, "device-chain"),
            (AuditionMismatch::AutomationTargets, "automation-targets"),
            (AuditionMismatch::PdcPlan, "pdc-plan"),
        ];
        assert_eq!(
            pinned.len(),
            AuditionMismatch::ALL.len(),
            "五值必须一个不漏"
        );
        for (value, literal) in pinned {
            assert_eq!(value.as_str(), literal, "短名不许漂移");
            assert!(
                AuditionMismatch::ALL.contains(&value),
                "`ALL` 必须含每一个值"
            );
            // 映射点也必须一致（⛔ 不让映射与枚举各说各话）。
            assert_eq!(mismatch_of(literal), value);
        }
        assert_eq!(
            AuditionStart::Refused(AuditionMismatch::AudioConfig).reason(),
            Some(AUDITION_TOPOLOGY_MISMATCH_REASON)
        );
        assert_eq!(AuditionStart::Started.reason(), None);
    }
}
