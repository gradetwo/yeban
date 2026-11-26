# 死 .slint 清单与文档替换表（T-FIX-230）

本清单由 T-FIX-230（issue `ISS-MWTZ0R8T-BE7904`）建立，用来阻止「照着文档去改一个不渲染的文件」。

判定口径：以 `crates/yeban-app/ui/app.slint` 的 import 可达性为准（组件被 import 且被实例化才算活树）。

## 一、已不在活树中的文件（5 个）

| 死文件 | 现状 | 活树对应文件 |
|---|---|---|
| `ui/transport.slint` | 从未被 import | `ui/transport_bar.slint` |
| `ui/sidebar.slint` | 从未被 import | `ui/left_panel.slint` |
| `ui/workspace/arrangement_view.slint` | 从未被 import | `ui/arrangement.slint` |
| `ui/module_actions.slint` | 从未被 import | **无**（该文件已不在活树中，改动无效） |
| `ui/controls/knob.slint` | 未被任何活文件 import | **无**（该文件已不在活树中，改动无效） |

> `ui/workspace/session_view.slint` **不在死文件清单内**：T-FIX-223（`66b4eaa11`）已由 `app.slint:14`
> `import { SessionView } from "workspace/session_view.slint";` 正式接回活树，引用它的文档不算错。

## 二、替换表（只用于文档与注释；⛔ 不要在死文件本体上改代码）

```
ui/transport.slint                  → ui/transport_bar.slint
ui/sidebar.slint                    → ui/left_panel.slint
ui/workspace/arrangement_view.slint → ui/arrangement.slint
ui/module_actions.slint             → （无对应活文件）改动无效
ui/controls/knob.slint              → （无对应活文件）改动无效
```

## 三、顺带查到的陈旧引用（超出 6 个死文件，另行登记）

`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 的目录树还列了两个**根本不存在**的文件：

- `ui/pianoroll.slint`（实际是 `ui/console/piano_roll.slint`）
- `ui/mixer.slint`（实际是 `ui/console/mixer_console.slint`）

本轮已在这两行就地更正并加注。

## 四、判据口径

- 活架构描述（目录树、模块清单、"触点/落点"这类**今天要照着改**的段落）必须用活文件名；
- ADR 历史正文里的旧文件名**保留原样**（历史裁决不重写），需要时在文首加勘误行（例：`ADR-0051`）；
- `docs/DEVELOPMENT_LEDGER.md` 是历史台账，按工单口径不纳入本判据。
