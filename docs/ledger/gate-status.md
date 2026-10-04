# 门禁状态表（`MUST-GATE-001..015` + `BASELINE-001..006`）

> **这份文件是人读「现在到底什么算绿」的唯一去处。** 手动档 `gates-manual.yml` 的 `pending` 门禁渲染它，
> 而 `scripts/gates/check_gate_status.py` 会在每次 `run-gates.sh light` 里校验**这张表本身没有腐烂**。

维护纪律（**必须遵守**，否则这张表会变成谎言）：

1. 每条门禁的状态只能取 **`已接线` / `部分` / `PENDING`**；
2. 标 `已接线` / `部分` **必须**给出**可复跑的判据**（测试名 / run id / 命令 / 文件路径）—— 没有证据的一律 `PENDING`；
3. 状态变化时**同一次提交**里改这张表；只改代码不改表 = 表在说谎（与 `docs/DEVELOPMENT_LEDGER.md` 的 L12/L18/L20 同族）；
4. `PENDING` 必须写清**为什么**：没实现 / 需要固定硬件 / 需要人类裁决 / 需要凭据 —— 四种性质完全不同，
   混在一起会让人误以为"再写点代码就好了"。

## A. 硬性发布门禁（`MUST-GATE-*`）

| ID | 要求 | 状态 | 证据 / 为什么还不到 |
| :--- | :--- | :--- | :--- |
| `MUST-GATE-001` | 实时回调零分配/零释放/零 I/O/零锁 | **部分** | 运行期断言已补上：`crates/yeban-engine/tests/rt_zero_alloc.rs` 用**计数型全局分配器**（`ARMED` 开关只统计窗口内）跑 10,000 个量子 + 63 次快照交换，断言 `allocations == 0 && deallocations == 0`；另有 `crates/yeban-engine/tests/meter_rt_contract.rs`（同样 `harness=false`，7 场景 / 10,242 量子，含真实电平路径）；结构性判据仍在 `crates/yeban-engine/src/block.rs`/`ring.rs`。**仍缺**：零锁等待与零阻塞 I/O 的运行期断言（当前只有"代码里没写"）|
| `MUST-GATE-002` | 同平台离线母带 bit-exact（L1） | **部分** | `crates/yeban-render/src/render.rs` 的"同种子两次渲染逐字节相同"与"1/2/4/8 线程 digest 相同"判据；`BASELINE-001` 实测 run 37228045430 里 `threads=1` 与 `auto` digest 相同；**跨机器**的 SHA-256 全同尚未做成门禁 |
| `MUST-GATE-003` | 跨架构 L2 一致性（< 1e-6） | **PENDING** | 需要 x86_64 与 AArch64 两条真跑后对账；ADR-0001 D7 明确未具备条件前保持 PENDING，**不得**用单架构读数冒充。**D32 补充**：`line/dsp-level` 实测 `f32::log10` 不保证正确舍入（`dbfs(√½)` 两架构差 1 ULP）⇒ 含超越函数的链路**跨架构位精确不可达**；本门禁的 1e-6 预算口径与此一致，但"位精确"只能同架构要求 |
| `MUST-GATE-004` | `cargo-deny` 100% 通过 | **已接线** | CI `deny` 腿四项全跑（advisories/bans/licenses/sources）；本地 `scripts/gates/run-gates.sh deny` |
| `MUST-GATE-005` | GPLv3 源码分发完备性（.slint + 锁定 Cargo.lock + vendor） | **部分** | `.slint` 与 `Cargo.lock` 均在版本控制内且 `lockfile` 腿校验锁未被改写；**`cargo vendor` 离线依赖包既无脚本也无门禁** |
| `MUST-GATE-006` | Zip-Slip 防御（100% 拦截） | **已接线** | `crates/yeban-model/src/container/path.rs` 的路径规范化（拒绝 `..`/绝对路径/跨卷符号链接/空段/NUL/反斜杠），判据在 `crates/yeban-model/src/container/mod.rs` 的测试里（84 条容器判据本机全跑）；`git show line-archive/container` 为证据留痕 |
| `MUST-GATE-007` | 解压炸弹防御（单条目 ≤2GB、比率 ≤100:1） | **已接线** | `ContainerLimits`（阈值可注入）在 `crates/yeban-model/src/container/zip.rs`：声明体积闸门 → 实际写出字节闸门 → 累计体积 → 膨胀比率（**fail-fast 用声明值**）；含"声明值撒谎"判据（按实际写入量再判一次）|
| `MUST-GATE-008` | `.yeban.lock` OS 建议锁 + `PROJECT_LOCKED` | **已接线** | Unix: `crates/yeban-mcp/src/domain/lock.rs` + `tests/lock_advisory.rs`（跨进程 `Command` + 文件握手 + `SIGKILL` 崩溃自愈），CI run 37232643213 绿。**Windows: 新增手动档 `windows` 真跑验证**（`windows-latest`，`cargo test/clippy -p yeban-model -p yeban-mcp --all-targets -D warnings`）—— 该门禁**三次红各抓到一类真问题**: ① `LockFileEx` 强制锁导致"持锁后读元数据"必然失败（store-container 修）② 判据隐含"本环境有 jsonschema"（改为响亮 SKIP）③ 平台相关的未使用 import 撞 `-D warnings`（加 `#[cfg(unix)]`）；**最终 run 37237134932 = success**。仍待: 把该门禁接进 `ci.yml` 的受影响集合（现在是手动档）|
| `MUST-GATE-009` | MCP 严格默认安全 | **部分** | `crates/yeban-mcp/src/security.rs` + `transport/http.rs`（默认关/只绑环回并回读断言/`0600`/`ui:inject` 先于 token 硬禁，112 判据）；`crates/yeban-ui-mcp/src/service.rs` 同模型（52+5 判据）；**缺**发行物层面的"默认关"断言 |
| `MUST-GATE-010` | 10,000 步 `proptest` 逆向守恒 | **已接线** | `crates/yeban-model/src/ops.rs` 的 `state_tree_is_conserved_under_inverse_application`：CI 上 `sequence_steps()` 取 `CI_SEQUENCE_STEPS = 10_000`，32 个 case，逐字节守恒断言 |
| `MUST-GATE-011` | `cargo-fuzz` 千万次变异（模糊测试） | **部分（已真跑三连，尚未达千万次）** | 手动档 `fuzz` 已真跑三轮：90 秒 → **947 850** 次；1000 秒 → **3 523 373** 次；3300 秒 → **7 020 456** 次（`cov: 725`、语料 `1401/449 KB`、`new_units_added 11131`、**零崩溃**）。**速率的实测规律**：`10 415 → 3 519 → 2 126` exec/s —— 随语料与输入上限增长而下降，所以"按秒数估"必然估错（我为此更正过一次外推）。**已按规范的措辞改成按执行次数**：`gates-manual.yml` 的 `fuzz_runs`（默认 **10 000 000**）用 `cargo +nightly fuzz run sfz_parse -- -runs=N`（直接用 libFuzzer 的执行计数，而不是秒数），作业超时提到 **180 分钟**（按 ~2 000 exec/s 估千万次约 85 分钟，90 分钟会卡在边界上）；`fuzz_seconds` 降级为安全上限。**已派发 `fuzz_runs=10000000` 的那一轮** —— 在它读回之前本门禁仍是"部分" |
**更正**：上一版按 90 秒的瞬时速率外推"约 16 分钟可达千万次"**是错的** —— 稳态速率随语料增长降到 3 519/s
（语料 1125→1386、输入上限 1680→4096），同一目标实际约需 **47 分钟**。已派发 `fuzz_seconds=3300`（≈55 分钟）
的长跑（按稳态速率约 1 160 万次）；**在它跑完前本门禁仍记"部分"** |
| `MUST-GATE-012` | 快照退役队列零泄漏 | **部分** | `crates/yeban-engine/src/snapshot.rs` 的退役队列与"主线程 Drop"判据；**高频/长时间交换压测**未做 |
| `MUST-GATE-013` | 仓库不含 ASIO SDK | **已接线** | 守卫 `G07`（`scripts/guards/policy_check.py` 的 ASIO 专有代码扫描），每次 `run-gates.sh light` 都跑 |
| `MUST-GATE-014` | 323 款采样署名全匹配 | **PENDING** | `assets/samples/ATTRIBUTION.md` 已写口径，但 `assets/samples/` **目录为空**（样本未入库）；`validate_schemas.py --repo-assets` 能对账"登记了什么"，**登记本身**需要人类的产品/授权决定 |
| `MUST-GATE-015` | Golden 图必须由 Tier-1 软光栅化产出 | **已接线** | `crates/yeban-ui-test-port/src/render.rs`（自研 `Platform` + `MinimalSoftwareWindow`，不用 `i-slint-backend-testing`）；真实界面 1920×1080 非黑 100%（run 37229660272）；CI 上传 `ui-screenshots-*` |

