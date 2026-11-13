# SFZ 采样资源解析规则（asset-resolution）

本文件把「一份 SFZ 里写下的路径，最终读到哪个文件」这件事的全部规则收在一处。
每条规则都给出代码出处（文件 ＋ 函数），与代码不一致时**以代码为准**、并请顺手修本文件。

适用面：`yeban_load_sfz` 装载、`yeban_render_master` 渲染采样器轨、app 的
`--sfz` / `--track-sampler` / `--export-wav` 路径。三条面共用同一套解析语义；
两处实现（MCP 侧与 app 侧）是**同一份口径的两份代码**，注释里互相点名，改一处必须看另一处。

---

## 1. `sample=` 的路径解析

出处：`crates/yeban-mcp/src/domain/sfz.rs::resolve_sample_path`；
app 侧同一语义在 `crates/yeban-app/src/sfz_load.rs::resolve_sample_path`。

1. **解析顺序**：先归一路径分隔符，再判绝对／相对（见规则 2、3）。
2. **`\` 是路径分隔符，不是转义**。SFZ 的 `opcode=value` 值里没有转义语法：
   解析器只去掉一层成对双引号（`crates/yeban-sfz/src/parser.rs::unquote`），其余字节原样进
   `Region::sample`。因此 `sample=arco\c4.wav` 里的 `\` 按 `/` 处理
   （`normalize_sample_path`，MCP 与 app 各一份、同一实现）。两个连续 `\` 变成两个 `/`，
   本函数**不折叠** `//`（操作系统会折叠，行为等价）。
3. **绝对路径原样使用**：POSIX `/x/y` 原样；Windows 盘符形式 `C:/…`（归一后）
   也判为绝对、原样交出（`is_windows_drive_absolute`：盘符字母 ＋ `:` ＋ `/`；`C:foo`
   这类盘符相对路径不在此列）；UNC `\\server\share\x` 归一后以 `/` 开头，同样原样。
   绝对路径**不拼** SFZ 目录。
4. **相对路径相对 SFZ 文件所在目录**（SFZ 惯例）：`sfz_path.parent().join(sample)`；
   SFZ 路径没有父目录时退回原样相对路径。
5. **`default_path` 在路径解析之前拼**：播放／读取用的是 `Region::sample_path()`
   （`crates/yeban-sfz/src/instrument.rs::sample_path`）——把 `<control> default_path=`
   前缀拼到 `sample` 前面（前缀末尾没有 `/` 时补一个）；`sample` 以 `/` 开头时不拼；
   `default_path` 为空串视为未设。拼好的串再走规则 2–4。
6. **内建静音不走路径解析**：`sample=*silence`（含大小写变体）是内建静音伪样本，
   判定用 `sample=` 的**原值**（`Region::is_builtin_silence`，⛔ 不用拼过
   `default_path` 的串）。它不读盘、不进 `sampleFailures`、渲染时写零
   （`crates/yeban-mcp/src/domain/sfz.rs` 的 `BUILTIN_SILENCE_SOURCE`）。

## 2. `#include` 的沙箱纪律

出处：`crates/yeban-sfz/src/parser.rs` 的 `IncludeResolver`（`check_relative` /
`canonical_include` / `expand_file`）。装载方以**主 SFZ 文件所在目录**为基准目录构造它
（`crates/yeban-mcp/src/domain/sfz.rs::resolve_include_sources`、
`crates/yeban-engine/src/sfz_instrument.rs`、`crates/yeban-app/src/sample_editor.rs`）。

7. **基准目录**：`IncludeResolver::new(base_dir, limits)` 把基准目录
   `fs::canonicalize` 后作为沙箱根；基准必须是已存在的目录。
8. **词法检查**（`check_relative`，在读任何文件之前）：路径为空 ⇒
   `IncludeEmptyPath`；含 NUL ⇒ `IncludeInvalidPath`；Windows 盘符绝对路径与
   `/` 开头的绝对路径 ⇒ `IncludeAbsolutePath`；先把 `\` 归一成 `/`，任一路径组件
   是 `..` ⇒ `IncludeEscape`；扩展名只认 `.sfz` / `.sfzh` / `.txt`（大小写不敏感），
   其余 ⇒ `IncludeUnsupportedExtension`。全部错误都带行号与路径，响亮失败。
9. **规范化检查**（`canonical_include`）：候选路径 `fs::canonicalize` 后仍必须位于
   基准目录之内（这条同时挡住**符号链接逃逸**）⇒ 越界是 `IncludeEscape`；
   目标不存在 ⇒ `IncludeNotFound`；不是文件 ⇒ `IncludeNotAFile`。
10. **两档错误口径**：`IncludeResolver::resolve` 是**严格档**——缺失的 include 就是
    错误；`resolve_lossy` / `resolve_text_lossy` 是**容错档**——目标不存在、glob
    无匹配只记一条 `IncludeIssue`（`NotFound` / `NoMatch`）并继续展开其余部分，
    但成环（`IncludeCycle`）、超深度（`IncludeDepthExceeded`）、超文件数、
    非 UTF-8、沙箱逃逸仍然是错误。**装载路径走容错档**（MCP 侧经
    `resolve_include_sources`，缺失项逐条进装载回执的 `includeIssues`
    数组与 `includeIssueCount` 计数，不让整台乐器因一条坏 include 不可加载）。
11. **纯文本解析不展开 include**：`parse_text`（无 I/O 的纯函数）遇到 `#include`
    只记 `Warning::IncludeIgnored`，⛔ 不得用它走装载路径；装载必须先经
    `IncludeResolver` 展开、再 `parse_sources`。
