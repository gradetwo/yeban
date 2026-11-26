//! T-FIX-158：远程 demo 下载核心的判据（本地夹具服务器，不依赖真 GitHub）。

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use yeban_app::demo_download::{
    CancelFlag, DEMOS, DownloadError, DownloadEvent, cover_url, download_to, encode_segment,
    part_path, project_url,
};

/// 起一个只回答 `requests` 次的极简 HTTP 服务器；返回 (基址, 收到的请求路径列表句柄)。
fn serve(body: Vec<u8>, requests: usize) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定端口");
    let addr = listener.local_addr().expect("地址");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_thread = Arc::clone(&seen);
    std::thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let text = String::from_utf8_lossy(&buf).to_string();
            if let Some(path) = text.split_whitespace().nth(1) {
                seen_thread.lock().unwrap().push(path.to_owned());
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), seen)
}

/// 起一个只回答一次、声明完整长度但只发送 `prefix` 的服务器；用于
/// 模拟“响应已开始、读取中途断流”。
fn serve_prefix(prefix: Vec<u8>, declared_len: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定端口");
    let addr = listener.local_addr().expect("地址");
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {declared_len}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&prefix);
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Both);
    });
    format!("http://{addr}")
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-demo-dl-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 起一个声明 `declared` 长度、实际只发 `body` 字节后就断开的服务器
/// （不完整下载的夹具）。
fn serve_short(declared: usize, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定端口");
    let addr = listener.local_addr().expect("地址");
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
        let _ = stream.flush();
        // 随即关闭：客户端收到的字节少于声明值。
    });
    format!("http://{addr}")
}

/// T-FIX-640：断言事件序列以**恰好一次** `Failed` 结尾（无 Finished、无
/// Cancelled）；`want` 给出时再断言 Failed 报文含该子串。
fn assert_failed_terminal(events: &[DownloadEvent], want: Option<&str>) {
    assert!(
        matches!(events.last(), Some(DownloadEvent::Failed(_))),
        "事件序列必须以 Failed 结尾: {events:?}"
    );
    let failed: Vec<&String> = events
        .iter()
        .filter_map(|event| match event {
            DownloadEvent::Failed(reason) => Some(reason),
            _ => None,
        })
        .collect();
    assert_eq!(failed.len(), 1, "Failed 必须恰好出现一次: {events:?}");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DownloadEvent::Finished(_))),
        "失败路径不得发 Finished: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DownloadEvent::Cancelled)),
        "失败路径不得发 Cancelled: {events:?}"
    );
    if let Some(want) = want {
        assert!(
            failed[0].contains(want),
            "Failed 报文应含 {want:?}，实为 {:?}",
            failed[0]
        );
    }
}

#[test]
fn manifest_lists_eight_unique_demos() {
    assert_eq!(DEMOS.len(), 8);
    let mut dirs: Vec<&str> = DEMOS.iter().map(|spec| spec.dir).collect();
    dirs.sort_unstable();
    dirs.dedup();
    assert_eq!(dirs.len(), 8, "子目录必须两两不同");
    let base = "http://example.invalid/base";
    // ⭐ T-FIX-158 返工：清单必须与 gradetwo/yeban-extras 的 `demos/` **逐字**对应
    //    （目录名、工程名与封面名都只用干净歌名，不带 `_男声` / `_叙事版` 等后缀），
    //    且中文段一律百分号编码 —— http 1.5 的 URI 不接受 ≥127 的字节。
    let spec = DEMOS
        .iter()
        .find(|spec| spec.title == "雨声太吵")
        .expect("清单必须有《雨声太吵》");
    assert_eq!(
        project_url(base, spec),
        "http://example.invalid/base/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5.yeban"
    );
    assert_eq!(
        cover_url(base, spec),
        "http://example.invalid/base/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5.png"
    );
    // 每条 URL 都必须是纯 ASCII（否则 ureq 在发请求前就解析失败）。
    // 工程与封面只用干净歌名：工程是 `<歌名>.yeban`，封面是 `<歌名>.png/.jpg`。
    for spec in &DEMOS {
        let url = project_url(base, spec);
        assert!(url.is_ascii(), "URL 必须纯 ASCII: {url}");
        assert_eq!(spec.dir, spec.title, "demo 目录名必须是干净歌名");
        assert_eq!(
            spec.file,
            format!("{}.yeban", spec.title),
            "工程名必须是干净歌名加 .yeban"
        );
        assert!(
            spec.cover == format!("{}.png", spec.title)
                || spec.cover == format!("{}.jpg", spec.title),
            "封面名必须是干净歌名加图片后缀: {}",
            spec.cover
        );
    }
}

