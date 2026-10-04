# 夜半 (Yeban) 第三方依赖与开源许可审计表 (Third-Party Licenses)

> **审计基准日期**：2026-10-04  
> **锁定标准**：符合 GPL-3.0-or-later 兼容性与 `deny.toml` 白名单规范

---

## 1. 核心 Rust Crate 依赖审计矩阵 (Core Dependency Audit Matrix)

| 组件名称 (Crate) | 最低锁定版本 | SPDX 许可证标识 | 引入范围与使用方式 | 是否修改 | 是否静态链接 | 是否分发 | 是否可选 | 风险等级 | 合规动作与处置措施 |
| :--- | :---: | :--- | :--- | :---: | :---: | :---: | :---: | :---: | :--- |
| **`slint`** | `=1.8.0` | `GPL-3.0-only` / `Commercial` | `yeban-app` GUI 界面宿主 | 否 | 是 | 是 | 否 | 中 | 夜半自身以 GPLv3 开源发布，完全符合 Slint GPLv3 授权条款；在 LEGAL.md 明晰第三方闭源 fork 限制。 |
| **`slint-build`** | `=1.8.0` | `GPL-3.0-only` / `Commercial` | 编译期 `.slint` 声明式 DSL 转译 | 否 | 否 (仅构建) | 否 | 否 | 极低 | 仅作为编译时 build-dependency，不引入运行时风险。 |
| **`cpal`** | `^0.15` | `Apache-2.0` | `yeban-engine` 跨平台声卡驱动直驱 | 否 | 是 | 是 | 否 | 极低 | 官方宽松许可；关闭其默认的专有 ASIO 编译特性，Windows 默认使用 WASAPI。 |
| **`rtrb`** | `^0.3` | `MIT` OR `Apache-2.0` | UI 与实时音频线程无锁 SPSC 通信 | 否 | 是 | 是 | 否 | 极低 | 宽松双协议，保持版权与许可文本。 |
| **`symphonia`** | `^0.5` | `MPL-2.0` | `yeban-decode` 全格式媒体解封装/解码 | 否 | 是 | 是 | 否 | 低 | 作为独立 crate 引用，不修改其内部源码，满足 MPL-2.0 隔离与源码声明要求。 |
| **`hound`** | `^3.5` | `Apache-2.0` | 标准 PCM WAV 读写 | 否 | 是 | 是 | 否 | 极低 | 宽松许可，保留声明。 |
| **`midly`** | `^0.5` | `MIT` OR `Apache-2.0` | 零堆分配 Standard MIDI 0/1 解析/序列化 | 否 | 是 | 是 | 否 | 极低 | 宽松双协议，保留声明。 |
| **`rubato`** | `^0.15` | `MIT` | Sinc 多相插值高精度采样率重采样 | 否 | 是 | 是 | 否 | 极低 | 宽松 MIT 许可，保留声明。 |
| **`biquad`** | `^0.4` | `MIT` | 通道条二阶 IIR 滤波拓扑计算 | 否 | 是 | 是 | 否 | 极低 | 宽松 MIT 许可，保留声明。 |
| **`signalsmith-stretch`** | `^0.1` | `MIT` | 瞬态保持时间弹性拉伸 (Time-Stretching) | 否 | 是 | 是 | 否 | 极低 | 宽松 MIT 许可，替代存在传染性或商业限制的 Rubber Band。 |
| **`rstar`** | `^0.12` | `MIT` OR `Apache-2.0` | 钢琴卷帘 10 万音符 R*-Tree 视口裁剪 | 否 | 是 | 是 | 否 | 极低 | 宽松双协议，保留声明。 |
| **`yrs`** | `^0.18` | `MIT` | [v1.5.0] Rust Yjs CRDT 多人/多 Agent 协同 | 否 | 是 | 是 | 是 | 极低 | 预留特性，宽松 MIT 许可。 |
| **`clack`** | `^0.3` | `MIT` OR `Apache-2.0` | `yeban-plugin-host` CLAP 宿主桥接 | 否 | 是 | 是 | 是 | 极低 | 现代开源友好协议；结合项目 GPLv3 §7 例外条款加载第三方插件。 |
| **`vst3-sys`** | `^0.6` | `GPL-3.0-only` | `yeban-plugin-host` VST3 FFI 绑定 | 否 | 是 | 是 | 是 | 低 | 自身以 GPLv3 发布，与夜半许可证天然兼容；对应 VST3 SDK 3.8.0+ 已为 MIT。默认作为实验特性关闭。 |
| **`nih-plug`** | Git锁定 | `GPL-3.0` OR `MPL-2.0` | `yeban-vst` 反向插件打包导出 | 否 | 是 | 是 | 是 | 低 | 物理隔离于外围独立 crate，默认不包含在主执行程序中。 |
| **`ulid`** | `^1.1` | `MIT` | 全局确定性时序 EntityId 标识符 | 否 | 是 | 是 | 否 | 极低 | 宽松 MIT 许可，保留声明。 |
| **`rand_xoshiro`** | `^0.6` | `MIT` OR `Apache-2.0` | 纯确定性 PRNG 伪随机数生成器 | 否 | 是 | 是 | 否 | 极低 | 固定算法，保障声学确定性导出。 |
| **`flate2` / `zip`** | `^1.0` / `^0.6` | `MIT` OR `Apache-2.0` | `.yeban` 容器与 Gzip 压缩 | 否 | 是 | 是 | 否 | 极低 | 配置防范 Zip-Slip 与解压炸弹。 |
| **`serde` / `serde_json`** | `^1.0` | `MIT` OR `Apache-2.0` | AST 数据模型确定性序列化 | 否 | 是 | 是 | 否 | 极低 | 宽松双协议。 |

