# 夜半 (Yeban) 法律声明、商标与合规政策 (Legal, Trademarks & Compliance)

> **项目名称**：夜半 (Yeban) / Yeban DAW  
> **开源许可证**：GNU General Public License v3.0 (GPLv3) 带 GPLv3 §7 附加许可条款  
> **适用版本**：`v1.0.0-rev1` (2026-10-04) | 项目研发起步版本：`v0.0.1` | 原规划 v3.0 正式确立为首个正式生产基线 `v1.0.0`  
> **源码托管**：GitHub (公开开源项目)

---

## 1. 商标与非关联免责声明 (Trademark & Non-Affiliation Disclaimer)

1. **非关联声明**：  
   “夜半 (Yeban)”（以及曾用代号 Groove Lab）是一个独立的开源纯 Rust 数字音频工作站项目，与 **Ableton AG**（Ableton Live 商标持有者）、**Bitwig GmbH**（Bitwig Studio 商标持有者）、**Apple Inc.**（Logic Pro 商标持有者）、**Cockos Incorporated**（REAPER 商标持有者）、**Image-Line Software NV**（FL Studio 商标持有者）、**PreSonus Audio Electronics, Inc.**（Studio One 商标持有者）、**Roland Corporation**（TR-808, TR-909 商标持有者）、**Solid State Logic**（SSL 商标持有者）以及 **Steinberg Media Technologies GmbH**（VST / ASIO 商标持有者）**无任何官方关联、赞助、从属或背书关系**。

