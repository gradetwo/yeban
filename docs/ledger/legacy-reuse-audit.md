# 历史代码复用审计：`groove` 与 `synth`

- **审计时刻**: 2026-10-05
- **审计范围**: `/Users/crow/work/music/groove`、`/Users/crow/work/music/synth`（只读；未运行任何构建）
- **目的**: 夜半是从头设计的项目，无历史包袱。历史代码**能用就直接用**，用不了就明确丢弃，
  并把"为什么"和**许可风险**留痕（SKILL「Reach outside before you invent」+「Licences are part of the design」）。

> 本文件是**账本**，不是规范。它记录一次搜索的结果，供下一个动手的人少走弯路。

---

## 1. 两条前提修正（先说清楚，否则后面全错）

1. **`groove` 不是 Rust 项目。** 它是 TypeScript + React 18 + Vite 的 PWA（MIT, "Groove Lab"），
   `src` 下 323,136 行 TS/TSX，**零** `.rs` 与 **零** `Cargo.toml`。唯一的"Rust 内容"是它
   vendored 的一个**已编译 wasm 产物**（来自 `synth`）。
2. **全机器只有一个 Rust crate，且它在 `synth` 里**：`synth/crates/synth-core`
   （单 crate，无 workspace，**零依赖**，`license = "MIT"`，22,346 行 Rust + ~7,000 行 vendored C/C++）。

结论：所谓"复用 groove/synth"实际上等于"复用 `synth-core` 的 DSP + 借鉴两个项目的**文档与方法论**"。

---

## 2. 唯一的大额复用：`synth/crates/synth-core/src/dsp/**`（MIT）

约 **8,000 行无依赖、无分配、带单元测试的纯 Rust DSP**，其中只有 3 个文件与 wasm 耦合且都有标量回退。

| 来源 | 行数 | 结论 | 落到夜半哪里 |
| :--- | :--- | :--- | :--- |
| `dsp/adsr.rs` | 262 | **可直接复用** | `yeban-dsp` 包络 |
| `dsp/ladder.rs` | 250 | **可直接复用**（自研 ZDF 4 极，带 < −70 dB THD 透明性测试） | `yeban-dsp` 滤波器 |
| `dsp/lfo.rs` / `dsp/util.rs` / `dsp/noise.rs` / `dsp/fmath.rs` / `dsp/simd.rs` | 110 / 220 / 221 / 617 / 129 | **可直接复用** | `yeban-dsp` 基础原语 |
| `dsp/wavetable.rs` | 514 | **可直接复用**（mipmap 限带表，实测 −105 dB 工厂波表 / −119 dB 导入锯齿 @C2） | `yeban-dsp` 振荡器 |
| `dsp/oversample.rs` | 304 | **可直接复用**（2× 过采样，显式 `OS_LATENCY`/`OS_TAPS`） | `yeban-dsp` |
| `dsp/comb.rs` / `dsp/delay.rs` / `dsp/reverb.rs` | 216 / 395 / 435 | **可直接复用** | `yeban-dsp` |
| `fx_shaping.rs` | 1,239 | **可直接复用**（bitcrusher / shaping EQ / transient shaper） | `yeban-dsp` |
| `dsp/convolution.rs` | 869 | **移植后可用**（分区大小是按 wasm arena 调的） | `yeban-dsp` |
| `dsp/sampler.rs` | 1,256 | **移植后可用**；`MAX_BASE_SAMPLES = 192_000`（4s@48k）是 **wasm 内存上限**，夜半应解除 | 与 `yeban-sfz` 配合 |
| `voice.rs` | 456 | **移植后可用**（抢占 + steal-fade + pending queue，注释质量高） | `yeban-engine` 声部池；`MAX_VOICES=32` 需按规范改为 512/1024 |
| `alloc_arena.rs` / `shim.rs` | 332 / 147 | **丢弃**（wasm 专用全局分配器与 freestanding libc shim） | — |
| vendored `daisysp/`、`soundpipe/`、`c_bridge/` | ~7,000（C/C++） | **丢弃**（与"纯血 Rust"宪章冲突；且该项目自己已用 `dsp/ladder.rs`、`dsp/adsr.rs` 替换了会出问题的 C） | — |
| `dual_filter.rs` | 693 | **丢弃**（`#[cfg(test)]` 专用测试台） | — |

