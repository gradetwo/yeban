//! `yeban-tui` 二进制入口：参数解析、终端进出、事件循环。
//!
//! 用法：
//! - `yeban-tui [工程文件.yeban]` —— 无参数时打开内置演示工程。
//! - `yeban-tui --write-demo <path>` —— 把演示工程（含资产）写成 `.yeban`
//!   文件后退出（供脚本与人工试用取夹具，不进 TUI）。

use std::io::stdout;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use yeban_tui::app::App;
use yeban_tui::ui;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(path) = flag_value(&args, "--write-demo") {
        return write_demo(&path);
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "yeban-tui — 夜半终端界面\n\n用法:\n  yeban-tui [工程文件.yeban]      打开工程（无参数 = 演示工程）\n  yeban-tui --write-demo <path>   写出演示工程文件后退出\n\n键位：共享动作从 GUI（Space 播放/暂停、Enter 回起点、Ctrl+S 保存、\nM 静音、S 独奏、Ctrl+Alt+方向键混音微调）；Vi 键只作导航与模式\n（hjkl、gg/G、计数前缀、i 改名、: 命令行）。完整映射表见 crate README。"
        );
        return ExitCode::SUCCESS;
    }
    let positional = args.iter().find(|arg| !arg.starts_with("--")).cloned();
    let mut app = match positional {
        Some(path) => match App::open_file(std::path::Path::new(&path)) {
            Ok(app) => app,
            Err(error) => {
                eprintln!("打开工程失败：{error}");
                return ExitCode::FAILURE;
            }
        },
        None => match App::with_demo() {
            Ok(app) => app,
            Err(error) => {
                eprintln!("打开演示工程失败：{error}");
                return ExitCode::FAILURE;
            }
        },
    };
    match run(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("TUI 运行失败：{error}");
            ExitCode::FAILURE
        }
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<PathBuf> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).map(PathBuf::from)
}

/// 把演示工程写成 `.yeban` 文件（全保真归档路径，与 GUI 保存同一实现）。
fn write_demo(path: &std::path::Path) -> ExitCode {
    let mut app = match App::with_demo() {
        Ok(app) => app,
        Err(error) => {
            eprintln!("构建演示工程失败：{error}");
            return ExitCode::FAILURE;
        }
    };
    match app.save_to(path) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// 事件循环：轮询按键（16 ms 节拍）→ 推进引擎一跳 → 重绘。
fn run(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let result = event_loop(&mut terminal, app);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last = Instant::now();
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if app.quit {
            return Ok(());
        }
        let timeout = Duration::from_millis(16);
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        let now = Instant::now();
        let elapsed = now.duration_since(last);
        if elapsed >= Duration::from_millis(10) {
            app.tick(elapsed.as_secs_f64());
            last = now;
        }
    }
}
