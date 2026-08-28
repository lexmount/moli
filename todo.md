# TODO

## Objective

把网络、render owner、CDP 调度保持在 async-first 模型上：Web API / CDP 发起网络只启动或恢复请求，不在 render owner 上同步等网络；跨 live `PageVm` / render owner / CDP dispatcher 的接口保持 async；本地纯投影可以继续是同步函数。

## Current State

- CDP 生产入口已是 async-first：domain dispatcher 只保留 `dispatch_async(...)` / `process_async(...)`，测试侧 `TestContext::process(...)`、`block_on_test_future(...)`、`test_async.rs` 已删除。
- `moli_core::page::Page` 的 public sync command facade 已清掉：runtime eval、testing outcome、loader lifecycle、subresource interception、cookie facade override、client rect / node removal / object resolution 等跨 render owner 调用只保留 async API。
- renderer owner 同步 command path 已清掉：`RendererPageHandle::run_sync_command(...)`、owner-local store sync branch、sync page-command dispatcher、blocking pending-navigation follow 都不存在。
- `RendererPageHandle::drop` 只做 detached cleanup；生产 CDP loaded-page teardown / `Page.close` 已显式 `close_async().await`。剩余 `drop(page)` 命中主要是 runtime inline tests，包括 detached cleanup 覆盖。
- teardown 复核已完成：`drop(page)` 只命中 `#[cfg(test)] mod tests` 内的 detached cleanup / shared-host cleanup / abort-restore 回归；CDP loaded page replacement、target reset、`Page.close` 生产路径都显式 await `close_async()`。`Page::close_async()` 文档已明确 deterministic teardown vs Drop best-effort 边界。
- create-page 期间 `location.*` 触发 navigation 已走 `TriggeredNavigation -> RenderRuntimeTurn::FollowPendingLocationNavigation`，进入 render-runtime pending turn 队列，不再在原始 V8 调用栈里递归重建页面。
- live-page snapshot / `networkidle` / `domstable` 路径已经进入 render-runtime pending-turn 队列：`RunLivePageCommand`、`WaitLivePageNetworkIdle`、`WaitLivePageDomStable` 会在需要时转成 `FollowLivePagePendingLocationNavigation`，每个 follow turn 最多跟随一次 pending `location.*` navigation。
- pending navigation 回归已覆盖正常链和循环链：page load、`Runtime.evaluate` 后 snapshot refresh、`networkidle` / `domstable` wait 驱动。循环链会稳定报错；`Runtime.evaluate` 触发循环后 owner-local entry 会恢复，随后显式 `close_async()` 能正常清理。
- live `Page::wait_for_network_idle` / `Page::wait_for_dom_stable` 已有正常 chained navigation、timeout navigation loop、普通 timeout、caller cancellation 的 entry-restoration 回归：正常链会等到最终 `/target?from=chain-mid` snapshot；循环链、普通 timeout 或 caller cancellation 后 owner entry 会恢复，显式 `close_async()` 能清理；取消 `networkidle` 后直接 drop page 也能通过 detached cleanup 最终移除 owner entry。
- live-page pending navigation turn 已落地在代码和 focused regression 中。实现选择保留现有“await 后 snapshot 反映最终导航”的外部语义，用 `RenderRuntimePendingTurn.reply_tx` 停住原 reply，用 `LivePagePendingNavigationCompletion` 保存命令 continuation。
- live wait cancellation 已在 turn boundary 处理：`networkidle` / `domstable` wait 不再用一个长 owner task 跑到原始 timeout，而是按短 pending-turn slice 重排；如果原 reply receiver 被 drop，下一轮会恢复 held entry 并停止 parked command。
- 注意：`domstable` 的 slice 当前比 `networkidle` 长，因为 DOM stability 的 `last_snapshot/stable_since` 还在单次 `wait_for_dom_stable()` 调用栈内；如果要更细 cancellation，需要先把这份状态搬进 continuation。
- 长 live-page command wait 也已收进 pending-turn 模型：`WaitForSelector` / `WaitForScriptTruthy` 不再通过通用 `RunLivePageCommand` 长 await 持有 entry，而是各自用短 wait slice；caller cancellation 后显式 `close_async()` 能清理 owner entry。
- `DrainTimeoutsBestEffort` 仍是通用 async page command，但已有 `aborting_async_page_command_restores_local_host_entry` 覆盖：丢弃 in-flight command future 后 owner entry 会恢复，后续 page command 仍可 dispatch。当前不再为它单独拆 pending turn。
- 已系统复核 `RendererPageCommand`：真正长等待已经是 dedicated pending turn；剩余 generic `RunLivePageCommand` 是 runtime eval/protocol dispatch、input、isolated world/runtime binding、DOM 小投影或小变更、child-frame projection、Fetch pending-subresource continue/fail/fulfill、settings/cookie facade 等单次 owner-local 操作。`Runtime.evaluate awaitPromise` 当前不在 generic command 内做长轮询，只做 wrapper 求值和一次 promise token readback；轮询式 promise wait 仍由 selector/script truthy 专用 turn 承担。
- API 面复核已完成：`Page` 上触碰 renderer owner 的操作仍只通过 async 方法；同步 public 方法只读 cached snapshot / DOM projection / stable id。CDP domain 生产入口仍只有 `dispatch_async(...)` / `process_async(...)`。
- `RendererOwnerLocalStore` 现在只暴露单步 live-page 操作 helper：dispatch 一个 async page command、wait 一个 `networkidle` / `domstable` 单元、follow 一个 pending navigation turn、commit 一个 snapshot。旧的 `continue_pending_location_navigation_on_entry(...)` inline loop 已删除。
- 剩余 `block_on` 命中主要是测试 harness 自建 tokio runtime、`moli` CDP socket 专用 current-thread runtime、render-runtime 专用线程入口、phase-one inline test module；未看到“async facade 内部偷偷 block_on render owner”的生产路径。
- 已清理的命名假阳性：CDP `sync_loaded_page_*` -> `apply_loaded_page_*`，owner-local access / snapshot refresh 的 `sync_*` -> `refresh_*`，ScriptVm post-parse runtime work 的 `sync_*` / `block_on_dynamic_loads` -> `refresh_*` / `wait_for_dynamic_loads`。
- async/render-owner 主线当前进入维护态：后续只在新增跨 owner/dispatcher 行为时继续用本文件的 guardrails 复核。

