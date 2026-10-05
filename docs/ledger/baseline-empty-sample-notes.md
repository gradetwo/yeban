# baseline-empty-sample 工作线账本（`--project-sample empty` —— 把 `BASELINE-002` 的取样对象交出来）

> 分支 `line/baseline-memory-measure`，工作树 `.worktrees/baseline-memory-measure`（main `5b279cb`）。
> 地盘：`crates/yeban-app/**`（`src/cli.rs` / `src/main.rs` / `tests/cli_contract.rs`）+ 本文件。
>
> **一个字节都没碰**：`scripts/**`（含 `scripts/gates/measure_rss.py` —— 仪器归集成者）、
> `.github/**`、`schemas/**`、根 `Cargo.toml` / `Cargo.lock`、`deny.toml`、`docs/adr/**`、
> `docs/YEBAN_*.md`、`docs/DEVELOPMENT_LEDGER.md`、`README*.md`、法务文件、其它 `crates/**`。
> **零新增依赖**：`Cargo.lock` 无改动（`git status` 里只有上面三个 `.rs` 与本文件）。

---

## 1. 本轮只做一件事

规范 `BASELINE-002` 的判据句是"**空**工程空闲常驻内存 ≤ 35 MB"
（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:354`），而本轮之前
`--project-sample` 只有 `default`（`bridge::demo_project()`，6 条普通轨 + 主总线）
与 `filled`（更重的规范样本）—— **两者都不是"空工程"** ⇒ 仪器量得到，但量的不是规范的对象。

本轮把那个对象**交出来**：`--project-sample empty` = `yeban_model::samples::default_project()`
（即 `YebanProjectV1::default()`，模型侧**合法且可读**的 0 轨空工程）。

改动（净 +171 / −10，三个文件）：

| 文件 | 改了什么 |
| :--- | :--- |
| `crates/yeban-app/src/cli.rs` | `Sample::Empty` 变体（`name()` / `project()` / `origin_label()`）、`parse` 接受 `empty`、`UnknownSample` 的可用集合加 `empty`、用法文本、两条判据（判据 37b） |
| `crates/yeban-app/src/main.rs` | 模块文档的运行形态表补 `empty` |
| `crates/yeban-app/tests/cli_contract.rs` | 真二进制判据 B3b：`empty` = 0 轨、`default` = 6 条普通轨 |

**没有第二套实现**：`empty` 走的是与 `default` / `filled` **逐字相同**的那一条
`Sample::project()` → `load_project` → `bridge::from_project` → `host::apply_view` 数据流，
没有任何"空工程特判"。它也不是手写夹具 —— 就是模型侧导出给契约对账用的那份空工程样本
（`samples::default_project()`，判据 `empty_sample_is_a_zero_track_project_with_witness`
用 `assert_eq!(empty, YebanProjectV1::default())` 钉住这一点）。

**为什么改了一句输出措辞**：来源行原先把**所有**样本都写成"内置演示工程"。
对 0 轨空工程那句话是**假的**（本仓库第一条纪律是不写假话），
因此新增 `Sample::origin_label()`：`empty` → `内置空工程 0 轨`，
而 `default` / `filled` 的措辞**逐字未变**（改它们要同步别的线与账本，超范围）。

---

## 2. 见证（轨道数 —— 证明模式生效，不是空转）

```bash
cd /Users/crow/work/music/yeban/.worktrees/baseline-memory-measure
APP=/Users/crow/work/music/yeban/target/debug/yeban-app
$APP --headless --project-sample default   # ② 对照
$APP --headless --project-sample empty     # ① 见证
```

实测输出（**本机 M2，2026-10-05，本轮 debug 二进制**）：

```text
$ $APP --headless --project-sample default
headless ok
project-source: sample=default (内置演示工程; 未给 --open ⇒ 未读任何文件)
project: id=01J8Z5Q0R7K3M9X2V4B6N8P0P0 bpm=120.00 ts=4/4 title="夜半 Yeban"
project-counts: tracks-all=7 master-track=1 scenes=4 sections=4 clips-pool=2 midi-notes=6 assets-indexed=0 asset-blobs=0 history-bytes=0
view-counts: tracks=6 master=1 clips=3 notes=6 sections=4 scenes=4 elements=215 dynamic-regions=14
headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— 控件树断言需要 crates/yeban-ui-test-port 的 testing backend