**复用纪律（必须执行，不是建议）**

1. 移植文件头保留原始版权与 **MIT** 声明，并注明来源路径；
2. 在 `THIRD_PARTY_LICENSES.md` 登记 `synth-core` 及其 MIT 许可；
3. 逐文件改写为夜半的接口与命名，**不做逐行转译**（SKILL：靠理解复用，不靠粘贴）；
4. 保留其已有的回归测试（尤其是 `ladder.rs` 的爆音连续性与频谱透明性测试——那是真 bug 的化石）。

---

## 3. 借鉴（learn-from），不复制代码

| 主题 | 来源 | 借鉴什么 |
| :--- | :--- | :--- |
| 速度图语义 | `synth/src/midi/tempo.ts` | 以**拍**为边界的 tempo segment、边界处精确换算、按拍号重启小节计数、`gridStepAt` |
| SMF 边界用例 | `synth/src/midi/smf.test.ts`、`groove/src/audio/MidiExporter.ts` | `writeVLQ`、歌词 `FF 05` 事件、GM 鼓通道映射、SMPTE division 明确抛错 |
| 帧寻址事件 | `synth/docs/notes/groove-host-protocol.md` | 绝对 `atFrame` 的事件队列语义（不对齐 128 帧量化、过期帧立即执行、非有限 `atFrame` 整条忽略）——正是走带需要的调度纪律 |
| 样本缓存不变量 | `groove/src/audio/sampleLoader.ts` | **缓存 promise 而不是结果**；**失败绝不缓存** |
| 工程文件规则 | `synth/src/state/persist.ts`、`patchfile.ts`、`projects.ts` | 前进/后退兼容信封（新版本拒绝并另存）、**永不抛异常的 total parser** + 硬上限、先写清单再替换活文档的崩溃安全顺序 |
| 撤销实现教训 | `groove/src/data/arrangementHistory.ts` | `canUndo/canRedo` 必须是渲染状态而不是 ref；栈放 ref；`commit` 读 ref 以免同一 tick 内两次编辑互相丢弃 |
| 确定性音频门禁 | `synth/scripts/dsp-baseline.mjs` + `tests/dsp-baseline*.json` | 用 RMS + 12 个对数频段幅度做**音色指纹**，任何非预期音色变化都必须让人确认 |
| 基准历史入库 | `synth/scripts/bench.mjs` | 持续性负载基准，并把每次读数**追加进 git 里的 performance 记录** |
| 参考实现在环 | `groove/.github/workflows/sfizz-oracle.yml` | 先编译一个**外部参考实现**并对齐读数，再信任自家实现（夜半做采样器时应照做） |
| 文档与源码双向契约 | `synth/scripts/verify-worklet-protocol.mjs` | 让"文档里的表"与"源码里的常量"不一致时构建失败——这是消除文档漂移最便宜的办法 |
| 渲染性能画像 | `groove/docs/RENDER_PROFILE.md` | 浏览器离线渲染中 96–99.9% 时间在 `startRendering()` 内；空图 107–181× 实时，1000 个 GainNode 就掉到 14.1s |
| 宿主决策论证 | `groove/docs/RUST_DECISION.md`、`HEADLESS_CORE_PLAN.md` | DSP 本身早已 10× 实时，真正的瓶颈是浏览器宿主——这正是夜半转原生桌面的经验依据 |
| 排版/符号 | `groove/public/fonts/bravura/bravura.woff2` + `OFL.txt` | Bravura SMuFL 乐谱字体，**SIL OFL 1.1**，可随夜半分发（注意保留字体名条款） |

