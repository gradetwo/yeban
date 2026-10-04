# ADR-0001 — 工作区拓扑、命名与版本钉死

- **状态**: **Accepted（负责人已于 2026-10-04 授权"按建议执行"，全部裁决追认）**
  （在此之前的状态是 `Proposed`：Agent 一律照此执行，但保留人类否决权；现在否决权未行使、追认已给出。
  逐条结论与建议列见 `docs/ledger/human-decisions.md`。）
- **日期**: 2026-10-05
- **依据**: `AGENTS.md` §1/§2/§4、`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §8、`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §3
- **背景**: 四份 Normative 规范之间以及规范与现实之间存在若干必须裁决的冲突与缺口。本 ADR 把裁决、依据与代价一次写清，避免每条工作线各自发明一套答案。

---

## 裁决一览

| # | 问题 | 裁决 | 依据与代价 |
| :-- | :--- | :--- | :--- |
| D1 | `yeban-ui-test-port` / `yeban-ui-mcp` 在 architecture §8 的 workspace 树里**没有**，但 §0.4/§12 反复要求 | **列为正式 workspace 成员** `crates/yeban-ui-test-port`、`crates/yeban-ui-mcp` | 取"功能需求优先于目录清单"：§12 的 DoD（UI 变更双重验证）没有这两个 crate 无法闭环。代价：crate 数比 §8 多两个 |
| D2 | `yeban-app/ui/` 文件清单两处不一致：architecture §8 给 4 个文件，UI/UX §8 给 11 个 | **以 UI/UX §8 的 11 个 `.slint` 为准**，architecture 的 4 个名字被包含关系取代（`pianoroll.slint`→`console/piano_roll.slint`，`mixer.slint`→`console/mixer_console.slint`）；24px 状态栏新增 `status_bar.slint` | UI/UX 规范更晚、更细、且是 UI 领域的专门规范。代价：实现时不得再出现根目录级的 `pianoroll.slint` |
| D3 | `schema_version` 字面值：规范注释写"固定为 3"，类型名却是 `YebanProjectV1` | **首个稳定文档 schema 版本 = `1`**；`writer_version` 取应用语义版本字符串（当前 `0.0.1`） | 规范正文的 v1.0.0-rev1/rev2 已把 Groove v3 编号**整体重基**为 `v0.0.1` 起步/`v1.0.0` 首发；而 `3` 是重基前的遗留数字。且历史上**从未发布过** `schema_version = 2/3` 的文档，所以"迁移兼容"没有对象。代价：若确实存在外部 v3 文档，需补一个迁移器（`ROAD-M1-005` 已预留 `src/migration/`） |
| D4 | 性能衰退阈值冲突：roadmap §5.3/§6.4 写 5%，`AGENTS.md` DoD 4 写 3% | **合并阻断用 3%**（更严者胜）；5% 只作为发布列车的容忍带 | `AGENTS.md` 是 Agent 的执行契约，DoD 明文写 3%；规范冲突时取严不取宽，且不需要修改任何 Normative 文档 |
| D5 | 依赖版本未钉死（slint / cpal / clack / zip / hound …） | **唯一事实源 = 根 `Cargo.toml` 的 `[workspace.dependencies]`**，成员一律 `foo.workspace = true`；`i-slint-backend-testing` 用 `=x.y.z` 与 `slint` 严格同版本 | 版本决策集中一处，任何一条工作线都不需要为了改版本去动共享文件（多线并行的关键）。测量记录见 `docs/DEVELOPMENT_LEDGER.md` |
| D6 | `ulid` 3.0.0 没有 `serde` feature，且 API 是 `Ulid::generate()` 而非规范写的 `Ulid::new()` | `EntityId` 的 `Serialize/Deserialize` **手写**，直接满足"26 字符 Crockford Base32 + 大小写不敏感"；`EntityId::new()` 内部调 `Ulid::generate()` | 规范要求的是**行为**（26 字符、大小写不敏感），不是某个 crate 的 API 形状。把行为固定在自己的类型上，上游 API 变动不再影响工程文件格式。代价：`ulid` 升级时只需重新验证 `ids.rs` 的测试 |
| D7 | 基准性能矩阵要求固定频率参考机（M2 Pro 12 核 / Ryzen 7840HS），但 GitHub 托管 runner 频率不固定 | **正确性门禁**全跑 GitHub 托管 runner；**BASELINE-001..006 与 L1/L2 确定性对账**标记 `PENDING`，等自托管固定频率 runner 就位 | 用托管 runner 的读数去判定基准达标是自欺（SKILL「本地绿不是绿」的同类错误）。在没有合格硬件之前，这些门禁**明确不通过**而不是假装通过 |
| D8 | `ROAD-M-1-006` 正文列 4 条人类审核项，修订记录说"三项" | 按正文的 **4 条**执行（法务 GPLv3 §7 措辞、ASIO 法务确认、商标查重、BDFL 发布签名） | 正文比修订摘要详细，且 4 条都真实存在。代价：发布前的人类工作量按 4 条计 |
| D9 | Phase 0 的 9 个 Spike 放哪 | 独立 `spikes/spike-0N-*/` 成员 crate，**各自一个目录，永不共享文件** | 9 条工作线可以真正并行（SKILL：一个文件只能有一个写者）。代价：spike 代码不进产品 crate，验证结论需被"汲取"进 `crates/` 后 spike 才能退役 |

### D10（补充）— `schemas/*.json` 与规范冲突时，以**规范**为准，改契约

`schemas/` 是机器校验契约，它必须描述我们真正要的东西。当契约与规范/ADR 冲突时：

1. **收紧**契约到规范要求的形状（不是放宽成 union 去兼容一个从未存在过的历史格式）；
2. 在契约里留下指向裁决的注释或 description；
3. 把冲突本身记进本 ADR（不许悄悄改一边）。

### D11 — `writer_version` 是语义版本字符串，不是整数

- **冲突**：`schemas/project.schema.json` 原本声明 `writer_version: integer`；架构 §2.2 与 D3 裁决它是
  形如 `"0.0.1"` 的**字符串**。`yeban-model` 的规范样本对账实测：只有这一项红，其余 required/pattern/range/enum 全过。
