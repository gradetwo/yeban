# line/range-render-warmup 进度

## 2026-11-18 20:1x 开工
- 基线 386d53fc5，工作树 ~/workspace/wt-range-warmup，盒 yanshi-dev（已唤醒，hostname 核对，/data 余 11 GB）。
- 根因定位：真区间渲染（B-1）中 `EngineTrackSource::seek_to` 把调度表过滤成
  `start_sample >= frame`，跨起点音符被整体删掉 ⇒ 区间静音。引擎本身的三条支路
  （SFZ 分段触发 / 复音 / 鼓机）都按绝对样本位置发声，缺的只是源层把跨点音符留下
  并让声部状态在起点处与全程一致。
- 定案机制：**源内预滚追补**。seek_to 检出跨起点音符（start < F < end）时不 seek、
  不换表，只记预滚起点 P = 跨点音符的最早 start；首个 render_block 先把引擎
  seek 到 P、内部渲染 [P, F) 并丢弃输出，声部状态由此与全程渲染在 F 处一致，
  再渲染真正的块。无跨点音符 ⇒ 旧路径逐字保留（边界起点逐位不变）。
- 残余冷启动（回执继续诚实登记）：起点前已结束音符的释放尾不重建；预滚点之前
  的声部池窃取历史不重演；宿主插件音源与图内效果尾音仍冷启动。

## 2026-11-18 20:4x 首轮盒验（yanshi-dev）
- 新判据 `a_note_spanning_the_range_start_is_chased_by_preroll` 首轮通过：
  夹具长音帧 48000..144000，中点 96037 起渲 ⇒ 全程同段 −7.68 LUFS、
  区间 −7.68 LUFS、bit_identical=true；边界起点逐位相同。TEST_EXIT=0
  （日志 /data/workspace/range-test1.log）。
- MCP 侧回执/工具文档/判据已按追补语义改写（提交 d8986b405）。
- 本地 cargo fmt --check 通过。待：盒上 render --lib 全套、mcp
  render_master 判据、clippy（render/mcp --all-targets）、app --lib check。

## 2026-11-18 21:1x 盒上全套验证全绿（range-verify2.log）
- FMT=0；CLIPPY_RENDER（--all-targets -D warnings）=0；
  TEST_RENDER（yeban-render --lib 全套）=0（333 passed）；
  CLIPPY_MCP=0；TEST_MCP_RENDER_MASTER=0（改写后的跨点段端到端
  逐字节等值通过）；TEST_MCP_LIB=0（750 passed）；CHECK_APP
  （yeban-app --lib，本线仅注释改动）=0。
- 线一完工，推分支 line/range-render-warmup。
