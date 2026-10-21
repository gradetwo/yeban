//! ⭐ ⭐ **路径 ⇒ 已摊平 SFZ 的缓存**（任务单 `#80`：把 `#79` 的模型链条接到**实时／GUI** 路径）。
//!
//! ## 它解决的那一个问题
//!
//! `#79` 把 `TrackV3.sampler` ⇒ `PreparedSfz` 的链条接到了**导出／录音**的构造期。实时／GUI
//! 那半边缺的正是"**发布点**在哪里读盘"：`EngineHost::publish_project` 每个编辑跳都可能被调
//! （生产路径 16 ms 一跳），而把路径摊平成摊平表要**读盘 ＋ 解析 ＋ 解码 PCM**。若在发布点
//! 直接调 [`crate::sfz_load::decode_sfz_plan`]，同一个工程静止时也会**每跳重读一遍磁盘**
//! —— 那是本单的原始约束原话（"要接实时须在 publish／reload 处加路径 ⇒ 摊平表的缓存"）。
//!
//! ## 数据流（⛔ 不新增第二份解码实现）
//!
//! ```text
//! EngineSnapshot::sfz_pending()            // #78：只带"这条轨有一份 SFZ"这一事实
//!    ──▶ SfzPlanCache::get_or_prepare(…)   // ⭐ 本模块：命中 ⇒ 复用；未命中 ⇒ build（读盘）
//!           build = crate::sfz_load::decode_sfz_plan(path, engine_hz)   // #79 的同一条管线
//!    ──▶ arm_pending_sfz(snapshot, hz, prebuilt)   // #79 的同一条注入入口（取代待摊平项）
//!    ──▶ EngineSnapshot::with_sfz(track, plan)
//!    ──▶ SynthEngine::render_track(…, Some(plan.prepared()), …)        // 实时侧第三条音源支路
//! ```
//!
//! ## 失效条件（⭐ 三条，全部可机械判定）
//!
//! | # | 条件 | 判定处 | 后果 |
//! | :--- | :--- | :--- | :--- |
//! | ① | **同一条轨的路径变了** | [`Binding::path`] 逐字节不等 | 未命中 ⇒ 重建（读盘 ＋1） |
//! | ② | **采样率变了** | [`Binding::engine_hz_bits`]（`f32::to_bits`）不等 | 未命中 ⇒ 重建（读盘 ＋1） |
//! | ③ | **路径不再被任何轨引用** | [`SfzPlanCache::evict_unreferenced`] | 该条目被**移除**（⛔ 只涨不落） |
//!
//! ⚠ ①②的判据是**逐字段相等**（`path` 是 `String`，采样率比**位**）而不是"看起来差不多"：
//! 采样率用 `==` 比浮点会把 `48_000.0` 与一个 `48_000.000_001` 判成同一个 —— 那种相等
//! 会让摊平表带着**另一个采样率**的播放速率继续发声（听不出、报告里也看不见的内容错）。
//!
//! ## ⚠ 命中是**按轨绑定**的（`sfz-cache-needs-2`：跨轨首跳不共享）
//!
//! 命中判据是"**这条轨**上一次用的就是这份 (路径, 采样率)"（[`SfzPlanCache::get_or_prepare`]
//! 的那一行比较），⛔ 不是"表里有没有这个路径"。两条后果：
//!
//! * ⭐ **它让失效条件 ① 可判**：轨的 `sampler.path` 一改，绑定就不相等 ⇒ 未命中 ⇒ 重建。
//!   若把命中判据写成"按路径查表"，路径**变了**（查新路径）本来就查不到 ⇒ 那条"永不失效"
//!   的注入**无法**把判据打红 —— 判据必须能红，所以命中判据按轨绑定。
//! * ⚠ **代价**：两条轨绑**同一份** SFZ 时，**首次那一跳各解码一次**（各建自己的绑定）；
//!   之后每一跳两条轨都命中。⛔ 本单不为此加第三种查表（宁可少一层隐含语义）；
//!   登记为 `sfz-cache-needs-2`（需要真正共享时应在引擎侧给 `PreparedSfz` 一个 `Arc`
//!   共享形体，那是引擎侧的决定）。
//!
//! ## 所有权裁决（任务单 `#80` 的第 3 条）
//!
//! 缓存住在 [`crate::engine_host::EngineHost`] 的一个**私有字段**上，⛔ 不是全局。理由三条：
//!
//! 1. **生命周期就是"这一代宿主"**：`EngineHost` 是实时引擎在控制侧的owner；缓存的有效期
//!    与"这台宿主手里的快照"同寿。工程换、宿主换 ⇒ 缓存自然作废（⛔ 不需要额外的失效协议）。
//! 2. **单写者、无锁**：`reload` / `publish_project` 都是 `&mut self`（控制线程），
//!    ⇒ 不需要 `Mutex`／`Arc` 包一层（全局缓存则必须，且会把"控制侧"变成"跨线程共享态"）。
//! 3. **判据可隔离**：全局缓存会让并行的判据互相看到对方的条目（本仓实测过这类假读数的代价）
//!    ⇒ 按宿主持有是唯一能让"读盘次数不增加"这条判据**确定**的形态。
//!
//! ## 红线 7（[MUST-GATE-001]）
//!
//! 本模块**全部**在控制侧：`get_or_prepare` 只在**未命中**时经 `build` 读盘（那条路是
//! [`crate::sfz_load::decode_sfz_plan`]），命中时只做一次 `BTreeMap` 查表与 `Arc` clone。
//! ⛔ 实时线程不读盘、不查本缓存：快照里那张摊平表是唯一的实时侧事实源。
//! ⚠ 注入要 clone 一份 `SfzPlan` 给快照（⭐ `#279` 起 PCM 是 `Arc<Vec<f32>>` ⇒ 这次 clone
//! 只复制区域表/路径，样本字节 `Arc::clone` 共享，⛔ 不再 memcpy PCM；
//! 见 `yeban_engine::sfz_instrument::PreparedSample` 的字段文档）——`sfz-cache-needs-1` 记的
//! 那次 PCM 拷贝不再发生，⛔ 不进回调。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use yeban_engine::sfz_instrument::SfzPlan;
use yeban_model::EntityId;