- **裁决**：契约改为 `type: string` + semver 形状的 `pattern`（**不是** `["integer","string"]`）。
- **代价**：若真有外部整数版本号文档，需要迁移器（`ROAD-M1-005` 预留了 `src/migration/`）。

### D12 — `Op` 全集补两个删除变体：`RemoveSection` / `RemoveScene`

- **缺口**：架构 §6.1 用 `SetSection { old_section: Option<_> }` 表达"新建"（`None`），但全集中**没有**
  删除段落/场景的变体 ⇒ 新建这一步的逆操作无法表达 ⇒ `MUST-GATE-010`（10,000 步撤销守恒）必然失败。
- **裁决**：补 `Op::RemoveSection` / `Op::RemoveScene`，语义完全由 `SetSection` 的逆定义（无自由度）。
- **同步**：`schemas/ops.schema.json` 的 `op.oneOf` 由 14 个变体补齐到 **23 个**（覆盖架构 §6.1 全集 + 这两个）。
- **待人类批准**：这是对规范 Op 全集的**扩展**，需要回写进架构 §6.1。Agent 不擅自改规范正文。

### D13 — `OpOrigin::McpProposal` 用**外部标签对象**序列化

- **冲突**：`schemas/ops.schema.json` 原本把 `origin` 声明为 7 值字符串 enum，但 `McpProposal` 携带
  `{ proposal_id, agent_name }` 结构化载荷。
- **裁决**：`origin` 的契约改为 `oneOf`：6 个单元变体仍是纯字符串（`"UserUi"` …），
  `McpProposal` 序列化为外部标签对象 `{"McpProposal":{"proposal_id":"…","agent_name":"…"}}`。
  这既保留了 serde 的默认（externally tagged）行为，也让契约与实现一致。
- **代价**：消费方必须按 `oneOf` 处理两种形状；换来的是不丢载荷。

### D14 — 流派规则库的数量以**实测值**为准，规范正文的"159 种"视为下限

- **冲突**：规范正文多处写"159 种流派规则"，而 `yeban-theory` 实际落地 **182 条**（全部自行编码、按来源分类登记）。
- **裁决**：数量由 crate 内的机械判据钉住（`library_size_is_pinned_to_the_measured_number`），
  规范里的"159"读作"**至少** 159 种"；实际数字记入 `docs/DEVELOPMENT_LEDGER.md` 与
  `docs/ledger/theory-core-notes.md`。**不修改规范正文**。
- **代价**：对外文案不得再写死任何数字（`Cargo.toml` 的 description 已改为不带数字的"流派规则库"）。

### D15 — `PPQ = 960` 允许在两个 crate 各有一份，但必须被对账

- **现状**：`yeban-model` 与 `yeban-theory` 各定义一份同值常量；为这一个整数新增
  `theory → model` 的依赖不划算（会让纯函数层依赖整个数据模型）。
- **裁决**：允许重复，但**必须**有独立对账：集成属性测试以"4/4 一小节 = 3840 tick"验证两者一致。
  日后再出现第三份时必须同样对账，否则合并为单一来源。

### D16 — 循环点微平滑窗以**规范公式**为准（π 版），不是对称 Hann 窗

- **冲突**：规范 §3.3 给的是 `w(n)=½[1−cos(πn/(N−1))]`（`w(0)=0, w(N−1)=1`，N=64 时 `w(32)≈0.5125`），
  而早期任务书里的示例判据 `w[0]==w[N−1]==0, w[N/2]≈1` 描述的是 2π 对称 Hann 窗。
- **裁决**：**以 Normative 规范为准**（π 版），"首尾相接无跳变"由**互补窗对**实现
  （fade-in 用 `w`，fade-out 用 `w` 的镜像）。差异已记入 `docs/ledger/dsp-core-provenance.md` §5.1。
- **待人类确认**：若产品意图是对称 Hann 窗，需要改规范 §3.3 —— 那属于人类对规范的修改，Agent 不代改。

### D17 — 依赖许可白名单接纳 `BSL-1.0`（**Accepted 2026-10-04**）

- **背景**：引入 Slint 后，`clipboard-win` / `error-code`（← `arboard` ← winit/slint，Windows 目标）
  声明 `BSL-1.0`（Boost Software License 1.0），被 `cargo deny` 拒绝。
- **裁决**：加入白名单。判定依据：BSL-1.0 是 OSI 认证 + FSF Free/Libre + 宽松 + 与 GPLv3 兼容，
  **不属于** AGENTS.md §2 红线 2 禁止的三类（非商业限制 / 专有不可再分发 / 不兼容 GPLv3）。
- **诚实声明**：提出该修改的是 Agent（集成者）。"接纳一个新许可"在精神上属于 `ROAD-M-1-006`
  的人类判断范畴，因此本裁决最初状态为 `Proposed`（**已于 2026-10-04 追认**；若未来否决，回滚 `deny.toml` 中该行即可），
  并改由 CI 安装 `libfontconfig`/改用其它剪贴板方案来绕行。
- **同类前置**：Slint 在 Linux 上还需要系统 `fontconfig` 开发库（见 `docs/CI_CD.md` §3.2），
  已由 CI 统一安装，而不是在各 crate 里加 feature 垫片。

### D18 — 规范里的 Slint 无头/测试 API **与上游 1.18.1 不符**，以上游为准并自研兜底

`line/ui-shell` 逐条核对了上游文档与源码（记录在 `docs/ledger/ui-shell-notes.md` §2），发现三处规范与现实的偏差：