## Now

- 不要继续机械清所有 `sync_*`。很多 JS/DOM 层函数是在同步更新 V8 object surface，例如 location/history/navigation/CSS/IndexedDB surface，这不是旧同步等待模型。
- 继续推进前，优先复扫是否出现新的危险同步等待：
  - `block_on_test_future|test_async|ctx\\.process\\(|TestContext::process`
  - `run_sync_command|block_on_in_any_runtime|dispatch_renderer_page_command_sync|follow_pending_location_navigation_blocking`
  - `dispatch_page_command_sync|dispatch_unit_page_command_sync|decode_.*_page_command_sync`
- 当前最有价值的结构项已经完成第一阶段：live-page pending navigation follow 不再是 owner-local inline loop；`networkidle` / `domstable` / selector / script truthy 等长 wait 都已有 turn-boundary cleanup；`DrainTimeoutsBestEffort` 的 abort restore 也有覆盖；`RendererPageCommand` 剩余 generic 命令已复核为单次 owner-local 操作。接下来不要继续机械拆所有 async 命令，只有新命令能跨网络、timer、DOM mutation、promise settlement 或 navigation chain 多轮等待时，才需要 dedicated pending turn 或明确的 cancellation/restoration 回归。
- 如果继续找产品行为事项，优先按当前 WPT/current-state 文档和代码重新建 focused TODO，而不是依赖已删除的阶段性 docs。`navigation.forward()` 的 `--dump-dom + virtual-time-budget` probe 不可靠，但 CDP-driven probe 可作为 oracle；forward result-promise 细节已解阻，现有 focused tests 已覆盖。

## Next

1. 巩固 live-page pending navigation turn：
   - 设计事实现在以实现和 focused regression 为准。
   - pending-turn state 已落地：`RendererPageToken + RendererPageLocalEntry + PageVmInitStage + follow_count + LivePagePendingNavigationCompletion`。
   - `Runtime.evaluate` / wait driver 触发 `location.*` 后的错误恢复、普通 wait timeout、caller cancellation 和 cleanup 行为已有 focused coverage。
   - `WaitForSelector` / `WaitForScriptTruthy` 已从通用 `RunLivePageCommand` 长 await 中拆出，有 focused cancellation coverage。
   - `DrainTimeoutsBestEffort` 已重新跑 abort restore coverage，当前不拆。
   - `RendererPageCommand` 分类已通过当前代码复核；后续新增命令先判断是否允许留在 generic `RunLivePageCommand`。
   - 目标仍是：页面脚本只产生 navigation request；后续由 render owner 的 top-level turn 继续，而不是在当前 command 内连续 follow。

