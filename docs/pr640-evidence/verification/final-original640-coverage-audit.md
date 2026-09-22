# 原 PR #640 最终拆分内容覆盖审计

结论：没有发现原 PR 必要文件或关键 hunk 遗漏。原 148 个修改文件全部归属于七个最终 PR。这里的“一对一”是每项功能修改均有承接，不是七个 diff 简单拼接后与旧基线整棵树相等。

## 可复核的全树证明

以原 base `373edf56`、原 HEAD `ad0df227` 和新 main `2793b2407` 做三方虚拟合并，得到原 PR 在当前 main 上的预期树 `93fb606f528ec918a4ba2c458918955f138ba8e4`，无文本冲突。它与最终组合树 `6fe6dd8f43edca8585379cd23b957057d902f0f1` 的**全树差异只有 8 个文件**：6 个文本兼容修正文件，以及 2 个后补网络测试文件；没有其他差异。

这比逐行是否出现的检索更强：它同时核对新增、删除、顺序和上下文，没有把不同 main 上的正常改动误当遗漏。旧 `partition-original-audit.json` 的 148/0 mismatch、`partition-audit.json` 的 150/0 mismatch 与本轮独立全树核对一致。

- 原 148 文件中，137 个甚至与原 ad0 的整个文件字节相同。
- 按当前 main 对齐后，142 个原文件完全等于三方合并结果；其余 6 个原文件均属于文本兼容修正。
- 最终 PR 的增量文件集合为原 148 + 网络后补 2 = 150。main 从 72413280 到 2793b2407 改 49 文件；从原 base373edf56 起实际改 141 文件。两种基线不可混称。

## 六文件兼容改写为什么不是遗漏

| 文件 | 保留的必要语义与变更 |
|---|---|
| moli-parser/src/session.rs | 去掉原 synthetic doctype，明确 NoQuirks；保持共享 plaintext tokenizer、PRE 和原文字面量语义，兼容 main 的无 doctype 契约。 |
| moli-parser/src/html.rs | 新增分块 CR/LF/NUL、字面脚本、无 doctype/NoQuirks 测试，原测试保留。 |
| moli-renderer-v8/src/dom_parser.rs | 用已有共享 text parser 取代 main 的 text/plain HTML 转义包装，覆盖 JSON/JS；显式 MIME 优先于扩展名；公开 scripting API 可用于 release。删除的是已经无调用的重复包装 helper。 |
| child_documents/commit.rs | 删除 main 后加的 text/plain 转义包装分支，将原始解码文字交给新 text parser，避免双包装；原 PR 的 MIME 参数传递仍在。 |
| child_documents/live_parser.rs | 删除 main 后加的 text/plain 完成后强制 NoQuirks；共享 text parser 已统一负责初始化，无需再次修补。原 finite-live-text parser 选择分支仍在。 |
| moli-protocol-server/src/protocol_server/tests/classic.rs | 原 MIME/main/iframe 测试保留，加入扩展名冲突及 DOM 结构断言。预期继续 strip 一次 BOM，并增加正确 CR/NUL 归一化；没有删掉 BOM 或伪脚本负对照。 |

原新增的非空行中仅两行未逐字保留：人工 doctype 那一行，以及 MIME 测试的单行 BOM 表达式。两者均已在上表解释；这不是仅凭行检索得出的完整性结论，全树对齐才是主证据。

## 网络补充及 main 保留

后补 `moli-protocol/src/domains/network/tests/mod.rs` 注册和 `redirect_wire.rs` 共 191 行，在真实 HTTP POST→302→307 场景核对服务端请求与 CDP。普通 requestWillBeSent 头并不等于线上头，因此补充测试保留 ExtraInfo 与服务端一致的正确判据；未删除原 ad0 网络测试。

原文件中的 main 差异包括 `moli-page-types/src/lib.rs` 和 image_resources 的磁盘池/ParkableImage 改造（main 提交 `18a9f45be`）、URL/SVG 属性、button-input popover、Fetch/XHR URL片段，以及新增 SVG/纯文本测试。这些已由三方合并保留；例如原640在 page-types 中只增加 child network journal/redirect字段，并未要求保留旧的独立临时文件实现。不能将 main 移除旧 spool 实现误判为拆分删除640功能。MediaQueryListEvent 模块拆分也属于 main，本次整个最终树与预期树的差异证明它未被覆盖。

## 测试覆盖保留

原 #640 新增的 131 个命名测试全部仍存在，0 遗漏。原六个被替换的测试名均有新名保留：五项几何契约从“等绘制才更新”变为“首个精确需求刷新，后续复用”，另一项 child-network 测试扩展为 transport metadata 断言。相关场景未整项删除。phase4/phase5 字体和滚动条 fixture 修正移动到 engineering；组合文件与原ad0一致，不是为拆分通过门禁弱化。

