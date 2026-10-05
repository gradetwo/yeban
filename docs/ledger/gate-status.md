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
| `MUST-GATE-003` | 跨架构 L2 一致性（< 1e-6） | **已接线（有真实跨架构读数）** | 手动档 `arm`（`gates-manual.yml`）在 **`ubuntu-24.04` 与 `ubuntu-24.04-arm`** 两腿各导出 L1 收据再对账；**run 37244030287 = success**（x86_64 腿 ✓ / aarch64 腿 ✓ / `arm 对账` ✓）。实测读数：参考工程 A（32 轨 / 8192 帧 / 16384 样本）——纯 IEEE 类 `digest=94074a0362db6f60c24c3a3f8923fdb0b64ae2bffa8bd92c7b826b84110f2ff8`、带增益（3 dB）`digest=e3bb731def72979a953314243214f53a1cffea41cce9f30cdfdf8edda3d92b89` —— **两条在两架构上逐字节相同** ⇒ 判决 `L1-bit-exact`（比 1e-6 预算更强）。**诚实边界**：① 该参考管线的实际执行路径**只含 IEEE 精确类运算**（收据里声明的 `T` 类未被真正执行），所以 D32 的 ulp 预算**尚未被真实跨架构分歧触发过**（它由 `crates/yeban-render/tests/l1_digest_contract.rs` 的 10 条判据与 18 条本机判据覆盖，但那不是跨架构证据）；② 本轮 `latency=none` ⇒ PDC 延迟线未被这份读数覆盖；③ 收据里的 `longest_path_frames=0` 与此一致 |
| `MUST-GATE-004` | `cargo-deny` 100% 通过 | **已接线** | CI `deny` 腿四项全跑（advisories/bans/licenses/sources）；本地 `scripts/gates/run-gates.sh deny` |
| `MUST-GATE-005` | GPLv3 源码分发包完备性（`.slint` + 锁定 `Cargo.lock` + `cargo vendor`） | **已接线** | 新增 `scripts/gates/check_vendor.sh`，**轻档**（不联网）校验：① **14 个 `.slint` 全部被 git 跟踪**；② `cargo metadata --locked` 通过（锁确定）；③ 若存在 `vendor/` 则 `.cargo/config.toml` 必须真指向它且 `--offline --locked` 可解 —— 三者在 CI **实测通过**；**重档 `--full`** 在**联网的**手动档 `inventory` 里真跑 `cargo vendor --locked`：**实测 `618 个 crate / 795M`**（run 37266761945 = success）。⇒ 规范点名的三个要素（`.slint` / 确定性锁 / 离线依赖）**各有机械判据且都在 CI 真跑过**。**边界（如实）**：发布用的**源码 tarball 打包步骤**仍属发布流程，本门禁校验的是三要素的可判定性；另有本机沙箱教训（`cargo` 不在 PATH 时脚本曾报假红）已修: 工具缺失记 `unknown`(exit 2) 而非 FAIL |
| `MUST-GATE-006` | Zip-Slip 防御（100% 拦截） | **已接线** | `crates/yeban-model/src/container/path.rs` 的路径规范化（拒绝 `..`/绝对路径/跨卷符号链接/空段/NUL/反斜杠），判据在 `crates/yeban-model/src/container/mod.rs` 的测试里（84 条容器判据本机全跑）；`git show line-archive/container` 为证据留痕 |
| `MUST-GATE-007` | 解压炸弹防御（单条目 ≤2GB、比率 ≤100:1） | **已接线** | `ContainerLimits`（阈值可注入）在 `crates/yeban-model/src/container/zip.rs`：声明体积闸门 → 实际写出字节闸门 → 累计体积 → 膨胀比率（**fail-fast 用声明值**）；含"声明值撒谎"判据（按实际写入量再判一次）|
| `MUST-GATE-008` | `.yeban.lock` OS 建议锁 + `PROJECT_LOCKED` | **已接线** | Unix: `crates/yeban-mcp/src/domain/lock.rs` + `tests/lock_advisory.rs`（跨进程 `Command` + 文件握手 + `SIGKILL` 崩溃自愈），CI run 37232643213 绿。**Windows: 新增手动档 `windows` 真跑验证**（`windows-latest`，`cargo test/clippy -p yeban-model -p yeban-mcp --all-targets -D warnings`）—— 该门禁**三次红各抓到一类真问题**: ① `LockFileEx` 强制锁导致"持锁后读元数据"必然失败（store-container 修）② 判据隐含"本环境有 jsonschema"（改为响亮 SKIP）③ 平台相关的未使用 import 撞 `-D warnings`（加 `#[cfg(unix)]`）；**最终 run 37237134932 = success**。仍待: 把该门禁接进 `ci.yml` 的受影响集合（现在是手动档）|
| `MUST-GATE-009` | MCP 严格默认安全 | **部分** | `crates/yeban-mcp/src/security.rs` + `transport/http.rs`（默认关/只绑环回并回读断言/`0600`/`ui:inject` 先于 token 硬禁，112 判据）；`crates/yeban-ui-mcp/src/service.rs` 同模型（52+5 判据）；**缺**发行物层面的"默认关"断言 |
| `MUST-GATE-010` | 10,000 步 `proptest` 逆向守恒 | **已接线** | `crates/yeban-model/src/ops.rs` 的 `state_tree_is_conserved_under_reverse_undo`：CI 上 `sequence_steps()` 取 `CI_SEQUENCE_STEPS = 10_000`，32 个 case，逐字节守恒断言 |
| `MUST-GATE-011` | `cargo-fuzz` 千万次变异（模糊测试） | **已接线（达标：恰好一千万次执行，零崩溃）** | **run 37243206669 = success**，libFuzzer 收尾行 **`#10000000 DONE cov: 721 ft: 4132 corp: 1381/406Kb lim: 4096 exec/s: 1629 rss: 597Mb`**；统计 `number_of_executed_units = 10 000 000`、`new_units_added = 12621`、**零崩溃**。门禁按**执行次数**（不是秒数）驱动：`gates-manual.yml` 的 `fuzz_runs` 默认 **10 000 000**，用 `cargo +nightly fuzz run sfz_parse -- -runs=N`，作业超时 180 分钟（`fuzz_seconds` 降级为安全上限）。**为什么必须按次数**：四轮实测的稳态速率随语料增长而下降 —— 90 s **947 850**（10 415 exec/s）→ 1000 s **3 523 373**（3 519）→ 3300 s **7 020 456**（2 126）→ 达标轮 **10 000 000**（**1 629**）—— 按秒数估次数必然估错（我为此更正过一次外推）。 |
**更正**：上一版按 90 秒的瞬时速率外推"约 16 分钟可达千万次"**是错的** —— 稳态速率随语料增长降到 3 519/s
（语料 1125→1386、输入上限 1680→4096），同一目标实际约需 **47 分钟**。已派发 `fuzz_seconds=3300`（≈55 分钟）
的长跑（按稳态速率约 1 160 万次）；**在它跑完前本门禁仍记"部分"** |
| `MUST-GATE-012` | 快照退役队列零泄漏 | **部分** | `crates/yeban-engine/src/snapshot.rs` 的退役队列与"主线程 Drop"判据；**高频/长时间交换压测**未做 |
| `MUST-GATE-013` | 仓库不含 ASIO SDK | **已接线** | 守卫 `G07`（`scripts/guards/policy_check.py` 的 ASIO 专有代码扫描），每次 `run-gates.sh light` 都跑 |
| `MUST-GATE-014` | 323 款原声乐器采样署名全匹配（仅限 CC0/CC-BY/MIT） | **部分** | **机制已接线 + 白名单现在真的被执行；但素材本体未分发 ⇒ 仓库内 0 字节被校验**。**素材来源**：按 `ADR-0001 D54` 复用 `groove` 的登记（33 个 SFZ 乐器 / 21 505 文件 / 9.371 GiB），按白名单过滤后**登记 30 款**（27 CC0 + 3 CC-BY；过滤 CC-BY-NC-SA 非商用、CC-Sampling-Plus 不在白名单、Unlicense 待裁决）。**入库形态 = 登记式**（`items[].optional=true`；GitHub 单次 push 上限 2 GB、clone 不宜多背 9.4 GiB）⇒ 实测入库 **8.5 MB**（清单 8 903 874 B + `ATTRIBUTION.md` 14 645 B），**新增音频字节 0**。**门禁原文**：`[ok] assets/samples/manifest.json: 0/20594 条资产的 SHA-256 与磁盘一致（另有 20594 项 optional 资产未随仓库分发）` —— 绿**只**代表清单结构与口径成立。**已修的洞（needs N2）**：此前**全仓没有任何代码读 `license`/`allowed_licenses`** ⇒ 往 `items[]` 塞非商用素材所有门禁全绿；现在 `validate_schemas.py --repo-assets` 会**逐条**校验 `licence_whitelist` 与 `commercial_usable`，并与根清单的 `allowed_licenses` 交叉对账（注入实测 EXIT=1，点名条目与许可）。**如实登记的差额**：规范要求 **323 款**，本次登记 **30 款** ⇒ **差 293 款**（`counts` / `ATTRIBUTION.md` / 根清单三处一致）。**真字节证据（本机）**：84 个文件从上游 `pin` 真下载后重算 sha256 **0 失败**（含 0 B 与 7 836 692 B 两端、2 个 tar.xz 的 52 个成员逐成员、2 个 zip 整包 358 909 701 B）⇒ 清单里的摘要是**真上游摘要**。CI run **37256429601 = success**（`checks` 在 Linux 大小写敏感 FS 上跑 `--repo-assets`） |
| `MUST-GATE-015` | Golden 图必须由 Tier-1 软光栅化产出 | **已接线** | `crates/yeban-ui-test-port/src/render.rs`（自研 `Platform` + `MinimalSoftwareWindow`，不用 `i-slint-backend-testing`）；真实界面 1920×1080 非黑 100%（run 37229660272）；CI 上传 `ui-screenshots-*` |

