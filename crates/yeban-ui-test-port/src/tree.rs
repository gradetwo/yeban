//! 语义控件树的数据模型与 JSON 序列化 —— `[UI-TEST-001]` / `[UI-MCP-001]` / `[UI-MCP-002]` 的数据底座。
//!
//! 规范来源 (Normative)：
//! - `[UI-TEST-001]` UI/UX §12.2「稳定语义元素寻址」：所有关键节点必须声明稳定语义 ID
//!   （`track-{i}-fader` / `note-{ulid}-rect` / `clip-{ulid}-header` / `tab-{name}-button`），
//!   **严禁绝对坐标寻址**；
//! - `[UI-MCP-001]` §12.3：`ReadOnly` 权限就是"控件树结构检索 + 响应式属性读取"；
//! - `[UI-MCP-002]` §12.5：比对前必须按**元素树元数据**取得高频刷新组件的矩形包围盒并置黑。
//!
//! ## 依赖方向（架构硬约束）
//!
//! 本 crate 由 `yeban-app` 通过 feature **单向**依赖（架构规范 §8 的 crate 清单）。
//! 因此本文件**绝不能**反向依赖 `yeban-app`：节点模型、语义 ID 格式校验、角色取值
//! 全部自持。`yeban-app` 侧的 `ElementRegistry` → 本模型的适配器写在
//! `crates/yeban-app/src/test_port_adapter.rs`（app 侧）。
//!
//! ## 为什么用 `BTreeMap`（红线 4 的精神）
//!
//! `AGENTS.md` §2 红线 4 禁止在持久化 AST 里用 `HashMap`/`HashSet`。控件树不是持久化 AST，
//! 但它同样要求"同一份输入、跨进程给出同一份字节"：CI 的 JSON 断言与 Golden 图 diff 都直接
//! 消费它。用 `HashMap` 会让键序随机 ⇒ 快照抖动。因此集合一律 `BTreeMap`，并把
//! "插入顺序不影响输出字节" 写成判据。
//!
//! ## 与 `i-slint-backend-testing` 的分工
//!
//! 本模型是**纯数据**：它既可以由 app 侧的静态注册表填（无窗口也能跑），也可以由有窗口实例的
//! `ElementHandle` 遍历填（`crate::inspect`）。两条来源产出同一种 JSON，因此断言脚本只写一遍。
//! Testing Backend **不渲染像素**，所以它只出现在本文件的数据来源侧，绝不出现在截图侧
//! （见 `crate::render` 与 `[MUST-GATE-015]`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::image::Rect;

/// Slint `AccessibleRole` 枚举的**全部**合法取值（kebab-case），共 30 个。
///
/// 出处（逐条核对过，不是凭记忆）：
/// - 语言参考「Built-in Enums → AccessibleRole」<https://docs.slint.dev/latest/docs/slint/reference/property-types/builtin-enums/>
/// - 上游源码 `i-slint-common-1.18.1/enums.rs:466` 的 `pub enum AccessibleRole`
///   （`for_each_enums!` 展开），含 8 个 landmark 角色与 `search`
///   <https://docs.rs/crate/i-slint-common/1.18.1/source/src/enums.rs>
///
/// 把它放在本 crate 而不是复用 `yeban-app::elements::ElementKind`，是为了守住依赖方向；
/// 两侧取值的一致性由 app 侧适配器的判据钉住（见 `test_port_adapter.rs`）。
pub const KNOWN_ROLES: [&str; 30] = [
    "banner",
    "button",
    "checkbox",
    "combobox",
    "complementary",
    "content-info",
    "form",
    "groupbox",
    "image",
    "list",
    "list-item",
    "main",
    "navigation",
    "none",
    "progress-indicator",
    "radio-button",
    "radio-group",
    "region",
    "search",
    "slider",
    "spinbox",
    "switch",
    "tab",
    "tab-list",
    "tab-panel",
    "table",
    "text",
    "text-input",
    "tree",
    "window-title-bar",
];

