//! 守卫：**判据表达式里的 `RefCell::borrow_mut()` 一律禁止**。
//!
//! ## 为什么需要这条判据
//!
//! 2026-11-22 macOS release 实测：设置→音频切输出设备，应用**直接 abort**
//! （`thread main panicked at host.rs:3515: RefCell already mutably borrowed`，
//! 随后 `panic in a function that cannot unwind`——回调跑在 winit/AppKit 的
//! extern C 派发里，展开穿不过去）。
//!
//! 形状是这样的：
//!
//! ```ignore
//! match production.borrow_mut().open_device(config) {
//!     Ok(_) => refresh_audio_status_line(&ui, &production), // 里面 production.borrow() ⇒ panic
//!     ...
//! }
//! ```
//!
//! `match` 判据表达式里的 `RefMut` **活过整个 `match`**（临时量的生命周期延到语句结束），
//! 于是任何一个分支再借同一个 `RefCell` 就 panic。`if let PAT = <expr 含 borrow_mut> { … }`
//! 同理。正确写法是先**结算**成值：`let opened = production.borrow_mut().open_device(..);`
//! 再 `match opened { .. }`。
//!
//! 仓里已有两处成文警告（`host.rs` 的 `reload` 与 `open_device` 旁），本判据把它们
//! 从「注释约定」变成「编译期之后仍会被执行的红线」：⛔ 不许只报"搜了没问题"。
//!
//! ## 判据不算「合法例外」
//!
//! 不提供白名单。唯一合法的写法（先结算）根本不命中本判据的扫描形状。

use std::fs;
use std::path::{Path, PathBuf};