## 共享文件关键 hunk 归属

- `moli-layout/tests/phase5_output_contract.rs`：engineering: fixed Ahem/forced overflow fixture foundation; layout: fragment and clean/dirty geometry contract additions. Combined file equals original ad0 byte-for-byte.
- `moli-renderer-v8/src/native_bridge/context_host/host_environment.rs`：layout: mutation/resource/media/focus dirty invalidation; stop: ?settled enum diagnostics. Combined file equals original ad0.
- `moli-renderer-v8/src/native_bridge/element/activation/default_action.rs`：layout: image submitter DomGeometry flush; input: focus_before_dispatch and activation paths; main: input[type=button] popover activation retained.
- `moli-protocol-server/src/protocol_server/tests/classic.rs`：input: DOM click/frame/navigation tests; text: main/child MIME test; compatibility: expand CR/NUL/MIME-vs-extension/structure assertions without deleting any original case.
- `moli-renderer-v8/src/native_bridge/element.rs`：input: activation helper wiring; forms: lookup counters/property-handler wiring; main: URL quirks and SVG fetchPriority retained.
- `moli-renderer-v8/src/native_bridge/element/forms.rs`：input: keyboard submission helper import; forms: detached receiver import cleanup.
- `moli-renderer-v8/src/runtime/owner_local_store/mod.rs`：input: publish pending location navigation after input reply; stop: retire pending main parser before StopDocumentLifecycle dispatch.
- `moli-renderer-v8/src/script_vm/tests/dom_xhr/forms.rs`：input: input/default activation/submission scenarios; forms: lookup and detached/adopted scenarios. Whole file equals original.
- `moli-protocol/src/conn/state/runtime_slot.rs`：network: durable response body forwarding; stop: inflight navigation cancellation forwarding.
- `moli-protocol/src/domains/network/main_document_progress/mod.rs`：network: redirect-hop journal/request method/header and child metadata; stop: NET_ERR_ABORTED preserves committed document.
- `moli-protocol/src/domains/network/main_document_progress/tests.rs`：network: hop/journal tests; stop: abort/document preservation policy tests.
- `moli-renderer-v8/src/native_bridge/context_host/child_documents/loads.rs`：network: raw response/journal/redirect retention; text: MIME-specific decoder. Whole file equals original.
- `moli-renderer-v8/src/live_document_parser.rs`：text: main/live finite text parser constructors; stop: Stopped reason.
- `moli-renderer-v8/src/runtime/phase_one/state.rs`：text: new_text/is_text_document; stop: into_stopped_page_vm.
- `moli-renderer-v8/src/runtime/phase_one/streaming.rs`：text: parser/decoder dispatch and preload suppression; stop: exact transfer cancellation handle. Whole file equals original.

## 逐文件归属

下表覆盖原 148 个文件；原 hunk header、双方内容哈希及精确对齐结果另见同名 JSON。

