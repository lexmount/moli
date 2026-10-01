**CDP 多页面状态与真实辅助窗口（2026-09-30）**

本分支让 DevTools target、页面脚本引用和导航状态共同指向实际 Page。带 opener 的脚本弹窗及链接同步创建辅助 Page，协议层接管同一个 Page；跨文档导航保留 Page 和 WindowProxy，并替换 Document、解析器和 Inspector attachment。窗口选择、页面可见性、浏览器焦点及 Inspector 启动暂停分别维护。

**实现与审查顺序**

| 模块 | 最终行为与约束 |
| --- | --- |
| Chromium 对照工具 | 保存操作、CDP 命令、原始响应、事件及 target/session 映射；支持固定复现、普通页面和弹窗随机序列、真实 DevTools 前端 |
| 旧 realm 生命周期 | 被 JavaScript 引用保留的旧 Document、DOM 和原生对象继续持有 backing；文档退役时停止活动任务，最后一个 realm 引用释放后回收；原生 DOMException 直接使用所属 realm 的内建原型 |
| 窗口、焦点与元数据 | 分别维护窗口、窗口内选中的 tab 和全浏览器焦点；关闭活动页选择相邻 tab；标题与 URL 由所属文档更新 discovery；activity 未变化的页面不参与 renderer 等待；事件回调读取对应转换阶段的状态 |
| 真实 Page、弹窗与导航 | 父页引用、CDP target、消息和脚本状态使用同一辅助 Page；名称与关闭状态属于 browsing context；已接受动作按窗口 ID 定位 target；稳定 Page 接口直接提交原始响应，首次导航、历史、关联组和空白页 reload 的环境按来源维护 |
| 多客户端 Inspector | tab session 支持恢复启动等待；同一 Page 的多个 Inspector session 共享启动暂停状态，恢复后不再由其他 session 的旧快照重新暂停 |

辅助文档响应桥接继续服务 `noopener` 和 Service Worker 等入口。文本响应、XML 响应及其他导航沿用既有字节解码和解析入口。导航提交在公共提交点按需整理 BiDi preload context；辅助 Page 接管由资源运行时替换步骤设置 navigator 身份，PageVm 缓存继续用于后续导航。

**行为回归**

原始编号对应 [16 项短复现](../fixtures/cdp-multipage-repros.json)。基线为 `14cc9792ef`，这些差异曾在三个独立浏览器进程中复现；下表描述最终实现，已执行的短复现均与 Chromium 对齐。

| 编号 | 已覆盖的行为 |
| --- | --- |
| R01 | 关闭当前 tab 后按 tab 顺序选择相邻页面 |
| R02、R03 | 后台页标题、同文档 URL、target discovery 事件和 HTTP 页面列表保持一致 |
| R04、R05 | 多个窗口独立选择 tab；跨 browser context 只有当前窗口的页面获得浏览器焦点；支持 `newWindow` |
| R06、R07 | 名称随 browsing context 保存，父页引用、子页改名和命名目标查找使用同一状态 |
| R08 | 弹窗首次真实导航替换初始空白历史占位；`document.open()` 不提前提交该占位 |
| R09、R10、R14 | opener、保留的窗口引用和消息收发方使用实际 Page；CDP 导航更新引用所见文档；消息保留 source/origin |
| R11 | 引用的 assign、replace、片段导航和 reload 导航实际 Page，并保留对应历史条目及 transitionType |
| R12、R13 | 脚本自关闭、父页关闭引用和协议关闭销毁同一窗口与 target |
| R15 | HTTP 文档响应只加载一次，重定向、Fetch、debugger、取消和下载仍走浏览器导航通路 |
| R16 | tab auto-attach session 接受 `Runtime.runIfWaitingForDebugger` |

额外回归覆盖同轮复用后改名与名称碰撞、一窗口一 live target、关联兄弟与独立根页面的命名查找、跨站导航及历史恢复、替换慢加载、初始空白页标题、关闭后的名称读写，以及空白页 reload 的 origin、base、referrer、策略和存储继承。脚本 reload 使用当前有效 base；浏览器/CDP reload 使用原导航的 fallback base 和 referrer。

集成测试检查页面正文、JS 状态、命名空间、编码、窗口身份、事件和历史。生命周期测试检查旧 Document/Function、DOM、Blob、AbortSignal 与最终 GC；V8/WebAssembly 测试覆盖已接受的 foreground task 在源页面关闭或执行 future 取消后仍交给存活页面。