/// 把注释与字符串字面量抹成空格（保留字节长度与换行，便于回报行号）。
///
/// 只做本判据需要的那点事：① `//` 行注释；② `/* */` 块注释（可嵌套一层就够）；
/// ③ `"…"` / `'…'` 字面量（避免里面的 `{` `}` `;` 干扰括号配平）。
fn strip_noise(src: &str) -> String {
    let bytes: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index];
        // 行注释
        if ch == '/' && index + 1 < bytes.len() && bytes[index + 1] == '/' {
            while index < bytes.len() && bytes[index] != '\n' {
                out.push(' ');
                index += 1;
            }
            continue;
        }
        // 块注释
        if ch == '/' && index + 1 < bytes.len() && bytes[index + 1] == '*' {
            out.push(' ');
            index += 1;
            out.push(' ');
            index += 1;
            while index + 1 < bytes.len() && !(bytes[index] == '*' && bytes[index + 1] == '/') {
                out.push(if bytes[index] == '\n' { '\n' } else { ' ' });
                index += 1;
            }
            if index + 1 < bytes.len() {
                out.push(' ');
                out.push(' ');
                index += 2;
            }
            continue;
        }
        // 原始字符串 r"..." / r#"..."#：无转义，终止符是引号 + 同数 #。
        // 按普通字符串抹会把末尾的 \" 当转义、吞掉后面整段代码 ⇒ 假违规。
        if ch == 'r' {
            let mut hashes = 0usize;
            while bytes.get(index + 1 + hashes) == Some(&'#') {
                hashes += 1;
            }
            if bytes.get(index + 1 + hashes) == Some(&'"') {
                out.push('r');
                index += 1;
                for _ in 0..hashes {
                    out.push('#');
                    index += 1;
                }
                out.push('"');
                index += 1;
                while index < bytes.len() {
                    let inner = bytes[index];
                    if inner == '"'
                        && (0..hashes).all(|off| bytes.get(index + 1 + off) == Some(&'#'))
                    {
                        out.push('"');
                        index += 1;
                        for _ in 0..hashes {
                            out.push('#');
                            index += 1;
                        }
                        break;
                    }
                    out.push(if inner == '\n' { '\n' } else { ' ' });
                    index += 1;
                }
                continue;
            }
        }
        // 字符串 / 字符字面量
        if ch == '"' || ch == '\'' {
            // ⚠ 单引号有两种含义：字符字面量 `'q'` / `'\n'`，以及**生命周期** `'a`。
            // 把生命周期当地一个字面量起头，就会一路吞到文件里下一个 `'`——中间整段代码
            // （包括真违规点）被抹成空格 ⇒ 判据假阴性（本判据第一版就漏了 `host.rs` 换工程
            // 分支里的设备重开，正是被前面某个 `<'a>` 吞掉的）。⇒ 只有"引号里确实是
            // 一个字符"时才进字面量模式，否则按生命周期前缀原样放行。
            if ch == '\'' {
                let escaped = bytes.get(index + 1) == Some(&'\\');
                let single_char = bytes.get(index + 2) == Some(&'\'');
                if !escaped && !single_char {
                    out.push(ch);
                    index += 1;
                    continue;
                }
            }
            let quote = ch;
            out.push(quote);
            index += 1;
            while index < bytes.len() {
                let inner = bytes[index];
                if inner == '\\' && index + 1 < bytes.len() {
                    out.push(' ');
                    out.push(' ');
                    index += 2;
                    continue;
                }
                if inner == quote {
                    out.push(quote);
                    index += 1;
                    break;
                }
                out.push(if inner == '\n' { '\n' } else { ' ' });
                index += 1;
            }
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

/// 从 `start` 起找到**顶层**（括号配平为 0 的）目标字符，返回其下标。
fn find_top_level(text: &[char], start: usize, target: char) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut index = start;
    while index < text.len() {
        let ch = text[index];
        // ⚠ 先判 target 再配平：`target == '{'` 时若先进 `'{' => depth += 1` 分支，
        // 判据表达式末尾的那个 `{` 永远返回不了（自检会红，判据变成装饰）。
        if ch == target && depth == 0 {
            return Some(index);
        }
        match ch {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
        index += 1;
    }
    None
}

/// 找出一处违规：返回 (行号, 片段)。
fn violations(src: &str) -> Vec<(usize, String)> {
    let clean = strip_noise(src);
    let chars: Vec<char> = clean.chars().collect();
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        // 关键字边界：`match` / `if let`
        let is_match = chars[index..].starts_with(&['m', 'a', 't', 'c', 'h'])
            && (index == 0 || !is_ident(chars[index - 1]))
            && chars.get(index + 5).is_none_or(|ch| !is_ident(*ch));
        let is_if_let = chars[index..].starts_with(&['i', 'f'])
            && (index == 0 || !is_ident(chars[index - 1]))
            && chars.get(index + 2) == Some(&' ')
            && chars[index..].len() > 5
            && chars[index + 3..].starts_with(&['l', 'e', 't'])
            && chars.get(index + 6).is_none_or(|ch| !is_ident(*ch));
        if !is_match && !is_if_let {
            index += 1;
            continue;
        }
        let after = index + if is_match { 5 } else { 6 };
        // `if let` 的模式里可能有 `{ … }`（结构体模式）⇒ 先跳到顶层 `=`，再找顶层 `{`。
        let head_start = if is_if_let {
            match find_top_level(&chars, after, '=') {
                Some(eq) => eq + 1,
                None => {
                    index = after;
                    continue;
                }
            }
        } else {
            after
        };
        let Some(brace) = find_top_level(&chars, head_start, '{') else {
            index = after;
            continue;
        };
        let head: String = chars[head_start..brace].iter().collect();
        if has_panicking_borrow_mut(&head) {
            let line = chars[..index].iter().filter(|ch| **ch == '\n').count() + 1;
            let snippet = head.trim().replace('\n', " ");
            found.push((line, snippet.chars().take(120).collect()));
        }
        index = brace + 1;
    }
    found
}

fn is_ident(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// 判据表达式里有没有**会 panic 的** `borrow_mut()`。
///
/// `try_borrow_mut()` 不算：借不到返回 `Err`，调用方 `else { return }` / `Err(_) =>`
/// 兜住即可 ⇒ 它是本判据**推荐**的写法（`#212` 的既有修法就是把它放在判据位置，
/// 让重入变成一个可处理的错误而不是 panic）。所以只认裸的 `borrow_mut()`。
fn has_panicking_borrow_mut(head: &str) -> bool {
    const NEEDLE: &str = "borrow_mut()";
    let mut rest = head;
    while let Some(pos) = rest.find(NEEDLE) {
        if !rest[..pos].ends_with("try_") {
            return true;
        }
        rest = &rest[pos + NEEDLE.len()..];
    }
    false
}

/// 仓库里所有 `yeban-app` 源文件（`src/**/*.rs`）。
fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();
    files
}

