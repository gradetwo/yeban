//! 语义控件树查询的 **JSON 形态**（稳定键序）—— `[UI-TEST-001]` / `[UI-MCP-001]` / `[UI-MCP-002]`。
//!
//! ## 这是**投影**，不是第二套模型
//!
//! 控件树的权威数据模型住在 `yeban_ui_test_port::tree`（[`ControlTree`] / [`ControlNode`] /
//! [`Role`] / [`Rect`]），由 `yeban-app` 的适配器与 `inspect::tree_from_element_root` 两条路填充。
//! 本模块**不重新定义节点**，只把那个模型投影成 JSON-RPC 的线格式。这样做的理由：
//!
//! 1. 依赖方向：`yeban-ui-test-port` 不知道 JSON-RPC，也不该知道（它连 `serde_json` 的
//!    用法都只限于 `dump_json`/`from_json` 的自有形状）；
//! 2. 线格式是**协议**而不是模型 —— 它需要一样模型里没有的东西：`visible` 的**证据强度**
//!    （见下）与 `source`（这棵树是从运行时遍历来的，还是在没有窗口时的静态投影）。
//!
//! 反过来，本模块**绝不**改 `yeban-ui-test-port` 的节点结构（那会让 `yeban-app` 的适配器
//! 与既有 47 条判据一起漂移）。若模型将来需要新字段，正确的做法是集成者裁决后改那一侧，
//! 本模块跟着投影 —— 而不是在这里长出一个第二事实源。
//!
//! ## `visible` 为什么是 `Option<bool>`（**不编造 false**）
//!
//! 上游 `visit_descendants` 的"可见"是**几何裁剪相交**的结论，不是读 `visible` 属性
//! （`i-slint-core-1.18.1/item_tree.rs:410-419`；见 `docs/ledger/ui-test-port-notes.md` §2 第 15 条）。
//! 因此本投影的规则是：
//!
//! | 来源 | `bounds` | `visible` | 含义 |
//! | :--- | :--- | :--- | :--- |
//! | [`TreeSource::Runtime`] | 有 | `true` | 有几何 ⇒ 它通过了上游的裁剪相交测试 ⇒ 当前帧可寻址 |
//! | [`TreeSource::Runtime`] | 无 | `null` | 有实例但拿不到几何 ⇒ **不可断定** |
//! | [`TreeSource::Registry`] | 无 | `null` | 静态注册表根本没有几何 ⇒ **不可断定** |
//!
//! 我们**永远不会**输出 `visible: false`：没有任何一条来源能给出"确定不可见"的正证据
//! （不可见的分支根本不会出现在运行时树里，而注册表不知道可见性）。
//! 编造 `false` 会让调用方以为"这个元素存在但被隐藏了" —— 那是错的。
//!
//! ## 稳定键序（红线 4 的精神）
//!
//! 节点按**语义 ID 升序**排列（`ControlTree` 内部是 `BTreeMap`，与插入顺序无关），
//! 每个节点的键序由结构体字段声明顺序固定。两条判据钉住它：
//! `json_is_byte_stable_regardless_of_insertion_order` 与
//! `tree_method_result_is_byte_stable_across_two_calls`（后者在 [`crate::service`] 里）。
//! CI 的 JSON 断言因此可以直接对字节断言，而不用排序后再比。

use serde::{Deserialize, Serialize};

use yeban_ui_test_port::image::Rect;
use yeban_ui_test_port::tree::ControlTree;

/// 这棵树的来源 —— 调用方据此知道自己拿到的**证据有多强**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TreeSource {
    /// 运行时遍历（`i-slint-backend-testing` 的 `ElementHandle`）。
    ///
    /// 它已经被上游按"几何可见"过滤过，因此**有几何的节点就是当前帧真正可寻址的节点**。
    /// 代价：`visible: false` 的分支不在里面 ⇒ 覆盖全集要另配一份注册表（见 `coverage`）。
    Runtime,
    /// 静态注册表投影（`yeban-app::elements::ElementRegistry` 一类，无窗口也能跑）。
    ///
    /// 它覆盖**声明过的全集**，但**没有任何几何**，因此 `bounds` 恒为 `null`、
    /// `visible` 恒为 `null`。
    Registry,
}

impl TreeSource {
    /// 线格式字面值。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Registry => "registry",
        }
    }
}