2. **商标权利归属**：  
   设计与技术文档中出现的所有第三方产品名称、商标、服务标记及注册商标（包括但不限于 Ableton Live, Bitwig Studio, Logic Pro, REAPER, FL Studio, Studio One, VST, ASIO 等）均为其各自法定所有者的财产。在本文档中引用上述名称仅用于技术对比、接口兼容性描述与架构设计借鉴，不构成任何商业混淆或侵犯意图。详细商标指引参见 [`TRADEMARK.md`](file:///home/crow/work/agy/review/TRADEMARK.md)。

---

## 2. Slint GUI 框架双授权与第三方闭源 Fork 声明 (Slint Licensing & Forks)

本项目用户界面层采用 **Slint GUI 框架**（由 SixtyFPS GmbH 开发维护）：

1. **夜半 (Yeban) 自身开源合规性**：  
   夜半 (Yeban) 项目整体采用 GPL-3.0-or-later 许可证开源分发，完全符合 Slint 的 GPLv3 开源授权条件，属于合法合规的开源链接。

2. **第三方 Fork 与商业闭源分发限制 (严正声明)**：  
   夜半 (Yeban) 以 GPLv3 发布。任何第三方主体若希望对本项目进行 Fork 并闭源分发衍生作品，**必须同时满足以下两项法律前提**：
   - **前提一 (Yeban 代码)**：必须获得夜半 (Yeban) 著作权持有人的单独商业闭源许可，否则其修改与分发必须整体受 GPLv3 约束；**声明：夜半 (Yeban) 目前不提供任何自有代码的商业闭源许可，全量代码仅以 GPLv3 提供**；
   - **前提二 (Slint 框架)**：若不愿遵循 Slint 的 GPLv3 开源条款，分发者必须自行向 SixtyFPS GmbH 购买并获取 Slint 商业许可。  
   **特别强调**：仅向 SixtyFPS GmbH 购买 Slint 商业许可，**绝不自动获得**闭源分发夜半 (Yeban) 代码的权利。

---

## 3. 音频插件格式合规性与 GPLv3 §7 附加许可 (Plugin Licensing)

### 3.1 CLAP 格式与 GPLv3 §7 附加许可

夜半 (Yeban) 在项目根目录 [`LICENSE`](file:///home/crow/work/agy/review/LICENSE) 中依法授予了基于 GPLv3 第 7 条的附加许可：

```text
Additional permission under GNU GPL version 3 section 7:

If you modify this Program, or any covered work, by linking or combining it
with independent CLAP plugins that are loaded at runtime through the CLAP
hosting interface, the licensors of this Program grant you additional
permission to convey the resulting work solely to the extent necessary to
load and run those independent plugins.

This additional permission applies solely to the copyrightable code of Yeban
itself. It does NOT apply to the Program itself, to modifications of the
Program, to Slint or any other third-party components, and it does NOT permit
proprietary distribution of the Program as a whole. Third-party independent
plugins loaded at runtime remain subject to their respective authors' licenses.
```

- **适用边界**：此例外仅赋予用户与宿主在运行时动态加载独立 CLAP 插件的合法权利；
- **排他限制**：严禁将夜半 (Yeban) 自身核心模块（`yeban-model`, `yeban-dsp`, `yeban-theory` 等）反向嵌入任何专有闭源插件并闭源分发。

### 3.2 VST3 格式状态与证据链

- **许可状态**：Steinberg Media Technologies GmbH 已于 2025 年 10 月将 VST3 SDK（自版本 3.8.0 起）切换为 **MIT 许可证**（来源：Steinberg 开发者官方公告及 GitHub `steinbergmedia/vst3sdk` 仓库发布记录）；
- **工程锁定与降级**：夜半 (Yeban) 锁定最低依赖为 VST3 SDK 3.8.0+。为防范潜在法务风险，VST3 宿主加载能力在官方发行二进制中**默认关闭**，作为 `experimental-vst3` 特性仅供用户在本地环境按需编译；
- **ASIO 专有驱动隔离**：Steinberg ASIO SDK 采用专有不可再分发许可证，**严禁将 ASIO 头文件或源码纳入夜半 Git 仓库**。Windows 平台默认首选开源免驱的 WASAPI 独占模式；ASIO 支持作为独立动态加载模块，仅限持有合法 SDK 的用户本地编译。

---

## 4. 用户创作内容独立性声明 (User Content Exemption)

- GPLv3 许可证仅约束夜半 (Yeban) 软件程序本体及其派生代码；
- 用户使用夜半 (Yeban) 创作的音乐作品，包括工程文件（`.yeban`）、MIDI 片段、自动化曲线、乐器预设、录音音轨以及离线导出的立体声/分轨母带音频（WAV、FLAC、MP3 等），**版权 100% 独立归属于创作者本人**；
- 本软件不会对用户内容产生任何开源传染效应，用户对其创作成果享有完全的商业化与分发自由。

---

## 5. 采样资产、字体与 AI 模型知识产权规范 (Assets & AI Models)

1. **原声乐器采样素材 (323 款)**：
   - 随附的原声乐器音色库严格限于 CC0 1.0 (Public Domain)、CC-BY 4.0 或 MIT/Apache-2.0 授权；
   - 详尽清单、作者署名及对应源码哈希见 [`assets/samples/ATTRIBUTION.md`](file:///home/crow/work/agy/review/assets/samples/ATTRIBUTION.md) 与 [`assets/manifest.json`](file:///home/crow/work/agy/review/assets/manifest.json)；
   - 严禁任何未经授权的商业 Kontakt 音色库切片或商业采样包混入。
2. **字体与图标资产**：
   - 界面字体仅使用开源字体（SIL Open Font License 1.1 或 Apache-2.0），清单见 [`assets/fonts/LICENSES.md`](file:///home/crow/work/agy/review/assets/fonts/LICENSES.md)；
   - UI 矢量图标采用 MIT 或 CC0 许可。
3. **AI 模型权重与检查点 (Checkpoints)**：
   - 软件代码许可不等于模型权重许可。官方发行包**不默认打包大体积专有神经模型权重**；
   - 本地扒带（Basic Pitch）使用其官方 Apache-2.0 授权的轻量 ONNX 权重；
   - 严禁任何带非商业限制（如 CC-BY-NC）或未授权语音克隆（如未经授权的 RVC/Diff-SVC 音色）模型进入主仓库。

---

## 6. 二进制分发渠道策略 (Distribution Channels)

为规避部分商业应用商店（如 Apple Mac App Store）中 DRM 条款与 GPLv3 “禁止施加额外限制”条款之间的潜在法务冲突：
- 夜半 (Yeban) 初期**不通过 Apple App Store 进行分发**；
- 官方优先通过 GitHub Releases、官方主页、Flathub、Homebrew Cask、WinGet、AUR 及 itch.io 进行原生桌面分发；
- 所有分发包均完整附带 `LICENSE`、`LEGAL.md`、`THIRD_PARTY_LICENSES.md`、`Cargo.lock` 及离线构建指南。