| 规范写法 | 上游 1.18.1 现实 | 本文裁决 |
| :--- | :--- | :--- |
| `SLINT_BACKEND=headless`（`ARCH-UI-003`、UI/UX §12.1） | **不存在**。上游只接受 `qt` / `winit` / `linuxkms`（可加 `-software` / `-skia` / `-vello` 后缀） | `--headless` 作为**夜半自研哨兵值**：不构造窗口、不初始化后端、不进事件循环，打印握手后退出 0。它只证明"无显示器环境能跑起来"，**不证明控件树正确**。规范措辞需人类修订。 |
| `slint::testing::init_integration_test_backend()` / `send_mouse_click()` / `send_keyboard_char()`（`ARCH-UI-005`） | **路径不存在**。实际是 `i-slint-backend-testing` 的 `ElementHandle`（`find_by_element_id` / `mock_single_click` / `mock_drag` / `query_descendants` …），且它**不渲染像素** | 控件树断言走 `i-slint-backend-testing` + `ElementHandle`；**截图**必须走 `slint::platform::Platform` + `SoftwareRenderer`（`MUST-GATE-015`）。两条路径分别由 `yeban-ui-test-port` 承载。 |
| `renderer-skia` 软件后端（UI/UX §12.1） | 需要 LLVM/clang 工具链 | 暂缓：用可移植的 `renderer-software` 兜底；启用 Skia 需人类评估（编译成本 vs 渲染一致性），列 PENDING。 |

- **附带好消息**：Slint 1.18 **原生提供 `accessible-id`**（官方定位即"用于自动化与测试识别控件"），
  因此 UI/UX §12.2 的语义 ID 约定（`track-{i}-fader` / `note-{ulid}-rect` / `clip-{ulid}-header` / `tab-{name}-button`）
  可以直接落在 `.slint` 上，而不必自建映射。它只在有窗口实例时存在，所以 `yeban-app` 另外维护一份
  纯 Rust 注册表（184 条 + 14 个动态遮罩区），并用**双向覆盖判据**把两者钉在一起（`.slint` 与注册表任一侧漂移即变红）。
- **代价**：在 `yeban-ui-test-port` 落地前，"UI 变更必须双重验证"（DoD 6）无法闭环 ——
  当前界面只是**编译通过**，从未被渲染器或人眼看过。

### D19 — 实时引擎与离线渲染**共用 PDC 算法**，但离线侧不得被迫拖入 cpal

- **问题**（由 `line/render-master` 实测暴露）：`ARCH-PDC-001/002` 要求"实时引擎与 Rayon 离线渲染
  共用同一 PDC 算法"。但该算法的天然宿主是 `yeban-engine`，而 `yeban-engine` 依赖 `cpal`；
  若 `yeban-render` 直接依赖 `yeban-engine`，一个纯离线渲染器就会被迫编译声卡驱动栈。
- **裁决**：
  1. `yeban-engine` 把**拓扑排序 + 关键路径延迟 + 环形延迟线**做成**不引用 cpal 的公共模块**
     （模块内不得出现设备 I/O；这样它可以被无 cpal 的构建消费）；
  2. `yeban-engine` 用 cargo feature 把设备 I/O 隔开：`default = ["device"]`，`device = ["dep:cpal"]`；
  3. `yeban-render` 以 `yeban-engine = { workspace = true, default-features = false }` 依赖它，
     于是离线渲染拿到同一份算法而不编译 cpal。
- **过渡期**：在 engine 提供该模块之前，`yeban-render` 自带一份**最小同构实现**（纯函数、零依赖），
  并配"等价性判据"防漂移；engine 落地后**必须把 render 侧那份退役**改为复用
  （登记在 `docs/ledger/render-master-notes.md` 的 needs 里，集成者负责在合并顺序上保证这一点）。
- **否决的备选**：把 PDC 放进 `yeban-model`（那是数据模型，放图算法会让"模型"变成杂物间）；
  新建 `yeban-graph` crate（为约 200 行代码多一个 crate，等出现第三个消费者再考虑）。
- **前提修正**：`ARCH-PDC-001` 明写 `DeviceDefinition::latency_samples`，但该字段此前**并不存在**。
  已按规范补上（`yeban-model`，`#[serde(default)]` 取 0 表示"未上报"，填充样本里给 32 采样点做覆盖）。

### D20 — 依赖许可白名单接纳 `Unlicense`（**Accepted 2026-10-04**）

- **背景**：`midly`（规范 §4 指定的 SMF 0/1 零堆分配编解码库）**只有** `Unlicense`，
  不像多数 crate 那样带 `MIT/Apache` 兜底分支，因此被 `cargo deny` 逐条拒绝。
- **裁决**：加入白名单。判定依据：Unlicense 是 OSI 认证 + FSF Free/Libre + 公有领域等效奉献 +
  与 GPLv3 兼容（无任何额外限制），不属于 AGENTS.md §2 红线 2 禁止的三类。
- **考虑过的替代**：自研 SMF 编解码器（约 180 行，可让 `yeban-render` 的 MIDI 模块零第三方依赖）。
  **否决**：SMF 的运行状态、VLQ 边界、tempo map 语义是"领域早已定型"的东西，
  自研等于把一个小问题变成长期维护问题（SKILL「不要重新发明已定型的东西」）。
- **诚实声明**：与 D17 一样，这是 Agent（集成者）提出的白名单扩张，**已于 2026-10-04 由负责人追认**；
  否决时改为自研 SMF 编解码器。

### D21 — 内部 path 依赖豁免通配检查（否则每条线都撞同一堵墙）

- **问题**：`[bans] wildcards = "deny"` 会拒绝"成员依赖另一个成员"——因为
  `[workspace.dependencies]` 里的内部条目只写 `path` 不写 `version`，继承出来的要求是 `*`。
  实测（隔离沙盒复现）：`yeban-render → yeban-model/dsp` 与 `yeban-app → yeban-ui-test-port`
  两条线**同时**被这一条拦下，而这是 Phase 2/3 几乎每条线都要写的依赖形态。
- **裁决**：`[bans] allow-wildcard-paths = true`。**只**放行带 `path` 的 `*`；registry 依赖仍必须写真实版本。
- **否决定的替代**：给内部条目补 `version = "0.0.1"` —— 否决，因为 `^0.0.1` 不匹配 `0.1.0`，
  工作区每次升版都要回头改一堆内部依赖，属于把一次性麻烦换成长期麻烦。
  （成员全部 `publish = false`，不存在"发布时需要版本"的诉求。）

### D22 — UI 内省能力必须由**构建期**打开（`with_debug_info(true)`），不是环境变量的运气