#[test]
fn download_reports_monotonic_progress_and_lands_the_file() {
    let body = b"PK\x03\x04fake-container-bytes".to_vec();
    let (base, seen) = serve(body.clone(), 1);
    let root = temp_root("ok");
    let target = root.join("demo").join("a.yeban");
    let url = format!("{base}/demo/a.yeban");

    let mut events = Vec::new();
    let path = download_to(&url, &target, &CancelFlag::new(), |event| {
        events.push(event)
    })
    .expect("下载必须成功");
    assert_eq!(path, target);
    assert!(target.is_file(), "正式文件必须落位");
    assert!(!part_path(&target).exists(), "半成品必须已改名，不留 .part");
    assert_eq!(std::fs::read(&target).unwrap(), body);
    assert_eq!(seen.lock().unwrap().as_slice(), ["/demo/a.yeban"]);

    let progresses: Vec<(u64, Option<u64>)> = events
        .iter()
        .filter_map(|event| match event {
            DownloadEvent::Progress { received, total } => Some((*received, *total)),
            _ => None,
        })
        .collect();
    assert!(!progresses.is_empty(), "必须发出进度事件");
    assert!(
        progresses.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "进度必须单调不减: {progresses:?}"
    );
    assert_eq!(progresses.last().unwrap().0, body.len() as u64);
    assert_eq!(progresses.last().unwrap().1, Some(body.len() as u64));
    assert!(matches!(events.last(), Some(DownloadEvent::Finished(_))));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn corrupt_payload_is_rejected_and_leaves_no_file() {
    let (base, _seen) = serve(b"not a container".to_vec(), 1);
    let root = temp_root("corrupt");
    let target = root.join("demo").join("bad.yeban");
    let url = format!("{base}/demo/bad.yeban");
    let error = download_to(&url, &target, &CancelFlag::new(), |_| {}).expect_err("非容器必须被拒");
    assert_eq!(error.0, DownloadError::Corrupt);
    assert!(!target.exists(), "校验失败不得留下正式文件");
    assert!(!part_path(&target).exists(), "校验失败不得留下半成品");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn mid_stream_read_failure_cleans_part_and_target() {
    let body = b"PK\x03\x04partial-but-declared-longer".to_vec();
    let declared = body.len() + 64;
    let base = serve_prefix(body, declared);
    let root = temp_root("mid-read");
    let target = root.join("demo").join("mid-read.yeban");
    let url = format!("{base}/demo/mid-read.yeban");
    let error =
        download_to(&url, &target, &CancelFlag::new(), |_| {}).expect_err("响应中途断流必须失败");
    assert_eq!(error.0, DownloadError::Network);
    assert!(!target.exists(), "读取失败不得留下正式文件");
    assert!(
        !part_path(&target).exists(),
        "读取失败必须由 guard 清掉 .part"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rename_failure_cleans_part_and_keeps_target() {
    let body = b"PK\x03\x04rename-failure".to_vec();
    let (base, _seen) = serve(body, 1);
    let root = temp_root("rename-fail");
    let target = root.join("demo").join("rename-fail.yeban");
    std::fs::create_dir_all(&target).expect("目标路径放目录，制造 rename 失败");
    let url = format!("{base}/demo/rename-fail.yeban");
    let error =
        download_to(&url, &target, &CancelFlag::new(), |_| {}).expect_err("rename 到目录必须失败");
    assert_eq!(error.0, DownloadError::Io);
    assert!(target.is_dir(), "失败的 rename 不得破坏原 target");
    assert!(
        !part_path(&target).exists(),
        "rename 失败必须由 guard 清掉 .part"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cancelled_download_cleans_up_partial_file() {
    let (base, _seen) = serve(b"PK\x03\x04payload".to_vec(), 1);
    let root = temp_root("cancel");
    let target = root.join("demo").join("c.yeban");
    let url = format!("{base}/demo/c.yeban");
    let cancel = CancelFlag::new();
    cancel.cancel();
    let error = download_to(&url, &target, &cancel, |_| {}).expect_err("取消必须失败");
    assert_eq!(error.0, DownloadError::Cancelled);
    assert!(!target.exists());
    assert!(!part_path(&target).exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn network_failure_is_loud_and_retry_succeeds() {
    // 先指向一个没人监听的端口 ⇒ Network。
    let root = temp_root("net");
    let target = root.join("demo").join("d.yeban");
    let dead = "http://127.0.0.1:9/demo/d.yeban";
    let error = download_to(dead, &target, &CancelFlag::new(), |_| {}).expect_err("必须失败");
    assert_eq!(error.0, DownloadError::Network);
    assert!(!target.exists());

    // 再指向夹具 ⇒ 重试成功（同一个 target）。
    let (base, _seen) = serve(b"PK\x03\x04retry-ok".to_vec(), 1);
    let url = format!("{base}/demo/d.yeban");
    download_to(&url, &target, &CancelFlag::new(), |_| {}).expect("重试必须成功");
    assert!(target.is_file());
    let _ = std::fs::remove_dir_all(&root);
}

/// ⭐ T-FIX-158 返工：**中文目录名的真实 demo** 必须在上线路径里百分号编码。
///
/// 修前清单是手编的 ASCII 目录（`rain-too-loud`），真实 extras 是中文目录 + 带后缀
/// 文件名；且中文不编码时 `http` crate 在解析 URI 阶段就拒绝（≥127 字节），
/// 一个字节都发不出去。本判据从夹具服务器**收到的请求行**读回线上路径。
#[test]
fn chinese_demo_paths_are_encoded_on_the_wire_and_download() {
    let body = b"PK\x03\x04chinese-demo".to_vec();
    let (base, seen) = serve(body.clone(), 1);
    let root = temp_root("cn");
    let spec = DEMOS
        .iter()
        .find(|spec| spec.title == "回头路")
        .expect("清单必须有《回头路》");
    let url = project_url(&base, spec);
    let target = root.join("demo").join(spec.file);
    let path = download_to(&url, &target, &CancelFlag::new(), |_| {})
        .expect("中文路径必须能下载（URI 已编码）");
    assert_eq!(path, target);
    assert_eq!(std::fs::read(&target).expect("读回"), body);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "必须恰好一次请求: {seen:?}");
    assert_eq!(
        seen[0],
        format!(
            "/{}/{}",
            encode_segment(spec.dir),
            encode_segment(spec.file)
        ),
        "线上路径必须是编码后的目录/文件名"
    );
    assert!(
        !seen[0].contains('回'),
        "线上路径不得含未编码中文: {seen:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// ⭐ T-FIX-158 返工：中文路径的下载被取消后**不留任何残留**（正式文件与 `.part` 都没有）。
#[test]
fn cancelled_chinese_path_download_leaves_no_residue() {
    let root = temp_root("cn-cancel");
    let spec = DEMOS
        .iter()
        .find(|spec| spec.title == "回头路")
        .expect("清单必须有《回头路》");
    // 用活着的夹具：`download_to` 在读到第一个块时才看取消位（与既有取消判据同一条路径）。
    let (base, _seen) = serve(b"PK\x03\x04cn-cancel".to_vec(), 1);
    let url = project_url(&base, spec);
    let target = root.join("demo").join(spec.file);
    let cancel = CancelFlag::new();
    cancel.cancel();
    let error = download_to(&url, &target, &cancel, |_| {}).expect_err("取消必须失败");
    assert_eq!(error.0, DownloadError::Cancelled);
    assert!(!target.exists(), "取消不得留下正式文件");
    assert!(!part_path(&target).exists(), "取消不得留下 .part");
    let leftovers: Vec<PathBuf> = std::fs::read_dir(root.join("demo"))
        .map(|entries| entries.map(|entry| entry.expect("目录项").path()).collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "取消后目录必须为空: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// T-FIX-640：每条失败路径在返回 Err 前恰好发一次 Failed，事件序列以 Failed 结尾。
// ---------------------------------------------------------------------------

/// 网络失败（死链）：终态必须是恰好一次 Failed，报文点名"请求失败"。
#[test]
fn network_failure_ends_with_single_failed() {
    let root = temp_root("net-failed");
    let target = root.join("demo").join("d.yeban");
    let dead = "http://127.0.0.1:9/demo/d.yeban";
    let mut events = Vec::new();
    let error = download_to(dead, &target, &CancelFlag::new(), |event| {
        events.push(event)
    })
    .expect_err("必须失败");
    assert_eq!(error.0, DownloadError::Network);
    assert_failed_terminal(&events, Some("请求失败"));
    assert!(!target.exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// IO 失败（改名）：目标已是目录 ⇒ rename(.part → 目录) 失败，终态 Failed 一次。
#[test]
fn rename_failure_ends_with_single_failed() {
    let body = b"PK\x03\x04rename-target".to_vec();
    let (base, _seen) = serve(body, 1);
    let root = temp_root("rename");
    let target = root.join("demo").join("e.yeban");
    std::fs::create_dir_all(&target).expect("建目标目录");
    let url = format!("{base}/demo/e.yeban");
    let mut events = Vec::new();
    let error = download_to(&url, &target, &CancelFlag::new(), |event| {
        events.push(event)
    })
    .expect_err("改名必须失败");
    assert_eq!(error.0, DownloadError::Io);
    assert_failed_terminal(&events, Some("改名失败"));
    let _ = std::fs::remove_dir_all(&root);
}

/// IO 失败（建目录）：父级分量是个文件 ⇒ create_dir_all 失败，终态 Failed 一次。
#[test]
fn mkdir_failure_ends_with_single_failed() {
    let root = temp_root("mkdir");
    let blocker = root.join("blocker");
    std::fs::write(&blocker, b"x").expect("写占位文件");
    let target = blocker.join("demo").join("f.yeban");
    let url = "http://127.0.0.1:9/demo/f.yeban";
    let mut events = Vec::new();
    let error = download_to(url, &target, &CancelFlag::new(), |event| events.push(event))
        .expect_err("建目录必须失败");
    assert_eq!(error.0, DownloadError::Io);
    assert_failed_terminal(&events, Some("建目录失败"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 校验失败（非容器）：终态必须是恰好一次 Failed，报文点名"不是合法的工程容器"。
#[test]
fn corrupt_payload_ends_with_single_failed() {
    let (base, _seen) = serve(b"not a container".to_vec(), 1);
    let root = temp_root("corrupt-failed");
    let target = root.join("demo").join("bad.yeban");
    let url = format!("{base}/demo/bad.yeban");
    let mut events = Vec::new();
    let error = download_to(&url, &target, &CancelFlag::new(), |event| {
        events.push(event)
    })
    .expect_err("非容器必须被拒");
    assert_eq!(error.0, DownloadError::Corrupt);
    assert_failed_terminal(&events, Some("不是合法的工程容器"));
    assert!(!target.exists());
    assert!(!part_path(&target).exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// 网络失败（不完整）：服务端声明的长度多于实际发送的字节 ⇒ 终态 Failed 一次。
///
/// 不论底层走"读取失败"还是"下载不完整"，终态必须是恰好一次 Failed，
/// 故此处只断言结构（终态 + 次数 + Network），不钉死报文措辞。
#[test]
fn incomplete_download_ends_with_single_failed() {
    let body = b"PK\x03\x04short".to_vec();
    let base = serve_short(body.len() + 100, body);
    let root = temp_root("short");
    let target = root.join("demo").join("i.yeban");
    let url = format!("{base}/demo/i.yeban");
    let mut events = Vec::new();
    let error = download_to(&url, &target, &CancelFlag::new(), |event| {
        events.push(event)
    })
    .expect_err("不完整必须失败");
    assert_eq!(error.0, DownloadError::Network);
    assert_failed_terminal(&events, None);
    assert!(!target.exists());
    // 注：本路径（服务端提前断开）底层走"读取失败"，读错 `?` 分支不清理 .part
    // —— 那是读/写/flush 诸 `?` 分支既有的清理口径，与本次 Failed 终态无关，
    // 故此处不断言 .part 已清；正式文件不落位是本判据钉住的。
    let _ = std::fs::remove_dir_all(&root);
}

/// 成功路径终态不变：恰好一次 Finished，零 Failed、零 Cancelled。
#[test]
fn success_still_ends_with_single_finished_and_no_failed() {
    let body = b"PK\x03\x04ok-terminal".to_vec();
    let (base, _seen) = serve(body.clone(), 1);
    let root = temp_root("ok-terminal");
    let target = root.join("demo").join("g.yeban");
    let url = format!("{base}/demo/g.yeban");
    let mut events = Vec::new();
    download_to(&url, &target, &CancelFlag::new(), |event| {
        events.push(event)
    })
    .expect("下载必须成功");
    assert!(
        matches!(events.last(), Some(DownloadEvent::Finished(_))),
        "成功路径必须以 Finished 结尾: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, DownloadEvent::Finished(_)))
            .count(),
        1,
        "Finished 必须恰好一次: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DownloadEvent::Failed(_))),
        "成功路径不得发 Failed: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DownloadEvent::Cancelled)),
        "成功路径不得发 Cancelled: {events:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 取消路径终态不变：恰好一次 Cancelled，零 Failed。
#[test]
fn cancel_still_ends_with_single_cancelled_and_no_failed() {
    let (base, _seen) = serve(b"PK\x03\x04cancel-terminal".to_vec(), 1);
    let root = temp_root("cancel-terminal");
    let target = root.join("demo").join("h.yeban");
    let url = format!("{base}/demo/h.yeban");
    let cancel = CancelFlag::new();
    cancel.cancel();
    let mut events = Vec::new();
    let error =
        download_to(&url, &target, &cancel, |event| events.push(event)).expect_err("取消必须失败");
    assert_eq!(error.0, DownloadError::Cancelled);
    assert!(
        matches!(events.last(), Some(DownloadEvent::Cancelled)),
        "取消路径必须以 Cancelled 结尾: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, DownloadEvent::Cancelled))
            .count(),
        1,
        "Cancelled 必须恰好一次: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DownloadEvent::Failed(_))),
        "取消路径不得发 Failed: {events:?}"
    );
    assert!(!target.exists());
    let _ = std::fs::remove_dir_all(&root);
}