/// 无障碍角色（`accessible-role` 的字面取值）。
///
/// 用带校验的 newtype 而不是 `enum`：`AccessibleRole` 在上游是 `#[non_exhaustive]`，
/// 硬编码一个 30 分支的 `enum` 只会在上游加角色时变成"我们需要发版"；
/// 而 newtype + [`Role::parse`] 既保留了"拼错就报错"，又不拒绝未来新增的合法角色
/// （新增只需往 [`KNOWN_ROLES`] 追加一行，且会被本文件的判据发现）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Role(String);

impl Role {
    /// 校验并构造。取值必须是 [`KNOWN_ROLES`] 之一。
    pub fn parse(raw: &str) -> Result<Self, TreeError> {
        if KNOWN_ROLES.contains(&raw) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(TreeError::UnknownRole {
                role: raw.to_owned(),
            })
        }
    }

    /// 字面取值。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Role {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 控件树里的一个节点。
///
/// 字段顺序即 JSON 键序（`serde` 按声明顺序输出），因此**不要**为了"看起来顺眼"重排字段：
/// 那会改掉 CI 断言的字节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlNode {
    /// 语义 ID（`[UI-TEST-001]` §12.2 的四个族命名）。格式由 [`is_well_formed_id`] 校验。
    pub id: String,
    /// `accessible-role`。
    pub role: Role,
    /// `accessible-label`（无人可读标签时为空串，而不是 `null` —— 空串在 JSON 断言里更省心）。
    pub label: String,
    /// 几何包围盒（物理/逻辑像素由调用方统一；`[UI-MCP-002]` 的遮罩直接消费它）。
    ///
    /// 无窗口实例时（纯注册表路径）为 `None`：静态清单证明不了几何，不编造数字。
    pub bounds: Option<Rect>,
    /// `[UI-MCP-002]` 是否属于每帧跳变的高频刷新区（VU 表 / 走带光标 / RTA / 时间码）。
    /// 为 `true` 时截图比对**必须**先置黑。
    pub dynamic_region: bool,
    /// 父节点的语义 ID（§12.2 的"组件层级选择器"需要它）。根节点为 `None`。
    pub parent: Option<String>,
}

impl ControlNode {
    /// 构造一个无几何、非动态区的节点（静态注册表路径的常见形态）。
    #[must_use]
    pub fn new(id: impl Into<String>, role: Role, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role,
            label: label.into(),
            bounds: None,
            dynamic_region: false,
            parent: None,
        }
    }

    /// 标注几何包围盒。
    #[must_use]
    pub fn with_bounds(mut self, bounds: Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// 标注为动态刷新区（截图前必须置黑）。
    #[must_use]
    pub fn as_dynamic(mut self) -> Self {
        self.dynamic_region = true;
        self
    }

    /// 标注父节点。
    #[must_use]
    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    /// 该节点在截图比对时是否需要被遮罩：**动态刷新区 + 有包围盒**。
    ///
    /// 动态区却没有包围盒 = 信息不足，不能假装遮过了 —— 这属于 [`TreeError::DynamicRegionWithoutBounds`]，
    /// 由 [`ControlTree::mask_rects`] 在比对前显式报错。
    #[must_use]
    pub const fn needs_masking(&self) -> bool {
        self.dynamic_region && self.bounds.is_some()
    }
}

/// 控件树的构造 / 反序列化错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    /// 语义 ID 格式不合法（`[UI-TEST-001]` §12.2 的命名约定被破坏）。
    MalformedId {
        /// 出错的 ID。
        id: String,
    },
    /// 同一个语义 ID 被登记了两次 —— ID 必须唯一，否则自动化脚本会寻址到不确定的节点。
    DuplicateId {
        /// 重复的 ID。
        id: String,
    },
    /// 角色取值不在 [`KNOWN_ROLES`] 里。
    UnknownRole {
        /// 出错的取值。
        role: String,
    },
    /// 动态刷新区没有包围盒 —— `[UI-MCP-002]` 无法执行遮罩。
    DynamicRegionWithoutBounds {
        /// 出错的 ID。
        id: String,
    },
    /// JSON 解析或结构校验失败。
    Json {
        /// 底层错误消息。
        message: String,
    },
}

