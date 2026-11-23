# 契约面降级清单（T-FIX-142）

基线：`822ab14fb72b62819686197654f6550f483ddb9b`。本文件由脚本按源码静态分类生成，**不是**「行为已验证」的证明。

> 再验证调整（T-FIX-142 复验，尖端 f3f685e0b）：基线后 T-FIX-131/171 把 search_focus_key.rs、console_maximize.rs、undo_tree_focus_trap.rs 的用例恢复为真实行为断言，复验重贴时按尖端取 HEAD（不改名）；a11y_screen_reader.rs 一处计数由 2 订正为 3；以下数字均按尖端实测。

## 口径

- 「降级用例」= 函数体内带 `新 UI 重构…` 标记、且函数体是「读 `.slint`/`host.rs` 源码 + `contains` 断言」的 `#[test]`；
  这类用例守的是「声明/接线仍在源码里」，不守行为。命名统一改为 `contract_surface_*`，由 `tests/contract_surface_meta.rs` 强制。
- 「可恢复」= 已有真实界面构造路径（`build_live_ui_with` / `live::` / `ui.ui()`）的同一主题；恢复工作按票排期，不在 T-FIX-142 硬啃。

## 总量

- 降级用例总数：**713**（分布于 **135** 个测试文件；另 3 行计 0，见下表）
- 本地标记族分布（近似，按措辞首 60 字聚合）：
  - 356 处：`新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。`
  - 314 处：`新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。`
  - 17 处：`新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。`
  - 11 处：`新 UI 重构后旧面板的视觉元素不再进控件树; 同一条宿主接线仍住在契约面。`
  - 8 处：`新 UI 重构: 旧标尺带视觉元素不再进控件树; 核对同一条宿主接线的契约面。`
  - 3 处：`新 UI 重构后旧面板视觉元素不再进控件树; 同一条宿主接线仍住在契约面。`
  - 2 处：`新 UI 重构: 旧 status bar 的静态文本断言不再成立; 核对契约面与宿主接线仍在。`
  - 1 处：`新 UI 重构: 旧 undo-tree-modal/panel 不再进控件树; 核对视图态属性 + 宿主接线。`
  - 1 处：`新 UI 重构: 旧五区 ID 不再进控件树; 核对契约面与接线仍在。`

> 票面依据的是其中**一个**措辞族（`旧组件树的视觉/交互契约不再成立…`，75 文件 / 361 处）；实跑发现同族还有
> `旧组件树的视觉元素不再进控件树…`（≈314 处）等共 **9 种措辞**，合计 713 处。本单按「同一类降级」一并改名，
> 差额已在交付回执里向管理侧响亮报告。

## 逐文件（降级用例数）

