//! `.yeban` 容器的正向判据：往返逐字节一致、写入确定性（`ARCH-DET-001`）、
//! §5.3 内容布局（`project.json` / `history.dag` / `assets/{sha256}`）、
//! 以及与真实 `unzip` 的互操作实测。
//!
//! 规范来源：
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.3 `[ARCH-SEC-003]`
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §5.2 `[ARCH-DET-001]`（写入确定性）
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `[MUST-GATE-006]` / `[MUST-GATE-007]`

use std::collections::BTreeMap;
use std::process::Command;

use proptest::prelude::*;
use yeban_model::container::{
    ASSETS_DIR, ContainerEntry, ContainerError, ContainerLimits, HISTORY_DAG_NAME,
    PROJECT_JSON_NAME, ProjectArchive, asset_entry_name, read_container, read_project_container,
    write_container, write_project_container, write_project_container_borrowed,
};
use yeban_model::{AssetHash, YebanProjectV1};

/// 确定性的伪随机字节（splitmix64）：不引入任何依赖，也不依赖系统随机源。
fn pseudo_random_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        out.extend_from_slice(&(z ^ (z >> 31)).to_le_bytes());
    }
    out.truncate(len);
    out
}

/// 往返判据的公共断言：写 → 读 → 逐字节相同，且两次写入的归档字节相同。
fn assert_round_trip(entries: &[ContainerEntry]) {
    let first = write_container(entries).expect("写入必须成功");
    let second = write_container(entries).expect("写入必须成功");
    assert_eq!(
        first, second,
        "同一输入两次写入必须逐字节相同 [ARCH-DET-001]"
    );

    let defaults = ContainerLimits::default();
    let archive = read_container(&first, &defaults).expect("读回必须成功");
    assert_eq!(
        archive.entries(),
        entries,
        "读回的条目必须与写入的逐字节相同"
    );

    // 用"读出的条目再写一次"也必须得到同样的字节（闭环）。
    let rewritten = write_container(archive.entries()).expect("重写必须成功");
    assert_eq!(rewritten, first, "读→写 闭环必须稳定");
}

/// 空容器往返（`unzip -l` 也能读的 22 字节 EOCD）。
#[test]
fn empty_container_round_trips() {
    assert_round_trip(&[]);
}

/// 单条目往返。
#[test]
fn single_entry_round_trips() {
    assert_round_trip(&[ContainerEntry::new(PROJECT_JSON_NAME, b"{}".to_vec())]);
}