| 原修改文件 | 最终分区 | 当前 main 对齐结果 |
|---|---|---|
| `.github/actions/use-ci-release/action.yml` | engineering | 完全保留 |
| `.github/scripts/unpack-ci-release.sh` | engineering | 完全保留 |
| `.github/scripts/unpack-ci-release.test.cjs` | engineering | 完全保留 |
| `README.md` | layout | 完全保留 |
| `docs/README.de.md` | layout | 完全保留 |
| `docs/README.es.md` | layout | 完全保留 |
| `docs/README.fr.md` | layout | 完全保留 |
| `docs/README.ja.md` | layout | 完全保留 |
| `docs/README.zh-CN.md` | layout | 完全保留 |
| `moli-bounded-buffer/src/buffer.rs` | network | 完全保留 |
| `moli-bounded-buffer/src/tests.rs` | network | 完全保留 |
| `moli-cdp-smoke/moli_cdp_smoke/groups/dom_input.py` | input | 完全保留 |
| `moli-cookie-import/src/sqlite.rs` | engineering | 完全保留 |
| `moli-curl/src/proxy.rs` | engineering | 完全保留 |
| `moli-dom/src/native/host/document.rs` | forms | 完全保留 |
| `moli-dom/src/native/host/query_index.rs` | forms | 完全保留 |
| `moli-encoding/src/document.rs` | text | 完全保留 |
| `moli-encoding/src/tests.rs` | text | 完全保留 |
| `moli-fetch/src/lib.rs` | network | 完全保留 |
| `moli-fetch/src/network_fetch_result.rs` | network | 完全保留 |
| `moli-fetch/src/request.rs` | network | 完全保留 |
| `moli-fetch/src/tests/support.rs` | engineering | 完全保留 |
| `moli-layout/src/builder.rs` | layout | 完全保留 |
| `moli-layout/src/inline.rs` | layout | 完全保留 |
| `moli-layout/src/layout_tree/pass_result.rs` | layout | 完全保留 |
| `moli-layout/src/pass.rs` | layout | 完全保留 |
| `moli-layout/src/projection.rs` | layout | 完全保留 |
| `moli-layout/src/taffy_tree.rs` | layout | 完全保留 |
| `moli-layout/src/world.rs` | layout | 完全保留 |
| `moli-layout/tests/phase4_layout_contract.rs` | engineering | 完全保留 |
| `moli-layout/tests/phase5_output_contract.rs` | engineering, layout | 完全保留 |
| `moli-page-types/src/lib.rs` | network | 完全保留 |
| `moli-parser/src/html.rs` | text | 上述文本兼容改写 |
| `moli-parser/src/session.rs` | text | 上述文本兼容改写 |
| `moli-parser/src/stream.rs` | text | 完全保留 |
| `moli-protocol-server/src/protocol_server/tests/classic.rs` | input, text | 上述文本兼容改写 |
| `moli-protocol-server/src/protocol_server/webdriver_bidi.rs` | input | 完全保留 |
| `moli-protocol-server/src/protocol_server/webdriver_classic.rs` | input | 完全保留 |
| `moli-protocol-server/src/protocol_server/webdriver_classic/helpers.rs` | input | 完全保留 |
| `moli-protocol-server/src/protocol_server/webdriver_classic/state.rs` | input | 完全保留 |
| `moli-protocol-webdriver-classic/src/actions.rs` | input | 完全保留 |
| `moli-protocol-webdriver-classic/src/commands/elements.rs` | input | 完全保留 |
| `moli-protocol-webdriver-classic/src/commands/mod.rs` | input | 完全保留 |
| `moli-protocol-webdriver-classic/src/lib.rs` | input | 完全保留 |
| `moli-protocol-webdriver-classic/src/tests.rs` | input | 完全保留 |
| `moli-protocol-webdriver-classic/src/types.rs` | input | 完全保留 |
| `moli-protocol/src/conn/browser_context/network_owner.rs` | network | 完全保留 |
| `moli-protocol/src/conn/devtools_command.rs` | input | 完全保留 |
| `moli-protocol/src/conn/page_state/fetch_state.rs` | network | 完全保留 |
| `moli-protocol/src/conn/runtime_load.rs` | stop | 完全保留 |
| `moli-protocol/src/conn/state/page_slot.rs` | stop | 完全保留 |
| `moli-protocol/src/conn/state/runtime_slot.rs` | network, stop | 完全保留 |
| `moli-protocol/src/domains/dom/resolve.rs` | input | 完全保留 |
| `moli-protocol/src/domains/dom/tests/geometry.rs` | layout | 完全保留 |
| `moli-protocol/src/domains/emulation/tests.rs` | text | 完全保留 |
| `moli-protocol/src/domains/input.rs` | input | 完全保留 |
| `moli-protocol/src/domains/input/tests.rs` | input | 完全保留 |
| `moli-protocol/src/domains/input/tests/keyboard_events.rs` | input | 完全保留 |
| `moli-protocol/src/domains/network.rs` | network | 完全保留 |
| `moli-protocol/src/domains/network/agent.rs` | network | 完全保留 |
| `moli-protocol/src/domains/network/backlog.rs` | network | 完全保留 |
| `moli-protocol/src/domains/network/events.rs` | network | 完全保留 |
| `moli-protocol/src/domains/network/main_document_progress/mod.rs` | network, stop | 完全保留 |
| `moli-protocol/src/domains/network/main_document_progress/tests.rs` | network, stop | 完全保留 |
| `moli-protocol/src/domains/network/output_queue.rs` | network | 完全保留 |
| `moli-protocol/src/domains/network/redirect_request.rs` | network | 完全保留 |
| `moli-protocol/src/domains/network/settings.rs` | network | 完全保留 |
| `moli-protocol/src/domains/network/tests/response_body.rs` | network | 完全保留 |
| `moli-protocol/src/domains/page.rs` | network | 完全保留 |
| `moli-protocol/src/domains/page/termination.rs` | stop | 完全保留 |
| `moli-protocol/src/domains/page/tests/lifecycle.rs` | text | 完全保留 |
| `moli-protocol/src/domains/page/tests/navigation.rs` | stop | 完全保留 |
| `moli-protocol/src/domains/page/tests/runtime.rs` | text | 完全保留 |
| `moli-protocol/src/domains/target/tests/tests_background_staging.rs` | text | 完全保留 |
| `moli-protocol/src/domains/target/worker_target.rs` | network | 完全保留 |
| `moli-renderer-v8/src/document_runtime/mutation_commands.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/dom_parser.rs` | text | 上述文本兼容改写 |
| `moli-renderer-v8/src/frame_owner_model.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/frame_owner_model/lifecycle_blockers.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/frame_owner_model/lifecycle_tasks.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/frame_owner_model/load_event_gate.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/frame_owner_model/records.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/frame_owner_model/store.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/frame_owner_model/store_tests.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/live_document_parser.rs` | stop, text | 完全保留 |
| `moli-renderer-v8/src/native_bridge/context_host/child_documents/commit.rs` | text | 上述文本兼容改写 |
| `moli-renderer-v8/src/native_bridge/context_host/child_documents/live_parser.rs` | text | 上述文本兼容改写 |
| `moli-renderer-v8/src/native_bridge/context_host/child_documents/loads.rs` | network, text | 完全保留 |
| `moli-renderer-v8/src/native_bridge/context_host/host_environment.rs` | layout, stop | 完全保留 |
| `moli-renderer-v8/src/native_bridge/context_host/image_resources/mod.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/context_host/layout.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/context_host/layout_snapshot.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/context_host/layout_state.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/context_host/main_document_lifecycle.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/native_bridge/document/detached_objects/builders/elements.rs` | forms | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element.rs` | input, forms | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/activation/default_action.rs` | layout, input | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/activation/mod.rs` | input | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/focus.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/forms.rs` | input, forms | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/forms/form_element.rs` | forms | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/forms/submission.rs` | input | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/geometry/metrics.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/geometry/rects.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/geometry/scroll.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/geometry/scroll_into_view.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/template_install.rs` | forms | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/template_install/accessors_forms.rs` | forms | 完全保留 |
| `moli-renderer-v8/src/native_bridge/element/template_install/accessors_forms/controls.rs` | forms | 完全保留 |
| `moli-renderer-v8/src/network/navigation/loader.rs` | network | 完全保留 |
| `moli-renderer-v8/src/page_task_queue/stylesheet_task.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/runtime/owner_local_store/entry.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/runtime/owner_local_store/mod.rs` | input, stop | 完全保留 |
| `moli-renderer-v8/src/runtime/page_vm/tests/child_document_completion.rs` | network | 完全保留 |
| `moli-renderer-v8/src/runtime/page_vm/tests/computed_size/sampling.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/runtime/page_vm/tests/fetch_xhr.rs` | engineering | 完全保留 |
| `moli-renderer-v8/src/runtime/page_vm/tests/grid_resolved_track_values.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/runtime/page_vm/tests/rendering_update.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/runtime/page_vm/tests/stylesheet_task.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/runtime/phase_one/bootstrap.rs` | text | 完全保留 |
| `moli-renderer-v8/src/runtime/phase_one/pending_residence.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/runtime/phase_one/state.rs` | stop, text | 完全保留 |
| `moli-renderer-v8/src/runtime/phase_one/streaming.rs` | stop, text | 完全保留 |
| `moli-renderer-v8/src/runtime/phase_one/streaming_input.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/runtime/phase_one/streaming_residence.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/runtime/protocol_output/transport_memory.rs` | network | 完全保留 |
| `moli-renderer-v8/src/runtime/tests.rs` | input | 完全保留 |
| `moli-renderer-v8/src/runtime/tests/open_streaming.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/script_vm.rs` | input | 完全保留 |
| `moli-renderer-v8/src/script_vm/input_dispatch.rs` | input | 完全保留 |
| `moli-renderer-v8/src/script_vm/stylesheet_page_tasks.rs` | stop | 完全保留 |
| `moli-renderer-v8/src/script_vm/tests/dom_elements/dom_surface.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/script_vm/tests/dom_xhr/computed_style.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/script_vm/tests/dom_xhr/dom.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/script_vm/tests/dom_xhr/forms.rs` | input, forms | 完全保留 |
| `moli-renderer-v8/src/script_vm/tests/dom_xhr/forms/event_activation.rs` | input | 完全保留 |
| `moli-renderer-v8/src/script_vm/tests/dom_xhr/forms/named_lookup.rs` | forms | 完全保留 |
| `moli-renderer-v8/src/script_vm/tests/dom_xhr/misc.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/style_engine/computed.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/style_engine/retained.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/style_engine/state.rs` | layout | 完全保留 |
| `moli-renderer-v8/src/stylesheet_runtime/connected.rs` | stop | 完全保留 |
| `moli-web-mime/src/classification.rs` | text | 完全保留 |
| `moli-web-mime/src/lib.rs` | text | 完全保留 |
| `moli-web-mime/src/tests.rs` | text | 完全保留 |
| `moli/src/fetch_dump/tests.rs` | text | 完全保留 |
| `moli/src/telemetry.rs` | engineering | 完全保留 |
| `moli/tests/fetch_cli.rs` | engineering | 完全保留 |

本审计只读固定 git 对象及组合源码，未修改工作树/refs、未运行 cargo 或访问应用站点。它证明拆分内容覆盖与必要兼容改写的边界，不替代当前版本的功能验收。
