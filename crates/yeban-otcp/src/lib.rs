//! `yeban-otcp`：OpenTune Standalone 子进程驱动（OTCP 协议）。
//!
//! 本 crate 只做**协议驱动**：启动用户自备的 OpenTune API 分支构建
//! （子进程），走公开的 OTCP JSON-RPC 协议做离线整 clip 音高校正
//! （调性 snap），再把校正后的音频经 MCP 提案换资产。
//!
//! # 许可边界（硬性）
//!
//! * 仓库里⛔ 不出现任何 OpenTune 代码；⛔ 不打包 OpenTune 二进制；
//! * 声码器权重（CC BY-NC-SA 4.0）⛔ 永不进仓库——权重只活在用户
//!   自己的 OpenTune 安装里（与 P0-1「权重用户自行下载」同口径）；
//! * OpenTune 是 AGPL-3.0，夜半是 GPL-3.0-only；本 crate 只驱动
//!   用户提供的 Standalone 子进程，⛔ 不链接、不嵌入 OpenTune 代码，
//!   许可边界干净；
//! * 依赖只有 `std` + `serde_json`（`cargo deny check` 无新增依赖）。
//!
//! # 设计
//!
//! 实现见 `~/workspace/opentune/DESIGN-otcp-driver.md`（权威），
//! 决策留痕见 `docs/adr/ADR-0070-otcp-driver.md`（D1–D3）。
//! 模块：`client`（JSON-RPC）/ `discovery`（发现文件）/
//! `supervisor`（进程监管）/ `pipeline`（校正流水线）/
//! `recovery`（模态框恢复）/ `measure`（量测门）/
//! `config`（配置与自测记录）/ `error`（错误分类）。
//!
//! # 失败语义
//!
//! `RENDER_FAILED` 大声失败、什么都不写、⛔ 不重试、⛔ 不回落干声
//! （设计稿 §6）。自测门（§6.3）先跑再接任务，`fail` 只注解错误，
//! ⛔ 不拦不命中缺陷的任务。

pub mod client;
pub mod config;
pub mod discovery;
pub mod error;
pub mod measure;
pub mod pipeline;
pub mod recovery;
pub mod supervisor;
pub mod wav;

pub use client::{OtcpClient, redact};
pub use config::{DisplayConfig, DriverStatus, OtcpConfig, OtcpTimeouts, SelfTestRecord};
pub use error::{OtcpError, OtcpRpcError, REASONS};
pub use measure::{Measurement, measure_export};
pub use pipeline::{
    CorrectionRequest, JobReceipt, Key, Note, OtcpDriver, VALID_SCALES, mark_delivered,
    read_receipt, write_initial_receipt, write_receipt,
};
pub use supervisor::{ServerSession, shutdown_server, start_server};