2. 继续补 navigation 边界测试：
   - live `Page::wait_for_network_idle` / `Page::wait_for_dom_stable` 的正常 chained navigation 和 loop restoration 回归已补。
   - live `Page::wait_for_network_idle` / `Page::wait_for_dom_stable` 的普通 timeout entry-restoration 回归已补。
   - caller cancellation 回归已补；selector / script truthy cancellation 回归也已补。下一步如果继续动 wait continuation，优先补新的边界类型，不要再重复正常链、普通 timeout 或 cancellation。
   - 保留当前正常链 / 循环链回归作为行为锁。

3. 复核 teardown：
   - 已完成：生产路径需要确定性 teardown 时均显式 `close_async()`。
   - 已完成：`drop(page)` 只作为测试里的 best-effort cleanup 行为锁；取消 live wait 后 drop page 的 eventual cleanup 已有 focused 回归。
   - 后续新增 owner/CDP teardown 时继续按这个规则：生产路径 await `close_async()`；`Drop` 只作为兜底，不作为业务完成信号。

4. 保持 API 面收敛：
   - 已完成：`Page` API 复核确认跨 render owner / live page / CDP dispatcher helper 仍是 async。
   - 已完成：本地纯计算 / snapshot projection helper 保持同步，没有为了统一命名强行 async 化。
   - 后续新增 API 时继续按这个规则：跨 owner/dispatcher async，本地 cache/projection sync。

## Later

- `History` / `Navigation` 的后续事项应重新开 focused current-state/TODO 入口：当前重点是只在 Chromium/CDP/WPT 证据稳定时补 traversal/result-object 边界；`navigation.forward()` result-promise 细节已由 CDP probe 解阻，当前不需要新增重复断言。
- 评估 CDP WebSocket server 的 dedicated current-thread runtime 是否可以在更大的 thread-affinity 抽象下统一，但不要把 `CdpConnection` future 放回 axum multithread executor。
- 全量清理时再跑一次 workspace 级 `cargo nextest run --no-fail-fast`，当前每轮只跑 focused tests。

## Verification

本轮重新确认：危险同步入口前三组扫描无输出；`continue_pending_location_navigation_on_entry` 已无命中；`RendererPageCommand` 剩余 generic 命令已按 single-operation / dedicated long-wait turn 分类；teardown 复核确认 `drop(page)` 只在 runtime tests，生产 CDP/owner teardown 显式 await `close_async()`；API 面复核确认 `Page` 同步 public 方法只读 cache/projection，跨 owner 操作仍为 async；`cargo fmt --all --check`、`cargo check -p moli-renderer-v8 --tests`、`cargo check -p moli --tests`、pending navigation focused nextest、`git diff --check` 均通过。新补的 live wait 正常链、loop restoration、普通 timeout restoration、caller cancellation restoration、cancellation 后 detached drop cleanup、selector/script truthy cancellation focused tests 也通过；`DrainTimeoutsBestEffort` 的 abort restore 回归仍通过。

最近通过：