初始历史占位、已出队任务的 Drop 转发，以及跨 Page 的 Promise rejection 收集分别覆盖不同状态，均保留独立回归。暂停作用域和 document replacement 继续负责现有 Page 的导航准备、取消与空白重载环境，未用重复公开适配接口替代。

**验证结果**

历史整理时，Rust 源码与整理前逐文件一致。2026-10-01 的后续修复再次从仓库根目录通过：

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-fail-fast
```

合并 `origin/main@75b191092d` 后的首轮全量结果为 **19,162 passed，16 skipped**，使用 32 路构建和测试，未触发重试。这些是本地工作区验证结果，不代表远端 CI 状态。新主线的初始文档测试分别覆盖普通浏览器创建和脚本弹窗：Chromium 实测普通创建不等待 `Runtime.runIfWaitingForDebugger` 才导航，脚本弹窗的启动暂停另行验证。下文记录后续空白导航修复的验证。

历史整理后的 [Chromium 对照](../results/cdp-ablation4-20260930/report.json) 包含 **39 组、528 步，全部零差异**：27 组固定复现加 12 个弹窗随机 seed（20260929–20260940，每组 30 步随机操作与 3 步初始化）。比较输入和规则保持不变。参考浏览器为 Chromium `145.0.7632.116`，使用独立 Xvfb、临时 profile、loopback 端口及本地 HTTP fixture；Moli 使用工作区构建并开启 `--layout`。

额外的 [Navigator 接管检查](../results/cdp-ablation4-navigator-20260930/report.json) 在两个浏览器上均通过，检查同步打开弹窗时缓存的 Navigator 身份、UA 数据内容、原生 getter，以及接管后和后续导航的 profile。

动态 ID 不参与跨引擎等值比较，URL fragment 合并后比较，协议可选的 `isDownload:false` 不算行为差异。焦点比较器保留原始观察，将 Chromium 实测的等价事件组合归一化；[Python 测试](../tests/test_multipage_cdp_fuzz.py) 继续检查错误状态、漏发和重复事件会报告差异。results 保存原始证据，属于本地产物，不加入版本控制。

显式空白导航的三个边界尚未纳入最终对照：脚本 assign 到 `about:blank`、`noopener` 显式空白页的来源/referrer，以及 CDP 显式空白导航的 Navigation API/frame 安全元数据。其他 CDP domain 的方法覆盖也不属于本报告的完成范围。当前结果限定于上述输入与回归测试。

**2026-10-01 审查补充**

四项审查问题分别修复并推送：

| 提交 | 行为 |
| --- | --- |
| `c14e39c3f3` | 空 URL 复用已有命名窗口时保留 Document、URL、DOM、脚本状态与历史；显式 `about:blank` 仍是导航请求 |
| `82c9f98ee4` | 命名查找涵盖关联的普通根 Page，先搜索当前 Page；协议使用已接受的名称对象身份，后续改名不重新选择目的窗口 |
| `63e03fae3f` | `window.open()` 复用窗口时同步更新原生 DOM opener，CDP 的创建者归属保持原值；链接复用不执行这项 setter |
| `a7bfc85983` | 普通弹窗归属来源页面的窗口，独立窗口请求分配新窗口；无用户激活的既有命名窗口复用不抢焦点 |

对照额外暴露的 `opener = null` 已由 `a10531127e` 修复：断开原生 opener 后，也定义自有 null 属性。后续原生 opener 更新不会覆盖这个脚本属性。`77de4b0111` 另外统一了导航取消的 Inspector 回复，消除原生回复和协议 fallback 竞争导致的错误文案差异。

[rebase 前的固定对照](../results/cdp-pr957-review-20261001/report.json) 使用同一个 headed Chromium 145.0.7632.116、独立 Xvfb 和本地 HTTP fixture，包含 8 组、61 步。59 步零差异，覆盖上述修复及上轮两个 P1 的复现。另 2 步仍能复现已记录的显式空白导航继承边界：导航后 Chromium 可访问 `sessionStorage`，Moli 报 `SecurityError`。没有修改比较器或删除这个输入。

参考浏览器在后台来源页创建新弹窗时会聚焦新弹窗，另一个窗口原先选中的页面保持 visible、失去 focus。这一结果分别验证了 CDP `userGesture` 为 true 和 false 的路径；已有窗口复用不提供用户激活时则保留原焦点。它与早期 headless 对照里“另一窗口仍有焦点”的描述不同，本轮以当前可执行浏览器结果为准。

[multi-context、multi-page、target-lifecycle、dom-input、tracing、Puppeteer 冒烟记录](../results/cdp-pr957-review-20261001/smoke-verification.json) 共 140 个场景通过，调度上限为 32。Puppeteer 首次因 shell 缺少 Node 和本地依赖而无法启动，使用本机 Node 24.15.0 及仓库锁定的依赖重跑后通过。

固定输入分别位于 [空 URL](../fixtures/cdp-popup-empty-url.json)、[命名根页面](../fixtures/cdp-popup-named-roots.json)、[DOM opener](../fixtures/cdp-popup-dom-opener.json) 和 [窗口归属及焦点](../fixtures/cdp-popup-window-policy.json)。本轮结果不代表全部 CDP 行为已与 Chromium 等价。

**Rebase 后的空白导航继承修复**

`96dae5228a` 修复已有命名窗口显式导航到 `about:blank` 的环境继承。在 renderer 接受调用时保存 initiator 的 origin、安全 token、存储 key、有效 base 和 policy；浏览器调度携带一次性句柄，原生/V8 状态继续留在所属 owner。新 Document 使用这个快照，目标窗口仍保留自己的 session-storage namespace。取消和替换使用既有 owner 清理与 entered-isolate 释放通路。CDP 的 securityOrigin 同时使用捕获的来源，避免报告旧目标的 origin。

[新增固定输入](../fixtures/cdp-popup-blank-inheritance.json) 覆盖调用后立即修改 `<base>`、兄弟窗口复用及后续 reload。[当前固定对照](../results/cdp-blank-initiator-20261001/report.json) 共 **10 组、81 步，全部零差异**，包含之前保留的两处存储失败输入，未修改比较器。Rust 集成测试另覆盖不同端口 origin 的兄弟窗口：检查 DOM origin、base、referrer、目标的存储 namespace、CDP securityOrigin 与跨 origin 访问；其预期来自 [Chromium 实际结果](../results/cdp-blank-initiator-20261001/chromium-cross-origin-ports.json)。默认 referrer 策略针对显式空白导航保留 origin，`<base>` 修改不改变 referrer。

格式检查、Clippy 和全量测试通过，**19,164 passed、16 skipped**。跨 context 焦点用例首次在激活后的 `hasFocus()` 断言失败，重试通过；该用例没有调用本次修改的命名导航路径，独立连续复测 **20/20 通过**，本轮未改变其断言。[验证记录](../results/cdp-blank-initiator-20261001/validation.json) 保留这次重试和测试日志。[六组冒烟](../results/cdp-blank-initiator-20261001/smoke-verification.json) 共 **140 个场景通过**，构建和测试并行度为 32。对照启动于提交前工作区，源码及固定输入与 `96dae5228a` 一致，未将旧提交号改写成新 HEAD。

额外的 [跨站探测](../results/cdp-cross-site-named-20261001/report.json) 没有复现命名窗口选错：两端都会在 browser/CDP 跨站导航后的新关联组创建新窗口。但它揭示了另一个未修的 referrer 缺口：普通 HTTP 弹窗首次导航的 `document.referrer` 在 Moli 中为空，而 Chromium 保留父页 URL；原报告的状态观察没有比较这个字段。这与本次修好的显式空白导航继承不同，保留完整输入和结果以便后续单独修复。

**空白导航的安全策略补充**

命名空白导航将发起方 policy 与目标 frame 的固定 sandbox 限制合并，不再继承目标旧响应的 CSP sandbox；对应的目标 Document sandbox 读取接口已删除。固定限制随既有辅助窗口身份在创建时保存，响应不能改写它。CDP `secureContextType` 与 `securityOrigin` 使用同一份调用时快照，原生安全状态来自 realm 的内部 slot，公开 `isSecureContext` 属性的脚本改写不会污染元数据。

[Chromium 实测](../results/cdp-blank-policy-20261001/chromium-results.json) 覆盖 HTTPS 发起方替换 HTTP 目标、反向组合，以及被 CSP sandbox 限制的旧目标替换为空白页。新增 Rust 回归检查安全状态相反的两种组合、旧响应策略丢弃，以及受 sandbox 限制的创建者向弹窗传递的固定限制：创建者导航到普通文档后，重新导航弹窗仍不能解除其模态框限制。安全状态测试通过 Fetch fulfillment 导航实际辅助 Page，保留其窗口身份和关联组。旧目标的顶层 CSP sandbox origin 与开窗限制在 Moli 中仍有历史缺口，旧响应替换测试检查替换后的 origin、存储和父页访问，不声称修复旧文档的这些限制。

这组修复从根目录通过格式检查、Clippy 和全量测试，**19,168 passed、16 skipped**，使用全部 32 个可用核。`lifecycle_decider_does_not_extend_initial_stage_timeout` 首次失败、重试通过；没有改变其断言。[验证记录](../results/cdp-blank-policy-20261001/validation.json) 保存定向测试、当前全量日志及 Chromium 输入结果。

**来源快照持有范围的消融**

`ScriptVmCapturedDocumentEnvironment` 不再强引用整个来源 `JsContextHost`，只保留必要的 initial environment 和所属 isolate。普通 origin 使用的 token 不需要来源 realm；opaque origin 与 `document.domain` 使用的 context token 通过已有 realm owner 保留原生 backing。取消时把 V8 状态交给既有 entered-isolate 释放队列，该队列现在也接收不附带 host 的状态。删除了仅为获得这个强引用而升级来源 host 的错误分支，未新增保存容器或清理流程。

三个 GC 回归已通过：来源 DOM 可以在普通 origin 快照消费前被回收，已接受 token 仍能消费；opaque 与 `document.domain` 的 realm 在取消前存活，取消后通过现有队列和 GC 释放。合并安全策略与旧 realm 回归的十项定向测试全部通过。

消融的格式检查、Clippy 和全量测试通过，**19,171 passed、16 skipped**。`same_context_targets_keep_local_network_policy_but_share_process_locale` 首次读到空标题、重试通过；该用例在导航回复后立即求值，未等待解析完成，未改变其断言。[当前验证](../results/cdp-blank-host-ablation-20261001/validation.json) 保存这个失败及重试日志。[固定 Chromium 对照](../results/cdp-blank-host-ablation-20261001/fuzz/report.json) 为 **10 组、81 步，全部零差异**；[六组冒烟](../results/cdp-blank-host-ablation-20261001/smoke/summary.json) 共 **140 个场景通过**。对照运行时 HEAD 为 `4a8009c90f`，使用包含消融的已构建工作区源码；原始运行记录保留该提交号。

**复现方法**

依赖 Chromium、Xvfb 和 benchmark 的 Python 依赖。从仓库根目录运行，并使用新的输出目录：

```sh
cargo build -p moli --bin moli
PYTHONPATH=moli-benchmark uv run --project moli-benchmark python -m moli_benchmark.multipage_cdp_fuzz \
  --moli target/debug/moli \
  --case-file moli-benchmark/fixtures/cdp-multipage-repros.json \
  --output moli-benchmark/results/cdp-multipage-repro