impl core::fmt::Display for TreeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MalformedId { id } => write!(
                f,
                "语义 ID 格式不合法: `{id}` (约定: 由 ASCII 字母数字分段、单个 `-` 连接; \
                 见 [UI-TEST-001] UI/UX §12.2)"
            ),
            Self::DuplicateId { id } => write!(f, "语义 ID 重复: `{id}` (ID 必须唯一)"),
            Self::UnknownRole { role } => {
                write!(f, "未知的 accessible-role: `{role}` (不在 KNOWN_ROLES 中)")
            }
            Self::DynamicRegionWithoutBounds { id } => write!(
                f,
                "动态刷新区 `{id}` 没有包围盒 —— [UI-MCP-002] 的遮罩无法执行, 拒绝静默放过"
            ),
            Self::Json { message } => write!(f, "控件树 JSON 错误: {message}"),
        }
    }
}

impl core::error::Error for TreeError {}

/// 语义控件树：`BTreeMap<语义 ID, 节点>`。
///
/// 顺序恒为 ID 升序（`BTreeMap`），与插入顺序无关 —— 这是 JSON 字节稳定的前提。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlTree {
    nodes: BTreeMap<String, ControlNode>,
}

impl ControlTree {
    /// 空树。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入节点。ID 格式不合法或重复一律返回 `Err` —— 静默覆盖会让"UI 改了但断言还是绿的"。
    pub fn insert(&mut self, node: ControlNode) -> Result<(), TreeError> {
        if !is_well_formed_id(&node.id) {
            return Err(TreeError::MalformedId { id: node.id });
        }
        // 角色也要在这里校验：`Role` 的 `Deserialize` 是**透明**的，
        // 反序列化可以绕过 `Role::parse` 造出 `"buton"` 这种非法角色。
        // 把校验放在**唯一的入口**上，`from_json` 与手工构造就不可能有两个口径。
        if Role::parse(node.role.as_str()).is_err() {
            return Err(TreeError::UnknownRole {
                role: node.role.as_str().to_owned(),
            });
        }
        if self.nodes.contains_key(&node.id) {
            return Err(TreeError::DuplicateId { id: node.id });
        }
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }

    /// 按 ID 精确查找。
    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&ControlNode> {
        self.nodes.get(id)
    }

