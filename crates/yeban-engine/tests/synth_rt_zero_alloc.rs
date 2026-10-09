//! `line/engine-sound` 的运行期零分配判据 J5：
//! **合成路径接入之后，实时回调窗口仍然零分配、零释放** [MUST-GATE-001, ARCH-RT-001]。
//!
//! # 为什么必须单开一个 `harness = false` 目标
//!
//! `rt_zero_alloc.rs`（既有的 `[MUST-GATE-001]` 目标）用的是
//! `yeban_model::samples::filled_project()`，它的 4 个音符只覆盖前 1.5 秒 ——
//! 10,000 个量子的窗口里**绝大多数时间没有声部在跑**，于是"零分配"可能只是
//! "什么都没做"。本目标用**音符铺满整个窗口**的夹具（256 个交叠音符）复测，
//! 并**同时**断言窗口内确实有非零样本（覆盖度自检，防假绿）。
//!
//! 计数型全局分配器是**进程全局**的，libtest 自己的线程会污染计数
//! （第一版实测 `allocations=9`），因此这里与既有目标一样关掉 libtest
//! （`[[test]] harness = false`），只让主线程跑测量。
//!
//! # 场景 5 / 6（`line/engine-wiring` 追加）：每轨插入压缩器
//!
//! `crate::insert` 把 `TrackV3.devices` 的内置效果器投影成**每轨压缩器**之后，
//! 逐样本路径多了"查表 + `Compressor::process_mono`"、快照边界多了
//! `Compressor::new` / `set_params` / `set_sample_rate`（含 `exp`）。前四个场景的夹具
//! **没有**已识别的效果器设备 ⇒ 插入链整段跳过 ⇒ 对它是空转。
//! 因此追加两个场景：**10,000 量子**（逐样本路径）与 **31 次快照交换**（重新武装路径），
//! 两者都断言 `allocations == 0 && deallocations == 0`，并对"压缩器真的在压"
//! （`insert_gain_reductions > 0`）与"窗口里真的有声"做覆盖度自检。
//!
//! # 场景 7 / 8（`line/engine-wiring-2` 追加）：每轨插入**通道条**
//!
//! 场景 5 / 6 的夹具只有**动态参数** ⇒ 投影出来的通道条 `eq_enabled = false` /
//! `filter_enabled = false`（见 `crate::insert` 模块文档 §4.1）⇒ **EQ 级与滤波级
//! 在那些场景里整级不执行**，"零分配"对它们是空转。而 EQ 级是通道条里**唯一**
//! 用栈数组做块处理的一级（`yeban_dsp::channel_strip` 模块注释 §3 的
//! `[f32; 64]` × 2）⇒ 必须单独覆盖。
//!
//! 场景 7 因此用**两条轨的工程**：轨 A 只有动态参数，轨 B 是完整通道条
//! （EQ ＋ 滤波 ＋ 动态）⇒ 一次 10,000 量子窗口同时覆盖两条逐样本路径与两个
//! 武装槽位。场景 8 对同一份工程做 31 次快照交换 ⇒ 覆盖**重新武装**
//! （`ChannelStrip::set_params` / `set_sample_rate`，含 EQ 系数与 `exp`）。
//! 两个场景都做覆盖度自检：`insert_strip_frames > 0`（整链处理过）与
//! `insert_gain_reductions > 0`（动态级真的压过）。
//!
//! # 场景 9 / 10（`line/engine-reverb` 追加）：每轨插入**混响**
//!
//! 这是本文件里**唯一**一个"武装一件器件需要堆"的器件：`Reverb::set_sample_rate`
//! 会分配并释放延迟线（`crates/yeban-dsp/src/reverb.rs:73`）。本票把那条分配钉在
//! **构造期**（`EngineRuntime::new`，音频回调之外），快照边界只允许 `set_params`。
//! 场景 9 用两条轨（一条同时带通道条＋混响、一条只有混响）跑 10,000 个量子；
//! 场景 10 做 31 次同采样率重新武装，随后发布一份 **44.1 kHz** 的快照 ——
//! 引擎在那里**拒绝**重建延迟线（`insert_reverb_rate_rejects` +1）而不是在音频线程
//! 分配，最后换回 48 kHz 证明重武装仍然可行。三段的每一个窗口都断言
//! `allocations == 0 && deallocations == 0`。覆盖度自检取**整窗的精确帧数**
//! （`2 × 10,001 × 128`）而不是"大于 0"。
//!
//! # 场景 11 / 12（`line/engine-drums` 追加）：每轨**鼓机音源**
//!
//! 鼓机（`yeban_dsp::drums`）与前面所有器件有**两处结构差别**，两处都必须被覆盖：
//!
//! 1. 它是**音源**（`trigger` + `render(position, out)`），不是插入链上的逐样本变换
//!    ⇒ 它的逐样本路径在 `SynthEngine::render_track` 里，**且**每个起音要过
//!    `DrumHit` 的构造与键位映射查找（`DrumNoteMap::voice_for`）；
//! 2. 它**不持有堆**：`set_sample_rate` 只重算在响槽位的系数（对比：混响在那里
//!    **拒绝**武装）⇒ **换采样率也必须零分配**，而这条路径前面的场景从没走过
//!    （场景 10 的换采样率分支恰恰是"拒绝"）。
//!
//! 场景 11 用两条轨（一条鼓机、一条复音合成器）跑 10,000 个量子，覆盖度自检取
//! **精确的触发数**（由夹具的音符栅格算出，不是"大于 0"）；场景 12 做 31 次同采样率
//! 重新武装 ＋ 一次 **44.1 kHz**（鼓机照常武装）＋ 换回 48 kHz ＋ 换一套键位映射
//! ＋ 换回复音合成器，每一步都断言零分配零释放。
//!
//! # 场景 13 / 14（`line/engine-2` 追加，`line/engine-5` 扩到六个键）：波形选择与**两条振荡器支路**
//!
//! 器件 `yeban_dsp::polysynth` 早就是双振荡器，但引擎侧到 `line/engine-2` 才把
//! `osc1_wave` / `osc2_wave` / `osc2_level` / `osc2_detune_cents` 四个参数投影进
//! `ToneParams`。既有全部场景的夹具都**没有**振荡器参数 ⇒ `osc2_level` 恒为 0
//! ⇒ 器件的第二条支路一帧也不执行，这条武装路径与逐样本路径都是空转。
//!
//! `line/engine-5` 补上剩下的两个键（`osc1_level` / `osc1_detune_cents`），本文件的
//! 夹具**一并**带上它们：`osc1` 支路的电平与失谐因此也走**重新武装**那条路径
//! （`with_osc1` → `OscSettings::new` → `PolySynth::set_params`）。
//!
//! 场景 13 用一条轨（音符铺满窗口）＋一台**只写振荡器键**的设备
//! （`osc1_wave = 4`、`osc1` 电平 0.8 / 失谐 −5 音分、`osc2` 开在电平 0.6、
//! 失谐 11 音分）跑 10,000 个量子；
//! 场景 14 在同一份夹具上做 31 次重新武装，**每一轮换一次四个参数**（两个波形下标 ＋
//! `osc1` 的电平与失谐），并做三条覆盖度见证：① 实时侧读数 `EngineRuntime::armed_tone`
//! 逐轮等于发布值；② 两台新建的实时侧只差一个 `osc1_wave` ⇒ 指纹必须不同；
//! ③ 两台新建的实时侧只差一个 `osc1_level` ⇒ 指纹必须不同（`line/engine-5` 新增，
//! 它是"补的那条投影真的到了器件"的差分级证据）。
//! 两个场景的每一个窗口都断言 `allocations == 0 && deallocations == 0`。
//!
//! # 场景 15（`line/engine-4` 追加）：`EngineStats` 的**跨线程只读镜像**
//!
//! 前面 14 个场景都在**同一条线程**上既渲染又读统计。设备腿的真实形态不是这样：
//! `EngineRuntime` 归 cpal 回调线程所有 ⇒ 控制线程只能用
//! `yeban_engine::stats_mirror::EngineStatsMirror` 读健康读数（`yeban-app` 的
//! `EngineHost::engine_stats` 在设备腿活跃时返回 `None`，其模块文档把修法登记为
//! "一条跨线程只读统计镜像"）。这条**读写分居两条线程**的形态此前没有任何零分配
//! 判据覆盖过。
//!
//! 场景 15 因此真的开一条音频线程（2_000 个量子，每量子发布一次镜像）＋ 控制线程
//! 在**同一个窗口**里持续 `read()`：窗口内的分配断言同时覆盖**写路径与读路径**。
//! 覆盖度见证取"读者看到至少两个不同的 `quanta` 值"（证明它读到过中间值）；
//! **权威判据**取静止点上的**逐字段等号**（镜像 == 音频线程自己读的
//! `EngineRuntime::stats()`）—— 与 `snapshot_retire_churn` 的"镜像 == 权威"同口径。
//!
//! `line/engine-5` 的**缺陷修复**（窗口边界，不是放宽断言）：计数型分配器是
//! **进程级**的，而音频线程收尾时会释放**窗口之前分配**的堆内存（`output` 缓冲、
//! `mirror_runtime` 的 PDC 池 `Vec`）—— 那一类释放只计 `dealloc` 不计 `alloc`，
//! 于是被记成 `allocations=0 deallocations=1`。**接线前**实测：连跑 4 次有 **3 次**红。
//! 修法：音频线程在 `rt_done` 之后**等主线程放行**（`rt_release`），而主线程
//! 在"置 `ARMED = false` + 读走计数"之后才放行 ⇒ 线程收尾落在窗口之外。
//! 断言仍是 `allocations == 0 && deallocations == 0`；牙齿由两条注入钉住
//! （读路径里分配 ⇒ `allocations=705335`；音频线程循环里分配 ⇒ `allocations=2000`）。
//!
//! # 场景 16（`line/engine-6` 追加）：实时侧**参数目标表**
//!
//! 接线之前音频线程对 `EngineEvent::SetParam` **只计数、不改 DSP**（缺口登记在
//! `crate::param` 模块文档 §0）。接入之后这条路径多了三段实时侧代码：**事件边界**
//! （`ParamTable::accept`：整数比较 + 至多 16 项的线性搜索 + 标量赋值）、
//! **逐样本**（`ParamTable::apply`：每个样本一次乘加 + 一次乘）与**快照边界**
//! （`ParamTable::set_sample_rate`：换采样率时重算 `α`，含 `exp`）。
//!
//! 场景 16 用一个 10,000 量子的窗口覆盖前两段（目标值每 500 个量子换一次，
//! 在两个**非恒等**值之间交替 ⇒ 恒等快路径从不生效），再用一个换采样率的窗口覆盖
//! 第三段。覆盖度自检取**精确帧数**（`10 000 × 128`），不是"大于 0"。
//!
//! `line/engine-9` 把这个窗口**加宽**（不另开场景）：同一批事件里再加一条**主总线**
//! 槽位（`MASTER_GAIN_SLOT`）的事件 ⇒ 第二个逐样本入口
//! （`ParamTable::apply_master`：每帧一次低通 ＋ 两条声道各一次乘）也在窗口里，
//! 覆盖度自检对**两个**见证读数分别取精确帧数。
//!
//! ⚠ **本目标只测四元组里的两个分量**（`allocations` / `deallocations`）：
//! 它没有锁探针，也没有 I/O 边界（那是 `tests/rt_zero_alloc.rs` 的
//! `[MUST-GATE-001]` 目标）。因此本文件的全部场景**不**声称"六分量全 0"。
//!
//! # 场景 17 / 18（`line/engine-7` 追加）：每轨插入**卷积混响**
//!
//! 卷积混响是引擎里**唯一**一件"要吃一份数据（IR）而不是只吃标量"的插入器件，因此它
//! 与前面所有器件有**两处结构差别**，两处都必须被覆盖：
//!
//! 1. 它的缓冲（四条 IR 频谱 ＋ 预延迟线）在 `EngineRuntime::new` 里按**引擎常量**的
//!    IR 长度（帧数 = 采样率 ÷ 10）建满 ⇒ 快照边界上的换 IR 只走**长度不变**那条路径
//!    （原地复用缓冲，零分配）。器件自己的契约写在
//!    `crates/yeban-dsp/src/convolution.rs` 的 `set_impulse_response` 文档里
//!    （现位于第 271 行）："**长度不变**时缓冲区原地复用（此时零分配）；长度改变时按
//!    新长度重新分配"。
//! 2. 那条边界上**重设 IR** 会重算 `分区数` 次 256 点变换（0.1 秒 IR 是 38 次/核）——
//!    那是乘加，不是分配，但它是本文件里**唯一**一处"快照边界有非平凡计算量"的路径。
//!    因此场景 18 显式走一次：把 `conv_ir_decay_s` 改掉再发布 ⇒ 内容标识
//!    （`ConvolutionPlan::ir_hash`）不同 ⇒ 走 `set_impulse_response`，断言仍零分配。
//!
//! 场景 17 用两条轨（一条同时带通道条＋卷积混响、一条只有卷积混响）跑 10,000 个量子；
//! 场景 18 做 31 次**等价**重新武装（内容标识相同 ⇒ 只 `set_params`）、1 次**换 IR**、
//! 1 次 **44.1 kHz**（IR 帧数变 ⇒ 引擎拒绝重建缓冲并按设备数计数）与 1 次换回 48 kHz。
//! 覆盖度自检取**整窗的精确帧数**（`2 × 10,001 × 128`），不是"大于 0"。
//!
//! # 场景 2 的覆盖度见证（`line/engine-11` 追加）：PDC 延迟读数
//!
//! 快照边界新增了两次**纯读**（`PdcPlan::total_latency` / `PdcPlan::output_latency`
//! → `EngineStats::pdc_alignment_frames` / `engine_output_latency_frames`），位置与
//! `CompensationBank::rearm` **同一个分支** ⇒ 它落在场景 2 的 63 个测量窗口里。
//! 见证方式与场景 ⑰c 同族：63 次交换之后两个读数必须等于**最后一份已武装快照**的计划值
//! （而不是构造初值 0），且"引擎输出延迟 − 对齐基准 = 母线限制器的 33 帧"。
//! 这条见证**不新增场景**，也不改变任何窗口的分配断言。
//!
//! # 场景 19（`line/engine-12` 追加）：电平 SPSC **满队列**下的实时窗口
//!
//! 新增读数 `EngineStats::meter_dropped_frames` 的来源是 `MeterPublisher::publish`
//! 在**音频回调路径**上按"没写进环里的条数"推进的那个计数器；读它发生在 `stats()` 里，
//! 而 `stats()` 由 `process_quantum` 每次回调调用一次（收尾发布跨线程镜像）
//! ⇒ 来源与读路径**都在**实时窗口内部。场景 19 让环**持续满着**（容量 1、控制面不抽干）
//! 跑完整个窗口：两个臂（容量 4096 / 容量 1）都断言
//! `allocations == 0 && deallocations == 0`，覆盖度见证取"容量 1 的臂
//! `meter_dropped_frames > 0`"，**权威判据**取 `写入 + 丢弃 == 大容量臂测出的
//! 本应发布帧数`（等号）—— 没有那条见证，"读数恒为 0"也能让等号成立（假绿）。

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use yeban_engine::meter::meter_channel;
use yeban_engine::mixer::BUS_LIMITER_LATENCY_FRAMES;
use yeban_engine::param::{MASTER_GAIN_SLOT, TRACK_GAIN_SLOT};
use yeban_engine::ring::{EngineEvent, ParamAddress, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};