/// 一个语义节点的 JSON 投影。
///
/// **字段声明顺序 = JSON 键序**，不要为了"看起来顺眼"重排：那会改掉调用方断言的字节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiNode {
    /// 语义 ID（`[UI-TEST-001]` §12.2 的四个族：`track-{i}-fader` / `note-{ulid}-rect` /
    /// `clip-{ulid}-header` / `tab-{name}-button`）。
    pub id: String,
    /// `accessible-role`（kebab-case，取值集合见 `yeban_ui_test_port::tree::KNOWN_ROLES`）。
    pub role: String,
    /// `accessible-label`（无人可读标签时是空串，不是 `null`）。
    pub label: String,
    /// 几何包围盒；无几何证据时为 `null`。
    pub bounds: Option<Rect>,
    /// 当前帧是否可寻址 —— `null` 表示**不可断定**（见模块文档的表）。
    pub visible: Option<bool>,
    /// `[UI-MCP-002]` 高频刷新区（VU 表 / 走带光标 / RTA / 时间码），截图比对前必须置黑。
    pub dynamic_region: bool,
    /// 该节点是否**当前可遮罩**：动态区 **且** 有包围盒。
    ///
    /// 与 `dynamicRegion` 分开报出来，是因为"登记成动态区但没有几何"是一条必须显式
    /// 处理的状况（[`crate::tree::TreeError::DynamicRegionWithoutBounds`]）；把它藏进
    /// 一个布尔里会让调用方以为遮罩成功了。
    pub maskable: bool,
    /// 父节点的语义 ID（`null` 表示根，或该来源给不出层级）。
    pub parent: Option<String>,
}

/// 整棵树的 JSON 投影。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiTree {
    /// 来源（决定 `visible` 的证据强度）。
    pub source: TreeSource,
    /// 节点数（等于 `nodes.len()`，冗余但让调用方不必先解析数组）。
    pub count: usize,
    /// 节点，**按语义 ID 升序**。
    pub nodes: Vec<UiNode>,
}

/// 投影失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    /// 动态刷新区没有包围盒 —— `[UI-MCP-002]` 的遮罩无从执行。
    #[error("动态刷新区 `{id}` 没有包围盒: [UI-MCP-002] 的遮罩无法执行")]
    DynamicRegionWithoutBounds {
        /// 出错的语义 ID。
        id: String,
    },
}

impl UiTree {
    /// 从**运行时**控件树投影（有几何 ⇒ `visible: true`）。
    #[must_use]
    pub fn from_runtime(tree: &ControlTree) -> Self {
        Self::project(tree, TreeSource::Runtime)
    }

    /// 从**静态注册表**投影（无几何 ⇒ `visible: null`）。
    #[must_use]
    pub fn from_registry(tree: &ControlTree) -> Self {
        Self::project(tree, TreeSource::Registry)
    }

    fn project(tree: &ControlTree, source: TreeSource) -> Self {
        // `ControlTree::iter` 是 `BTreeMap` 的值迭代 ⇒ 已经按 ID 升序。
        // 这里**不**再排序: 排序会掩盖"模型换了容器"这类改动 (红线 4 的精神是让顺序来自
        // 容器本身), 而且一条判据专门断言 `ids` 与 `sorted` 相等。
        let nodes = tree
            .iter()
            .map(|node| UiNode {
                id: node.id.clone(),
                role: node.role.as_str().to_owned(),
                label: node.label.clone(),
                bounds: node.bounds,
                visible: match (source, node.bounds) {
                    (TreeSource::Runtime, Some(_)) => Some(true),
                    _ => None,
                },
                dynamic_region: node.dynamic_region,
                maskable: node.needs_masking(),
                parent: node.parent.clone(),
            })
            .collect::<Vec<_>>();
        Self {
            source,
            count: nodes.len(),
            nodes,
        }
    }

    /// 按语义 ID 精确查找（线性扫描 `nodes`，顺序稳定）。
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&UiNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// §12.2 的族查询 + `[UI-MCP-002]` 的动态区过滤（**保持 ID 升序**）。
    ///
    /// 过滤只影响 `nodes`/`count`，不改变 `source` —— 调用方因此仍然知道
    /// "这些 `visible` 是哪种强度的证据"。
    #[must_use]
    pub fn filtered(&self, prefix: Option<&str>, dynamic_only: bool) -> Self {
        let nodes = self
            .nodes
            .iter()
            .filter(|node| prefix.is_none_or(|prefix| node.id.starts_with(prefix)))
            .filter(|node| !dynamic_only || node.dynamic_region)
            .cloned()
            .collect::<Vec<_>>();
        Self {
            source: self.source,
            count: nodes.len(),
            nodes,
        }
    }

