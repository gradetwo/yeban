# baseline-headless-idle 工作线账本（`BASELINE-002` 的**真实取样对象**：`--headless-idle`）

> 分支 `line/baseline-headless-idle`，工作树 `.worktrees/baseline-headless-idle`（main `fe58fad`）。
> 地盘：`crates/yeban-app/**`；本文件新建。
> **禁改且一个字节都没碰**：`scripts/**`（测量脚本与纪律由集成者维护）、`.github/**`、`schemas/**`、
> 根 `Cargo.toml` / `Cargo.lock` / `deny.toml`、`docs/adr/**`、`docs/YEBAN_*.md`、
> `docs/DEVELOPMENT_LEDGER.md`、`docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md`、
> 其它 `crates/**`。
>
> 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:354` 逐字）：
>
> > | **[BASELINE-002]** | **常驻内存基线** | 空工程空闲内存占用 | 操作系统内存工作集（Working Set）统计 | **≤ 35 MB** | Phase 0 |
>
> 上一刀（`baseline-empty-sample`，已入 main）修好了「**空**工程」那一半；本刀修「**空闲常驻**」那一半：
> 此前 `--headless` 走 `cli::run_batch`，而那个模块的第一句就是「零 Slint 依赖」——
> **一个 Slint 对象都不构造**，于是规范所指的运行时（组件树 / 属性 / 回调 / 字形与光栅缓存）
> 从来没进过读数。

---

## 1. 结论（三句话）

1. 新增 `--headless-idle --idle-seconds N`：装**自研软件平台**（`MinimalSoftwareWindow` + `SoftwareRenderer`，
   **零新增依赖** —— 用的是 `yeban-app` 已经启用的 `renderer-software`），构造**真** `MainWindow`
   （唯一注入点仍是 `host::build_main_window`，D28），**逐行光栅化一帧**当见证，然后空闲 N 秒退出。
2. **本机（M2，debug）并排实测**：`--headless` = **10.05 / 10.05 MB**，
   `--headless-idle --idle-seconds 2` = **34.34 / 34.28 MB** ⇒ **差 +24.3 MB（3.4 倍）**。
   这个读数**落在**上一条线记录的 `11.14 → 55.97 MB` 量级带**之内**（55.97 = Tier-1 端口建一个真窗口
   的读数，`docs/ledger/baseline-memory-notes.md` §3.2）。
3. 空闲**不攀升**：`--idle-seconds` 取 1 / 2 / 6 / 6 秒，四个读数 **34.48 / 34.34 / 34.14 / 34.27 MB**
   （极差 **0.34 MB**，且 6 秒**低于** 1 秒 ⇒ 无单调攀升）。⇒ 判据 ③ 成立，阈值依据见 §4。

**一句话给集成者**：规范那句「空工程空闲常驻」现在**第一次**被量到了 —— 本机 debug 下 **34.3 MB**，
卡在 35 MB 线上（6 轨演示工程同法是 **35.2 MB，已 over-target**）。

---

## 2. 见证：凭什么说「真的建了控件树」（不是空转）

`--headless-idle` 打出的见证行（本机原始输出，两次运行**逐字符相同**）：

```text
headless-idle ok
project-source: sample=empty (内置空工程 0 轨; 未给 --open ⇒ 未读任何文件)
project: id=00000000000000000000000000 bpm=120.00 ts=4/4 title="Untitled"
project-counts: tracks-all=0 master-track=0 scenes=0 sections=0 clips-pool=0 midi-notes=0 assets-indexed=0 asset-blobs=0 history-bytes=0
view-counts: tracks=0 master=0 clips=0 notes=0 sections=0 scenes=0 elements=99 dynamic-regions=8
headless-idle-witness: windows-created=1 size=1920x1080 rendered=true lines=1080 non-black-pixels=2073600 non-black-permille=1000 distinct-colors=2206 elements=99
headless-idle-idle: seconds=2 elapsed-ms=2012 ticks=136
headless-idle: 已构造真 MainWindow + 逐行光栅化一帧; 未创建 OS 窗口 (平台 = MinimalSoftwareWindow/SoftwareRenderer)、未进阻塞事件循环、未连声卡/未建引擎; 运行时控件树遍历仍属 yeban-ui-test-port 的 testing backend
```

逐字段的**含义**与**为什么它伪造不了**：

| 字段 | 实测 | 它证明了什么 |
| :--- | :--- | :--- |
| `windows-created` | **1** | 平台自己的 `create_window_adapter` 被调用次数。0 ⇒ Slint 连窗口都没要，谈何控件树 |
| `rendered` | **true** | `MinimalSoftwareWindow::draw_if_needed` 的返回值（先显式 `request_redraw`，否则永远 false） |
| `lines` | **1080** | 逐行光栅化回调覆盖了**整个窗口高度**（= `PhysicalSize::height`），不是画了一条线就收工 |
| `size` | **1920x1080** | 与 GUI / Tier-1 端口**同源**（`DemoScene.viewport_*` = `[UI-GRID-002]` 全展开档） |
| `non-black-pixels` | **2073600** | = 1920×1080（全部像素）。像素是伪造不了的：只有真存在一棵被布局过的活控件树，`SoftwareRenderer` 才吐得出像素 |
| `distinct-colors` | **2206** | 这一项才是**判别力**所在：`non-black` 在本主题下饱和（1000‰），而 2206 种颜色说明真的画了多图元 + 文本抗锯齿，而不是一块纯色 |
| `elements` | **99** | 复用既有 `view-counts:` 的**同一份** `ElementRegistry` ⇒ 与 `--headless` 的元素数**可比**（判据里断言两行同值） |

**为什么是「逐行」而不是整帧 `render()`**：整帧缓冲 `1920×1080×3B ≈ 5.93 MiB` 属于 **Tier-1 截图设施**的账，
`baseline-memory-notes.md` §4.1 第 2 条明确写了「别把这 80 MB 记到空工程空闲占用上」。
`render_by_line` 的光栅化工作量与整帧完全相同（同一个 `render_window_frame_by_line`），
但只保留**一行**缓冲 ⇒ 读数贴近「控件树常驻」而不是「截图设施常驻」。

---

## 3. 判据 ①②：并排读数与差（同一仪器、同一样本）

仪器：`scripts/gates/measure_rss.py`（集成者地盘，**只读未改**），
RSS = `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss`，**一条命令一个进程**。

| # | 命令（`$APP` = `target/debug/yeban-app`） | 读数 MB | 与 `--headless` 的差 | verdict |
| ---: | :--- | ---: | ---: | :--- |
| 1 | `--project-sample empty --headless` | **10.05** | — | within-target |
| 2 | 同上（重复） | **10.05** | — | within-target |
| 3 | `--project-sample empty --headless-idle --idle-seconds 2` | **34.34** | **+24.29** | within-target |
| 4 | 同上（重复） | **34.28** | **+24.23** | within-target |
| 5 | `--project-sample default --headless`（6 轨，对照） | 11.42 | — | within-target |
| 6 | `--project-sample default --headless-idle --idle-seconds 2`（对照） | **35.23** | — | **over-target** |

**读法**（判据 ② 的判据句）：`--headless-idle` 必须**明显高于** `--headless`。
- 实测 **+24.3 MB / 3.4 倍** —— 既不是「太低」（10.x ⇒ 没建树），也不是「不变」（⇒ 开关没生效）；
- 落点 **34.3 MB** 在既有量级带 `11.14 → 55.97` 之**内**（该带的两端都来自**本机同一条仪器**，
  见 `baseline-memory-notes.md` §3.2 / §4 消融表第 0 行与第 2 行）；
- 旁证（判据 ② 的「不许换口径达标」）：**6 轨演示工程在同一模式下 35.23 MB ⇒ 已 over-target**，
  说明这条读数**随被量对象变化**，不是恒定值、也不是被仪器地板钉住的数。
- 两次重复的极差 **0.06 MB**（#3 vs #4）⇒ 读数可复现，不是抖动。

---

## 4. 判据 ③：空闲 N 秒之后读数**不再攀升**（含阈值依据）

| 空闲秒数 | 读数 MB | 说明 |
| ---: | ---: | :--- |
| 1 | **34.48** | 最短档 |
| 2 | **34.34 / 34.28** | 两次重复（极差 0.06） |
| 6 | **34.14 / 34.27** | 空闲时长 ×3 |

**极差 = 34.48 − 34.14 = 0.34 MB（约 1.0%）**，且**6 秒档低于 1 秒档** ⇒ 没有随空闲时长单调攀升的形状。

**阈值依据（写下来，不是拍的）**：判据句需要「在噪声内相等」，于是阈值必须来自**同一批读数**的噪声本身：
① 同一条命令两次运行的极差 = **0.06 MB**（#3 vs #4，§3）；
② 空闲时长从 1 s 拉到 6 s（+5 s，且期间**只**做 `sleep`、不重绘、不分配）的极差 = **0.34 MB**。
取 **|Δ| ≤ 0.5 MB（≈1.5%）判为「噪声内相等」**：它比观测极差（0.34）宽一点，留了余量；
而一个真的「空闲期分配/泄漏」在这 5 秒里会表现为**单调上升**（本主题下 UI 每帧几十 KB～MB 级），
不会被 0.5 MB 这个宽度吞掉。

**边界（必须与读数一起引用）**：`ru_maxrss` 是**峰值**高水位，不是稳态采样（仪器文件头自己写明）。
本档空闲期间**刻意不跑事件循环、不重绘、不分配**（见 `src/headless_idle.rs` 的 ④ 段注释），
因此「空闲 N 秒」量的是「控件树建好之后静静待着」的常驻集，而不是「事件循环在忙」的峰值。

---

## 5. 判据 ④：门禁与测试（本机真跑）

```bash
# 两条都必须在**本工作树**里跑（cargo-local.sh L28-38 会把工作区切到当前 git 顶层）
bash scripts/dev/cargo-local.sh test -p yeban-app        # exit 0
bash scripts/gates/run-gates.sh light                    # 门禁通过 (mode=light)
```

| 判据 | 命令 | 结果 |
| :--- | :--- | :--- |
| ④a | `bash scripts/dev/cargo-local.sh test -p yeban-app` | **exit 0**：本地单测 + 集成（含新增 `headless_idle_builds_a_real_control_tree_and_reports_a_non_trivial_witness`）全绿 |
| ④b | `bash scripts/gates/run-gates.sh light` | **门禁通过 (mode=light)**：fmt / 14 条守卫 / 7 条文档契约 / 许可清单 |
| ④c | `cargo clippy -p yeban-app --all-targets --no-deps -- -D warnings` | 本机零告警（`--no-deps` 是为了不重编译 Slint；CI 的 workspace 全量 clippy 是权威） |

**编译纪律**：本机**没有**做首次全量构建 —— 复用主仓 `target/`（`CARGO_TARGET_DIR=/Users/crow/work/music/yeban/target`）
的增量缓存，Slint 依赖全部命中缓存（`cargo check` 2.48 s、`cargo build` 8.99 s 完成）。**Cargo.toml / Cargo.lock 一字未改**。

---

## 6. 复跑命令（逐字可复制）

```bash
cd /Users/crow/work/music/yeban/.worktrees/baseline-headless-idle
CARGO_TARGET_DIR=/Users/crow/work/music/yeban/target bash scripts/dev/cargo-local.sh build -p yeban-app
APP=/Users/crow/work/music/yeban/target/debug/yeban-app
M="python3 scripts/gates/measure_rss.py"