mod support;

use support::{MixSpec, NoteSpec, note_project, tuned_project, two_track_project};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, SampleRate};

/// 包住 [`System`] 的计数型分配器。
struct CountingAllocator;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法只做"计数 + 原样转发给 `System`"，不改变指针/布局语义，
// 也不持有跨调用状态（计数器是原子量）。`unsafe` 块的边界就是 `System` 的调用本身。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `layout` 合法（`GlobalAlloc` 的契约）。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr` 来自本分配器且 `layout` 与分配时一致。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            // 重新分配在实时路径上同样禁止（可能搬迁并复制），因此也计入分配。
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: 由调用方保证 `ptr`/`layout` 合法且 `new_size > 0`。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure<F: FnOnce()>(label: &str, body: F) -> (usize, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    DEALLOCATIONS.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    body();
    ARMED.store(false, Ordering::SeqCst);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    let deallocations = DEALLOCATIONS.load(Ordering::SeqCst);
    println!("[engine-sound/J5] {label}: allocations={allocations} deallocations={deallocations}");
    (allocations, deallocations)
}

/// 256 个交叠音符的夹具：`i` 从 0 起每 240 tick 起音、时值 480 tick。
///
/// 覆盖窗口 = `256 × 240 + 480 = 61920` tick = 1,548,000 样本，
/// 比判据的 10,000 个量子（1,280,000 样本）更长 ⇒ **整个测量窗口里都有声部在跑**。
fn saturated_notes() -> Vec<NoteSpec> {
    (0..256u64)
        .map(|index| NoteSpec::at(index * 240, 480, 60 + (index % 12) as u8, 100))
        .collect()
}

/// 场景 11 / 12 的**键位映射**（五个鼓件各一个音高）。
const DRUM_MAP: [u8; 5] = [36, 38, 42, 46, 39];

/// 场景 15 的音频线程量子数。
///
/// 取 2_000 而不是 10_000：这一条要的是"控制线程在窗口**中间**读到过中间值"，
/// 而读者循环的迭代速度远高于一次量子（实测每量子约 0.1 ms 量级）⇒ 2_000 已经
/// 足够产生大量不同读数，同时把"两条线程并行"的墙钟压在 1 秒以内。
const MIRROR_QUANTA: u64 = 2_000;

/// 场景 16 的量子数（与前面几个逐样本场景同量级）。
const PARAM_QUANTA: usize = 10_000;

/// 场景 16 里"每多少个量子换一次目标值"。
///
/// 500 ⇒ 一个窗口 20 次重新设目标：足够覆盖事件边界的命中路径（`accept` 的线性搜索
/// 与 `set_target`），又不至于让窗口里的事件数淹掉逐样本路径。
const PARAM_RETARGET_EVERY: usize = 500;

/// 场景 11 / 12 的鼓机音符：与 [`saturated_notes`] 同一个时间栅格（每 240 tick 起音），
/// 但音高**轮流落在 [`DRUM_MAP`] 的五个音高上** ⇒ 每一记都命中一个鼓件。
fn drum_notes() -> Vec<NoteSpec> {
    (0..256u64)
        .map(|index| NoteSpec::at(index * 240, 480, DRUM_MAP[(index % 5) as usize], 100))
        .collect()
}

/// 一台**完整键位映射**的鼓机设备（`InternalInstrument`）。
fn drum_device(extra: &[(&str, f32)], map: [f32; 5]) -> DeviceDefinition {
    let mut params = vec![
        ParameterValue {
            name: "kick_note".to_owned(),
            value: map[0],
            unit: None,
        },
        ParameterValue {
            name: "snare_note".to_owned(),
            value: map[1],
            unit: None,
        },
        ParameterValue {
            name: "closed_hat_note".to_owned(),
            value: map[2],
            unit: None,
        },
        ParameterValue {
            name: "open_hat_note".to_owned(),
            value: map[3],
            unit: None,
        },
        ParameterValue {
            name: "clap_note".to_owned(),
            value: map[4],
            unit: None,
        },
    ];
    params.extend(extra.iter().map(|(name, value)| ParameterValue {
        name: (*name).to_owned(),
        value: *value,
        unit: None,
    }));
    DeviceDefinition {
        id: EntityId::new(),
        name: "Yeban Drums".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params,
        latency_samples: 0,
    }
}

/// 场景 13 / 14 的**六个振荡器参数**（写成结构体：逐轮变化时只改其中几个字段，
/// 也免得再写一个八参数的夹具函数）。
#[derive(Clone, Copy, Debug)]
struct OscFixture {
    osc1_wave: f32,
    osc2_wave: f32,
    osc1_level: f32,
    osc1_detune_cents: f32,
    osc2_level: f32,
    osc2_detune_cents: f32,
}

/// 场景 13 / 14 的**振荡器设备**（只写振荡器键 ⇒ 滤波器旁通、音源是复音合成器）。
///
/// 刻意**不**带任何鼓机键位（`kick_note` 等）：带上就会被识别成鼓机，
/// 场景 13 / 14 要测的恰恰是复音合成器的**振荡器**路径。
fn osc_device(osc: OscFixture) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Hollow".to_owned(),
        kind: DeviceKind::InternalInstrument,
        bypassed: false,
        params: vec![
            ParameterValue {
                name: "osc1_wave".to_owned(),
                value: osc.osc1_wave,
                unit: None,
            },
            ParameterValue {
                name: "osc2_wave".to_owned(),
                value: osc.osc2_wave,
                unit: None,
            },
            ParameterValue {
                name: "osc1_level".to_owned(),
                value: osc.osc1_level,
                unit: None,
            },
            ParameterValue {
                name: "osc1_detune_cents".to_owned(),
                value: osc.osc1_detune_cents,
                unit: None,
            },
            ParameterValue {
                name: "osc2_level".to_owned(),
                value: osc.osc2_level,
                unit: None,
            },
            ParameterValue {
                name: "osc2_detune_cents".to_owned(),
                value: osc.osc2_detune_cents,
                unit: None,
            },
        ],
        latency_samples: 0,
    }
}

/// 把一台振荡器设备挂到夹具轨上（**只改夹具**，不改模型层）。
fn set_oscillators(project: &mut yeban_model::YebanProjectV1, track: EntityId, osc: OscFixture) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = vec![osc_device(osc)];
}