    /// `[UI-MCP-002]` 的全部动态区节点（按 ID 升序）。
    pub fn dynamic_regions(&self) -> impl Iterator<Item = &UiNode> + '_ {
        self.nodes.iter().filter(|node| node.dynamic_region)
    }

    /// `[UI-MCP-002]` 截图比对前必须置黑的矩形，**连同它是谁**一起返回。
    ///
    /// 与 `yeban_ui_test_port::mask::mask_rects_from_tree` 的区别只有一处：
    /// 这里保留 `id`，因为 JSON 调用方需要把矩形映射回语义 ID（判据
    /// `dynamic_region_rects_come_from_the_runtime_bounds` 断言这条链路，
    /// 注入验证 D 就是把它改成硬编码坐标）。
    ///
    /// # Errors
    ///
    /// 有节点登记成动态区却没有包围盒 —— 拒绝静默放过。
    pub fn mask_rects(&self) -> Result<Vec<(&str, Rect)>, ProjectionError> {
        let mut rects = Vec::new();
        for node in self.dynamic_regions() {
            match node.bounds {
                Some(rect) => rects.push((node.id.as_str(), rect)),
                None => {
                    return Err(ProjectionError::DynamicRegionWithoutBounds {
                        id: node.id.clone(),
                    });
                }
            }
        }
        Ok(rects)
    }

    /// 稳定 JSON（美化 + 结尾换行）。
    ///
    /// 不 panic：`serde_json` 序列化本类型不可能失败（无 NaN、无非常量键），
    /// 但生产代码里不留 `unwrap`；失败时退化为 `{}` —— 那会立刻被
    /// `json_is_byte_stable_regardless_of_insertion_order` 抓住。
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        let mut out = serde_json::to_string_pretty(self).unwrap_or_else(|_| String::from("{}"));
        out.push('\n');
        out
    }

    /// 紧凑 JSON（线上形态：`ui/tree` 的 `result.tree` 就是这么出去的）。
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| String::from("{}"))
    }
}

/// 双向覆盖（`[UI-TEST-001]`）：运行时树与静态注册表**两个方向**的差异。
///
/// 上游遍历是几何可见性过滤后的集合，所以：
/// - `missingAtRuntime`（注册表有、运行时没有）**必然非空**（`visible: false` 的分支不在树里）；
/// - `unknownAtRuntime`（运行时有、注册表没有）**必须为空**，否则就是"UI 加了 ID 忘了登记"。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    /// 注册表登记了但运行时树里找不到的 ID（升序）。
    pub missing_at_runtime: Vec<String>,
    /// 运行时树里有但注册表没登记的 ID（升序）。
    pub unknown_at_runtime: Vec<String>,
    /// 运行时节点数。
    pub runtime_count: usize,
    /// 注册表节点数。
    pub registry_count: usize,
}

