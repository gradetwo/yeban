//! `yeban-tui` 二进制入口：参数解析、终端进出、事件循环。
//!
//! 用法：
//! - `yeban-tui [工程文件.yeban]` —— 无参数时打开内置演示工程。
//! - `yeban-tui --write-demo <path>` —— 把演示工程（含资产）写成 `.yeban`
//!   文件后退出（供脚本与人工试用取夹具，不进 TUI）。

use std::io::stdout;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use yeban_tui::app::{App, METER_HEARTBEAT_MS};
use yeban_tui::ui;

static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

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

trait TerminalOps {
    fn enable_raw_mode(&mut self) -> std::io::Result<()>;
    fn disable_raw_mode(&mut self) -> std::io::Result<()>;
    fn enter_alternate_screen(&mut self) -> std::io::Result<()>;
    fn leave_alternate_screen(&mut self) -> std::io::Result<()>;
    fn hide_cursor(&mut self) -> std::io::Result<()>;
    fn show_cursor(&mut self) -> std::io::Result<()>;
    fn was_externally_restored(&self) -> bool {
        false
    }
    fn mark_restored(&mut self) {}
}

struct CrosstermOps;

impl TerminalOps for CrosstermOps {
    fn enable_raw_mode(&mut self) -> std::io::Result<()> {
        enable_raw_mode()?;
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn disable_raw_mode(&mut self) -> std::io::Result<()> {
        disable_raw_mode()
    }

    fn enter_alternate_screen(&mut self) -> std::io::Result<()> {
        execute!(stdout(), EnterAlternateScreen)
    }

    fn leave_alternate_screen(&mut self) -> std::io::Result<()> {
        execute!(stdout(), LeaveAlternateScreen)
    }

    fn hide_cursor(&mut self) -> std::io::Result<()> {
        execute!(stdout(), Hide)
    }

    fn show_cursor(&mut self) -> std::io::Result<()> {
        execute!(stdout(), Show)
    }

    fn was_externally_restored(&self) -> bool {
        !TERMINAL_ACTIVE.load(Ordering::SeqCst)
    }

    fn mark_restored(&mut self) {
        TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
    }
}

struct TerminalGuard<O: TerminalOps> {
    ops: O,
    raw_mode: bool,
    alt_screen: bool,
    cursor_hidden: bool,
}

impl<O: TerminalOps> TerminalGuard<O> {
    fn enter_with(ops: O) -> std::io::Result<Self> {
        let mut guard = Self {
            ops,
            raw_mode: false,
            alt_screen: false,
            cursor_hidden: false,
        };
        guard.ops.enable_raw_mode()?;
        guard.raw_mode = true;
        guard.ops.enter_alternate_screen()?;
        guard.alt_screen = true;
        guard.ops.hide_cursor()?;
        guard.cursor_hidden = true;
        Ok(guard)
    }

    fn restore_inner(&mut self) -> std::io::Result<()> {
        if self.ops.was_externally_restored() {
            self.cursor_hidden = false;
            self.alt_screen = false;
            self.raw_mode = false;
            return Ok(());
        }
        if !self.cursor_hidden && !self.alt_screen && !self.raw_mode {
            return Ok(());
        }
        let mut first_err: Option<std::io::Error> = None;
        let mut record = |res: std::io::Result<()>| {
            if let Err(err) = res
                && first_err.is_none()
            {
                first_err = Some(err);
            }
        };
        if self.cursor_hidden {
            self.cursor_hidden = false;
            record(self.ops.show_cursor());
        }
        if self.alt_screen {
            self.alt_screen = false;
            record(self.ops.leave_alternate_screen());
        }
        if self.raw_mode {
            self.raw_mode = false;
            record(self.ops.disable_raw_mode());
        }
        self.ops.mark_restored();
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn restore(&mut self) -> std::io::Result<()> {
        self.restore_inner()
    }
}

impl<O: TerminalOps> Drop for TerminalGuard<O> {
    fn drop(&mut self) {
        let _ = self.restore_inner();
    }
}

fn restore_terminal_if_active() {
    if TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, Show);
    }
}

/// 进程 panic 钩子的签名（`std::panic::take_hook` 的返回类型）。
type PanicHook = dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send;

struct PanicHookGuard {
    previous: Option<Arc<PanicHook>>,
}

impl PanicHookGuard {
    fn install_with(restore: fn()) -> Self {
        let previous: Arc<PanicHook> = Arc::from(std::panic::take_hook());
        let prev_for_hook = Arc::clone(&previous);
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev_for_hook(info);
        }));
        Self {
            previous: Some(previous),
        }
    }
}

impl Drop for PanicHookGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::panic::set_hook(Box::new(move |info| previous(info)));
        }
    }
}