## B. 性能基准线（`BASELINE-*`）

| ID | 要求 | 状态 | 证据 / 为什么还不到 |
| :--- | :--- | :--- | :--- |
| `BASELINE-001` | 离线母带渲染（32 轨参考工程 A；目标 ≥100× 实时） | **部分** | run 37228045430（`gates-manual` 的 `bench` 档）：30 秒音频单线程 **106.3×** / Rayon 自动 **135.9×**，两次 digest 相同；**托管 runner 不是规范指定的参考硬件** ⇒ 只算数量级超过，不算达标 |
| `BASELINE-002` | 空工程空闲常驻内存（目标 ≤35 MB） | **PENDING** | 无任何内存测量；需真实进程 + 平台内存 API（CI 可测但未接） |
| `BASELINE-003` | 界面渲染帧率（10 万音符滚动；目标稳定 120 FPS） | **PENDING** | 帧率判据需要固定刷新率/无噪声硬件（`spikes/README.md` 已登记）；**不得**用托管 runner 读数宣布通过 |
| `BASELINE-004` | 单步撤销时延 p99（目标 ≤0.2 ms = 200 µs） | **部分** | 新增 `crates/yeban-model/examples/bench_undo.rs`（另接进手动档 `bench`）。**本机 M2 / `--release` 实测（20,000 次/op）**：`undo_set_param` p99 **0.084 µs**、`undo_set_macro` 0.084 µs、`undo_move_clip` 0.084 µs、`undo_batch2`（两步批次）**1.834 µs** —— 全部远优于 200 µs 目标（max 有 6~23 µs 的调度离群值，故以 p99 为准）。**仍差**：规范点名 Apple M2 Pro 12 核 / Ryzen 7 7840HS 的**确认性**复跑；更大的工程规模与更多 `Op` 种类的扫描 |
| `BASELINE-005` | 音频硬件往返时延（目标 ≤5.5 ms） | **PENDING** | 需要真实声卡 + 回环（CoreAudio/WASAPI/PipeWire 原生查询）；**唯一无法用 CI 绕行**的门禁，必须有音频硬件的机器 |
| `BASELINE-006` | AI 交互效率（JSON ≤4 KB、Token 中位数 ≤600） | **PENDING** | 需要"生成 16 小节段落"的完整 MCP 往返统计；十个工具已能真做事，但**载荷统计未接**，且 Token 口径需人类裁决用哪个 tokenizer |

## C. 三条工具事实（避免误读）

1. **`PENDING` 不等于"没做"**：006/007/008 是**正在做**；003/005/BASELINE-003/005 是**当前硬件条件下做不出有意义结论**；
   014 是**需要人类决定装哪些样本**；011 是**设施已接线但从未执行**。三种性质在证据列里分开写了。
2. **"本地绿"不算绿**：只有 `ci.yml` / `gates-manual.yml` 的判决算数（`docs/CI_CD.md` §3）。
3. **读完数先看环境**：托管 runner 的数字只能给数量级；任何"达标"结论都要在规范指定的参考硬件上复跑同一命令。
4. **三个 crate 是"故意空的"，不要提前实现**（规范写明它们的版本阶段）：
   `yeban-services` = **v1.1.0**（外部专业软件联动与 Ping 时延校准）、
   `yeban-plugin-host` = **v2.0.0**（跨进程崩溃隔离宿主）、`yeban-vst` = **v2.0.0**（反向 VST3/CLAP 打包）。
   它们的空壳**不是欠债**；提前实现等于把资源从 v1.0 的缺口上挪走。
   （出处：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 的 crate 责任表。）