- **实测发现**（`line/ui-test-port`，证据在 `docs/ledger/ui-test-port-notes.md` §3 第 5 条）：
  Slint 的 `ElementHandle` 遍历依赖**编译期生成的 debug info**，而上游默认值
  （`i-slint-compiler-1.18.1/lib.rs:282`）是"环境变量 `SLINT_EMIT_DEBUG_INFO` 存在才开"——
  **默认关闭**。没有它时 `element_count()` 返回 `None`：**运行时控件树恒为空，且不报错**。
- **裁决**：`crates/yeban-app/build.rs` 与 `crates/yeban-ui-test-port/build.rs` 都必须用
  `slint_build::compile_with_config(..., CompilerConfiguration::new().with_debug_info(true))`，
  **不依赖任何环境变量**。CI 的 workspace 腿显式运行
  `cargo test -p yeban-app --features ui-test-port`（该测试带 `required-features`，默认不跑）。
- **代价（明说）**：生成代码带元素树元数据 ⇒ 二进制略大、编译略慢。
  这是 AGENTS.md §3 DoD 6（UI 变更必须双重验证）的必要成本，不是可选装饰。
- **教训**：一个"恒为空且不报错"的能力等于没有能力，而且比没有更坏 —— 它会让断言以"0 == 0"通过。
  这与 L12（本机门禁空跑）是同一族：**判据必须能失败，能力必须能证伪**。

### D23 — `SSIM ≥ 0.98` 的语义限定：它**不是**"任何看得见的差异都会红"

- **实测量化**（同一条线，用仓库里真实的 SSIM 实现）：

  | 变化 | 面积 | 未遮罩 SSIM |
  | :--- | ---: | ---: |
  | 8×88 窄条换纯色 | 2.9% | **0.9999**（拉不下 0.98） |
  | 60×88 大块换**等亮度**平色 | 22% | **0.9967**（拉不下 0.98） |
  | 同一大块**掉到黑** | 22% | **0.7040** ✓ 被检出 |
  | 抹掉 64×40 平色块 | 10.7% | **0.8481** ✓ 被检出 |

- **裁决**：UI/UX §12.5 那句"关键静态视觉缺陷 100% 灵敏检出"必须限定为
  **"带来足够局部亮度/对比度改变的变化"**。SSIM 是局部统计的均值，对细长条与等亮度换色几乎免疫。
  因此：**"必须被检出"的判据只挑"成块 + 亮度差大"的变化**，并配一条前置断言（该动态区必须 ≥10% 画面），
  防止后来者用"缩小区域"的方式悄悄削弱判据。规范若需要"任何可见差异都红"，**需要第二判据**
  （例如像素差占比阈值）—— 本 ADR 不改规范，只登记这个缺口，等人类决定。
- **诚实声明**：这条限定来自真实量化，不是推测；但它只在一个夹具尺寸/配色下测过，
  换配色或换尺寸可能需要重新量化。

### D24 — 界面字体：用系统字体栈 + **分平台** Golden，**不**为 UI 捆绑 CJK 字体

- **规范的两处相互拉扯**：UI/UX §12.5 要求"Golden 图按 OS 独立维护（Linux FreeType / macOS CoreText /
  Windows DirectWrite，严禁混用）"，而同一节的 CI 要求里又写"CI 需打包 Noto Sans CJK 字体并配置字体降级链"。
  两者不能同时成立：要么字体确定（可以跨平台比图），要么图分平台（字体可以不确定）。
- **另外两个硬约束**：
  1. 交付指标要求**单二进制 < 25MB**，而一份 CJK 字体动辄 10MB+ ⇒ 为 UI 捆绑 CJK 字体直接威胁该指标；
  2. `line/ui-test-port` 实测：Tier-1 光栅化在**同一字体环境**下是确定的（两次不同 runner 的截图指纹一致），
     但字体环境一变，截图必然变 ⇒ 跨平台比图本来就不成立。
- **裁决**：**不捆绑 CJK 字体**。
  · 应用侧：使用系统字体栈（`crates/yeban-app/ui/tokens.slint` 里写的是
    `PingFang SC` / `Microsoft YaHei` / `Noto Sans CJK SC` 这类**列表**）—— 不占二进制预算。
    ⚠ **2026-10-05 修正（`line/app-introspect` 的上游实测）**：Slint 1.18.1 的 Parley 路径**不把
    `font-family` 的逗号列表当回退链** —— 整串被当成**一个** family 名，随后只回退到
    `SansSerif`/`SystemUi` 两个泛型家族（依据：`sharedparley/shaping.rs:86-105`）。
    因此 `tokens.slint` 里那条"链"**不是链**：汉字能否渲染，完全取决于**系统里是否存在覆盖 CJK 的字体**
    （fontconfig / CoreText 决定），而不是取决于我们写了几个名字。
    这不改变 D24 的结论（不捆绑字体），但**改变了理由**：我们不是"配置了降级链"，而是"依赖系统字体覆盖"。
    规范 §12.5 里"配置字体降级链"的措辞同样需要修订。CI 因此**必须**装 `fonts-noto-cjk`（已装），
    否则"稳定地渲染成豆腐块"。该线实测：无字体时汉字区域墨迹 **24 px**（几乎什么都没画），
    有字体后同一区域 **648 px**（27×），并已把 `cjk_ink >= 150` 做成判据。
  · 测试侧：**分平台**维护 Golden 基准（`tests/golden/<platform>/`），跨平台只比较**结构性判据**
    （控件树 JSON、几何比例、颜色 token），不比较像素；
  · **明确不采纳** `i-slint-backend-testing` 的内部 `configure_test_fonts()`：它是内部 feature，
    会把字体换成内嵌 NotoSans，从而让"生产渲染"与"测试渲染"字体不一致 —— 那会让 Golden 失去代表性。
- **代价（明说）**：跨平台像素级回归不可比；`RSK-17`（CI 缺 CJK 字体）从"风险"变成"已接受的限制"，
  但仍需保证 CI 环境有**某种** CJK 字体，否则中文会渲染成豆腐块而 Golden 反而"稳定地错"。
  因此 CI 仍需安装 `fonts-noto-cjk`（运行时依赖，不是仓库资产），并在 UI 判据里断言"中文字形非 tofu"
  （可用字符包围盒非零或与已知 tofu 图样比对）。这一条登记为待接线判据。