/// 多条目往返（含嵌套路径与二进制内容）。
#[test]
fn multi_entry_round_trips() {
    assert_round_trip(&[
        ContainerEntry::new(PROJECT_JSON_NAME, br#"{"schema_version":1}"#.to_vec()),
        ContainerEntry::new(HISTORY_DAG_NAME, b"dag-bytes".to_vec()),
        ContainerEntry::new("assets/deadbeef", vec![0x00, 0xFF, 0x7F, 0x80]),
    ]);
}

/// 几 MB 的大条目往返逐字节相同（真实音频资产规模的代理）。
#[test]
fn megabyte_payload_round_trips_byte_exact() {
    let payload = pseudo_random_bytes(3 * 1024 * 1024, 0x1234_5678_9ABC_DEF0);
    assert_round_trip(&[ContainerEntry::new("assets/big", payload)]);
}

/// 归档视图的辅助方法。
#[test]
fn archive_view_helpers_behave() {
    let entries = vec![
        ContainerEntry::new("a", b"1".to_vec()),
        ContainerEntry::new("b/c", b"2".to_vec()),
    ];
    let archive = read_container(
        &write_container(&entries).unwrap(),
        &ContainerLimits::default(),
    )
    .expect("读回必须成功");
    assert_eq!(archive.len(), 2);
    assert!(!archive.is_empty());
    assert_eq!(archive.names().collect::<Vec<_>>(), vec!["a", "b/c"]);
    assert_eq!(
        archive.get("b/c").map(|entry| entry.data.as_slice()),
        Some(&b"2"[..])
    );
    assert!(archive.get("missing").is_none());
    assert_eq!(archive.into_entries(), entries);
}

/// 真实 `unzip` 的互操作实测：本实现写出的是**标准 ZIP**，不是自演格式。
///
/// 若机器上没有 `unzip`（极小化 CI 容器），判据会打印一行说明并跳过 —— 跳过是**显式**的，
/// 不会伪装成通过；本机实测输出见 `docs/ledger/container-notes.md` §unzip。
#[test]
fn unzip_reads_our_container() {
    let payload = pseudo_random_bytes(4096, 42);
    let bytes = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, br#"{"schema_version":1}"#.to_vec()),
        ContainerEntry::new("assets/abc", payload.clone()),
    ])
    .expect("写入必须成功");

    let dir = std::env::temp_dir().join(format!("yeban-container-interop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录");
    let path = dir.join("interop.yeban");
    std::fs::write(&path, &bytes).expect("写出临时归档");

    let list = match Command::new("unzip").arg("-l").arg(&path).output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("skip: 本机没有 unzip ({error})");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
    };
    assert!(list.status.success(), "unzip -l 必须成功");
    let listing = String::from_utf8_lossy(&list.stdout);
    assert!(
        listing.contains(PROJECT_JSON_NAME),
        "清单必须含 project.json: {listing}"
    );
    assert!(
        listing.contains("assets/abc"),
        "清单必须含 assets/abc: {listing}"
    );

    let test = Command::new("unzip")
        .arg("-t")
        .arg(&path)
        .output()
        .expect("unzip -t");
    assert!(test.status.success(), "unzip -t 必须报告无损坏");

    let extracted = Command::new("unzip")
        .arg("-p")
        .arg(&path)
        .arg("assets/abc")
        .output()
        .expect("unzip -p");
    assert!(extracted.status.success(), "unzip -p 必须成功");
    assert_eq!(
        extracted.stdout, payload,
        "unzip 解出的字节必须与写入的完全相同"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// §5.3 内容布局：project.json / history.dag / assets/{sha256}
// ======================================================================

/// 组装一个带两份资产的工程容器夹具。
fn project_fixture() -> (YebanProjectV1, Vec<u8>, BTreeMap<AssetHash, Vec<u8>>) {
    let project = YebanProjectV1::default();
    let history = b"[{\"index\":0,\"root\":true}]".to_vec();
    let mut assets = BTreeMap::new();
    for data in [b"kick-bytes".to_vec(), b"snare-bytes".to_vec()] {
        assets.insert(AssetHash::of_bytes(&data), data);
    }
    (project, history, assets)
}

/// 工程容器往返：文档、提交树、资产池三者逐字节守恒。
#[test]
fn project_container_round_trips() {
    let (project, history, assets) = project_fixture();
    let bytes = write_project_container(&project, &history, &assets).expect("写入必须成功");
    let ProjectArchive {
        project: read_project,
        history_dag,
        assets: read_assets,
    } = read_project_container(&bytes, &ContainerLimits::default()).expect("读回必须成功");

    assert_eq!(read_project, project);
    assert_eq!(history_dag, history);
    let expected: Vec<(AssetHash, Vec<u8>)> = assets.into_iter().collect();
    assert_eq!(read_assets, expected);

    // 条目名必须是 §5.3 的 `assets/{sha256}` 形状（64 位小写十六进制）。
    let name = asset_entry_name(&expected[0].0);
    assert_eq!(
        name,
        format!("{ASSETS_DIR}/{}", expected[0].0),
        "资产条目名必须由 ASSETS_DIR 与摘要拼成"
    );
    assert_eq!(name.len(), ASSETS_DIR.len() + 1 + 64);
    assert!(name.starts_with("assets/"));
}

/// `ARCH-DET-001`：`BTreeMap` 的插入顺序不影响归档字节（键序确定 ⇒ 字节确定）。
///
/// ⚠ 注意本条的**判别力有限**：`BTreeMap` 的迭代序由键序决定，与插入顺序无关，
/// 因此下面的 `.rev()` 之后迭代序与原来**逐元素相同** —— 它证明的是
/// "`write_project_container` 不读 `BTreeMap` 的内部布局"，**不是**"写入器对任意
/// 输入顺序免疫"。后者由 `borrowed_asset_writer_is_independent_of_input_slice_order`
/// （确定性反转）与 `borrowed_asset_writer_is_permutation_invariant`（proptest 旋转）承担。
#[test]
fn project_container_is_independent_of_btreemap_insertion_order() {
    let (project, history, assets) = project_fixture();
    let reversed: BTreeMap<AssetHash, Vec<u8>> = assets
        .iter()
        .rev()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let first = write_project_container(&project, &history, &assets).expect("写入必须成功");
    let second = write_project_container(&project, &history, &reversed).expect("写入必须成功");
    assert_eq!(first, second, "资产插入顺序不得影响归档字节");
}

// ======================================================================
// 借用形态的资产写出面（`docs/ledger/app-cli-notes.md` §8 needs-2）
// ======================================================================

/// 借用形态与 `BTreeMap` 形态**逐字节一致**：两条入口共用同一份写入核心,
/// 因此接入借用形态不改变任何既有 `.yeban` 文件的字节。
#[test]
fn borrowed_asset_writer_is_byte_identical_to_the_btreemap_writer() {
    let (project, history, assets) = project_fixture();
    let borrowed: Vec<(&AssetHash, &[u8])> = assets
        .iter()
        .map(|(hash, data)| (hash, data.as_slice()))
        .collect();

    let from_owned = write_project_container(&project, &history, &assets).expect("写入必须成功");
    let from_borrowed =
        write_project_container_borrowed(&project, &history, &borrowed).expect("写入必须成功");
    assert_eq!(
        from_owned, from_borrowed,
        "同一工程内容经两条入口必须写出逐字节相同的归档"
    );

    // 借用形态同样守恒：读回的工程 / 提交树 / 资产池与输入相同。
    let archive =
        read_project_container(&from_borrowed, &ContainerLimits::default()).expect("读回必须成功");
    assert_eq!(archive.project, project);
    assert_eq!(archive.history_dag, history);
    let expected: Vec<(AssetHash, Vec<u8>)> = assets.into_iter().collect();
    assert_eq!(archive.assets, expected);
}

/// `ARCH-DET-001`：借用形态的**输入切片顺序**不影响归档字节（写出前按哈希升序）。
#[test]
fn borrowed_asset_writer_is_independent_of_input_slice_order() {
    let (project, history, assets) = project_fixture();
    let mut borrowed: Vec<(&AssetHash, &[u8])> = assets
        .iter()
        .map(|(hash, data)| (hash, data.as_slice()))
        .collect();
    assert_eq!(borrowed.len(), 2, "夹具必须有两份资产才能谈顺序");

    let first =
        write_project_container_borrowed(&project, &history, &borrowed).expect("写入必须成功");
    borrowed.reverse();
    let second =
        write_project_container_borrowed(&project, &history, &borrowed).expect("写入必须成功");
    assert_eq!(first, second, "资产切片顺序不得影响归档字节");
    assert_eq!(
        first,
        write_project_container(&project, &history, &assets).expect("写入必须成功"),
        "借用形态的字节必须等于 BTreeMap 形态（键序是唯一权威顺序）"
    );
}

/// CAS 完整性：借用形态同样拒绝"字节与哈希不符"。
#[test]
fn borrowed_asset_writer_rejects_a_hash_mismatch() {
    let (project, history, _) = project_fixture();
    let claimed = AssetHash::of_bytes(b"claimed");
    let assets: Vec<(&AssetHash, &[u8])> = vec![(&claimed, b"actual".as_slice())];
    match write_project_container_borrowed(&project, &history, &assets) {
        Err(ContainerError::AssetHashMismatch { declared, actual }) => {
            assert_eq!(declared, claimed.to_string());
            assert_eq!(actual, AssetHash::of_bytes(b"actual").to_string());
        }
        other => panic!("期望 AssetHashMismatch，实际 {other:?}"),
    }
}

/// 同一条资产出现两次（哈希重复）必须拒绝：`BTreeMap` 形态**不可能**表达这种输入,
/// 因此这条判据只可能落在借用形态上。
#[test]
fn borrowed_asset_writer_rejects_duplicate_asset_hashes() {
    let (project, history, _) = project_fixture();
    let data = b"kick-bytes";
    let hash = AssetHash::of_bytes(data);
    let assets: Vec<(&AssetHash, &[u8])> = vec![(&hash, data.as_slice()), (&hash, data.as_slice())];
    match write_project_container_borrowed(&project, &history, &assets) {
        Err(ContainerError::DuplicateEntryName { name }) => {
            assert_eq!(name, asset_entry_name(&hash), "重复的条目名必须被点名");
        }
        other => panic!("期望 DuplicateEntryName，实际 {other:?}"),
    }
}

/// CAS 完整性：资产字节与条目名（哈希）不符必须拒绝。
#[test]
fn asset_hash_mismatch_is_rejected() {
    let (project, history, _) = project_fixture();
    let mut assets = BTreeMap::new();
    assets.insert(AssetHash::of_bytes(b"claimed"), b"actual".to_vec());
    match write_project_container(&project, &history, &assets) {
        Err(ContainerError::AssetHashMismatch { declared, actual }) => {
            assert_eq!(declared, AssetHash::of_bytes(b"claimed").to_string());
            assert_eq!(actual, AssetHash::of_bytes(b"actual").to_string());
        }
        other => panic!("期望 AssetHashMismatch，实际 {other:?}"),
    }
}

/// 读取侧同样校验 CAS：条目名与内容不符必须拒绝。
#[test]
fn read_project_container_verifies_asset_hashes() {
    let bytes = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, b"{}".to_vec()),
        ContainerEntry::new(HISTORY_DAG_NAME, b"[]".to_vec()),
        ContainerEntry::new(
            format!("{ASSETS_DIR}/{}", AssetHash::of_bytes(b"claimed")),
            b"tampered".to_vec(),
        ),
    ])
    .expect("写入必须成功");
    match read_project_container(&bytes, &ContainerLimits::default()) {
        Err(ContainerError::AssetHashMismatch { declared, actual }) => {
            assert_eq!(declared, AssetHash::of_bytes(b"claimed").to_string());
            assert_eq!(actual, AssetHash::of_bytes(b"tampered").to_string());
        }
        other => panic!("期望 AssetHashMismatch，实际 {other:?}"),
    }
}

