# 诊断报告：逐边 PDC 慢路波形落点（提前 64 帧）

- 线：`line/pdc-waveform-investigation`，基线 main `b6ff64563`。
- 对象：v35 stage 30 判据 `crates/yeban-engine/tests/pdc_edge_compensation.rs`
  文件头登记的未决项——逐边档实测波形里有一份分量比计划**提前 64 帧**
  （`dual64[t]` 里出现 `base[t + 64]`），节点级实现下不出现。
- 结论：**根因已证成并已修复**；落点已钉进判据（新断言 E5，先红后绿）。

## 复现（数字与单位）

夹具与判据同形：120 BPM / 48 kHz，量子 128 帧，渲染 8 个量子 = 1024 帧；
总线上报延迟 64 帧。逐位命中帧数（左声道，共 1024 帧）：

| 关系 | 命中 |
| :--- | ---: |
| `busonly == base`（经总线分量无偏移） | 1024 |
| `dir64 == base[t − 64]`（节点级线延迟正确） | 1024 |
| `dual64 == base + base[t − 64]`（期望式） | 512（仅每量子后半） |
| `dual64 == base + rotate64(base)`（每量子内前 64 帧 `base[t + 64]`、后 64 帧 `base[t − 64]`） | **1024** |

起音帧：base = 1，dual64 = **0**（提前的直接证据）。

插桩（盒上副本在 `EdgeBank::apply_pair` 打印 delay 与块首样本）：每量子共
4 次调用——逐轨段以真实块 B 处理两条边（delay=64 的输出正确：
`[0×64, B[0..64]]`）；其后又以**全零块**处理一遍，delay=64 的线在零块遍
输出 `[B[64..128], 0×64]` 并被路由进 master。两遍之和恰为上表旋转式。

## 根因

`EngineRuntime::render_branch` 的总线转发段（3a⁗，`crates/yeban-engine/src/rt.rs`）
遍历 `RoutingPlan::slots()`——该表含**全部可渲染节点**（`routing.rs` 的
`compile` 按拓扑序收录每个非 master 轨），并把每个节点的总线累积缓冲经其
补偿线再转发一次。纯轨 `split` 的自有信号已在逐轨段（3a）经边线喂过并路由；
它无入边，总线累积整量子恒为零。转发段的第二遍因此喂的是零块，但延迟线是
有状态的：它把上一遍尚未到点的真实信号尾巴（`B[64..128]`）在这遍吐出并
路由进目的地 ⇒ 该分量在本量子内提前 64 帧出现（D=64、量子=128 时呈逐量子
旋转）。这不是 `route_panned` 的求和问题，也不是 `EdgeBank`/`DelayLine`
的缓冲语义问题（两者经分量隔离测量均逐位正确）——是**同一条边线在一个
量子内被两个站点重复喂入**。

对照为何干净（同轮实测）：`dir64`（单边工程）`uses_buses == false`，3a⁗
整段跳过；`dual0` 各线 D=0，第二遍输出恒零；`busonly` 的受影响线 D=0。

## 修复

3a⁗ 循环跳过 `!routing.has_incoming_audio(node)` 的节点（`rt.rs`，+14 行
含注释）。依据：这类节点的累积可证恒空——只有 `route_panned` 写累积且只
写目的地，而「有入边」正是该谓词；处理型总线把自声源汇入本槽同样以有入边
为前提。对 D=0 的工程第二遍输出恒零 ⇒ 跳过逐位不变；只改变「非零延迟 +
无入边 + 图里有总线」这一已损坏类别。

判据：`pdc_edge_compensation.rs` 新增 **E5**（`dual64[t] == base[t] +
base[t − 64]` 逐位，1024 帧全量断言），文件头「待查」登记已改写为收口
记录。红绿实证：仅同步新判据到未修代码 ⇒ 整测失败于 E5；同步修复后
`TEST_EXIT=0`（1 passed）。

## 验证（yanshi-5，日志在盒上 /data/workspace/）

- `cargo test -p yeban-engine --no-fail-fast` 全套：**889 passed / 0 failed**
  （69 个结果行，含 lib、全部集成套件与 doctests；含 pdc_bus_latency、
  pdc_mix_path、pdc_edge_compensation 三个二进制）。日志 `pdc-engine-suite.log`。
- `cargo clippy -p yeban-engine --all-targets -- -D warnings`：退出 0。
  日志 `pdc-clippy.log`。
- `cargo fmt -p yeban-engine -- --check`：退出 0（本地工作树）。
- 判据单独复跑（格式化后）：`TEST_EXIT=0`，日志 `pdc-edge-final.log`。

## 同族残余（已界定、本线未修，规格可直接立线）

同一不变量（一条节点的补偿线每量子只喂一次、喂其完整输出）在两个形状下
仍不成立，Fix A 不覆盖：

1. **总线/混合节点且补偿延迟 D > 128 帧**：其线逐轨段先喂自有静音/自有
   内容、3a⁗ 再喂收入之和。D ≤ 128 时两遍输出恰分区无重叠（变体 B 实测：
   三总线汇 master、bus3 上报 128、bus1/bus2 的 D=128，期望式
   `2·A[t−128] + A[t]` 逐位 1024/1024 成立）；D > 128 分区破裂。复现
   规格：同形工程把上报延迟改为 256，量两条 D=256 总线分量的落点。
2. **混合轨（自有片段 + 入边、无已武装插入器件）且任一出边 D > 0**：
   自有内容与收入之和是两份真实流，先后喂同一条线，输出为两流的交错
   延迟而非「和」的延迟。复现规格：stage 30 夹具给 split 加一条入边
   （另一轨 → split），保留 split 自有音符，量直连分量落点。

两条都需「节点输出先汇合、再单点喂线」的结构改动（牵动相位银行与发送
平滑的施加位置），应另立线并配各自判据。