### D25 — `mcp-tools.schema.json` 的两处硬缺陷：错误码集合不全 + 根不引用 definitions

`line/mcp-core` 在实现 MCP 工具层时逐条比对契约与架构 §7.2，发现两处（均已在 main 修掉）：

1. **错误码集合不全**：schema 的 `ToolResponse.error.code` 是 **7 值闭合 enum**，而架构 §7.2 逐工具列出的并集是
   **16 个**，交集只有 3 个 ⇒ 13 个领域错误码（`FILE_NOT_FOUND` / `DISK_FULL` / `CLIP_NOT_FOUND` / `CONFLICT` …）
   **没有家**：任何真实的领域失败都会产出被契约判为非法的响应。
   · **裁决**：取**两集合的联集**（20 个值），既有 7 个一个不删（`PERMISSION_DENIED` 还是 scope 强制的必需码），
     规范并集一个不缺。**扩展 enum 属契约变更，状态已由负责人追认（2026-10-04）**（与 D17/D20 同一处理方式）。
2. **根不引用 `definitions`**：根只有 `$schema/$id/title/description/type/definitions`，
   于是 `validate_schemas.py --samples-dir` 在本 schema 上**是空转的** —— 实测
   `{"anything":[1,2,3]}` 与 `{"name":"完全不在枚举里的工具"}` **都能通过根校验**。
   · **裁决**：根改为 `oneOf($ref ToolCall, $ref ToolResponse)`（去掉裸 `type: object`）。
     实测修复后上述两个垃圾样本都被拒。这让契约从"定义了一堆没人用的类型"变成**承重**的。

**方法论留痕**（值得后续线照做）：该线没有直接改 `schemas/**`（权威契约），而是
① 两套集合都实现、② 把缺口写进**机器可读样本**、③ 把缺口钉成"实测常量"判据、
④ 另加一条判据在"根开始引用 definitions"的那一刻变红，提醒升级口径。
集成者这次修复会**故意**触发那条提醒判据 —— 那是它按设计工作，不是回归。

### D26 — `symphonia 0.6` / `rubato 5.0` 的实际 API 与规范引用的版本**不一致**

`line/decode-core` 按"先核验再写"的纪律读了 docs.rs 与本地 registry 里的**真实源码**，
发现规范点名的 API 在钉死的版本里并不存在（与 D18/D22 同一族：**规范引用了旧版上游**）：

| 规范/直觉里的名字 | 0.6.1 / 5.0.1 的实际形态 |
| :--- | :--- |
| `rubato::SincFixedIn` / `FftFixedIn` / `FastFixedIn` | **不存在**。实际是 `Async::new_sinc` / `Async::new_poly`、`Fft`（需 `fft_resampler` feature）、`Slip` |
| `Resampler::process` 的"输出帧数" | `process_all_into_buffer` 返回 `(usize, usize)`，第二个是 `ceil(ratio × input_len)` 而**不是**实际写入帧数（源码 `lib.rs:330/398`）⇒ 必须按 expected 帧取有效区 |
| `symphonia` 的 `AudioBufferRef` | 0.6 改名为 `GenericAudioBufferRef` |
| `symphonia` 的 `next_packet` | EOF 是 **`Ok(None)`**（不是 `Err(UnexpectedEof)`） |
| `MediaSource` 的 `Read + Seek` | **没有** blanket impl；只有 `File` 与 `Cursor`（内存/自定义源要走 `Cursor`） |

- **裁决**：以**实际版本**为准实现（已落地），并把差异写成 ADR + 台账；规范的措辞需要人类按此修订
  （规范修订属人类职责，Agent 不擅改 `docs/YEBAN_*.md`）。
- **证据出处（逐条 URL 与确切签名）**：见 `docs/ledger/decode-core-notes.md` §2.1（symphonia）与 §2.2（rubato），
  以及 §7.3/§7.4 的 CI 读数与该线 4 次注入变红的原始记录。本 ADR 只记结论，不复制 URL 清单。
- **顺带两个真实的"挂死"坑**（该线逐行读上游源码发现，比断言失败值钱得多）：
  1. **截断的 RIFF/WAVE 会让解封装器"不报错也不推进"** —— `read_boxed_slice` 对 EOF 是**截短返回**，
     而边界用的是**声明**长度 ⇒ 解码循环会永久空转，CI 直接挂到 60 分钟超时。
     该线因此加了 `IdleGuard`（连续 N 个包不推进就报错），并有判据。
  2. **RIFF 奇数长度块必须补一个不计入块长度的填充字节**，否则 `ChunksReader` 在末尾吃 `UnexpectedEof`
     —— 表现为"两个位深测试莫名失败"。8-bit / 24-bit 夹具极易凑出奇数长度。

### D27 — 扩 `Op` 全集：补 `AddClip`/`RemoveClip` 与 `AddRoutingNode`/`RemoveRoutingNode`

- **问题（`line/tools-domain` 实测发现）**：规范 §7.2 要求 `yeban_propose_section` 产出"声部连接 + 配器骨架"，
  但在本裁决之前，`Op` 全集**表达不出来**：
  · `AddClipPlacement` 的前置条件是"片段**已经在** `clip_pool` 里"，而**没有任何变体能把条目放进池子**；
  · `ConnectRouting` 的前置条件是"两端**已经在** `routing_graph.nodes` 里"，同样**没有任何变体能把节点放进去**。
  于是那条工具只能返回 `data.unwired = ["clipPoolEntries","routingEdges"]`。这是**规范要求的能力在操作日志层缺失**，
  与 D12（补 `RemoveSection`/`RemoveScene`）同一族。
- **裁决**：`Op` 从 23 个变体扩到 **27** 个：
  · `AddClip { clip }` / `RemoveClip { clip_id, previous_clip }`（后者前置：片段存在且**无摆放引用** → `ClipInUse`）；
  · `AddRoutingNode { node }` / `RemoveRoutingNode { node }`（后者前置：节点存在且**无边引用** → `RoutingNodeInUse`）。
  · 节点按**字典序**插入，因此 `nodes` 恒有序 ⇒ 增删互为逆操作且**不需要额外载荷**（可逐字节还原）。
  · 新增 `ModelError::ClipInUse` / `ModelError::RoutingNodeInUse` 两个具体错误（不用笼统的 `OpStateMismatch`）。
  同步扩展 `schemas/ops.schema.json` 的 `op.oneOf`（23 → 27 个分支）。