/// 资产条目名必须是规范 SHA-256 文本。
#[test]
fn malformed_asset_entry_name_is_rejected() {
    let bytes = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, b"{}".to_vec()),
        ContainerEntry::new(HISTORY_DAG_NAME, b"[]".to_vec()),
        ContainerEntry::new("assets/NOT-A-HASH", b"x".to_vec()),
    ])
    .expect("写入必须成功");
    assert_eq!(
        read_project_container(&bytes, &ContainerLimits::default()),
        Err(ContainerError::InvalidAssetName {
            name: "assets/NOT-A-HASH".into()
        })
    );
}

/// §5.3 之外的条目必须拒绝（避免"容器里塞任意文件"变成隐性后门）。
#[test]
fn foreign_entries_are_rejected_by_the_project_reader() {
    let bytes = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, b"{}".to_vec()),
        ContainerEntry::new(HISTORY_DAG_NAME, b"[]".to_vec()),
        ContainerEntry::new("notes.txt", b"hi".to_vec()),
    ])
    .expect("写入必须成功");
    assert_eq!(
        read_project_container(&bytes, &ContainerLimits::default()),
        Err(ContainerError::UnexpectedContainerEntry {
            name: "notes.txt".into()
        })
    );
}

/// 缺少 `project.json`。
#[test]
fn missing_project_json_is_rejected() {
    let bytes = write_container(&[ContainerEntry::new(HISTORY_DAG_NAME, b"[]".to_vec())]).unwrap();
    assert_eq!(
        read_project_container(&bytes, &ContainerLimits::default()),
        Err(ContainerError::MissingProjectJson)
    );
}

