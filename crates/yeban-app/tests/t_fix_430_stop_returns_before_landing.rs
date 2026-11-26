//! **T-FIX-430 行为判据**：停录回调**先返回**，量化＋写盘在后台线程完成。
//!
//! 静态结构契约在 `t_fix_430_async_record_landing.rs`（`include_str!` 判字串）。本文件补
//! 票面 §3 要的**行为**判据：用可注入的**长帧源**（60 s × 48 kHz 立体声 = 288 万帧）在
//! UI 测试端口触发停录，逐条断言
//!
//! | # | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | ① | `invoke_toggle_record()` 停录调用耗时 < 50 ms | 量化/写盘**不在** UI 主线程（修前同步实现要搬 23 MB 样本＋17 MB WAV，debug 下远超 50 ms） |
//! | ② | 停录返回那一刻 WAV 字节数 < 最终字节数 | 写盘**还没做完**就返回了：重活确实在之后的后台线程里（⛔ 不是"先写完再返回"） |
//! | ③ | 停录后 pump 事件循环，工程里恰好落 1 条摆放 | 后台结果由定时器回收，⛔ 不丢、不重复落 |
//! | ④ | 回读 WAV 帧数 == 源吐出的帧数 | 异步化**没有**为了快而丢帧（⛔ 不是"少录一段"换来的不卡） |
//!
//! ## 反向腿
//!
//! 把 `land_sink(` 从 `thread::spawn` 闭包里挪回 `stop_async` 体内（即退回同步实现）：
//! ① 立刻变红（停录耗时升到几百 ms），② 变红（返回时文件已完整）。实证记录见回执。
//!
//! ## 无头（⛔ 不碰声卡）
//!
//! 源是脚本化阶梯波形（与 `record_places_clip_on_track.rs` 同一条口径），WAV 落
//! `std::env::temp_dir()`，`Drop` 清理。无头平台不跑事件循环 ⇒ 落地靠主动
//! `update_timers_and_animations()` 驱动（生产由 Slint 事件循环自动驱动，同一份 `Timer`）。

#![allow(unused)]
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::record_device::InputSourceFactory;
use yeban_app::record_wav::{InputSource, RecordOptions, RecordPlan};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, YebanProjectV1};

const NOW: u64 = 1_760_000_000_000;
const WINDOW: yeban_ui_test_port::Size = yeban_ui_test_port::Size::new(1440, 900);
const PLAYHEAD_TICK: u64 = 960;

/// 一次 take 的目标长度：60 s × 48 kHz = 288 万帧（票面举例的"60 s 立体声"）。
const TAKE_SECONDS: u64 = 60;
const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const TAKE_FRAMES: u64 = RATE as u64 * TAKE_SECONDS;

/// 停录回调的耗时上限（ms）。同步实现下 288 万帧量化＋17 MB 写盘在 debug 构建里是几百 ms
/// 量级，两条腿差一个数量级以上 ⇒ 50 ms 这条线不是"卡得很紧的巧合"。
const STOP_BUDGET_MS: u128 = 50;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-430-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 建环参数：一块 = 1 s 音频（48 000 帧），环容 2 s。
///
/// ⚠ 块**故意**比默认 4096 大得多：脚本源是瞬时产出，worker 每轮只 sleep 1 ms ⇒ 60 块
/// 大约 100 ms 就能把整段 60 s 喂满（⛔ 不用真的等 60 s 墙钟）。
fn options() -> RecordOptions {
    RecordOptions {
        seconds: 1.0,
        sample_rate: RATE,
        channels: CHANNELS,
        capacity_frames: 96_000,
        drain_batch: 48_000,
        ..RecordOptions::default()
    }
}

fn fixture_project() -> YebanProjectV1 {
    let mut project = filled_project();
    project.transport.count_in_bars = 0;
    for track in project.tracks.values_mut() {
        track.clips.clear();
    }
    project
}

/// 全工程摆放总数。
fn placements_total(port: &Rc<UndoPort>) -> usize {
    port.project()
        .tracks
        .values()
        .map(|track| track.clips.len())
        .sum()
}

/// 脚本化长帧源：**一次吐满 `frames` 帧就返回 0**（⛔ 不循环、不无限供帧）。
///
/// 返回 0 之后 worker 只 sleep，不再推帧 ⇒ `pulled_frames()` 稳定在总量，判据可以
/// 精确等它到齐再停录。
struct LongTakeSource {
    remaining: u64,
    frame: u64,
}

impl InputSource for LongTakeSource {
    fn channels(&self) -> u16 {
        CHANNELS
    }

    fn next_block(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let want = (frames as u64).min(self.remaining) as usize;
        if want == 0 {
            return 0;
        }
        let channels = usize::from(CHANNELS);
        for index in 0..want {
            // 阶梯非零值：既不是静音腿，也不依赖任何设备。
            let value = ((self.frame + index as u64) % 32) as f32 / 64.0;
            for channel in 0..channels {
                out[index * channels + channel] = value;
            }
        }
        self.frame += want as u64;
        self.remaining -= want as u64;
        want
    }

    fn label(&self) -> &'static str {
        "t-fix-430-long-take(60s-48k-stereo)"
    }
}

struct LongTakeFactory {
    frames: u64,
}

impl InputSourceFactory for LongTakeFactory {
    fn open(&self, options: &RecordOptions) -> Result<Box<dyn InputSource + Send>, String> {
        assert_eq!(options.channels, CHANNELS, "判据只按立体声建夹具");
        Ok(Box::new(LongTakeSource {
            remaining: self.frames,
            frame: 0,
        }))
    }

    fn label(&self) -> &'static str {
        "t-fix-430-long-take-factory"
    }
}