- **顺带修掉一条判据的盲区（值得单列）**：原有的
  `op_variants_match_ops_schema_exactly` 比较的是 **`showcase_ops()` 与契约**。
  于是"给 `Op` 加了变体、但既没加进 `showcase_ops()` 也没加进契约"这种情况**两边都看不见、判据全绿** ——
  而它正是这次漂移的形态（`Op` 缺 4 个变体而所有判据都是绿的）。
  新增判据 `every_op_variant_is_declared_in_the_contract`：利用 `name()` 的 `match self` **是穷举的**
  （少一个变体就编译不过）这一事实，从源码抽取 `Self::<Variant>` 作为**枚举全集**，与契约做双向断言。
  实测：从契约删掉一个分支 → 判据立刻红并指名 `只在枚举里而契约缺失: ["AddRoutingNode"]`。
- **代价**：`Op` 是持久化契约的一部分 ⇒ 老文档不受影响（新变体只是**新增**可表达的操作）；
  但**任何按 23 个变体写死的外部实现都要跟到 27**（本仓库的 `samples.rs` 里那处计数断言已经因此红过一次，
  按"改了计数就要重跑派生该计数的判据"的既有纪律修正）。

### D28 — 界面数据链路：**纯函数投影层 + 唯一注入点 + 整数位置**

`line/app-binding` 把界面从"渲染演示常量"改成"由 `YebanProjectV1` 驱动"。三条裁决固化下来：

1. **投影层零 Slint 依赖**（`crates/yeban-app/src/bridge.rs`）：`YebanProjectV1 → ViewState` 是**纯函数**。
   好处是实测到的：该线的 31 条判据能在**本机**用 `rustc --edition 2024 --test` 真跑（Slint 本机禁编译），
   而"模型字段 → 视图字段"的映射错误因此在推 CI 之前就被抓到了。**这条与 `ui-test-port`/`render`/`decode`
   三条线选择"把纯计算部分做成零重依赖模块"是同一个模式，值得作为默认做法。**
2. **位置一律 960 PPQ 整数运算**：`tick_to_px = tick / ticks_per_pixel`（整数除法）、`px_to_tick` 用 `checked_mul`，
   越界返回 `Err`。理由不只是确定性：`ticks_per_pixel = 30` **不是 2 的幂**，
   任何浮点实现在 `tick_to_px(30, 30)` 上都会算出 `0` —— 那是会直接毁掉吸附的错。
3. **注入实现只能有一份**（`crates/yeban-app/src/host.rs`）：`main.rs` 与判据**共用**它。
   两份注入 = 两份会漂移的事实源；判据若走自己那份，"测过的"就不是"发布的"。

### D29 — UI 控制面的方法名与 scope 归属（工程裁决 + 一处待人类裁决）

- **方法名**（规范未给 JSON-RPC 方法名）：`ui/methods`、`ui/tree`、`ui/node`、`ui/property`、
  `ui/dynamic_regions`、`ui/screenshot`、`ui/coverage`，以及**逐字采用** §12.4 函数名的
  `ui/dispatch_pointer_down|move|up`、`ui/dispatch_key_press`，加管理类 `ui/switch_main_view`/`ui/force_save`/`ui/reload_engine`。
  出处逐条记在 `docs/ledger/ui-mcp-notes.md` §2。
- **补齐了一处规范给出的"能力"而契约没给"码"的缺口**：`-32006 ELEMENT_NOT_FOUND`、`-32008 CAPTURE_FAILED`、
  `-32009 GEOMETRY_UNAVAILABLE` 作为**JSON-RPC 错误码**新增，**不塞进** `ToolResponse.error.code`
  （那是工具级领域码，D25 已裁定其集合）。两者是不同的层，混用会让 D25 的联集变成垃圾桶。
- **待人类裁决**：六级 scope（`ui:read`/`ui:screenshot`/`ui:inject`/`app:save`/`app:reload-engine`/`app:admin`）
  里**没有"界面状态写"**这一级，于是 `ui/switch_main_view` 暂挂 `app:admin`（本地裁决、已登记）。
  另外端口三级 `Permission` 与网络六级 `Scope` 是否需要合并，也一并待裁决。

### D30 — `.yeban` 归档容器：手写**最小 ZIP 子集**（零依赖），并用"拒绝"消除读法歧义

- **裁决**：**不引入 `zip` 也不引入 `flate2`**，在 `yeban-model` 里手写最小 ZIP 读写
  （`src/container/{mod,zip,path,crc32}.rs`）。理由：① 红线 9 与依赖政策要求新增依赖逐条裁决；
  ② 容器格式本身**非常简单**，真正的难点是**防御**而不是编解码；③ 零依赖让 `yeban-model`
  保持本机可编译 ⇒ 84 条对抗性判据能在本机真跑（这条收益是实测的）。
- **写入**：只写 `stored`(method 0)，但产出的是**标准 ZIP**（本机用 `unzip 6.00` 实测可列可读）。
  资产池里是已压缩音频，deflate 收益本就很小。
- **读取**：不支持的东西**一律明确报错**（`UnsupportedCompression` / `UnsupportedZip64` /
  `EncryptedEntryUnsupported` / `UnsupportedDataDescriptor` / `UnsupportedMultiDisk` / `EntryNameNotUtf8`），
  **绝不静默跳过或猜**。
- **核心裁决（安全相关）**：ZIP 允许 local header 与 central directory 对同一字段给出**不同**值 ——
  这是"不同解包器读出不同文件"的真实攻击面。本读取器**两处都读、逐字段比对，任何不一致直接拒绝**。
  **歧义本身就是漏洞；消除歧义的方式是拒绝，而不是"选一个为准"。**