---

## 4. 明确丢弃

| 项 | 原因 |
| :--- | :--- |
| `groove/src/audio/{AudioEngine,masterGraph,EffectsRack}.ts` | Web Audio 节点图，与 cpal 原生引擎无共同点 |
| `groove/src/components/**`、`synth/src/components/**`（React/Tailwind，约 43,800 行） | Slint 与 DOM 零共享 |
| `groove/src/audio/sfz/**`（TS, 2,979 行） | **代码不复用**，但语义（keyswitch / include / define / CC gate / `mirrorPlan`）值得移植时对照重写为 Rust |
| 旧 Node/TS MCP（`groove/src/mcp/*`） | 夜半的 MCP 是 Rust，且安全模型（环回绑定 + 0600 Token + 六级 scope）完全不同 |
| `groove/vendor/gs1/**`、`groove/public/gs1/*.wasm` | 已编译 wasm 产物，被"直接复用 Rust 源码"取代 |
| `synth/crates/synth-core/target/`（457MB）、两侧 `node_modules`（1.08GB） | 构建产物 |

---

## 5. 许可风险（唯一真正的雷区）

| 项 | 许可 | 结论 |
| :--- | :--- | :--- |
| `synth-core`（含 vendored DaisySP / Plaits / Soundpipe） | MIT | 可复制进 GPLv3 的夜半（保留声明） |
| `groove`、`synth` 本体 | MIT | 同上 |
| Bravura 字体 | SIL OFL 1.1 | 可分发；遵守保留字体名条款 |
| `@breezystack/lamejs 1.2.7` | LGPL-3.0 | 与 GPLv3 兼容，但它是 JS chunk，纯 Rust 的夜半不需要 |
| **`groove/public/samples/manifest.json` + `groove/src/data/libraryLicence.ts`** | 其中 **1 个库是 `CC-BY-NC-SA`**，且该逻辑**主动接受** `CC-BY-NC-*` 与 `unknown-mirrored`（"找不到许可也镜像，收到请求再撤"） | ⛔ **绝不可移植进夜半。** CC-BY-NC 属于"非商业限制"，是 GPLv3 §7 明令禁止的附加限制；GPLv3 又向所有下游授予商业使用权，因此"我们非商业"这条自我豁免在夜半**不成立**。夜半的采样政策只允许 CC0 / CC-BY / CC-BY-SA / 公有领域 / OFL。（所幸 `groove/public/samples/` 只有 manifest，没有音频字节，当前仓库没有被污染。） |
| `groove/tools/covers/_review/*.jpg`（14 个，1.2–2.5MB） | **无许可声明** | 来源不明，不得复制 |

> 结论：**代码复用没有许可障碍，唯一的许可障碍是 groove 的采样清单与它的许可接受逻辑。**

---

## 6. 待办（进 `needs`）

1. 判定哪些 DSP 文件先进 `yeban-dsp`：建议 `util/fmath/noise/adsr/ladder/lfo/comb/delay/reverb/oversample/wavetable/fx_shaping` 首批，
   `sampler/convolution/voice` 等 `yeban-sfz`/`yeban-engine` 就位后再移植；
2. `THIRD_PARTY_LICENSES.md` 增补 `synth-core`（MIT, 2026, GROOVE SYNTH GS-1 contributors）与 Bravura（OFL-1.1）条目；
3. 采样政策落到 `assets/samples/ATTRIBUTION.md` 时，只允许 CC0/CC-BY/CC-BY-SA/PD/OFL，**明令排除 CC-BY-NC**；
4. `groove/docs/DAW_GAP_ARRANGEMENT.md`、`DAW_GAP_CREATION.md` 是可读的设计输入（每条断言都引了官方手册原文），
   建议在 UI 交互设计时对照阅读，但**不作为规范**。
