//! 测试支持代码（std-only）。
//!
//! 刻意**不**使用 `tempfile`：本 crate 的依赖预算只允许 `thiserror`
//! （见 `docs/ledger/sfz-core-notes.md`「依赖纪律」）。这里用
//! `std::env::temp_dir()` + 进程号 + 原子计数器保证唯一性，并在 `Drop` 里清理。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 一个自动清理的临时目录。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// 创建一个唯一命名的临时目录。
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yeban-sfz-test-{}-{tag}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// 临时目录路径。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// SHA-256（纯 Rust、**无浮点**、无依赖）—— 用于把「两次运行相同」升级成**字面摘要契约**。
///
/// 为什么自己写：本 crate 的依赖预算只允许 `thiserror`，而 R70② 要求把
/// 「A == B 是自比」换成「`sha256(A) == 字面常量`」。全程只用整数运算
/// （`+`／`^`／`&`／`rotate_right`／比较），因此**不引入任何超越函数**（R77）。
///
/// `support` 被**多个**测试目标各自编译一次，而只有  用到本函数
/// ⇒ 其余目标需要 `allow(dead_code)`（用 `expect` 反而会在用到的目标里失败）。
#[allow(dead_code)]
pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut message = bytes.to_vec();
    let bit_len = (bytes.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    // 刻意不用 `chunks_exact`：`clippy::chunks-exact-to-as-chunks` 在 `-D warnings`
    // 下会把它顶回来（`as_chunks` 尚未稳定到我们愿意依赖）。显式切片一样清楚。
    for block in 0..message.len() / 64 {
        let chunk = &message[block * 64..block * 64 + 64];
        let mut w = [0u32; 64];
        for (index, word) in w.iter_mut().take(16).enumerate() {
            let bytes = &chunk[index * 4..index * 4 + 4];
            *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        let (mut e, mut f, mut g, mut hh) = (h[4], h[5], h[6], h[7]);
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// **R84 的文档契约形态**：按**描述**定位到那一行，再从**那一行**取 64 位十六进制摘要。
///
/// 为什么不用 `doc.contains(常量)`：那只证明「这个串在文件里出现过」——
/// **两行对调**、或某一行的描述名被改掉，`contains` 都照样通过（按构造可证：
/// `contains` 只问「出现过没有」，对调不改变出现集合）。
///
/// 实现：先筛出**含该描述**的行，再取该行里**恰好 64 位十六进制**的那个反引号单元格。
#[allow(dead_code)]
pub fn documented_digest(doc: &str, row_label: &str) -> Option<String> {
    doc.lines()
        .filter(|line| line.contains(row_label))
        .find_map(|line| {
            line.split('`')
                .find(|cell| cell.len() == 64 && cell.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .map(str::to_string)
        })
}