impl Coverage {
    /// 两个方向都为空才算闭合。
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing_at_runtime.is_empty() && self.unknown_at_runtime.is_empty()
    }

    /// 用一份注册表 ID 清单做交叉核对（线上形态：调用方把注册表快照传进来）。
    #[must_use]
    pub fn between(runtime: &UiTree, registry_ids: &[String]) -> Self {
        let mut missing = Vec::new();
        for id in registry_ids {
            if runtime.find(id).is_none() {
                missing.push(id.clone());
            }
        }
        let mut unknown = Vec::new();
        for node in &runtime.nodes {
            if !registry_ids.iter().any(|id| id == &node.id) {
                unknown.push(node.id.clone());
            }
        }
        // 两个方向都排过序: 输入顺序不得影响输出 (稳定 JSON 的前提)。
        missing.sort_unstable();
        unknown.sort_unstable();
        Self {
            missing_at_runtime: missing,
            unknown_at_runtime: unknown,
            runtime_count: runtime.count,
            registry_count: registry_ids.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_ui_test_port::tree::{ControlNode, Role};

    fn role(name: &str) -> Role {
        Role::parse(name).expect("测试角色必须是合法取值")
    }

    /// 一棵确定性夹具树：3 个静态节点 + 2 个高频刷新区（几何都是**运行时**才有的）。
    fn fixture() -> ControlTree {
        let mut tree = ControlTree::new();
        tree.insert(ControlNode::new(
            "track-0-fader",
            role("slider"),
            "轨道 0 推子",
        ))
        .expect("插入");
        tree.insert(
            ControlNode::new("clip-01J8Z-2-header", role("list-item"), "剪辑头")
                .with_bounds(Rect::new(40, 60, 120, 24))
                .with_parent("track-0-header"),
        )
        .expect("插入");
        tree.insert(
            ControlNode::new("mixer-vu-track-0", role("progress-indicator"), "VU 0")
                .with_bounds(Rect::new(120, 240, 8, 64))
                .as_dynamic(),
        )
        .expect("插入");
        tree.insert(
            ControlNode::new("transport-timecode", role("text"), "时间码")
                .with_bounds(Rect::new(8, 4, 96, 16))
                .as_dynamic(),
        )
        .expect("插入");
        tree
    }

    /// 判据 1: JSON 逐字节稳定，**与插入顺序无关**，且节点按语义 ID 升序。
    ///
    /// 注入验证 C（把节点顺序改成逆序 / 依赖插入顺序）会让本判据变红。
    #[test]
    fn json_is_byte_stable_regardless_of_insertion_order() {
        let forward = UiTree::from_runtime(&fixture());

        // 逆序插入构造同一棵树。
        let source = fixture();
        let mut reversed = ControlTree::new();
        for id in source.ids().collect::<Vec<_>>().into_iter().rev() {
            reversed
                .insert(source.find_by_id(id).expect("存在").clone())
                .expect("插入");
        }
        let backward = UiTree::from_runtime(&reversed);

        assert_eq!(forward, backward, "投影结果不得依赖插入顺序");
        assert_eq!(
            forward.to_json_pretty(),
            backward.to_json_pretty(),
            "逐字节相同"
        );
        assert!(forward.to_json_pretty().ends_with("}\n"));

        let ids = forward
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "节点必须按语义 ID 升序");
        assert_eq!(forward.count, 4, "夹具是 4 个节点");

        // 两次序列化必须逐字节相同（判据 ④ 的上半：稳定；下半在 service 里）。
        assert_eq!(forward.to_json_pretty(), forward.to_json_pretty());
        assert_eq!(forward.to_json(), forward.to_json());
    }

    /// 判据 2: `visible` 是**证据**而不是猜测 —— 运行时无几何时报 `null`，注册表恒 `null`。
    #[test]
    fn visible_is_null_when_there_is_no_geometry_evidence() {
        let runtime = UiTree::from_runtime(&fixture());
        assert_eq!(runtime.find("track-0-fader").expect("存在").visible, None);
        assert_eq!(
            runtime.find("mixer-vu-track-0").expect("存在").visible,
            Some(true)
        );

        let registry = UiTree::from_registry(&fixture());
        assert_eq!(registry.source, TreeSource::Registry);
        for node in &registry.nodes {
            assert_eq!(node.visible, None, "注册表给不出可见性正证据");
        }
        assert_eq!(
            registry.count, runtime.count,
            "两条来源的节点集合相同, 差别只在 `visible`/`source`"
        );
        // 任何投影都**不**输出 `false` —— 我们没有"确定不可见"的正证据。
        for tree in [&runtime, &registry] {
            assert!(tree.nodes.iter().all(|node| node.visible != Some(false)));
        }
    }

    /// 判据 3: 角色 / 标签 / 父 ID 原样过线，且 `maskable` 只在"动态区 **且** 有几何"时为真。
    #[test]
    fn role_label_parent_and_maskable_are_projected_faithfully() {
        let tree = UiTree::from_runtime(&fixture());
        let clip = tree.find("clip-01J8Z-2-header").expect("存在");
        assert_eq!(clip.role, "list-item");
        assert_eq!(clip.label, "剪辑头");
        assert_eq!(clip.parent.as_deref(), Some("track-0-header"));
        assert!(!clip.dynamic_region);
        assert!(!clip.maskable);

        let vu = tree.find("mixer-vu-track-0").expect("存在");
        assert!(vu.dynamic_region && vu.maskable);
        assert_eq!(vu.bounds, Some(Rect::new(120, 240, 8, 64)));

        // 反例: 登记成动态区但没有几何 ⇒ 不可遮罩, 且 `mask_rects` 必须**报错**。
        let mut broken = ControlTree::new();
        broken
            .insert(
                ControlNode::new("mixer-vu-track-1", role("progress-indicator"), "VU 1")
                    .as_dynamic(),
            )
            .expect("插入");
        let broken = UiTree::from_runtime(&broken);
        assert!(!broken.find("mixer-vu-track-1").expect("存在").maskable);
        assert_eq!(
            broken.mask_rects(),
            Err(ProjectionError::DynamicRegionWithoutBounds {
                id: "mixer-vu-track-1".to_owned()
            })
        );
    }

    /// 判据 4: **动态区矩形来自运行时包围盒**（`[UI-MCP-002]` §12.5），不是硬编码坐标。
    ///
    /// 注入验证 D（把矩形写成常量）会让本判据的第一条断言变红：矩形必须跟着包围盒走。
    #[test]
    fn dynamic_region_rects_come_from_the_runtime_bounds() {
        let tree = UiTree::from_runtime(&fixture());
        let rects = tree.mask_rects().expect("有几何的动态区");
        assert_eq!(
            rects,
            vec![
                ("mixer-vu-track-0", Rect::new(120, 240, 8, 64)),
                ("transport-timecode", Rect::new(8, 4, 96, 16)),
            ],
            "矩形 = 运行时包围盒, 且按语义 ID 升序"
        );
        assert_eq!(rects.len(), tree.dynamic_regions().count());

        // 把包围盒挪走 ⇒ 矩形必须跟着挪 (证明它来自树, 不是常量)。
        let mut moved = ControlTree::new();
        for node in fixture().iter() {
            let node = if node.id == "mixer-vu-track-0" {
                node.clone().with_bounds(Rect::new(777, 333, 8, 64))
            } else {
                node.clone()
            };
            moved.insert(node).expect("插入");
        }
        let moved = UiTree::from_runtime(&moved);
        assert_eq!(
            moved.mask_rects().expect("有几何")[0].1,
            Rect::new(777, 333, 8, 64),
            "矩形必须来自当前树里的包围盒"
        );

        // 与 `yeban-ui-test-port` 的遮罩口径对账: 同一棵树算出的矩形集合必须一致。
        assert_eq!(
            yeban_ui_test_port::mask::mask_rects_from_tree(&fixture()).expect("遮罩矩形"),
            tree.mask_rects()
                .expect("有几何")
                .into_iter()
                .map(|(_, rect)| rect)
                .collect::<Vec<_>>()
        );
    }

    /// 判据 5: `[UI-TEST-001]` 双向覆盖 —— 两个方向都报出来，且顺序稳定。
    #[test]
    fn coverage_reports_both_directions_deterministically() {
        let runtime = UiTree::from_runtime(&fixture());
        let registry_ids = vec![
            "track-0-fader".to_owned(),
            "tab-mixer-button".to_owned(),
            "arrangement-playhead".to_owned(),
        ];
        let coverage = Coverage::between(&runtime, &registry_ids);
        assert_eq!(
            coverage.unknown_at_runtime,
            [
                "clip-01J8Z-2-header",
                "mixer-vu-track-0",
                "transport-timecode"
            ]
        );
        assert_eq!(
            coverage.missing_at_runtime,
            ["arrangement-playhead", "tab-mixer-button"]
        );
        assert!(!coverage.is_complete());
        assert_eq!(coverage.runtime_count, 4);
        assert_eq!(coverage.registry_count, 3);

        // 输入顺序不得影响输出字节。
        let mut shuffled = registry_ids.clone();
        shuffled.reverse();
        assert_eq!(
            Coverage::between(&runtime, &shuffled),
            coverage,
            "覆盖结果必须与输入顺序无关"
        );

        let all_ids = runtime
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        assert!(Coverage::between(&runtime, &all_ids).is_complete());
    }

    /// 判据 6: 过滤（`prefix` / `dynamicOnly`）保持 ID 升序，且不改 `source`。
    #[test]
    fn filtering_keeps_id_order_and_source() {
        let tree = UiTree::from_runtime(&fixture());
        let tracks = tree.filtered(Some("track-"), false);
        assert_eq!(tracks.count, 1);
        assert_eq!(tracks.source, TreeSource::Runtime);
        assert_eq!(tracks.nodes[0].id, "track-0-fader");

        let dynamic = tree.filtered(None, true);
        assert_eq!(
            dynamic
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["mixer-vu-track-0", "transport-timecode"]
        );
        assert_eq!(dynamic.filtered(Some("mixer-"), true).count, 1);
        assert_eq!(dynamic.filtered(Some("nope-"), false).count, 0);
        assert_eq!(tree.filtered(None, false), tree);
    }
}