use crate::sfz_load::SfzLoadError;

/// 一条轨**上一次**是用哪一份 (路径, 采样率) 摊平的 —— **失效判据的载体**。
///
/// ⚠ 它与 `by_path` 里的摊平表成对存在：`by_track` 说"这条轨此刻应该用哪一份"（命中判据），
/// `by_path` 才是那份数据本体（按路径存 ⇒ 失效条件 ③ 的回收单位是"路径"）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    /// 路径（**逐字**来自 `PendingSfz::path`／`TrackV3.sampler.path`，⛔ 不规范化）。
    path: String,
    /// 摊平用的引擎采样率**位型**（`f32::to_bits`）。比位而不是比 `==`：见模块文档 ①②。
    engine_hz_bits: u32,
}

/// 缓存的**读数**（判据与诊断）。单位：`entries` / `tracks` 是**此刻的条数**，
/// `hits` / `misses` / `evicted` 是**累计条数**（单调不减）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SfzCacheReadings {
    /// 此刻缓存着几份摊平表（按**路径**去重）。
    pub entries: usize,
    /// 此刻有绑定记录的轨数（每条轨一条 `(路径, 采样率)`）。
    pub tracks: usize,
    /// 命中次数（复用：**没有**读盘）。
    pub hits: u64,
    /// 未命中次数（重建：读盘一次 ＋ 解码该 SFZ 引用的样本）。
    pub misses: u64,
    /// 被移除的**条目**数（失效条件 ③；⛔ 不是轨数）。
    pub evicted: u64,
}

/// ⭐ **路径 ⇒ 已摊平 SFZ** 的**控制侧**缓存（所有权裁决见模块文档）。
#[derive(Debug, Default)]
pub struct SfzPlanCache {
    /// 摊平表本体，按**路径**存（失效条件 ③ 的回收单位就是这个键）。
    by_path: BTreeMap<String, Arc<SfzPlan>>,
    /// 每条轨**上一次**用的 `(路径, 采样率)` —— 命中判据（按轨绑定，见模块文档）。
    by_track: BTreeMap<EntityId, Binding>,
    hits: u64,
    misses: u64,
    evicted: u64,
}