---

## 2. 外部模型与媒体资产许可状态 (Models & Media Assets)

| 资产类别 | 资产描述 / 路径 | 许可证 | 是否分发 | 风险评估与合规动作 |
| :--- | :--- | :--- | :---: | :--- |
| **原声乐器采样 (323 款)** | `assets/samples/*` | `CC0-1.0`, `CC-BY-4.0`, `MIT` | 是 | 详见 `assets/samples/ATTRIBUTION.md`，CI 实施 SHA-256 指纹自动化核查。 |
| **界面英文字体 (Inter)** | `assets/fonts/Inter-*` | `OFL-1.1` | 是 | 保留 SIL Open Font License 1.1 完整文本与保留字体名称声明。 |
| **界面等宽字体 (JetBrains Mono)** | `assets/fonts/JetBrainsMono-*` | `OFL-1.1` | 是 | 保留 SIL Open Font License 1.1 完整文本。 |
| **界面矢量图标 (Lucide)** | `crates/yeban-app/ui/icons/*` | `ISC` (MIT 兼容) | 是 | 保留 Lucide 图标集开源版权声明。 |
| **AI 模型权重 (Basic Pitch)** | `assets/models/basic_pitch.onnx` | `Apache-2.0` | 可选 | Spotify 官方开源权重，遵循宽松 Apache-2.0 协议，模型清单详见 `assets/models/MANIFEST.json`。 |
| **AI 模型权重 (DeepFilterNet)** | 外置 Sidecar 路径 | `MIT` / `Apache-2.0` | 否 | 官方主仓库不随附大体积权重，由用户本地环境按需下载加载。 |
| **专有格式 (NKI / EXS24 / RVC)** | 无 | **N/A** | **严禁** | 彻底从主线仓库中剔除，不收录、不分发任何相关解析器或模型。 |
| **Steinberg ASIO SDK** | 无 | `Steinberg Proprietary` | **严禁** | 严禁包含专有头文件，Windows 默认使用开源 WASAPI 独占模式。 |
