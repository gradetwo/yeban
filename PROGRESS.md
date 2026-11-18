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