$ $APP --headless --project-sample empty
headless ok
project-source: sample=empty (内置空工程 0 轨; 未给 --open ⇒ 未读任何文件)
project: id=00000000000000000000000000 bpm=120.00 ts=4/4 title="Untitled"
project-counts: tracks-all=0 master-track=0 scenes=0 sections=0 clips-pool=0 midi-notes=0 assets-indexed=0 asset-blobs=0 history-bytes=0
view-counts: tracks=0 master=0 clips=0 notes=0 sections=0 scenes=0 elements=99 dynamic-regions=8
headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— 控件树断言需要 crates/yeban-ui-test-port 的 testing backend
```

**怎么读这两个数**（口径必须写明，否则见证会被读错）：
`project-counts:` 的 `tracks-all` **含主总线**（`cli.rs` 的 `ProjectCounts::tracks` 就是
`project.tracks.len()`）。因此：

| 样本 | `tracks-all` | `master-track` | 普通轨 = 差 | 见证 |
| :--- | ---: | ---: | ---: | :--- |
| `empty` | **0** | 0 | **0** | ① 真的是 0 轨空工程 |
| `default` | **7** | 1 | **6** | ② 6 轨演示工程（对照） |

第二条独立见证是 `view-counts: tracks=`（**投影侧**数出来的非主总线轨道）：
`empty` = 0 / `default` = 6 —— 说明空工程**真的被投影过**
（`elements=99` 而不是 0，也不是"投影失败被吞掉"），而不是被某个特判跳过了。

---

## 3. 两条读数并排（`scripts/gates/measure_rss.py`，**未改该脚本**）

仪器本身就是"一条命令一个进程"（`ru_maxrss` 是 `RUSAGE_CHILDREN` 的**高水位**，
见脚本文件头）⇒ **并排读数 = 对同一条命令分别调用两次**。
`measure_rss.py` 的 `--label` 就是为这件事准备的，因此本轮**不需要**改 `scripts/**`。

```bash
cd /Users/crow/work/music/yeban/.worktrees/baseline-memory-measure
APP=/Users/crow/work/music/yeban/target/debug/yeban-app
for s in default empty; do
  python3 scripts/gates/measure_rss.py --label "app-headless-$s" --timeout 120 -- \
    $APP --headless --project-sample $s
done
```

连跑三轮的读数（**同一台机、同一条二进制、同一秒级的重复性检查**）：

```text
BENCH baseline=002 label=app-headless-default peak_rss_mb=11.44 target_mb=35.0 child_exit=0 verdict=within-target
BENCH baseline=002 label=app-headless-empty   peak_rss_mb=9.98  target_mb=35.0 child_exit=0 verdict=within-target
BENCH baseline=002 label=app-headless-default peak_rss_mb=11.38 target_mb=35.0 child_exit=0 verdict=within-target
BENCH baseline=002 label=app-headless-empty   peak_rss_mb=9.98  target_mb=35.0 child_exit=0 verdict=within-target
BENCH baseline=002 label=app-headless-default peak_rss_mb=11.42 target_mb=35.0 child_exit=0 verdict=within-target
BENCH baseline=002 label=app-headless-empty   peak_rss_mb=9.97  target_mb=35.0 child_exit=0 verdict=within-target
```

| 标签 | 三轮读数 MB | 极差 | verdict |
| :--- | :--- | ---: | :--- |
| `app-headless-default`（6 轨演示，现行判据） | 11.44 / 11.38 / 11.42 | 0.06 | within-target |
| `app-headless-empty`（**规范所指的对象**） | 9.98 / 9.98 / 9.97 | 0.01 | within-target |

⇒ 空工程比 6 轨演示工程轻 **≈ 1.4 MB**（同一进程、同一条命令、只差 `--project-sample`）。
两个读数都**远在 35 MB 以内**，且**都打印、都可复跑**。

---

## 4. 判据（本机全部真跑过）

| # | 判据 | 命令（本机真跑） | 结果 |
| ---: | :--- | :--- | :--- |
| ① | `empty` 的轨道数 = 0（见证） | `$APP --headless --project-sample empty` | `tracks-all=0 master-track=0`、`view-counts: tracks=0` ✅ |
| ② | `default` 的轨道数 = 6（对照） | `$APP --headless --project-sample default` | `tracks-all=7 master-track=1` ⇒ 普通轨 6、`view-counts: tracks=6` ✅ |
| ③ | 两条读数都打印且可复跑 | 上面 §3 的 `for` 循环（各 3 轮） | `peak_rss_mb=11.4x` / `9.9x`，`verdict=within-target` ✅ |
| ④ | `run-gates.sh light` 通过 | `bash scripts/gates/run-gates.sh light` | `门禁通过 (mode=light)`，EXIT=0，14 条守卫全绿 ✅ |
| ⑤ | 判据进代码（不是只跑一次） | `cargo-local.sh test -p yeban-app --lib empty_sample`／`--test cli_contract empty_sample` | `1 passed` / `1 passed` ✅ |
| ⑥ | 整个 app crate 的判据 | `cargo-local.sh test -p yeban-app` | 退出码 0（含 `cli_contract` 15 条、`real_ui_tier1` 15 条、`live_ui_mcp` 16 条、`undo_wiring_ui` 10 条、doc-tests 0） ✅ |
| ⑦ | 编译纪律 | `CARGO_TARGET_DIR=/Users/crow/work/music/yeban/target bash scripts/dev/cargo-local.sh build -p yeban-app --bin yeban-app` | **28.5s** 完成（复用主仓缓存的 Slint 等 registry 依赖 ⇒ 不是首次全量构建） ✅ |

**新增的两条判据在哪**：
`crates/yeban-app/src/cli.rs` 的 `cli::tests::empty_sample_is_a_zero_track_project_with_witness`
（样本 0 轨 + `validate()` + 两条读数不同 = 不空转）与
`crates/yeban-app/tests/cli_contract.rs` 的 `empty_sample_reports_zero_tracks_and_default_reports_six`
（**真二进制**上的 argv / stdout，`tracks-all - master-track = 6`）。

---

## 5. 本机 vs CI

- **本机（M2，已跑）**：§4 的 ①–⑦。所有 app 读数都是 **debug** 二进制
  （`/Users/crow/work/music/yeban/target/debug/yeban-app`）。
- **CI（判决）**：本轮推送后由 `scripts/dev/ci-verdict.sh` 读回（`steps` 数一并记录，
  `steps=0` 的"空心绿"不算判决）。release 列在本机不可得（release 下 Slint 是首次全量构建，
  纪律禁止）⇒ release 属下一轮（或 CI 的 `bench` 档）。
- 取 CI 日志用 `XDG_CACHE_HOME=/Users/crow/work/music/.cache gh …`（`gh` 缓存不进仓库）。

---

## 6. 诚实边界（**必须**与读数一起引用）

1. 这里读的是**峰值 RSS**，不是规范字面的"空闲内存 / Working Set"。
   对 `--headless` 这种短命进程二者接近，但**不等价**；真正的"空闲常驻"要在规范指定的
   参考机上、用规范指定的仪器复跑。
2. 托管 runner / 本机 debug 的**绝对值只作数量级参考**，跨平台、跨 libc、跨 profile 不可直接相比。
3. 本轮**交的是取样对象，不是达标结论**：`empty` 让 `BASELINE-002` 第一次可以被量到
   规范所指的那个对象；"是否达标"仍须在参考机上用同一条命令复跑后才有结论。
   `gate-status.md` 里 `BASELINE-002` 的状态**不由本线改写**（那是集成者地盘）。

## 7. 明确留在范围外（下一轮/集成者）

- `--headless-idle`（"空闲"语义的独立开关）与 settle 后再采样的稳定值；
- release 列 / 参考机复跑 / CI `bench` 档的正式读数；
- `scripts/**` 的任何改动（若集成者要让 `measure_rss.py` 一次跑两条命令**并排**打印，
  本线提供的两个 `--label` 取值与命令已在 §3 就绪）；
- `gate-status.md` / `docs/DEVELOPMENT_LEDGER.md` 的记账。
