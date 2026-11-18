# 期 4 进度（line/perlane-sidechain-p4）

- 19:46 开工。基线 386d53fc5，分支已建 ~/workspace/wt-p4。wow 标记已建；wow cap 21:11 CST（480 分钟，05:11:29Z 起），/data 同步后余 5.7 GB（停线 3 GB）。盒上 warm 构建（render/mcp --lib 测试二进制）已完成 EXIT=0。
- 已定口径：同轨 lane 键 = 链前抽头（pre-chain tap）；跨轨 lane 边本期只登记不求值（回执计数响亮），缺口写回执规格。
- 实施序：①808 声部（dsp DrumVoice::Bass808=13 复用 kick() 滑音正弦体 + engine bass808_note/参数族 + theory kit_setup 电子族写 bass808_note=35 与默认参数）→ ②模型 RoutingEdge source_lane/dest_lane + 校验 + schema → ③render 同轨 lane 键求值（attach 分流 + LaneSplitSource 两遍渲染 + process_with_key）+ 回执计数 → ④生成（theory 去掉侧链违规拒绝、mcp/app 两路生成边）→ ⑤判据（render 音频 + model + mcp）。
- 20:0x 代码事实已核：期 1 同层求值已在（layer_key_orders）；期 2 已有 theory sidechain 解析与「生成响亮拒绝」（lane_chain_generation_violations 第 1 类），期 4 改为真生成边；ConnectRouting 携整条 RoutingEdge，生成侧只需填 lane 字段。

## 20:2x 进度
- 段① 808 槽已提交推送：2028ad480。段② 模型字段+全树清扫（89 文件）已提交推送：27901d4dd。
- 清扫脚本两类误伤已修复并审计：函数签名 `-> RoutingEdge {` 被误匹配（render.rs 2 处、engine/tests/sidechain_key.rs 2 处、mcp section_build.rs 1 处），全部修复，残形扫描清零。
- check2（model/dsp/engine/theory/render/mcp --all-targets）：仅剩 MCP error.rs 穷尽 match 缺新变体一臂，已补（InvalidParameterRange 组）。
- 段③ render 已写：LaneKeyRoute/LaneKeyLayout 类型、attach_lane_key_routes（同轨双端解析）、execute 装源后安装（源不接受响亮失败）、LaneSplitSource 两遍键控渲染（第一遍键源组链前渲染、第二遍复制+process_with_key）。待盒上编译。