fn assemble(
    project: YebanProjectV1,
) -> (
    yeban_ui_test_port::render::Tier1Window,
    yeban_app::ui::MainWindow,
    Rc<UndoPort>,
) {
    let tier1 =
        yeban_ui_test_port::render::Tier1Window::install(WINDOW).expect("装 Tier-1 无头平台");
    let view = ViewState::from_project(&project).expect("装配时投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window(&view, &scene).expect("生产窗口构造入口");
    tier1
        .resize(WINDOW)
        .expect("窗口尺寸定下来之后再注入一次布局");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("t430-fast-stop", "yeban-app", project, NOW).expect("打开撤销会话"),
    ));
    (tier1, ui, port)
}

/// ⭐ 主判据：停录**先返回**，量化＋写盘在后台完成，且整段一帧不少。
#[test]
fn stop_returns_before_the_quantize_and_write_finish() {
    let scratch = Scratch::new("fast-stop");
    let path = scratch.dir.join("long-take.wav");
    let project = fixture_project();
    let (_tier1, ui, port) = assemble(project.clone());

    let recorder = host::wire_record_at(
        &ui,
        Some(Rc::new(RefCell::new(project.clone()))),
        Some(Rc::clone(&port)),
        RecordPlan {
            path: path.clone(),
            options: options(),
        },
        Rc::new(LongTakeFactory {
            frames: TAKE_FRAMES,
        }),
        Rc::new(host::UiRecordOutcome),
        Rc::new(|| PLAYHEAD_TICK),
    );

    ui.invoke_toggle_record();
    assert!(ui.get_recording(), "起录必须成功（工厂能开）");

    // ① 等源把整段吐完：worker 每轮一块（1 s）＋ 1 ms 休眠 ⇒ 60 轮量级。
    let pull_deadline = Instant::now() + Duration::from_secs(15);
    while recorder.borrow().pulled_frames() < TAKE_FRAMES {
        assert!(
            Instant::now() < pull_deadline,
            "脚本源 15 s 内没吐完 {TAKE_FRAMES} 帧（pulled={}）：夹具本身失效",
            recorder.borrow().pulled_frames()
        );
        std::thread::sleep(Duration::from_millis(4));
    }
    let pulled = recorder.borrow().pulled_frames();

    // ② 停录：这一刻只许"停线程 + 把落地任务交出去"。
    let stop_began = Instant::now();
    ui.invoke_toggle_record();
    let stop_ms = stop_began.elapsed().as_millis();

    assert!(!ui.get_recording(), "停录后界面 recording 必须立刻归零");
    assert!(
        stop_ms < STOP_BUDGET_MS,
        "停录回调耗时 {stop_ms} ms ≥ {STOP_BUDGET_MS} ms：量化/写盘仍在 UI 主线程（T-FIX-430 回退）"
    );

    // ③ 停录返回那一刻：写盘还没做完（重活确实排到了后台）。
    let bytes_at_stop = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);

    // ④ 后台落地：pump 事件循环直到工程里出现摆放。
    let land_began = Instant::now();
    let mut pumps: u32 = 0;
    let settle = Instant::now() + Duration::from_secs(60);
    while Instant::now() < settle {
        slint::platform::update_timers_and_animations();
        pumps = pumps.saturating_add(1);
        if placements_total(&port) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let land_ms = land_began.elapsed().as_millis();

    assert!(
        pumps > 0,
        "落地期间事件循环必须能继续转（⛔ 不能死等 join）"
    );
    assert_eq!(placements_total(&port), 1, "一次停录必须恰好落一条摆放");

    // ⑤ 整段都落了：帧数与源吐出的总量逐字相等（⛔ 不为"快"丢帧）。
    let final_bytes = std::fs::metadata(&path)
        .expect("落地后必须读得到 WAV")
        .len();
    assert!(
        bytes_at_stop < final_bytes,
        "停录返回时 WAV 已是最终大小（{bytes_at_stop} vs {final_bytes}）：写盘仍在调用线程完成"
    );
    let (format, buffer) = yeban_render::wav::read_plain_wav(&path).expect("读回 WAV");
    assert_eq!(format.channels, CHANNELS, "声道数必须与源一致");
    assert_eq!(format.sample_rate, RATE, "采样率必须与源一致");
    let frames = buffer.len() as u64 / u64::from(format.channels);
    assert_eq!(
        frames, pulled,
        "落盘帧数（{frames}）必须等于源吐出帧数（{pulled}）：异步化不许丢帧"
    );
    // ⚠ 头长不是死板的 44：24-bit 的 `fmt ` 块带扩展（hound 写 68 字节头）。只钉
    // "数据区正好是 frames × channels × 3" 且头部开销在小范围内，⛔ 不假装头就是 44。
    let data_bytes = frames * u64::from(format.channels) * 3;
    assert!(
        final_bytes >= data_bytes + 44,
        "WAV 比数据区还小（{final_bytes} < {data_bytes} + 头）：落盘被截断"
    );
    assert!(
        final_bytes - data_bytes < 512,
        "WAV 头部开销 {} 字节异常（data={data_bytes} 文件={final_bytes}）",
        final_bytes - data_bytes
    );

    // 让人肉复核时能看出两条腿的量级差（停录 vs 落地）。
    println!(
        "[T-FIX-430] stop={stop_ms}ms land={land_ms}ms pumps={pumps} frames={frames} \
         bytes_at_stop={bytes_at_stop} final={final_bytes}"
    );

    // ⛔ 反向腿提示（留在日志里，便于复核者按票面复现）：
    // 把 `std::thread::spawn(move || land_sink(` 改回 `stop_async` 体内直接 `land_sink(`，
    // 本判据会在 stop_ms 断言处变红。
}