| 文件 | 降级用例数 | 分类 | 建议 owner |
|---|---:|---|---|
| `live_ui_mcp.rs` | 64 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `roll_edit_tools.rs` | 40 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `cli_contract.rs` | 37 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `song_overview_panel.rs` | 27 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `pane_no_overlap.rs` | 18 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `arrangement_ruler_seek.rs` | 15 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `record_device_wiring.rs` | 14 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `theme_selection.rs` | 14 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `sfz_path_resolution.rs` | 13 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `dead_ui_controls_repro.rs` | 11 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `clip_duration.rs` | 10 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_26_labelled_chords_are_bound.rs` | 10 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `undo_wiring_ui.rs` | 10 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `clip_mute_toggle.rs` | 9 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `sample_library.rs` | 9 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_27_status_bar_tip_promises_are_bound.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_30_serve_transport_is_wired.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_31_serve_seek_is_wired.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_32_serve_parks_every_new_engine.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `delete_selected_clip.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `device_bypass_identity.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `device_clip_creation.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `mixer_keyboard_nudge.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `track_routing_crud.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `track_selection.rs` | 8 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_24_bpm_draft_survives_reprojection.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `device_rack_projection.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `file_picker_dialogs.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `instrument_install.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `nightfall_ui_wiring.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `round10_action_feedback.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `round11_action_feedback.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `track_reorder_and_navigation.rs` | 7 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `clip_ms_nudge.rs` | 6 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `piano_roll_humanize_quantize_ui.rs` | 6 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `round9_action_feedback.rs` | 6 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `send_strip_projection.rs` | 6 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_11_serve_header_gestures.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_13_roll_hit_scope.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_29_metronome_writes_the_document.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `device_rack_narrow_window.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `in_process_mcp_lock.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `instrument_probe_async.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `master_device_param_ui.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `master_device_ui.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `mcp_scene_launch.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `mixer_horizontal_scroll.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `mixer_keyboard_switch.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `production_meter_leg.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `record_lands_in_pools.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `rewind_sound.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `round7_interaction_feedback.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `round8_action_feedback.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `scene_launch_wiring.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `search_focus_key.rs` | 0 | 已恢复为真实行为断言（T-FIX-131） | T-FIX-131 已恢复，本单不改名 |
| `sidechain_dynamics_expansion.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `single_writer_session.rs` | 5 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `arrangement_vertical_scroll.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `asset_pool_panel.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `automation_recording_and_controls.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_23_count_in_writes_the_document.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `import_midi_losses_ui.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `mixer_fader_double_click.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `perf_draw_budget.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `round5_ui_wiring.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `round6_addressable_roles.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `sfz_ui_selector.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `tool_matrix_wiring.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `track_select_keyboard.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `undo_tree_honesty.rs` | 4 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_1_serve_write_surface.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_3_chord_injection.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `device_rack_two_segments.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `engine_error_banner.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `piano_roll_drag_scroll.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `track_instrument_switch.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `track_kinds_wiring.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `a11y_screen_reader.rs` | 3 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记；复验订正 2→3） |
| `ai_drawer_key_ui.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `arrangement_section_seek.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `compact_follows_window_width.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_21_modal_mask_swallows_clicks.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_8_serve_save_gesture.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `export_dialog_button_focus.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `export_dialog_key_capture.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `export_midi_ui.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `export_wav_browse.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `fabricated_state_honesty.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `fps_gpu_path.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `gui_export_clipping.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `gui_export_limiter.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `mixer_toggle_labels.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `new_project_registration.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `readout_accessible_values.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `record_places_clip_on_track.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `send_strip_no_overlap.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `status_bar_accessible_values.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `status_bar_honesty.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `transport_timecode_tick.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `ui_module_actions.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `ui_module_actions_no_port.rs` | 2 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `automation_labels_follow_playhead.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `clip_drag.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `clip_selection.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `console_maximize.rs` | 0 | 已恢复为真实行为断言（T-FIX-171） | T-FIX-171 已恢复，本单不改名 |
| `console_tab_keys.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `d_ui_28_export_length_seeds_from_audible_end.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `export_dialog_modal_closure.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `gui_clipping_warning.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `gui_save_writes_live_session.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `import_audio_entry.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `master_mute_solo.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `metronome_switch.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `module_drawer_receipt.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `new_project_ui.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `open_project_ui.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `playhead_follows_transport.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `production_reprojection.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `production_save_ui.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `record_key_ui.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `revert_button.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `roll_scope.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `scene_color_swatch.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `section_color_swatch.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `session_back_to_arrangement.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `sidebar_category.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `sidebar_item.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `sidebar_search.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `splitter_drag.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `ui_adr0057a_test_session.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `ui_bpm_field.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `ui_export_dialog.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `ui_macro_panel.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `ui_module_actions_async.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `undo_tree_close_button.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `undo_tree_focus_trap.rs` | 0 | 已恢复为真实行为断言（T-FIX-131） | T-FIX-131 已恢复，本单不改名；元判据显式跳过 |
| `undo_tree_navigation.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |
| `velocity_bar_a11y_value.rs` | 1 | 纯结构断言（源码 contains） | T-FIX-142（本单：已改名+登记） |

## 三个实例的处置（T-FIX-142 第 4 条）

| 实例 | 位置 | 处置 |
|---|---|---|
| Ctrl/Cmd+F 聚焦 | `tests/search_focus_key.rs`（用例 `cmd_ctrl_f_moves_…`） | 未改名——T-FIX-131 已恢复为真实行为断言（本单不改名） |
| 导出弹窗按钮取焦 | `tests/export_dialog_button_focus.rs:171-172` | 同上；字段名断言未动，绑定/取焦归 T-FIX-131 |
| automation 死文件守护 | `src/automation.rs:1527`、`:1751` | 已改名 `contract_surface_*` 并注明「守护对象是从未 import 的死文件」；死文件与注册表归 T-FIX-144 |

## 未做

- 713 条中「可恢复」的具体清单未逐条列出：本单只给出分类口径与文件级分布，逐条恢复按主题拆票（焦点 → 131，死 slint/a11y → 144，属性 → 143）。
- `src/**/*.rs` 内的单元测试不在本清单统计范围（`src/automation.rs` 的两条已单独标记）。
