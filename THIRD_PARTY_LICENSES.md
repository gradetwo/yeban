# 夜半 (Yeban) 第三方依赖与开源许可审计表 (Third-Party Licenses)

> **审计基准日期**：2026-10-05（版本列已按 `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` D5 的**实测**钉死值校正；
> 原表抄自规范起草期的计划值，与真实 `[workspace.dependencies]` 不一致 —— 那属于"文档说谎"，已修）
> **锁定标准**：符合 GPL-3.0-only 兼容性与 `deny.toml` 白名单规范
> **机器清单**：[`docs/ledger/dependency-licenses.md`](docs/ledger/dependency-licenses.md) 由
> `scripts/gates/license_inventory.py` 从真实依赖图生成，CI 用 `--check` 防止漂移

---

## 1. 核心 Rust Crate 依赖审计矩阵 (Core Dependency Audit Matrix)

> 版本列 = 根 `Cargo.toml` 的 `[workspace.dependencies]` 实际声明值。
> "已引入" 指当前有成员 crate 真正引用（会被编译/进 `Cargo.lock`）；"已登记" 指仅登记待用。
> 全量（含传递依赖）见机器清单。

| 组件名称 (Crate) | 声明版本 | SPDX 许可证标识 | 引入范围与使用方式 | 状态 | 风险等级 | 合规动作与处置措施 |
| :--- | :---: | :--- | :--- | :---: | :---: | :--- |
| **`slint`** | `1.18.1` | `GPL-3.0-only` / `Commercial` | `yeban-app` GUI 界面宿主 | 已登记 | 中 | 夜半自身以 GPLv3 开源发布，符合 Slint GPLv3 授权条款；第三方闭源 fork 须自行取得 Slint 商业许可（见 `LEGAL.md`）。 |
| **`slint-build`** | `1.18.1` | `GPL-3.0-only` / `Commercial` | 编译期 `.slint` DSL 转译 | 已登记 | 极低 | 仅 build-dependency，不进入运行时分发物。 |
| **`i-slint-backend-testing`** | `=1.18.1`（精确） | `GPL-3.0-only` / `Commercial` | 仅属性断言（**不渲染像素**） | 已登记 | 中 | 内部 crate 不遵循 semver，必须与 `slint` **精确同版本**；`MUST-GATE-015` 禁止用它产出 Golden 图。 |
| **`cpal`** | `0.18.2` | `Apache-2.0` | `yeban-engine` 跨平台声卡直驱 | 已登记 | 极低 | 关闭默认的专有 ASIO 编译特性，Windows 默认 WASAPI 独占。 |
| **`rtrb`** | `0.4.0` | `MIT` OR `Apache-2.0` | UI 与实时音频线程无锁 SPSC 通信 | 已登记 | 极低 | 宽松双协议，保留版权与许可文本。 |
| **`symphonia`** | `0.6.1` | `MPL-2.0` | `yeban-decode` 全格式解封装/解码 | 已登记 | 低 | 以独立 crate 引用、不修改内部源码，满足 MPL-2.0 的文件级隔离与源码声明要求。 |
| **`rubato`** | `5.0.1` | `MIT` | 高精度多相采样率重采样 | 已登记 | 极低 | 宽松 MIT，保留声明。 |
| **`hound`** | `3.5.1` | `Apache-2.0` | 标准 PCM WAV 读写 | 已登记 | 极低 | 宽松许可，保留声明。 |
| **`midly`** | `0.5.3` | `MIT` OR `Apache-2.0` | 零堆分配 SMF 0/1 解析/序列化 | 已登记 | 极低 | 宽松双协议，保留声明。 |
| **`midir`** | `0.11.0` | `MIT` | 物理 MIDI 输入（[v1.1.0] 起） | 已登记 | 极低 | 宽松 MIT，保留声明。 |
| **`raz`**/**`rayon`** | `1.12.0` | `MIT` OR `Apache-2.0` | `yeban-render` 离线母带多核并行 | 已登记 | 极低 | 宽松双协议，保留声明。 |
| **`signalsmith-stretch`** | `0.1.3` | `MIT` | 瞬态保持时间弹性拉伸 | 已登记 | 极低 | 唯一合法的拉伸算法，**显式替代**存在商业/GPL 双轨限制的 Rubber Band（`deny.toml` 中已 `deny` Rubber Band）。 |
| **`rstar`** | `0.13.0` | `MIT` OR `Apache-2.0` | 钢琴卷帘 10 万音符 R\*-Tree 视口裁剪 | 已登记 | 极低 | 宽松双协议，保留声明。 |
| **`clack`** | 未登记（crates.io 无此包名） | `MIT` OR `Apache-2.0` | `yeban-plugin-host` CLAP 宿主桥接 | 待选型 | 中 | [v2.0.0] 前必须完成选型核验（名称/许可/维护状态），见 `docs/DEVELOPMENT_LEDGER.md` 的 pending。 |
| **`vst3-sys`** | 未登记 | `GPL-3.0-only` | `yeban-plugin-host` VST3 FFI 绑定 | 待选型 | 低 | 对应 VST3 SDK ≥ 3.8.0（MIT）；默认作为实验特性关闭。 |
| **`nih-plug`** | 未登记（crates.io 无此包名） | `GPL-3.0` OR `MPL-2.0` | `yeban-vst` 反向插件打包 | 待选型 | 中 | **maintenance mode（RSK-35）**；`deny.toml` 的 `unmaintained = "workspace"` 会在它进入依赖图时报警。 |
| **`ulid`** | `3.0.0` | `MIT` | 确定性时序 `EntityId`（26 字符 Crockford Base32） | **已引入** | 极低 | 3.0.0 **无 `serde` feature**，序列化由 `yeban-model` 手写实现（ADR-0001 D6）。 |
| **`serde` / `serde_json`** | `1.0.229` / `1.0.151` | `MIT` OR `Apache-2.0` | AST 数据模型确定性序列化 | **已引入** | 极低 | 宽松双协议。 |
| **`thiserror`** | `2.0.21` | `MIT` OR `Apache-2.0` | 各 crate 的错误类型派生 | **已引入** | 极低 | 宽松双协议。 |
| **`sha2`** | `0.11.0` | `MIT` OR `Apache-2.0` | `AssetHash` / `ContentHash` 内容寻址摘要 | **已引入** | 极低 | 宽松双协议。 |
| **`libm`** | `0.2.16` | `MIT` | `yeban-theory` 的确定性浮点超越函数（L1 契约要求不依赖平台 libm） | **已引入** | 极低 | 纯 Rust 实现，保障跨平台位级一致。 |
| **`rand_xoshiro`** | `0.8.1` | `MIT` OR `Apache-2.0` | 固定种子 PRNG（L1 确定性） | 已登记 | 极低 | 目前 `yeban-model` / `yeban-theory` **刻意手写 splitmix64** 而未引入它（少一个依赖、确定性更可控）。 |
| **`flate2` / `zip`** | `1.1.10` / `8.6.0` | `MIT` OR `Apache-2.0` | `.yeban` 容器与 Gzip 压缩 | 已登记 | 极低 | 必须配置 Zip-Slip 与解压炸弹防御（`MUST-GATE-006/007`）。 |
| **`notify`** | `8.2.0` | `CC0-1.0` | [v1.1.0] 外部编辑器文件监听与热重载 | 已登记 | 极低 | 公有领域奉献，保留声明。 |
| **`proptest` / `criterion` / `iai-callgrind`** | `1.11.0` / `0.8.2` / `0.16.1` | `MIT` OR `Apache-2.0` | 属性测试与基准门禁（**dev-only**） | **已引入**(dev) | 极低 | 仅在开发/CI 期使用，不进入分发物。 |

---

## 2. 外部模型与媒体资产许可状态 (Models & Media Assets)

| 资产类别 | 资产描述 / 路径 | 许可证 | 是否分发 | 风险评估与合规动作 |
| :--- | :--- | :--- | :---: | :--- |
| **原声乐器采样 (323 款)** | `assets/samples/*` | `CC0-1.0`, `CC-BY-4.0`, `MIT` | 是 | 详见 `assets/samples/ATTRIBUTION.md`，CI 实施 SHA-256 指纹自动化核查（`MUST-GATE-014`）。 |
| **品牌标识 (夜半 logo)** | `assets/brand/**` | 项目自有（见 `TRADEMARK.md`） | 是 | 母版 `yeban.svg` 拆出 10 个变体；再生成方式见 `assets/brand/README.md`。 |
| **乐谱字体 (Bravura)** | 尚未入库（候选：`synth`/`groove` 的 `bravura.woff2`） | `OFL-1.1` | 否 | 复用审计已登记为可用（`docs/ledger/legacy-reuse-audit.md`）。入库时须同时携带 `OFL.txt` 并遵守保留字体名条款。 |
| **界面英文字体 (Inter)** | `assets/fonts/Inter-*` | `OFL-1.1` | 计划 | 保留 SIL Open Font License 1.1 完整文本与保留字体名称声明。 |
| **界面等宽字体 (JetBrains Mono)** | `assets/fonts/JetBrainsMono-*` | `OFL-1.1` | 计划 | 同上。 |
| **界面矢量图标 (Lucide)** | `crates/yeban-app/ui/icons/*` | `ISC`（MIT 兼容） | 计划 | 保留 Lucide 版权声明。 |
| **AI 模型权重 (Basic Pitch)** | `assets/models/basic_pitch.onnx` | `Apache-2.0` | 可选 | Spotify 官方开源权重；清单见 `assets/models/MANIFEST.json`。 |
| **AI 模型权重 (DeepFilterNet)** | 外置 Sidecar 路径 | `MIT` / `Apache-2.0` | 否 | 主仓库不随附大体积权重，由用户本地按需下载。 |
| **历史项目采样清单** | `groove/public/samples/manifest.json`（**未采用**） | 含 `CC-BY-NC-SA` | **严禁** | 该清单主动接受非商业限制许可，**与 GPLv3 不兼容**，不得移植；夜半只允许 CC0/CC-BY/CC-BY-SA/公有领域/OFL。见 `docs/ledger/legacy-reuse-audit.md` §5。 |
| **专有格式 (NKI / EXS24 / RVC)** | 无 | **N/A** | **严禁** | 从主线仓库彻底剔除，不收录、不分发任何相关解析器或模型。 |
| **Steinberg ASIO SDK** | 无 | `Steinberg Proprietary` | **严禁** | 严禁包含专有头文件（`MUST-GATE-013` 机械扫描）；Windows 默认 WASAPI 独占。 |

---

## 3. 源码级移植归属 (Source-Level Ports)

`cargo deny check` 只审**依赖图**，看不到"从别处源码改写移植进来"的代码。
这一类归属只能由本文件承载，因此**每次移植都必须在同一提交里补上条目**，否则构成许可瑕疵。

### synth-core（GROOVE SYNTH GS-1 DSP core）

- **许可**：MIT
- **版权**：Copyright (c) 2026 GROOVE SYNTH GS-1 contributors
- **来源**：本机历史项目 `/Users/crow/work/music/synth`，crate `crates/synth-core`
  （`src/dsp/**` 与 `src/fx_shaping.rs`）；该项目自身未公开托管，因此登记为本地历史仓库来源。
- **夜半使用范围**：`crates/yeban-dsp/src/{envelope,filter,oscillator,noise,math,block,oversample,delay,comb,reverb,shaping}.rs`
  为其改写移植（接口、命名、采样率传递方式均为夜半重写；**未**移植 wasm 专用模块与 vendored C/C++）。
- **逐文件裁决、差异与"未移植清单"**：[`docs/ledger/dsp-core-provenance.md`](docs/ledger/dsp-core-provenance.md)
- **许可全文**：

```text
MIT License

Copyright (c) 2026 GROOVE SYNTH GS-1 contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