/// 把一段输出折进 FNV-1a 64 中间值（跨量子累积用；只做整数运算，**不分配**）。
fn osc_fold(mut hash: u64, samples: &[f32]) -> u64 {
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// 从**新建的**实时侧渲染 `quanta` 个量子，返回输出的累积指纹。
///
/// `osc1_wave` 与 `osc1_level` 是**仅有的**两个变量（其余振荡器参数固定），调用方每次
/// 只改其中一个 ⇒ 两个指纹的差就是那一个参数的效果。
///
/// ⚠ 全部构造（工程 → 快照 → `EngineRuntime`）都在测量窗口**之外** —— 这个函数只用于
/// 场景 14 的差分覆盖度见证，不参与"零分配"读数。
fn osc_render_fingerprint(osc1_wave: f32, osc1_level: f32, quanta: usize) -> u64 {
    let fixture = note_project(&saturated_notes());
    let mut project = fixture.project;
    let track = fixture.track;
    set_oscillators(
        &mut project,
        track,
        OscFixture {
            osc1_wave,
            osc2_wave: 1.0,
            osc1_level,
            osc1_detune_cents: 0.0,
            osc2_level: 0.6,
            osc2_detune_cents: 11.0,
        },
    );
    let snapshot = EngineSnapshot::from_project(&project, 1).expect("差分夹具快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(8192);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut output = vec![0.0f32; 128 * 2];
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for _ in 0..quanta {
        runtime.process_quantum(&mut output, 2);
        hash = osc_fold(hash, &output);
    }
    hash
}

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    // ---- 装配（**允许分配**：全部发生在打开设备之前）----
    let fixture = note_project(&saturated_notes());
    let snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, mut queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(8192);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut output = vec![0.0f32; 128 * 2];

    // 预热：包络模板武装、波表读取的首个 mip 级选择等一次性路径。
    // 不计入实时窗口（否则会把"初始化"记到实时路径头上 —— 那是假红）。
    runtime.process_quantum(&mut output, 2);

    // ---- 场景 1：音符铺满窗口，连续 10,000 个量子必须零分配、零释放 ----
    let mut nonzero = 0usize;
    let mut peak = 0.0f32;
    let (allocations, deallocations) = measure("10_000 quanta (saturated notes)", || {
        for _ in 0..10_000 {
            runtime.process_quantum(&mut output, 2);
            for sample in &output {
                if *sample != 0.0 {
                    nonzero += 1;
                }
                peak = peak.max(sample.abs());
            }
        }
    });
    if allocations != 0 {
        failures.push(format!(
            "合成路径在实时窗口内分配了 {allocations} 次堆内存 —— [MUST-GATE-001] 一票否决"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "合成路径在实时窗口内释放了 {deallocations} 次 —— [MUST-GATE-001] 一票否决"
        ));
    }
    // 覆盖度自检：窗口里必须**真的**在出声，否则"零分配"是空转。
    if nonzero == 0 {
        failures
            .push("10,000 个量子的窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let stats = runtime.stats();
    if stats.notes_triggered < 200 {
        failures.push(format!(
            "只触发了 {} 个音符，未达到压测规模（应 ≥ 200）",
            stats.notes_triggered
        ));
    }
    if stats.voice_steals != 0 {
        failures.push(format!(
            "夹具不应触及复音上限，但发生了 {} 次声部窃取",
            stats.voice_steals
        ));
    }

    // ---- 场景 2：高频快照交换（每次交换都要重新对齐声部池）----
    let mut switches = 0u64;
    for revision in 2..=64u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&fixture.project, revision).expect("快照");
        slot.publish(next);
        let (allocations, deallocations) = measure("snapshot swap + quantum", || {
            runtime.process_quantum(&mut output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "快照交换期间的实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "实时线程释放了 {deallocations} 次（revision={revision}）—— 旧快照必须经退役队列交回主线程"
            ));
        }
        switches += queue.drain(64) as u64;
    }
    if switches == 0 {
        failures.push("主线程侧没有从退役队列回收任何旧快照 —— 场景 2 是空转".to_owned());
    }
    // ---- 场景 2 的覆盖度见证（`line/engine-11` 追加）：PDC 延迟读数 ----
    //
    // 快照边界新增了两次纯读（`PdcPlan::total_latency` / `output_latency`），位置与
    // `CompensationBank::rearm` 同一个分支 ⇒ 上面 63 个测量窗口都真的走过它。
    // 见证方式与场景 ⑰c 同族：读数必须等于**最后一份已武装快照**的计划值。
    // 它同时证明"窗口里真的发生了重新武装"（否则读数会停在构造初值 0）。
    let latency_stats = runtime.stats();
    let fixture_plan =
        EngineSnapshot::from_project(&fixture.project, 64).expect("最后一份等价快照");
    let (want_alignment, want_output) = (
        fixture_plan.pdc().total_latency(),
        fixture_plan.pdc().output_latency(),
    );
    if latency_stats.pdc_alignment_frames != want_alignment
        || latency_stats.engine_output_latency_frames != want_output
    {
        failures.push(format!(
            "快照边界的 PDC 延迟读数没有跟上已武装快照：对齐 {} vs 计划 {want_alignment}、\
             引擎输出 {} vs 计划 {want_output}",
            latency_stats.pdc_alignment_frames, latency_stats.engine_output_latency_frames
        ));
    }
    if latency_stats.engine_output_latency_frames
        != latency_stats.pdc_alignment_frames + BUS_LIMITER_LATENCY_FRAMES
    {
        failures.push(format!(
            "生产路径的引擎输出延迟必须比对对齐基准多 {BUS_LIMITER_LATENCY_FRAMES} 帧：\
             对齐={} 输出={}",
            latency_stats.pdc_alignment_frames, latency_stats.engine_output_latency_frames
        ));
    }

    // ---- 场景 3：回到 `filled_project`（真实密度的规范样本）复测 ----
    let filled = yeban_model::samples::filled_project();
    let filled_snapshot = EngineSnapshot::from_project(&filled, 1).expect("规范样本快照");
    let filled_slot = SnapshotSlot::new(filled_snapshot);
    let (filled_retire, _filled_queue) = retire_channel(8);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut filled_runtime = EngineRuntime::new(&filled_slot, filled_retire, receiver, publisher);
    let mut filled_output = vec![0.0f32; 128 * 2];
    filled_runtime.process_quantum(&mut filled_output, 2);
    let mut filled_nonzero = 0usize;
    let (allocations, deallocations) = measure("filled_project 4_000 quanta", || {
        for _ in 0..4_000 {
            filled_runtime.process_quantum(&mut filled_output, 2);
            for sample in &filled_output {
                if *sample != 0.0 {
                    filled_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "filled_project 窗口不是零分配零释放: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if filled_nonzero == 0 {
        failures
            .push("filled_project 的 4,000 个量子没有任何非零样本 —— 覆盖度自检失败".to_owned());
    }
    let filled_stats = filled_runtime.stats();
    if filled_stats.scheduled_notes == 0 {
        failures.push("filled_project 的快照没有调度任何音符".to_owned());
    }

    // ---- 场景 4：**整条混音链**（滤波器 + 声相 + 母线限制器）仍然零分配 ----
    //
    // 为什么必须单独一个场景：前三个场景里母线是"等增益复制 + 不压限"的，
    // 声部滤波器也从未被调用。本线的三个新器件（声部低通、声相增益、前瞻限制器）
    // 都在实时路径上，**必须**单独证明它们不分配。
    //
    // 夹具设计（每一项都对应一个器件）：
    //   * 256 个交叠音符（沿用 `saturated_notes`）⇒ 整个窗口有声部在跑；
    //   * `+6 dB` 音量 ⇒ 峰值约 1.42 > 阈值 0.9 ⇒ **限制器真的在工作**；
    //   * `pan = -1.0` ⇒ 右声道应当**逐位静音**（声相定律真的在线）；
    //   * `cutoff = 2 kHz` ⇒ 声部低通真的被调用；
    //   * 另外再叠 40 个**同时起音**的长音符 ⇒ 逼出声部窃取淡出（池只有 16 声部）。
    let mut mix_notes = saturated_notes();
    mix_notes.extend((0..40u64).map(|index| NoteSpec::at(0, 96_000, 48 + (index % 12) as u8, 100)));
    let mix_fixture = tuned_project(
        &mix_notes,
        MixSpec {
            volume_db: 6.0,
            pan: -1.0,
            cutoff_hz: Some(2_000.0),
            resonance: 0.4,
        },
    );
    let mix_snapshot =
        EngineSnapshot::from_project(&mix_fixture.project, 1).expect("混音链夹具必须能编译成快照");
    let mix_slot = SnapshotSlot::new(mix_snapshot);
    let (mix_retire, _mix_queue) = retire_channel(8);
    let (_sender, mix_receiver) = event_channel(64);
    let (mix_publisher, _mix_collector) = meter_channel(8192);
    let mut mix_runtime = EngineRuntime::new(&mix_slot, mix_retire, mix_receiver, mix_publisher);
    let mut mix_output = vec![0.0f32; 128 * 2];
    mix_runtime.process_quantum(&mut mix_output, 2);

    let mut mix_nan = 0usize;
    let (allocations, deallocations) =
        measure("mix chain (filter+pan+limiter+steal) 2_000 quanta", || {
            for _ in 0..2_000 {
                mix_runtime.process_quantum(&mut mix_output, 2);
                for sample in &mix_output {
                    if sample.is_nan() {
                        mix_nan += 1;
                    }
                }
            }
        });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "混音链在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if mix_nan != 0 {
        failures.push(format!("混音链输出了 {mix_nan} 个 NaN 样本"));
    }
    let mix_stats = mix_runtime.stats();
    if mix_stats.limiter_gain_reductions == 0 {
        failures.push(
            "混音链夹具没有驱动限制器（reductions = 0）—— 这条零分配判据没有覆盖限制器".to_owned(),
        );
    }
    if mix_stats.voice_steals == 0 {
        failures.push("混音链夹具没有触发声部窃取 —— 淡出路径没有被这条零分配判据覆盖".to_owned());
    }
    // ⭐ **当前压限**（`line/engine-mix` 台账登记的"按量子发布当前 GR"）的见证：
    // 它是一个**每量子覆写**的量规 ⇒ 必须在这条零分配窗口里**非零**（限制器确实在压），
    // 且必须 `≤` 全程最大压限（量规与累计量同单位 ⇒ 口径必须自洽）。
    // 没有这两条断言，本窗口只走过了 `limiter_max_reduction` 那条路径，
    // 量规的写入路径是不是实时安全的就**没有被测到**。
    if mix_stats.limiter_current_reduction <= 0.0 {
        failures.push(
            "混音链的**当前**压限读数为 0 —— 这条零分配窗口没有覆盖量规的写入路径".to_owned(),
        );
    }
    if mix_stats.limiter_current_reduction > mix_stats.limiter_max_reduction {
        failures.push(format!(
            "当前压限 {} 大于全程最大压限 {} —— 量规与累计量的口径不一致",
            mix_stats.limiter_current_reduction, mix_stats.limiter_max_reduction
        ));
    }
    // 声相真的在线：全左 ⇒ 右声道必须**逐位**静音。
    let right_nonzero = mix_output
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|sample| **sample != 0.0)
        .count();
    if right_nonzero != 0 {
        failures.push(format!(
            "全左声相下右声道仍有 {right_nonzero} 个非零样本 —— 声相定律没有生效"
        ));
    }
    println!(
        "[engine-mix/J5] 混音链: quanta={} reductions={} steals={} 最大压限={:.4} 当前压限={:.4} 右声道非零={right_nonzero}",
        mix_stats.quanta,
        mix_stats.limiter_gain_reductions,
        mix_stats.voice_steals,
        mix_stats.limiter_max_reduction,
        mix_stats.limiter_current_reduction,
    );

    // ---- 场景 5：**每轨插入压缩器**（`crate::insert`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景：前四个场景的夹具都**没有**已识别的效果器设备
    // ⇒ `armed_compressors` 全是 `None` ⇒ 插入链**整段跳过**，"零分配"对它是空转。
    // 接线之后逐样本路径多了"查表 + `Compressor::process_mono`"，快照边界多了
    // `Compressor::new` / `set_params` / `set_sample_rate`（三者都会重算含 `exp` 的
    // 一阶低通系数）—— 这两条路径都必须证明不分配 [MUST-GATE-001, ARCH-RT-001]。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 256 个交叠音符（沿用 `saturated_notes`）⇒ 整个窗口有声部在跑；
    //   * 阈值 −30 dBFS / 比率 8 ⇒ 压缩器**每个量子真的在压**（行为覆盖，防假绿）；
    //   * 场景 6 再叠"每量子换一次快照" ⇒ 覆盖**重新武装**路径（`set_params`）。
    let mut insert_fixture = note_project(&saturated_notes());
    let insert_track = insert_fixture.track;
    {
        let entry = insert_fixture
            .project
            .tracks
            .get_mut(&insert_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Comp".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let insert_snapshot =
        EngineSnapshot::from_project(&insert_fixture.project, 1).expect("插入链夹具快照");
    assert!(
        !insert_snapshot.inserts().is_empty(),
        "插入链夹具必须真的挂上压缩器，否则本场景是空转"
    );
    let insert_slot = SnapshotSlot::new(insert_snapshot);
    let (insert_retire, mut insert_queue) = retire_channel(64);
    let (_sender, insert_receiver) = event_channel(64);
    let (insert_publisher, _insert_collector) = meter_channel(8192);
    let mut insert_runtime = EngineRuntime::new(
        &insert_slot,
        insert_retire,
        insert_receiver,
        insert_publisher,
    );
    let mut insert_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`Compressor::new`）与首个量子的一次性路径。
    insert_runtime.process_quantum(&mut insert_output, 2);
    if insert_runtime.armed_insert_slot_count() != 1 {
        failures.push(format!(
            "插入链夹具应武装 1 台压缩器，实际 {} 台",
            insert_runtime.armed_insert_slot_count()
        ));
    }

    let mut insert_nonzero = 0usize;
    let (allocations, deallocations) = measure("insert compressor 10_000 quanta", || {
        for _ in 0..10_000 {
            insert_runtime.process_quantum(&mut insert_output, 2);
            for sample in &insert_output {
                if *sample != 0.0 {
                    insert_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨插入压缩器在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if insert_nonzero == 0 {
        failures.push("插入链窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let insert_stats = insert_runtime.stats();
    if insert_stats.insert_gain_reductions == 0 {
        failures.push(
            "插入压缩器一次都没压到样本（reductions = 0）—— 这条零分配判据没有覆盖压缩器"
                .to_owned(),
        );
    }
    println!(
        "[engine-wiring/J5] 插入压缩器: quanta={} 压过样本={} 最大衰减={:.3} dB 非零样本={insert_nonzero}",
        insert_stats.quanta,
        insert_stats.insert_gain_reductions,
        insert_stats.insert_max_reduction_db,
    );

    // ---- 场景 6：快照交换时**重新武装**压缩器（`set_params` / `set_sample_rate`）----
    let mut insert_switches = 0u64;
    for revision in 2..=32u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&insert_fixture.project, revision).expect("快照");
        insert_slot.publish(next);
        let (allocations, deallocations) = measure("insert re-arm + quantum", || {
            insert_runtime.process_quantum(&mut insert_output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "重新武装插入压缩器时实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "重新武装插入压缩器时实时线程释放了 {deallocations} 次（revision={revision}）"
            ));
        }
        insert_switches += insert_queue.drain(64) as u64;
    }
    if insert_switches == 0 {
        failures.push("插入链场景没有从退役队列回收任何旧快照 —— 场景 6 是空转".to_owned());
    }

    // ---- 场景 7：**每轨插入通道条**（`crate::insert`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景：见文件头 "场景 7 / 8"。要点是场景 5 / 6 的夹具只让
    // **动态级**启用 ⇒ 通道条里那一级用**栈数组**做块处理的 EQ（以及滤波级）没有被覆盖。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 两条轨，各自的音符都铺满整个窗口（沿用 `saturated_notes`）；
    //   * 轨 A 只有动态参数（`threshold_db = −30` / `ratio = 8`）⇒ 只有动态级启用；
    //   * 轨 B 有 EQ ＋ 滤波 ＋ 动态参数 ⇒ **EQ 级与滤波级真的执行**（块路径被走到）；
    //   * 两条轨 ⇒ 两个武装槽位（覆盖"多轨各占一槽"）。
    let insert_notes = saturated_notes();
    let (mut strip_project, strip_only_track, compressor_only_track) =
        two_track_project(&insert_notes, &insert_notes);
    {
        let entry = strip_project
            .tracks
            .get_mut(&compressor_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Comp".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    {
        let entry = strip_project
            .tracks
            .get_mut(&strip_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Strip".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                // EQ 级：低架提升 ＋ 高架衰减（⇒ 三个双二阶都真的跑）。
                ParameterValue {
                    name: "eq_low_gain".to_owned(),
                    value: 6.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "eq_high_gain".to_owned(),
                    value: -12.0,
                    unit: Some("dB".to_owned()),
                },
                // 滤波级：900 Hz 低通 ⇒ 两个声道各一份四极梯形状态都真的跑。
                ParameterValue {
                    name: "cutoff_hz".to_owned(),
                    value: 900.0,
                    unit: Some("Hz".to_owned()),
                },
                // 动态级：与轨 A 同一口径 ⇒ 两条轨都会压。
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let strip_snapshot = EngineSnapshot::from_project(&strip_project, 1).expect("通道条夹具快照");
    if strip_snapshot.inserts().len() != 2 {
        failures.push(format!(
            "通道条夹具应有 2 条插入链（两条轨各一台器件），实际 {} 条",
            strip_snapshot.inserts().len()
        ));
    }
    let strip_slot = SnapshotSlot::new(strip_snapshot);
    let (strip_retire, mut strip_queue) = retire_channel(64);
    let (_sender, strip_receiver) = event_channel(64);
    let (strip_publisher, _strip_collector) = meter_channel(8192);
    let mut strip_runtime =
        EngineRuntime::new(&strip_slot, strip_retire, strip_receiver, strip_publisher);
    let mut strip_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`ChannelStrip::new` 的 EQ／滤波／压缩系数计算）与首个量子。
    strip_runtime.process_quantum(&mut strip_output, 2);
    if strip_runtime.armed_insert_slot_count() != 2 {
        failures.push(format!(
            "通道条夹具应武装 2 台器件（两条轨各一台），实际 {} 台",
            strip_runtime.armed_insert_slot_count()
        ));
    }
    if strip_runtime.armed_strip(&strip_only_track).is_none()
        || strip_runtime.armed_strip(&compressor_only_track).is_none()
    {
        failures.push("通道条夹具的两条轨都必须武装进实时侧".to_owned());
    }

    let mut strip_nonzero = 0usize;
    let (allocations, deallocations) = measure("insert channel strip 10_000 quanta", || {
        for _ in 0..10_000 {
            strip_runtime.process_quantum(&mut strip_output, 2);
            for sample in &strip_output {
                if *sample != 0.0 {
                    strip_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨插入通道条在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if strip_nonzero == 0 {
        failures.push("通道条窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let strip_stats = strip_runtime.stats();
    // 覆盖度：整链处理过帧（EQ／滤波／动态三级都在被调用），且动态级真的压过。
    if strip_stats.insert_strip_frames == 0 {
        failures.push(
            "通道条一次都没有处理过帧（insert_strip_frames = 0）—— 这条零分配判据没有覆盖通道条"
                .to_owned(),
        );
    }
    if strip_stats.insert_gain_reductions == 0 {
        failures.push(
            "通道条的动态级一次都没压到帧（reductions = 0）—— 这条零分配判据没有覆盖动态级"
                .to_owned(),
        );
    }
    println!(
        "[engine-wiring-2/J7] 插入通道条: quanta={} 整链处理帧数={} 动态级压过帧数={} 最大衰减={:.3} dB 非零样本={strip_nonzero}",
        strip_stats.quanta,
        strip_stats.insert_strip_frames,
        strip_stats.insert_gain_reductions,
        strip_stats.insert_max_reduction_db,
    );

    // ---- 场景 8：快照交换时**重新武装**通道条（`set_params` / `set_sample_rate`）----
    let mut strip_switches = 0u64;
    for revision in 2..=32u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&strip_project, revision).expect("快照");
        strip_slot.publish(next);
        let (allocations, deallocations) = measure("strip re-arm + quantum", || {
            strip_runtime.process_quantum(&mut strip_output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "重新武装插入通道条时实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "重新武装插入通道条时实时线程释放了 {deallocations} 次（revision={revision}）"
            ));
        }
        strip_switches += strip_queue.drain(64) as u64;
    }
    if strip_switches == 0 {
        failures.push("通道条场景没有从退役队列回收任何旧快照 —— 场景 8 是空转".to_owned());
    }

    // ---- 场景 9：**每轨插入混响**（`crate::insert` 的 `reverb`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景（本场景与前面所有场景的**结构差别**）：
    // `Reverb::set_sample_rate` 会分配并释放延迟线（`Vec`）—— 它是整个引擎里唯一一个
    // "武装一件器件需要堆"的器件。本票把那条分配**钉在构造期**
    // （`EngineRuntime::new`），快照边界只允许 `set_params`。这条判据就是那个说法的
    // 运行期证据：10,000 个量子（逐样本路径）与 31 次快照交换（重新武装路径）
    // 都必须零分配零释放 [MUST-GATE-001, ARCH-RT-001]。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 两条轨，各自的音符都铺满整个窗口（沿用 `saturated_notes`）；
    //   * 轨 A 同时带**通道条**与**混响**旋钮（同一台设备两件器件，覆盖"一条轨两级"）；
    //   * 轨 B 只有混响旋钮（覆盖"一台设备只出混响"）；
    //   * 湿声 1.0 + 衰减最长（`size = 1.0`）⇒ 混响**每个量子都在处理**，
    //     且尾巴足够长（覆盖度自检因此不是空转）。
    let reverb_notes = saturated_notes();
    let (mut reverb_project, reverb_both_track, reverb_only_track) =
        two_track_project(&reverb_notes, &reverb_notes);
    {
        let entry = reverb_project
            .tracks
            .get_mut(&reverb_both_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Strip+Reverb".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                // 通道条：EQ ＋ 滤波 ＋ 动态（复用场景 7 的口径）。
                ParameterValue {
                    name: "eq_low_gain".to_owned(),
                    value: 6.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "cutoff_hz".to_owned(),
                    value: 6_000.0,
                    unit: Some("Hz".to_owned()),
                },
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
                // 混响：全湿、最长衰减、带预延迟。
                ParameterValue {
                    name: "reverb_size".to_owned(),
                    value: 1.0,
                    unit: None,
                },
                ParameterValue {
                    name: "reverb_wet".to_owned(),
                    value: 1.0,
                    unit: None,
                },
                ParameterValue {
                    name: "reverb_predelay".to_owned(),
                    value: 0.02,
                    unit: Some("s".to_owned()),
                },
            ],
            latency_samples: 0,
        }];
    }
    {
        let entry = reverb_project
            .tracks
            .get_mut(&reverb_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Reverb".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "reverb_size".to_owned(),
                    value: 0.9,
                    unit: None,
                },
                ParameterValue {
                    name: "reverb_wet".to_owned(),
                    value: 0.8,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let reverb_snapshot = EngineSnapshot::from_project(&reverb_project, 1).expect("混响夹具快照");
    if reverb_snapshot.inserts().len() != 2 {
        failures.push(format!(
            "混响夹具应有 2 条插入链（两条轨各一台器件），实际 {} 条",
            reverb_snapshot.inserts().len()
        ));
    }
    let reverb_slot = SnapshotSlot::new(reverb_snapshot);
    let (reverb_retire, mut reverb_queue) = retire_channel(64);
    let (_sender, reverb_receiver) = event_channel(64);
    let (reverb_publisher, _reverb_collector) = meter_channel(8192);
    let mut reverb_runtime = EngineRuntime::new(
        &reverb_slot,
        reverb_retire,
        reverb_receiver,
        reverb_publisher,
    );
    let mut reverb_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`Reverb::set_params`）与首个量子。延迟线本身在 `new` 里就分配好了。
    reverb_runtime.process_quantum(&mut reverb_output, 2);
    if reverb_runtime.armed_reverb_slot_count() != 2 {
        failures.push(format!(
            "混响夹具应武装 2 台混响，实际 {} 台",
            reverb_runtime.armed_reverb_slot_count()
        ));
    }
    if reverb_runtime.armed_reverb_sample_rate() != 48_000 {
        failures.push(format!(
            "混响延迟线池应按初始快照的 48 kHz 武装，实际 {} Hz",
            reverb_runtime.armed_reverb_sample_rate()
        ));
    }
    if reverb_runtime.armed_reverb(&reverb_both_track).is_none()
        || reverb_runtime.armed_reverb(&reverb_only_track).is_none()
    {
        failures.push("混响夹具的两条轨都必须武装进实时侧".to_owned());
    }

    let mut reverb_nonzero = 0usize;
    let (allocations, deallocations) = measure("insert reverb 10_000 quanta", || {
        for _ in 0..10_000 {
            reverb_runtime.process_quantum(&mut reverb_output, 2);
            for sample in &reverb_output {
                if *sample != 0.0 {
                    reverb_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨插入混响在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if reverb_nonzero == 0 {
        failures.push("混响窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let reverb_stats = reverb_runtime.stats();
    // 覆盖度：混响真的处理过帧。门槛取**场景 9 的实际规模**而不是 0：
    // `> 0` 对"只跑了一个量子"也成立，那条读数无法区分"整窗都在处理"与"只处理了一次"。
    let expected_reverb_frames = 2 * (10_000 + 1) * 128;
    if reverb_stats.insert_reverb_frames != expected_reverb_frames {
        failures.push(format!(
            "混响整窗处理帧数={}（期望 {} = 2 条轨 × 10,001 个量子 × 128 帧）—— \
             这条零分配判据没有覆盖整条逐样本路径",
            reverb_stats.insert_reverb_frames, expected_reverb_frames
        ));
    }
    if reverb_stats.insert_reverb_rate_rejects != 0 {
        failures.push(format!(
            "采样率没有变，混响不该被拒绝武装，实际拒绝 {} 次",
            reverb_stats.insert_reverb_rate_rejects
        ));
    }
    println!(
        "[engine-wiring-3/J9] 插入混响: quanta={} 混响处理帧数={} 非零样本={reverb_nonzero} 武装采样率={}",
        reverb_stats.quanta,
        reverb_stats.insert_reverb_frames,
        reverb_runtime.armed_reverb_sample_rate(),
    );

    // ---- 场景 10：重新武装混响 + **换采样率时拒绝重建延迟线**（都零分配）----
    //
    // 两半各自对应一条真实路径：
    //   * 前 31 次交换 = **同一个采样率** ⇒ 走 `set_params`（以及槽位重占时的
    //     `set_sample_rate` 同值复位，它走 `fill(0.0)` 分支、**零分配**）；
    //   * 最后一次 = **换采样率**（44.1 kHz）⇒ 引擎**拒绝**重建延迟线
    //     （`Reverb::set_sample_rate` 会 `Vec` 重分配 + 释放），整段不武装并计数。
    //     一个"照着场景 7/8 写"的实现会在这里调用 `set_sample_rate` ⇒ 本场景变红。
    let mut reverb_switches = 0u64;
    for revision in 2..=32u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&reverb_project, revision).expect("快照");
        reverb_slot.publish(next);
        let (allocations, deallocations) = measure("reverb re-arm + quantum", || {
            reverb_runtime.process_quantum(&mut reverb_output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "重新武装插入混响时实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "重新武装插入混响时实时线程释放了 {deallocations} 次（revision={revision}）"
            ));
        }
        reverb_switches += reverb_queue.drain(64) as u64;
    }
    if reverb_switches == 0 {
        failures.push("混响场景没有从退役队列回收任何旧快照 —— 场景 10 是空转".to_owned());
    }
    if reverb_runtime.armed_reverb_slot_count() != 2 {
        failures.push("同采样率重新武装之后两条轨的混响都必须仍被武装".to_owned());
    }

    // 换采样率：44.1 kHz 的快照**不**武装混响（延迟线不为新采样率重建）。
    let mut shifted = reverb_project.clone();
    shifted.audio_config.sample_rate = SampleRate::Hz44100;
    let shifted_snapshot = EngineSnapshot::from_project(&shifted, 33).expect("换采样率快照");
    reverb_slot.publish(shifted_snapshot);
    let before_rejects = reverb_runtime.stats().insert_reverb_rate_rejects;
    let (allocations, deallocations) = measure("reverb rate-mismatch re-arm + quantum", || {
        reverb_runtime.process_quantum(&mut reverb_output, 2);
    });
    if allocations != 0 {
        failures.push(format!(
            "换采样率时实时路径分配了 {allocations} 次 —— 延迟线绝不能在音频线程重建"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "换采样率时实时线程释放了 {deallocations} 次 —— 延迟线绝不能在音频线程重建"
        ));
    }
    let after = reverb_runtime.stats();
    if after.insert_reverb_rate_rejects != before_rejects + 1 {
        failures.push(format!(
            "换采样率应恰好累加 1 次拒绝，实测 {} -> {}",
            before_rejects, after.insert_reverb_rate_rejects
        ));
    }
    if reverb_runtime.armed_reverb_slot_count() != 0 {
        failures.push(format!(
            "换采样率之后混响必须整段不武装，实际仍武装 {} 台",
            reverb_runtime.armed_reverb_slot_count()
        ));
    }
    // 换回 48 kHz：必须能重新武装（守卫是"拒绝这一份"，不是"永久停用"）。
    let back_snapshot = EngineSnapshot::from_project(&reverb_project, 34).expect("换回快照");
    reverb_slot.publish(back_snapshot);
    let (allocations, deallocations) = measure("reverb rate-restore re-arm + quantum", || {
        reverb_runtime.process_quantum(&mut reverb_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换回 48 kHz 重新武装混响时分配/释放: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if reverb_runtime.armed_reverb_slot_count() != 2 {
        failures.push("换回 48 kHz 之后两条轨的混响必须重新武装".to_owned());
    }
    println!(
        "[engine-wiring-3/J10] 混响重新武装: 交换={} 次；换采样率后拒绝累计={} 次；武装采样率={} Hz",
        reverb_switches,
        reverb_runtime.stats().insert_reverb_rate_rejects,
        reverb_runtime.armed_reverb_sample_rate(),
    );

    // ---- 场景 11：**每轨鼓机音源**（`crate::drums`）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景（见文件头 "场景 11 / 12"）：鼓机是**音源**，它的逐样本
    // 路径在 `SynthEngine::render_track` 里（触发 + 渲染），而不是插入链上的变换；
    // 且它的 `set_sample_rate` **不分配**（与混响相反）。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 两条轨：一根鼓机轨（音符的音高轮流命中五个鼓件）＋ 一条复音合成器轨
    //     （`cutoff_hz` ⇒ 同一窗口里另一条音源仍在跑）；
    //   * 鼓机的尾巴取器件默认值（底鼓 0.40 s）⇒ 整窗都有槽位在响。
    let (mut drum_project, drum_track, drum_synth_track) =
        two_track_project(&drum_notes(), &saturated_notes());
    {
        let entry = drum_project
            .tracks
            .get_mut(&drum_track)
            .expect("夹具里必须有那条鼓机轨");
        entry.devices = vec![drum_device(
            &[("kick_decay_s", 0.40), ("master_level", 0.9)],
            [36.0, 38.0, 42.0, 46.0, 39.0],
        )];
    }
    {
        let entry = drum_project
            .tracks
            .get_mut(&drum_synth_track)
            .expect("夹具里必须有那条复音轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Hollow".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "cutoff_hz".to_owned(),
                    value: 800.0,
                    unit: Some("Hz".to_owned()),
                },
                ParameterValue {
                    name: "resonance".to_owned(),
                    value: 0.3,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let drum_snapshot = EngineSnapshot::from_project(&drum_project, 1).expect("鼓机夹具快照");
    if drum_snapshot.drums().len() != 1 {
        failures.push(format!(
            "鼓机夹具应只有 1 条鼓机轨，实际 {} 条（只写了 cutoff_hz 的那条不算）",
            drum_snapshot.drums().len()
        ));
    }
    let drum_slot = SnapshotSlot::new(drum_snapshot);
    let (drum_retire, mut drum_queue) = retire_channel(64);
    let (_drum_sender, drum_receiver) = event_channel(64);
    let (drum_publisher, _drum_collector) = meter_channel(8192);
    let mut drum_runtime =
        EngineRuntime::new(&drum_slot, drum_retire, drum_receiver, drum_publisher);
    let mut drum_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`DrumMachine::set_params`）与首个量子。
    drum_runtime.process_quantum(&mut drum_output, 2);
    if drum_runtime.armed_drum_slot_count() != 1 {
        failures.push(format!(
            "鼓机夹具应武装 1 台鼓机，实际 {} 台",
            drum_runtime.armed_drum_slot_count()
        ));
    }
    if drum_runtime.armed_drums(&drum_track).is_none() {
        failures.push("鼓机轨必须武装进实时侧（复音轨不得武装）".to_owned());
    }

    // 覆盖度自检的**精确**期望：落在整窗里的起音数（由夹具栅格算出，不是"大于 0"）。
    // 窗口 = 预热 1 个量子 ＋ 下面 10,000 个量子 = 10,001 × 128 帧。
    let window_end = (10_001u64) * 128;
    let expected_drum_hits = (0..256u64)
        .filter(|index| index * 240 * 25 < window_end)
        .count();
    let mut drum_nonzero = 0usize;
    let (allocations, deallocations) = measure("drum instrument 10_000 quanta", || {
        for _ in 0..10_000 {
            drum_runtime.process_quantum(&mut drum_output, 2);
            for sample in &drum_output {
                if *sample != 0.0 {
                    drum_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨鼓机音源在实时窗口内分配/释放了内存: allocations={allocations} deallocations={deallocations}"
        ));
    }
    if drum_nonzero == 0 {
        failures.push("鼓机窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let drum_stats = drum_runtime.stats();
    if drum_stats.drum_hits != expected_drum_hits as u64 {
        failures.push(format!(
            "鼓击数={}（期望 {expected_drum_hits} = 窗口内的起音数）—— 这条零分配判据\
             没有覆盖整条触发路径",
            drum_stats.drum_hits
        ));
    }
    if drum_stats.insert_strip_frames != 0 {
        failures.push("鼓机是音源、不是插入器件：`insert_strip_frames` 必须为 0".to_owned());
    }
    println!(
        "[engine-drums/J11] 鼓机音源: quanta={} 鼓击={} 非零样本={drum_nonzero} 武装槽位={}",
        drum_stats.quanta,
        drum_stats.drum_hits,
        drum_runtime.armed_drum_slot_count(),
    );

    // ---- 场景 12：重新武装鼓机（同采样率 / 换采样率 / 换映射 / 换回复音合成器）----
    //
    // 四段各自对应一条真实路径：
    //   * 前 31 次交换 = **同一个采样率、同一份映射** ⇒ 一个字段都不写（状态全保留）；
    //   * 第 32 次 = **换采样率**（44.1 kHz）⇒ `DrumMachine::set_sample_rate` 重算在响
    //     槽位的系数。⚠ 这与混响**相反**（混响在那里拒绝武装）：鼓机没有延迟线，
    //     所以它必须能跟着采样率走，而且**不许分配**；
    //   * 第 33 次 = **换一套键位映射**（同一轨）⇒ 只 `set_params`（状态保留）；
    //   * 第 34 次 = **换回复音合成器** ⇒ 鼓机 `reset`、武装槽位归 0。
    let mut drum_switches = 0u64;
    for revision in 2..=32u64 {
        let next = EngineSnapshot::from_project(&drum_project, revision).expect("快照");
        drum_slot.publish(next);
        let (allocations, deallocations) = measure("drum re-arm + quantum", || {
            drum_runtime.process_quantum(&mut drum_output, 2);
        });
        if allocations != 0 || deallocations != 0 {
            failures.push(format!(
                "重新武装鼓机时实时路径分配/释放: allocations={allocations} deallocations={deallocations}（revision={revision}）"
            ));
        }
        drum_switches += drum_queue.drain(64) as u64;
    }
    if drum_switches == 0 {
        failures.push("鼓机场景没有从退役队列回收任何旧快照 —— 场景 12 是空转".to_owned());
    }
    if drum_runtime.armed_drum_slot_count() != 1 {
        failures.push("同采样率重新武装之后鼓机必须仍被武装".to_owned());
    }

    // 换采样率：鼓机照常武装（它没有需要重建的缓冲），且零分配。
    let mut shifted_drums = drum_project.clone();
    shifted_drums.audio_config.sample_rate = SampleRate::Hz44100;
    let shifted_drum_snapshot =
        EngineSnapshot::from_project(&shifted_drums, 33).expect("换采样率快照");
    drum_slot.publish(shifted_drum_snapshot);
    let (allocations, deallocations) = measure("drum rate-change re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换采样率重新武装鼓机时分配/释放: allocations={allocations} deallocations={deallocations}\
             —— `DrumMachine::set_sample_rate` 只重算系数，不许碰堆"
        ));
    }
    let armed_after_rate_change = drum_runtime.armed_drum_slot_count();
    if armed_after_rate_change != 1 {
        failures.push(format!(
            "换采样率之后鼓机必须仍被武装（对比：混响在同样的分支里拒绝），实际 {armed_after_rate_change} 台"
        ));
    }

    // 换回 48 kHz。
    let back_drum_snapshot = EngineSnapshot::from_project(&drum_project, 34).expect("换回快照");
    drum_slot.publish(back_drum_snapshot);
    let (allocations, deallocations) = measure("drum rate-restore re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换回 48 kHz 重新武装鼓机时分配/释放: allocations={allocations} deallocations={deallocations}"
        ));
    }

    // 换一套键位映射（同一轨）：整体上移一个八度。
    let mut remapped = drum_project.clone();
    {
        let entry = remapped
            .tracks
            .get_mut(&drum_track)
            .expect("夹具里必须有那条鼓机轨");
        entry.devices = vec![drum_device(&[], [48.0, 50.0, 54.0, 58.0, 51.0])];
    }
    let remapped_snapshot = EngineSnapshot::from_project(&remapped, 35).expect("换映射快照");
    drum_slot.publish(remapped_snapshot);
    let (allocations, deallocations) = measure("drum remap re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换键位映射重新武装鼓机时分配/释放: allocations={allocations} deallocations={deallocations}"
        ));
    }
    let remapped_params = drum_runtime.armed_drums(&drum_track);
    if remapped_params.map(|params| params.notes().kick()) != Some(48) {
        failures.push(format!(
            "换映射之后实时侧读到的底鼓音高不是 48：{remapped_params:?}"
        ));
    }

    // 换回复音合成器：鼓机必须被 `reset`、武装槽位归 0，且零分配。
    let mut synth_again = drum_project.clone();
    {
        let entry = synth_again
            .tracks
            .get_mut(&drum_track)
            .expect("夹具里必须有那条鼓机轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Hollow".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: vec![ParameterValue {
                name: "cutoff_hz".to_owned(),
                value: 800.0,
                unit: Some("Hz".to_owned()),
            }],
            latency_samples: 0,
        }];
    }
    let synth_again_snapshot = EngineSnapshot::from_project(&synth_again, 36).expect("换回快照");
    drum_slot.publish(synth_again_snapshot);
    let (allocations, deallocations) = measure("drum -> poly synth re-arm + quantum", || {
        drum_runtime.process_quantum(&mut drum_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "从鼓机换回复音合成器时分配/释放: allocations={allocations} deallocations={deallocations}\
             —— `DrumMachine::reset` 是赋值，不许释放"
        ));
    }
    let armed_after_swap = drum_runtime.armed_drum_slot_count();
    if armed_after_swap != 0 {
        failures.push(format!(
            "换回复音合成器之后鼓机必须不武装，实际仍武装 {armed_after_swap} 台"
        ));
    }
    if drum_runtime.armed_drums(&drum_track).is_some() {
        failures.push("换回复音合成器之后该轨的鼓机读数必须变 None".to_owned());
    }
    println!(
        "[engine-drums/J12] 鼓机重新武装: 同采样率交换={} 次；换采样率后武装槽位={}；\
         换映射后底鼓音高={:?}；换回复音后武装槽位={}",
        drum_switches,
        armed_after_rate_change,
        remapped_params.map(|params| params.notes().kick()),
        armed_after_swap,
    );

    // ---- 场景 13：**波形选择 + 两条振荡器支路**在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景：器件（`yeban_dsp::polysynth`）早就是双振荡器，但引擎侧
    // 到 `line/engine-2` 才把 `osc1_wave` / `osc2_wave` / `osc2_level` /
    // `osc2_detune_cents` 四个参数投影进 `ToneParams`，`line/engine-5` 再补
    // `osc1_level` / `osc1_detune_cents` ⇒ 这条**武装路径**
    // （`poly_synth_params` → `set_params`）与"两条支路真的在逐样本路径上跑"
    // 都没有被既有场景走过（既有夹具的 `osc2_level` 恒为 0 ⇒ 第二条支路一帧也不执行）。
    //
    // 夹具：音符铺满窗口（同场景 1 的栅格）＋一条轨挂**只写振荡器键**的设备
    // （⇒ 滤波器旁通、音源是复音合成器、`osc1_wave = 4`（glass）、`osc1` 电平 0.8 /
    // 失谐 −5 音分、`osc2` 开在电平 0.6、失谐 11 音分、波表 1（organ））。
    let osc_fixture = note_project(&saturated_notes());
    let mut osc_project = osc_fixture.project;
    let osc_track = osc_fixture.track;
    set_oscillators(
        &mut osc_project,
        osc_track,
        OscFixture {
            osc1_wave: 4.0,
            osc2_wave: 1.0,
            osc1_level: 0.8,
            osc1_detune_cents: -5.0,
            osc2_level: 0.6,
            osc2_detune_cents: 11.0,
        },
    );
    let osc_snapshot = EngineSnapshot::from_project(&osc_project, 1).expect("振荡器夹具快照");
    match osc_snapshot.tone(&osc_track) {
        Some(tone)
            if tone.osc1_wave() == 4
                && tone.osc2_wave() == 1
                && tone.osc1_level() == 0.8
                && tone.osc1_detune_cents() == -5.0
                && tone.osc2_level() == 0.6 =>
        {
            if !tone.is_bypass() {
                failures.push("只有振荡器键的夹具必须是滤波器旁通".to_owned());
            }
        }
        other => failures.push(format!("夹具快照的振荡器投影不对：{other:?}")),
    }
    let osc_slot = SnapshotSlot::new(osc_snapshot);
    let (osc_retire, mut osc_queue) = retire_channel(64);
    let (_osc_sender, osc_receiver) = event_channel(64);
    let (osc_publisher, _osc_collector) = meter_channel(8192);
    let mut osc_runtime = EngineRuntime::new(&osc_slot, osc_retire, osc_receiver, osc_publisher);
    let mut osc_output = vec![0.0f32; 128 * 2];
    // 预热：首次武装（`PolySynth::set_params`，含 `tan`）与首个量子。
    osc_runtime.process_quantum(&mut osc_output, 2);
    if osc_runtime
        .armed_tone(&osc_track)
        .map(|tone| tone.osc1_wave())
        != Some(4)
    {
        failures.push("预热之后实时侧必须已经武装 glass（下标 4）".to_owned());
    }
    if osc_runtime
        .armed_tone(&osc_track)
        .map(|tone| tone.osc1_level())
        != Some(0.8)
    {
        failures.push("预热之后实时侧必须已经武装 `osc1_level = 0.8`".to_owned());
    }

    let mut osc_nonzero = 0usize;
    let (allocations, deallocations) =
        measure("oscillator (waveform + 2nd osc) 10_000 quanta", || {
            for _ in 0..10_000 {
                osc_runtime.process_quantum(&mut osc_output, 2);
                for sample in &osc_output {
                    if *sample != 0.0 {
                        osc_nonzero += 1;
                    }
                }
            }
        });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "波形选择 / 第二条振荡器在实时窗口内分配/释放了内存: \
             allocations={allocations} deallocations={deallocations}"
        ));
    }
    if osc_nonzero == 0 {
        failures
            .push("波形/第二条支路窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let osc_stats = osc_runtime.stats();
    if osc_stats.notes_triggered < 200 {
        failures.push(format!(
            "振荡器窗口只触发了 {} 个音符，未达到压测规模（应 ≥ 200）",
            osc_stats.notes_triggered
        ));
    }

    // ---- 场景 14：**重新武装振荡器参数**（两个波形下标 / 两条支路的电平与失谐）----
    //
    // 每一轮都换一次 `osc1_wave` / `osc2_wave` **以及** `osc1_level` / `osc1_detune_cents`：
    // 覆盖 `EngineSnapshot::from_project` 的投影、`begin_snapshot` 的 `tone != slot.tone`
    // 分支与器件的 `set_params`。覆盖度见证有两条：
    //   ① 每一轮的实时侧读数（`armed_tone`）等于发布的波形下标与 `osc1` 电平/失谐；
    //   ② 循环之后的**差分**检查：两台新建的实时侧只差一个参数，输出指纹必须不同
    //      （见下面的注释：从**同一台** runtime 的相邻量子比指纹是**没有牙的**）。
    let mut osc_switches = 0u64;
    let mut osc_rearms_matching = 0u64;
    for revision in 2..=32u64 {
        let osc1_wave = (revision % 5) as f32;
        let osc2_wave = ((revision + 2) % 5) as f32;
        // `osc1` 的电平/失谐逐轮变化（避开两端的钳制边界，保证读数可逐位比较）。
        let osc1_level = 0.4 + 0.05 * ((revision % 4) as f32);
        let osc1_detune_cents = -9.0 + 3.0 * ((revision % 3) as f32);
        set_oscillators(
            &mut osc_project,
            osc_track,
            OscFixture {
                osc1_wave,
                osc2_wave,
                osc1_level,
                osc1_detune_cents,
                osc2_level: 0.6,
                osc2_detune_cents: 11.0,
            },
        );
        let next = EngineSnapshot::from_project(&osc_project, revision).expect("快照");
        osc_slot.publish(next);
        let (allocations, deallocations) = measure("oscillator re-arm + quantum", || {
            osc_runtime.process_quantum(&mut osc_output, 2);
        });
        if allocations != 0 || deallocations != 0 {
            failures.push(format!(
                "重新武装振荡器参数时实时路径分配/释放: \
                 allocations={allocations} deallocations={deallocations}（revision={revision}）"
            ));
        }
        match osc_runtime.armed_tone(&osc_track) {
            Some(tone)
                if u32::from(tone.osc1_wave()) == revision as u32 % 5
                    && tone.osc1_level() == osc1_level
                    && tone.osc1_detune_cents() == osc1_detune_cents =>
            {
                osc_rearms_matching += 1;
            }
            other => failures.push(format!(
                "重新武装后波形下标 / `osc1` 电平失谐不是 ({}, {osc1_level}, {osc1_detune_cents})：\
                 {other:?}（revision={revision}）",
                revision % 5
            )),
        }
        osc_switches += osc_queue.drain(64) as u64;
    }
    if osc_switches == 0 {
        failures.push("振荡器场景没有从退役队列回收任何旧快照 —— 场景 14 是空转".to_owned());
    }
    if osc_rearms_matching != 31 {
        failures.push(format!(
            "31 次重新武装里只有 {osc_rearms_matching} 次的实时侧波形下标 / `osc1` 电平失谐与发布值一致"
        ));
    }

    // ⚠ **差分**覆盖度见证（这一条才有牙）。
    //
    // 第一版写的是"同一台 runtime 的相邻量子指纹必须变化"，`line/engine-2` 的注入 I1
    // （`poly_synth_params` 丢掉 `.with_oscillators(..)`）实测**不红** —— 因为相邻量子
    // 之间声部状态本来就在推进（相位、包络、失谐拍频），指纹变化与波形无关。
    // 改成"两台**新建**的实时侧只差一个参数"：声部状态从零开始、逐量子同步推进，
    // 唯一的差别就是那个参数 ⇒ 指纹不同才是"参数真的到了器件"。
    // I1 下第一条变红（`line/engine-2` 实测）；`line/engine-5` 的注入 I5
    // （`poly_synth_params` 把 `osc1` 的电平/失谐写死回 `1.0` / `0.0`）下第二条变红（本票实测）。
    let low = osc_render_fingerprint(0.0, 0.8, 8);
    let high = osc_render_fingerprint(4.0, 0.8, 8);
    if low == high {
        failures.push(
            "只差一个 `osc1_wave` 的两台实时侧给出了相同的输出指纹 —— 波形参数没有真的到器件"
                .to_owned(),
        );
    }
    let quiet = osc_render_fingerprint(1.0, 0.4, 8);
    let loud = osc_render_fingerprint(1.0, 1.0, 8);
    if quiet == loud {
        failures.push(
            "只差一个 `osc1_level` 的两台实时侧给出了相同的输出指纹 —— \
             `osc1` 电平没有真的到器件"
                .to_owned(),
        );
    }
    println!(
        "[engine-osc/J13] 波形选择 + 两条振荡器支路: quanta={} 非零样本={osc_nonzero} \
         重新武装={osc_rearms_matching}/31 退役回收={osc_switches} 次；\
         差分指纹 wave0={low:#018x} wave4={high:#018x} level0.4={quiet:#018x} level1.0={loud:#018x}",
        osc_stats.quanta,
    );

    // ---- 场景 16：实时侧**参数目标表**（`crate::param`）----
    //
    // 为什么必须单独一个场景：接线之前音频线程对 `EngineEvent::SetParam` **只计数、
    // 不改 DSP**（缺口登记在 `crate::param` 模块文档 §0）。接入之后这条路径多了三段
    // 实时侧代码，本场景把它们**全部**放进同一个零分配窗口：
    //
    //   * **事件边界**（`ParamTable::accept`）：整数比较 + 至多 16 项的线性搜索 +
    //     标量赋值（窗口里每 `PARAM_RETARGET_EVERY` 个量子发一条新目标）；
    //   * **逐样本**（`ParamTable::apply`）：每个样本一次乘加（平滑器）与一次乘；
    //   * **快照边界**（`ParamTable::set_sample_rate`）：换采样率时重算 `α`（含 `exp`，
    //     单独一个窗口覆盖，见下面的 44.1 kHz 那一段）。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 256 个交叠音符（沿用 `saturated_notes`）⇒ 窗口里真的有信号可乘；
    //   * 目标值在两个**非恒等**值之间交替（`0.5` / `0.25`）⇒ `apply` 的恒等快路径
    //     从不生效 ⇒ 覆盖度自检可以取**精确帧数**（`10_000 × 128`）而不是"大于 0"。
    //
    // `line/engine-9` 追加（**同一个窗口**，不再另开一个）：每 `PARAM_RETARGET_EVERY`
    // 个量子**同时**发一条**主总线**槽位（`MASTER_GAIN_SLOT`）的事件 ⇒ 第二个逐样本
    // 入口（`ParamTable::apply_master`：每帧一次单极点低通 ＋ 两条声道各一次乘）
    // 也落在零分配窗口里。两条事件打包成**一次** `publish`（批量无锁通道的用法）。
    let param_fixture = note_project(&saturated_notes());
    let param_track = param_fixture.track;
    let param_master = param_fixture.master;
    let param_snapshot =
        EngineSnapshot::from_project(&param_fixture.project, 1).expect("参数表夹具快照");
    let param_slot = SnapshotSlot::new(param_snapshot);
    let (param_retire, _param_queue) = retire_channel(8);
    let (mut param_sender, param_receiver) = event_channel(64);
    let (param_publisher, _param_collector) = meter_channel(8192);
    let mut param_runtime =
        EngineRuntime::new(&param_slot, param_retire, param_receiver, param_publisher);
    // 预热（窗口外）：首量子的波表/包络一次性路径。
    let mut param_output = vec![0.0f32; 128 * 2];
    param_runtime.process_quantum(&mut param_output, 2);
    let param_base_frames = param_runtime.stats().param_gain_frames;

    let (allocations, deallocations) = measure("param automation 10_000 quanta", || {
        for quantum in 0..PARAM_QUANTA {
            if quantum % PARAM_RETARGET_EVERY == 0 {
                let value = if (quantum / PARAM_RETARGET_EVERY).is_multiple_of(2) {
                    0.5
                } else {
                    0.25
                };
                let accepted = param_sender.publish(&[
                    EngineEvent::SetParam {
                        target: ParamAddress::new(param_track, TRACK_GAIN_SLOT),
                        value,
                    },
                    EngineEvent::SetParam {
                        target: ParamAddress::new(param_master, MASTER_GAIN_SLOT),
                        value,
                    },
                ]);
                assert_eq!(accepted, 2, "两条参数事件必须真的进队列");
            }
            param_output.fill(0.0);
            param_runtime.process_quantum(&mut param_output, 2);
        }
    });
    let param_stats = param_runtime.stats();
    let param_frames = param_stats
        .param_gain_frames
        .saturating_sub(param_base_frames);
    let param_master_frames = param_stats.param_master_gain_frames;
    let param_slots = param_runtime.armed_param_slot_count();
    println!(
        "[engine-param/J16] 实时侧参数目标表: allocations={allocations} \
         deallocations={deallocations} 乘过的帧数={param_frames} \
         主总线乘过的帧数={param_master_frames} 槽位={param_slots} \
         非法值={} 未映射={} 容量不足={}",
        param_stats.param_gain_rejects,
        param_stats.param_unmapped_events,
        param_stats.param_capacity_drops,
    );
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "实时侧参数目标表在窗口内分配/释放了内存: allocations={allocations} \
             deallocations={deallocations}（事件边界 + 逐样本乘法都在窗口里）"
        ));
    }
    if param_slots != 1 {
        failures.push(format!(
            "参数目标表在窗口里建了 {param_slots} 个槽位，期望 1 个 —— 场景 16 是空转"
        ));
    }
    if param_frames != (PARAM_QUANTA * 128) as u64 {
        failures.push(format!(
            "参数目标表只乘过 {param_frames} 帧，期望 {}（{PARAM_QUANTA} 量子 × 128 帧；\
             非恒等目标 ⇒ 每一个量子都必须走乘法）",
            PARAM_QUANTA * 128
        ));
    }
    if param_master_frames != (PARAM_QUANTA * 128) as u64 {
        failures.push(format!(
            "主总线参数槽位只乘过 {param_master_frames} 帧，期望 {}（{PARAM_QUANTA} 量子 × 128 帧；\
             非恒等目标 ⇒ 每一个量子的两条声道都必须被乘）",
            PARAM_QUANTA * 128
        ));
    }
    if param_runtime.armed_master_param_target().is_none() {
        failures.push(
            "主总线参数槽位没有被武装 —— 场景 16 的第二个入口（`apply_master`）是空转".to_owned(),
        );
    }
    if param_stats.param_gain_rejects != 0
        || param_stats.param_unmapped_events != 0
        || param_stats.param_capacity_drops != 0
    {
        failures.push(format!(
            "参数目标表在窗口里记了非零的拒绝读数：非法值={} 未映射={} 容量不足={}",
            param_stats.param_gain_rejects,
            param_stats.param_unmapped_events,
            param_stats.param_capacity_drops
        ));
    }

    // 换采样率：快照边界的 `ParamTable::set_sample_rate`（α 重算，含 `exp`）也必须在
    // 窗口内零分配。快照在**窗口之外**发布，窗口里只跑那一个量子。
    let mut param_shifted = param_fixture.project.clone();
    param_shifted.audio_config.sample_rate = SampleRate::Hz44100;
    let param_shifted_snapshot =
        EngineSnapshot::from_project(&param_shifted, 2).expect("换采样率快照");
    param_slot.publish(param_shifted_snapshot);
    let (allocations, deallocations) = measure("param rate-change re-arm + quantum", || {
        param_output.fill(0.0);
        param_runtime.process_quantum(&mut param_output, 2);
    });
    println!(
        "[engine-param/J16] 换采样率: allocations={allocations} deallocations={deallocations} \
         目标保留={:?}",
        param_runtime.armed_param_target(&param_track),
    );
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "参数目标表在换采样率的快照边界分配/释放了内存: allocations={allocations} \
             deallocations={deallocations}"
        ));
    }
    if param_runtime.armed_param_target(&param_track).is_none() {
        failures.push("换采样率把已武装的参数目标丢了 —— α 重算不该复位平滑器".to_owned());
    }

    // ---- 场景 17：**每轨插入卷积混响**（`crate::insert` 的第三件器件）在实时窗口内零分配 ----
    //
    // 为什么必须单独一个场景（本场景与前面所有场景的**结构差别**）：
    // 卷积混响是引擎里**唯一**一件"要吃一份数据（IR）而不是只吃标量"的插入器件。本票
    // 把那条数据流按"构造期预建 + 快照边界同长度换 IR"的形状接上（`crate::insert`
    // 模块文档 §9.2／§9.5）：
    //   * IR 的**长度**是引擎常量 ⇒ 每个槽位的频谱缓冲在 `EngineRuntime::new` 里建满；
    //   * 快照边界只允许**长度不变**的 `set_impulse_response`（原地复用缓冲）与 `set_params`。
    // 这条判据就是那个说法的运行期证据：10,000 个量子（逐样本路径）必须零分配零释放
    // [MUST-GATE-001, ARCH-RT-001]。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 两条轨，各自的音符都铺满整个窗口（沿用 `saturated_notes`）；
    //   * 轨 A 的**同一台设备**同时带通道条**与**卷积混响旋钮（覆盖"一条轨上三级串联"）；
    //   * 轨 B 只有卷积混响旋钮（覆盖"一台设备只出卷积混响"）；
    //   * `conv_wet = 1.0` ⇒ 器件每个量子都在处理（`is_active()` 恒真）。
    let (mut conv_project, conv_strip_track, conv_only_track) =
        two_track_project(&saturated_notes(), &saturated_notes());
    {
        let entry = conv_project
            .tracks
            .get_mut(&conv_strip_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Strip+Convolution".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "eq_low_gain".to_owned(),
                    value: 6.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "threshold_db".to_owned(),
                    value: -30.0,
                    unit: Some("dB".to_owned()),
                },
                ParameterValue {
                    name: "ratio".to_owned(),
                    value: 8.0,
                    unit: None,
                },
                ParameterValue {
                    name: "conv_wet".to_owned(),
                    value: 1.0,
                    unit: None,
                },
                ParameterValue {
                    name: "conv_dry".to_owned(),
                    value: 0.5,
                    unit: None,
                },
                ParameterValue {
                    name: "conv_ir_decay_s".to_owned(),
                    value: 0.35,
                    unit: Some("s".to_owned()),
                },
                ParameterValue {
                    name: "conv_ir_seed".to_owned(),
                    value: 7.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    {
        let entry = conv_project
            .tracks
            .get_mut(&conv_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices = vec![DeviceDefinition {
            id: EntityId::new(),
            name: "Convolution".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "conv_wet".to_owned(),
                    value: 0.8,
                    unit: None,
                },
                ParameterValue {
                    name: "conv_predelay_s".to_owned(),
                    value: 0.02,
                    unit: Some("s".to_owned()),
                },
                ParameterValue {
                    name: "conv_ir_seed".to_owned(),
                    value: 11.0,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    let conv_snapshot = EngineSnapshot::from_project(&conv_project, 1).expect("卷积混响夹具快照");
    if conv_snapshot.inserts().len() != 2 {
        failures.push(format!(
            "卷积混响夹具应有 2 条插入链（两条轨各一台设备），实际 {} 条",
            conv_snapshot.inserts().len()
        ));
    }
    let conv_slot = SnapshotSlot::new(conv_snapshot);
    let (conv_retire, mut conv_queue) = retire_channel(64);
    let (_sender, conv_receiver) = event_channel(64);
    let (conv_publisher, _conv_collector) = meter_channel(8192);
    let mut conv_runtime =
        EngineRuntime::new(&conv_slot, conv_retire, conv_receiver, conv_publisher);
    let mut conv_output = vec![0.0f32; 128 * 2];
    // 首次武装（`set_impulse_response` 同长度换 IR ＋ `set_params`）与首个量子。
    // 频谱缓冲与预延迟线本身在 `new` 里就分配好了（那是本器件唯一的分配点）。
    // ⚠ 这一步**在测量窗口里**（与前面几个场景的"预热"不同）：它是"池的预建长度
    // 必须等于投影的长度"这条契约的**唯一**拦截点 —— 若两处用了不同的长度，
    // `set_impulse_response` 会在这里重建缓冲（分配），而后面每一次同长度的换 IR
    // 都会是零分配 ⇒ 只有这个窗口能看见它。
    let (first_allocations, first_deallocations) =
        measure("convolution first arm + quantum", || {
            conv_runtime.process_quantum(&mut conv_output, 2);
        });
    if first_allocations != 0 || first_deallocations != 0 {
        failures.push(format!(
            "卷积混响**首次武装**时分配/释放了内存: allocations={first_allocations} \
             deallocations={first_deallocations} —— 两条已知成因：池的预建 IR 长度与投影的 \
             长度不一致（器件会按新长度重建缓冲），或那条路径自己造了临时缓冲 \
             （例如改用会在方法内分配零切片的入口）",
        ));
    }
    if conv_runtime.armed_convolution_slot_count() != 2 {
        failures.push(format!(
            "卷积混响夹具应武装 2 台，实际 {} 台",
            conv_runtime.armed_convolution_slot_count()
        ));
    }
    if conv_runtime.armed_convolution_sample_rate() != 48_000 {
        failures.push(format!(
            "卷积混响缓冲池应按初始快照的 48 kHz 预建，实际 {} Hz",
            conv_runtime.armed_convolution_sample_rate()
        ));
    }
    let conv_ir_frames = yeban_engine::insert::convolution_ir_frames(48_000);
    for (label, track) in [
        ("通道条＋卷积混响", conv_strip_track),
        ("只有卷积混响", conv_only_track),
    ] {
        if conv_runtime.armed_convolution(&track).is_none() {
            failures.push(format!("{label} 那条轨必须武装进实时侧"));
        }
        if conv_runtime.armed_convolution_ir_frames(&track) != Some(conv_ir_frames) {
            failures.push(format!(
                "{label} 那条轨武装的 IR 帧数应为 {conv_ir_frames}，实际 {:?}",
                conv_runtime.armed_convolution_ir_frames(&track)
            ));
        }
    }

    let mut conv_nonzero = 0usize;
    let (allocations, deallocations) = measure("insert convolution 10_000 quanta", || {
        for _ in 0..10_000 {
            conv_runtime.process_quantum(&mut conv_output, 2);
            for sample in &conv_output {
                if *sample != 0.0 {
                    conv_nonzero += 1;
                }
            }
        }
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "每轨插入卷积混响在实时窗口内分配/释放了内存: allocations={allocations} \
             deallocations={deallocations}"
        ));
    }
    if conv_nonzero == 0 {
        failures.push("卷积混响窗口里没有任何非零样本 —— 零分配判据是空转（假绿）".to_owned());
    }
    let conv_stats = conv_runtime.stats();
    // 覆盖度：取**整窗的精确帧数**（与场景 9 同款），不是"大于 0"。
    let expected_conv_frames = 2 * (10_000 + 1) * 128;
    if conv_stats.insert_convolution_frames != expected_conv_frames {
        failures.push(format!(
            "卷积混响整窗处理帧数={}（期望 {} = 2 条轨 × 10,001 个量子 × 128 帧）—— \
             这条零分配判据没有覆盖整条逐样本路径",
            conv_stats.insert_convolution_frames, expected_conv_frames
        ));
    }
    if conv_stats.insert_convolution_rejects != 0 {
        failures.push(format!(
            "采样率没有变、IR 合法，卷积混响不该被拒绝武装，实际拒绝 {} 次",
            conv_stats.insert_convolution_rejects
        ));
    }
    println!(
        "[engine-7/J17] 插入卷积混响: quanta={} 处理帧数={} IR帧数={conv_ir_frames} \
         非零样本={conv_nonzero} 武装采样率={}；尺寸读数（字节）：ConvolutionReverb={} \
         EngineRuntime={}（后者**不含**池本身：池是 Box<[_; 16]>）",
        conv_stats.quanta,
        conv_stats.insert_convolution_frames,
        conv_runtime.armed_convolution_sample_rate(),
        core::mem::size_of::<yeban_engine::insert::ConvolutionReverb>(),
        core::mem::size_of::<EngineRuntime>(),
    );
    // 尺寸判据（本票实测栈溢出的机械形式）：卷积混响池必须是 `Box<[_; 16]>` 而不是内联数组
    // —— 否则 `EngineRuntime`（一个**按值返回、按值传递**的结构体）的每一个局部变量都会
    // 背上一份 16 × `ConvolutionReverb` 的值。
    //
    // 门槛 `192 KiB` 的来历（**实测**）：本机 `size_of::<ConvolutionReverb>() = 5 192` 字节、
    // `size_of::<EngineRuntime>() = 140 784` 字节（**Box 版**，上一条打印就是它；其中
    // `Box<[T]>` 这个胖指针占 16 字节）。
    // 把池改回内联 `[ConvolutionReverb; 16]` 之后的值 = `140 784 − 16 + 16 × 5 192 = 223 840`
    // 字节（算术）> `196 608`（192 KiB）⇒ **本判据变红**；而那次改动在本机还实测让
    // `tests/param_automation.rs` 的 `the_default_paths_are_bit_identical_to_no_wiring`
    // 以 `fatal runtime error: stack overflow` 中止（SIGABRT）。
    if core::mem::size_of::<EngineRuntime>() >= 192 * 1024 {
        failures.push(format!(
            "EngineRuntime 的值尺寸是 {} 字节（门槛 196 608）—— 器件池必须留在堆上（Box）",
            core::mem::size_of::<EngineRuntime>()
        ));
    }

    // ---- 场景 18：重新武装卷积混响（等价 / **换 IR** / 换采样率拒绝 / 换回）（都零分配）----
    //
    // 四段各自对应一条真实路径：
    //   * 31 次等价交换 = **同一条 IR** ⇒ 内容标识（`ConvolutionPlan::ir_hash`）相同
    //     ⇒ 只走 `set_params`（器件的时间状态保留，输出逐位不变）；
    //   * 1 次**换衰减**交换 = 内容标识不同 ⇒ 走 `set_impulse_response`（**同长度**，
    //     原地复用缓冲）⇒ **零分配**。这是全引擎唯一一处"在快照边界上重算
    //     `分区数` 次 256 点变换"的地方：它是乘加，不是分配；
    //   * 1 次**换采样率**交换（44.1 kHz）⇒ IR 帧数变成 `44_100 ÷ 10 = 4_410 ≠ 4_800`
    //     ⇒ 引擎**拒绝**重建缓冲（那会 `Vec` 重分配 + 释放），整段不武装并计数
    //     （每台设备 +1）。一个"无条件重设 IR"的实现会在这里分配 ⇒ 本场景变红；
    //   * 1 次换回 48 kHz ⇒ 必须能重新武装（守卫是"拒绝这一份"，不是"永久停用"）。
    let mut conv_switches = 0u64;
    for revision in 2..=32u64 {
        // 发布在窗口**之外**：控制线程允许分配。
        let next = EngineSnapshot::from_project(&conv_project, revision).expect("快照");
        conv_slot.publish(next);
        let (allocations, deallocations) = measure("convolution re-arm + quantum", || {
            conv_runtime.process_quantum(&mut conv_output, 2);
        });
        if allocations != 0 {
            failures.push(format!(
                "重新武装卷积混响时实时路径分配了 {allocations} 次（revision={revision}）"
            ));
        }
        if deallocations != 0 {
            failures.push(format!(
                "重新武装卷积混响时实时线程释放了 {deallocations} 次（revision={revision}）"
            ));
        }
        conv_switches += conv_queue.drain(64) as u64;
    }
    if conv_switches == 0 {
        failures.push("卷积混响场景没有从退役队列回收任何旧快照 —— 场景 18 是空转".to_owned());
    }
    if conv_runtime.armed_convolution_slot_count() != 2 {
        failures.push("同采样率重新武装之后两条轨的卷积混响都必须仍被武装".to_owned());
    }

    // **换 IR**（衰减旋钮改了）：同长度 ⇒ 必须零分配。
    let mut conv_redecayed = conv_project.clone();
    {
        let entry = conv_redecayed
            .tracks
            .get_mut(&conv_only_track)
            .expect("夹具里必须有那条 MIDI 轨");
        entry.devices[0].params.push(ParameterValue {
            name: "conv_ir_decay_s".to_owned(),
            value: 0.05,
            unit: Some("s".to_owned()),
        });
    }
    let redecayed_snapshot =
        EngineSnapshot::from_project(&conv_redecayed, 33).expect("换 IR 的快照");
    conv_slot.publish(redecayed_snapshot);
    let (allocations, deallocations) = measure("convolution new-IR re-arm + quantum", || {
        conv_runtime.process_quantum(&mut conv_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换一条**同长度**的 IR 时分配/释放了内存: allocations={allocations} \
             deallocations={deallocations} —— 同长度换 IR 必须原地复用缓冲"
        ));
    }
    if conv_runtime.armed_convolution_slot_count() != 2 {
        failures.push("换 IR 之后两条轨的卷积混响都必须仍被武装".to_owned());
    }

    // 换采样率：44.1 kHz 的快照**不**武装卷积混响（IR 帧数会变 ⇒ 缓冲要重建）。
    let mut conv_shifted = conv_redecayed.clone();
    conv_shifted.audio_config.sample_rate = SampleRate::Hz44100;
    let conv_shifted_snapshot =
        EngineSnapshot::from_project(&conv_shifted, 34).expect("换采样率快照");
    conv_slot.publish(conv_shifted_snapshot);
    let before_rejects = conv_runtime.stats().insert_convolution_rejects;
    let (allocations, deallocations) =
        measure("convolution rate-mismatch re-arm + quantum", || {
            conv_runtime.process_quantum(&mut conv_output, 2);
        });
    if allocations != 0 {
        failures.push(format!(
            "换采样率时实时路径分配了 {allocations} 次 —— IR 缓冲绝不能在音频线程重建"
        ));
    }
    if deallocations != 0 {
        failures.push(format!(
            "换采样率时实时线程释放了 {deallocations} 次 —— IR 缓冲绝不能在音频线程重建"
        ));
    }
    let after = conv_runtime.stats();
    if after.insert_convolution_rejects != before_rejects + 2 {
        failures.push(format!(
            "换采样率应按'这一份快照里的卷积混响设备数'（2 台）累加拒绝，实测 {} -> {}",
            before_rejects, after.insert_convolution_rejects
        ));
    }
    if conv_runtime.armed_convolution_slot_count() != 0 {
        failures.push(format!(
            "换采样率之后卷积混响必须整段不武装，实际仍武装 {} 台",
            conv_runtime.armed_convolution_slot_count()
        ));
    }
    // 换回 48 kHz：必须能重新武装（守卫是"拒绝这一份"，不是"永久停用"）。
    let conv_back_snapshot = EngineSnapshot::from_project(&conv_redecayed, 35).expect("换回快照");
    conv_slot.publish(conv_back_snapshot);
    let (allocations, deallocations) = measure("convolution rate-restore re-arm + quantum", || {
        conv_runtime.process_quantum(&mut conv_output, 2);
    });
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "换回 48 kHz 重新武装卷积混响时分配/释放: allocations={allocations} \
             deallocations={deallocations}"
        ));
    }
    if conv_runtime.armed_convolution_slot_count() != 2 {
        failures.push("换回 48 kHz 之后两条轨的卷积混响必须重新武装".to_owned());
    }
    println!(
        "[engine-7/J18] 卷积混响重新武装: 等价交换={} 次；换 IR 1 次；换采样率后拒绝累计={} 次；\
         武装采样率={} Hz",
        conv_switches,
        conv_runtime.stats().insert_convolution_rejects,
        conv_runtime.armed_convolution_sample_rate(),
    );

    // ---- 场景 15：`EngineStats` 的**跨线程只读镜像**（设备腿形态）----
    //
    // 为什么必须单独一个场景：前面 14 个场景都在**同一条线程**上既渲染又读统计
    // （`runtime.stats()`）。设备腿的真实形态是**另一条线程**读（cpal 回调线程拥有
    // `EngineRuntime` ⇒ 控制线程拿不到 `&EngineRuntime`），而这条形态从来没有被
    // 零分配判据覆盖过。本场景把"写者 = 音频线程 / 读者 = 控制线程"真的分开跑。
    //
    // 夹具设计（每一项都对应一件事）：
    //   * 256 个交叠音符（沿用 `saturated_notes`）⇒ 窗口里真的有声部在跑，
    //     统计量每一量子都在变（否则"镜像会更新"这条见证可能空转）；
    //   * 音频线程跑 2_000 个量子，每量子发布一次镜像；控制线程**在同一个窗口里**
    //     持续 `read()` ⇒ 零分配断言同时覆盖**写路径与读路径**；
    //   * 覆盖度见证：读者必须看到**至少两个不同**的 `quanta` 值（证明它在窗口
    //     中间读到了中间值，而不是只在收尾读了一次）。
    //
    // ⚠ 计数型分配器是**进程全局**的，所以三件事必须做对，否则读数会假红/假绿：
    //   ① 建线程与全部 `Arc` 都在**武装之前**（`std::thread::spawn` 自己会分配）；
    //   ② 音频线程进入闭包后先报 `ready`，等 `go` 再开始渲染 ⇒ 线程启动期的分配
    //      不会被记到实时窗口头上；
    //   ③ 读者循环里**只做原子读与整数比较**（不 `println!`、不 `Vec::push`）——
    //      读路径本身也在"零分配"这句话的范围内。
    let mirror_fixture = note_project(&saturated_notes());
    let mirror_snapshot =
        EngineSnapshot::from_project(&mirror_fixture.project, 1).expect("统计镜像夹具快照");
    let mirror_slot = SnapshotSlot::new(mirror_snapshot);
    let (mirror_retire, _mirror_queue) = retire_channel(8);
    let (_mirror_sender, mirror_receiver) = event_channel(64);
    let (mirror_publisher, _mirror_collector) = meter_channel(8192);
    let mut mirror_runtime = EngineRuntime::new(
        &mirror_slot,
        mirror_retire,
        mirror_receiver,
        mirror_publisher,
    );
    let mirror = mirror_runtime.stats_mirror();
    // 预热（窗口外）：包络模板、波表 mip 级、首量子的一次性路径。
    // ⚠ 这块缓冲**移动**进音频线程复用（不在窗口里 `vec!`）：`vec!` 会分配，
    // 而第一版正是在窗口里分配了它 ⇒ 实测 `allocations=1 deallocations=1`（假红）。
    // 这条教训与"建线程必须在武装之前"是同一族：**窗口里只许有实时路径本身**。
    let mut mirror_warmup = vec![0.0f32; 128 * 2];
    mirror_runtime.process_quantum(&mut mirror_warmup, 2);
    let mirror_base_quanta = mirror_runtime.stats().quanta;

    let rt_ready = Arc::new(AtomicBool::new(false));
    let rt_go = Arc::new(AtomicBool::new(false));
    let rt_done = Arc::new(AtomicBool::new(false));
    let rt_ready_child = Arc::clone(&rt_ready);
    let rt_go_child = Arc::clone(&rt_go);
    let rt_done_child = Arc::clone(&rt_done);
    // ⚠ `JoinHandle` 的返回值就是**权威读数**（音频线程自己在最后一个量子之后读的），
    // 用来与控制线程在静止点读到的镜像做逐字段等号。
    //
    // ⚠⚠ **音频线程在 `rt_done` 之后还要等主线程放行（`rt_release`）才返回**。
    // 理由（本票实测的缺陷）：线程收尾会释放**窗口之前分配**的堆内存
    // （`output` 缓冲、`mirror_runtime` 里 PDC 池的 `Vec`），那一类释放只计
    // `dealloc`、不计 `alloc` ⇒ 会以 `allocations=0 deallocations=1` 的形式被记成
    // "实时窗口内分配/释放"，而它与渲染路径、读路径都无关。**接线前**（`origin/main`）
    // 实测：同一条判据连跑 4 次有 **3 次**红、红色行逐字相同（本机 M2；独立探针见报告）。
    // 因此本判据**不放宽任何断言**（仍然是 `allocations == 0 && deallocations == 0`）：
    // 它只是不再把"线程收尾"算进"实时渲染 ＋ 控制线程读"的窗口 —— 窗口由主线程
    // 在读到计数之后显式关闭（`ARMED = false` 与计数读取都在 `rt_release` 之前）。
    let rt_release = Arc::new(AtomicBool::new(false));
    let rt_release_child = Arc::clone(&rt_release);
    let audio_thread = std::thread::spawn(move || {
        rt_ready_child.store(true, Ordering::SeqCst);
        while !rt_go_child.load(Ordering::SeqCst) {
            std::hint::spin_loop();
        }
        let mut output = mirror_warmup;
        for _ in 0..MIRROR_QUANTA {
            mirror_runtime.process_quantum(&mut output, 2);
        }
        let authoritative = mirror_runtime.stats();
        rt_done_child.store(true, Ordering::SeqCst);
        while !rt_release_child.load(Ordering::SeqCst) {
            std::hint::spin_loop();
        }
        authoritative
    });
    while !rt_ready.load(Ordering::SeqCst) {
        std::hint::spin_loop();
    }

    ALLOCATIONS.store(0, Ordering::SeqCst);
    DEALLOCATIONS.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    rt_go.store(true, Ordering::SeqCst);
    let mut reads = 0u64;
    let mut monotone_violations = 0u64;
    let mut distinct_quanta = 0u64;
    let mut last_quanta = 0u64;
    let mut first_quanta = 0u64;
    let mut observed = false;
    while !rt_done.load(Ordering::SeqCst) {
        let snapshot = mirror.read();
        reads += 1;
        if observed && snapshot.quanta < last_quanta {
            monotone_violations += 1;
        }
        if snapshot.quanta != last_quanta {
            distinct_quanta += 1;
            last_quanta = snapshot.quanta;
        }
        if !observed {
            first_quanta = snapshot.quanta;
            observed = true;
        }
    }
    let final_read = mirror.read();
    ARMED.store(false, Ordering::SeqCst);
    let (allocations, deallocations) = (
        ALLOCATIONS.load(Ordering::SeqCst),
        DEALLOCATIONS.load(Ordering::SeqCst),
    );
    println!(
        "[engine-stats-mirror/J15] 跨线程只读镜像: allocations={allocations} \
         deallocations={deallocations} 读次数={reads} 不同 quanta 值={distinct_quanta} \
         区间={first_quanta}..={} 单调违例={monotone_violations}",
        final_read.quanta
    );
    let authoritative = {
        // ⚠ 放行必须在**读到计数之后**（音频线程的收尾释放因此落在窗口之外）。
        rt_release.store(true, Ordering::SeqCst);
        audio_thread.join().expect("音频线程不许 panic")
    };
    if allocations != 0 || deallocations != 0 {
        failures.push(format!(
            "跨线程统计镜像在实时窗口内分配/释放了内存: allocations={allocations} \
             deallocations={deallocations}（写路径与控制线程读路径都在窗口里）"
        ));
    }
    if reads == 0 {
        failures.push("控制线程一次都没读到镜像 —— 场景 15 是空转".to_owned());
    }
    if distinct_quanta < 2 {
        failures.push(format!(
            "控制线程只看到 {distinct_quanta} 个不同的 `quanta` 值 ⇒ 它没有在窗口中间读到中间值，\
             这条判据退化成'收尾读一次'"
        ));
    }
    if monotone_violations != 0 {
        failures.push(format!(
            "镜像的 `quanta` 出现 {monotone_violations} 次回退 —— 计数类字段必须单调不减"
        ));
    }
    // ⭐ **权威判据**：静止点上镜像与权威读数**逐字段相等**（与
    // `snapshot_retire_churn` 的"镜像 == 权威"同一个口径，且同样是**等号**、无容差）。
    if final_read != authoritative {
        failures.push(format!(
            "静止点上镜像与权威读数不一致：\n  镜像 = {final_read:?}\n  权威 = {authoritative:?}"
        ));
    }
    // 覆盖度自检：窗口里真的处理了量子（否则"相等"可能只是两个全零）。
    // ⚠ 比对的是**增量**（`quanta` 是累计量，预热那一个量子在窗口之前就已经计入）。
    if authoritative.quanta != mirror_base_quanta + MIRROR_QUANTA {
        failures.push(format!(
            "音频线程在窗口里推进了 {} 个量子，期望 {MIRROR_QUANTA} 个（窗口前基线 {mirror_base_quanta}）",
            authoritative.quanta - mirror_base_quanta
        ));
    }
    if authoritative.notes_triggered == 0 {
        failures.push("统计镜像场景的窗口里没有触发任何音符 —— 读数没有在动".to_owned());
    }

    // ---- 场景 19：电平 SPSC **满队列**下的实时窗口（`line/engine-12` 追加）----
    //
    // 新增读数 `EngineStats::meter_dropped_frames` 的来源是发布端自己的计数器
    // （`MeterPublisher::publish` 在**音频回调路径**上按"没写进去的条数"推进它）；
    // 读它发生在 `stats()` 里，而 `stats()` 由 `process_quantum` 每次回调调用一次
    // （收尾发布跨线程镜像）⇒ 这条读数与它的来源都在实时窗口内部。本条场景让环
    // **持续满着**（容量 1、控制面不抽干）跑完整个窗口：丢帧分支与新读数的读路径
    // 都在被测量的那 2_000 个量子内，断言仍然是 `allocations == 0 && deallocations == 0`。
    //
    // 两个臂用**同一个夹具**（`note_project`：1 条轨 + 母线 ⇒ 每量子 2 帧）：
    // 大容量臂给出"本应发布的帧数"这个独立基线，容量 1 的臂必须满足
    // `写入 + 丢弃 == 那个基线`（**等号**，无容差）。没有覆盖度见证的话，
    // "读数恒为 0" 也能让等号成立（假绿）。
    const DROP_QUANTA: usize = 2_000;
    let mut drop_output = vec![0.0f32; 128 * 2];

    let drop_snapshot_a =
        EngineSnapshot::from_project(&fixture.project, 1).expect("满队列夹具快照");
    let drop_slot_a = SnapshotSlot::new(drop_snapshot_a);
    let (drop_retire_a, _drop_queue_a) = retire_channel(8);
    let (_drop_sender_a, drop_receiver_a) = event_channel(64);
    let (drop_publisher_a, _drop_collector_a) = meter_channel(4096);
    let mut drop_runtime_a = EngineRuntime::new(
        &drop_slot_a,
        drop_retire_a,
        drop_receiver_a,
        drop_publisher_a,
    );

    let drop_snapshot_b =
        EngineSnapshot::from_project(&fixture.project, 1).expect("满队列夹具快照");
    let drop_slot_b = SnapshotSlot::new(drop_snapshot_b);
    let (drop_retire_b, _drop_queue_b) = retire_channel(8);
    let (_drop_sender_b, drop_receiver_b) = event_channel(64);
    let (drop_publisher_b, _drop_collector_b) = meter_channel(1);
    let mut drop_runtime_b = EngineRuntime::new(
        &drop_slot_b,
        drop_retire_b,
        drop_receiver_b,
        drop_publisher_b,
    );

    // 预热（窗口外）：首份快照武装。
    drop_runtime_a.process_quantum(&mut drop_output, 2);
    drop_runtime_b.process_quantum(&mut drop_output, 2);

    let (drop_a_alloc, drop_a_dealloc) = measure("meter queue 4096 (no drain)", || {
        for _ in 0..DROP_QUANTA {
            drop_runtime_a.process_quantum(&mut drop_output, 2);
        }
    });
    let drop_baseline = drop_runtime_a.stats();
    let (drop_b_alloc, drop_b_dealloc) = measure("meter queue 1 (no drain)", || {
        for _ in 0..DROP_QUANTA {
            drop_runtime_b.process_quantum(&mut drop_output, 2);
        }
    });
    let drop_stats = drop_runtime_b.stats();
    println!(
        "[engine-12/J19] 电平满队列窗口: 大容量臂 2_000 量子 allocations={drop_a_alloc} \
         deallocations={drop_a_dealloc} 本应发布帧数={}; 容量 1 臂 allocations={drop_b_alloc} \
         deallocations={drop_b_dealloc} 写入={} 丢弃={} capacity_drops={}",
        drop_baseline.meter_frames,
        drop_stats.meter_frames,
        drop_stats.meter_dropped_frames,
        drop_stats.meter_capacity_drops
    );
    if drop_a_alloc != 0 || drop_a_dealloc != 0 {
        failures.push(format!(
            "大容量电平臂在实时窗口内分配/释放了内存: allocations={drop_a_alloc} \
             deallocations={drop_a_dealloc}"
        ));
    }
    if drop_b_alloc != 0 || drop_b_dealloc != 0 {
        failures.push(format!(
            "满队列电平臂在实时窗口内分配/释放了内存: allocations={drop_b_alloc} \
             deallocations={drop_b_dealloc}（丢帧计数与新读数的读路径都在窗口里）"
        ));
    }
    if drop_baseline.meter_dropped_frames != 0 {
        failures.push(format!(
            "大容量电平臂不该丢帧，实际 {} —— 读数会假警报",
            drop_baseline.meter_dropped_frames
        ));
    }
    if drop_stats.meter_dropped_frames == 0 {
        failures.push(
            "容量 1 + 不抽干的窗口里 `meter_dropped_frames` 仍是 0 —— 覆盖度不足（假绿）"
                .to_owned(),
        );
    }
    if drop_stats.meter_frames + drop_stats.meter_dropped_frames != drop_baseline.meter_frames {
        failures.push(format!(
            "本应发布的帧数必须等于 写入 + 丢弃：{} + {} ≠ {}",
            drop_stats.meter_frames, drop_stats.meter_dropped_frames, drop_baseline.meter_frames
        ));
    }
    if drop_stats.meter_capacity_drops != 0 {
        failures.push(format!(
            "SPSC 环满不该被记成批次容量不足: capacity_drops={}",
            drop_stats.meter_capacity_drops
        ));
    }
    if !drop_stats.is_meter_lagging() || !drop_stats.is_meter_lagging_since(&drop_baseline) {
        failures.push("丢过帧 ⇒ `is_meter_lagging` / `is_meter_lagging_since` 必须为真".to_owned());
    }
    // 静止点上跨线程镜像必须带上这个字段（与权威读数逐字段相等）。
    let drop_mirror = drop_runtime_b.stats_mirror().read();
    if drop_mirror != drop_stats {
        failures.push(format!(
            "满队列臂的镜像与权威读数不一致：镜像 meter_dropped_frames={} 权威={}",
            drop_mirror.meter_dropped_frames, drop_stats.meter_dropped_frames
        ));
    }

    println!(
        "[engine-sound/J5] 汇总: quanta={} scheduled_notes={} notes_triggered={} voice_steals={} \
         非零样本={nonzero} 峰值={peak:.6} filled(nonzero={filled_nonzero}, scheduled={}, triggered={})",
        stats.quanta,
        stats.scheduled_notes,
        stats.notes_triggered,
        stats.voice_steals,
        filled_stats.scheduled_notes,
        filled_stats.notes_triggered,
    );

    if failures.is_empty() {
        println!(
            "[engine-sound/J5] ok: 10,000 量子（音符铺满窗口）+ 63 次快照交换 + \
             filled_project 4,000 量子 + 2,000 量子整条混音链 + 10,000 量子每轨插入压缩器 \
             + 31 次插入链重新武装 + 10,000 量子每轨插入通道条（EQ＋滤波＋动态） \
             + 31 次通道条重新武装 + 10,000 量子每轨插入混响（两条轨，含通道条＋混响同一台设备） \
             + 31 次混响重新武装 + 换采样率时的拒绝路径 \
             + 10,000 量子每轨鼓机音源（两条轨，一条鼓机＋一条复音） \
             + 31 次鼓机重新武装 + 换采样率 / 换键位映射 / 换回复音合成器 \
             + 10,000 量子波形选择＋两条振荡器支路 + 31 次振荡器重新武装 \
             + 10,000 量子实时侧参数目标表（事件边界 + 逐样本乘法）+ 换采样率的 α 重算 \
             + 10,000 量子每轨插入卷积混响（两条轨，含通道条＋卷积混响同一台设备；\
             IR 在构造期预建、快照边界只做同长度换 IR）\
             + 31 次等价重新武装 + 1 次换 IR + 换采样率时的拒绝路径 + 换回复武装 \
             + 2,000 量子 `EngineStats` 跨线程只读镜像（写者＝音频线程 / 读者＝控制线程），\
             实时窗口内零分配零释放（63 次快照交换同时见证 PDC 延迟读数：对齐 {want_alignment} / \
             引擎输出 {want_output}）\
             + 2 × 2,000 量子电平满队列窗口（容量 1 ⇒ 丢帧读数 meter_dropped_frames 在窗口内\
             非零，且 写入 + 丢弃 == 本应发布帧数）"
        );
        ExitCode::SUCCESS
    } else {
        for failure in &failures {
            eprintln!("[engine-sound/J5] FAIL: {failure}");
        }
        ExitCode::FAILURE
    }
}