/// 缺少 `history.dag`（`project.json` 本身合法，因此必须是 `MissingHistoryDag`）。
#[test]
fn missing_history_dag_is_rejected() {
    let project_json = serde_json::to_vec(&YebanProjectV1::default()).expect("序列化工程");
    let bytes = write_container(&[ContainerEntry::new(PROJECT_JSON_NAME, project_json)]).unwrap();
    assert_eq!(
        read_project_container(&bytes, &ContainerLimits::default()),
        Err(ContainerError::MissingHistoryDag)
    );
}

/// `project.json` 不是合法工程文档。
#[test]
fn invalid_project_json_is_rejected() {
    let bytes = write_container(&[
        ContainerEntry::new(PROJECT_JSON_NAME, b"{not json".to_vec()),
        ContainerEntry::new(HISTORY_DAG_NAME, b"[]".to_vec()),
    ])
    .unwrap();
    assert!(matches!(
        read_project_container(&bytes, &ContainerLimits::default()),
        Err(ContainerError::InvalidProjectJson { .. })
    ));
}

// ======================================================================
// 属性测试：随机条目的往返
// ======================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// 随机条目集合的往返必须逐字节守恒，且两次写入必须相同。
    #[test]
    fn random_entries_round_trip_byte_exact(
        raw in prop::collection::vec((0usize..16, prop::collection::vec(any::<u8>(), 0..96)), 0..6)
    ) {
        let mut seen = std::collections::BTreeSet::new();
        let mut entries = Vec::new();
        for (index, data) in raw {
            let name = format!("e{index}");
            if seen.insert(name.clone()) {
                entries.push(ContainerEntry::new(name, data));
            }
        }

        let first = write_container(&entries).expect("写入必须成功");
        let second = write_container(&entries).expect("写入必须成功");
        prop_assert_eq!(&first, &second);

        let archive = read_container(&first, &ContainerLimits::default()).expect("读回必须成功");
        prop_assert_eq!(archive.entries(), entries.as_slice());
    }

    /// 借用形态对**任意输入顺序**都写出同一份字节（写出前按哈希升序）,
    /// 且读回的资产池与输入集合逐字节相同（同样按哈希升序）。
    #[test]
    fn borrowed_asset_writer_is_permutation_invariant(
        seeds in prop::collection::vec(any::<u64>(), 1..5)
    ) {
        let project = YebanProjectV1::default();
        let history = b"history".to_vec();
        let mut owned: Vec<(AssetHash, Vec<u8>)> = Vec::new();
        for seed in &seeds {
            let len = 1 + (seed % 64) as usize;
            let data = pseudo_random_bytes(len, *seed);
            let hash = AssetHash::of_bytes(&data);
            if owned.iter().any(|(existing, _)| existing == &hash) {
                continue;
            }
            owned.push((hash, data));
        }
        prop_assume!(!owned.is_empty());
        owned.sort_by(|left, right| left.0.cmp(&right.0));

        let reference: Vec<(&AssetHash, &[u8])> = owned
            .iter()
            .map(|(hash, data)| (hash, data.as_slice()))
            .collect();
        let baseline =
            write_project_container_borrowed(&project, &history, &reference).expect("写入必须成功");

        let mut rotated = reference.clone();
        if rotated.len() >= 2 {
            // 旋转 1 位在 `len >= 2` 时**一定**改变顺序（`% len` 会退化成恒等变换）。
            rotated.rotate_left(1);
        }
        let again =
            write_project_container_borrowed(&project, &history, &rotated).expect("写入必须成功");
        prop_assert_eq!(&baseline, &again);

        let archive =
            read_project_container(&baseline, &ContainerLimits::default()).expect("读回必须成功");
        prop_assert_eq!(archive.assets, owned);
    }
}
