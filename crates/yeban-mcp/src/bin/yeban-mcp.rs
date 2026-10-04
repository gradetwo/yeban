//! `yeban-mcp` 独立可执行文件（形态 B：stdio 批处理；可选形态 A：环回 HTTP）
//! [ROAD-M4-001, ROAD-M4-002, ARCH-SEC-002, MUST-GATE-009]。
//!
//! ```text
//! 用法: yeban-mcp [选项]
//!
//!   --stdio                逐行 JSON-RPC 批处理 (默认, 读到 EOF 就退出)
//!   --print-token          生成/读取 ~/.yeban/session.token 并打印 (不启动任何服务)
//!   --token-file <PATH>    覆盖令牌文件路径 (默认 ~/.yeban/session.token)
//!   --scopes <LIST>        逗号分隔的授予作用域 (默认 app:admin)
//!   --test-mode            打开测试模式 (只有这个模式允许 ui:inject)
//!   --enable-mcp-http      请求启动环回 HTTP 传输 (需要 --features mcp-http)
//!   -h, --help             打印本帮助
//! ```
//!
//! ## 冷启动预算
//!
//! `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.1 给形态 B 定的目标是
//! **冷启动 ≤ 20ms**。本二进制**尚未做这项测量** —— 见
//! `docs/ledger/mcp-core-notes.md` §4 的 `pending` 清单：没有 `iai-callgrind`
//! 打点、也没有 CI 侧的启动耗时判据，把"看起来很快"写成"达标"是假绿。
//!
//! ## 退出码
//!
//! | 码 | 含义 |
//! | ---: | :--- |
//! | 0 | 正常结束（批处理读完 EOF，或 `--print-token` / `--help`） |
//! | 1 | 运行期失败（令牌文件不可用、I/O 失败、HTTP 循环退出） |
//! | 2 | 用法错误 / 要求了一个本次构建没有的 feature |

use std::path::PathBuf;
use std::process::ExitCode;

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, RunMode, ScopeSet, TokenFile};
use yeban_mcp::transport::{self, HttpStartup};

/// 帮助文本。
const USAGE: &str = "\
用法: yeban-mcp [选项]

  --stdio                逐行 JSON-RPC 批处理 (默认, 读到 EOF 就退出)
  --print-token          生成/读取 ~/.yeban/session.token 并打印 (不启动任何服务)
  --token-file <PATH>    覆盖令牌文件路径 (默认 ~/.yeban/session.token)
  --scopes <LIST>        逗号分隔的授予作用域 (默认 app:admin)
  --test-mode            打开测试模式 (只有这个模式允许 ui:inject)
  --enable-mcp-http      请求启动环回 HTTP 传输 (需要 --features mcp-http)
  -h, --help             打印本帮助

安全默认 [ARCH-SEC-002 / MUST-GATE-009]:
  * HTTP 传输默认关闭, 且只在 127.0.0.1 上以系统动态端口监听;
  * 所有 HTTP 请求必须携带 `Authorization: Bearer <TOKEN>`;
  * 令牌落在 ~/.yeban/session.token, 权限强制 0600;
  * 生产模式硬禁作用域 ui:inject。
";

/// 一次失败的退出方式。
#[derive(Debug)]
struct Failure {
    message: String,
    code: u8,
}

impl Failure {
    /// 运行期失败（退出码 1）。
    fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }

    /// 用法 / 配置失败（退出码 2）。
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }
}

/// 解析后的调用意图。
enum Invocation {
    /// 打印用法并成功退出。
    Help,
    /// 按配置运行。
    Run(Box<Config>),
}

/// 运行配置。
#[derive(Debug, Default)]
struct Config {
    print_token: bool,
    token_file: Option<PathBuf>,
    scopes: Option<String>,
    test_mode: bool,
    enable_http: bool,
}

impl Config {
    /// 令牌文件路径（默认 `~/.yeban/session.token`）。
    ///
    /// # Errors
    ///
    /// 未显式指定且找不到家目录。
    fn token_path(&self) -> Result<PathBuf, String> {
        match &self.token_file {
            Some(path) => Ok(path.clone()),
            None => TokenFile::default_path().map_err(|error| error.to_string()),
        }
    }

    /// 授予的作用域（默认 `app:admin`）。
    ///
    /// # Errors
    ///
    /// 作用域清单里出现未知名字。
    fn granted(&self) -> Result<ScopeSet, String> {
        match &self.scopes {
            Some(list) => ScopeSet::parse_list(list).map_err(|error| error.to_string()),
            None => Ok(ScopeSet::from_scopes([
                yeban_mcp::security::Scope::AppAdmin,
            ])),
        }
    }