$APP --project-sample empty --headless                          # 见证：headless ok + view-counts elements=99
$APP --project-sample empty --headless-idle --idle-seconds 2    # 见证：headless-idle-witness + headless-idle-idle

$M --label headless-empty   --timeout 180 -- $APP --project-sample empty --headless
$M --label idle2-empty      --timeout 180 -- $APP --project-sample empty --headless-idle --idle-seconds 2
$M --label idle6-empty      --timeout 180 -- $APP --project-sample empty --headless-idle --idle-seconds 6
```

---

## 7. 本机 vs CI（**严格区分**，不许含混）

| 项 | 本机（M2，debug） | CI |
| :--- | :--- | :--- |
| `--headless-idle` 能跑 + 见证 | ✅ 本文件 §2 的原始输出 | ✅ 由新增集成判据在 `cargo test -p yeban-app` 里机械断言（真二进制） |
| 并排读数 / 差 / 不攀升 | ✅ §3 / §4（`measure_rss.py`） | ❌ 本刀**未**接线到 `bench` 档（`scripts/**` 是集成者地盘，本线禁改）⇒ 见 §8 needs-1 |
| **release** 列 | ⛔ **本机不可得**（release 需首次全量构建，违反本机纪律）⇒ **如实 SKIP** | 未取（需要 `bench`/`gates-manual` 档） |
| 达标判定（≤35 MB） | 仅 debug：**34.3 MB**（空工程） | 待参考机复跑（规范原话要求） |

---

## 8. needs（明确交出去，不假装闭环）

1. **把 `--headless-idle` 接进 `measure_rss.py` 的调用点**（`bench` 档 / `gates-manual.yml`）——
   `scripts/**` 与 `.github/**` 是集成者地盘，本线**一个字节都没碰**。建议标签与命令：
   `--label app-idle-empty -- $APP --project-sample empty --headless-idle --idle-seconds N`（N 建议 ≥ 5，
   让「空闲不攀升」在 CI 上也可读）。
2. **release 列**：本机不可得（首次全量构建违反本机纪律）。需要 CI 上一条 release 构建 + 同一仪器。
3. **运行时控件树遍历**（元素数**从活的 Slint item tree** 数出来，而不是从 `ElementRegistry` 投影数）：
   那需要 `crates/yeban-ui-test-port` 的 testing backend（`i-slint-backend-testing`），
   属另一条线；本刀刻意不引（零新增依赖）。本刀的替代见证是**像素**（§2），它能证明「树真的被布局并光栅化」，
   但不能给出「运行时控件数」这个数。
4. **`--headless-idle` 与 `--save-as` 等组合**：当前**明确拒收**（用法错误，退出码 2），
   理由是「不许静默丢掉写盘/导出」。若将来确实要组合，必须先写清顺序契约再放开。

---

## 9. 净行数（相对 main `fe58fad`）

| 文件 | 变更 |
| :--- | ---: |
| `crates/yeban-app/src/headless_idle.rs`（新增） | +264 |
| `crates/yeban-app/src/cli.rs` | +467 / −8 |
| `crates/yeban-app/tests/cli_contract.rs` | +173 / −1 |
| `crates/yeban-app/src/lib.rs` | +1 |
| `crates/yeban-app/src/main.rs` | +10 / −4 |
| **合计** | **+915 / −13**（含本文件另计） |

**CI 判决**：本轮推送后由集成者读回（本文件**不**在同一提交里追写判决 ——
「等判决期间不推送」是 `docs/CI_CD.md` 的纪律，追写会触发新 run 把在飞证据掐掉）。
