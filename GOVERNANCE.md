# 夜半 (Yeban) 开源治理与决策机制 (Governance)

## 1. 治理哲学 (Governance Philosophy)

夜半 (Yeban DAW) 是一个致力于打造纯血原生桌面数字音频工作站的开源项目。项目采用 **BDFL (Benevolent Dictator for Life) + 核心维护者小组 (Core Maintainers)** 结合 **AI Agent 全自主驱动开发** 的现代化开源治理模型。

## 2. 角色与职责 (Roles & Responsibilities)

1. **核心维护团队 (Maintainers)**：
   - 负责把控项目整体架构演进方向；
   - 监督法律与合规安全（开源协议、CLAP 例外边界、依赖审计、商标与资产版权）；
   - 审核并签署重大版本发布与发布包签名；
   - 拥有对主分支（`main`）的最终合入审查权。
2. **AI Agent 自主开发者 (Autonomous AI Agents)**：
   - 依据需求规范 ID 与机器可执行契约自动生成工程代码；
   - 自动运行单元测试、模糊测试、基准打点与无头视觉回归测试；
   - 维护自动化质量门禁与测试覆盖率；
   - 严禁自行修改 `LICENSE`、`LEGAL.md`、`SECURITY.md`、`TRADEMARK.md` 等法务敏感文件。
3. **开源社区贡献者 (Contributors)**：
   - 提交 Issue 缺陷报告与功能倡议；
   - 提交遵循 DCO 1.1 协议的代码与音色资产 PR。

## 3. 决策流程与重大变更 (Decision Process & RFC)

涉及以下重大变更必须发起 RFC (Request for Comments) 并在 Maintainers 达成共识后实施：
1. 涉及底层核心数据模型（`yeban-model`）的不兼容破坏性变更；
2. 引入新的外部 C/C++ FFI 依赖或改变现有依赖许可证范围；
3. 修改或扩展 Yeban Intent API / 双 MCP 协议工具接口；
4. 任何对法律合规文本与分发策略的调整。

## 4. 安全响应流程 (Security Response)

涉及潜在安全漏洞（如跨进程共享内存溢出、Zip-Slip 路径穿越、解压炸弹拒绝服务攻击等），请遵循 [`SECURITY.md`](file:///home/crow/work/agy/review/SECURITY.md) 的独立私密披露流程，严禁直接在公开 Issue 中披露未修补的脆弱性。