impl SfzPlanCache {
    /// 空缓存。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// ⭐ 取**这条轨此刻**该用的摊平表：命中 ⇒ 复用（⛔ 不读盘）；未命中 ⇒ `build` 后入缓存。
    ///
    /// ⭐ **命中判据 = 这条轨上一次用的就是这份 (路径, 采样率)**（[`Binding`] 逐字段相等）。
    /// ⛔ 不是"表里有没有这个路径"：按路径查表时"路径变了"本来就查不到 ⇒ 失效条件 ① 无法判。
    /// ⚠ 按轨绑定的代价（跨轨首跳不共享）见模块文档的 `sfz-cache-needs-2`。
    ///
    /// `build` 是**唯一**的读盘/解码出口 —— 产品路径传的是
    /// [`crate::sfz_load::decode_sfz_plan`]（`#79` 的同一条六步管线），⛔ 本模块不另写一份。
    ///
    /// ⚠ **失败 ⇒ 缓存一位不动**（旧条目留着供下一条轨/下一次发布用），错误原样透出
    /// ⇒ 调用方（`reload`／`publish_project`）明确失败，⛔ 不静默回退到复音合成器。
    ///
    /// # Errors
    ///
    /// `build` 的错误（SFZ 文件读不到 / 文本非法 / 区域表摊不平）。
    pub fn get_or_prepare<F>(
        &mut self,
        track: EntityId,
        path: &str,
        engine_hz: f32,
        build: F,
    ) -> Result<Arc<SfzPlan>, SfzLoadError>
    where
        F: FnOnce(&str, f32) -> Result<SfzPlan, SfzLoadError>,
    {
        let binding = Binding {
            path: path.to_owned(),
            engine_hz_bits: engine_hz.to_bits(),
        };
        // ⭐ 命中 ⇔ **同一条轨**上一次用的就是这份 (路径, 采样率) **且**那份摊平表还在。
        if self.by_track.get(&track) == Some(&binding)
            && let Some(plan) = self.by_path.get(path)
        {
            self.hits = self.hits.saturating_add(1);
            return Ok(Arc::clone(plan));
        }
        // 未命中 ⇒ 重建（读盘发生在这里 —— 控制侧）。
        let plan = build(path, engine_hz)?;
        self.misses = self.misses.saturating_add(1);
        self.by_path.insert(path.to_owned(), Arc::new(plan));
        self.by_track.insert(track, binding);
        // ⭐ 路径变了的**旧那份**在这里走（失效条件 ③）。
        self.evict_unreferenced();
        Ok(Arc::clone(
            self.by_path.get(path).expect("刚插入 ⇒ 必然在表里"),
        ))
    }

    /// ⭐ **只保留还有 SFZ 绑定的轨**（失效条件 ③ 的入口）。
    ///
    /// 调用点在**每次发布之前**（[`crate::engine_host::EngineHost`]）：把 sampler 清回 `None`
    /// 的轨在这里丢掉它的绑定记录，随后它的路径若不再被任何轨引用 ⇒ 摊平表被移除。
    /// ⛔ 不调它的话，检出/改绑留下的旧条目会**只涨不落**（工程开着越久占得越多）。
    pub fn retain_tracks(&mut self, live: &BTreeSet<EntityId>) {
        let before = self.by_track.len();
        self.by_track.retain(|track, _| live.contains(track));
        if self.by_track.len() != before {
            self.evict_unreferenced();
        }
    }

    /// 丢掉"没有任何轨在用"的摊平表条目，并把**移除的条数**记进 `evicted`。
    fn evict_unreferenced(&mut self) {
        let before = self.by_path.len();
        {
            let live: BTreeSet<&str> = self
                .by_track
                .values()
                .map(|binding| binding.path.as_str())
                .collect();
            self.by_path.retain(|path, _| live.contains(path.as_str()));
        }
        let removed = before.saturating_sub(self.by_path.len());
        self.evicted = self
            .evicted
            .saturating_add(u64::try_from(removed).unwrap_or(u64::MAX));
    }