```bash
cargo fmt --all --check
cargo check -p moli-renderer-v8 --tests
cargo check -p moli --tests
cargo check -p moli-cdp --tests
cargo check -p moli --tests
cargo nextest run -p moli-renderer-v8 refresh_post_parse_runtime_work post_parse_lifecycle_driver_task_execution_marks_only_host_event_tasks_for_runtime_work_refresh --no-fail-fast
cargo nextest run -p moli async_page_command_snapshot_follow_can_adopt_pending_location_navigation location_navigation_keeps_same_renderer_page_id location_navigation_refreshes_owner_vm_incarnation_for_dispatch follows_location_reload_during_page_load --no-fail-fast
cargo nextest run -p moli rejects_location_navigation_loop_during_page_load async_page_command_snapshot_rejects_chained_location_navigation_loop follows_chained_location_navigation_during_page_load async_page_command_snapshot_follow_can_adopt_pending_location_navigation --no-fail-fast
cargo nextest run -p moli rejects_timeout_location_navigation_loop_during_network_idle_wait rejects_timeout_location_navigation_loop_during_domstable_wait rejects_location_navigation_loop_during_page_load follows_chained_location_navigation_during_page_load follows_async_location_search_during_network_idle_wait follows_async_location_search_during_domstable_wait --no-fail-fast
cargo nextest run -p moli async_page_command_snapshot_rejects_chained_location_navigation_loop async_page_command_snapshot_follows_chained_location_navigation follows_timeout_chained_location_navigation_during_network_idle_wait follows_timeout_chained_location_navigation_during_domstable_wait rejects_timeout_location_navigation_loop_during_network_idle_wait rejects_timeout_location_navigation_loop_during_domstable_wait --no-fail-fast
cargo nextest run -p moli async_page_command_snapshot_rejects_chained_location_navigation_loop async_page_command_snapshot_follows_chained_location_navigation live_page_networkidle_follows_chained_location_navigation live_page_domstable_follows_chained_location_navigation live_page_networkidle_rejects_chained_location_navigation_loop_and_restores_entry live_page_domstable_rejects_chained_location_navigation_loop_and_restores_entry follows_timeout_chained_location_navigation_during_network_idle_wait follows_timeout_chained_location_navigation_during_domstable_wait rejects_timeout_location_navigation_loop_during_network_idle_wait rejects_timeout_location_navigation_loop_during_domstable_wait --no-fail-fast
cargo nextest run -p moli live_page_networkidle_timeout_restores_entry_for_close live_page_domstable_timeout_restores_entry_for_close --no-fail-fast
cargo nextest run -p moli live_page_networkidle_cancellation_restores_entry_for_close live_page_domstable_cancellation_restores_entry_for_close --no-fail-fast
cargo nextest run -p moli live_page_networkidle_cancellation_allows_detached_drop_cleanup --no-fail-fast
cargo nextest run -p moli wait_for_selector_finds_late_element_via_renderer_wait_loop wait_for_selector_times_out_for_missing_selector wait_for_selector_reports_invalid_selector_errors wait_for_selector_cancellation_restores_entry_for_close wait_for_script_truthy_observes_delayed_fetch_state_change wait_for_script_truthy_times_out_for_false_predicate wait_for_script_truthy_reports_invalid_expression_errors wait_for_script_truthy_cancellation_restores_entry_for_close --no-fail-fast
cargo nextest run -p moli aborting_async_page_command_restores_local_host_entry --no-fail-fast
cargo nextest run -p moli fetch_registers_page_in_renderer_registry_until_page_is_dropped fetch_registers_page_in_renderer_registry_until_page_is_closed_async live_page_networkidle_cancellation_allows_detached_drop_cleanup dropping_page_only_removes_its_entry_from_shared_local_host renderer_owner_recreates_local_host_after_last_page_drops page_drop_removes_owner_entry_via_bound_slot --no-fail-fast
cargo nextest run -p moli navigation_forward_result_promises_settle_after_async_traversal navigation_back_result_promises_settle_after_async_traversal navigation_traverse_to_result_promises_settle_after_async_traversal navigation_forward_dispatches_currententrychange_traverse_event_surface --no-fail-fast
git diff --check
```

## Re-Evaluate Before Continuing

先跑这些扫描，确认没有旧入口回流：

```bash
rg -n "block_on_test_future|test_async|ctx\\.process\\(|TestContext::process|load_page_via_runtime_for_tests|load_navigation_request_via_runtime_for_tests" moli-cdp/src -g '*.rs'
rg -n "run_sync_command|block_on_in_any_runtime|dispatch_renderer_page_command_sync|follow_pending_location_navigation_blocking" moli-renderer-v8/src moli/src moli-cdp/src -g '*.rs'
rg -n "dispatch_page_command_sync|dispatch_page_command_sync_readonly|dispatch_unit_page_command_sync|decode_.*_page_command_sync|run_sync_command" moli-core/src/page/mod.rs
rg -n "pub fn process\\(|process\\(conn, cmd, out\\)" moli-cdp/src/domains -g '*.rs'
rg -n "drop\\(page|close_async\\(" moli/src moli-cdp/src moli/src -g '*.rs' -g '!**/tests.rs' -g '!**/tests/**'
```

前三组预期无输出；`drop(page|close_async)` 当前预期只看到 `Page::close_async`、CDP teardown / `Page.close` 显式 close，以及 runtime inline tests。