- **判定顺序是契约的一部分**：膨胀比率闸门用**声明值** fail-fast（炸弹的声明本身就足以拒绝），
  后面再用**实际写出字节**兜底 —— 两者覆盖不同攻击（真炸弹 vs 伪造声明），各有判据。
- **阈值必须可注入**（`ContainerLimits`），否则"单条目 ≤2GB / 比率 ≤100:1"这条判据在小数据上根本没法变红。
- **仍缺（登记为 pending）**：`deflate` 支持（**且届时必须补"压缩后大小的实际量"这条独立闸门**，
  因为 stored 下它与尺寸一致性判据部分重叠）、Unicode NFC/NFD 折叠（APFS 上理论上存在重名覆盖通道）、
  Windows 上标数字设备名别名。
- **下一个能力切片**：`yeban-mcp/src/domain/store.rs`（`ARCH-SEC-004` 原子落盘）目前**不调用**本模块；
  把 `.yeban` 的保存/加载接到 `read/write_project_container` 上是下一步（本线只交付字节层，不碰文件系统）。

### D31 — `.yeban.lock`：用 `flock(2)`（不是 `fcntl`），且**只读打开也上共享锁**

- **事实核验（先核验再写码）**：`std::fs::File::{try_lock, try_lock_shared, lock, lock_shared, unlock}`
  与 `TryLockError` **自 1.89.0 起稳定** ⇒ **零新增依赖**（不需要 `libc`、不需要 `windows-sys`）。
  Unix 实现是 **`flock(2)`**；Windows 是 `LockFileEx`。
- **为什么不是 `fcntl(F_SETLK)`**（实测决定）：`fcntl` 的锁在**同一进程的多个 fd 之间不冲突**，
  因此拦不住"同进程开两次工程"；`flock` 会冲突。规范 `ARCH-SEC-001` 的字面提到 `fcntl` 语义，
  **本裁决以意图为准**（并发打开必须被拦住），字面偏差记录在此。
- **行为变更（需人类确认）**：**只读打开现在也会创建并持有一把*共享*锁**（此前只读不建锁文件、不受保护）。
  理由：读者与写者必须锁**同一个 inode**，否则"只读"只是一句口号 —— 写者会绕过共享锁。
- **陈旧锁语义**：**建议锁本身就是"持有者还活着"的证据**（内核在进程死亡时自动释放）。
  `SIGKILL` 之后锁文件**仍在磁盘上**，但**立即可接管**，且元数据被原子重写（陈旧 PID 被覆盖）。
  "文件存在 = 被占用"是**错的**，并有判据专门钉住这一点（旧实现正是这么错的，会永久锁死）。
- **跨进程证据**：用 `Command` 重跑测试二进制 + **文件握手**（不是 sleep 猜时间）做双向互斥验证，
  以及 `SIGKILL` 崩溃自愈。
- **未验证（不许宣称）**：Windows 分支（`LockFileEx`）**从未编译过**；非 Unix/非 Windows 平台只有
  `UnsupportedPlatform` 的映射判据。两者都登记为 pending。
- **心跳有意收窄**：不参与"是否被占用"的判定（建议锁已经是单调的活体证据），只进诊断载荷。

### D32 — 跨架构"位精确"必须**按运算类别分策**：IEEE 精确类零容差，超越函数类给预算

- **实测发现**（`line/dsp-level`，其**自己写的**"搬迁前后逐位不变"判据把它打红）：
  冻结的 285 条 `f32` 位模式里，**只有 1 条**在 x86_64 与 aarch64 上不同 ——
  `dbfs(√½)`：aarch64 `0xc040a8c2` vs x86_64 `0xc040a8c3`，**差 1 ULP**。
  根因是 **`f32::log10` 不要求正确舍入**（Rust 的标准库超越函数只保证精度、不保证位模式）。
- **裁决（按运算类别分策）**：
  1. **IEEE-754 精确类**（钳位、比较、`abs`、`max/min`、加/减/乘/除、`sqrt`）：
     **跨架构零容差、逐位相同**必须成立 —— 这些运算由 IEEE-754 完全规定。
  2. **超越函数类**（`log`/`log10`/`exp`/`sin`/…）：给一个**显式 ulp 预算**（本仓库取 4096 ulp），
     并在**冻结架构**（项目基准：aarch64）上仍要求逐位相同；另一个架构只要求落在预算内。
  3. 预算必须**远紧于**任何真实的行为漂移 —— 本仓库的真实漂移量级（例如把峰值释放率从 20 改成 10 dB/s）
     约 5×10⁴ ulp，而预算是 4096 ulp（紧 10 倍以上）⇒ **注入照样会红**，判据没有失去判别力。
- **为什么这条重要**：`MUST-GATE-003`（跨架构 L2 < 1e-6）与 `ARCH-DET-001`（L1 位精确）的措辞必须承认这一层：
  **"位精确"在含超越函数的链路上跨架构是不可达的**，除非把超越函数换成自研的、可跨平台复现的实现
  （那意味着自己实现 `log10` 并冻结其位模式 —— 成本与风险都很高，**不在本阶段**）。
  因此：L1 位精确只在**同一架构同一工具链**下要求（现状如此）；跨架构只要求**数值预算**。
- **顺带的方法论**：这条 1-ULP 差异**不是别人发现的**，是那条线为"搬家不改行为"写的判据发现的。
  它在台账里如实写了两轮读数（79c9c37 红 → a5b9c7d 绿）与"按类别分策"的理由 —— 这是正确处置。

---

## D5 的落地细节（版本钉死）

测量时刻：**2026-10-05**，方法：`https://crates.io/api/v1/crates/<name>` 的 `max_stable_version`。