/// 事件循环：轮询按键（16 ms 节拍）→ 推进引擎一跳 → 重绘。
fn run(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let _panic_guard = PanicHookGuard::install_with(restore_terminal_if_active);
    let mut guard = TerminalGuard::enter_with(CrosstermOps)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let result = event_loop(&mut terminal, app);
    drop(terminal);
    let restore_result = guard.restore();
    result?;
    restore_result?;
    Ok(())
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
        // 与 GUI 电平心跳同一个周期（62.5 Hz）：轮询超时与 tick 闸门都用它，
        // 于是终端重绘节拍与 GUI 的 `HEARTBEAT_PERIOD_MS = 16` 同口径。
        let timeout = Duration::from_millis(METER_HEARTBEAT_MS);
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        let now = Instant::now();
        let elapsed = now.duration_since(last);
        if elapsed >= Duration::from_millis(METER_HEARTBEAT_MS) {
            app.tick();
            last = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;
    use std::sync::Mutex;

    static PANIC_HOOK_TEST_LOCK: Mutex<()> = Mutex::new(());
    static PANIC_EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    #[derive(Default)]
    struct MockState {
        calls: Vec<&'static str>,
        fail_on: Option<&'static str>,
        externally_restored: bool,
    }

    struct MockTerminalOps {
        state: Rc<RefCell<MockState>>,
    }

    impl MockTerminalOps {
        fn new() -> (Self, Rc<RefCell<MockState>>) {
            let state = Rc::new(RefCell::new(MockState::default()));
            (
                Self {
                    state: Rc::clone(&state),
                },
                state,
            )
        }

        fn record(&self, step: &'static str) -> io::Result<()> {
            let mut st = self.state.borrow_mut();
            st.calls.push(step);
            if st.fail_on == Some(step) {
                return Err(io::Error::other(step));
            }
            Ok(())
        }
    }

    impl TerminalOps for MockTerminalOps {
        fn enable_raw_mode(&mut self) -> io::Result<()> {
            self.record("enable_raw_mode")
        }

        fn disable_raw_mode(&mut self) -> io::Result<()> {
            self.record("disable_raw_mode")
        }

        fn enter_alternate_screen(&mut self) -> io::Result<()> {
            self.record("enter_alternate_screen")
        }

        fn leave_alternate_screen(&mut self) -> io::Result<()> {
            self.record("leave_alternate_screen")
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.record("hide_cursor")
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.record("show_cursor")
        }

        fn was_externally_restored(&self) -> bool {
            self.state.borrow().externally_restored
        }

        fn mark_restored(&mut self) {
            self.state.borrow_mut().externally_restored = true;
        }
    }

    #[test]
    fn terminal_guard_normal_restore_is_ordered_and_drop_does_not_repeat() {
        let (ops, state) = MockTerminalOps::new();
        let mut guard = TerminalGuard::enter_with(ops).expect("enter");
        guard.restore().expect("restore");
        drop(guard);
        assert_eq!(
            state.borrow().calls,
            vec![
                "enable_raw_mode",
                "enter_alternate_screen",
                "hide_cursor",
                "show_cursor",
                "leave_alternate_screen",
                "disable_raw_mode",
            ]
        );
        assert!(state.borrow().externally_restored);
    }

    #[test]
    fn terminal_guard_drop_restores_on_early_exit() {
        let (ops, state) = MockTerminalOps::new();
        {
            let _guard = TerminalGuard::enter_with(ops).expect("enter");
        }
        assert_eq!(
            state.borrow().calls,
            vec![
                "enable_raw_mode",
                "enter_alternate_screen",
                "hide_cursor",
                "show_cursor",
                "leave_alternate_screen",
                "disable_raw_mode",
            ]
        );
    }

    #[test]
    fn terminal_guard_partial_init_failure_cleans_up_raw_mode() {
        let (ops, state) = MockTerminalOps::new();
        state.borrow_mut().fail_on = Some("enter_alternate_screen");
        assert!(TerminalGuard::enter_with(ops).is_err());
        assert_eq!(
            state.borrow().calls,
            vec![
                "enable_raw_mode",
                "enter_alternate_screen",
                "disable_raw_mode",
            ]
        );
    }

    #[test]
    fn terminal_guard_restore_continues_on_step_error() {
        let (ops, state) = MockTerminalOps::new();
        let mut guard = TerminalGuard::enter_with(ops).expect("enter");
        state.borrow_mut().fail_on = Some("show_cursor");
        let err = guard.restore().expect_err("show_cursor error");
        assert_eq!(err.to_string(), "show_cursor");
        drop(guard);
        assert_eq!(
            state.borrow().calls,
            vec![
                "enable_raw_mode",
                "enter_alternate_screen",
                "hide_cursor",
                "show_cursor",
                "leave_alternate_screen",
                "disable_raw_mode",
            ]
        );
    }

    struct HookReset(Option<Box<PanicHook>>);

    impl Drop for HookReset {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                std::panic::set_hook(prev);
            }
        }
    }

    fn record_restore() {
        PANIC_EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push("restore");
    }

    #[test]
    fn panic_hook_restores_terminal_before_previous_hook_and_unwinds_cleanly() {
        let _lock = PANIC_HOOK_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _reset = HookReset(Some(std::panic::take_hook()));
        PANIC_EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        std::panic::set_hook(Box::new(|_| {
            PANIC_EVENTS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("previous");
        }));

        let (ops, state) = MockTerminalOps::new();
        {
            let _panic_guard = PanicHookGuard::install_with(record_restore);
            let _term_guard = TerminalGuard::enter_with(ops).expect("enter");
            let state_for_panic = Rc::clone(&state);
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                state_for_panic.borrow_mut().externally_restored = true;
                panic!("simulated panic");
            }));
            assert!(caught.is_err());
        }

        assert_eq!(
            *PANIC_EVENTS.lock().unwrap_or_else(|e| e.into_inner()),
            vec!["restore", "previous"]
        );
        // Panic hook simulated external restoration, so Drop did not repeat teardown.
        assert_eq!(
            state.borrow().calls,
            vec!["enable_raw_mode", "enter_alternate_screen", "hide_cursor"]
        );

        PANIC_EVENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let after_drop = std::panic::catch_unwind(|| panic!("after guard drop"));
        assert!(after_drop.is_err());
        assert_eq!(
            *PANIC_EVENTS.lock().unwrap_or_else(|e| e.into_inner()),
            vec!["previous"]
        );

        TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
        restore_terminal_if_active();
        assert!(!TERMINAL_ACTIVE.load(Ordering::SeqCst));
    }
}
