# PDC 逐边慢路波形落点调查（line/pdc-waveform-investigation）

- 基线：origin/main b6ff64563（2026-11-19 开工 ls-remote 现读，与简报一致）。
- 盒：yanshi-5（唤醒 06:48 CST，maxRun 120，cap 约 08:48 CST）。标记已建。
- 问题：v35 stage 30 判据 `pdc_edge_compensation.rs` 文件头登记——逐边慢路产出的
  `dual64[t]` 里有一份 `base[t + 64]` 分量（直连边该晚 64 帧的分量提前了 64 帧）；
  节点级实现下不出现（干净的 `2 · base[t − 64]`）。判据现只钉计划/武装/非节点级，
  未钉波形落点。

## 代码阅读结论（测量前）

- `DelayLine`（graph.rs:655-855）本体语义正确：读指针 = 写指针 − delay，
  fresh 线先出 delay 个零，之后是输入延迟 delay 帧。单声道/成对两版同式。
- rt.rs 两处逐边慢路（逐轨段宏 `route_panned_or_per_edge!` 约 2303 行、总线转发段
  约 2640 行）结构同形：基信号存回 track_scratch(_r) → 每边拷一份进
  reverb_scratch[0]/[1] → `EdgeBank::apply_pair` 就地延迟 → `route_panned` 单边路由。
- 量子 = 128 帧（support::render），延迟 64 = 半个量子。
- `edges_share_one_shift` 读已武装 EdgeBank 的 `delay_of`，缺失按 0。

## 测量结果（探针 v1+v2，盒上 pdc_probe）

- base 起音第 1 帧；dual64 起音第 **0** 帧。
- `busonly == base`：1024/1024 逐位（经总线分量无偏移）。
- `dir64 == base[t-64]`：1024/1024 逐位（节点级线延迟正确）。
- `dual64 == base + base[t-64]`（期望式）：512/1024（只有每量子后半成立）。
- `dual64 == base + rotate64(base)`（每量子内前 64 帧取 base[t+64]、后 64 帧取
  base[t-64]）：**1024/1024 逐位**。
- 插桩（盒上副本 EdgeBank::apply_pair 加 eprintln）：每量子 4 次调用——轨段以真实块
  B 处理两条边（delay=64 输出正确：[0×64, B[0..64]]），其后又以**全零块**处理一遍，
  delay=64 线在零块遍输出 [B[64..128], 0×64] 并被路由。两遍之和 = 旋转。

## 根因（已证成）

`render_branch` 的总线转发段（3a⁗）遍历 `RoutingPlan::slots()`（= 全部可渲染节点，
含纯轨），把每个节点的总线累积缓冲经补偿线**再转发一次**。纯轨 split 已在逐轨段
用真实信号喂过自己的边线；它无入边 ⇒ 累积恒为零 ⇒ 转发段的第二遍喂的是零块，
但延迟线会把上一遍未到点的真实信号尾巴（B[64..128]）在这遍吐出并路由进 master，
落点比计划提前 64 帧（量子 128、D=64 时恰成每量子旋转）。`uses_buses == false`
时 3a⁗ 整段跳过 ⇒ 节点级对照（dir64）干净；dual0 各线 D=0 ⇒ 第二遍输出恒零，
无伪影。

## 修复

3a⁗ 循环跳过 `!routing.has_incoming_audio(node)` 的节点：其累积可证恒空
（只有 route_panned 写累积、且只写目的地 = has_incoming_audio 的节点；处理型
总线的自声源汇入同样以 has_incoming_audio 为前提），跳过对 D=0 工程逐位不变，
只改变「非零延迟 + 无入边 + 图里有总线」这一已损坏类别。

## 同族残余（已界定、本线不扩修，登记为后续规格）

同一不变量（一条节点的补偿线每量子只能被喂一次、且喂的是该节点本量子的完整
输出）在另外两个形状下仍未成立，Fix A 不覆盖：

1. **总线/混合节点且补偿延迟 D > 128 帧（一个量子）**：其线在逐轨段先被喂
   自有静音（纯总线）或自有内容（混合轨），再在 3a⁗ 被喂收入之和。代数上
   D ≤ 128 时两遍输出恰好分区无重叠（变体 B 实测 D=128：期望式逐位
   1024/1024 成立），D > 128 时分区破裂、分量落点偏移。复现规格：三总线汇
   master、一总线上报延迟 256 的工程，量 D=256 的两条总线分量落点。
2. **混合轨（自有片段 + 入边、无已武装插入器件）且任一出边 D > 0**：自有
   内容（逐轨段）与收入之和（3a⁗）是两份真实流，先后喂同一条线 ⇒ 输出是
   两条流的交错延迟，不是「和」的延迟。复现规格：在 stage 30 夹具上给
   split 加一条入边（另一条轨 → split）并保留 split 自有音符，量其直连
   分量落点。

两条都需要「节点输出先汇合、再单点喂线」的结构性改动（涉及相位银行与发送
平滑的施加位置），应另立线并配各自判据，不在本线顺手扩修。

## 步骤

- [x] 探针分解＋插桩定位。
- [x] 根因定位（代码路径＋测量互证）。
- [x] 修复（3a⁗ 跳过无入边节点）＋落点判据 E5 先红（未修：整测失败于 E5）
      后绿（修复后 1 passed）。
- [x] 验证（yanshi-5）：yeban-engine 全套 889 passed / 0 failed
      （/data/workspace/pdc-engine-suite.log）；clippy -p yeban-engine
      --all-targets -D warnings 退出 0（pdc-clippy.log）；fmt --check 退出 0；
      判据格式化后复跑 TEST_EXIT=0（pdc-edge-final.log）。诊断报告
      DIAGNOSIS-pdc.md（含同族残余两条的复现规格）。