| crate | 钉死版本 | 备注 |
| :--- | :--- | :--- |
| slint / slint-build | 1.18.1 | 要求 Rust ≥ 1.92（因此工作区 MSRV 定为 1.92） |
| i-slint-backend-testing | `=1.18.1` | 内部 crate，不遵循 semver，必须精确同版本 |
| cpal | 0.18.2 | |
| rtrb | 0.4.0 | 唯一 UI↔音频无锁通道 |
| symphonia | 0.6.1 | MPL-2.0，MSRV 1.85 |
| rubato | 5.0.1 | |
| ulid | 3.0.0 | 见 D6 |
| serde / serde_json | 1.0.229 / 1.0.151 | |
| thiserror | 2.0.21 | |
| sha2 | 0.11.0 | CAS 摘要 |
| proptest | 1.11.0 | 关掉默认的 fork/timeout，纯数据模型不需要进程 fork |
| criterion / iai-callgrind | 0.8.2 / 0.16.1 | |
| zip / flate2 / hound / midly / midir / notify / rstar / signalsmith-stretch | 8.6.0 / 1.1.10 / 3.5.1 / 0.5.3 / 0.11.0 / 8.2.0 / 0.13.0 / 0.1.3 | 尚未被任何成员引用，因此在 `Cargo.lock` 里还看不到 |

**未采纳的替代**：`synth` 里手工重写的 `dsp/ladder.rs` 取代了 vendored DaisySP，方向正确但那是 C/C++ 混合；
夜半坚持纯 Rust，因此不引入 DaisySP / Soundpipe（见 `ledger/legacy-reuse-audit.md`）。

---

## D10（补充）— 本机与 CI 的分工，写进规范级别的纪律

- 本机是 Apple M2 **开发/编辑**环境。允许：`cargo fmt`、无重依赖 crate 的 `clippy/test`、全部守卫脚本。
- 禁止在本机跑：`--workspace` 全量构建、Slint / cpal / symphonia 编译、基准、模糊测试、跨架构对账。
- 上述限制由 `scripts/dev/cargo-local.sh` 与 `scripts/gates/run-gates.sh light` **机械执行**，不靠自觉。
- 任何"绿"只有来自 CI 的 run 才算数；未读取的判决记为 `pending`。

### D43 — **1.0.0 之前不存在"历史包袱 / 旧版本兼容"这回事**（负责人 2026-10-04 明确授权）

- **政策原文**：在发布 **1.0.0 正式版**之前，我们**不要有任何历史包袱和旧版本兼容之类的需求**，
  一切都在开发状态；如果测试或开发过程中发现问题、或出现架构/设计上的更优解，
  **直接推翻之前的设计和代码就好**。
- **它是一条"授权推翻"的裁决**，优先级高于本 ADR 里所有**出于兼容性**而做出的让步。具体地：
  1. **凡是"为了读旧文件/旧格式"而存在的分支，一律删除**，而不是"保留 + 记删除条件"。
     第一批点名：`yeban-mcp` 的**裸 JSON 兼容读路径**（`bare-json`）、`yeban-app` 的
     `DocumentFormat::ProjectJson`（同一件事在界面侧的复制品）。容器是**唯一**格式。
  2. **凡是"为了旧工程能读"而加的 `#[serde(default)]`，一律重新审视**：
     若该字段在设计上是**必需**的，就**要求**它（缺失即报错），让损坏/过期的文件**响亮失败**，
     而不是被默认值悄悄补全 —— 后者会让"读进来一个语义不完整的工程"变成静默成功。
     （若字段在设计上天然可选，如 `Option<T>`，那是**语义**而非兼容，予以保留。）
  3. **契约（`schemas/*.json`）可以随设计收紧**：不再需要"只加不删"的保守演化，
     也不必为"曾经发布过的形状"留 union。
  4. **数据库/文件格式可以随时改版**：`schema_version` 的作用从"兼容多版本"降级为
     **"拒绝不匹配版本"**（这是**错误检测**，不是兼容）。
- **不解除的东西（重要）**：这条授权**只针对"历史兼容"**，不解禁任何红线：
  确定性（`ARCH-DET-001/002`、D32）、实时零分配、容器安全闸门（Zip-Slip/炸弹/上限）、
  许可合规、`#![forbid(unsafe_code)]`、以及"只有 CI 判决算绿"这些**一律不变**。
  推翻设计**仍然**要走同样的证据标准：判据、注入→变红、CI 判决。
- **落地方式**：受影响的既有裁决**就地改写**而不是叠加（这是本条政策的直接推论）：
  `HD-20`（裸 JSON 何时删 → **立刻删**）、`HD-23`（`history.dag` 版本信封 → **不必要**，
  版本只用于拒绝不匹配）、`HD-16`/`HD-30`（容器子集与 deflate：**按需要**决定，不再以"兼容"为由保留）。
  后续所有"要不要为旧格式留一条路"的讨论，**默认答案是"不留"**。

## 待人类批准/补充

> **唯一入口**：`docs/ledger/human-decisions.md`（40 项，编号 `HD-01..HD-40`，每项都写了选项/建议/不决定的后果）。
> 本节只做**索引**，细节与"当前处置"以那份清单为准；它由 `scripts/gates/check_decisions.py` 机械守卫
> （本 ADR 里任何标了 `Proposed`/`待人类`/`需人类` 的裁决若不在册，`run-gates.sh light` 会红。
> **2026-10-04 起本 ADR 的全部裁决已追认**，那 42 项的逐条结论见该清单。）

1. 本 ADR 全部裁决（尤其 D3 的 `schema_version = 1`、D7 的 PENDING 策略、D12 对 `Op` 全集的扩展、
   D16 的窗函数口径、D17 的 BSL-1.0 与 D20 的 Unlicense 接纳）；
2. `ROAD-M-1-006` 的 4 条人类审核（法务措辞、ASIO、商标、发布签名）——Agent 不得代签；
2b. **规范措辞修订**：`SLINT_BACKEND=headless` 与 `slint::testing::*` 两处在 `ARCH-UI-003/005` 与
   UI/UX §12.1 中与上游 1.18.1 不符（ADR-0001 D18），需要人类改写规范正文；
3. 自托管固定频率 runner 的预算与接入时间（决定 BASELINE 与确定性门禁何时接线）；
4. `website` 分支的 Cloudflare 凭据（`CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID`）；
5. `LEGAL.md` / `GOVERNANCE.md` 里 6 处失效的 `file:///home/crow/work/agy/review/...` 绝对链接
   （AGENTS.md §2 红线 1 禁止 Agent 修改这些文件，因此 `scripts/gates/check_docs_links.py`
   对它们**只告警不阻断**，等人类负责人修复）。
