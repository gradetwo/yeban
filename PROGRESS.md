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
