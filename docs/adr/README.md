# ADR 目录

Architecture Decision Record。当 Normative 规范之间冲突、或规范对某个实现细节**没有**定义，而
必须在今天做出选择才能继续开发时，把选择、依据、代价写在这里。

规则（来自 `docs/skills/yeban-dev-workflow/SKILL.md`）：

- **不修改 Normative 文档来做裁决。** 规范是权威事实源，改动它是人类的责任；Agent 的裁决写在 ADR 里，
  并明确标注是 Proposed 还是 Accepted。
- **每条裁决必须写代价。** "取更严的一个"、"取更晚的一份规范"都要写清楚代价是什么。
- **裁决必须可被推翻。** 写清"待人类批准"的条目，人类改判后更新本文件而不是另开一份。
- **规范从未定义的东西也要记。** 例如 UI 字体栈、间距/圆角 scale、DPR 细则、Splitter 约束对象——
  这些在 `ADR-0001` 的"缺口"里登记，实现时按登记的口径来，不各写各的。

| 编号 | 标题 | 状态 |
| :--- | :--- | :--- |
| [ADR-0001](./ADR-0001-workspace-topology-and-version-pinning.md) | 工作区拓扑、命名与版本钉死 | Accepted（负责人已于 2026-10-04 授权"按建议执行"，全部裁决追认） |
| [ADR-0002](./ADR-0002-baseline-verdict-hardware.md) | BASELINE 系列的判定硬件（参考机 vs 托管 runner） | Proposed（待 HD-49）|
| [ADR-0003](./ADR-0003-save-button-top-bar-slot.md) | 顶栏保存按钮的槽位（挪徽章 vs 改规范序列） | Proposed（待人类裁决）|
| [ADR-0004](./ADR-0004-track-height-timeline-zoom-and-folding.md) | 编排视图的轨道高度、横向时间轴缩放与轨道折叠（每轨持久数据 vs 视图缩放级；文件夹实体 vs 视图语义） | Proposed（待人类裁决）|
| [ADR-0005](./ADR-0005-single-mutable-project-authority.md) | GUI 进程里唯一可变的工程权威（选项 (a) 的追认、投影注入面与 `.yeban.lock` 写者边界） | Proposed（待人类裁决）|