    /// 是否登记了该 ID。
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }

    /// 节点总数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 树是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 按 ID 升序遍历（顺序稳定）。
    pub fn iter(&self) -> impl Iterator<Item = &ControlNode> + '_ {
        self.nodes.values()
    }

    /// 按 ID 升序返回全部语义 ID。
    pub fn ids(&self) -> impl Iterator<Item = &str> + '_ {
        self.nodes.keys().map(String::as_str)
    }

    /// `[UI-TEST-001]` §12.2 的族查询：ID 以 `prefix` 开头的全部节点。
    pub fn with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = &'a ControlNode> + 'a {
        self.nodes
            .values()
            .filter(move |node| node.id.starts_with(prefix))
    }

    /// `[UI-MCP-002]` 全部高频刷新区。
    pub fn dynamic_regions(&self) -> impl Iterator<Item = &ControlNode> + '_ {
        self.nodes.values().filter(|node| node.dynamic_region)
    }

    /// 某个父节点下的直接子节点（按 ID 升序）。
    pub fn children_of<'a>(
        &'a self,
        parent: &'a str,
    ) -> impl Iterator<Item = &'a ControlNode> + 'a {
        self.nodes
            .values()
            .filter(move |node| node.parent.as_deref() == Some(parent))
    }

    /// `[UI-MCP-002]` 截图比对前必须置黑的矩形清单。
    ///
    /// 与 [`Self::dynamic_regions`] 的区别：这里**拒绝**"登记成动态区却没有包围盒"的节点。
    /// 那种情况下遮罩无从执行，静默跳过就等于把假阳性放进 CI。
    pub fn mask_rects(&self) -> Result<Vec<Rect>, TreeError> {
        let mut rects = Vec::new();
        for node in self.dynamic_regions() {
            match node.bounds {
                Some(rect) => rects.push(rect),
                None => {
                    return Err(TreeError::DynamicRegionWithoutBounds {
                        id: node.id.clone(),
                    });
                }
            }
        }
        Ok(rects)
    }

    /// 稳定 JSON 转储：键序 = ID 升序（`BTreeMap` + `serde_json` 保序），末尾带换行。
    ///
    /// 不 panic：`serde_json` 序列化 `BTreeMap<String, ControlNode>` 不可能失败
    /// （键全是字符串、无 NaN），但生产代码里不留 `unwrap`；失败时退化为 `{}`，
    /// 并被 `json_is_stable_across_insertion_orders` 一类的判据在 CI 上立刻抓住。
    #[must_use]
    pub fn dump_json(&self) -> String {
        let mut out =
            serde_json::to_string_pretty(&self.nodes).unwrap_or_else(|_| String::from("{}"));
        out.push('\n');
        out
    }

    /// 从 JSON 反序列化并做与 [`Self::insert`] **同一套**结构校验。
    ///
    /// CI 的断言脚本可能读 JSON 再喂回来；如果反序列化路径少校验一次，
    /// 就会出现"构造时拒绝、读回来却接受"的漏洞。
    pub fn from_json(json: &str) -> Result<Self, TreeError> {
        let raw: BTreeMap<String, ControlNode> =
            serde_json::from_str(json).map_err(|err| TreeError::Json {
                message: err.to_string(),
            })?;
        let mut tree = Self::new();
        for (key, node) in raw {
            if key != node.id {
                return Err(TreeError::Json {
                    message: format!("JSON 键 `{key}` 与节点 id `{}` 不一致", node.id),
                });
            }
            tree.insert(node)?;
        }
        Ok(tree)
    }

    /// 从**静态注册表**继承"哪些是高频刷新区"（`[UI-MCP-002]`）。
    ///
    /// 有窗口实例时（`crate::inspect`）能读到真实的 `accessible-id` 与几何，但**读不到**
    /// "这个节点每帧跳变"这件事 —— 那属于 app 的业务知识（`ElementRegistry::dynamic_region`）。
    /// 反过来，静态注册表没有几何。两边的信息必须**合并**才能得到可用的遮罩矩形：
    /// 几何来自运行时，动态标记来自注册表。
    ///
    /// 只对**两边都存在的 ID** 生效；注册表里有而运行时没有的 ID 不会凭空造节点
    /// （那是 [`Self::coverage_against`] 要报告的事，不是这里要掩盖的事）。
    ///
    /// 返回被标注为动态区的节点数。
    pub fn merge_dynamic_flags_from(&mut self, registry: &Self) -> usize {
        let mut marked = 0;
        for (id, node) in &mut self.nodes {
            let Some(source) = registry.nodes.get(id) else {
                continue;
            };
            if source.dynamic_region {
                node.dynamic_region = true;
                marked += 1;
            }
            if node.label.is_empty() && !source.label.is_empty() {
                node.label.clone_from(&source.label);
            }
        }
        marked
    }

    /// 与静态注册表交叉核对（`[UI-TEST-001]` 的"UI 与注册表双向覆盖"）。
    pub fn coverage_against(&self, registry: &Self) -> Coverage {
        let mut missing_at_runtime = Vec::new();
        for id in registry.nodes.keys() {
            if !self.nodes.contains_key(id) {
                missing_at_runtime.push(id.clone());
            }
        }
        let mut unknown_at_runtime = Vec::new();
        for id in self.nodes.keys() {
            if !registry.nodes.contains_key(id) {
                unknown_at_runtime.push(id.clone());
            }
        }
        Coverage {
            missing_at_runtime,
            unknown_at_runtime,
        }
    }
}