/// **判据本体**：`yeban-app` 全 crate 不许有「判据表达式里借可变」的写法。
#[test]
fn no_borrow_mut_in_judgement_head() {
    let mut reported: Vec<String> = Vec::new();
    for file in source_files() {
        let text = fs::read_to_string(&file).expect("读源文件");
        for (line, snippet) in violations(&text) {
            reported.push(format!("{}:{line} → {snippet}", file.display()));
        }
    }
    assert!(
        reported.is_empty(),
        "判据表达式里的 RefCell::borrow_mut() 会让 RefMut 活过整个 match/if-let；\
         任何分支再借同一个 RefCell 就 panic（release 下在 extern C 派发里 ⇒ abort）。\
         正确写法：先 `let x = cell.borrow_mut().f();` 再 `match x {{ .. }}`。\
         违规点：\n{}",
        reported.join("\n")
    );
}

/// 自检：判据自己的扫描器要能认出现场那个形状，且**不能**把合法写法当成违规。
///
/// （判据的判据：没有这条，扫描器写错就变成一条永远绿的装饰。）
#[test]
fn scanner_recognises_the_incident_and_accepts_the_fix() {
    let incident = r#"
        match production.borrow_mut().open_device(config) {
            Ok(_) => refresh_audio_status_line(&ui, &production),
            Err(error) => report(error),
        }
    "#;
    assert_eq!(
        violations(incident).len(),
        1,
        "必须认出事故形状（match 判据里 borrow_mut）"
    );

    let if_let_incident = r#"
        if let MixerGesture::Fader { last_db, moved, .. } = &mut *gesture.borrow_mut() {
            *moved = true;
        }
    "#;
    assert_eq!(
        violations(if_let_incident).len(),
        1,
        "结构体模式里有花括号也必须认出 if-let 形状"
    );

    for fixed in [
        r#"
        let opened = production.borrow_mut().open_device(config);
        match opened {
            Ok(_) => refresh_audio_status_line(&ui, &production),
            Err(error) => report(error),
        }
    "#,
        r#"
        match kind {
            Kind::A => cell.borrow_mut().push(1),
            Kind::B => {}
        }
    "#,
    ] {
        assert!(
            violations(fixed).is_empty(),
            "合法写法（先结算 / 判据里不借）不得误报：{fixed}"
        );
    }

    // 注释里写形状不算违规（否则解释性注释会把判据逼红）。
    let commented = r#"
        // match production.borrow_mut().open_device(config) { .. }  ← 反例，注释里
        let opened = production.borrow_mut().open_device(config);
        match opened { Ok(_) => {}, Err(_) => {} }
    "#;
    assert!(violations(commented).is_empty(), "注释里的反例不得算违规");

    // ⚠ 生命周期不得吞掉后面的代码：本判据第一版把 `'a` 当成字符字面量的起头，
    // 于是一路吞到下一个 `'`，`host.rs` 换工程分支里那处设备重开（真违规）被抹成空格
    // ⇒ 判据假阴性。这条自检盯死那个形状：前面有生命周期、后面照样要认出来。
    let after_lifetime = r#"
        fn project<'a>(path: &'a str) -> char { 'q' }
        match production.borrow_mut().open_device(config) {
            Ok(_) => report(production.borrow().status()),
            Err(_) => {}
        }
    "#;
    assert_eq!(
        violations(after_lifetime).len(),
        1,
        "生命周期 / 字符字面量不得让后面的违规点漏报"
    );

    // `try_borrow_mut()` **不**算违规：借不到是 `Err`，调用方兜住 ⇒ 这是推荐写法
    // （`#212` 的既有修法）。判据只看裸的 `borrow_mut()`。
    let try_form = r#"
        let toggle = {
            let mut recorder = match recorder_cell.try_borrow_mut() {
                Ok(recorder) => recorder,
                Err(_) => return,
            };
            recorder.is_recording()
        };
    "#;
    assert!(
        violations(try_form).is_empty(),
        "try_borrow_mut 在判据位置是推荐写法，不得逼红"
    );
    // 但同一个判据表达式里既有 try 又有裸 borrow_mut ⇒ 仍要认出来。
    let mixed = r#"
        match cell.try_borrow_mut().and_then(|_| other.borrow_mut().take()) {
            Ok(v) => v,
            Err(_) => {}
        }
    "#;
    assert_eq!(
        violations(mixed).len(),
        1,
        "混在一条判据里的裸 borrow_mut 仍算违规"
    );
}
