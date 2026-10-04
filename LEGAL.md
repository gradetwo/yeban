# 夜半 (Yeban) 法律声明、商标与合规政策 (Legal, Trademarks & Compliance)

> **项目名称**：夜半 (Yeban) / Yeban DAW  
> **开源许可证**：GNU General Public License v3.0 (GPLv3) 带 GPLv3 §7 附加许可条款  
> **源码托管**：GitHub (开源项目)

---

## 1. 商标与非关联免责声明 (Trademark & Non-Affiliation Disclaimer)

1. **非关联声明**：  
   “夜半 (Yeban)”（以及曾用代号 Groove Lab）是一个独立的开源纯 Rust 数字音频工作站项目，与 **Ableton AG**（Ableton Live 商标持有者）、**Bitwig GmbH**（Bitwig Studio 商标持有者）、**Apple Inc.**（Logic Pro 商标持有者）、**Cockos Incorporated**（REAPER 商标持有者）、**Image-Line Software NV**（FL Studio 商标持有者）、**PreSonus Audio Electronics, Inc.**（Studio One 商标持有者）以及 **Steinberg Media Technologies GmbH**（VST / ASIO 商标持有者）**无任何官方关联、赞助、从属或背书关系**。

2. **商标权利归属**：  
   设计与技术文档中出现的所有第三方产品名称、商标、服务标记及注册商标（包括但不限于 Ableton Live, Bitwig Studio, Logic Pro, REAPER, FL Studio, Studio One, VST, ASIO 等）均为其各自法定所有者的财产。本文档中引用上述名称仅用于技术对比、接口兼容性描述与架构设计借鉴，不构成任何侵权故意或商业混淆。

---

## 2. Slint GUI 框架双授权与分发声明 (Slint Dual-Licensing Policy)

本项目用户界面层采用 **Slint GUI 框架**（由 SixtyFPS GmbH 开发维护）：
1. **Yeban 本身开源合规性**：  
   夜半 (Yeban) 项目整体采用 GPLv3 许可证开源分发，符合 Slint 的 GPLv3 开源授权条件，属于完全合法合规的开源链接。
2. **第三方 Fork 与商业化分发限制**：  
   根据 Slint 官方授权条款，任何基于夜半 (Yeban) 代码进行 Fork、二次修改，并希望以**闭源商业或非 GPLv3 兼容许可证**分发二进制产品的第三方主体，**无法继续免费享受 GPLv3 授权**，必须自行联系 SixtyFPS GmbH 获取 Slint 商业授权许可。夜半 (Yeban) 项目团队不提供 Slint 商业许可的转授、兜底或担保。

---

## 3. 音频插件格式合规性与附加许可 (Plugin Standards Compliance)

### 3.1 CLAP 格式 (CLever Audio Plug-in)
- **协议**：MIT 许可证。
- **GPLv3 §7 附加许可**：依据项目根目录 `LICENSE` 中的附加许可条款，夜半 (Yeban) 明确允许宿主通过 `clack` 框架动态加载、组合专有或商业 CLAP 插件，无需强制将第三方插件开源为 GPLv3。
- **限制**：此例外绝不允许专有第三方插件直接静态链接或内嵌夜半 (Yeban) 的核心引擎代码（`yeban-model`, `yeban-dsp`, `yeban-theory` 等）。

### 3.2 VST3 格式
- **协议状态**：Steinberg 已于 2025 年 10 月将 VST3 SDK（自 3.8.0 版本起）正式切换为 **MIT 许可证**。
- **合规锁定**：夜半 (Yeban) 严格锁定最低依赖版本为 VST3 SDK 3.8.0+，使用 MIT 许可路径消除历史双许可复杂性。
- **ASIO 注意事项**：ASIO 接口仍受 Steinberg 单独协议约束（虽已扩展支持 GPLv3 应用程序），在 Linux/macOS 默认优先使用系统原生驱动（ALSA/PipeWire/CoreAudio），在 Windows 优先推荐 WASAPI Exclusive 驱动，ASIO 驱动支持由外部隔离模块按需加载。

---

## 4. 采样资产与音源知识产权规范 (Audio Sample Assets)

夜半 (Yeban) 内置与随附分发的所有音频采样库（323 款原声乐器图谱）均遵循严格的知识产权白名单：
- 仅收录 CC0 1.0 (Public Domain)、CC-BY 4.0 或 MIT/Apache-2.0 授权的无争议开源高质量采样；
- 严禁收录未经授权的商业商业音色库切片（严禁使用 Kontakt 商业库切片、商业采样包）；
- 完整资产归属与来源清单请查阅 [`assets/samples/ATTRIBUTION.md`](file:///home/crow/work/agy/review/assets/samples/ATTRIBUTION.md)。