/// 运行时控件树与静态注册表的差异（两个方向都必须为空）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// 注册表登记了、但运行时树里**找不到**的 ID。含义：UI 声明了却不在无障碍树里
    /// （元素没被创建 / 不可见 / `accessible-id` 拼错）。
    pub missing_at_runtime: Vec<String>,
    /// 运行时树里有、但注册表**没登记**的 ID。含义：UI 加了 ID 却忘了登记到注册表。
    pub unknown_at_runtime: Vec<String>,
}

impl Coverage {
    /// 两个方向都为空才算闭合。
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing_at_runtime.is_empty() && self.unknown_at_runtime.is_empty()
    }
}

impl core::fmt::Display for Coverage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "注册表有而运行时缺 {} 条 {:?}; 运行时有而注册表缺 {} 条 {:?}",
            self.missing_at_runtime.len(),
            self.missing_at_runtime,
            self.unknown_at_runtime.len(),
            self.unknown_at_runtime
        )
    }
}

/// 校验语义 ID 的**形状**：ASCII 字母数字分段，用单个 `-` 连接。
///
/// 允许大写是因为 `note-{ulid}-rect` / `clip-{ulid}-header` 里的 ULID 是 Crockford Base32
/// （规范要求大小写不敏感，本项目统一写大写）。
/// 拒绝：空串、前导/尾随 `-`、连续 `--`、空格、下划线、任何非 ASCII 字符。
///
/// 与 `yeban-app::elements::is_well_formed_id` 是**同一条规则的两份实现**（依赖方向不允许
/// 复用）；两侧一致性由 app 侧适配器的判据 `id_format_rule_matches_yeban_app` 逐条对账。
#[must_use]
pub fn is_well_formed_id(id: &str) -> bool {
    if id.is_empty() || id.starts_with('-') || id.ends_with('-') || id.contains("--") {
        return false;
    }
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, role: &str) -> ControlNode {
        ControlNode::new(id, Role::parse(role).expect("测试角色必须是合法取值"), id)
    }

    /// 判据 1: **JSON 字节与插入顺序无关**（红线 4 的精神 + CI 断言的前提）。
    ///
    /// 变异测试：把 `ControlTree.nodes` 从 `BTreeMap` 换成 `HashMap`，本判据变红。
    #[test]
    fn json_is_stable_across_insertion_orders() {
        let ids = [
            "track-0-fader",
            "note-01J8Z-1-rect",
            "clip-01J8Z-2-header",
            "tab-mixer-button",
        ];

        let mut forward = ControlTree::new();
        for id in ids {
            forward.insert(node(id, "list-item")).expect("插入应当成功");
        }
        let mut backward = ControlTree::new();
        for id in ids.iter().rev() {
            backward
                .insert(node(id, "list-item"))
                .expect("插入应当成功");
        }

        assert_eq!(forward.dump_json(), backward.dump_json());
        assert_eq!(
            forward.ids().collect::<Vec<_>>(),
            vec![
                "clip-01J8Z-2-header",
                "note-01J8Z-1-rect",
                "tab-mixer-button",
                "track-0-fader"
            ],
            "遍历顺序必须是 ID 升序"
        );
        assert!(forward.dump_json().ends_with("}\n"));
    }

    /// 判据 2: 重复 ID 与非法 ID 必须被**拒绝**，不能被静默覆盖/接受。
    #[test]
    fn duplicate_and_malformed_ids_are_rejected() {
        let mut tree = ControlTree::new();
        tree.insert(node("track-0-fader", "slider"))
            .expect("首次插入应当成功");
        assert_eq!(
            tree.insert(node("track-0-fader", "button")),
            Err(TreeError::DuplicateId {
                id: "track-0-fader".to_owned()
            })
        );
        assert_eq!(tree.len(), 1, "重复插入不得改变树");

        for bad in [
            "",
            "-x",
            "x-",
            "a--b",
            "track_0_fader",
            "track 0 fader",
            "音符-0",
        ] {
            assert_eq!(
                tree.insert(node(bad, "button")),
                Err(TreeError::MalformedId { id: bad.to_owned() }),
                "非法 ID `{bad}` 必须被拒绝"
            );
        }
        assert!(is_well_formed_id("note-01J8ZQ9K2M-rect"));
    }

    /// 判据 3: 角色取值必须来自 Slint 的 `AccessibleRole` 全集（拼错即报错）。
    #[test]
    fn roles_are_validated_against_slint_accessible_role() {
        assert_eq!(
            Role::parse("content-info").expect("合法角色").as_str(),
            "content-info"
        );
        assert!(Role::parse("progress-indicator").is_ok());
        assert!(Role::parse("window-title-bar").is_ok());
        assert_eq!(
            Role::parse("buton"),
            Err(TreeError::UnknownRole {
                role: "buton".to_owned()
            })
        );
        assert_eq!(
            Role::parse("ContentInfo"),
            Err(TreeError::UnknownRole {
                role: "ContentInfo".to_owned()
            }),
            "角色必须是小写 kebab-case, 不接受 Rust 变体名"
        );

        // 角色清单本身：排序、去重、无空串 —— 防止追加时手滑。
        let mut sorted = KNOWN_ROLES;
        sorted.sort_unstable();
        assert_eq!(sorted, KNOWN_ROLES, "KNOWN_ROLES 必须已按字典序排列");
        assert_eq!(
            KNOWN_ROLES.len(),
            30,
            "Slint 1.18.1 的 AccessibleRole 共 30 个取值"
        );
        let unique: std::collections::BTreeSet<&str> = KNOWN_ROLES.into_iter().collect();
        assert_eq!(unique.len(), KNOWN_ROLES.len(), "KNOWN_ROLES 不得有重复项");
    }

    /// 判据 4: JSON 往返必须逐字节稳定 **且** 键与节点 id 一致（反序列化不能绕过校验）。
    #[test]
    fn json_round_trip_is_lossless_and_validated() {
        let mut tree = ControlTree::new();
        tree.insert(
            node("mixer-vu-track-0", "progress-indicator")
                .with_bounds(Rect::new(120, 240, 8, 64))
                .as_dynamic(),
        )
        .expect("插入应当成功");
        tree.insert(node("track-0-fader", "slider").with_bounds(Rect::new(8, 100, 24, 160)))
            .expect("插入应当成功");

        let json = tree.dump_json();
        let parsed = ControlTree::from_json(&json).expect("往返应当成功");
        assert_eq!(parsed, tree);
        assert_eq!(parsed.dump_json(), json, "往返后的字节必须完全相同");

        // 键与 id 不一致必须报错（手工改坏 JSON：只改**键**，不改节点里的 id）。
        let tampered = json.replace("\"track-0-fader\":", "\"track-9-fader\":");
        assert_ne!(tampered, json, "变异必须真的改到了字节");
        assert!(matches!(
            ControlTree::from_json(&tampered),
            Err(TreeError::Json { .. })
        ));

        // 非法 ID 从 JSON 进来同样被拒绝。
        let bad = json.replace("track-0-fader", "track_0_fader");
        assert!(matches!(
            ControlTree::from_json(&bad),
            Err(TreeError::MalformedId { .. }) | Err(TreeError::Json { .. })
        ));

        // 非法角色从 JSON 进来同样被拒绝 —— `Role` 的 Deserialize 是透明的,
        // 这条判据证明反序列化**没有**绕过校验（把 insert 里的角色校验删掉即变红）。
        let bad_role = json.replace("\"slider\"", "\"buton\"");
        assert_ne!(bad_role, json, "变异必须真的改到了字节");
        assert_eq!(
            ControlTree::from_json(&bad_role),
            Err(TreeError::UnknownRole {
                role: "buton".to_owned()
            })
        );
    }

    /// 判据 5: `[UI-MCP-002]` —— 动态区必须能产出遮罩矩形；"动态区没包围盒"必须报错而不是放过。
    #[test]
    fn mask_rects_require_bounds_on_dynamic_regions() {
        let mut tree = ControlTree::new();
        tree.insert(
            node("transport-playhead", "image")
                .with_bounds(Rect::new(300, 0, 2, 600))
                .as_dynamic(),
        )
        .expect("插入应当成功");
        tree.insert(node("track-0-fader", "slider"))
            .expect("插入应当成功");

        assert_eq!(tree.mask_rects(), Ok(vec![Rect::new(300, 0, 2, 600)]));
        assert_eq!(tree.dynamic_regions().count(), 1);
        assert_eq!(tree.with_prefix("transport-").count(), 1);

        let mut broken = ControlTree::new();
        broken
            .insert(node("mixer-vu-track-1", "progress-indicator").as_dynamic())
            .expect("插入应当成功");
        assert_eq!(
            broken.mask_rects(),
            Err(TreeError::DynamicRegionWithoutBounds {
                id: "mixer-vu-track-1".to_owned()
            }),
            "动态区缺包围盒必须显式报错"
        );
    }

    /// 判据 6: **双向覆盖**（`[UI-TEST-001]`）—— 注册表与运行时控件树的差异必须两个方向都报出来。
    ///
    /// 这正是 `line/ui-shell` 留在 notes 里的缺口（"`accessible-id` 在重复元素上的唯一性没有实测"
    /// 之外，还有"没有窗口实例就没法交叉核对"）：现在由本模块的 `coverage_against` 提供判据形状，
    /// 有窗口实例的填充由 `crate::inspect` 完成。
    #[test]
    fn coverage_reports_both_directions() {
        let mut registry = ControlTree::new();
        registry
            .insert(node("track-0-fader", "slider"))
            .expect("插入应当成功");
        registry
            .insert(node("tab-mixer-button", "tab"))
            .expect("插入应当成功");

        let mut runtime = ControlTree::new();
        runtime
            .insert(node("track-0-fader", "slider"))
            .expect("插入应当成功");
        runtime
            .insert(node("track-1-fader", "slider"))
            .expect("插入应当成功");

        let coverage = runtime.coverage_against(&registry);
        assert_eq!(coverage.missing_at_runtime, ["tab-mixer-button"]);
        assert_eq!(coverage.unknown_at_runtime, ["track-1-fader"]);
        assert!(!coverage.is_complete());
        assert!(coverage.to_string().contains("tab-mixer-button"));

        let same = registry.coverage_against(&registry);
        assert!(same.is_complete());
    }

    /// 判据 7: 动态标记必须**从注册表流向运行时树**，且只对两边都存在的 ID 生效。
    ///
    /// 变异测试：把 `merge_dynamic_flags_from` 改成空实现，`[UI-MCP-002]` 的遮罩在运行时树上
    /// 会永远拿不到矩形 —— 本判据变红。
    #[test]
    fn dynamic_flags_flow_from_registry_into_the_runtime_tree() {
        let mut registry = ControlTree::new();
        registry
            .insert(
                node("mixer-vu-track-0", "progress-indicator")
                    .with_bounds(Rect::new(0, 0, 4, 8))
                    .as_dynamic(),
            )
            .expect("插入应当成功");
        registry
            .insert(node("track-0-fader", "slider"))
            .expect("插入应当成功");
        registry
            .insert(node("tab-mixer-button", "tab"))
            .expect("插入应当成功");

        // 运行时树：有几何，但不知道谁在跳变；且没有 tab 节点。
        let mut runtime = ControlTree::new();
        runtime
            .insert(
                node("mixer-vu-track-0", "progress-indicator").with_bounds(Rect::new(10, 20, 4, 8)),
            )
            .expect("插入应当成功");
        runtime
            .insert(node("track-0-fader", "slider").with_bounds(Rect::new(1, 2, 3, 4)))
            .expect("插入应当成功");

        assert!(runtime.mask_rects().expect("无动态区时应为空").is_empty());
        assert_eq!(runtime.merge_dynamic_flags_from(&registry), 1);
        assert_eq!(runtime.mask_rects(), Ok(vec![Rect::new(10, 20, 4, 8)]));
        assert!(
            !runtime
                .find_by_id("track-0-fader")
                .expect("存在")
                .dynamic_region
        );
        assert_eq!(runtime.len(), 2, "合并不新增/删除节点");
    }
}