12. **glob 形式**（`*` / `?` / `**`）只从基准目录向下遍历、不跟随符号链接，
    受 `ParseLimits` 的 `max_glob_matches` / `max_glob_scanned` / `max_glob_depth`
    三重上限约束；`#define` 宏在展开 include 路径之前先行展开，未定义宏会告警。

## 3. 采样文件的读取与失败记账

出处：`crates/yeban-mcp/src/domain/sfz.rs` 的 `read_sample_file` / `SampleCache` /
`SampleLimits`。

13. **按后缀分腿解码**（`is_plain_wav_suffix`，只决定走哪条腿、⛔ 不决定成败，
    也不做内容嗅探）：后缀 `.wav` / `.wave`（大小写不敏感）走 hound 读 PCM 并折到
    声道 0；其余后缀交给 `yeban_decode::decode_mono_channel0` 按**内容**探测容器。
    `yeban-decode` 的支持清单是明确的：WAV、FLAC、OGG / Vorbis、ADPCM
    （`crates/yeban-decode/src/lib.rs` 模块文档）；上游的 Matroska、MP3、AAC、
    ALAC、AIFF、CAF、ISO-MP4 **一律不启用**，这类文件会解码失败并进 `sampleFailures`。
14. **单文件上限 64 MiB**（`MAX_SAMPLE_FILE_BYTES`）：读之前先看 `metadata.len()`，
    超限直接拒绝读取并把原因写进失败记账，⛔ **绝不截断**（截断会让听到的与
    文件里的不是同一段音频）。路径是目录、文件不存在同样是逐条失败，不是装载失败。
15. **装载／渲染预算**（`SampleLimits`，逐轨记账、解码结果跨轨共享于
    `SampleStore`）：不同样本文件数缺省上限 512（`MAX_SAMPLE_FILES_READ`）、
    累计解码帧数缺省上限 64,000,000（`MAX_SAMPLE_FRAMES_TOTAL`）；运行期可用
    `yeban_load_sfz` / `yeban_render_master` 的 `maxSampleFiles` /
    `maxSampleFrames` 覆盖，`0` ＝ 不限。任一上限命中即**停止再读**，
    并把 `sampleReadsTruncated` 置 `true`——响亮报告，⛔ 不静默丢弃。
    预算只对本次渲染**会发声的已用子集**记账（按需装载 `DemandLoadFilter`）；
    已用子集自身超预算仍然响亮拒绝。
16. **失败记账的形状**（装载与渲染回执同口径）：`sampleFailures` 是逐条字符串数组
    （路径 ＋ 原因）；同批读数还有 `sampleFilesReadCount` /
    `sampleFilesFailedCount` / `sampleFramesLoaded` / `sampleReadsTruncated` /
    回显本次预算的 `sampleLimits`；每个 region 另有 `sample_file_state`
    （读到的描述串，或 `unreadable(<路径>): <原因>`、`builtin-silence(*silence)`、
    `noSampleOpcode`、`skippedDemandLoad` 之一）。
17. **缺样本的后果**：MCP 的装载／渲染路径上，读不到样本的 region 是**数字静音**
    并计入 `silentRegionCount`（引擎摊平口径：缺样本 ＋ 内建静音），
    ⛔ **没有正弦回退**。正弦回退只存在于试听面：app 的
    `crates/yeban-app/src/sfz_load.rs::region_sample_for` 读不到时回退内存正弦，
    并把原因写进来源文案（不假装读到了文件）；`yeban_render_sfz_wav` 同理。

## 4. 渲染时采样器绑定的解析（容器池优先、磁盘其次）

出处：`crates/yeban-mcp/src/domain/render.rs::prepare_sampler_track` /
`resolve_sampler_path`；内嵌装载在 `crates/yeban-mcp/src/domain/sfz.rs::load_with_embedded`，
内嵌映射的收集在 `crates/yeban-mcp/src/domain/embed_samples.rs`。

18. **内嵌腿（容器池优先）**：音轨的采样器绑定串（`TrackV3::sampler`）若命中工程的
    `embedded_samples` 表（绑定路径 → SHA-256），SFZ 文本与全部样本字节都取自
    会话 CAS 池（`AssetStore`），⛔ 不读盘；`sample=` 的解析锚点是**绑定路径原样**
    （拼出的就是内嵌映射表的键，见 `embedded_sample_key`）。内嵌 SFZ 文本含
    `#include` ⇒ 响亮失败（`RENDER_FAILED` / `samplerUnreadable`）：保存侧从不
    内嵌这类 SFZ，要渲染请用磁盘上的原文件。哈希非法、资产不在池里、文本非
    UTF-8 同样是 `samplerUnreadable`，⛔ 不回退磁盘腿蒙混。
19. **磁盘腿（未内嵌时）**：`resolve_sampler_path` 按顺序试候选、第一个
    `is_file` 命中即用，全不中 ⇒ `RENDER_FAILED` 并在报文里列出**试过的全部路径**：
    - 绑定串是绝对路径 ⇒ 只试它本身；
    - 相对路径依次试：① 工程文件所在目录 ＋ 原串；② 采样库根
      （`sample_roots::library_root`）＋ 剥掉 `assets/samples/` 前缀后的相对路径
      （与 app 及下载脚本 `--dest` 同一条规则）；③ 仓库根
      （`sample_roots::repo_root`）＋ 原串；④ 原串本身（相对进程 cwd）。
20. **绑定串的存入改写**：经 `yeban_edit_notes` 的 `setTrackSampler` 绑定时，
    落在采样库根之下的绝对路径会被改写成 `assets/samples/` ＋ 库根下相对部分
    存入工程（换机器仍可经规则 19 的第②档解析回原文件）；其余形式原样留痕，
    绑定时 ⛔ 不查存在性、不查扩展名——读不到在渲染时按规则 18／19 响亮失败。