## B. 性能基准线（`BASELINE-*`）

| ID | 要求 | 状态 | 证据 / 为什么还不到 |
| :--- | :--- | :--- | :--- |
| `BASELINE-001` | 离线母带渲染（32 轨参考工程 A；目标 ≥100× 实时） | **部分** | run 37228045430（`gates-manual` 的 `bench` 档）：30 秒音频单线程 **106.3×** / Rayon 自动 **135.9×**，两次 digest 相同；**托管 runner 不是规范指定的参考硬件** ⇒ 只算数量级超过，不算达标 |
| `BASELINE-002` | 空工程空闲常驻内存（目标 ≤35 MB） | **部分** | 新增 `scripts/gates/measure_rss.py`（RSS 取自 **`resource.getrusage(RUSAGE_CHILDREN).ru_maxrss`** —— 不需要读别的进程、不写 `unsafe`；实测教训：第一版用 `ps` 在受限环境里直接 `Operation not permitted`），并已接进手动档 `bench`（两条：`yeban-app --headless` 加载演示工程 + 模型层参考点）。**本机实测（M2）**：`yeban-model` 导出规范样本的峰值 RSS **2.75 MB**（远低于 35 MB；同一工具对 73 MB 的分配能正确判 `over-target`，说明它**有判别力**而不是恒绿）。**仍差**：① `yeban-app` 的读数只能在 CI 上取（Slint 重依赖，本机按纪律不编译）；② 规范要的是**长命进程的空闲值**，而 `--headless` 是短命进程（两者接近但不相同）；③ 达标判定要在规范指定的参考机上复跑 |
| `BASELINE-003` | 界面渲染帧率（10 万音符滚动；目标稳定 120 FPS） | **PENDING** | 帧率判据需要固定刷新率/无噪声硬件（`spikes/README.md` 已登记）；**不得**用托管 runner 读数宣布通过 |
| `BASELINE-004` | 单步撤销时延 p99（目标 ≤0.2 ms = 200 µs） | **部分** | 新增 `crates/yeban-model/examples/bench_undo.rs`（另接进手动档 `bench`）。**本机 M2 / `--release` 实测（20,000 次/op）**：`undo_set_param` p99 **0.084 µs**、`undo_set_macro` 0.084 µs、`undo_move_clip` 0.084 µs、`undo_batch2`（两步批次）**1.834 µs** —— 全部远优于 200 µs 目标（max 有 6~23 µs 的调度离群值，故以 p99 为准）。**仍差**：规范点名 Apple M2 Pro 12 核 / Ryzen 7 7840HS 的**确认性**复跑；更大的工程规模与更多 `Op` 种类的扫描 |
| `BASELINE-005` | 音频硬件往返时延（目标 ≤5.5 ms） | **PENDING（机器已就绪，缺硬件与口径裁决）** | **工具已交付**（`line/audio-latency`，CI run 37249839115 = success）：`crates/yeban-engine/examples/measure_latency.rs` + `src/latency.rs` + `tests/latency_cli_contract.rs`，能测 ⑩ 项：设备/驱动/接口、默认配置、缓冲区间与 `Unknown`、协商结果、**已开流实际缓冲帧数**、**标称时延**（输入/输出分别）、**主机报告的驱动侧时延**（`playback − callback` / `callback − capture`）、**回调调度抖动** p50/p99/max、后端错误回调次数。**不能测**：**声学 / DAC-ADC 往返**（本门禁本体 —— 需**物理回环**或人类裁决"原生 API 口径"）、以及 ARCH §3.5 的内部 DSP 1.00 ms。**故意不给**驱动侧输入+输出**合计**（合计最像"达标总数"；输出里有 `driver_io_sum_is_roundtrip=false`）。无设备环境实测输出 `verdict=no-device ... this is NOT a pass and NOT 0 ms`、**退出码 3**；5 条注入全部变红（含'无设备当 0 ms'、'标称当实测往返'、'驱动侧当回环证据'）。**仍缺（人类）**：① 裁决"原生 API 口径"是否满足规范；② 有声卡（+回环）的参考机；③ 内部 DSP 1.00 ms 的单独对账 |
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