    /// 逐参数解析。
    fn parse<I: Iterator<Item = String>>(mut args: I) -> Result<Invocation, String> {
        let mut config = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => return Ok(Invocation::Help),
                "--stdio" => {}
                "--print-token" => config.print_token = true,
                "--test-mode" => config.test_mode = true,
                "--enable-mcp-http" => config.enable_http = true,
                "--token-file" => {
                    let value = args.next().ok_or("`--token-file` 需要一个路径参数")?;
                    config.token_file = Some(PathBuf::from(value));
                }
                "--scopes" => {
                    let value = args.next().ok_or("`--scopes` 需要一个逗号分隔的清单")?;
                    config.scopes = Some(value);
                }
                other => return Err(format!("未知参数 `{other}`")),
            }
        }
        Ok(Invocation::Run(Box::new(config)))
    }
}

fn main() -> ExitCode {
    let config = match Config::parse(std::env::args().skip(1)) {
        Ok(Invocation::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Invocation::Run(config)) => config,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match execute(&config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("error: {}", failure.message);
            ExitCode::from(failure.code)
        }
    }
}

/// 执行一次调用。
fn execute(config: &Config) -> Result<(), Failure> {
    let granted = config.granted().map_err(Failure::usage)?;
    let mode = RunMode::from_test_flag(config.test_mode);
    let token_path = config.token_path().map_err(Failure::usage)?;
    let token_file = TokenFile::new(token_path);

    if config.print_token {
        let (token, created) = token_file
            .load_or_create()
            .map_err(|error| Failure::runtime(error.to_string()))?;
        println!("{}", token.expose());
        eprintln!(
            "yeban-mcp: {} {} (权限 0600)",
            if created { "已生成" } else { "已复用" },
            token_file.path().display()
        );
        return Ok(());
    }

    // 运行期要求了本次构建没有的 feature 属于**配置**问题 (退出码 2), 不是运行期故障。
    let startup = transport::plan_http_startup(config.enable_http)
        .map_err(|error| Failure::usage(error.to_string()))?;
    let token = match startup {
        // 形态 A: 令牌必须落盘并带 0600, 因为它是 HTTP 的鉴权依据。
        HttpStartup::Enabled => {
            token_file
                .load_or_create()
                .map_err(|error| Failure::runtime(error.to_string()))?
                .0
        }
        // 形态 B: stdio 的能力边界就是"能往本进程写 stdin", 因此**不碰磁盘**。
        // 仍然生成一个 256-bit 令牌: 判定链路在两种形态下完全一致, 没有"stdio 就跳过"的分支。
        HttpStartup::Disabled => BearerToken::generate().token,
    };

    let mut dispatcher = Dispatcher::new(token, granted, mode);
    match startup {
        HttpStartup::Enabled => serve_http(dispatcher),
        HttpStartup::Disabled => {
            let report = transport::stdio::serve_stdio(&mut dispatcher)
                .map_err(|error| Failure::runtime(format!("stdio 批处理失败: {error}")))?;
            eprintln!(
                "yeban-mcp: 处理 {} 行, 写出 {} 条响应 (跳过 {} 个空行, 模式 {}); 令牌 {}",
                report.lines_read,
                report.responses_written,
                report.blank_lines_skipped,
                mode.as_str(),
                token_file.path().display()
            );
            Ok(())
        }
    }
}

/// 启动环回 HTTP 传输（形态 A）。
#[cfg(feature = "mcp-http")]
fn serve_http(dispatcher: Dispatcher) -> Result<(), Failure> {
    let server = yeban_mcp::transport::http::HttpServer::bind_loopback(dispatcher)
        .map_err(|error| Failure::runtime(error.to_string()))?;
    let address = server
        .local_addr()
        .map_err(|error| Failure::runtime(error.to_string()))?;
    eprintln!(
        "yeban-mcp: 环回 HTTP 已启动 http://{address}{} (只监听环回; Bearer Token 见 ~/.yeban/session.token)",
        yeban_mcp::transport::http::MCP_PATH
    );
    server
        .serve_forever()
        .map_err(|error| Failure::runtime(error.to_string()))
}

/// 没有 `mcp-http` feature 时的诚实拒绝。
#[cfg(not(feature = "mcp-http"))]
fn serve_http(_dispatcher: Dispatcher) -> Result<(), Failure> {
    Err(Failure::usage(format!(
        "本次构建未编译 `{}` feature: 请用 `--features {}` 重新构建",
        transport::HTTP_FEATURE_NAME,
        transport::HTTP_FEATURE_NAME
    )))
}