```

单项复现可加 `--only R10-stale-window-reference`；任意保存的序列可通过 `--actions <actions.json>` 回放。[其他固定输入](../fixtures/) 覆盖窗口身份、焦点等待、命名复用、关联组、导航竞争、引用导航和空白页标题。

```sh
PYTHONPATH=moli-benchmark uv run --project moli-benchmark python -m moli_benchmark.multipage_cdp_fuzz \
  --moli target/debug/moli --seed 20260929 --seeds 16 --popup-seeds 12 --steps 30 \
  --output moli-benchmark/results/cdp-multipage-random

PYTHONPATH=moli-benchmark uv run --project moli-benchmark python -m moli_benchmark.multipage_cdp_frontend \
  --moli target/debug/moli \
  --output moli-benchmark/results/cdp-multipage-frontend
```

工具位于 [multipage_cdp_fuzz.py](../moli_benchmark/multipage_cdp_fuzz.py) 和 [multipage_cdp_frontend.py](../moli_benchmark/multipage_cdp_frontend.py)。本地 Chromium 源码参考 revision 为 `a03603fe9af6230a12f1b2fb2c18a7d003a0d937`；tab attach 与 focus emulation 场景参考 Blink 的 `inspector-protocol/target` 和 `inspector-protocol/emulation` 测试，预期结果来自实际运行的参考浏览器。