    /// 此刻的读数（[`SfzCacheReadings`]）。
    #[must_use]
    pub fn readings(&self) -> SfzCacheReadings {
        SfzCacheReadings {
            entries: self.by_path.len(),
            tracks: self.by_track.len(),
            hits: self.hits,
            misses: self.misses,
            evicted: self.evicted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一份最小的摊平表（判据不读盘：`prepare` 的广播路径只用一份内存正弦）。
    fn tiny_plan() -> SfzPlan {
        let instrument = yeban_engine::sfz_instrument::SfzInstrument::load(
            "<region> sample=a.wav key=60\n".to_owned(),
            &yeban_sfz::parser::ParseLimits::default(),
        )
        .expect("最小 SFZ 必须解析成功");
        SfzPlan::prepare(
            &instrument,
            &yeban_engine::sfz_instrument::InMemorySample::sine(440.0, 48_000.0, 64),
            48_000.0,
        )
        .expect("摊平必须成功")
    }

    /// 缓存的机械性质：同轨同路径同采样率 ⇒ 命中不重建；路径变 / 采样率变 ⇒ 重建 ＋ 回收；
    /// 跨轨首次**不**共享（`sfz-cache-needs-2`，⛔ 不假装）。
    #[test]
    fn the_cache_reuses_by_binding_and_invalidates_on_path_or_rate_change() {
        let mut cache = SfzPlanCache::new();
        let a = EntityId::new();
        let b = EntityId::new();
        // `RefCell` 只是让判据能在闭包还活着时读回重建记录（闭包按 `Fn` 借它）。
        let builds = std::cell::RefCell::new(Vec::<(String, u32)>::new());
        let build = |path: &str, hz: f32| {
            builds.borrow_mut().push((path.to_owned(), hz.to_bits()));
            Ok(tiny_plan())
        };
        let build_count = || builds.borrow().len();

        // ① 首次 ⇒ 未命中、读盘一次。
        cache
            .get_or_prepare(a, "a.sfz", 48_000.0, build)
            .expect("首次摊平");
        assert_eq!(cache.readings().misses, 1, "首次必须未命中");
        assert_eq!(cache.readings().hits, 0, "还没有命中");
        assert_eq!(build_count(), 1, "首次必须真的重建一次");

        // ② 同一条轨 ＋ 同路径 ＋ 同采样率 ⇒ 复用，**不重建**（要求 ①）。
        cache
            .get_or_prepare(a, "a.sfz", 48_000.0, build)
            .expect("复用");
        assert_eq!(cache.readings().hits, 1, "同轨同路径同采样率必须命中");
        assert_eq!(build_count(), 1, "命中⛔ 不得再读盘/重建");

        // ③ 同一条轨**路径变** ⇒ 重建；旧条目（a.sfz）不再被引用 ⇒ 被移除。
        cache
            .get_or_prepare(a, "b.sfz", 48_000.0, build)
            .expect("路径变 ⇒ 重建");
        assert_eq!(cache.readings().misses, 2, "路径变必须未命中");
        assert_eq!(build_count(), 2, "路径变必须真的重建一次");
        assert_eq!(builds.borrow()[1].0, "b.sfz", "重建的必须是新路径");
        assert_eq!(cache.readings().entries, 1, "旧条目必须被回收");
        assert_eq!(cache.readings().evicted, 1, "回收计数 +1");

        // ④ 同一条轨**采样率变** ⇒ 重建（同一个路径也算未命中）。
        cache
            .get_or_prepare(a, "b.sfz", 96_000.0, build)
            .expect("采样率变 ⇒ 重建");
        assert_eq!(cache.readings().misses, 3, "采样率变必须未命中");
        assert_eq!(
            builds.borrow()[2].1,
            96_000.0_f32.to_bits(),
            "重建用的是新采样率"
        );

        // ⑤ ⚠ **跨轨首次不共享**（按轨绑定）：另一条轨碰同一份 (路径, 采样率) 仍是未命中。
        //    这是**登记过的代价**（模块文档 `sfz-cache-needs-2`），⛔ 不是缺陷被藏起来。
        cache
            .get_or_prepare(b, "b.sfz", 96_000.0, build)
            .expect("跨轨首跳重建");
        assert_eq!(cache.readings().misses, 4, "跨轨首跳必须未命中（已登记）");
        assert_eq!(cache.readings().hits, 1, "命中仍只有第 ② 条那一次");

        // ⑥ 清空绑定 ⇒ 条目全部回收。
        cache.retain_tracks(&BTreeSet::new());
        assert_eq!(cache.readings().entries, 0, "没有轨引用 ⇒ 条目归零");
        assert_eq!(cache.readings().tracks, 0, "绑定记录也归零");
        assert_eq!(cache.readings().evicted, 2, "两次回收都被记下");
    }
}
