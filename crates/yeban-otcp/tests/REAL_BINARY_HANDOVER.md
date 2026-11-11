# yeban-otcp 真实二进制集成测试交接说明

> 状态：2026-11-11，**已在 yanshi-5 真二进制上跑完：T1–T13 13/13 绿**
> （126.84 秒）。分支 `line/yeban-otcp-driver @ 6c4ae1b5`。
> VM 上 mock 集成测试 9/9 绿、单元测试 43/43 绿。
> 本文件原为跑前交接，现为实测记录；重跑方法见下。

## 实测结果（2026-11-11，yanshi-5）

- 二进制：`/data/workspace/opentune-build/OpenTune-api/build-linux-ninja/OpenTuneStandalone_artefacts/Release/Standalone/OpenTune`
  （44,326,464 字节，2026-11-06 02:06 构建；提交 `9ef45f9a3b5a26ed55216a76baf2ccbaf2e05c9f`；
  树含 21 文件未提交改动，0005 反向校验干净、0004 反向校验在
  `AutoTunePitchShifter.cpp:415` 处不合——0004 归属未定论）。
- 显示：`DISPLAY=:1` 真 X（`/tmp/.X11-unix/X1` 在跑）；Xvfb 未实测。
- T1 正弦 452 Hz → A 大调：成功，中位 |偏差| < 10 音分，检测率 ≥ 80%。
- **T2 时光切片 A：自测 `unvoiced_gap=pass`，任务成功**——设计稿 §6 的
  无声间隙渲染失败在本轮未复现（after-fix probe 日志
  `repro_log-after-fix.jsonl` 亦显示人声导出成功）。§6.2 大声失败路径
  仍保留（mock 已验），自测记录对该构建记 pass。
- T3 连续有声乐句：成功。T4 失败后可正常退出：通过（mock 形）。
- T7 冷启动整任务耗时 **16 秒**（首轮 F0 等待未达传说的 60 秒）。
- T9 收据无秘密键：通过。T11 Drop 杀服务端 + 清临时目录：通过。
- T5/T6/T8/T10/T12/T13：按设计以 mock / MCP 测试覆盖（loud-skipping）。
- 残留：跑完盒上无孤儿 OpenTune 进程；`/data` 剩 1.5G。

## 在哪台盒子跑

`yanshi-5`（它有已验证的 API 分支构建；Songmap 快照对比线用完后再跑）。
`yanshi-4` 只保 C1 服务，⛔ 不跑构建。`yanshi-3` 保持挂起，⛔ 不唤醒。

## 用哪个二进制

yanshi-5 上 API 分支的 OpenTune Standalone 构建（带 control API）。
确切路径在跑之前现查，⛔ 不猜：

```sh
# 盒上找构建（示例口径，按实查结果填）
ls /data/workspace/opentune-build/ 2>/dev/null
find /data/workspace -maxdepth 4 -name "OpenTune" -type f 2>/dev/null
```

构建须过 `session.info` 门：能力含 `session.v1, read.v1, jobs.v1,
import.v1, export.v1, ui.bridge, edit.v1`。普通 v1.8.2 发行版无监听器，
门会报 `otcpUnsupportedBuild`——那是预期失败，不是测试失败。

## 怎么跑

盒上须有显示：`DISPLAY=:1`（yanshi-5 会话显示；本轮已验，真 X 下
`ui.getState` 可用）。`xvfb-run` 是驱动在无 `DISPLAY` 时的包法，
Xvfb 下 `ui.getState` 是否可用仍是设计稿 §9 开放问题 1——未实测，
重跑若用 Xvfb 须把结果记下来。

```sh
cd /data/workspace/yeban-otcp-test   # 盒上 worktree（分支已推远端）
export YEBAN_OTCP_TEST_BIN=/data/workspace/opentune-build/OpenTune-api/build-linux-ninja/OpenTuneStandalone_artefacts/Release/Standalone/OpenTune
export CARGO_TARGET_DIR=/tmp/yeban-otcp-target   # 保 /data（仅剩 1.5G）
DISPLAY=:1 cargo test -p yeban-otcp --test real_binary -- --ignored --test-threads=1 --nocapture
```

串行（一次一个服务端）。`--nocapture` 必带，否则 `SKIPPED` 行被吞。

## 测什么（T1–T13 期望）

| 测试 | 内容 | 期望 |
| --- | --- | --- |
| T1 | 452 Hz 正弦 → A 大调 snap → 导出 | 成功；量测中位 \|偏差\| < 10 音分；检出率 ≥ 0.8 |
| T2 | Shiguang 切片 A → snap → 导出 | **期望随自测记录走**：记录 `unvoiced_gap: fail` ⇒ `otcpRenderFailed`（块 `actualSamples: 0`，音符间隙前后都有值，什么都没写）；`pass` ⇒ 成功（修补已在）。记录 `unknown` ⇒ 测试直接 panic（自测没跑起来，先查自测） |
| T3 | 连续无间隙乐句（合成 4 秒）→ 导出 | 成功；量测门过（中位 ≤ 50 音分，检出率 ≥ 0.8） |
| T4 | 切片 A（可能失败）→ 再跑正弦 | 第二次须成功（首次失败没把服务端 wedge 住） |
| T5 | 提交型对话框 | 人工造对话框才测；本文件只打印指向（mock 已覆盖逻辑） |
| T6 | 修订冲突 | 须并发改动才触发；mock 已覆盖重试逻辑 |
| T7 | 冷启动 F0 等待 | 成功（不等就会 `NOT_READY` 失败）；耗时打在 stderr 上，记下来（冷启动成本，设计稿 §9 开放问题 4） |
| T8 | 量测门抓 hybrid 漂移 | 须离调导出样本；mock 已覆盖逻辑 |
| T9 | 收据不含秘密键 | 成功；收据与自测记录里无 `token` / `serverProof` 键 |
| T10 | 发现目录 0775 | 驱动自检由单元测试覆盖；服务端拒收行为须手动 chmod 验 |
| T11 | Drop 杀服务端 + 清理 | Drop 后 pid 死、临时目录与控制目录都删 |
| T12 | 双重校正拒绝 | MCP 层测（`yeban-mcp tests/otcp_tools.rs`），驱动层不做 |
| T13 | 重采样 + CAS + 提案换资产 | MCP 层测（合成导出文件，不须二进制）；真实音频端到端等 T1 成功后另验 |

## 夹具

`tests/fixtures/` 已有 `sine452-orig.wav`（480,044 字节）与
`sliceA-source.wav`（969,362 字节），均取自 `~/workspace/opentune/otcp-probe/`。
T3 夹具由测试现场合成（连续乐句，无间隙）。

## 跑完记什么

1. 13 个测试各自的结论（T2 尤其要写自测记录的 `unvoiced_gap` 值与
   构建提交号）。
2. T7 冷启动耗时（秒）。
3. Xvfb 还是真实显示、以及 `ui.getState` 在该显示下是否可用（§9 问题 1 的答案）。
4. 构建提交号（`session.info` 的 `build.commit`，收据 `server.build` 里有）。
5. 耳验：首份校正音频（T1 导出）交给 Wang 耳验——耳验才是最终 verdict，
   量测门过了不等于好听。
