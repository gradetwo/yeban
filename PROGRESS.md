# line/sfz-file-size-gate 进度

## 2026-11-18 20:5x 本地实现
- 字节门落点：`crates/yeban-mcp/src/domain/sfz.rs` 的
  `MAX_SAMPLE_FILE_BYTES`（64 → 256 MiB）。两条执法腿（盘上
  `read_sample_file` 与 CAS 字节 `decode_sample_bytes`）都按常量符号
  引用，一处改、两腿齐。导入侧 `MAX_AUDIO_FILE_BYTES` 早已是 256 MiB，
  其文档里"SFZ 门 64 MiB 不随本条变"的旧注已改为对齐后的口径；
  `sfz_stream.rs` 四处 64 MiB 文案同步为 256 MiB。
- 总量门未动：`MAX_SAMPLE_FRAMES_TOTAL` 64 000 000 帧、
  `MAX_SAMPLE_FILES_READ` 1024（代码现读值；任务单写的也是 1024）。
- 帧数门测算（256 MiB = 268 435 456 B，WAV 数据 ≈ 文件 − 44 B）：
  16-bit 立体声 4 B/帧 ≈ 67 108 703 帧 > 64 000 000 ⇒ 仍被帧数门挡；
  16-bit 单声道 ≈ 134 217 378 帧 ⇒ 挡；24-bit 立体声 6 B/帧
  ≈ 44 739 235 帧 ⇒ 放行；24-bit 单声道 ≈ 89 478 470 帧 ⇒ 挡；
  32-bit float 立体声 8 B/帧 ≈ 33 554 301 帧 ⇒ 放行；float 单声道
  ≈ 67 108 703 帧 ⇒ 挡。本次对齐实际放行的是 24-bit / float 立体声档。
- 测试：超限稀疏文件改 256 MiB + 1（仍响亮拒绝）；新增回归
  `a_sample_file_over_64_mib_loads_after_the_byte_gate_alignment`
  （24-bit 立体声 12 000 000 帧真 WAV，盘上 72 000 044 B，必须读入）。
- 待：盒上 yeban-sfz 全套＋yeban-mcp sfz 相关测试、clippy、fmt。

## 2026-11-18 20:40–20:42 磁盘问答（父 agent 问）
- 20:40–20:42 之间我删的是：`/data/workspace/yeban-range-warmup/target`
  （线一工作树在盒上的构建目录，du 5.6 GB）。原因：CHECK_APP 首跑
  以 ENOSPC 退出 101（rustc 建临时目录失败，非代码问题）；线一已验完
  并推送（tip c269887be），其盒上 target 按清盘原则可删。删后 df 回
  5.6 GB（父 agent 20:42 live 复核 5.4 GB，差额是随后重跑 CHECK_APP
  写回的约 0.2 GB）。未动：sfz-gate 的 target（4.6 GB）、两树源码、
  全部日志与脚本。执行前已核 hostname=instabox-yanshi-dev。

## 2026-11-18 21:2x 盒上全套验证全绿（sfz-verify1.log＋sfz-check-app.log）
- FMT=0；CLIPPY_MCP（--all-targets -D warnings）=0；TEST_SFZ_PKG
  （yeban-sfz 全套）=0；TEST_MCP_SFZ_FILES=0（新回归
  `a_sample_file_over_64_mib_loads_after_the_byte_gate_alignment` 与
  超限拒绝判据均在日志里具名 ok，来源串
  `frames=12000000 channels=2 rate=48000`）；TEST_MCP_LIB=0
  （750 passed）；CHECK_APP（yeban-app --lib）首跑 101 系满盘 ENOSPC，
  清掉线一盒上 target 后重跑 =0。
- 线二完工，推分支 line/sfz-file-size-gate。

---

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
