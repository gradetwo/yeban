# 夜半 (Yeban) 安全政策与漏洞披露规范 (Security Policy)

夜半 (Yeban) 数字音频工作站高度重视工程安全性，包括音频实时运行边界、跨进程共享内存（POSIX shm）沙盒隔离防护，以及防止恶意工程文件代码执行。

---

## 1. 支持的版本 (Supported Versions)

目前仅对处于活跃开发分支的最新主版本提供安全更新：

| 版本系列 | 状态 | 安全更新支持 |
| :--- | :--- | :--- |
| `0.3.x` (当前开发干线) | 活跃开发 | 支持 |
| `< 0.3.0` | 历史过渡版本 | 不再支持 |

---

## 2. 漏洞报告与披露流程 (Reporting a Vulnerability)

如果您在夜半 (Yeban) 中发现了安全漏洞，**请切勿通过公开的 GitHub Issue 进行汇报**。请遵循以下负责任的披露流程：

1. **报告渠道**：请发送加密邮件至 `security@yeban.audio`（或通过 GitHub Security Advisory 提交私密报告）；
2. **报告内容**：
   - 漏洞的详细描述与影响范围；
   - 能够稳定复现的 Minimal Reproducible Example（如特定的畸变 `.yeban` / `.sfz` 文件或 PoC 代码）；
   - 涉及的操作系统平台与硬件架构；
3. **响应时效**：
   - 安全小组将在 48 小时内确认收到报告并进行初步影响评估；
   - 确认存在漏洞后，将在专用私有修复分支中开发补丁，并在 14 天内完成修复并发布安全公告。

---

## 3. 核心安全边界与威胁模型 (Threat Model & Security Boundaries)

在评估安全缺陷时，夜半 (Yeban) 重点关注以下关键架构边界：

1. **跨进程插件沙盒与共享内存逃逸 (SHM Sandbox Escape)**：
   - 第三方 VST3 / CLAP 商业插件运行在隔离的子进程（`yeban-plugin-host`）中；
   - 宿主与插件仅通过 POSIX `shm_open` 或 Windows MMF 交换音频与 MIDI 环形帧；
   - 任何由子进程越界内存写入破坏宿主内存空间、或触发宿主进程任意代码执行的缺陷均被视为 **Critical 级漏洞**。

2. **音频文件与音色库解析安全 (Parser Hardening)**：
   - `crates/yeban-sfz`（SFZ 词法解析器）与 `crates/yeban-decode`（多格式音频解码器）必须具备防御恶意构造畸变文件的能力；
   - 持续接入 `cargo-fuzz`（LLVM libFuzzer）进行模糊测试，杜绝整数溢出、OOM 内存耗尽与越界访问。

3. **实时音频线程完整性 (Real-time Thread Integrity)**：
   - 实时音频回调由操作系统内核调度，通过无锁 SPSC 与主线程通信；
   - 任何可能诱发实时音频线程优先级反转（Priority Inversion）、死锁或持续 CPU 挂起的外部输入均属于高风险缺陷。
