# Popup / Auxiliary Browsing Context：现状、Chromium 对照与统一方案

日期：2026-08-02；最后更新：2026-08-23

## 2026-08-23 P6R10 Window call realm identity 与同步 child script 收口结论

P6R10 完成了 P6R9 留下的两个实际可达边界。`window.open()` 现在分别保存 receiver、entry
与 accessing realm 的 typed identity，child Document 动态插入的 inline classic script 也在
DOM mutation 返回前执行。两条修复都落在通用 Window operation 与 Document script owner，popup
继续复用 stable child-frame WindowProxy/realm 基础，没有增加 popup 专用 facade 或调度器。

Chromium 基线仍固定在 `a03603fe9af6230a12f1b2fb2c18a7d003a0d937`。其中
`LocalDOMWindow::open()` 在 `local_dom_window.cc:2296-2450` 明确保留两组不同事实。

| Chromium owner fact | P6R10 的对应实现 |
| --- | --- |
| `this` 对应的 `LocalDOMWindow` 决定 receiver frame、target tree、sandbox admission、transient activation、opener relation 与 session storage clone | `WindowOperationReceiver` 在 WebIDL argument conversion 前冻结 exact owner、dispatch scope、realm token 与 V8 context，conversion 返回后只接受同一 generation |
| `EnteredDOMWindow` 完成 URL，并创建 `FrameLoadRequest`、referrer 与 creator fetch client | entry identity 提供 base URL、Document policy、CSP/Trusted Types、resource loader、referrer 和 destination request source |
| caller realm 负责 receiver access check 与 argument conversion | accessing host 与 receiver host 分开解析；跨 Page borrowed method 使用 related-page script-agent 与 effective-origin 检查 |
| target lookup 从 receiver frame 开始，request creator 仍是 entered Window | popup activation source 指向 receiver Page/Frame，activation 内的 navigation request source 指向 entry Page/Frame |

这组拆分也覆盖 author getter 引发的同步 replacement。调用 `Window.prototype.open` 后，URL
conversion 可以删除并重新插入 receiver iframe。旧 LocalWindow generation 此时返回 `null`，不会按稳定
child handle 重新绑定到 replacement realm。跨 Page 的同源 related Window receiver 则保持可调用；endpoint
marker 只识别 proxy 形态，最终授权由 observer identity、target identity、script-agent membership 与 effective
origin 一起决定。

动态 child script 路径已经改成 mutation turn 内的有序候选。主 Document script 与 child inline
classic script 按 DOM mutation 发现顺序准备，child script 在 exact child owner scope 中完成 CSP、Trusted
Types、nonce、integrity 与 source 解析，随后同步进入该 child realm。外层 DOM API 返回时才执行原有
microtask checkpoint；脚本引发的 nested mutation 不提前 drain microtask。脚本同步移除自身 frame 的场景也由
同一 owner currentness 检查收口。

这次清理采用物理删除。没有 production writer 的 `ENTERED_CHILD_WINDOW_HANDLE_SLOT` 已移除，旧
`PendingChildDynamicDocumentScript`、`FrameDocumentDynamicClassic*`、ready task、owner action 与 followup
调度栈也已移除。`ACTIVE_CHILD_WINDOW_HANDLE_SLOT` 仍有 production writer，继续承担当前 child owner scope。
完整旧 owner 不会转移到 `cfg(test)`。`cfg(test)` 只保留构造 production 输入的 fixture、无副作用只读
accessor 与断言。

最终 release binary SHA-256 为
`f193e206c22d7888940a7ea7ab6a15fbf8a2d8f8233f3f6914b34976b7599a1b`。WPT checkout 固定在
`db95fafd1fcef8428805e41eb5705d444e8c67ce`。四个 `multiple-globals` focused cases 在 CLI 与
CDP 均为 4/4 pass。81-case popup 矩阵的最终结果如下。

| 入口 | P6R9 基线 | P6R10 最终结果 | 变化 |
| --- | --- | --- | --- |
| CLI | 35 pass / 27 fail / 19 timeout | 40 pass / 26 fail / 15 timeout | 新增 5 个 pass，没有丢失既有 pass |
| CDP | 35 pass / 27 fail / 19 harness-stalled | 40 pass / 26 fail / 15 harness-stalled | 归一化 case 分类与 CLI 完全一致 |

新增通过项是三个 Location multiple-globals case、`context-for-window-open.html` 与
`choose-_self-001.html`。最终代码相对前一版 P6R10 release 的逐 case status 和 failure-name 没有漂移。
CLI 与 CDP 的 timeout 表达不同，归一化后的 81 个 case status 全部一致。产物保存在
`/tmp/moli-wpt-popup-p6r10-final2-{multiple-globals-,}{cli,cdp}-20260823-1`。

按当前产品边界，P6R10 已满足单进程 popup owner 计划的最后两个 exit。production lightweight 扫描、
entered child ghost marker 和旧 async DynamicClassic stack 都保持零命中。81-case 矩阵仍有 26 个 fail 与
15 个 timeout，这些结果需要按 history、opener、name、testharness completion 等责任方继续分类。没有证据时，
不能把它们一律算作 popup owner 缺口，也不能据此恢复已经删除的双栈。

## 2026-08-23 P6R9 child identity 与 navigation source 复核结论

这轮复核确认了两处仍在 production 路径上的 child browsing-context identity 错误，也收回了
P6R8 文档对 remote form 和 remote descendant 的过度要求。第一处错误把 `<iframe name>` 属性
和 browsing-context name 存在同一个字段中。owner refresh 会把脚本写入的 `window.name` 覆盖掉，
随后 `_self` 和普通 named lookup 都可能丢失目标。第二处错误把 Location 与 `window.open()` 的
相对 URL 都按 incumbent realm 解析，本地 child Location 路径还会退化成只带 URL 的 bootstrap，
因此 entry realm 的 base 与 incumbent Document 的 referrer/source 无法同时成立。

本轮修复让既有 child entry 在 owner attribute refresh 时保留 browsing-context name，并让
HTTP(S) child Location 交付统一的 typed GET request。URL 使用 entry settings object 的 base，
request source、network `Referer` 和最终 `document.referrer` 则来自 incumbent Window/Document。
`blob:` 等同步 materialization 路径仍使用既有 URL bootstrap，避免把同步 child commit 错改成
network request task。

Chromium 对照也改变了剩余工作判断。`RemoteFrame::Navigate()` 的 wire 保存 URL、POST body、
headers、referrer、form 标记和 initiator，不传递 DOM Element 或 V8 `FormData`。Blink 只在 source
Window 能访问本地 target frame 时把 `source_element` 交给 target `NavigateEvent`，`FormData` 随后由
该本地元素重建。跨 agent 搬运 DOM facade 既不符合 Chromium，也不符合 Moli 的 isolate owner。
当前 remote request carrier 已覆盖产品需要的 method、body、headers、referrer 与 scheduler identity，
所以 remote DOM element/`FormData` wire 不再是 popup exit condition。

同理，当前 Page owner 已在 target Page 内递归处理本地 descendant 的 beforeunload、pagehide、unload
和 close ACK。只有真实 OOPIF 或多进程 descendant 出现后，才需要跨 endpoint 聚合 descendant ACK。
Moli 当前没有这种 production producer，这项工作归入可选多进程基础设施，不再计入单进程 popup 终态。

按三个口径看 P6R9 结束时的状态如下。P6R10 已完成表中的 receiver/entry 与 child script 两项，
当前结论以上一节为准。

| 口径 | 当前判断 | 剩余重点 |
| --- | --- | --- |
| 常用 DOM popup 行为 | 约 99% | 扩大 focused WPT，并清理暴露出来的通用 child lifecycle/script scheduler 阻塞 |
| Moli 单进程 popup 终态 | 约 99% | `window.open()` receiver、entry 与 incumbent 的精确分离，外加 focused WPT/CDP 重新分类 |
| Chromium 完整基础设施等价 | 不作为当前产品 exit | 真实 process/channel、capability broker、fenced/guest、完整 Reporting/agent reunification |

这里的百分比是风险加权工程估计，不是测试通过率。production lightweight owner、fallback、
mirrored loader/parser/lifecycle 和 observation seam 已由 P6R4 物理删除，tracked Rust 宽口径扫描仍为
零命中。完整旧 owner 不应以 `cfg(test)` 形式保留。只有构造 production 输入的 fixture，或无副作用
读取 production state 的 accessor，才适合放在 `cfg(test)` 下。

## 2026-08-23 P6R8 remote JavaScript URL closure 结论

此前改动的核心方向符合预期：popup 已复用 child-frame 的 stable WindowProxy/realm
基础，并把真实 auxiliary Page、Document、loader、target/session、name/opener、policy、
lifecycle 与 remote route 收进 typed owner。G5/G6 又证明 stable logical endpoint 可以和
agent-local V8 projection 分离，same-group cross-origin commit 不需要退回 lightweight
record 或共享 opener realm。

本轮复核也修正两个过度外推。第一，strict wire seam 不等于真实多进程 renderer 已完成；
第二，真实 OS process/OOPIF 不是 Moli popup 终态的必选前置条件。Moli 的产品目标是低资源
headless browser 的可观察语义，不要求复制 Chromium 的完整 SiteInstance/Mojo 拓扑。G6B1
应作为当前单进程 owner 的 fail-closed transport contract，以及未来若采用多进程时的复用
边界；不应为了 popup 单独先建设一套假的 process supervisor。

按三个不同口径看 P6R8 结束时的进度。后续 P6R9/P6R10 结果以上文最新结论为准。

| 口径 | 当前判断 | 剩余重点 |
| --- | --- | --- |
| 常用 DOM popup 行为 | 约 99% | 更宽 focused WPT 与通用 child scheduler 长尾 |
| Moli 单进程 popup 终态 | P6R8 当时约 99% | 当时剩余 receiver/entry/incumbent identity 与更宽 focused WPT/CDP，后由 P6R9/P6R10 收口 |
| Chromium 完整基础设施等价 | 不作为当前产品 exit | 真实 process/channel、capability broker、fenced/guest、完整 Reporting/agent reunification |

这里的百分比是风险加权工程估计，不是测试通过率。旧的 94-95% 估计把“process-neutral
schema 已建立”误算成“process lifecycle 已完成”，同时低估了 lightweight 兼容面和 detached lifetime 的删除风险，
现已作废。P6R2 已完成 group-safe opaque-origin identity，P6R3 又完成真实 local Document realm 的
JS-retained lifetime 与无引用 GC 回收；P6R4 随后按 owner 依赖顺序物理删除 compatibility record、realm alias、
mirrored parser/loader/lifecycle 与 protocol fallback。2026-08-23 对 tracked Rust 的
`lightweight_popup|LightweightPopup|lightweight popup` 宽口径扫描已从 112 个文件、1492 处命中降为零。P6R9
复核后，remote descendant lifecycle 与 remote DOM form carrier 已按 Chromium 和 production reachability
重新分类。P6R10 随后完成 receiver/entry/accessing identity 与通用 child dynamic-script scheduler，
并重跑更宽 focused WPT/CDP。剩余外部失败按各自 owner 继续分类。
P6R5 又补上 protocol target scheduler 之外的 production direct `Browser` owner；CLI/WebFetch 不再因没有
output consumer 而留下无人采纳、无人驱动的真实 auxiliary Page。P6R6 随后把 initial-empty creator fallback base、incumbent
source/base、target-local destination queue 和 renderer-authored history seed 接到同一个 Page/commit owner；固定六例
`initial-empty-document/window-open-*` 现在 CLI/CDP 均为 6/6 case、13/13 subtest 通过。
P6R7 又让 direct `Browser` 的 root Page 与 auxiliary Page 消费同一份 cross-document traversal seed，
`history.back()` 和 `history.forward()` 不再停在无人处理的 renderer owner action。旧 standalone observation seam
已在前一笔清理中物理删除，没有作为 history controller 重新引入。
P6R8 随后让 remote top 与 remote child 的 `javascript:` 导航进入 typed wire 和目标 Page/Frame owner。
source Document 先完成 CSP 与 Window access 准入，目标 owner 再按当前 Document identity、origin、
`document.domain`、target CSP 与 Trusted Types 决定是否在目标 main realm 执行。main world、isolated world
和 universal-access isolated world 都保留精确 source identity。被拒绝的同名 remote target 仍算选中，
不会退回新建第二个 popup。
P6R9 又修复 child browsing-context name 被 owner attribute refresh 覆盖的问题，并让 child Location 的
entry base 与 incumbent request source 分别由正确 realm 提供。P6R10 随后补齐 `window.open()` 的
receiver/entry/accessing identity，并让 child Document 动态 inline classic script 在 mutation 返回前同步执行。
外部 `_self` 与四个 multiple-globals WPT 均已转绿。

状态：架构评估与分阶段迁移设计；`popup-refactor` 已完成 Phase 1 primitive 抽取、
Phase 2A script-agent identity / current-policy baseline，以及 Phase 2B 的 selective
shared-agent ownership、V8 foreground routing、核心可行性矩阵和两次 release 长序列
内存验收。Phase 3 第一纵切已经把 renderer-owned auxiliary browsing-context/Page
reservation 和 selective related-agent admission 接入 production initial `about:blank`
target build；第二纵切的首个基础提交又建立了 live Page replacement reservation 和
prepared-document environment reuse，第二个提交已经把 prepared replacement commit、原
stable Page slot 内的 `PageVm`/view publication 和同一个 core `Page` 的 adoption 边界接通；
第三个提交已经让 protocol `Page.navigate` / target navigation 在已有 Page 上使用这份
replacement path，并覆盖 active、background、inactive target 及 Fetch response-stage；
`noopener` 显式使用 fresh agent。第四个提交又把 opener 同步拿到的同一 V8
WindowProxy 交给 related auxiliary Page 的首个 realm，并保留 inherited `about:blank`
的 creator security token。第五个基础提交把 main default realm bootstrap 拆成 callback
内可用的 in-scope prebootstrap 与 callback 后 Inspector materialization，并验证两段之间
Window/Document identity 不变。第六个基础提交又把 related Page admission 从 isolate
holder 内部路由改成 live source Page membership capability，并允许在已经进入的 opener
scope 内重建 target realm 的独立 native bridge bindings、复用缓存的 Inspector backend
handle，全程不回借 holder。第三纵切 D 已经把这些基础接到 production：对保留 opener、
非命名 target 的 initial `about:blank`，`window.open()` 在同步 callback 内创建并暂存真实
auxiliary `PageVm`、独立 realm 和唯一 Document，protocol target 随后采纳同一份 residence，
不再创建或 replay 第二份 initial Document。该窄路径同时继承 creator origin、referrer、
base URL、policy/storage authority，并保留 Classic WebDriver 所需的不可伪造 target identity，
而不恢复 opener 侧 Document owner。Phase 4 第一纵切又把相同的真实 initial Page 扩展到保留
opener、非命名、非 `javascript:` 的 non-empty URL：destination 在 target admission 后只由
auxiliary Page owner 导航一次，opener host 不再启动 mirrored loader。Phase 4 第二纵切又把
destination 提升为携带 exact `TargetPageResidenceIdentity` 的 typed claim，并在 target-local
slot 中建立 `Held → Published → Consumed` authority；旧 admission 因 Page generation 变化而失效
后，不能导航 replacement Page，也不能被 `Page.enable` 等入口从 target URL 重建。Phase 3-4 的
single-owner/navigation 主干现已完成；Phase 5 的 local/Fresh creation policy 已在 E3 exit，
script-closable、beforeunload/unload 与 renderer close ACK/timeout 已在 L1 exit；local focus/active
Page/focused-frame authority 又已在 L2 exit；P6R2/P6R3 已完成当前产品的 local identity/lifetime closure，
P6R4 已物理删除本地 lightweight 双栈；P6R5 已让 direct `Browser`/CLI 复用同一真实 Page owner，P6R6 又完成
initial-empty/no-commit URL、single destination queue 与 protocol history replacement 的固定 WPT closure。P6R7
继续补齐 direct `Browser` root/auxiliary Page 的跨文档 back/forward，并保持 protocol browser history authority。
P6R10 已完成 P6R9 留下的 receiver/entry/accessing identity 与通用 child script scheduler 两项，
并以 81-case CLI/CDP 矩阵重新分类。当前单进程 popup owner 计划的 exit 已满足。remote DOM form carrier
和没有 production producer 的 OOPIF descendant ACK 不属于当前 popup exit，剩余外部失败按 history、
opener、name 与 testharness completion 等责任方继续处理。
Phase 4 第三纵切已经把 HTTP 204/205 收敛为不提交 Document 的独立 terminal，
并覆盖 initial realm/history 保留和后续 redirect replacement。第四纵切又把 replacement
Document 的 commit/attachment publication 与其精确 DCL continuation 分开：protocol 可以在
parser 仍阻塞时控制已经提交的 realm，renderer owner 则用独立 typed terminal 交付同一 turn 的
output fence 与最终 PageState，避免 popup、普通 navigation、Classic/BiDi 和 direct child lane
各自猜测异步完成。第五纵切进一步补齐 Fetch response-stage 的 effective-response terminal：
`fulfillRequest` / `continueResponse` 覆盖后的 204/205 与原始 204 都不提交 Document，而原始 204
被 fulfill 为 200 时仍可正常提交；buffered synthetic body 不再绕过公共 no-commit/download
分类器。第六纵切进一步把普通 main-document pre-response transport failure 收敛成 browser-owned
error Document：请求失败 URL 继续作为 Target/history URL，新 Document 使用
`chrome-error://chromewebdata/` 并通过 `unreachableUrl` 暴露原 URL；stable Page、popup
WindowProxy 和 opener graph 保持，Document/realm 按正常 replacement 边界替换。同步 initial
realm 会把 opener 的数值 viewport surface 安装到最终 target Context，而不是即将 detach 的临时
facade；Page script environment 也会跨 realm 保存实际 opener 值。named target、`noopener`、
`javascript:` URL 和 target admission 前的早期任务在当时仍需后续纵切收敛。Phase 5 第一纵切 A
已经把 child-frame stable WindowProxy 的 V8 access-check/handler primitive 扩展到同一
related-page script agent 中的真实 top-level Page：opener 现在可在跨源 commit 后观察 Chromium
restricted Window whitelist、own property/descriptor/symbol 形状和稳定 identity；跨 Page
`postMessage` 会保存真实 source WindowProxy 与 source origin，`window.location =` /
`location.replace()` 则进入目标 Page 已有的 navigation owner。该纵切同时修正了 shared isolate
中 host-local opaque LocalWindow id 碰撞造成的伪同源，以及 child primitive 的 configurable
descriptor、well-known symbol value 和 `[object Object]` 形状。动态 `closed`、`close()` /
target teardown 的 Phase 5 第二纵切 B 也已接通：related Page 的 same-origin / cross-origin
`close()` 会同步进入唯一 `Closing` 状态，经 target Page 自己的 output FIFO 交给 protocol，最终与
`Target.closeTarget` 共享 Page discard 和 stable WindowProxy closed facade；`open(url); popup.close()`
不会再启动 destination navigation。Phase 5 第三纵切 C 也已经完成 live relation/child projection：
related top-level cross-origin WindowProxy 的 index/name 不再复制到静态 surface，而是从目标 Page 的
child registry 动态解析到既有 stable child WindowProxy；插入、移除、重命名、`then` / `open` named
shadow 和 ownKeys 排除 named child 均有回归。opener 则由 Page-scoped edge 跨 realm 保存；显式
`window.opener = null`、opener 最终 discard 和后续 navigation 使用同一 sever 结果，关闭 popup 自身仍
按 Chromium 保留尚存活的 opener。Phase 5B 当时有意保留的 script-closable policy、
beforeunload/unload 与 renderer ACK/timeout 已由 Phase 5L1 收口；browser-context active Page、focused
frame/element 与 `focus()` transaction 又由 Phase 5L2 收口。Phase 5D 第一纵切 D1 也已收敛 restricted Location internal methods：ownKeys 只保留
`href` / `replace` / `then` 和 3 个 fallback symbol，unknown get/has/descriptor/set/delete/define、
prototype mutation 与 preventExtensions 均按 WPT 形状处理。第二纵切 D2 进一步完成 restricted
Window internal methods：denied/unknown name 和 out-of-range index 不再以伪 own accessor 或
`undefined` 泄漏，delete/define/set、null prototype、extensibility 和 exact ownKeys 顺序均对齐本地
Chromium WPT；named child 可遮住 `document` / `open` / `then`，但不能遮住 `focus` / `close` 等
cross-origin exposed property。D2.5 又把同一 projection owner 扩展到 generic nested child：live
Document 的 get/query/descriptor/enumerator/length 全部直接读取 scoped child registry，同一 Document
内 insert/remove/rename 不再复活 surface snapshot；预物化 stable child WindowProxy facade 也改用唯一
security token 和正式 access surface，不会泄漏调用方 raw global。D3a 进一步完成
`CrossOriginPropertyDescriptorMap` 的 accessing-Realm 侧：Window/Location 的 method、getter、setter
按 incumbent Realm 缓存，具有该 Realm 的 `Function.prototype`、标准 name/length 与 accessor descriptor；
共享 wrapper 的 native callback 则从 receiver 解析真实 target Context/Page owner，避免在 opener host 上执行
popup 的 close、postMessage 或 Location navigation。D3b 又让非 top observer 的 `parent` / `top`
直接复用 stable top-level WindowProxy，并在 index/name lookup 时按 observer Realm 与 target child
origin 决定是否 materialize 同一个 stable child proxy；same-host sibling 与 related Page 跨 host
路径都已有回归。Phase 5E 第一纵切 E1 现已把 production 的非命名、非 `javascript:`
`window.open(..., "noopener|noreferrer")` 以及 hyperlink `_blank` implicit/explicit noopener
切到唯一 Fresh auxiliary Page：调用方返回 `null`，不创建 lightweight browsing context、镜像
Document 或第二 loader；initial empty Document referrer、目标导航的网络 `Referer` 与提交后
`document.referrer` 由 creator policy 一次冻结为三个独立投影。Phase 5E 第二纵切 E2A 又把
opener-preserving、非 `javascript:` 的 named `window.open()` 接到同一真实 initial Page，并在
related-page group 内以 live name/lifecycle registry 解析 stable WindowProxy；reuse activation 携带
精确 renderer Page residence，protocol 的 target-name map 降级为 projection。动态 `window.name`、
navigation 后 name 保留、closed target 排除，以及 existing target 的 noopener/null-return/opener 保留
均由同一 owner 处理。E2B 随后把新建 named noopener/noreferrer `window.open()` 收敛为保留真实 name
的 private Fresh Page；E2C 又把普通 named hyperlink 接入同一 renderer lookup/creation authority：existing
named iframe 仍优先，related Page 精确复用，新建 opener-preserving target 标记 `Related`，新建
noopener/noreferrer target 标记 `FreshNamed` 且不进入 protocol 全局 name projection。E2D 现已在 form
submission owner 中完成同一条 named / `_blank` 纵切：submitter/form/`<base target>` 先确定 effective
target，现有 named iframe 仍优先；related Page hit 或 Related/Fresh miss 与完整 GET/POST request、三类
referrer、Page reservation 和 form-specific target `NavigateEvent` 一起冻结。protocol 的
`Held → Published → Consumed` claim 现在携带 method、raw body、Content-Type 与 request kind，POST
不会再退化成 GET，也不会把 `_blank` 错误导航到 opener。E2E 进一步把 ordinary-name
`window.open()` 与 hyperlink 的 source-subtree / current Page / ordered related Pages 完整 frame-tree
查找接回 renderer owner：related Page 的 nested child 由它自己的 `JsContextHost` 导航，candidate 会执行
普通 origin/ancestor `CanNavigate`，初始 inherited `about:blank` 的 tuple origin 也不再被 URL 重算成 opaque
origin。E2F 已让 ordinary named form 的 exact GET/POST request 消费同一 resolver；E2G 又让 source form
持有 typed Page/child scheduler route 和精确 navigation-load generation，跨 Page retarget 可以取消旧 child
task、loader 与 parser ledger，而不会误取消同一 child 后来发生的无关导航。E2H 进一步把 child
`window.open()`、hyperlink 与 form 命中 current top 时的 initiating Window/Document、source URL、
Referrer Policy 和 suppression 冻结进同一 typed request；target Page 仍是唯一 scheduler/loader owner，
但 redirect、cross-origin 与 Fetch URL override 不再把 target root 误当 initiator，最终
`document.referrer` 也按实际 commit URL 重算。E2I 又完成 sandbox 对**新建** auxiliary context 的
renderer-side admission：attribute / response CSP 的 `allow-popups` 与
`allow-popups-to-escape-sandbox` 已独立建模，`window.open()`、hyperlink、form 和兼容 queue 都在 existing
target lookup 之后、Page reservation / `Page.windowOpen` 之前消费同一 typed policy。被拒绝的
`window.open()` 返回 `null`，不留下 popup activation；escape token 本身不会错误放行。`javascript:` 的完整
target-realm execution 在该阶段仍未完成，现已由 E2N 收口。E2J 随后把准入时冻结的 sandbox frame policy 作为 renderer-owned opaque
carrier 交给 Fresh/no-local-proxy target：显式 noopener `window.open()` 与 implicit noopener hyperlink/form
都由 target Page slot 跨 initial `about:blank` 和后续 Document replacement 持有同一 policy；protocol 只保管
typed value，不解释 sandbox flags。继承 sandbox 的 Fresh Page 现在会在 realm 可观察前安装 opaque origin、
`document.domain` 禁止、script gate 与 nonce-bearing storage key，并与每次 response CSP sandbox 求交集，
`noopener` 仍独立切断 opener/group。E2K 又把 DevTools `userGesture` 与 trusted mouse/key/touch input 收敛为
Page/frame-tree lifetime 的 transient/sticky activation ledger：existing target lookup 不进入 creation gate，sandbox
拒绝不消费，真正 admitted 的 new-context transaction 才执行 browser-context popup-blocker policy 并至多消费一次
精确 generation；`Page.windowOpen.userGesture` 只观察消费前冻结值。Moli 保留自动化默认放行，并提供严格
require-activation policy；完整 top-level `CanNavigate`、`javascript:` target-realm execution、focus
transaction、COOP group sever 与 remote/disconnected endpoint 在该阶段仍未完成。E2L 现已把 `allow-forms` 收回
source-Document form owner：iframe attribute、所有 response-CSP `sandbox` policy 与 inherited popup policy
共同形成 typed `DocumentSandboxPolicy::allows_forms`；`requestSubmit()` 在 validation/`submit` 前拒绝，直接
`submit()` 则保留 `formdata` entry-list construction，随后复核 form connected/current owner policy，再阻止
actual target navigation。它与 `allow-popups` 独立，早门禁也不会误消费 E2K transient activation。Chromium
direct `submit()` 还会在晚门禁前完成 target selection，并可能只留下 initial empty popup。E2L.1 已把这项
实现顺序收进同一 owner：ordinary named / `_blank` 在晚门禁前先选择 existing target 或创建 initial empty
auxiliary Page；被拒绝的新 target 仍完成 popup admission、activation consume、`Page.windowOpen` 与 target
creation，但 renderer-to-protocol carrier 的 destination request 为 `None`。protocol 因此保留 URL 为空的
DevTools target 和唯一 `about:blank` Document，不发布 loader/scheduler work，也不会从空 target URL 反推
一次伪导航。E2M.1 又建立了 renderer-owned **local `CanNavigate` authority**：current/related Page 的
top/child、兼容 lightweight endpoint 统一消费 committed sandbox navigation flags、top-navigation token、
policy-container sticky guard、transient/sticky activation、target/ancestor origin、typed opener relation 与
destination origin/site exception；ordinary named resolver、special target、form、same/cross-origin Location 均不再
各自猜测权限。跨源相对 Location URL 也以 source execution-context identity 的 Document base 解析，policy 与
真正排队的 request 使用同一绝对 URL。RemoteFrame/fenced/embedder 分支、file-local 特例、security console
diagnostic 仍是后续阶段。E2N 进一步把 full-creator `window.open()`、hyperlink 与 form 的
`javascript:` request 放进最终选中/新建的真实 target Page pending slot：target selection/creation 与 stable
WindowProxy 返回保持同步，执行则严格推迟到 target Page 的 networking-task owner turn。target Document identity、
source carrier、target CSP、异常/non-string/string completion、执行中另起 navigation、普通 navigation supersession
与 Document replacement cancellation 都由同一个 Page/Document currentness 边界判定；protocol activation 对
`javascript:` 只观察 target create/reuse，不再发布第二个 destination navigation。Fresh/noopener target 仍会创建
唯一 initial Page，但不会在无 opener 且 opaque 的跨源 realm 中错误执行 creator 提供的脚本。
E2O 在这份 single-owner 基础上补齐 policy 收尾：最终 target Page 或 stable child FrameRealm 都按
`target CSP → target Trusted Types pre-navigation check → execution` 处理 JavaScript URL；enforce/report-only、
default policy rewrite、invalid reconstructed URL 和 callback exception 均不向调度 API 同步抛错。producer 侧则
区分 existing hit 与 creation miss：existing target 先解析 stable WindowProxy，再只做 source inline-navigation
CSP；miss 才在创建前做 source CSP + Trusted Types gate。form submission 同时接入无 `default-src` fallback 的
`form-action`，并保持 target selection/new initial Page、late `allow-forms`、source CSP、`form-action` 与 target
scheduler 的 Chromium 顺序。实现直接复用已经成熟的 child-frame stable WindowProxy/realm 与 E2N target Page
task，而没有引入第三套 policy executor。
Phase 5E3 现已完成 local/Fresh creation-policy 收口：Service Worker
`clients.openWindow()` 与 notification navigation 不再借当前 root Window 伪造 source，也不再调用
lightweight popup owner；两者以 browser-context source 创建无 opener 的 Fresh auxiliary Page，并与 DOM
producer 共用同一 target admission/navigation pipeline。`clients.openWindow()` 的 Promise continuation 绑定
exact reserved Page、worker version/generation 与 request id，跨 Fetch fulfill、redirect、transport terminal 和
background commit 保持 move-only authority，只在 exact committed/current/execution-ready/same-origin
WindowClient 上返回对象，其余已打开但不可观察或未提交路径返回 `null`。同一 source turn 的 ordinary target
navigation 与后继 `javascript:` target task 即使分裂为多个 renderer publication，也会在普通导航 start 后、
commit 前经过 exact target Page owner lane；fast response 不再抢先替换执行该 task 的 Document。至此本地
DOM 与非 DOM production creation producer 都以真实 Page 为权威 owner。Phase 5L1 随后把 `OpenedByDOM`、
history/浏览器设置 script-close gate、root→local descendants `beforeunload`、一次 confirmation、
`pagehide`/`unload`、network-drained barrier、renderer unload ACK/timeout 和最终 target teardown 收进同一个
exact Page transaction。Phase 5L2 随后建立跨 Document 保留的 Page focus state、focused-frame ancestry、
native `document.hasFocus()`、CSS/event transition，并让 `Window.focus()`、`Target.activateTarget`、
`Page.bringToFront`、focus emulation、window state、activated target creation 与 close promotion 共用 exact
Page activation transaction；active-target 与 effective-focus 是两个独立 Page 位，因此 parked Page 即使被
focus emulation 投影为 focused，`window.focus()` 仍会请求真正 promotion。Phase 5G1 又完成本地
committed-response COOP group sever：目标 Page/Target/session 和 Page scheduler identity 保持，命中 Chromium
swap matrix 时则在 commit 前预留新 browsing-context group、script agent、isolate、realm/WindowProxy 与
renderer output stream；取消 preparation 不伤旧 group，成功 commit 才 sever opener/name/old membership，并把
旧 group 持有的 WindowProxy 停驻为 disconnected/closed facade。Phase 5G2 随后把每个 redirect response 与
terminal response 收进一个 navigation-owned COOP status：enforced mismatch 跨 hop 累积、report-only virtual
group 独立推进、Reporting endpoint 在 response source 有效期内解析并生成报告请求，最终 state 穿过普通 body、
Fetch response override 和 transport-error Document 进入唯一 commit。G2 同时以 exact output-owner reservation
修复同一 Page 上旧 provisional release 清除新 generation owner 的竞态，并让 Fetch fulfill 使用 redirect 后的
authoritative final URL。Phase 5G3 又完成 Chromium `SanitizeResponse()` 的 sandbox/COOP blocked terminal：
renderer-owned sanitizer 合并 inherited auxiliary sandbox 与当前有效 response CSP，在任何 redirect follow 或
Document preparation 前拒绝非 `unsafe-none` enforced COOP；普通 streaming/captured response 与 Fetch effective
response 都转入同一个 `ERR_BLOCKED_BY_RESPONSE` browser-owned error Document。阻断会在原 target/session 内强制
real+virtual group switch、sever 旧 opener/proxy，且 redirect 目标不会产生第二次网络请求；前一个 Document
自己的 response CSP sandbox 不会错误污染后续 navigation。真正 remote endpoint、RemoteFrame/fenced/embedder、
identity/lifetime 和 Phase 6 compatibility 双栈删除在当时仍是独立大阶段；前者现已由 P6R2/P6R3 完成当前产品
local exit。Phase 5G4 随后把旧 proxy 从“private slot
直接保存目标 V8 object，再由 creation context 反查 host”迁到 group-qualified typed endpoint：每个 related
top-level target 由 browsing-context group 分配非零 generation，普通 Document replacement 保留 `(group,
generation)`，COOP group switch 分配新 pair。`postMessage`、cross-origin Location、`close()`、`focus()` 与 child/name
projection 都必须先由 source group registry 解析 exact active endpoint；stale/closing/disconnected endpoint 统一
表现为 `closed=true`、`length=0` 并丢弃 routed operation；`opener` 继续服从 relation policy（COOP sever 为
`null`，普通 final close 保留已建立的 opener edge），尤其不再把失败的 `postMessage` 解析回退到 incumbent
opener。Phase 5G5 随后完成 **same-group、cross-agent top-level RemoteWindowProxy** 纵切：logical target state 与
agent-local V8 projection 分离，跨源 related Page commit 到 fresh isolate/script agent 时保留 Page、group、endpoint、
name、opener 与旧 agent 中的 stable proxy；旧 projection 转成 live remote facade，新 agent materialize 自己的
LocalWindow 与 opener facade。`postMessage`、Location assign/replace、focus、close 通过 typed renderer output 进入
protocol exact background target/Page，等待 target ACK 并在执行前后复核 loaded Page generation 与 endpoint。
named `window.open`/hyperlink/form 也能命中 remote top-level，不再因为目标不在 source isolate 而创建第二个 popup。
Phase 5G6A 随后把同一边界扩展到 **remote nested browsing context**：目标 Page 发布不含 host/V8 handle 的
Document-qualified frame tree；source agent 按 endpoint/root Document/frame id 建立 stable RemoteFrame proxy，并让
动态 `length`、index/name、parent/top、Location、postMessage、named hyperlink/window.open/form 进入 exact target
Frame owner。form POST 的 method/body/header/referrer 与 source-assigned scheduler id 一起跨 Page，A→B retarget 会先在
A owner 上精确取消原 loader/parser generation；root Document replacement 后旧 child proxy 立即 disconnected，不能
凭复用的数字 frame id 串到新 Document。protocol remote command 又增加有限 ACK deadline，target Page crash/拆除后
retained proxy 安全 no-op。G6A 完成时仍不是跨 OS process 的完整 RemoteFrame，carrier 当时还包含进程内 Rust/V8
capability。Phase 5G6B1 现已把这份 carrier 改成 strict v1 process-neutral wire：command只保存 validated route 与
encoded bytes，remote-frame tree按 monotonic revision逐 snapshot编码并做完整 topology校验，structured clone显式
拥有 ArrayBuffer/port/stream/Blob/File attachment。logical endpoint之外又增加 execution-channel generation，只有
committed same-group agent replacement才旋转；ACK waiter被丢弃时可在 target actor admission前取消 queued command。
跨 agent Wasm按 Chromium派发 `messageerror`而不传递 compiled module；FileSystemHandle/OPFS File则因尚无 browser
broker在 transfer side effect前拒绝。真实 process spawn/channel/crash/restart/rebind 仍未实现；只有项目另行采用
多进程 renderer 时才进入独立 G6B2，不把这一 wire seam 冒充多进程完成，也不把多进程建设当作当前 popup 删除前置。

代码基线：

- Moli 原始评估：`2e351a545b04`，分支 `cdp-better-4784u`；实施状态以
  `popup-refactor` 当前分支为准。
- 2026-08-22 rebase/revisit merge-base：`origin/master@228330684f`；最终门禁结果记录在 G6B1
  复核证据中。
- Chromium：`a03603fe9af6`，本地 checkout `/home/donoughliu/chromium/src`。

本文讨论 HTML `window.open()`、`target=_blank` 和命名 target 创建的 auxiliary
top-level browsing context。它不是 Blink 用于 `<select>`、权限气泡等 UI 的
`PagePopup` 机制。

## 结论

原始评估基线中的 popup 不是“缺几个 API”，而是同时存在两套相互独立的实现：

1. opener `PageVM` 内的 `LightweightPopupBrowsingContextRecord` 提供同步返回给 JS
   的 Window-like facade，并自行加载、解析和执行 popup 文档；
2. protocol 收到 renderer output 后再创建一个独立 popup target、独立 `PageVM`
   和独立 document isolate，并再次导航同一个 URL。

两条路径各自已有不少能力，但它们不是同一个 browsing context。当前测试甚至把
“popup URL 恰好有两个 load owner”写成了预期。这会导致网络副作用重复、opener
拿到的 Window 与 CDP target 看到不同 DOM、`close()` 与 target 生命周期分裂，继续
补 facade 只会扩大同步成本。

`popup-refactor` 当前已经为 opener-preserving 的非命名/普通 named `window.open()`、ordinary named hyperlink
和 full-creator form auxiliary target 建立 production 迁移路径；其中 ordinary destination 与
`javascript:` 都由最终 target Page 持有。creator 与 target 共享同一
stable WindowProxy、initial realm、Document 和 Page residence；non-empty destination 在 target admission
后从该 Page 发起一次 replacement navigation，opener host 不再保存对应 lightweight Document record 或
启动 mirrored loader。`javascript:` 的 target 选择/创建同步完成，脚本在该 target 的 Page task 上异步执行，
不会作为 protocol/browser navigation 再跑一遍。非命名及普通 named 的 `noopener` / `noreferrer` 创建路径与 hyperlink/form
`_blank` 也已经进入独立 Fresh Page 的 single-owner 路径，只是不向 creator 暴露 local WindowProxy；
form POST 的 exact body/header 则沿同一 target-owned navigation claim 发出。Phase 5E3 又把 Service Worker
`clients.openWindow()` 与 notification navigation 接入无 opener 的 Fresh Page，并补齐 exact Promise terminal
与 ordinary→`javascript:` protocol ordering。上述双实现判断现在只适用于 compatibility/standalone fallback、
legacy lightweight facade 及尚未建立 remote endpoint 的入口，不再适用于 production SW/notification producer；
因此 lightweight 模型仍是 Phase 6 的主要删除对象，而不是可以继续扩展的长期架构。

建议采纳下面的方向：

- 复用 child-frame 已经成熟的 stable `WindowProxy`、可替换 `LocalWindow` /
  `Document` generation、独立 realm、security token、跨源 facade 和 typed realm
  materialization 基础；
- 复用方式是把这些能力抽成通用 browsing-context primitive，而不是把 popup
  伪装成 iframe；
- 每个 auxiliary top-level context 仍拥有独立 Page runtime、history、task queue 和
  CDP target；
- 需要同步脚本关系的 opener / popup 共享一个可承载多个 Page realm 的 script
  agent（第一版可以是共享 V8 isolate），而不是共享同一个 V8 `Context`；
- renderer 创建的那个真实 auxiliary Page 必须被 protocol target 直接绑定，protocol
  不再创建第二个 Page、第二次导航或镜像文档；
- `noopener`、COOP group switch 和未来需要进程/agent 隔离的路径通过 remote
  WindowProxy endpoint 或独立 script agent 表达。

在开始主迁移前，必须先完成 Phase 2B 小型可行性实验：证明当前“每个并发 Page
script environment 一个 isolate”可以选择性演化为“每 script agent 一个 isolate、
每 Page/Document 一个 V8 Context”，同时不破坏 CDP object id、inspector context、
page-local task/event routing、关闭语义和内存 containment。

## 术语与必须分开的层次

本文用以下术语，避免把不同层级合并成一个“popup object”：

| 概念 | 本文含义 | 典型生命周期 |
|---|---|---|
| browsing context | 一个可导航上下文；popup 是 auxiliary top-level context，iframe 是 nested context | 可跨多次 navigation |
| WindowProxy | 调用方持有的稳定外壳，按当前 origin 和当前 inner Window 转发或拒绝属性访问 | browsing context 生命周期 |
| LocalWindow | 某个已提交 Document 的 inner Window owner | 通常随 cross-document navigation 替换 |
| realm | 一个 V8 `Context` 及其 global lexical environment / intrinsics | 通常随 LocalWindow / Document generation 替换 |
| script agent | 能让 V8 object 同步互相引用的执行宿主；拟议实现可拥有一个 isolate | 可承载多个相关 Page/realm |
| Page runtime / `PageVM` | 一个 top-level Page 的任务、导航、文档和协议执行责任方 | top-level context 生命周期 |
| browsing-context group | related pages、命名 target 查找和 opener 关系的边界 | 可因 `noopener` / COOP 分裂 |
| CDP target | 对同一个 Page runtime 的协议身份和 session 路由 | Page 可观察生命周期 |

关键点是：

- “共享 isolate”不等于“共享 realm”。两个 same-origin Window 应有不同 V8
  `Context`、不同全局 lexical state 和不同 platform singleton identity；它们只是
  可以通过 stable WindowProxy 同步互访。
- “同一个 browser context”也不等于“同一个 browsing-context group”或“同一个
  isolate”。browser context 主要承载 profile/storage/permission 隔离。
- “独立 CDP target”不要求再创建一份 renderer 文档。target 是观察和控制 Page 的
  协议身份，不是第二个页面副本。

## 评估方法与证据边界

本次评估直接阅读了 Moli 和本地 Chromium 源码，并运行了聚焦 nextest。
关键入口如下。

Moli popup：

- `moli-renderer-v8/src/context_bootstrap/window_runtime/dialogs.rs`
  - `window_open_callback`
- `moli-renderer-v8/src/native_bridge/context_host/popups.rs`
  - `LightweightPopupBrowsingContextRecord`
  - `create_lightweight_popup_window`
  - `commit_lightweight_popup_document`
  - `execute_lightweight_popup_document_scripts`
- `moli-protocol/src/domains/page/popup.rs`
- `moli-protocol/src/domains/target/lifecycle.rs`
  - `PopupTargetCreation`
  - `ensure_popup_initial_document_page_async`
- `moli-protocol/src/domains/target/tests/tests_target_creation.rs`
  - `window_open_hands_off_session_storage_snapshot_and_initial_storage_key`

Moli child-frame / realm：

- `moli-renderer-v8/src/frame_owner_model.rs`
- `moli-renderer-v8/src/frame_owner_model/records.rs`
- `moli-renderer-v8/src/frame_owner_model/store.rs`
- `moli-renderer-v8/src/native_bridge/context_host/child_frame_runtime/window.rs`
- `moli-renderer-v8/src/native_bridge/context_host/child_frame_runtime/isolated_world.rs`
- `moli-renderer-v8/src/script_vm/child_frame_realm_materialization.rs`
- `moli-renderer-v8/src/script_vm/post_parse.rs`
- `moli-renderer-v8/src/native_bridge/context_host/window_execution_context/`
- `moli-renderer-v8/src/native_bridge/context_host/window_security_tokens.rs`

Chromium：

- `third_party/blink/renderer/core/frame/local_dom_window.cc`
  - `LocalDOMWindow::open`
- `third_party/blink/renderer/core/page/frame_tree.cc`
  - `FrameTree::FindOrCreateFrameForNavigation`
  - `FrameTree::FindFrameForNavigationInternal`
- `third_party/blink/renderer/core/page/create_window.cc`
  - `CreateNewWindow`
- `third_party/blink/renderer/bindings/core/v8/window_proxy.h`
- `third_party/blink/renderer/bindings/core/v8/local_window_proxy.*`
- `third_party/blink/renderer/bindings/core/v8/remote_window_proxy.*`
- `content/browser/renderer_host/render_frame_host_impl.cc`
  - `RenderFrameHostImpl::CreateNewWindow`
- `content/browser/web_contents/web_contents_impl.cc`
  - `WebContentsImpl::CreateNewWindow`
- `content/common/frame.mojom`
  - `CreateNewWindowParams` / `CreateNewWindowReply`

没有在本次文档工作中重新编译 Chromium，也没有重新跑 WPT。Chromium 结论是源码
对照；WPT 数字来自仓库当前已提交的 case list，只能作为风险信号，不能当作新鲜的
回归结果。

## Moli 当前实现

下面 1-4 节保留原始架构评估，便于说明迁移为什么必要；其后 Phase 3 实施记录是当前分支
状态的增量事实。尤其是第三纵切 D 已经替换了窄 initial `about:blank` 路径，不能再把该路径
计入“两个 Document / 两个 Page owner”。

### 1. `window.open()` 同步路径

`window_open_callback` 已经处理了不少正确的前置语义：

- 使用 entered Window / creator Document 解析 URL；
- 非法 URL 抛错；
- 解析 window features；
- 识别 `_self`、`_parent`、`_top` 和 `_blank`；
- 处理 `noopener` / `noreferrer` 和 anchor `_blank` 的 implicit noopener；
- 尝试复用命名 lightweight popup；
- 新 popup 返回一个稳定的 synthetic Window shell，或在 opener 被抑制时返回
  `null`；
- 生成 renderer output，供 protocol 后续创建/复用 target。

这些行为让常见页面不至于在 `const w = window.open(...)` 处立即失败，也为
`Page.windowOpen`、target auto-attach 和 session storage handoff 提供了输入。

### 2. opener 内的 lightweight popup

`create_lightweight_popup_window` 在 opener 当前 V8 context 中通过
`instantiate_window_shell` 创建 Window-like object，并把它存入
`LightweightPopupBrowsingContextRecord`。record 已包含很多浏览器状的状态：

- stable Window shell / popup id / name / opener endpoint；
- initial `about:blank` Document、LocalWindow id 和后续 Document generation；
- location、history、navigation projection；
- local/session storage、storage key 和 session snapshot；
- timer、message、fetch/XHR、worker、CSP 与部分 lifecycle 状态；
- 非 `about:blank` URL 的 renderer-local load、parser 和 script execution。

但这里的 LocalWindow id 是 Moli owner identity，不代表出现了新的 V8 realm。
源码已经明确记录：lightweight popup 仍共享 opener 的 concrete V8 context；它在
`WindowExecutionContextRealmRecords` 中只能作为 scoped alias，不能注册成另一个
concrete realm。`document.domain` 更新 security token 时也必须跳过 popup，否则会
错误修改 opener context 的 token。

popup 文档脚本还走一条专用执行路径：扫描 DOM 中的 script，并用包含
`with (__scope) { with (window) { ... } }` 的 wrapper 模拟 popup global。它可以覆盖
不少脚本，但无法等价表达独立 global lexical environment、intrinsics、模块图、
inspector context、跨 realm wrapper 和完整 WindowProxy security semantics。

### 3. protocol 创建的真实 target

renderer output 到达 `moli-protocol` 后，`PopupTargetCreation` 会：

- 分配 target/session 相关身份；
- 创建 auxiliary/background target；
- 确保 popup initial Document `Page` 存在；
- 建立独立 `PageVM` 和 document isolate；
- 对请求 URL 再执行一次真实 target navigation；
- 接入 `Target.setAutoAttach`、`waitForDebuggerOnStart`、Fetch/Network 和 Runtime
  evaluate 等 CDP 路由。

这个 target 对 Playwright/CDP 来说是“真的”，但它与 opener 返回的 lightweight
Window 不是同一个 Page。

### 4. 当前实际拓扑

```text
opener target / opener PageVM / opener isolate
    |
    | window.open()
    +--> LightweightPopupBrowsingContextRecord
    |       +-- synthetic Window shell（共享 opener V8 Context）
    |       +-- mirrored loader/parser/script/lifecycle
    |
    +--> frozen renderer popup output
            |
            v
       protocol target creation
            +-- popup target
            +-- popup PageVM
            +-- popup document isolate
            +-- second navigation/load
```

`window_open_hands_off_session_storage_snapshot_and_initial_storage_key` 的测试注释明确
把第一个请求称为 opener lightweight facade 的 mirrored load，把第二个请求称为
真实 auxiliary target navigation，并断言 popup URL “must have exactly two load
owners”。这不是原始评估时的推测，而是 Phase 4 第一纵切之前的入库合同；同一测试现已
反转为“exactly one authoritative navigation owner”，当前实现状态见 Phase 4 第一纵切 A。

### 5. 已经可用的能力

当前实现不应被概括成“完全没做”。已存在的有价值能力包括：

- 常见 `window.open` 参数、features、invalid URL 和 special target 处理；
- named popup 的部分复用和 `targetInfoChanged`；
- initial `about:blank`、history、204/205 不替换文档等局部语义；
- opener policy、`noopener` / `noreferrer`、implicit noopener；
- session storage snapshot、storage key、browser-context storage partition；
- renderer-local popup 的 timer、message、fetch/XHR、worker、CSP 和部分 child blocker；
- CDP target create/attach/auto-attach/wait-for-debugger；
- popup target 的 Fetch/Network interception、Runtime evaluate、browser context 和 dialog
  路由；
- 跨 root/child/lightweight source 的 frozen popup activation identity，避免 protocol
  消费时回查已经变化的 current source。

这些能力应迁移到统一 owner，而不是删除后重写。

### 6. 核心缺口

| 语义 | 当前状态 | 直接后果 |
|---|---|---|
| authoritative Page | related、Fresh、SW `clients.openWindow()`、notification、direct `Browser` 与曾经的 compatibility fixture 都已统一到真实 Page；P6R4 已删除 lightweight owner，P6R5 又补齐 direct owner wake | 当前没有第二 Page 风险；P6R10 已补 receiver/entry realm 证据并重跑 81-case CLI/CDP 矩阵 |
| navigation owner | DOM、非 DOM、protocol 与 direct `Browser` producer 的一个 URL 只有一个 Page/Frame scheduler owner；P6R6/P6R7 已补 initial target queue 与 direct history traversal；P6R9 又让 HTTP(S) child Location 保存 typed source request；P6R10 让 child inline classic script 在 mutation turn 内同步执行 | remote method/body/header/referrer/scheduler 已完整；redirect response policy 仍属于独立 navigation 长尾 |
| top-level initiator / referrer | E2H 已让 current-top `window.open()`、hyperlink/form 与 related target request 保存 exact source Window/Document 和 policy；E2N 的 target-local `javascript:` task 也保留这份 source carrier；preflight、transport redirect/Fetch URL override、最终 `document.referrer` 不再从 target root 反推 | redirect response 自身更新 policy、完整 Fetch response-stage override 仍需独立收口 |
| realm | related、Fresh 与非 DOM target 都使用真实独立 target realm；P6R3 已闭合 detached realm lifetime，P6R4 已删除共享 opener `Context` 的 alias；P6R8 又让 remote `javascript:` 固定在目标 main realm 执行 | 当前没有第二套 popup realm；未来真实 renderer process capability lifetime 仍属可选基础设施 |
| synchronous access | related path 直接访问 target 的真实 Document；E2N 同步返回 stable target WindowProxy、异步执行 target task；G5/G6 对 remote endpoint 使用 observer-local proxy 和 typed command；P6R10 已冻结 `window.open()` receiver/entry/accessing identity，并同步执行 child dynamic inline classic script | 当前单进程 exit 已满足；可选 process death 不计入当前 exit |
| cross-origin WindowProxy | related local Page 已复用 stable outer proxy 与 per-Realm restricted surface；G4 由 browsing-context group 分配 typed endpoint generation，G5 再分离 logical target 与 per-agent projection并完成 remote top-level typed command/ACK；G6A 已复制 Document-qualified remote child tree，在 observer agent 建立 stable nested proxy，并把 dynamic child projection、Location、postMessage、named target 和 exact frame scheduler 路由到 target Page/Frame；G6B1 又把 command/frame policy/clone carrier编码为 strict versioned bytes，并增加 execution-channel generation和 queued cancellation。G1 COOP old proxy、root replacement 后 child proxy与 close 后 facade 都按 typed currentness 断连 | process-neutral seam 已走通；真正 renderer IPC/channel/process death/restart、browser capability broker、agent reunification、fenced/embedder 完整 policy/lifecycle replication 仍缺失 |
| `window.close()` | 真实 auxiliary Page 已由 Phase 5B/L1 统一 script-closable、subtree beforeunload、network drain、pagehide/unload、renderer ACK/timeout 和最终 teardown；Fresh noopener 不向 opener 暴露 close handle；P6R4 已删除 compatibility close owner | 当前 target Page 已拥有本地 descendant 事务；实际 OOPIF descendant ACK 留给可选多进程基础设施 |
| focus/blur | Phase 5L2 已建立彼此独立的 browser-context active-target/effective-focus Page 位与 renderer focused-frame authority；`Window.focus()` 保留 transient-activation/opener admission，跨 Page owner action 冻结 exact renderer Page，target activation、focus emulation、window minimize/restore、activated creation 与 close promotion 都更新 native `document.hasFocus()`、CSS `:focus` / `:focus-within` 和 Chromium event order；modal prompt 阻塞 owner lane 时，browser activation 不等待旧 Window，exact Page focus/surface command 留在 owner FIFO；top-level `blur()` 保持 metric-only no-op；G1 的 local agent switch 保留同一 Page active/focused state | RemoteFrame/COOP 的 cross-process focus endpoint 与 embedder activation 尚未建模 |
| named target | E2A-E2D 已统一 `window.open()`、full-creator hyperlink/form 的 related lookup、Fresh group split 和 exact Page handoff；E2E 已补 child-source、related nested local frame-tree order；E2F/E2G 已让 form 消费 typed target，并跨 Page 保存 cancellable scheduler generation；E2M.1 已让 current/related local candidate 统一经过 renderer `CanNavigate` authority；G1 在 COOP commit 时从旧 related registry 注销 target；G6A 又按 related Page/frame document order 复制 remote nested name，并让 window.open/hyperlink/form 命中 exact frame token，remote same-form A→B 可取消 A 的精确 loader/parser generation；G6B1 让整棵 remote tree按 monotonic revision经 strict wire/topology校验；P6R8 保留 denied remote name selection，不再回退新建 Page | ordinary related RemoteFrame lookup与进程中立 revision seam 已建模；fenced/guest/embedder fallback 与真实 process currentness 仍属后续可选基础设施 |
| opener / COOP | G1 已持久化 typed browsing-context-group/committed COOP state并完成 local sever；G2 又逐 hop消费 redirect + terminal response，累积 enforced swap、推进 report-only virtual group、解析 Reporting endpoint并发出 navigation reports；G3 已在 redirect follow/final effective response前执行 sandbox sanitation并强制 real+virtual sever；G4 统一旧 endpoint currentness；G5 再让 same-group跨 agent replacement复制 opener endpoint并建立 canonical projection；G6A 让 remote child source/target也依附同一 top endpoint与 root Document generation；G6B1 新增 current execution-channel generation，只有 committed agent transition旋转，canceled preparation保持不变 | 完整 Reporting queue/source 隔离、真实 renderer process/channel death/restart、protocol group projection、agent reunification和 fenced/guest隔离仍未完成 |
| popup blocker / user activation | E2K 已建立 renderer-owned 5s transient + sticky ledger、exact generation consume、existing-target bypass、sandbox-before-blocker order、pre-consumption `Page.windowOpen` observation 和 browser-context allow/require policy；E2M.1 已把 transient/sticky 状态接入 local top-navigation decision；DevTools gesture 与 trusted mouse/key/touch 共用 owner；G6A 的 remote child navigation admission 读取 source committed ledger/policy，但不伪造 target activation | 尚无 content-setting/CDP 配置面与 blocked console/UI diagnostic；跨进程 per-frame activation/visibility、focus transfer 和 history activation 长尾未完成 |
| sandbox | E2I 已统一 attribute/response-CSP `allow-popups` 新建准入，并区分 escape 只控制继承；E2J 又让 Fresh/no-local-proxy target Page 跨 initial 与后续 Document 持有 renderer-frozen policy；E2L/L.1 已补 source `allow-forms` 和 creation-only side effect；E2M.1 已提交并冻结 navigation/top-navigation flags、frame-owner token provenance 与 Chromium `can_navigate_top_without_user_gesture` 等价 guard，并用于 local target selection/navigation；G3 已补 response sanitation；G6A 又复制 related remote frame 的 committed sandbox/origin/document.domain facts，支持 target/ancestor access 与 sandboxed-ancestor refusal；G6B1 将完整当前 `DocumentPolicyContainer` 投影编码进 revisioned strict wire | security console/Audits diagnostic、完整 RemoteFrame top/opener exceptions、fenced/embedder 与 file-local 特例仍缺失 |
| initial empty Document | related、Fresh 与 direct `Browser` target 都只由目标 Page 创建一份；P6R6 已补 creator fallback base、no-commit URL 与 target-local destination queue | focused WPT/CDP 仍需扩大，owner 已唯一 |
| script loader / JavaScript URL task | ordinary URL、form POST 与 E3 SW/notification destination 都只使用 selected target Page 的唯一 loader；E2N/E2O 已完成 local target task、CSP、Trusted Types 与 form-action；G6A 完成 remote child ordinary scheduler；P6R8 又让 remote top/child `javascript:` 使用 typed source carrier，并在当前目标 main realm 执行；P6R10 删除旧 async DynamicClassic stack并把 child inline script 接入同步 mutation candidate | Chromium 不跨 remote wire 传 DOM source element 或 V8 `FormData`；redirect-time browser policy 与剩余外部用例按独立 owner 处理 |

P6R4 已按这张表的 owner 依赖顺序删除 lightweight 路径。后续工作只扩展真实 Page、Frame、
endpoint 和 lifecycle owner，不能再恢复 popup 专用的 loader、realm 或 observation seam。

### 7. 当前与历史 WPT 风险快照

P6R10 已按同一份 81-case 清单重跑 release CLI/CDP。最终结果分别为
40 pass / 26 fail / 15 timeout 和 40 pass / 26 fail / 15 harness-stalled，归一化后的逐 case
status 完全一致。四个 multiple-globals focused cases 在两条入口均为 4/4 pass。以下静态关键字清单
保留为早期历史记录，不能覆盖 P6R10 的当前结果。

对 `moli-benchmark/wpt-cross-current/{passed,failed,timeout}-cases.txt` 使用下面的
粗粒度关键字切片：

```text
window-open|window_open|browsing-context-names|noopener|noreferrer|opener|auxiliary
```

当前清单的静态关键字计数为：

| 状态 | case 数 |
|---|---:|
| pass | 30 |
| fail | 12 |
| timeout | 28 |

这些 case list 的采集早于最近 E2A-E2O，不是当前 `popup-refactor` 的运行结果，也不能作为新 owner/scheduler
路径的验收证据。E2I 已固定 commit/binary 跑过三例 focused slice，结果和限制见对应实施章节；完整 popup
目标集仍需固定 timeout、并发度与正确性输出后重新分类。在此之前，下面的历史 case 只能用于选择 focused
slice。

代表性风险包括：

- `multiple-globals/context-for-window-open.html` timeout；
- `windows/auxiliary-browsing-contexts/opener*.html` 多项 timeout；
- `browsing-context-names/choose-existing-001.html` timeout；
- `initial-empty-document/window-open-204-pushState-replaceState.html` fail；
- `the-window-object/window-open-noreferrer.html` fail。

也已有明确通过项，例如 initial-empty 204 fragment、部分 feature tokenization、
`noreferrer-null-opener` 和 anchor implicit noopener。这个分布与“表面和协议能力已有，
多 global / opener / named-context 生命周期仍不完整”的代码结论一致。

## Chromium 的责任链

Chromium 的具体类很多，但对 Moli 最重要的不是复制多进程结构，而是保持同一
条语义责任链。

### 1. Blink 完成调用方语义和 target 选择

`LocalDOMWindow::open`：

- 从 entered Window 完成 URL；
- 解析 window features、referrer、user gesture 和 attribution；
- 通过 `FrameTree::FindOrCreateFrameForNavigation` 选择 special/named target 或创建
  新 auxiliary context；
- 对得到的那个 frame 发起导航；
- special target 保持返回现有 Window；普通 `noopener` 新窗口返回 `null`；
- existing named target 在适用时更新 opener 并返回该 context 的 `DOMWindow`。

`FrameTree::FindFrameForNavigationInternal` 的查找范围依次包含：

- `_self` / `_current`、`_top`、`_parent`、`_blank` 等关键字；
- 当前 frame subtree；
- 当前 Page 的完整 frame tree；
- `Page::RelatedPages()` 中其它 Page 的 frame tree；
- embedder fallback。

所以命名 target 不是某个 `Window` object 的局部 map，而是 related browsing contexts
上的查找和 `CanNavigate` policy。

### 2. Blink 与 browser process 共同创建一个真实 Page

`blink::CreateNewWindow` 设置 auxiliary frame type，检查 dismissal、URL/security、
sandbox popup flags，分配 session storage namespace，并调用 `ChromeClient::CreateWindow`。
保留 opener 的路径会 clone session storage；`noopener` 路径不沿用这份 clone。

`RenderFrameHostImpl::CreateNewWindow` 在 browser process 中统一处理：

- popup blocker / embedder policy；
- transient user activation 判断与消耗；
- storage namespace；
- credentialless、fenced frame、COOP 导致的 opener suppression；
- virtual browsing-context group；
- 是否建立新的 `BrowsingInstance`；
- initial empty Document policy/COOP reporter；
- DevTools `wait_for_debugger`；
- frame、widget、interface 和 document token。

`WebContentsImpl::CreateNewWindow` 创建实际的新 `WebContents` / `Page` / `FrameTree`。
保留脚本关系的 popup 与 source `SiteInstance`/`BrowsingInstance` 协作；opener suppressed
或禁止 JS access 的路径进入新的 BrowsingInstance，source renderer 不拿到可访问 handle。

Moli 不需要照搬 UI thread、widget、SiteInstance 或多进程 IPC，但需要保留：

- 创建 policy 只有一个最终裁决；
- initial empty context 先真实存在；
- 之后的 navigation 只作用于该 context；
- DevTools target 观察同一个 Page；
- opener suppression 改变返回 handle 和 group/agent 关系，而不是创建一份镜像。

### 3. stable outer WindowProxy + replaceable inner global

`window_proxy.h` 对 split Window model 的注释非常直接：

- outer global proxy 跨 navigation 复用；
- 每个 Document 通常对应新的 inner global object；
- initial empty Document 到 same-origin 首次 commit 是允许复用 inner global 的唯一特殊
  情况；
- same-origin access 转发到当前 inner global；
- cross-origin access 进入 outer proxy interceptors；
- local frame 使用 `LocalWindowProxy`，跨进程 frame 使用 `RemoteWindowProxy`。

这正是 Moli child-frame 已经开始实现、而 lightweight popup 绕开的基础。

### 4. Chromium 与当前 Moli 的对比

| 维度 | Chromium | 当前 Moli | 目标 |
|---|---|---|---|
| 新窗口实体 | 一个真实 Page/FrameTree | lightweight record + target Page 两份 | 一个 auxiliary Page runtime |
| JS 返回值 | 指向真实 context 的 WindowProxy，或 `null` | 指向 opener 内 facade | 指向真实 auxiliary context 的 stable proxy |
| initial empty Document | 新 Page 的真实初始文档 | facade 与 target 各一份 | 同一个真实初始文档 |
| navigation | 对选中的 frame/Page 导航一次 | mirror load + target navigation | 一个 navigation token、一个 loader owner |
| realm | Page/Frame 的 V8 context | facade 共享 opener context | popup 独立 V8 context |
| same-origin sync access | 真实 cross-context object access | facade 模拟 | 共享 script agent 上的真实 proxy 转发 |
| cross-origin | local/remote WindowProxy + access checks | 部分 facade restriction | 复用通用 proxy/access surface |
| named target | frame tree + related pages + policy | E2A 已统一 related `window.open()`；其余 producer/group split 仍有 legacy registry | group-level registry + single context identity |
| opener | frame relationship，可被 suppression/COOP 切断 | facade 与 target relationship 可漂移 | group graph 的唯一 opener edge |
| storage | 创建时确定 namespace/clone policy | snapshot handoff 后 target 另建 | 创建 transaction 只分配/clone 一次 |
| CDP | target 对应实际 Page | target 对应第二个 Page | target bind/adopt renderer-created Page |
| close | Window/Page/target 同一生命周期 | record 与 target 分开 | 同一 close transaction |
| popup gate | browser-side policy + activation consume | 局部 userGesture/policy | owner-level gate，结果冻结一次 |
| COOP | BrowsingInstance / virtual BCG / opener sever | boolean/policy projection 为主 | group switch + proxy endpoint 更新 |

## 为什么 child-frame 基础值得复用

child-frame 当前实现已经从早期 same-realm shell 演化成一条较完整的 realm ownership
链。关键能力包括：

### 1. stable proxy 与 realm promotion

`ChildWindowProxyRecords` 保持：

- stable child WindowProxy identity；
- live Window wrapper 与 facade context；
- stable `parent` / `top`；
- same-origin 和 cross-origin endpoint projection；
- caller-specific cross-origin access surface；
- default execution context id。

`take_child_window_proxy_shell_for_realm` 会把 pre-bootstrap facade context 的 global
detach，`post_parse` 创建真正的 child V8 `Context` 时复用这个 global object，随后由
`promote_child_window_proxy_shell_to_realm` 完成 promotion。这样 JS 早先拿到的
`iframe.contentWindow` identity 不会因 realm materialization 或 navigation 改变。

### 2. 明确的 LocalWindow / Document transition

`frame_owner_model` 已把 transition 写成 typed decision：

- `Installed` / `Preserved` / `Replaced` / `Retired`；
- `ReplaceLocalWindow`；
- `ReuseInitialEmptyLocalWindow`。

`store` 只在 initial empty、same accessible origin、policy/domain 条件匹配时复用
LocalWindow；其它 cross-document commit 替换 LocalWindow。旧 generation 不能因为
异步完成事件再写入新 Document。

### 3. 独立 V8 Context 与精确 owner

`ensure_prebootstrapped_child_default_context` 为 child 创建独立 V8 `Context`，注册
Window execution context binding/security token，并按需请求 materialization。

`child_frame_realm_materialization` 使用 typed task，携带 exact child handle、owner 和
generation；执行前重新验证 currentness，失败时回滚，成功后注册 inspector context。
这比 popup 专用 `with(window)` wrapper 更接近浏览器 realm 模型。

### 4. security 与跨源表面

child 路径已经具备：

- effective-origin security token；
- same-origin/cross-origin access decision；
- restricted WindowProxy surface；
- named/indexed property projection；
- `postMessage` 和 cross-origin `location` navigation；
- same-origin → cross-origin → same-origin round trip 时的 stable proxy identity。

它还不是 Chromium WindowProxy 的完整实现，但责任边界是正确的：security 决策位于
stable proxy / concrete realm 边界，而不是散落在每个 WebAPI callback。

### 5. initial empty 与旧 realm 退休

child initial empty Document 可以继承 creator origin/policy/resource authority，并带有
明确的 load-token suppression。commit 会 preflight exact owner，决定是否复用 initial
empty LocalWindow，安装新 Document loader，并退休旧 Document 的 callbacks、wrapper、
IndexedDB 等状态。

这些正是 popup 需要的机制。

## 不能直接“把 popup 当 child frame”

复用 child 基础不等于创建一个隐藏 iframe。两者有不可抹平的产品语义：

| child frame | auxiliary popup |
|---|---|
| 有 parent browsing context | 是 top-level context，没有 parent |
| `frameElement` 指向 owner element | `frameElement === null` |
| `top` 指向所属 top-level Page | `top === self` |
| 通常随 owner element detach | 可以在 opener 关闭后继续存在 |
| 参与 parent parser/load blocker | 不应成为 opener Document 的 child load blocker |
| 属于同一 Page 的 frame tree | 拥有独立 Page、history 和 CDP target |
| name lookup 首先是 frame-tree 语义 | name 还需跨 related Page 查找 |
| owner key 当前是 iframe `DomHandle` | 需要独立 `BrowsingContextId` / Page identity |

所以应该抽取 stable proxy、realm、generation 和 access-control primitive，再分别由
`NestedBrowsingContext` 与 `AuxiliaryTopLevelBrowsingContext` 组合。

## 架构选项

### 选项 A：保留双实现并增强同步

做法：继续维护 lightweight DOM，在每次 mutation、navigation、history、storage、close
时同步到 target Page。

结论：拒绝。

原因：

- 两个 loader owner 无法安全去重所有网络和服务端副作用；
- JS object identity、Promise、module namespace、DOM wrapper、Event 和 lexical binding
  不能通过状态复制等价同步；
- currentness/generation race 会成倍增加；
- CLI 与 CDP 的完成条件会继续不同。

### 选项 B：target 保持独立 isolate，opener 只持 remote facade

做法：总是先创建 target PageVM/isolate，opener 返回 RPC 风格 facade。

结论：只适合 `noopener`、COOP-separated 或真正 remote 的访问路径，不能作为普通
same-origin `window.open` 的唯一模型。

原因：same-origin opener 可以同步读取/写入 popup DOM、传递 JS object、调用函数并
观察立即结果。跨 isolate RPC 不能在不引入阻塞和对象代理系统的情况下满足这些语义。

### 选项 C：把 popup realm 永久放在 opener PageVM

做法：直接把 child realm machinery 用于 opener 内一个 top-level-shaped record，CDP
target 代理进 opener PageVM。

结论：可作为短期原型，不适合作为终态。

原因：popup 应有独立 Page task/lifecycle/target，并可在 opener 关闭后存活；把它永久
嵌在 opener PageVM 会让 close、调度、target session、memory accounting 和 ownership
持续特殊化。

### 选项 D：related-page group + shared script agent + 独立 Page runtime

做法：

- browsing-context group 管 related Page、name 和 opener graph；
- script agent 管可以同步互访的 V8 contexts；第一版可让保留 opener 的 popup 与
  opener 共享 isolate；
- opener Page 和 popup Page 各有独立 `PageVM`、main WindowProxy、Document realm、
  task queue、history 和 target binding；
- child 与 popup 共用抽取后的 WindowProxy/LocalWindow/realm primitive；
- protocol target 绑定已经存在的 popup Page residence，不再另建 Page。

结论：推荐。

它同时满足同步 JS identity、独立 Page/CDP 生命周期和单一 document owner。代价是要
放宽当前 per-page-isolate policy，因此必须先做可行性验证。

不建议把所有 browser-context Page 无条件放进一个全局 isolate。共享范围应由 script
agent / browsing relationship 决定，并保留未来按 origin/COOP 切分 agent 的能力。

## 目标架构

下面是逻辑结构，不要求按同名 Rust struct 落地：

```text
RendererBrowserContextRuntime
  |
  +-- BrowsingContextGroupRegistry
       |
       +-- RendererBrowsingContextGroup
            +-- related-page name registry
            +-- opener relationship graph
            +-- one or more RendererScriptAgent
            |     +-- V8 isolate / inspector backend
            |     +-- realm/context registry
            |
            +-- opener Auxiliary/Primary PageRuntime
            |     +-- PageVM
            |     +-- stable main WindowProxy
            |     +-- current LocalWindow/Document realm
            |     +-- CDP target binding
            |
            +-- popup Auxiliary PageRuntime
                  +-- PageVM
                  +-- stable main WindowProxy
                  +-- current LocalWindow/Document realm
                  +-- CDP target binding
```

### 通用 browsing-context record

拟议的通用 record 至少应表达：

```text
BrowsingContextIdentity
  id
  kind = PrimaryTopLevel | AuxiliaryTopLevel | Nested
  group_id
  script_agent_id
  name
  parent?                 // 只用于 nested
  opener?                 // 只是一条可切断的 related-context edge
  stable_window_proxy
  current_local_window_generation
  current_document_generation
  current_realm/context_token
  origin/policy/security_token
  history/session_storage_namespace/storage_key
  lifecycle = InitialEmpty | Active | Closing | Closed
  page_residence?         // top-level only
  target_binding?         // top-level only
```

不要把所有字段都放进一个巨型 struct；上面只是必须有唯一 owner 的状态清单。具体可
拆成 identity、relationship、document owner、proxy endpoint 和 Page residence。

### 责任归属

| 责任 | 建议 owner |
|---|---|
| context id、kind、name、parent/opener edge | browsing-context group |
| stable WindowProxy 和 local/remote endpoint | 通用 WindowProxy host |
| LocalWindow/Document generation | browsing-context document owner |
| V8 isolate、context registry、inspector backend | script agent |
| navigation、task queue、history、Page lifecycle | top-level PageVM / nested frame owner |
| browser-context storage/permission/network policy | renderer browser-context runtime |
| target id、session、auto-attach 和 CDP policy | protocol target controller |
| target 到 renderer Page 的绑定 | `RendererPageResidenceIdentity` 与 target residence bridge |

protocol 可以决定是否 auto-attach、是否 wait for debugger、如何发 CDP event，但不能
再拥有第二个 popup loader 或文档。

## 目标 `window.open()` transaction

### 新 auxiliary context

建议的顺序如下：

1. 在 entered realm 捕获 exact source Page/Document generation，完成 URL 和 features
   解析。
2. 在 browsing-context group 中解析 special target / named target，并执行
   `CanNavigate`、sandbox、popup blocker、transient activation、opener/COOP policy。
3. 如果选择已有 context，只对该 context 排队一次 navigation，并返回其 stable
   WindowProxy；不要创建 popup carrier 或 target。
4. 如果创建新 context，先同步分配 auxiliary context id、Page residence、stable main
   WindowProxy 和真实 initial empty Document realm。
5. initial empty Document 继承 creator origin/policy；按最终 opener policy 分配或 clone
   session storage namespace，整个 transaction 只做一次。
6. 把 non-empty URL 记录成该 Page 的一个 pending navigation token。此时
   `window.open()` 已可返回，调用方可以立即执行 `w.document.write(...)`。
7. renderer 发布 immutable `AuxiliaryContextCreated` output，其中携带 context/Page
   residence、source generation、target name、features、opener policy 和 pending
   navigation identity；它是“已创建 Page 的通知”，不是“请 protocol 再创建 Page”。
8. protocol 为这份 Page residence 分配并绑定 CDP target，应用 auto-attach、Fetch/
   Network、Runtime script 和 wait-for-debugger policy。
9. owner runtime 只在 target admission 完成后释放 popup 自身的 task/script/navigation；
   `waitForDebuggerOnStart` 只暂停这一个真实 target。
10. 对 pending URL 执行一次 navigation。redirect 可以产生多个网络 hop，但一个
    navigation token 只有一个 authoritative loader。

`Page.windowOpen` 应从同一 creation record 派生为观测事件，不应作为创建第二个 Page
的触发器。

### 同步创建与 protocol admission 的边界

`window.open()` 必须同步返回，但 CDP output 通常在当前 renderer turn 结束后才被
protocol 消费。不能用 sleep、drain 或轮询填这个时间差。建议显式建模：

- `PendingAuxiliaryPage` 已拥有 initial empty realm，因此 opener 的同步跨 Window 操作
  是真实操作；
- popup 自己的新 task、parser 和目标 URL navigation 在 `TargetAdmission` 前不可运行；
- protocol 不存在的 CLI 路径使用同一 admission API 的默认立即接受策略；
- auto-attach 配置应在 renderer browser-context runtime 中有可读取的冻结 snapshot，
  或由 owner loop 做明确 handshake；
- stale admission 必须带 Page residence/generation，不能释放已关闭或已替换的 popup。

### named target

命名查找必须统一为 group operation：

1. special keyword；
2. source frame subtree / current Page frame tree；
3. related Page；
4. policy fallback。

查找到现有 popup 后：

- 返回同一 stable WindowProxy；
- 只导航同一 Page；
- 必要时 focus 该 Page；
- 已关闭 context 不参与查找；
- `noopener` 对 special target 和 existing target 的具体 Chromium/WPT 行为要由专门
  compatibility test 固定，不能只靠 feature parser 推断。

### `noopener` / `noreferrer`

创建仍然发生，但：

- `window.open()` 返回 `null`；
- 新 context 没有可脚本访问的 opener edge；
- `noreferrer` 同时影响 referrer；
- 它可以进入新的 browsing-context group / script agent；
- source renderer 不需要持有 local proxy，protocol 仍可观察独立 target；
- storage namespace clone policy应与 Chromium/WPT 对齐，不能无条件复用保留 opener
  路径的 snapshot。

### cross-origin navigation 与 COOP

普通 related popup 从 same-origin initial empty Document 导航到 cross-origin 后：

- source 已持有的 WindowProxy identity 保持稳定；
- access surface 切到 cross-origin restriction；
- `postMessage` 和允许的 `location` write 仍走 endpoint；
- 回到 same-origin 后可重新转发到新的 LocalWindow realm，但不能复活旧 realm。

第一阶段可以让 related cross-origin Page 留在同一 isolate，并依赖 security token 和
access checks；这符合 Blink 中 cross-origin LocalFrame 也可能存在的事实。未来需要
agent/process separation 时再把 endpoint 切成 remote。

COOP group switch 不只是设置 `crossOriginIsolated` boolean。它要：

- 分配/切换 browsing-context group；
- sever opener relationship；
- 让旧 group 持有的 proxy endpoint 呈现断开/closed 语义；
- 防止旧 generation 的 message/navigation/async completion 进入新 group；
- 必要时切换 script agent。V8 context 不能跨 isolate 搬迁，因此 agent split 必须是
  新 realm commit，而不是移动已有 handle。

### close 与 target 生命周期

`window.close()`、`Target.closeTarget`、opener 观察到的 `popup.closed`、
`Target.targetDestroyed` 和资源回收必须落到同一个 close transaction：

1. context 从 `Active` 原子进入 `Closing`；
2. beforeunload/unload policy 由同一个 Page owner 决定；
3. 拒绝新 navigation/task，旧 async completion 因 generation 不匹配被丢弃；
4. 关闭 Document/realm/Page resources；
5. 从 name/opener registry 移除；
6. protocol 从同一状态变化派生 targetDestroyed；
7. stable proxy 继续可被旧 JS handle 观察为 `closed`，但不再暴露 live inner Window。

popup 不应因为 opener Page 关闭而自动销毁，除非产品 policy 明确如此。

## 迁移计划

迁移应按能够独立验证的不变量切片，不做一次性全仓库重写。

### Phase 0：冻结现状与目标 probe

目的：在改 ownership 前把关键可观察差异变成最小复现。

- 保留当前双 load 测试作为“已知债务”的 characterization，但移除任何把双 load 描述
  为长期正确语义的文档；
- 增加或整理本地 HTML/CDP probe：
  - `const w = open('about:blank'); w.document.write(...)` 后 CDP DOM 与 `w.document`
    必须相同；
  - non-empty URL 服务端只收到一个 top-level navigation request；
  - popup script mutation 可由 opener 和 popup target 同时观察；
  - `w.close()` 产生对应 targetDestroyed；
  - popup 在 opener close 后继续运行；
  - same-origin → cross-origin → same-origin 的 proxy identity；
  - named popup reuse；
  - 204/205 保留 initial Document/history；
- 用本地 Chromium 对同一 probe 录制 event/order/return-value 参考。

目标语义测试在相应 phase 完成前可以作为独立 probe 或明确 ignored debt，不能通过
放宽断言让错误路径变绿。

### Phase 1：从 child 抽取通用 primitive，行为不变

- 把 key 从裸 `DomHandle` 提升为可表达 nested/top-level 的 typed context identity；
- 抽取 stable proxy record、LocalWindow transition、realm materialization request、
  security/access surface 和 retirement hook；
- child adapter 继续提供 `parent`、`top`、`frameElement` 和 parent load blocker；
- 所有现有 child focused nextest 必须保持通过；
- 此阶段不改 popup production path，避免同时改基础和调用方。

完成标志：通用 primitive 不依赖 iframe owner element，child 行为无回退。

实施状态（`popup-refactor`，Phase 1 第一至三切片）：

- 已新增与 iframe owner、Page target、popup carrier 无关的
  `BrowsingContextId` / `BrowsingContextKind`；main 与 child frame owner record
  现在显式持有该 identity；
- owner-model 的 stable WindowProxy record、LocalWindow commit transition、Document
  creation/initial-empty transition 和 realm materialization request 已移到通用
  browsing-context model；frame 层通过 type alias 维持现有调用语义；
- child V8 WindowProxy registry 的 authoritative key 已从 `DomHandle` 改为
  `BrowsingContextId`；`DomHandle` 仅保留在 child adapter 中用于从 iframe owner
  查找 context，owner rebind 仍保留当前 stable proxy 行为；
- realm lifecycle 和 exact Document/LocalWindow/realm currentness 已抽成参数化的通用
  primitive；frame owner 通过 typed alias 保留原有状态机和 stale-generation 判定；
- Document owner retirement transaction 现在同时携带 `BrowsingContextId` 和精确的
  retired/current owner generation；initial empty install、navigation replacement、
  `document.open()` 和 detach 都从 owner store 发布同一形状的 transition，iframe
  `DomHandle` 仅由 frame adapter 额外携带；
- main 与 child Document replacement 现在都组合同一个通用 owner transaction；
  external-state retirement hook 携带 context id、retired owner 和 exact Document token，
  child adapter 只追加 iframe handle 并消费 child 专属清理；
- origin access comparison、realm access policy、default/isolated world 和
  `RealmHostProjection` 已移到通用 browsing-context model；child realm 初始化与
  isolated-world rebind 必须同时匹配 context id、exact owner、realm token 和 world，
  `parent` / `top` / `frameElement` 仍由 child adapter 安装；
- popup production path、loader、protocol target creation 和双 load characterization
  尚未改变。这正是 Phase 1 的行为不变边界，不代表 popup 已修复；后续 auxiliary
  Page 可以组合上述 primitive，而不需要继承 iframe owner element 或 parent load
  blocker。通用 primitive 已不依赖 iframe owner element，Phase 1 完成。

### Phase 2：shared script-agent 可行性实验

#### Phase 2A：显式 identity 与当前策略基线

Phase 2A 当时的源码基线已经引入 typed `ScriptAgentId`，由 document isolate holder 分配并通过
Moli runtime memory diagnostics 暴露。`RendererPageScriptEnvironment` 是当前
agent identity 的稳定宿主：

- 同时存活的两个顶层 Page script environment 必须报告不同 `ScriptAgentId`；
- 同一个稳定 Page 的 cross-document navigation 必须保留 `ScriptAgentId` 和 main
  WindowProxy，但替换 Document realm/context generation；
- child default world、isolated world 和同 Page navigation generation 复用所属 Page
  agent；
- fresh Page diagnostics 明确报告 `scriptAgentScope = page-script-environment`；Phase 2A
  当时尚未开放 related-page admission。

这一步只把现有策略变成可命名、可测试的边界，没有让两个 production Page 共享
isolate，也没有改变 popup 双实现或双 load。

仓库历史上已经有过 renderer-owner-wide shared document isolate：`40321d2894` 建立
基础，`310362ebe3` 将其设为默认。旧回归覆盖了多个 Page 的不同 V8 Context/global、
不同 Inspector context group、peer close 后存活、navigation context replacement、
timer/fetch/worker/IndexedDB 和 stale generation 路由。因此“一个 isolate 能承载多个
Page realm”本身不是未知项。

但该策略随后因跨页面 V8 heap 累积和回收边界过宽，在 `7b17efa965` 切回 per-Page
containment，并由 `b149639b6d` 接受为临时 workaround。历史功能回归不能覆盖这一
内存失败，也不能证明把所有 renderer-owner Page 再次放入同一 agent 是安全的。当前
设计因此只允许由 browsing relationship admission 的 related Page 共享 agent；禁止
恢复 owner-wide 默认共享。历史和当前 workaround 的边界见
[Per-Page Document Isolate 临时 Workaround](per-page-document-isolate-temporary-workaround-2026-07-10.md)。

#### Phase 2B：selective related-page admission

建立最小实验，让两个 Page residence 在同一 script agent/isolate 中拥有不同 V8
Context，并验证：

- realm 的 `globalThis`、intrinsics、lexical bindings 相互独立；
- same-origin WindowProxy 可同步传递 object/function/DOM wrapper；
- context embedder data 能精确路由到各自 Page/Document generation；
- CDP executionContextId、remote object id、object group 和 binding 按 target/session
  隔离；
- inspector context created/destroyed 顺序正确；
- microtask、timer、fetch、worker、IndexedDB 和 unhandled rejection 回到来源 Page；
- 关闭一个 Page 不销毁另一个 Page 的 isolate/resources；
- popup 在 opener PageVM drop 后仍可执行；
- 不引入裸指针、泄漏全局 cache、sleep/drain/retry。
- related Page 全部关闭后 agent/isolate 可确定销毁；非 related Page 不会被 admission；
- 多轮 related-page create/close 与 navigation churn 的 heap/RSS 不退回
  renderer-owner-wide 累积模式。

如果实验无法在当前 `RendererPageScriptEnvironment` / `PageVm` ownership 下保持这些
不变量，应先重构 script-agent owner，不能退回 mirror 同步。

实施状态（Phase 2B 完成时的 test-only 验证基线；Phase 3 第一纵切已提升窄入口）：

- document isolate 现在持有 `RendererScriptAgentV8ForegroundTaskRouter`，production
  fresh Page 默认只注册一个 Page member，因此现有 per-Page containment policy 不变；
- stable `RendererPageScriptEnvironment` 持有 RAII agent membership；same-Page
  replacement 复用 membership，Page slot retirement 先撤销 route，再清理 Page tasks；
- Phase 2B 中 `#[cfg(test)]` 的 related-page reservation 可以从同 renderer owner 的 live source
  Page 共享 isolate holder，同时创建独立 Page environment、main WindowProxy、V8
  Context、task sources、output journal 和 Inspector binding；Phase 3 第一纵切已把
  reservation/admission/router 提升为 production primitive，但只允许 renderer 明确新建的
  auxiliary context 使用，默认 Page 仍是 fresh；
- V8 foreground task 是 isolate/agent scoped，V8 不提供 originating `Context`。router
  只让一个 live member 执行 concrete task 一次，随后给其他 member 排入 typed
  checkpoint；如果执行 Page 正在退休，尚未执行的 concrete task 会转投 surviving
  member，checkpoint-only payload 不跨 Page 退休；
- 初版只把 task 路由给一个 member 时，peer Page 的 `awaitPromise` 稳定在 30 秒门禁
  超时；加入 task-once + peer-checkpoint 后同一用例通过。这说明 fan-out 是多 realm
  agent 的必要 checkpoint 语义，不应以 sleep、轮询或把 task 重复执行多次替代；
- 聚焦实验已经证明：两个 related Page 报告同一 `ScriptAgentId` / 一个 isolate，
  但拥有不同 global、intrinsics、main WindowProxy、Inspector context group；跨 target
  remote object fail closed；source Page close 后 peer、navigation replacement 和 async
  WebAssembly foreground continuation 仍可工作；
- 默认两个并发 Page 仍报告两个 agent/两个 isolate；同一 Page navigation 仍报告一个
  agent/一个 membership。三条 policy 回归共同防止 test admission 外溢到 production。
- 第二切片增加 `#[cfg(test)]` owner-thread probe，把 peer Page 已存在的 stable main
  WindowProxy 直接安装到 related Page realm；没有创建第二个 proxy、mirror global 或
  旁路 DOM。same-origin Page 已验证普通 object、function 和 DOM wrapper 可同步跨 realm
  传递；peer 同源 navigation 后，保存的 proxy 保持严格相等并投影到 replacement
  Document，新 realm 不继承旧 global property；
- timer、`fetch(data:)`、unhandled rejection 分别回到创建它们的 Page realm；A/B 同时
  有 pending work 时不会把 completion 或 rejection event 投到 peer；
- 同一个 Inspector isolate backend 中，同名 object group 仍由 Page inspector binding
  隔离，A 的 `Runtime.releaseObjectGroup` 不释放 B 的 remote object；同名 isolated world
  中 A 的 `Runtime.addBinding` 不注入 B，binding observation 只进入 A 的 output stream；
- dedicated worker 的 message route 和 Page-local IndexedDB manager route 在共享 isolate
  下保持精确；admission source close 后 peer 的 worker event 和 IndexedDB transaction
  仍可完成；
- 三个 member 按中间 Page → 原始 source 的非 LIFO 顺序关闭后，survivor 可再次 admission
  新 member；`scriptAgentPageCount` 按 3→2→1→2→1 变化，整个序列只创建并最终销毁一个
  document isolate。这里的 membership count 与只统计未 commit residence 的全局
  `reserved` diagnostics 已明确区分。
- shared isolate 让此前被 per-Page isolate disposal 掩盖的 Context 自引用变得可观察：
  rusty_v8 Context slot 由 Context annex 持有，但 `BridgeContextWindowWrapper` 和
  `IntrinsicInterfaceRegistry` 又分别通过强 `v8::Global` 指回该 realm 的 Window、
  constructor、prototype 和 public interface。只丢 Rust `Global<Context>` 不能打破这个
  跨 Rust/V8 的 ownership cycle；peer Page close 后 Context 和 global handles 会留在仍
  存活的 agent isolate 中；
- teardown 现在先清所属 wrapper cache，再只移除上述两个明确拥有 realm-local strong
  V8 handle 的 slot。host pointer、runtime-observable token、resource owner、Promise
  rejection 和其它 execution-owner metadata 保持到 V8 真正回收旧 realm，因此 retained
  child-realm function 仍能按原 owner fail closed，而不是退化成通用 `no access`；
- 新的普通回归连续创建三个 related peer；每个 peer 都执行一次 cross-document
  navigation，再关闭并由 anchor agent 触发 GC。active/native context 必须是 anchor+1，
  replacement 后不能增加，close 后必须回到 anchor baseline；detached context、Inspector
  registry、agent member count 和最终 isolate accounting 同时设为硬断言；
- 新的 ignored release acceptance 默认执行 120 轮，每轮分配 24,000 个 JS objects、
  4,000 个 DOM nodes 和 1,000 个 resolved Promises，每 12 轮导航一次 peer。它记录
  `/proc/self/status`、V8 heap/physical/external memory、global handles、native/detached
  contexts、Inspector registry、agent membership 和 isolate accounting；release 且至少
  60 轮时，后半段线性斜率硬门为 used heap `<= 0.02 MiB/轮`、RSS
  `<= 0.20 MiB/轮`。

本切片还暴露并修正了一条独立 fixture 生命周期：最初 full suite 中 standalone
`ScriptVm` 在 realm bootstrap 后立即丢弃 membership，导致异步 WebAssembly foreground
task 没有 live Page route，5 秒门禁超时。membership 现在显式穿过 page-realm/default-world
bootstrap 并保留到 `ScriptVm` 生命周期；owner-managed Page 仍由 stable Page environment
额外持有，并在 Page slot retirement 时主动撤销。第一切片合入前验证结果：

```bash
cargo nextest run -p moli-renderer-v8 \
  script_vm::tests::browser_api::misc::webassembly_compile_accepts_spec_valid_bounds_above_v8_instantiation_limits
# 1 passed

cargo nextest run -p moli-renderer-v8 \
  -E 'test(/runtime::tests::(related_page_script_agent_experiment_shares_isolate_and_survives_source_close|per_page_isolate_policy_uses_distinct_isolates_and_isolates_contexts|per_page_isolate_policy_reuses_navigation_isolate_and_replaces_contexts)$/)'
# 3 passed

cargo nextest run --no-fail-fast
# 15551 passed, 17 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# passed
```

第二切片最终验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(/runtime::tests::related_page_script_agent_/)'
# 7 passed

# 上述 7-case filter 连续执行 20 轮
# 20/20 passed，合计 140 case executions

cargo nextest run --no-fail-fast
# 15557 passed, 17 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# passed
```

#### Phase 2B 第三切片：realm teardown 根因与 release 长序列

最初的零 payload 诊断只保留一个 anchor Page，重复创建/关闭 related peer，并每两轮
导航一次 peer。修复前四轮 post-close native context 数依次为 `2 / 4 / 5 / 7`，used
global handles 为 `10,208 / 15,552 / 18,080 / 23,328 bytes`，detached context 为
`0 / 1 / 1 / 2`。每个未导航 peer 本身留下一个 Context，每次 peer navigation 又留下
一个 retired Context；即使追加 V8 full GC 也不下降。与此同时：

- Inspector default-context registry 在 peer close 后已经回到 `1`；
- agent membership 已从 `2` 回到 `1`；
- `RendererPageScriptEnvironment` 的最后一个 `Rc` owner 确实析构；
- 整个 anchor 关闭后 isolate accounting 也回到 baseline。

因此问题不是 Inspector registry、Page testing handle、environment clone 或 GC 调度，
而是仍存活 isolate 内的 strong V8 root。进一步核对 Context annex 后确认了上节所述的
两个 self-cycle slot。

第一次修复尝试在 retirement 时调用 `Context::clear_all_slots()`。它能让内存 probe
立即转绿，但全量 nextest 准确地否决了这个边界：retained old-child XHR、fetch、Beacon
function 丢失 host pointer，预期的旧 realm shutdown/`false` 语义变成 `no access`；child
self-navigation load 和一次 `document.domain` navigation 也失败。最终实现只释放两个
拥有 realm-local strong V8 handles 的 slot，保留其它旧 realm metadata。对应 5 个
child-navigation 回归与新的 related realm 释放回归组成 6-case 交叉集合，最终全部通过。

exact release 源码快照：

- commit：`847448b8447f0d226567394d2e878265d3d0cafe`；
- Git tree：`58acc6161960baef5466636f81dca99ba1318b4f`；
- profile：Cargo `release`，独立
  `CARGO_TARGET_DIR=target/related-agent-memory-release-847448b844`；
- rustc：`1.96.1 (31fca3adb 2026-06-26)`，host
  `x86_64-unknown-linux-gnu`；
- host：Linux `6.12.73+deb13-amd64`，Intel Core i9-13900K，32 online logical CPUs；
- 测试 binary SHA-256：
  `10fa659fea2c262cc26260afb7f5af6bfdc9edc64beabf04c2840f259a6127d4`。

两次运行使用同一个 binary、相同 workload 和硬门：

| 指标 | run 1 | run 2 | 门禁/解释 |
|---|---:|---:|---|
| 120 轮 elapsed | `8.474 s` | `8.206 s` | 完整 payload 与 10 次 peer navigation |
| 后 60 轮 post-close used-heap slope | `0.000000000 MiB/轮` | `0.000000000 MiB/轮` | `<= 0.02` |
| 后 60 轮 post-close RSS slope | `0.025481 MiB/轮` | `0.005147 MiB/轮` | `<= 0.20` |
| 首/末 10 轮均值 used-heap delta | `0.006378 MiB` | `0.006378 MiB` | 非线性增长指标，不单独设门 |
| 首/末 10 轮均值 RSS delta | `4.605078 MiB` | `3.907813 MiB` | allocator/file-backed 平台变化，不单独设门 |
| peak active used heap | `16.373085 MiB` | `16.264664 MiB` | peer heavy payload 存活时 |
| peak active RSS | `125.832031 MiB` | `125.355469 MiB` | 同上 |
| final post-close used heap | `1.825363 MiB` | `1.825363 MiB` | anchor-only |
| final post-close RSS | `102.847656 MiB` | `101.660156 MiB` | anchor-only |
| max detached contexts（active/nav/close） | `0 / 0 / 0` | `0 / 0 / 0` | 硬断言 |
| native contexts（anchor/active/post-nav/post-close） | `1 / 2 / 2 / 1` | `1 / 2 / 2 / 1` | 每轮硬断言 |
| used global handles（first → last post-close） | `7,744 → 7,840 B` | `7,744 → 7,840 B` | 没有按 peer/导航累积 |
| isolate accounting（baseline → live → final） | `0 → 1 → 0` | `0 → 1 → 0` | created/destroyed 均为 `1` |

原始 JSON 位于 ignored `target/` 下，不作为源码提交：

- `target/related-agent-memory/847448b844-run1.json`，SHA-256
  `94d47df8d63d6b86a4772d5e3fe755659c976cb20eae6071840d4ca97421aa20`；
- `target/related-agent-memory/847448b844-run2.json`，SHA-256
  `39088d5bfbcce683530fb0af01fb1deb5c8c4f7eebc5952953bf35928c65521e`。

复跑命令；run 2 只替换 `OUTPUT` 文件名：

```bash
env \
  CARGO_TARGET_DIR=/home/donoughliu/code/lightmount3/target/related-agent-memory-release-847448b844 \
  MOLI_RELATED_AGENT_MEMORY_ITERATIONS=120 \
  MOLI_RELATED_AGENT_MEMORY_PEER_NAVIGATION_EVERY=12 \
  MOLI_RELATED_AGENT_MEMORY_PAYLOAD_OBJECTS=24000 \
  MOLI_RELATED_AGENT_MEMORY_DOM_NODES=4000 \
  MOLI_RELATED_AGENT_MEMORY_PROMISES=1000 \
  MOLI_RELATED_AGENT_MEMORY_OUTPUT=/home/donoughliu/code/lightmount3/target/related-agent-memory/847448b844-run1.json \
  MOLI_RELATED_AGENT_MEMORY_COMMIT=847448b8447f0d226567394d2e878265d3d0cafe \
  cargo nextest run -p moli-renderer-v8 --release --run-ignored only \
    -E 'test(/runtime::tests::related_page_script_agent_release_memory_acceptance$/)'
```

最终 repository gate：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(/runtime::tests::related_page_script_agent_/)'
# 8 passed

cargo nextest run --no-fail-fast
# 15634 passed, 18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# passed
```

按本文定义，Phase 2B 的 selective related-page shared-agent 可行性实验至此完成：功能、
Page-local 路由、non-LIFO ownership、realm retirement、isolate lifetime 和两次 release
长序列内存门均有证据。这个结论只授权 Phase 3 打开显式 relationship admission，不授权
把 owner-wide sharing 恢复为默认，也不表示 popup 已完成。Phase 3 第一纵切已经提升
related admission 和 Page reservation；第三纵切 A 又把 WindowProxy cross-realm link
提升到 production 的 opener-preserving popup 路径。production 的单一 initial Document
owner、multi-session Inspector event 顺序、
SharedWorker/ServiceWorker、跨 origin agent split 和真实 popup 并发 close/navigation 仍分别
属于 Phase 3-5。

### Phase 3：真实 auxiliary initial empty Page

- 在 renderer owner runtime 中创建 `PendingAuxiliaryPage`；
- 为它创建 stable main WindowProxy、独立 V8 Context 和 initial empty Document；
- `window.open('about:blank')` 返回这份真实 proxy；
- protocol target bind/adopt 同一个 `RendererPageResidenceIdentity`；
- immediate `document.write`、storage clone 和 Runtime evaluate 指向同一个 Document；
- close 从第一天就走统一 transaction。

优先只切 `about:blank` 垂直路径，因为它能验证最关键的同步创建、realm identity 和
target adoption，又不先引入网络导航。

实施状态（Phase 3 第一纵切：identity reservation / initial target adoption）：

- renderer Page script environment 现在持有一个不反向拥有 Page/VM 的窄 allocator；
  `window.open()` 或 hyperlink 确认新建 lightweight auxiliary context 时，同步产生
  `RendererPendingAuxiliaryPage`。它把 typed `AuxiliaryTopLevel` browsing-context id 与
  exact `RendererPageReservationToken` 绑定在一个不可拆错的 carrier 中；
- opener 可见时 reservation 显式携带 `RelatedAuxiliaryPage { opener_page_id }`，
  `noopener` / `noreferrer` 携带 `Fresh`。普通 Page 创建、`Target.createTarget` 和
  renderer owner 内其他 Page 不会隐式加入共享 agent；
- popup activation 将该 carrier 原样交给 protocol。新 target 的 `TargetPageSlot` 长期
  保留 auxiliary browsing-context id，并在 initial empty Document build 时一次性消费
  renderer Page reservation；protocol 不再为这条路径制造第二个 initial Page id；
- initial build 不再仅凭 BrowserContext metadata 选择 NavigationEngine。它会从当前与
  retained background engine 中找到能消费 exact reservation 的 opener renderer owner，
  再创建共享该 owner 的 engine wrapper；找不到 owner 时 fail closed，不放宽 token 校验。
  这保证 active、inactive background、BiDi user-context 和轻量测试 fixture 都不会把
  renderer 已接受的 auxiliary Page 偷换到另一个 owner；
- named lightweight target reuse 不产生第二份 reservation；尚未 materialize lightweight
  context 的 fallback action、browser-context action 和 service-worker action 仍走旧路径；
- production related Page 使用已验收的 script-agent router/membership。初始
  `about:blank` 集成回归证明 opener 与 popup 有不同 Page/Context/realm，但报告同一个
  `scriptAgentId` 和一个 live document isolate；`noopener` popup 采用不同 agent；
- `HeapProfiler.moliDiagnostics` 改为从 Page snapshot 汇总唯一 `scriptAgentId`，
  不再把 loaded Page 数机械等同于 V8 isolate 数。V8 heap、GC、Inspector default-context
  registry 和 foreground wake 的诊断 scope 相应标为 script-agent，而 target-document
  计数仍保持 Page/Document-local；
- shared-agent Inspector pause bridge 已从单一 target route 改为按
  `RendererDevToolsAgentToken -> Page output journal` 路由。关闭或替换 popup Page 只撤销
  该 Page 的 route、pause session 和 queued command，不会永久关闭 opener target；nested
  pause loop 也按 agent token 选择 V8 Inspector session，而不是假定一个 isolate 只有一个
  context group。Classic WebDriver 命名 popup 的“创建、切换、导航复用、再回到 opener
  click”路径覆盖了这个 lifetime。没有 concrete Page pause route 的低层 Inspector binding
  仍可把普通通知留在 agent-local queue；只有 `Debugger.paused` 必须有精确 Page route，
  防止共享 bridge 的 route 存在性误伤 replacement/overlap teardown；
- owner handoff 与 Inspector lifetime 由 initial adoption、inactive-background CDP、BiDi
  viewport inheritance、Classic named-popup reuse、replacement/overlap binding teardown 及
  `closing_related_page_route_keeps_opener_target_routable` 联合覆盖。

本纵切完成时的实跑证据：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(/script_vm::inspector_pause::tests/) | test(/script_vm::inspector::tests::replacement_document_binding_does_not_adopt_previous_agent_outbound/) | test(/script_vm::inspector::tests::dropping_overlapping_peer_binding_does_not_deactivate_current_agent/) | test(/script_vm::tests::window_execution_context::strict_window_binding_resolves_registry_policy_and_rejects_retired_realm/) | test(/runtime::tests::related_page_script_agent/) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned)' \
  --no-fail-fast
# 20 passed

cargo nextest run -p moli-protocol --no-fail-fast
# 3233 passed

cargo nextest run -p moli \
  -E 'test(websocket_bidi_set_viewport_user_context_inherits_through_window_open) | test(webdriver_classic_named_popup_reuse_navigates_existing_window)'
# 2 passed

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# 15635 passed, 18 skipped
```

这一纵切只完成“renderer 先保留身份、protocol initial `about:blank` 接管”的不变量。
它尚未完成 Phase 3：`window.open()` 返回的仍是 opener PageVM 内 lightweight proxy，因而
opener immediate `document.write` 与 CDP target 仍不是同一个 Document；现有 protocol
cross-document `Page.navigate` 仍会分配 fresh Page/agent。

#### Phase 3 第二纵切 A：live Page replacement prepare 基础

本提交先收窄 prepared document 进入稳定 Page replacement path 前必须成立的 ownership
边界，没有提前改 protocol install：

- `RendererPageHandle` 通过 renderer owner command 异步预留 replacement Document；token
  保留原 Page id / owner-local host，并携带当前已提交 `PageVm` creation id 与唯一 nonce；
- owner-local store 只保留同一 Page 最新的未消费 reservation。旧 nonce 在 isolate
  bootstrap 前以 `superseded` 失败；Page 已发生其它 cross-document commit 时，旧
  generation 也在 bootstrap 前 fail closed；
- prepare 不再为该 token 创建 fresh/related isolate 或第二套 Page task sources，而是从
  stable Page slot 取得 `RendererPageScriptEnvironment`，调用既有
  `bootstrap_replacement_document_isolate()`。因此 reservation 已明确复用 script agent、
  isolate、agent membership、Page task producer routes、output journal，并声明复用 stable
  main WindowProxy；
- isolate reservation 现在区分“initial creation 自己拥有 output stream”和“replacement
  只借用 live Page stream”。replacement cancel、stale failure 或当前尚未开放的 commit
  拒绝只释放 prepared residence，不能发送 live stream `Closed`，也不能改变 isolate
  created/live/destroyed accounting；
- 在 replacement install/handle ownership 尚未接入前，`commit()` 显式报错并同步取消
  residence，避免误入只允许新 Page 的 `attach_page_entry_for_owner`，更不能靠同 Page id
  碰撞偶然失败。

聚焦回归同时覆盖了旧 prepared-document 自有 stream 的取消语义，防止修复 replacement
时把 initial creation 的 stream lifetime 放宽：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(/runtime::tests::(canceled_prepared_document_closes_its_ordered_output_stream|prepared_external_raw_document_waits_for_matching_commit_permit|per_page_isolate_policy_reuses_navigation_isolate_and_replaces_contexts|related_page_script_agent_experiment_shares_isolate_and_survives_source_close|canceling_prepared_live_page_replacement_preserves_page_environment_and_output_stream|stale_live_page_replacement_reservation_fails_before_isolate_bootstrap|newer_live_page_replacement_reservation_supersedes_unconsumed_nonce)$/)' \
  --no-fail-fast
# 7 passed

cargo nextest run --no-fail-fast
# 15638 passed, 18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# passed
```

#### Phase 3 第二纵切 B：stable Page replacement commit / core adoption

本提交已经完成上一节要求的 replacement commit 边界，但尚未改 protocol：

- `PreparedRendererDocument::commit_page_replacement()` 使用独立 owner command；只有
  `ExistingPageReplacement` admission 可以进入。初始 Page prepared commit 与 replacement
  commit 类型上分开，误用时释放 prepared residence 并 fail closed；
- commit 在拆旧 realm 前再次比对 reservation 记录的 `PageVm` creation id、stable slot
  generation 和当前 resident generation。prepare 后若发生另一场 cross-document commit，
  stale prepared Document 不能覆盖新 Document；
- 旧 Document lifecycle 以 `SupersededByCrossDocumentNavigation` 结束，旧 default Inspector
  context 和 Page-context resources 在新 realm bootstrap 前撤销；新 PageVm 复用原 script
  agent、isolate、agent membership、typed Page task routes、output journal 和 stable main
  WindowProxy；JavaScript dialog broker 与 Inspector pause bridge 属于 Document-scoped
  adoption artifact，由新 PageVm 重新产生并在 stable handle 上替换；
- streaming raw 与 NativeDom 两条 prepared bootstrap 都把结果直接安装到 checked-out 的原
  `RendererPageLocalEntry`。phase-one residence、pending location navigation 和 post-parse
  lifecycle 都沿用现有 live Page continuation，不进入 initial Page attach；
- replacement publication 使用同一 Page id，推进 `vm_creation_id` 和 `view_generation`，并
  通过 typed replacement-settled wake 解锁等待 committed view 的命令。response metadata
  同时更新原 stable `RendererPageState`，而不是保留旧 status/headers/initiator；
- `DocumentCommit` reply 可以在 streaming response 尚未结束时返回 non-owning replacement
  result，后台继续同一 phase-one/lifecycle。它保留 `ReturnWithPendingNavigation` policy，
  protocol-owned script navigation 不会被 standalone adapter 偷走；
- `RendererPageReplacementCommit` 只含 Page identity、新 Document DevTools agent token、
  新 Document dialog broker / pause bridge、Page state、creation diagnostics/artifacts 和可选
  download，不含 `RendererPageHandle`、Page cancel sender 或第二份 close authority；
- core `PreparedDocumentPage::commit_page_replacement(..., &mut Page)` 在 renderer commit 前校验
  exact owner/Page identity，并让原 `Page`/`RendererPageHandle` 显式采纳新 agent token 和
  state。原 handle 仍是唯一 command/close owner，已有 renderer-agent attachment id 不被
  偷换。

聚焦回归覆盖 stable identity、realm retirement、commit-time stale generation、open stream
DocumentCommit、NativeDom 和 core adoption：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(/runtime::tests::(prepared_live_page_replacement_commits_in_stable_page_slot_without_new_handle|prepared_live_page_replacement_document_commit_replies_before_stream_completion|prepared_live_page_replacement_document_commit_preserves_browser_owned_tail_navigation|prepared_native_dom_live_page_replacement_uses_the_stable_page_slot|prepared_live_page_replacement_revalidates_generation_at_commit|canceling_prepared_live_page_replacement_preserves_page_environment_and_output_stream|stale_live_page_replacement_reservation_fails_before_isolate_bootstrap|newer_live_page_replacement_reservation_supersedes_unconsumed_nonce)$/)' \
  --no-fail-fast
# 8 passed

cargo nextest run -p moli-core \
  -E 'test(runtime::navigation_engine::tests::core_page_adopts_prepared_renderer_replacement_without_replacing_ownership)' \
  --no-fail-fast
# 1 passed

cargo nextest run --no-fail-fast
# 15644 passed, 18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# passed
```

第二纵切 B 单独完成时仍不是 Phase 3 完成标志；它要求的 protocol 切换由下面的第二纵切 C
完成。

#### Phase 3 第二纵切 C：protocol stable Page navigation

本提交把已有 top-level Page 的 cross-document navigation 从“创建 fresh Page，再替换 target
slot”切换为“在同一 stable Page residence 中替换 Document”。这包括 `Page.navigate` 与
target-owned navigation，覆盖 active target、同 BrowserContext background target 和 inactive
background target：

- navigation detached work 启动前，同时捕获 exact `TargetPageResidenceIdentity` 与
  `RendererPageResidenceIdentity`，并从当前 core `Page` 取得 non-owning replacement
  reservation。后台 future 只持 reservation capability，不复制 `RendererPageHandle` 或 close
  authority；
- stable commit 保留同一个 core `Page` handle、renderer `PageId`、target Page residence
  generation 与 `TargetPageAttachmentId`。新 Document 改用新的 DevTools agent token、
  renderer-agent attachment、default realm、execution context 和 context group；旧 realm 与
  attachment 按 replacement 顺序退休；
- inline HTML、`data:`、普通网络 streaming response，以及 Fetch response-stage 的 buffered /
  captured response 都携带同一个 stable target carrier。Fetch pause 后才确定的 commit
  configuration 会先写回 prepared Document，再进入 renderer commit，避免 interception
  旁路重新分配 Page；
- stable reservation 不能只按 BrowserContext 或“最近一个 engine”选择 renderer owner。
  navigation 会比对 captured `RendererOwnerLocalHostId`，必要时从当前或 retained background
  engine 中取出 exact owner 的 `NavigationEngine`；找不到时在 prepare 前 fail closed。这修复
  了同 BrowserContext 内 background target 导航误用 active target renderer owner 的路径；
- renderer commit settlement 现在发布新 Document 的初始 output，建立 predecessor fence，
  再把 creation artifacts、lifecycle binding、main-resource body 和 navigation engine 交给同一
  target owner。protocol 不再因为 Page identity 稳定而漏掉 execution-context / console /
  lifecycle 可观察输出；
- renderer-agent candidate 先作为 transaction 准备，renderer Page commit 成功后再把 stable
  target 的 DevTools channel、全部 frontend session call route 和 Page state 切到新
  attachment。Document-scoped JavaScript dialog broker、Inspector pause bridge 与 dialog scope
  同步轮换，旧 Document 的 pending dialog 不会泄漏到新 Document；
- `pending_live_page_replacement_reservations` 只负责 prepare admission，另用 latest
  reservation 记录保持 commit-time ordering。于是两个已经 prepare 的并发 candidate 中，后
  reservation 可以提交，旧 candidate 以 `PagePreserved` 失败，不能覆盖新 Document 或关闭
  stable Page；
- replacement error 明确携带 `PagePreserved` / `PageRetired` disposition。pre-commit identity、
  nonce 或 candidate mismatch 会回滚 renderer channel 并保留旧 Page；一旦旧 realm 已退休，
  后续 materialization / protocol commit 失败就 fail closed 丢弃当前 Page，不能伪装成可回滚；
- 已从 scheduler registry claim、正在等待 Promise 的 Inspector command 也属于旧 Document。
  replacement 会遍历该 target 的 primary / auxiliary session，给这些 await 精确发送一次
  `Inspected target navigated or closed`，避免命令永久挂起；
- related popup 继续遵守 selective shared-agent 结论：opener 与 popup 在同一个 document
  isolate 中运行，但保持两个 Page realm / execution context，而不是把“两个 Document”误报成
  “两个 isolate”；
- `Target.createTarget` 的 background initial load 之后可能立即进入完整 Page navigation。
  该 target-to-Page future 边界显式 boxed，避免 test thread 同时保留 initial build、response
  plan 与 navigation state machine 导致确定性 stack overflow；这里没有加入 sleep、retry 或
  调大线程栈来掩盖问题。

本纵切的聚焦与 crate 级证据包括：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(newer_live_page_replacement_supersedes_prepared_candidate_without_retiring_page)' \
  --no-fail-fast
# 1 passed

cargo nextest run -p moli-protocol \
  -E 'test(cross_document_page_navigate_replaces_realm_in_stable_page_residence) | test(interleaved_response_heads_only_commit_the_current_prepared_document) | test(runtime_evaluate_await_promise_pending_is_terminated_once_by_navigation_replacement) | test(same_context_background_session_can_stage_its_own_locale_and_timezone_before_promotion)' \
  --no-fail-fast
# 4 passed

cargo nextest run -p moli-protocol \
  -E 'test(local_storage_mutations_fan_out_across_targets_without_leaking_session_storage)' \
  --no-fail-fast
# default test stack 下连续 3 次通过

cargo nextest run -p moli-protocol --no-fail-fast
# 3234 passed

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# 15646 passed, 18 skipped
```

第二纵切 C 单独完成时仍不是 Phase 3 完成标志：它只统一了已经存在的 Page 内的
cross-document replacement，还没有改变 `window.open()` 同步返回的 lightweight
WindowProxy/Document。下面的第三纵切 A 先统一 proxy identity；initial Document owner 和
mirrored load 仍是后续边界。

#### Phase 3 第三纵切 A：opener-visible stable WindowProxy handoff

本提交把已经在 Phase 2B 验收的 related-page 跨 realm WindowProxy 能力接入 production
popup creation，但刻意不把 initial Document mirror 误报为完成：

- `window.open()` / hyperlink 在确认需要新建 lightweight browsing context 后，先从 opener
  的 `RendererPageScriptEnvironment` 预留 exact related auxiliary Page。named target reuse
  不再分配 Page，也不会覆盖已有 handoff；
- opener-preserving 路径不再创建只能留在 opener realm 的 synthetic Window wrapper。它用
  normal Window global template 创建一个由临时 V8 Context 持有的真实 global proxy，立即
  安装现有同步 popup surface，并把这同一个对象返回给 author script。临时 facade 也初始化
  eager intrinsic interface registry；否则第二个命令里重新物化 `HTMLDocument` 等 wrapper 时
  会因为 facade realm 没有 prototype registry 而终止进程。只要 lightweight mirrored loader
  尚未删除，facade 还必须拥有独立 runtime-observable context token，并从 facade realm 内安装
  popup id / opener private slots；这样 handoff 前的 response script 仍能观察 creator，但不把
  Phase 5 尚未实现的 target opener graph 伪装成已完成；
- V8 handle 不进入可跨 renderer/protocol transport 的 `RendererPendingAuxiliaryPage`。opener
  Page 的窄 allocator 持有 owner-local registry，以 reserved target `PageId` 为 key 暂存
  `WindowProxy + facade Context + optional creator security token`；registry 不反向持有 Page、
  PageVM 或 protocol target，因此不会形成 ownership cycle；
- owner-local store 消费 `RelatedAuxiliaryPage { opener_page_id }` 时，必须先找到 exact live
  source Page environment，再一次性取走对应 proxy。目标 `RendererPageScriptEnvironment`
  在首个 realm bootstrap 前登记它，`ScriptVmContextBootstrap` detach 临时 facade，并把 exact
  proxy 作为 `ContextOptions::global_object` 交给真实 auxiliary default Context；没有 alias、
  proxy 状态复制或等待 protocol 的同步补丁；
- initial `about:blank` 对普通 origin 可以重新计算相同 internalized token，但 opaque origin
  与 `document.domain` mutation 使用 V8 unique token。handoff 因此额外只消费一次 creator
  token，保证 inherited initial realm 可由 opener 同步访问；后续 cross-document replacement
  不再复用该 token，而是按新 Document origin 正常计算；
- `noopener` / `noreferrer` reservation 仍是 `Fresh` agent，调用方返回 `null`，不会尝试把
  V8 object 搬到另一个 isolate；ServiceWorker `clients.openWindow` / notification fallback
  也显式保持无 handoff 的旧 lightweight 路径；
- facade inner global 退役后，旧的 lightweight popup private marker 不一定还能从 opener
  realm 直接读取。Classic WebDriver 的 Window reference adapter 因此先走 marker 快路径，
  再由 opener host 按其仍持有的 `LightweightPopupBrowsingContextRecord.window_proxy` exact
  identity 回查 popup id；这不会给 target realm 重新安装 opener-local popup marker，也不会
  把 target 的正常 top-level Window 行为重新路由回 lightweight owner；
- protocol 端到端回归不是比较 metadata：popup session 在真实 auxiliary realm 写入 global
  与 `document.body`，opener 保存的 handle 必须读取同一值；opener 再经该 handle 写入，popup
  session 必须反向观察到。这证明双方使用同一个 stable proxy/inner realm projection，而不是
  两个 proxy 的 mirror synchronization。

本纵切当前的聚焦证据：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(related_page_script_agent_transfers_stable_window_proxy_objects_and_dom_wrappers)' \
  --no-fail-fast
# 2 passed

cargo nextest run -p moli-protocol \
  opener_window_handle_projects_the_renderer_owned_auxiliary_realm \
  --no-fail-fast
# 1 passed

cargo nextest run -p moli-protocol \
  window_open_hands_off_session_storage_snapshot_and_initial_storage_key \
  --no-fail-fast
# 1 passed；现阶段仍明确验证 mirrored loader 的双 owner characterization

cargo nextest run -p moli-protocol \
  window_open_named_target_reused_in_same_command_emits_one_page_event \
  --no-fail-fast
# 1 passed；覆盖 facade intrinsic registry 与 named reuse

cargo nextest run -p moli \
  webdriver_classic_execute_script_round_trips_window_and_frame_references \
  --no-fail-fast --stress-count 5
# 5/5 passed；覆盖 handoff 后 popup Window reference identity 回查

cargo nextest run -p moli-renderer-v8 \
  owner_scheduler_applies_popup_terminal_from_stable_page_route \
  --no-fail-fast --stress-count 5
# 5/5 passed；覆盖 facade token、opener projection 与 mirrored terminal application

cargo nextest run --no-fail-fast
# 15833 passed, 18 skipped
```

这仍不是 Phase 3 完成标志。同步调用期间 `popup.document` 仍由 opener PageVM 的
`LightweightPopupDocumentRecord` 提供；protocol 接管后 stable proxy 已投射到真实 auxiliary
realm，但此前的 DOM mutation 还不会成为 target initial Document。lightweight record 也仍
拥有 Document/navigation/close 状态，并对 non-empty URL 发起 mirrored load。下一纵切必须让
真实 auxiliary Page environment 从同步创建起就拥有唯一 initial realm/Document，删除对应
lightweight Document owner；之后 Phase 4 才能把 pending non-empty URL 收敛为一次
authoritative navigation。

#### Phase 3 第三纵切 B：in-scope main realm prebootstrap 基础

本提交先解决上一节下一步的 reentrancy 前置条件，尚未改变 popup production ownership：

- `window.open()` native callback 执行时，opener `ScriptVm` 已通过 document-isolate holder
  持有 `OwnedIsolate` 的可变借用。此时再次调用 `PageVm::new()` 会重入同一个 `RefCell`；临时
  释放借用、重入 owner loop 或从裸 isolate pointer 再造 scope 都不满足本文的不变量；
- main default realm 现在和 child default realm 一样有显式 in-scope primitive。
  `ScriptVmContextBootstrap::new_main_default_in_scope()` 接受调用方已经进入的
  `PinScope` 与 isolate global template，在同一 scope 内创建真实 V8 `Context`、安装 stable
  main WindowProxy、native bridge、runtime token 和完整 Window surface；
- `ScriptVmPageRealmBootstrap` 把这一步产出为
  `ScriptVmPreinspectorDefaultWorldBootstrap`。在该边界，独立 `DocumentRuntime` /
  `JsContextHost`、main Document resource authority、Window execution-context registration
  和 baseline globals 已经就绪，author script 可以同步访问该 realm；但 Inspector default
  context 尚未发布；
- callback/outer scope 退出后，`materialize_default_inspector_context()` 才借用 isolate-level
  Inspector backend，把同一个 Context 注册到对应 Page binding。它不创建第二个 Context、
  不 detach/reattach WindowProxy，也不重建或复制 Document；
- 普通 Page、replacement Document 和现有 related auxiliary target 的创建已经全部经过这份
  两段式实现，只是立即连续执行两段，因此现有 production event/ownership surface 不需要
  旁路；下一提交可以在两段之间暂存 renderer-owned auxiliary Page realm；
- 新回归在持有 shared isolate owner borrow 且已经进入模拟 opener Context 的情况下直接调用
  in-scope prebootstrap。Inspector registry 必须仍为 `0`；随后在预创建 realm 中保存
  `document` / `Array` identity 并写入 `document.body`，后置 materialization 后 registry 必须
  精确变为 `1`，且两个 identity 与 DOM mutation 全部保持。

聚焦证据：

```bash
cargo nextest run -p moli-renderer-v8 \
  main_default_realm_prebootstrap_preserves_window_and_document_until_inspector_materialization \
  --no-fail-fast
# 1 passed

cargo nextest run -p moli-renderer-v8 \
  -E 'test(main_default_realm_prebootstrap_preserves_window_and_document_until_inspector_materialization) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(related_page_script_agent_transfers_stable_window_proxy_objects_and_dom_wrappers) | test(/script_vm::inspector_pause::tests/) | test(/script_vm::inspector::tests::replacement_document_binding_does_not_adopt_previous_agent_outbound/) | test(/script_vm::inspector::tests::dropping_overlapping_peer_binding_does_not_deactivate_current_agent/)' \
  --no-fail-fast
# 16 passed

cargo nextest run -p moli-protocol \
  -E 'test(opener_window_handle_projects_the_renderer_owned_auxiliary_realm) | test(window_open_hands_off_session_storage_snapshot_and_initial_storage_key) | test(window_open_named_target_reused_in_same_command_emits_one_page_event)' \
  --no-fail-fast
# 3 passed

cargo nextest run --no-fail-fast
# 15834 passed, 18 skipped
```

这仍不是 Phase 3 完成标志：`window.open()` 尚未创建
`ScriptVmPreinspectorDefaultWorldBootstrap`，protocol initial build 也尚未消费这份 staged
realm/Page residence。下一纵切应让 related auxiliary reservation 同步准备独立 DomHost、
Page task routes、resource/storage authority 和这份 in-scope realm，再由 protocol target 只做
Inspector/target adoption；不能把 lightweight DOM replay 到新 Page，也不能重新创建一份
initial Document。

#### Phase 3 第三纵切 C：in-scope related-agent admission 基础

这一提交继续收窄 native callback 内剩余的 isolate holder 重入点，仍不改变 production
popup 的 Document owner：

- `RendererScriptAgentPageMembership` 现在是 admission authority。只有仍 active 的 source
  Page membership 能为一个明确的 target Page route 调用 `admit_related_page()`；调用方不再
  为了取得 holder 内的 router 而借用 document isolate。普通 owner-lane related Page build
  也改走同一能力，避免同步路径与既有异步路径形成两套 admission 规则；
- `RendererDocumentIsolateBootstrap` 和稳定 `RendererPageScriptEnvironment` 缓存同一份
  `RendererInspectorIsolateBackendHandle`。创建 target Page binding 不需要在 callback 栈上
  重新进入 holder 读取 backend；handle 仍不暴露任何 V8/Inspector mutation authority；
- `NativeBridgeBindings::build_peer_in_scope()` 只复用 source isolate 的 Window global /
  cross-origin Window global templates，并在 caller 已进入的 `PinScope` 内重建独立 bridge、
  wrapper templates 和 cache。target `JsContextHost` 不会共享 opener 的 mutable bridge state；
- `RendererPageScriptEnvironment::bootstrap_related_page_document_isolate_in_scope()` 把上述两份
  capability 合并成 callback-safe bootstrap。失败或尚未 adoption 时，RAII membership 会撤销
  target Page route，不会给 shared script agent 留下幽灵成员；
- 回归在 holder 已持有 mutable borrow 且 opener Context 已进入时执行这份 admission，再用
  新 bindings 创建独立 target Context。它同时验证 exact isolate identity、target Page id、
  page-count `1 -> 2 -> 1` 和未 adoption bootstrap 的 rollback；旧实现会在读取 router 或
  Inspector backend 时直接触发 `RefCell` 重入。

聚焦证据：

```bash
cargo nextest run -p moli-renderer-v8 \
  related_page_isolate_admission_builds_peer_bindings_inside_entered_opener_scope \
  --no-fail-fast
# 1 passed

cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_isolate_admission_builds_peer_bindings_inside_entered_opener_scope) | test(main_default_realm_prebootstrap_preserves_window_and_document_until_inspector_materialization) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(related_page_script_agent_experiment_shares_isolate_and_survives_source_close) | test(related_page_script_agent_transfers_stable_window_proxy_objects_and_dom_wrappers)' \
  --no-fail-fast
# 5 passed

cargo nextest run -p moli-protocol \
  -E 'test(opener_window_handle_projects_the_renderer_owned_auxiliary_realm) | test(window_open_hands_off_session_storage_snapshot_and_initial_storage_key) | test(window_open_named_target_reused_in_same_command_emits_one_page_event)' \
  --no-fail-fast
# 3 passed

cargo nextest run --no-fail-fast
# 15835 passed, 18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# both passed
```

这仍不是 Phase 3 完成标志。in-scope capability 目前只在窄回归中直接调用；production
`window.open()` 仍先创建 facade，owner 后续才创建 target `DomHost` / Page task residence。
下一提交应在 related auxiliary reservation 上同步创建并暂存 exact task sources、resource /
storage authority 和 `ScriptVmPreinspectorDefaultWorldBootstrap`，然后让现有 initial Page build
消费它并仅 materialize Inspector/target ownership。不能为消除 facade 而把 lightweight DOM
内容 replay 到第二份 Document。

#### Phase 3 第三纵切 D：synchronous Page residence / exact initial Document adoption

本提交完成了上一节定义的 production 纵切，但有意把准入范围限制在最能证明 identity 与
ownership 的 initial-empty 路径：

- 必须由 live opener Page 产生 `RelatedAuxiliaryPage` reservation；
- `noopener` / `noreferrer` 不进入，因为它们是 `Fresh` agent 且调用方也不应取得同步
  Window reference；
- target 必须是空字符串或 `_blank` 等非命名 target。可追踪 name 仍留给 group-level named
  registry 纵切，不能先创建一份无法复用的真实 Page；
- URL 必须是 `about:blank`，允许 fragment。non-empty URL 仍由 Phase 4 负责唯一 authoritative
  navigation，不能在本提交中继续保留 mirrored request 又声称 owner 已统一。

同步 callback 内的新拓扑是：

```text
opener PageVm / shared related script agent
    |
    | window.open("about:blank", "_blank")
    v
owner-local staged auxiliary PageVm
    +-- stable WindowProxy（立即返回给 opener）
    +-- independent V8 Context / inner Window
    +-- unique DomHost / initial Document
    +-- Page task sources / lifecycle / resource authority / storage authority
    +-- unopened Inspector binding + Page output journal
    |
    | exact RendererPageReservationToken
    v
protocol popup target
    +-- adopt 上述同一 PageVm / Context / Document
    +-- 只补 frame、session、Inspector 与 target configuration
```

具体 ownership 与同步语义如下：

- `open_lightweight_popup_window()` 在 legacy record 创建之前识别上述准入条件。它捕获 creator
  base URL、policy container、document referrer、inherited origin、request client、local /
  session storage、top-level storage key、IndexedDB 与 bucket authority；随后创建 detachable
  stable WindowProxy shell 和 canonical initial HTML DomHost；
- inherited origin 不再从 `about:blank` URL 重新算成 `"null"`。创建输入在 realm bootstrap 前
  安装到 root `DocumentRuntime` policy container 和 `DocumentFetchContext`，Window 的 runtime
  origin slot 也在 target Context 内同步覆盖。`Location.origin` 对 `about:blank` 读取该 inherited
  runtime origin；root `Document.referrer` 直接读取同一 policy container；default runtime realm
  inventory 也读取 Document resource authority，而不再从 URL 重算 origin。HTTP opener 回归同时
  验证 `window.origin`、`location.origin`、`document.referrer`、fallback `baseURI` 和 target
  `Runtime.executionContextCreated.origin`；
- `_blank` 是选择关键字，不是 browsing-context name。真实 realm 的 `window.name` 因此初始化为
  空字符串；不能把传入的 `_blank` 写进 target Window；
- source `JsContextHost` 不再保存对应 `LightweightPopupBrowsingContextRecord`。否则 opener host
  会通过 record 中的 `Global<WindowProxy>` 反向强持有 target realm，形成第二 owner，并使 target
  close 后的 realm containment 无法证明。protocol handoff 需要的 session-storage snapshot 与
  initial storage key 改为 `OpenedLightweightPopup` 上的一次性 carrier；
- Classic WebDriver 仍需把 opener 返回的 WindowProxy 编码成随后创建的 window handle。真实
  target Window 因此只带一个独立的 V8 private auxiliary-popup identity；host serializer 可以
  识别它，但 author script 无法伪造。这个 marker 不提供 Document、navigation、timer 或 close
  ownership，也不会让现有 lightweight API 把真实 Window 重新路由到 opener host；
- owner-local store 以 `(RendererOwnerLocalHostId, PageId)` 暂存完整 `PageVm`，而不是暂存可被
  replay 的 DOM snapshot。它在已经进入的 opener isolate scope 内创建 exact Page task source、
  typed producer routes、output stream、Page Inspector binding、related-agent membership、peer
  native bindings 和 pre-Inspector default realm；
- callback 内不能重新借用同一 document-isolate holder。无 restore session 时 bootstrap 不再
  为一次空 reattach 进入 holder；IndexedDB / bucket 初始 backend 也直接写入当前 target Context
  与 host。所有需要 V8 scope 的初始化都在 caller scope 完成，退出 callback 后才允许普通
  owner command 再进入 holder；
- staged initial Document 同步设为 `readyState=complete`，并用 typed lifecycle transition 记录
  DCL/load 已达成。后续空 lifecycle turn 识别“里程碑已完成且没有 work”并返回 Idle，不依靠
  sleep、retry、任意 drain 或重复 dispatch 修正状态；
- isolate reservation 在 `PageVm` construction 的异常区间暂时 disarm，避免失败析构递归借用
  当前 bound owner-local store；成功后 rearm，交回正常 Page lifetime。owner-local store 析构
  会先 drain staged Page，再显式撤销 reservation，避免 source Page 先析构时留下 related-agent
  membership 或 Inspector route。

protocol initial target build 现在是 adoption，而不是第二次 bootstrap：

- initial request 必须仍是匹配的 `about:blank`；owner command 用 exact reservation 一次性取走
  staged `PageVm`。protocol 为旧 creation path 预留的 service-worker client 被释放，保留同步
  Page 已经创建的 client；
- target 提供 root frame id、main-document commit、Inspector session restore、isolated worlds、
  bindings 和最终环境配置。adoption 把 lifecycle journal 绑定到 Page output stream，并在同一
  V8 Context 上 materialize Inspector；它不创建 Context、WindowProxy、Document 或 DomHost；
- opener 在 target activation 前已经可以改 DOM、设置 global、访问 storage 或产生 renderer
  observation。output journal 暂存尚未发布的 author records，adoption 先插入
  `executionContextsCleared -> frame commit -> contextCreated` 前缀，再原序追加这些 records；一旦
  stream prefix 已发布就 fail closed，不能用乱序事件掩盖 ownership 错误；
- adoption 只替换 protocol 提供的 request transport / network policy 和 Page-level runtime
  configuration，随后直接进入现有 Page creation phase two。环境应用模式显式区分普通创建、
  staged 创建和 staged adoption：adoption 不得用 synthetic `about:blank` response 的空 CSP、
  referrer policy、COEP 或 Document-Isolation Policy 覆盖 creator-derived policy container；
  sandbox 导出的 script-disable 只能保持或收紧，不能被 target 默认配置放宽（显式 CDP
  `Page.setBypassCSP` 仍是独立的调试器能力）。同步保存的 `Document` object、body mutation、
  global lexical/realm state、WindowProxy 和 Page id 全部保持原对象。

核心回归不再只比较 metadata：

- `opener_window_handle_projects_the_renderer_owned_auxiliary_realm` 在 opener 的同一次
  `Runtime.evaluate` 中保存 exact `popup.document`，写入 target global 与 body，并检查 name、
  origin、referrer 和 base URL；attach target 后验证 `window.opener` 保存的 Document 就是当前
  `document`，同步 realm/global/DOM 状态全部存活；target 再修改 DOM/global，opener 必须经原
  WindowProxy 和原 Document 看到变化，反向 proxy mutation 也必须成立；
- `window_open_hands_off_session_storage_snapshot_and_initial_storage_key` 使用 HTTP opener，证明
  非 opaque creator origin、referrer、base URL、localStorage 共享和 sessionStorage clone 在
  target adoption 前后保持一致；它也在同步 WindowProxy 与 attach 后的 exact target realm 两侧
  验证 creator response CSP 继续拒绝 `eval`。后者显式设置 CDP
  `allowUnsafeEvalBlockedByCSP=false`，避免把调试器默认的临时 CSP 豁免误判为 policy 丢失；
- Classic WebDriver round-trip 覆盖多个顺序不同的 `about:blank#fragment` popup。真实 proxy 的
  private target identity 必须稳定映射到 window handle，重复引用不能退化成循环对象 clone。

本纵切的聚焦与 repository gate 实跑证据：

```bash
cargo nextest run -p moli-protocol \
  -E 'test(opener_window_handle_projects_the_renderer_owned_auxiliary_realm) | test(window_open_hands_off_session_storage_snapshot_and_initial_storage_key) | test(popup_initial_empty_document_frame_tree_inherits_opener_origin) | test(popup_initial_empty_document_record_captures_creator_identity) | test(rust_cdp_chromium_target_window_open_empty_url_creates_about_blank_popup) | test(window_open_named_target_reused_in_same_command_emits_one_page_event)' \
  --no-fail-fast
# 6 passed

cargo nextest run -p moli \
  webdriver_classic_execute_script_round_trips_window_and_frame_references \
  --no-fail-fast
# 1 passed

cargo check -p moli-renderer-v8 --all-targets
cargo check -p moli-protocol --all-targets
# passed

cargo nextest run --no-fail-fast
# 15835 passed, 18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# both passed
```

第三纵切 D 单独完成的是 Phase 3 的窄 initial-empty owner 不变量，不是 popup 完成标志。
该提交当时明确保留的缺口如下；其中 non-empty URL 的首个 owner 纵切已由下一节继续收敛：

- named target 与 `noopener` 仍走 legacy creation；non-empty URL 在该提交时也走 legacy，
  下一节 Phase 4 第一纵切 A 已只迁移保留 opener 的非命名、非 `javascript:` 路径；
- target 级 `close()` / `window.close()` 还没有统一 transaction，真实 target Window 也不能因
  一个旧 lightweight close callback 被误认为已关闭；
- protocol 的 document-start scripts、isolated worlds 和 runtime bindings 到 adoption 时才可得，
  尚未证明它们相对 opener 的同步 initial-Document mutation 具有 Chromium 一致的顺序；
- target activation 前启动 timer/fetch、触发 modal dialog 或发布 Inspector-sensitive output 的
  scheduler 边界尚缺专门回归。当前 output journal 对已发布 prefix 拒绝 adoption，这是安全门，
  不是这些时序已经完成的证据；
- initial request 与 staged URL 不匹配时会 fail closed；staged residence 目前依赖 owner teardown
  作为最终清理安全网，后续应增加不递归借用 owner store 的 eager reject/retire transaction；
- initial DomHost 继续沿用本项目 child initial-empty 的完整 HTML tree 约定。是否需要进一步对齐
  Chromium 对 doctype / parser state 的细节，应由 WPT/Chromium probe 决定，不能在 identity
  纵切中凭印象改树形。

### Phase 4：non-empty URL 单一导航

- pending URL 只交给 auxiliary Page owner；
- 接入 target admission / wait-for-debugger；
- Fetch/Network interception 绑定同一个 navigation token；
- 删除 lightweight mirrored load；
- 把“exactly two load owners”测试改为“exactly one authoritative navigation owner”；
- 验证 redirect、204/205、error page、history、DCL/load/done 和 opener immediate
  mutation。

完成标志：同一个 popup URL 不再因为实现结构产生两个请求。

#### Phase 4 第一纵切 A：non-named related popup 的唯一 navigation owner

本提交把上一节的 exact initial Page residence 扩展到保留 opener、非命名、URL 可解析且
scheme 不是 `javascript:` 的 `window.open()`。它解决的是最直接的双请求 owner，不把尚未
迁移的 name/group/opener policy 或全部 navigation terminal 语义混入同一提交。

renderer 同步路径现在遵守以下边界：

- non-empty destination 不是 initial Document URL。同步 callback 始终构造真实
  `about:blank` initial Document（显式 `about:blank#fragment` 仍保留其 fragment），继承
  creator origin、policy、referrer、base URL、storage authority 和 stable WindowProxy；
- stable WindowProxy shell 只负责 identity handoff；opener 的 `innerWidth`、`innerHeight`、
  `outerWidth`、`outerHeight` 和 `devicePixelRatio` 数值 surface 会在真实 target Context
  初始化时复制到最终 inner Window。把这些值只写到临时 facade 会在 realm handoff 时丢失，
  已由 Chromium/WPT 移植的 BiDi user-context viewport 回归覆盖；
- requested destination 只保留在 immutable `RendererPendingPopupActivation` 中。同步返回后，
  opener 可以立即修改 popup global 与 `document.body`；source `JsContextHost` 不创建
  `LightweightPopupBrowsingContextRecord`，也不调用
  `start_lightweight_popup_document_load()`；
- protocol 先用 exact `RendererPendingAuxiliaryPage` materialize/adopt 上述 initial Page，发布
  target/attach/Inspector ownership，然后才把 requested URL 变成绑定该 target residence 的
  `PopupTargetNavigationOwnerAction`。后续 fetch、response、replacement commit、lifecycle 和
  generation 继续走现有 stable Page navigation path，没有新增 protocol loader；
- `waitForDebuggerOnStart` 是明确 admission gate：等待期间 target session 可以观察 initial
  realm，但 destination 请求数必须为零；`Runtime.runIfWaitingForDebugger` 之后才释放这一份
  target-owned navigation；
- eligible staging 失败时不再 fall through 到 legacy lightweight loader。调用方失去同步 proxy
  并由普通 target fallback 继续处理，比悄悄恢复两个 authoritative owner 更安全；正常
  production owner-local path 由聚焦回归证明可成功 staging。

`window_open_hands_off_session_storage_snapshot_and_initial_storage_key` 现在同时覆盖两段：

1. initial `about:blank` adoption 前后保持 exact Document、origin/referrer、storage 与 CSP；
2. gated HTTP non-empty popup 在 admission 前保存 target proxy/Document/global/body mutation，
   auto-attached target session 必须看到同一对象；等待调试器时服务端计数为 `0`，resume 后
   完成 cross-origin replacement，最终计数严格为 `1`。旧注释与断言中的 two owners 已删除。

本纵切的实跑聚焦证据：

```bash
cargo nextest run -p moli-protocol \
  -E 'test(window_open_hands_off_session_storage_snapshot_and_initial_storage_key) | test(opener_window_handle_projects_the_renderer_owned_auxiliary_realm) | test(rust_cdp_chromium_target_window_open_blank_creates_popup_target) | test(rust_cdp_chromium_target_window_open_auto_attached_popup_materializes_initial_document) | test(rust_cdp_chromium_target_window_open_waiting_popup_routes_initial_document_after_resume) | test(window_open_emits_popup_target_created_from_runtime_work) | test(rust_cdp_chromium_target_window_open_javascript_url_still_reports_popup_target) | test(window_open_named_target_reuses_existing_popup_target) | test(rust_cdp_chromium_target_window_open_empty_url_creates_about_blank_popup) | test(popup_initial_about_blank_adopts_renderer_page_and_related_script_agent)' \
  --no-fail-fast
# 10 passed

cargo nextest run -p moli-renderer-v8 \
  -E 'test(window_open_non_about_returns_lightweight_popup_and_dispatches_load) | test(window_open_named_lightweight_popup_reuses_without_recloning_session_storage)' \
  --no-fail-fast
# 2 passed；standalone / named legacy 边界未被 production admission 改写

cargo nextest run -p moli \
  websocket_bidi_set_viewport_user_context_inherits_through_window_open \
  --no-fail-fast
# 1 passed；同步 stable WindowProxy 与 navigation 后 target 都继承 user-context viewport

cargo nextest run -p moli-renderer-v8 \
  window_open_lightweight_popup_inherits_opener_viewport_surface \
  --no-fail-fast
# 1 passed；保留 legacy fallback 的 viewport 行为

cargo nextest run --no-fail-fast
# 15837 passed，18 skipped（rebase 到 origin/master f16860e4fb 后）

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# both passed
```

#### Phase 4 第二纵切 B：Page-residence-bound navigation claim

第一纵切已经消除了 renderer/protocol 双 loader，但当时的
`PopupTargetNavigationOwnerAction` 只冻结 browser context id、target id 和 URL。target 与 CDP
session 可以跨 renderer Page replacement 存活，因此 target id 不是足够的导航权限；此外
`waitForDebuggerOnStart` 恢复路径仍会从当前 target URL 重新推导一次 initial navigation。旧
activation 若晚于 Page replacement 到达，就可能把属于 initial Page 的 destination 应用到新
residence。

本纵切把 navigation admission 收敛为以下状态机：

```text
RendererPendingPopupActivation
  -> capture exact target route + TargetPageResidenceIdentity + frozen URL
  -> TargetRuntimeSlot::Held(action)
       immediate admission       -> Published(claim) -> scheduler owner action
       waitForDebugger admission -> Published(claim) -> runIfWaitingForDebugger owner turn
  -> validate exact route + target + loaded_page_generation
  -> Consumed(claim) tombstone
       current -> one Page navigation
       stale   -> drop; never rescan target URL
```

具体边界如下：

- target creation 在发布 `Target.targetCreated` / attach lifecycle 前捕获 exact
  `TargetPageResidenceIdentity` 并把 initial destination stage 到该 target 的
  `TargetRuntimeSlot`；捕获或 staging 失败会回滚不完整 target，不能留下“只有 URL、没有 owner”
  的半接受状态；
- `PopupTargetNavigationClaimIdentity` 同时冻结 Page residence/generation、browser context、
  concrete target、URL 和 navigation kind。普通 admission 只把这一个 move-only action 发布给
  protocol scheduler；named-target reuse 暂时仍走 legacy group policy，但其既有 action 也获得
  相同的 Page-generation currentness 检查；
- `waitForDebuggerOnStart` 期间 action 保持 `Held`，`Page.enable` 和
  `Page.createIsolatedWorld` 不能触发 destination。`Runtime.runIfWaitingForDebugger` 是明确的
  target-owner admission turn：它把同一 action 变成 `Published` 后直接消费，并保留触发恢复的
  explicit popup session 作为 execution attachment，使 Fetch pause/fulfill 与后续 lifecycle 都
  继续路由到同一个 session；这里没有重新读取 target URL；
- completion 先把匹配的 `Published` claim 原子变成 `Consumed`，再检查 exact
  `TargetPageResidenceIdentity`。即使检查发现 Page generation 已变化，`Consumed` tombstone 也
  保留在 target slot；所有通用 initial-navigation 入口看到 `Held`、`Published` 或 `Consumed`
  都会拒绝从 target URL 制造第二份工作；
- Page replacement 不清除这份 authority。这样旧 action 必须在新 generation 上 fail closed，
  而不是因为状态被清空后被 generic fallback 重新解释；target teardown 则连同整个 slot 一起
  回收它；
- admission action 和其内部 Page navigation 都沿用现有 `Box::pin` orchestration 边界。若把
  这两个大 async state 直接内嵌进通用 initial-navigation future，即使普通
  `Target.createTarget` 不走 popup 分支，也会放大 Tokio worker 的栈布局；target-creation
  storage fan-out 回归把这个非业务分支的栈边界一并锁住。

聚焦回归分别证明正常 action、stale generation 和 debugger admission：

```bash
cargo nextest run -p moli-protocol \
  -E 'test(local_storage_mutations_fan_out_across_targets_without_leaking_session_storage) or test(popup_navigation_owner_action_rejects_replaced_page_and_cannot_be_rescanned) or test(rust_cdp_chromium_target_window_open_waiting_popup_routes_initial_document_after_resume) or test(popup_activation_creates_target_and_schedules_navigation_without_page_readback)' \
  --no-fail-fast
# 4 passed

cargo nextest run --no-fail-fast
# 15839 passed，18 skipped（rebase 到 origin/master c597ac97dc 后）

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# both passed
```

其中 stale 回归在 action 发布后推进 target 的 `loaded_page_generation`，随后证明：旧 action 不
产生 navigation/event、新 Page 仍是 `about:blank`，再次调用通用 target-URL initial-navigation
入口也不能复活该 destination。debugger 回归同时证明 waiting 期间请求数为零，resume 后只有
同一 explicit popup session 的一份 Fetch/Network 请求，并能完成 replacement lifecycle。
第一次全量运行让未 boxed 的 nested future 在上述 DOMStorage target-creation 回归中稳定触发
stack overflow；单独复现后恢复 heap-boxed orchestration 边界，4 个聚焦用例与第二次全量均
通过，因此没有把它归类为 flaky。

#### Phase 4 第三纵切 C：no-commit HTTP terminal 与 initial Document history

204/205 不是普通 transport failure，也不是一个可交给 renderer 构造新 Page 的空 HTML
response。它们已经收到 HTTP response，但导航必须以“不提交 Document”结束。旧实现把二者
继续送入 response-stage preparation / `Page` construction，因而可能替换 popup 的 initial
Document、丢掉 opener 同步写入的 realm/DOM 状态，并错误发布 `frameNavigated`、DCL、load 或
`loadingFinished`。只在 popup 调用点绕过 commit 也不够：streaming、buffered、Fetch
response-stage 和 background `Page.navigate` 会从不同入口到达同一个 load outcome 边界。

Chromium 对照给出的是一个明确的两层合同：

- `content/browser/renderer_host/navigation_request.cc` 将 204/205 归为不可 render/commit 的
  response，并中止这次 navigation；`navigation_controller_impl_browsertest.cc` 对应回归证明
  不会新增 NavigationEntry；
- `third_party/blink/renderer/core/dom/document.h` 用 `Document::IsInitialEmptyDocument()` 保存
  Document 身份，而不是从 URL 推断；`frame_loader.cc` 和
  `document_loader.cc::UpdateForSameDocumentNavigation` 在 URL/history update 步骤把 initial
  empty Document 上的标准导航转换为 replacement。fragment、`history.pushState()` 和
  `history.replaceState()` 不会让该 Document 失去 initial 身份，`document.open()` 则会显式
  退出；
- WPT `initial-empty-document/window-open-204-fragment.html`、
  `window-open-204-pushState-replaceState.html` 和 `window-open-history-length.html` 把这些
  行为连成同一个矩阵：204 后仍是原 initial Document，same-document 更新后
  `history.length == 1`，下一次成功 cross-document navigation 仍替换该唯一条目；
- inspector-protocol 的 `page/navigate-204.js` 和
  `network/navigation-204-loading-failed.js` 要求 `Page.navigate` 返回
  `net::ERR_ABORTED`，Network 顺序为 `responseReceived → loadingFailed(canceled=true)`，而不是
  `loadingFinished`。

Moli 现在把这条语义放在公共 navigation terminal 边界：

```text
HTTP response head (204/205)
  -> publish response metadata / redirect hops
  -> NavigationLoadOutcome::NoCommitResponse
  -> CompletedNoCommitResponseProgressTransfer
  -> FailedNavigationDocumentPolicy::PreserveCommittedDocument
     + FailedNavigationHistoryPolicy::RetainInitialEmptyDocumentReplacement
     + FailedNavigationResponseMode::CdpErrorTextResult
  -> responseReceived
  -> loadingFailed(errorText = net::ERR_ABORTED, canceled = true)
```

具体责任边界如下：

- `NavigationLoadOutcome::NoCommitResponse` 是独立 typed outcome，携带 final URL 和已经完成的
  main-document progress transfer。它不复用 `NetworkFailure(String)`：后者仍保留现有的 failed
  navigation / Document invalidation policy，避免在尚未决定 error-page 设计前悄悄改变普通网络
  错误；
- streaming response 和 captured/buffered response 都在 prepared-Document/Page construction
  之前识别 204/205；Fetch response-stage preparation 也拒绝为它们准备 Page。background
  `Page.navigate` 不再提前发送 success result，因此 terminal owner 能返回 Chromium 形状的
  `{frameId, loaderId, errorText: "net::ERR_ABORTED", isDownload: false}`；
- no-commit progress 先保留 response/redirect 元数据，再用同一 request id 发布 canceled
  `loadingFailed`。由于没有 renderer DCL/load boundary 可以在后续 turn 解锁 body phase，terminal
  turn 会显式让 response/body-failed 两阶段可见，但仍通过同一个 progress queue 保证源顺序；
- materialization 使用 `PreserveCommittedDocument`：popup 的 exact stable Page、WindowProxy、
  V8 Context、Document、global 与 body mutation 全部保持，且不会发布新
  `Page.frameNavigated`、DCL 或 load。failed response body 仍进入统一的 failed-body bookkeeping，
  不制造“协议终态完成但 body owner 悬空”的旁路；
- browser-owned initial history 在 popup 创建边界显式 stage
  `ReplaceInitialEmptyDocument`。no-commit terminal 不从“当前 URL/Document 看起来像 initial”反推
  新意图，而是只保留已经 pending 的 initial replacement；reload、traverse 和普通 append 的
  pending update 都会丢弃。这样先后遇到 204、205 后，下一次成功导航仍替换 popup 的唯一条目，
  同时普通顶层 `about:blank` 的首次 `Page.navigate` 仍按既有 Chromium 合同追加；
- renderer-owned history 在 `JsContextHost` 上保存持久的 root initial-Document bit。related
  auxiliary Page 在原 stable realm 构造时设置它，fragment、`pushState`、Navigation API
  same-document mutation 和后续 cross-document seed 都据此转换为 replacement；URL 变成
  `about:blank#...` 后仍然正确。`document.open()` 在 root Document replacement owner 边界清除
  该 bit，与 Blink 的 Document 合同一致；
- 后续 redirect success 继续走既有 replacement Page path。integration matrix 要求 redirect
  每个 hop 恰好一条 `requestWillBeSent`、共享 request id、后续 hop 携带前一跳
  `redirectResponse`，最终只出现一次 `frameNavigated`、一次 DCL 和一次 load，并把 initial realm
  替换为一个 history entry。

本纵切新增的 end-to-end popup 用例从 `waitForDebuggerOnStart` 的真实 initial Page 开始：等待时
写入 opener global/body marker；resume 后完成 204；依次执行 fragment 和 `pushState`；再执行
205；最后导航到 redirect chain。它同时检查每个 no-commit terminal 的 CDP 形状、事件顺序、
Document/realm identity、browser/renderer history projection，以及成功 replacement 的请求和
lifecycle 基数。renderer 附近的回归单独锁住 initial bit 对 fragment、`pushState`、cross-document
seed 和 `document.open()` 的作用；progress queue 与 browser history owner 也各有边界测试。

聚焦与全量验证：

```bash
cargo nextest run -p moli-protocol --no-fail-fast \
  -E 'test(completed_no_commit_response_progress_orders_response_before_canceled_failure) | test(active_target_initial_empty_document_record_tracks_navigation_lifecycle) | test(rust_smoke_fixture_serves_navigation_no_commit_routes) | test(page_navigate_network_failure_invalidates_previous_document) | test(popup_no_commit_responses_preserve_initial_document_before_redirect_replacement)'
# 5 passed

cargo nextest run -p moli-protocol --no-fail-fast \
  -E 'test(navigation_history_marks_reload_as_reload_transition) | test(navigation_history_supports_playwright_back_forward_commands) | test(navigated_within_document_matches_chromium_mixed_history_sequence) | test(navigation_history_is_preserved_per_parked_target) | test(renderer_history_back_uses_browser_owned_navigation_history) | test(rust_cdp_capability_page_navigation_history_round_trip)'
# 6 passed

cargo nextest run -p moli-renderer-v8 --no-fail-fast \
  -E 'test(root_initial_empty_document_replaces_same_and_cross_document_history_updates) | test(document_open_exits_root_initial_empty_history_replacement_mode) | test(window_open_204_popup_ignores_navigation_and_preserves_initial_empty_history) | test(window_open_without_url_replaces_initial_empty_history_on_first_navigation)'
# 4 passed

cargo nextest run --no-fail-fast
# 15844 passed，18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# both passed
```

第一次全量运行暴露 6 个稳定可复现的 browser-owned history 回归：当时实现只要 target 仍在
initial empty Document，就在每次 cross-document start 无条件重新 arm replacement，导致普通顶层
`about:blank` 的首次导航、reload、traverse 和 parked-target history 少一个 entry。6 个用例聚焦
复跑为 0/6，因而没有归类为 flaky。最终实现把意图来源收回 popup 创建边界，并让 no-commit
terminal 只保留已经存在的 `ReplaceInitialEmptyDocument`；相同 6 个用例恢复为 6/6，第二次全量
为 15844/15844。

本纵切没有把以下问题伪装成已经完成：普通 DNS/connect/TLS/HTTP transport failure 应保留旧
Document、提交统一 error page，还是使旧 Document 失效，仍需下一纵切用 Chromium/CDP/WPT
矩阵明确命名 policy。当时尚缺的 Fetch fulfill/continue 204/205 interception 入口矩阵现已由
Phase 4 第五纵切 E 补齐；它也反向证明当时所谓“公共 response builder 已共享 classification”并
不完整，因为 response-stage synthetic buffered-body 入口仍存在一条直达 prepared Document 的旁路。

Phase 4 尚未完成，后续纵切仍必须补齐：

- 普通 network error/error-page 的 Document policy、CDP response shape、history、DCL/load/done
  和 opener-visible state 仍需专门 integration 矩阵；
- 该阶段的 named target、`noopener` / `noreferrer` 和 `javascript:` URL 仍由 legacy policy/path
  处理；其中新建非命名 noopener/noreferrer 后续已由 Phase 5E1 迁移，named/group 与
  `javascript:` semantics 仍未完成；
- target admission 前启动 timer/fetch/modal 或先关闭 popup 的行为仍需和 close transaction 一起
  定义，不能靠当前 output journal 的 fail-closed 门槛代替正常时序。

#### Phase 4 第四纵切 D：DocumentCommit / exact continuation owner boundary

第三纵切证明了 no-commit terminal 不应构造 replacement Document，但成功 response 的另一条
边界仍不够准确：renderer 在 parser 尚未走到 DCL 时已经拥有可用的 replacement realm 和
Document，protocol 也必须发布新的 execution context、允许 debugger/configuration 控制并返回
早期 `Page.navigate` 结果；与此同时，DCL、load、最终 title/history 和 renderer output 又不能
由 command future 的返回时刻推断。原实现把这些状态压在一个 async completion 里，不同入口会
出现两类相反错误：要么为等待最终 Page snapshot 而锁死 debugger/Fetch 控制命令，要么过早取
一份仍在变化的 PageState，并让旧 Document 的异步完成污染 replacement generation。

Chromium 对齐约束不是“所有命令都等到 DCL”。已经 commit 的 Document 即使 parser-blocking
script 仍在等待网络，也可通过其 replacement execution context 执行 `Runtime.evaluate`，此时
`document.readyState == "loading"`、`document.body` 甚至可以仍为 `null`。只有 attachment 尚在
replacement cutover 时，document-bound 命令才必须等待；`Runtime.addBinding`、preload、isolated
world 和 debugger resume 等配置/控制面还必须穿过这个 cutover，才能在第一段 author script
之前生效。DCL 是独立的 lifecycle target，不是 realm usability gate。

本纵切把成功 navigation 收敛成以下两段 owner transaction：

```text
response / prepared replacement
  -> DocumentCommit
     -> adopt exact stable Page residence + replacement realm/Document
     -> publish attachment / executionContextCreated / early navigate result
     -> release renderer attachment cutover
  -> RendererDocumentContinuationObserver (exact loader + generation)
     -> renderer owner reaches this Document's DCL target
     -> capture RendererOutputFence + Arc<RendererPageState> in the same owner turn
     -> typed Send completion lane
     -> project predecessor, then apply PageState iff target + loader are still current
     -> refresh history title / Target.targetInfoChanged
     -> later continue to exact Load lifecycle observation
```

核心实现边界如下：

- renderer 的 `RendererDocumentContinuationPublisher/Observer` 是一次性 typed terminal。publisher
  被安装到创建或 replacement navigation 的真实 owner continuation，terminal 同时携带 exact
  `RendererOutputFence` 和 `Arc<RendererPageState>`；两者来自同一个 owner turn，protocol 不再在
  fence 之后另发 snapshot command，因而不会跨 turn 或跨 generation 观察到不一致状态。publisher
  drop 也会显式产生 canceled terminal，receiver 不会永久悬挂；
- continuation target 固定为该 committed Document 的 DCL。phase-one producer 被网络、parser
  source、debugger pause 或 location navigation 挡住时只保留真实 owner turn，不再因为
  `DocumentCommit` reply 已经发出就提前 settle producer park。若 parser script 发起 replacement
  navigation，源 Document 的 continuation 会按 generation 终止，不能补发源 DCL；successor
  navigation 拥有自己的 token、loader 和 terminal；
- owner 恢复 live Page 时先取得同一 renderer stream 的 output fence，再 settle terminal。这样
  lifecycle、Inspector、popup/worker/child-frame publication 都先按 concrete cursor 进入 protocol
  scheduler，PageState 才能替换 protocol cache；`PageState` 只在 gate 的 target id、session owner
  route 和 loader id 仍与当前 Page 匹配时应用，旧 completion 无权修改新 target/runtime；
- CDP scheduler 为 continuation 使用独立的 Send receiver，不与 background navigation completion
  或 background event gate 混成同一含义。background navigation gate 负责 early navigate result
  之后的 load residence/event ordering；continuation gate 只表达 exact committed Document 的 typed
  terminal。Classic WebDriver、WebDriver BiDi 和 CDP actor 都消费相同 completion，不各自重建
  renderer wait；
- attachment cutover 和 DCL gate 使用不同 command policy。document-bound Runtime/DOM/CSS 等命令
  在 renderer attachment suspended 时仍等待 replacement commit；persistent configuration/control
  命令可跨过 suspension。commit 之后，`Runtime.evaluate` 等命令可访问仍为 `loading` 的当前
  Document；当前只有依赖 DCL PageState title projection 的 `Page.getNavigationHistory` 等待 typed
  continuation。这个拆分由 parser-blocking WebSocket CDP 回归锁住，不能再把宽泛的
  `waits_for_document_navigation_to_finish` 直接复用为 DCL gate；
- prepared commit configuration 会在 author script 前安装 document-start preload、isolated world
  和 Runtime binding。browser-internal bootstrap script 使用专门的 Inspector execution path：它不
  触发 instrumentation pause，并使用真实 replacement origin；author script 仍按普通 Debugger
  policy 发布和暂停。这避免为“先配置后执行”重放一份 Document 或临时关闭 debugger；
- typed PageState 应用后刷新 browser-owned current history entry 的 title，并基于 exact target
  delta 生成 `Target.targetInfoChanged`。active、background、inactive/parked target 都沿用 target
  owner identity；旧 loader 的 completion 即使稍后到达也只会被消费和丢弃，不能改写当前 title；
- protocol-neutral direct command 测试不再以“pending queue 暂时为空”代替 load。Navigate、Reload
  和 TraverseHistory 的 `wait: Load` 路径从 command result 提取 exact loader，注册
  `RendererDocumentLifecycleMilestone::Load` waiter，驱动 typed scheduler input 到 `Reached` 后再
  完成对应 main-Document load residence。child-frame navigation、input、`Target.createTarget`、
  Classic 和 BiDi lane 因而验证的是同一 lifecycle invariant，而不是某次 executor 恰好先跑完；
- `TestContext` 保留 production-shaped owner ordering：队首若是 fixture-only、缺少 background owner
  lane 的 popup action，可以让另一个 target 的 ready work 越过；同一 command 的 follow-up 也可
  越过尚未 ready 的旧 load residence。这个有限规则使 parser script 的 `location` successor 能
  接管 owner，源 DCL 被压制；它不是任意 drain/retry，也没有加入 sleep 或无限 polling。

本纵切的回归矩阵覆盖：stable Page background DCL→load continuation、parser script replacement
及独立 response gate、preload/world/binding 先于 author script、Debugger instrumentation
pause/resume、Fetch auth/response-stage、target discovery 与 title delta、browser-session target
route、auto-attach owner、Playwright script execution disable、history back/forward、popup
create/navigate/close、child-frame protocol-neutral navigation、direct input、worker/AudioWorklet
owner continuation，以及 Classic/BiDi completion routing。

聚焦验证证据：

```bash
cargo nextest run -p moli-protocol --no-fail-fast \
  --success-output never --status-level fail --final-status-level fail --show-progress none
# 3295 passed

cargo nextest run -p moli-protocol-cdp --no-fail-fast
# 8 passed

cargo nextest run -p moli \
  websocket_cdp_raw_client_runtime_evaluate_immediately_after_page_navigate_succeeds \
  websocket_cdp_runtime_control_command_waits_for_navigation_attachment_cutover \
  websocket_cdp_runtime_evaluate_uses_committed_page_while_parser_blocking_source_is_pending \
  --no-fail-fast
# 3 passed；同时锁住 pre-commit cutover 和 post-commit loading Document 两侧

cargo nextest run --no-fail-fast
# 15884 passed，18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

raw-CDP 立即 evaluate 回归不再把 body 已解析当作 attachment commit 的必要条件：它验证新
Document URL 和合法 readyState；若 readyState 已越过 `loading`，才要求 body link 已存在。这样
workspace 高并发下 continuation 尚未跑完不再被误判为 `NoDocumentLoaded`，而真正绑定旧
`about:blank` 或 attachment 尚不可用仍会失败。

以上是 rebase 前结果；若 rebase 改变 Rust 基线，必须在 rebase 后重复，而不能沿用这组结果。

本纵切仍不是 Phase 4 完成标志。除上一节列出的 network error、named/`noopener`/`javascript:`
矩阵外，target admission 前已经排队的 timer/fetch/dialog/Inspector output、`window.close()` 与
target close 的单一 transaction，以及 completion lane 关闭时的 production teardown 诊断仍需
后续切片完成。exact continuation 只建立了这些行为可依赖的 owner/lifecycle 基础，没有替它们
定义产品语义。

#### Phase 4 第五纵切 E：Fetch response-stage effective response terminal

第三纵切 C 已经证明直接网络 204/205 必须保留 popup initial Document，但当 response head 被
Fetch 拦截后，决定 navigation terminal 的不再只是服务器原始状态，而是 DevTools action 释放的
effective response。这个入口不能只补一条“原始 204 继续后仍失败”的容易路径；必须同时证明
override 能把可提交响应变成 no-commit，也能把原始 no-commit 响应变回可提交响应，否则实现仍
可能在原始 head 或 synthetic body 的某一侧过早作出不可逆决定。

Chromium 的责任链给出了明确合同：

- `third_party/blink/public/devtools_protocol/domains/Fetch.pdl` 要求
  `continueResponse` 修改 status 或 headers 时同时给出两者；全都省略则沿用原始 response head；
- `content/browser/devtools/protocol/fetch_handler.cc::ContinueResponse` 在完整 override 时直接复用
  `FulfillRequest` 构造新的 HTTP response head，不带 override 时才原样 continue；
- `content/browser/devtools/devtools_url_loader_interceptor.cc` 把 override 安装到下游可见的
  `URLResponseHead`，保留原 body 或替换 synthetic body。因而后续 navigation 看到的是 effective
  status/header，而不是一份只供 Network domain 展示的旁路 metadata；
- `content/browser/renderer_host/navigation_request.cc` 在处理这份 response head 时把 204/205 判为
  `response_should_be_rendered_ == false`，设置 `net::ERR_ABORTED` 并终止而不 commit；
- `content/browser/devtools/protocol/page_handler.cc::NavigationReset` 最终从同一个
  `NavigationRequest` 读取 net error，`Page.navigate` 因而返回 `errorText: net::ERR_ABORTED`。这也
  解释了为何不能先发 success，再在 Network domain 单独把请求标成 canceled。

本轮矩阵第一次运行确实发现了后一类分裂。streaming/captured 网络 builder 已在创建 replacement
Page 前识别 204/205，但 response-stage `Fetch.fulfillRequest` 使用的
`build_navigation_from_buffered_body_source_with_load_inputs_async` 自己重复了一份 Page reservation
和 prepared-document construction，直接把 synthetic response 包成 `ResponseCommitReady`。当原始
200 被 fulfill 为 204 时，旧实现会同时发布 `Network.responseReceived(status=204)`、成功的
`Page.navigate`、`Page.frameNavigated`、`DOM.documentUpdated` 和 DCL；Network projection 与真实
Document owner 互相矛盾。

修复没有在 Fetch command handler 追加 204/205 特判，而是删除这份重复 construction。buffered
body source 现在先构造 typed `ResponseHead`，再委托既有
`build_navigation_from_captured_raw_response_with_load_inputs_async`，由一个公共边界依次分类
no-commit、download 和 committable response。这样 classification 发生在 renderer Page reservation
之前；已有 200 response-stage prepared candidate 在 synthetic 204 terminal 被丢弃，原始 204 则
从未创建 candidate，而 synthetic 200 仍可在释放 pause 后创建并提交新 Document。最终路径是：

```text
PausedDocumentTransfer
  -> fulfill synthetic head/body OR continue original/overridden head + original body
  -> captured/streaming effective-response classifier
     -> 204/205: NavigationLoadOutcome::NoCommitResponse
     -> attachment: NavigationLoadOutcome::Download
     -> otherwise: ResponseCommitReady
  -> one shared materialized navigation terminal
```

新增集成矩阵如下；四格都从已经安装的 stable Page/Document 开始，并使用真实 response-stage
`Fetch.requestPaused`：

| 原始状态 | terminal action | effective 状态 | 预期结果 |
| --- | --- | --- | --- |
| 200 | `Fetch.fulfillRequest` | 204 | `ERR_ABORTED`，保留旧 Document/realm |
| 200 | `Fetch.continueResponse` 完整 override | 205 | `ERR_ABORTED`，保留旧 Document/realm |
| 204 | `Fetch.continueResponse` 无 override | 204 | `ERR_ABORTED`，保留旧 Document/realm |
| 204 | `Fetch.fulfillRequest` | 200 | 恰好一次新 Document commit |

前三格共同断言 effective `responseReceived` 先于同 request id 的
`loadingFailed(canceled=true)`，不存在 `loadingFinished`、`frameNavigated`、DOM update、DCL/load；
target Page residence、renderer Page residence、attachment、renderer agent、HTML 和 pending
navigation 状态全部保持。第四格反向断言 `responseReceived(200)`、`loadingFinished`、唯一
`frameNavigated`、DOM update、DCL/load 和 synthetic body，同时 stable Page/attachment 不变而
Document agent 必须变化。它还锁住原始 204 pause 不预建 renderer agent，避免未来为了优化
response-stage 又按原始 head 提前终止。第三纵切的真实 popup 204/205→fragment/pushState→redirect
用例继续负责 popup initial realm/history；本纵切在公共 Fetch/navigation 边界补入口矩阵，两者
组合覆盖 popup 与普通 stable Page，而不复制一份更大的 popup scenario。

聚焦验证：

```bash
cargo nextest run -p moli-protocol \
  response_stage_effective_no_content_statuses_abort_without_committing
# 1 passed

cargo nextest run -p moli-protocol \
  response_stage_fulfill_can_replace_original_no_content_with_committable_response
# 1 passed

cargo nextest run -p moli-protocol \
  -E 'test(/(continue_response_can_override_status_and_headers|fulfill_request_completes_navigation_with_synthetic_response|popup_no_commit_responses_preserve_initial_document_before_redirect_replacement)/)'
# 4 passed（同时命中一条同名 subresource override 回归）

cargo nextest run --no-fail-fast
# 15886 passed，18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# both passed
```

本纵切补齐的是 204/205 effective-response interception terminal，不改变普通
DNS/connect/TLS/HTTP failure 的 Document/error-page policy，也不扩大到 named target、`noopener`、
`javascript:` 或 close transaction；close transaction 后来已由 Phase 5B accepted-close 纵切完成，
其余仍是 Phase 4/5 后续切片。

#### Phase 4 第六纵切 F：pre-response transport failure 的 browser-owned error Document

第三纵切 C 和第五纵切 E 已经把“收到一个不可提交的 HTTP response”定义为保留旧 Document 的
no-commit terminal，但普通 DNS/connect/reset 等 transport failure 是另一类行为：请求尚未得到
可渲染 response，Chromium 会提交一个新的 browser-owned error Document。旧 Moli 路径把
`NetworkFetchFailure` 直接 materialize 为 failed navigation，并使 target 的旧 Document 不再可用；
popup initial `about:blank` 因而既没有 Chromium 的错误页，也不能继续通过原 stable Page/WindowProxy
观察新 realm。这个差异同时影响 `Page.navigate`、Target/history、Network terminal、Runtime realm
和 popup opener，不能只在错误返回字符串处补一个 HTML。

本纵切先明确一个窄而可验证的范围：普通 top-level main-document fetch 在 response metadata 之前
返回 `NetworkFetchFailure`。这里包括本轮连接接受后立即断开的最小复现，以及沿用同一 typed failure
的常见 DNS/connect 类错误。它不把证书 interstitial、HTTP 4xx/5xx、proxy CONNECT、显式 offline /
blocked URL、request-stage `Fetch.failRequest`、continued interception transport failure 或 policy
block 混成同一种产品语义；这些入口必须各自先确定 Chromium terminal，再决定是否复用 error
Document primitive。

##### Chromium 源码与二进制证据

对照基线为本地 `/home/donoughliu/chromium/src` commit
`a03603fe9af6230a12f1b2fb2c18a7d003a0d937`，`out/Default/chrome --version` 为
`Chromium 147.0.7709.0`。运行时 probe 使用：

```bash
/home/donoughliu/chromium/src/out/Default/chrome \
  --headless=new --disable-gpu --no-sandbox \
  --remote-debugging-port=9229 \
  --user-data-dir=/tmp/moli-chromium-network-error-probe \
  --noerrdialogs --no-first-run --ozone-platform=headless \
  --ozone-override-screen-size=800,600 --use-angle=swiftshader-webgl \
  about:blank
```

CDP probe 分别执行普通 `Page.navigate` 和 `window.open()`，目标是一个接受 TCP 后不返回 HTTP head
便关闭连接的本地 server。它记录 Target/Page/Network/Runtime 事件，并在 popup opener 中保留同步
返回的 WindowProxy 引用。观察结果如下：

| 可观察面 | Chromium 147 结果 |
| --- | --- |
| browsing context | frame id、target id 和 popup opener relation 保持；不是销毁 target 后另建错误页 target |
| Document URL | `location.href` / `Page.frameNavigated.frame.url` 为 `chrome-error://chromewebdata/` |
| 请求 URL | `Page.frameNavigated.frame.unreachableUrl`、Target URL 和当前 history entry 仍是失败 URL |
| Page.navigate | success-shaped callback，包含原 loader/frame identity、`errorText` 和 `isDownload: false` |
| Network | 同一 request id 上 `requestWillBeSent → loadingFailed → loadingFinished`，没有 `responseReceived` |
| realm/lifecycle | 旧 global/Document 状态消失，新 execution context 可执行；随后才有 DCL 和 load |
| popup initial history | error Document 替换 initial entry，`history.length == 1` |
| opener | popup 新 realm 中 `window.opener !== null`；opener 保存的 WindowProxy identity 不变 |

`loadingFailed` 后同 request id 又出现 `loadingFinished(encodedDataLength=0)` 看起来反直觉，但这是本地
Chromium 二进制的实际协议序列；实现和测试保留该事实，不用“一个请求只能有一个 terminal”的内部
直觉改写 CDP。probe 中 frame commit 位于二者之间，DCL/load 位于 `loadingFinished` 之后。

源码责任链与运行时结果一致：

- `content/browser/renderer_host/navigation_request.cc::CommitErrorPage` 仍走一次 cross-document
  commit，并通过 `ShouldReplaceCurrentEntryForFailedNavigation()` 决定 history replacement；initial
  entry 必须被替换；
- `content/renderer/render_frame_impl.cc::FailedNavigation` 把 Document URL 设置为
  `content::kUnreachableWebDataURL`，再把失败 URL 写入 `WebNavigationParams::unreachable_url`；同文件
  明确说明 HistoryItem 使用 unreachable URL，而不是内部错误页 URL；
- `content/public/common/url_constants.h` 将该内部 URL 定义为
  `chrome-error://chromewebdata/`；它不是一个可当作普通 WebUI 导航的 `chrome://` 页面；
- `third_party/blink/public/web/web_navigation_params.h` 和
  `third_party/blink/renderer/core/loader/document_loader.*` 把 unreachable URL 保存在 DocumentLoader
  上，供 frame/DevTools projection 使用；
- `content/browser/devtools/protocol/page_handler.cc::DispatchNavigateCallback` 从同一个
  `NavigationRequest::GetNetErrorCode()` 生成 `errorText`，所以不能先返回普通 success，再只在
  Network domain 标记失败；
- `third_party/blink/web_tests/inspector-protocol/page/frameNavigatedToUnreachableUrl.js` 直接锁住
  `frameNavigated.frame.unreachableUrl`，browser tests 还检查 error Document 的内部 URL和 opaque
  origin。

##### Moli owner transaction

实现复用已经成熟的 stable Page replacement/realm 基础，不创建第二个 Page，也不恢复 lightweight
popup loader。普通 pre-response failure 现在走：

```text
NetworkFetchFailure(original request / request id / net error)
  -> browser-owned NetworkErrorPageNavigation
     { error_text, unreachable_url }
  -> synthetic internal ResponseHead
     { final_url = chrome-error://chromewebdata/, status = 200, text/html }
  -> existing prepared replacement reservation
  -> DocumentCommit on the exact stable Page
     -> detach old default realm, keep Page-owned WindowProxy
     -> install error Document realm with opaque/insecure security state
     -> Page.frameNavigated(url = internal, unreachableUrl = requested)
  -> exact renderer DCL/load continuation
```

这里必须区分三种 URL，不能再用一个 `final_url` 同时满足所有观察面：

| owner / projection | 本纵切保存的 URL | 原因 |
| --- | --- | --- |
| renderer `Page` / Document / frame tree | `chrome-error://chromewebdata/` | 当前可执行 realm 确实是 error Document |
| `unreachableUrl` | transport failure 的 current request URL | DevTools 需要知道哪个资源不可达 |
| Target identity / browser history | transport failure URL | 地址栏、Target、history 代表用户请求，而不是内部实现 URL |

`RendererMainDocumentCommit` 因而新增可选 `unreachable_url`，frame commit 和 `Page.getFrameTree` /
resource tree 都从 Document identity 投影内部 URL和 unreachable URL；Target/history commit API 则显式
接收另一份 browser-visible URL。history snapshot 不再从 `Page.final_url()` 反推地址栏 URL，避免
后续 title refresh 把 error entry 偷换成内部 URL。error Document 使用 opaque origin，CDP frame /
Target security origin 投影为 `://`，secure-context type 为 `InsecureScheme`。

内部错误 HTML 只提供轻量、可脚本化、可诊断的 Document：title 使用失败 host，正文展示转义后的
URL和 net error。它通过与普通 response 相同的 parser、realm、DCL/load 和 PageState owner 路径
构建，但不是原网络请求的 response：Network domain 不发布 `responseReceived`，也不把 synthetic
body 存进 main-resource response-body cache。这样 `Runtime.evaluate`、DOM snapshot 和 lifecycle
都能观察真实新 Document，同时 `Network.getResponseBody` 不会伪装服务器返回了一份 200 HTML。

Network progress 使用一个专门的 two-boundary gate，而不是在 commit 调用点手排 JSON：

```text
response-visible boundary -> loadingFailed(errorText, canceled=false)
renderer output boundary  -> frame commit / contexts cleared + created
body-finished boundary     -> loadingFinished(encodedDataLength=0)
DCL/load continuation      -> DOMContentLoaded / load / stoppedLoading
```

`Page.navigate` result 在 failure progress 可见后返回 `{frameId, loaderId, errorText,
isDownload:false}`。无显式 navigate command 的 popup initial destination 仍消费相同 activity，只是不
制造 command response。active、background 和 stable replacement 共用这一 transaction；error
Document 的 main resource 不进入普通 response store，旧 loader/generation 的 completion 也仍受
existing Document token/currentness gate 限制。

##### Stable WindowProxy、realm 与 opener

popup 回归第一次运行暴露了一个比 Network 更底层的真实缺口：related auxiliary Page 已经复用
stable main WindowProxy，但 `window.opener` 只存在于旧 realm 的 `WINDOW_OPENER_SLOT`；
`detach_global()` 后新 error realm 得到 `null`。修复没有从 protocol `TargetInfo.openerId` 反向制造
JS object。`RendererPageScriptEnvironment` 现在在旧 main realm commit 前捕获该槽中的实际 V8 value，
在同一 stable WindowProxy 绑定新 Context、完成 bootstrap 后再恢复。于是：

- target/core Page residence 和 renderer Page/WindowProxy residence 跨失败保持；
- renderer attachment、execution context、global lexical state 和 Document generation 必须变化；
- popup 新 error realm 仍有 `window.opener`，旧 realm/body marker 不会泄漏；
- opener 同步保存的两个 popup WindowProxy 引用在失败前后仍严格相等；
- 保存的是实际槽值而不是一条 target id，未来 opener 被显式 sever 为 `null` 时也不会被导航重新连上。

同一回归也精确暴露了下一层边界：当时 opener 保存的 stable popup WindowProxy 在跨源 commit 后仍
保持 identity，但 `.closed` 会得到 `SecurityError`，说明 top-level related Page 尚未接入 child-frame
已有的 restricted cross-origin access surface。Phase 5 第一纵切 A 已在下节按完整 allowlist 处理该
边界，而不是只为 `.closed` 开洞；原 identity 回归也已扩展为 Window/Location descriptor、ownKeys、
`postMessage` source 和 target-owned location navigation 的端到端矩阵。动态 close state 仍属于下一
纵切；该记录描述 Phase 5A 当时边界，Phase 5B 现已用 shared Page lifecycle authority 替换常量值。

##### 回归矩阵与当前边界

新增/扩展回归覆盖：

- Page domain：普通 navigate failure 提交内部 error frame、`unreachableUrl` 和 requested-URL
  history；
- Network domain：`requestWillBeSent < loadingFailed < frame commit < loadingFinished < DCL/load`，
  同 request id、无 `responseReceived`，并锁住 response/body 两个 progress boundary；
- Runtime：error Document 可执行，旧 global 不存在，stable target/core/renderer Page identity 保持而
  renderer attachment 更新；
- popup end-to-end：真实 connection drop、initial `about:blank` replacement、history length 1、
  Target opener metadata、new-realm opener 和 opener-side stable WindowProxy identity；
- Page frame/resource tree：Document URL 与 Target/history URL 不再互相覆盖。

本轮聚焦验证与全量门禁：

```bash
cargo nextest run -p moli-protocol navigate_failure --no-fail-fast
# 2 passed

cargo nextest run -p moli-protocol main_document_navigation_failure --no-fail-fast
# 2 passed

cargo nextest run -p moli-protocol \
  page_navigate_network_failure_commits_error_document_in_stable_page --no-fail-fast
# 1 passed

cargo nextest run -p moli-protocol \
  error_page_progress_releases_failed_before_finished_at_separate_boundaries --no-fail-fast
# 1 passed

cargo nextest run -p moli-protocol \
  popup_transport_failure_commits_error_document_in_stable_auxiliary_page --no-fail-fast
# 1 passed

cargo nextest run --no-fail-fast
# 15888 passed，18 skipped

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# both passed
```

以上矩阵仍不足以宣称“所有网络错误完成”。下一批必须至少分别覆盖 redirect-then-drop、DNS、TLS
证书/interstitial、proxy、offline/blocked policy、Fetch request-stage fail/continue、reload/traverse
到 error entry，以及 error page 上的再次导航。尤其 redirect failure 的 Network redirect metadata、
error HistoryItem method/state 和 `Network.getResponseBody` 错误形状目前证据较弱。named target、
`noopener` / COOP、cross-origin WindowProxy 的每调用方细节仍按 Phase 5 后续纵切处理；动态 close
transaction 已由 Phase 5B accepted-close 闭环继续收敛。

### Phase 5：name、opener、cross-origin、sandbox 与 COOP

#### 第一纵切 A：复用 stable WindowProxy 的 related top-level 跨源 surface

本纵切已完成。范围刻意限定为：两个真实 top-level Page 已通过 production auxiliary admission
进入同一个 related-page script agent，opener 持有 popup 的实际 stable main WindowProxy，随后 popup
提交不同源 Document。它不建立 named-target registry，不改变 `noopener` / COOP 的 fresh-agent
policy，也不宣称 close transaction 已完成。

##### Chromium / WPT 合同

直接对照的主要证据是：

- Chromium `third_party/blink/renderer/bindings/core/v8/window_proxy.h`：同一个 browsing context
  保留稳定 WindowProxy，访问安全检查位于 proxy/realm 边界；
- WPT `html/browsers/origin/cross-origin-objects/cross-origin-objects.html`：锁住 Window / Location
  allowlist、own descriptor、ownKeys、well-known symbols、prototype、内部方法和每 incumbent wrapper；
- 同目录 `cross-origin-objects-function-{caching,length,name}.html`：锁住函数 identity、`name`、
  `length` 和 descriptor 返回的函数缓存；
- 同目录 `window-location-and-location-href-cross-realm-set.html`：锁住 Location setter 的 receiver、
  URL coercion 和异常 realm；
- popup error-Document 行为继续参考 Chromium `NavigationRequest::CommitErrorPage` 路径；该路径证明
  transport failure 替换 Document，而不是销毁 auxiliary browsing context 或 opener edge。

对不含子 frame 的跨源 Window，WPT 的 string allowlist 是：

| 类别 | 名称 | 本纵切状态 |
| --- | --- | --- |
| identity / relation | `window`、`self`、`frames`、`parent`、`top`、`opener` | 已接真实 stable WindowProxy；top-level 的 parent/top/self 均是自身，opener 来自实际 V8 opener slot |
| live scalar | `closed`、`length` | 可跨源读取；`length` 从目标 Page child count 读取，`closed` 由稳定 Page environment 的 `Active → Closing → Closed` authority 动态投影 |
| callable | `postMessage`、`blur`、`close`、`focus` | descriptor/name/length/缓存形状已对齐；`postMessage` 已跨 Page 交付，`close()` 已进入 target-owned close transaction，`blur` / `focus` 仍无动态事务 |
| navigation | `location` setter、`location.href` setter、`location.replace()` | 已进入目标 Page 原有 Location/navigation owner；读 `href` 与其他敏感属性抛 `SecurityError` |
| promise assimilation | `then` | own、值为 `undefined`；没有名为 `then` 的 child 时不会把 WindowProxy 当 thenable |
| well-known symbols | `Symbol.toStringTag`、`Symbol.hasInstance`、`Symbol.isConcatSpreadable` | own、non-enumerable、non-writable、configurable，值均为 `undefined` |

`globalThis` 不在 HTML cross-origin Window allowlist 中。本纵切从共用 child primitive 中移除了此前
错误暴露的 cross-origin `globalThis` identity alias；读取它与 `document`、`name`、任意未知属性
一样抛调用方 realm 的 `SecurityError`。well-known symbol 不再伪造 `"Window"` / `"Location"`
tag，因此 `Object.prototype.toString.call(crossOriginWindow)` 和 Location 都是 WPT 要求的
`[object Object]`，不是 `[object Window]` / `[object Location]`。

当前已锁住的 descriptor 规则是：

- allowlist string/symbol property 都作为 own property 投影；
- 普通值和方法 `writable:false`、`enumerable:false`、`configurable:true`；
- `location` 是带 getter/setter 的 non-enumerable、configurable own accessor；
- 数字 child index 应为 `writable:false`、`enumerable:true`、`configurable:true`；
- 未知 Window property 的 read、descriptor 和 `hasOwnProperty` 都抛 `SecurityError`；
- 无 child 的 `Object.getOwnPropertyNames(window)` 精确只含 14 个 string allowlist 名称，
  `Object.keys(window)` 为空，三个 symbols 按 WPT 顺序位于 symbol own keys 中；
- cross-origin Window / Location prototype 为 `null`。

##### Owner 设计：实际 proxy + target-owned surface

实现没有给 popup 再建一个“restricted facade”。现有 Window global template 本来就安装了 child-frame
使用的 V8 security-token access check 和 named/indexed property handlers；本纵切把其授权域从“同一
`JsContextHost` 的 top/child realm”窄化扩展为“显式 related、共享同一 script-agent isolate 的两个
current top-level default realms”：

```text
opener current Context
  -> saved popup stable WindowProxy (target Page owns the object)
     -> V8 security-token/access-check
        -> same effective tuple origin: access actual target global
        -> cross origin: target JsContextHost cross-origin access surface
           -> identity / descriptor / ownKeys
           -> target postMessage queue
           -> target Location navigation owner
        -> unrelated Page / stale realm / non-top endpoint: deny
```

关键责任边界如下：

1. `RendererPageScriptEnvironment::is_related_page_peer` 只接受不同 Page id、相同 isolate identity。
   一个 document isolate 对应一个 script agent；V8 access-check 发生时 holder 已被可变借用，因此热路径
   不能再次借 holder 查询 script-agent id。第一次回归确实捕获了该 reentrant `RefCell` borrow，当前
   实现使用稳定 `Rc` identity，不引入裸指针 cache 或临时释放借用。
2. `window_access_check_callback` 只有在两个 context 的 host 不同且满足 related-page gate 时才进入
   cross-Page origin 判断；fresh Page、`noopener` Page、stale owner 和非 top-level endpoint 不因此
   获得能力。
3. 每个 main default realm bootstrap 在目标 `JsContextHost` 中建立 cross-origin access surface，但
   surface 的 identity slots 指回 Page environment 持有的实际 stable main WindowProxy。navigation
   替换 Context 后重新建立 target-owned surface，调用方保存的 proxy object 本身不变。
4. surface 在恢复 navigation-persistent opener slot 之后读取实际 opener value。它没有根据 CDP
   `openerId` 反造 JS 对象，也没有把 opener target metadata 当作 JS graph authority。
5. unknown descriptor 路径由 handler 显式在 lexical/incumbent realm 创建 DOM `SecurityError`，避免
   V8 对 `undefined` descriptor 生成错误 realm 的普通 `TypeError`。

这正是“复用 child-frame stable WindowProxy/realm 基础”的含义：共享 access-check、origin、
handler、descriptor 和 Location primitive；popup 仍是独立 top-level Page/target，没有
`frameElement`、parent load blocker 或 iframe owner 特例。

##### child Document preload registry 必须按 owner 冻结

workspace 并发门禁同时暴露了 child-frame 基础中的一个既有时序缺口。一个 child Document 已经创建，
但它的 realm-materialization Page task 可能因调度负载晚于
`Page.addScriptToEvaluateOnNewDocument(runImmediately:true)` 执行；旧实现到 materialization body 才读
Page-wide 最新 script registry，于是本应只在当前 top-level world 立即执行的新脚本，又被追溯重放到
更早创建、同名的 child isolated world。聚焦运行通常先完成 child task，因此看不见；workspace 高负载
能稳定把它放大为 `typeof childMarker === "string"`。

这里没有给 CDP 命令加 drain、retry 或 sleep。责任边界提升到 exact child Document owner：

- initial-empty Document、普通导航 commit 和 `document.open()` replacement 创建新 owner 时，冻结当时
  可见的完整 document-start script registry；
- 后续 default/named child realm materialization 只消费该 owner 的快照，不再读取 Page-wide 最新脚本；
- 快照与 Document owner 同寿命，同一 Document 的测试性/内部 realm replacement 继续使用同一份配置，
  只在 owner retirement 时清理；
- later registry update 仍由 top-level `runImmediately` 处理，并被之后创建的 child Document 捕获，不会
  因修复而丢失 future-document preload。

低层回归用同一 Page 锁住“更新前 child 不追溯注入、更新后 child 正常继承”；原 protocol
world-name 回归在并发 core+renderer 负载下从修复前第 6/22/36 次内可复现，变为修复后连续
100 次通过。这项修复是复用 child-frame realm primitive 的必要收敛，不是 popup 调用点补丁。

##### opaque origin：不能比较 host-local LocalWindow id

真实 error popup 回归暴露了一个 shared-isolate 特有问题。`WindowAccessOrigin::Opaque` 过去用
`WindowExecutionContextOwner::Frame(LocalWindowId)` 作为 non-serialized identity；该 id 只在单个
`JsContextHost` 内唯一。两个 Page 都可能分配 `LocalWindowId(1)`，跨 host 直接比较会把两个独立
opaque origin 错判为同源，并绕过 restricted surface。

第一阶段防御曾让 related-page 跨 host gate 对 opaque origin 一律 fail closed，并依赖 initial Context 的 V8
security token 保住常用继承路径。P6R2 已替换这项临时状态。对照固定 Chromium
`security_origin.cc:173-210,249-317,566-611,708-712`：Blink 的 opaque `SecurityOrigin` 保存
`url::Origin::Nonce`，copy/IPC reconstruction 复制 nonce，`CreateUniqueOpaque()` / `DeriveNewOpaqueOrigin()`
创建新 nonce，`IsSameOriginWith()` 和 `IsSameOriginDomainWith()` 在任一侧 opaque 时只比较 nonce。Moli 因此复用
browser-context 单调分配的 `OpaqueOriginNonce`，不再用 V8 token 或 host-local owner id 充当 Rust authority。

nonce 的生命周期绑定 LocalWindow，而不是机械绑定 Document。普通 child navigation 安装 replacement
LocalWindow 时换 nonce；`document.open()` 只换 Document owner、保留 LocalWindow，因此必须保留 nonce。top-level
Page 构造时从 inherited StorageKey 复用 nonce，独立 opaque Page 则由 browser-context runtime 分配新值。这样
`data:` opener → related initial `about:blank` 在完整 Page adoption 前后都能通过同源 DOM access，而另一个同样
序列化为 `null` 的 related Page 仍抛 `SecurityError`。

remote top-level state 与 remote frame snapshot 同步复制 nonce；frame wire 升到 v2，并严格拒绝 opaque-without-nonce、
tuple-with-nonce、zero nonce 和 opaque `document.domain`。remote `CanNavigate` 的 target/ancestor origin comparison
消费该 identity，不再从公开字符串 `null` 反造无 identity origin。nonce 仍不会暴露给 JavaScript，
`origin`/`MessageEvent.origin` 继续返回 `null`。剩余 WPT 长尾是 sandbox/COOP/remote navigation 的外部矩阵，
不再是 owner 模型缺口。

##### cross-Page `postMessage`

跨源 `postMessage` 继续调用目标 Window surface 上的同一 native binding，但 acceptance 时需要同时
知道 target 和 incumbent source：

- binding 的 current context/host 是目标 popup Page，因此 payload 进入 popup 的 Page-owned
  WindowMessage task queue，target origin 在 dispatch 时再次检查；
- incumbent context 是 opener Page；只有它与 target 属于同一 related script agent、且是 current
  top-level default realm 时才接受；
- acceptance 保存 source origin、source owner/realm token，以及 source Page 的实际 stable main
  WindowProxy `v8::Global`；
- `MessageEvent` materialization 直接使用该 source proxy，所以 target 中
  `event.source === opener`，而不是 target global、`null` 或一份 synthetic facade；
- structured clone、transfer list、`messageerror`、target-generation/currentness 和 task ordering 继续
  复用原 WindowMessage owner path。

端到端测试在 error popup realm 注册 listener，由 opener 对保存的跨源 WindowProxy 发送 object，
锁住 `{data, origin:"null", sourceIsOpener:true}`。Phase 5B 又补上 source/target Page 最终关闭后的
断开边界：保留的 stable proxy 继续可读 `closed === true`，但旧 realm function / DOM wrapper 会在
解引用 native host 前 fail closed，不能因为 source `v8::Global` 仍存在就延长 Rust host 生命周期。

##### target-owned Location navigation

cross-origin Location object 不保存 protocol target id，也不从 opener host 发起 mirrored navigation。
它只保存目标 stable WindowProxy marker；`window.location = value`、`location.href = value` 和
`location.replace(value)` 完成 WebIDL USVString coercion 后，取目标 Window 的真实 public Location
slot，进入已有 `navigate_location_object` owner：

```text
opener expression
  -> popup restricted Location setter / replace binding
  -> popup current Window Location slot
  -> popup RendererPageScriptEnvironment task/output route
  -> exact popup TargetPageResidenceIdentity navigation
  -> replace popup Document/realm; keep Page + stable WindowProxy
```

CDP 回归从 opener 先执行 assignment、再执行 `replace()`，两次都等待 background popup target 的
typed scheduler state，而不是 sleep/drain；新 Document 的 title/body 只能在 popup session 观察，
target/core Page residence 和 renderer Page/WindowProxy residence 均保持，opener 保存的两个 proxy
引用在两次 navigation 后仍严格相等。

#### 第二纵切 B：统一 close transaction 与最终 WindowProxy 断开

本纵切已完成 close transaction 的第一条 production 闭环，范围是已经迁移为真实 related
auxiliary Page 的 top-level popup，以及既有 `Page.close` / `Target.closeTarget` target teardown。
它没有把 lightweight named/`noopener` popup 伪装成已经迁移，也没有在这一层实现 popup blocker、
sandbox、COOP 或完整 unload policy。

##### Chromium 合同与 Moli 对应边界

本地 Chromium `a03603fe9af6` 的关键事实如下：

- `DOMWindow::Close` 只接受 outermost main frame，并以 incumbent Document 的 `CanNavigate`、
  `OpenedByDOM` / history length、`ShouldClose` 等条件决定脚本是否可关；调用 `Page::CloseSoon()` 后又
  立即设置 `window_is_closing_`，保证延迟关闭真正发生前 `window.closed` 已经返回 `true`；
- `DOMWindow::closed()` 同时观察 `window_is_closing_`、Frame 是否存在和 Frame 是否仍有 Page；它不是
  bootstrap 时写死的普通 data property；
- `Page::CloseSoon()` 先把 Page 标记为 closing、停止 loader，再把 browser close request 排到当前
  JavaScript 完成之后；这样深层 JS 调用不会在嵌套 loop 中把正在执行的 realm 提前销毁；
- browser 侧 `RenderFrameHostImpl::ClosePage` 把 renderer-origin 和 browser-origin close 汇到同一
  unload / final close path，并在 renderer-origin request 已不再指向 active main frame 时拒绝误关新页；
  `WebContentsImpl::ClosePage` 也进入该入口。

Moli 对应采用两阶段状态，而不是从 `close()` callback 直接 drop `PageVm`：

```text
target Window.close()
  -> RendererPageScriptEnvironment: Active -> Closing（同步、幂等）
  -> target Page output FIFO: RendererOwnerAction::TopLevelClose
  -> protocol exact TargetPageResidenceIdentity preflight
  -> fail pending Inspector awaits / fetches + acquire renderer output fence
  -> PageTargetTerminationOwnerAction::WindowClose
  -> common target/session close path + targetDestroyed
  -> renderer final Page teardown: Closing -> Closed
  -> same stable WindowProxy reattached to a host-free restricted facade
```

这里有五个必须保持的 owner 不变量：

1. `RendererPageScriptEnvironment` 与 stable Page/WindowProxy 同寿命，并跨 replacement `PageVm` 复用；
   `Active → Closing` 只能成功一次，所以同一 turn 的重复 `close()` 只产生一个 owner action。普通
   cross-document navigation 不改变该状态。
2. same-origin direct Window 和 related cross-origin Window surface 都调用目标 Page 的 close authority。
   跨 Page 调用发生在 opener turn 时，typed `TopLevelCloseOutputHandoff` 只唤醒目标 Page owner 来冻结
   它自己的 FIFO，不携带第二份 close authority；busy Page 在本 turn 返回时结算，尚未 admission 的
   initial Page 在创建提交时结算。
3. protocol 在 renderer output 进入 ingress 时冻结 target id、session owner scope 和
   `TargetPageResidenceIdentity`。延迟 action 不能跟随一个 session 去关闭后来安装的 Page；pending
   Inspector await、navigation/subresource fetch 先产生 terminal output，renderer fence 通过后才发布
   最终 target termination。
4. initial Page creation diagnostics 会携带 `top_level_browsing_context_closing`。因此
   `const p = open(url); p.close()` 即使完全发生在 target admission 前，也仍先创建可观察的 target，
   随后按自己的 close FIFO 销毁；目标 URL 的 navigation claim 根本不会 stage/publish，不靠取消一个
   已经开始的请求来掩盖副作用。
5. 每个 `JsContextHost` 拥有一个 Document 级 liveness token，并把同一个 token 装进 default、isolated、
   child 和临时 facade Context 的非 owning host slot。child realm navigation 只退休自己的 owner/token，
   不提前熄灭 Document host token，因此保留的旧 child `fetch` / XHR / Runtime binding 仍按原 owner 语义
   fail closed；当整个 Document host retirement 时 token 一次性失效，即使某个旧 child Context 已从 live
   realm store 移除，任何 raw-pointer callback 也会先拒绝访问。当前仍可枚举的 Context 还会被显式标成
   disconnected、移除 host slot 并清空 bridge pointer。这个 Document 边界不改变 stable Page 的 close
   state；只有 stable Page 最终 discard 才进一步把原 main WindowProxy 从旧 global detach，挂到无
   `JsContextHost` 的 restricted facade。opener 保存的引用仍严格相等并观察
   `{closed:true, opener:<original opener>, length:0}`；敏感属性抛 `SecurityError`，旧 realm function 和 DOM wrapper
   抛 `TypeError`。Document replacement/cancel 回归明确锁住 `closed === false`，防止把 realm teardown
   提升成 browsing-context teardown。

`window.close()`、`Page.close` 和 `Target.closeTarget` 的触发前置条件不同，但最终 target/session
closure 和 renderer Page discard 已经共用同一责任边界。`WindowClose` 保留独立 termination kind，
用于诊断触发来源，而不是复制销毁逻辑。`targetDestroyed` 仍由既有 target-host closure event plan
统一生成，重复 close 或晚到 action 通过 exact target/Page currentness 变成 no-op。

##### 本纵切仍未实现的 Chromium close policy

本轮刻意没有把以下行为塞进 V8 callback 或 protocol 调用点：

- `OpenedByDOM`、history length 1 和浏览器设置共同决定的 script-closable gate；
- incumbent `CanNavigate`、sandbox navigation flag、COOP browsing-context group sever；
- `beforeunload` / `unload`、dialog、`ShouldClose` 和 renderer ACK/timeout；
- close 与已经提交的 navigation、named-target reuse、opener sever 的完整竞态矩阵。

这些是 creation/group policy 与通用 Page unload lifecycle 的后续纵切。当前实现对已迁移的 top-level
Page 允许脚本发起 close；证据只支持“请求一旦被接受，transaction、取消、target 事件与断开语义
一致”，不能解读成 Chromium 的所有“是否允许关闭”条件已经完成。

> 2026-08-06 状态更新：本小节记录的是 Phase 5B 当时的明确非目标。Phase 5L1 已补齐本地真实 Page 的
> `OpenedByDOM` / history / browser-setting script-closable gate、subtree `beforeunload`、dialog、
> `pagehide`/`unload`、renderer ACK/timeout 与 close/navigation currentness。COOP/RemoteFrame、focus 以及
> detached-realm lifetime 仍按后续大阶段处理；不能把 L1 外推为完整 group/remote close。

##### 聚焦证据

renderer 回归覆盖：跨 Page 两次 `close()` 只发布一个 target-owned action，target 自身与 opener
同步观察 `closed === true`；最终 close 后 stable proxy identity 不变，旧 function/DOM wrapper fail
closed；普通 navigation 和取消的 prepared replacement 保持 `closed === false`。另有组合回归先让
child realm 因 navigation 离开 live store，再退休整个 Document host：host 退休前旧 child
`fetch` 仍返回既有 shutting-down TypeError，退休后同一 closure 在原 Promise/TypeError realm 安全拒绝，
证明安全性不依赖保存所有旧 `Global<Context>`。

protocol 回归覆盖：`open(url); popup.close()` 的 evaluation response 和 `targetCreated` 都早于唯一
`targetDestroyed`，目标监听 socket 没有收到连接，target/session residence 被移除；随后 opener
仍观察同一个 closed proxy。另一个用例从 `Target.closeTarget` 关闭真实 popup，得到完全相同的最终
proxy facade，证明 browser-origin close 没有旁路 renderer teardown。

本地 Chromium 行为探针还校正了两个边界。关闭 DOM-opened popup 后，保存的 proxy 仍满足
`popup.opener === opener`，所以 closed facade 保留原 opener edge，而不是擅自 sever 为 `null`。另一方面，
Chromium 的 Oilpan/V8 lifetime 能让已关闭 popup 或已移除 iframe 的旧 Node 和函数继续读取 detached
Document；Moli 当前 DocumentRuntime/`JsContextHost` 还不是由 V8 wrapper 共同拥有，因而本纵切
只能在 host retirement 时让这些 raw-host-backed 值抛 `TypeError` 来保证内存安全。这是明确的兼容性
缺口，不应把“安全 fail closed”解读成 Chromium 的 detached-Document 完整语义。

`chromium 145.0.7632.116 --headless --dump-dom` 的本地最小探针结果为：移除 same-origin iframe 后，
`[savedNode.textContent, savedFunction(), savedWindow.closed]` 是 `['old','old',true]`；关闭 DOM-opened
popup 后，下一 task 中的
`[savedNode.textContent, savedFunction(), popup.closed, popup.opener === opener]` 是
`['old','old',true,true]`。这与上述 source lifecycle 分支一致，也把当前 Moli 的安全降级边界
固定成可复查的行为差异。

#### Phase 5C：live child relation 与 Page-scoped opener edge

Phase 5A 最初为 related top-level WindowProxy 建立的 restricted surface 仍保存了 bootstrap 时的
child count/name 和 opener value。`length` 虽然会读取目标 host，但 numeric index、named child、
ownKeys 和 opener getter 仍可能互相矛盾。Phase 5C 没有在 `appendChild`、attribute setter、remove 或
navigation 调用点逐个刷新 surface；它把投影重新接回已经拥有 browsing-context identity 的 owner。

top-level cross-origin Window 的 access surface 现在只保存稳定 allowlist/function/Location 基础，不再
保存 child index/name。V8 named/indexed handler 每次从目标 WindowProxy 的 creation context 找到精确
`JsContextHost`，先通过既有 child registry 同步当前 subtree，再按 frame-tree sibling order 解析 child，
最终调用 `child_browsing_context_window_proxy_for_top()`。因此 index、name 和 iframe `contentWindow`
返回的是同一个 child-frame stable WindowProxy，而不是另造 detached placeholder：

```text
related top-level WindowProxy handler
  -> target Page current JsContextHost
  -> top-level child browsing-context registry
  -> stable child WindowProxy / current LocalWindow realm
  -> caller-observed numeric or named value
```

named lookup 的次序按本地 Chromium source/WPT 校正为：cross-origin exposed IDL property 优先，其次是
document-tree child browsing-context name，最后才是 `then` / symbol fallback 或 SecurityError。于是名为
`close` 的 child 不能遮住 allowlist method；名为 `open` 的 child 在跨源观察方可见；名为 `then` 的 child
会遮住默认 `undefined`，移除后又恢复非 thenable fallback。普通 named child 不参加
`[[OwnPropertyKeys]]`，numeric child 则继续作为 enumerable index 出现在 keys；这与 Chromium
`WindowProperties::AnonymousNamedGetter()`、bindings generator 的
`CrossOriginGetOwnPropertyHelper` 以及 `v8_cross_origin_property_support.cc` 的 fallback/enumerator 顺序
一致。

opener 不再只是 LocalWindow private slot。`RendererPageScriptEnvironment` 保存 Page-scoped
`top_level_opener_edge`，initial auxiliary realm 直接绑定实际 stable opener WindowProxy；Document
replacement 只把同一 edge 投影到新 realm。`Window.opener` setter 对齐 Chromium
`DOMWindow::setOpenerForBindings()`：传入 `null` 先 sever browsing-context edge；无论值是否为 null，
随后都在当前 Window 上建立 writable/enumerable/configurable own data property。非 null 值因此只
shadow accessor，不改变底层 edge。跨源 popup opener accessor 读取目标 Page edge；opener Page 最终
discard 后，stable opener proxy 的 closed marker 会把存活 popup 的 edge 折叠为 `null`。反方向关闭
popup 不会错误 sever 其仍存活的 opener，closed facade 继续投影原 edge。

回归覆盖以下状态变化：

- 跨源 target 动态插入名为 `alpha`、`then`、`open` 的三个 iframe；observer 的 index/name/descriptor
  都指向同一 stable child WindowProxy，keys 只含 `0..2`，普通 named child 不进入 own names；
- rename `alpha → renamed` 并移除 `then` 后，length/indices/name 同步变化，旧 `alpha` 重新抛
  `SecurityError`，`then` 恢复 `undefined`，没有 stale index/name；
- target 执行 `window.opener = null` 后，保存的原 accessor getter、target 自身和跨源 observer 都看到
  `null`，own data descriptor 形状正确，Document replacement 后仍不重连；另有 non-null 赋值回归
  锁住“只建立 ordinary shadow、原 getter/edge 不变”；
- 不显式 sever 时，关闭 popup 自身仍保留 opener；关闭 opener Page 时，存活 popup 在最终 discard 后
  看到 `null`，导航后保持 sever。

本纵切当时没有实现 COOP/`noopener` 的 browsing-context-group switch 或 remote endpoint；Page-scoped
edge 成为后续 group policy transaction 的接入点。local committed-response COOP switch 现已由 G1 接入该
边界，但真正 remote endpoint、redirect/report-only 和跨进程 group 仍不能从“JS setter/本地 opener discard
已 live”外推。

#### Phase 5D1：restricted Location internal methods

Phase 5A 的 cross-origin Location 用 null-prototype target 加 JavaScript `Proxy` 实现，但 target 同时
安装了 `origin`、`assign`、`reload` 等 denied accessor。这虽然让直接读取抛错，却产生两个错误事实：
denied name 会泄漏进 `[[OwnPropertyKeys]]`，其 descriptor/hasOwnProperty 还会被报告为 own property；
完全未知的 key 则沿普通 target 路径返回 `undefined`。handler 也没有拥有 `[[SetPrototypeOf]]` 与
`[[PreventExtensions]]`，因此这些 internal methods 会落回普通 extensible object 语义。

D1 把 restricted Location 的 target 缩成唯一可枚举事实：`href` setter、`replace` method、`then`
fallback 和 `Symbol.toStringTag` / `Symbol.hasInstance` / `Symbol.isConcatSpreadable`。Proxy handler 负责
其余策略，不再用 denied accessor 假装接口表：

```text
cross-origin Location Proxy
  -> minimal null-prototype target
       href / replace / then / three fallback symbols
  -> get / has / getOwnPropertyDescriptor
       allow target key; otherwise SecurityError in the accessing realm
  -> set
       href navigation only; otherwise SecurityError
  -> deleteProperty / defineProperty
       always SecurityError
  -> setPrototypeOf
       null => true; non-null => false
  -> preventExtensions
       false; target remains extensible
```

这与本地 Chromium 的
`third_party/blink/renderer/bindings/scripts/bind_gen/interface.py`（generated cross-origin
getter/descriptor/query/enumerator）和
`third_party/blink/renderer/platform/bindings/v8_cross_origin_property_support.cc`
（fallback/ownKeys）一致，也直接覆盖
`third_party/blink/web_tests/external/wpt/html/browsers/origin/cross-origin-objects/cross-origin-objects.html`
中的 Location 矩阵。回归锁住以下结果：

- `Object.getOwnPropertyNames(location)` 精确为 `['href','replace','then']`，string keys 不可枚举，
  3 个 symbol 位于其后且 descriptor 都是 non-writable/non-enumerable/configurable；
- `href` descriptor 只有本地 realm 可调用的 setter，`replace` 是 readonly method，fallback value 为
  `undefined`；existing identity、WebIDL conversion 和 navigation 路径不变；
- denied/unknown key 的 get、`in`、descriptor、hasOwnProperty、set、delete、define 均抛访问方 realm 的
  `SecurityError`，不再返回 `undefined` 或暴露伪 descriptor；
- prototype 保持 `null`：设为 `null` 成功，设为其他对象时 Reflect 返回 false、Object/legacy setter
  抛 `TypeError`；直接 `location.__proto__ = value` 仍属于 denied property set，抛 `SecurityError`；
- `Object.isExtensible()` 始终为 true，Reflect.preventExtensions 返回 false，Object.preventExtensions
  抛 `TypeError`，失败后不会偷偷冻结 target。

D1 没有声称完成整个 Phase 5D。Window 的 denied/unknown property、out-of-range index、delete/define、
prototype/preventExtensions 与 ownKeys 精确矩阵仍由 D2 负责；D1 当时的 cross-origin
Window/Location method 和 accessor 仍从 target surface 创建，后续已由 D3a 改成按 accessing
realm/incumbent 分配。

#### Phase 5D2：restricted Window internal methods

D2 延续 D1 的原则：restricted surface 只保存真实可观察事实，拒绝策略属于 WindowProxy internal
methods。旧 child surface 为 `document`、`setTimeout`、`open` 等大量 denied name 安装 throwing
accessor，导致这些 name 错误进入 own keys、descriptor 和 hasOwnProperty；不存在的 numeric index 又因
直接调用 V8 `get()` 而把“property missing”误判成值为 `undefined` 的 own property。这既不符合
`cross-origin-objects.html`，也阻止名为 `document` / `open` / `then` 的 child browsing context 按
Chromium 的 named getter precedence 出现。

D2 删除整张 denied accessor 表。stable WindowProxy 的 V8 named/indexed handler 与 detached fallback
Proxy 现在共同拥有以下矩阵：

```text
cross-origin WindowProxy
  -> minimal access surface
       live exposed properties / actual child indices / actual named children
       then fallback / three fallback symbols
  -> [[Get]] / [[HasProperty]] / [[GetOwnProperty]]
       exposed property or existing child => value/descriptor
       denied/unknown name or missing index => accessing-realm SecurityError
  -> [[Set]]
       location navigation only; every other name/index => SecurityError
  -> [[Delete]] / [[DefineOwnProperty]]
       every name/index, present or absent => SecurityError
  -> [[GetPrototypeOf]]
       null
  -> [[SetPrototypeOf]]
       null => success; non-null => false / TypeError according to caller API
  -> [[IsExtensible]] / [[PreventExtensions]]
       true; Reflect => false, Object => TypeError, target remains extensible
  -> [[OwnPropertyKeys]]
       numeric child indices, exposed strings, one final then, three symbols
       ordinary named children excluded
```

named lookup 继续复用 Phase 5C 的 child registry precedence，而不是恢复接口名称黑名单：只有
cross-origin exposed Window property 保留优先级。于是 `name="focus"` 仍得到 readonly `focus()`，
`name="document"` 则得到与 numeric index 相同的 stable child WindowProxy；named descriptor 是
non-writable/non-enumerable/configurable，且普通 named child 不进入 ownKeys。`then` 是唯一例外：named
child 可以遮住 fallback，但 ownKeys 中仍只有一个 `then`，并保持为最后一个 string key。enumerator
同时显式保证 indices 在前、3 个 well-known symbols 在末尾，不再依赖 target property 的安装顺序。

这里存在一个 V8 版本边界。仓库当前的 V8 137 会在 foreign global 的
`Object.setPrototypeOf` / legacy proto setter / `preventExtensions` 到达 interceptor 前先执行 security
access check，因而统一抛 `SecurityError`；本地 Chromium 对应 WPT 要求 non-null prototype 和
Object.preventExtensions 抛访问方 `TypeError`，Reflect 返回 false，而 null prototype 成功。D2 没有
为此包装或替换 stable WindowProxy identity。它在每个访问方 Window realm 安装 native intrinsic
adapter，并且只在参数同时满足以下条件时接管：

1. 参数严格等于其 creation Context 的 global proxy；
2. 当前 Context 与该 Context 不同；
3. 既有 Window access-check owner 判定当前调用方不能访问目标。

其余普通 object、same-origin Window 和非法参数全部调用保存在 callback data 中的原始 V8 intrinsic；
回归同时锁住 ordinary delegation、function name/length 和 native-code 形状。cross-origin Window 的
template 也标为 immutable-prototype exotic object，保证 same-origin 转发路径仍遵守 Window 自身的
immutable prototype 约束。detached fallback 本来就是 JavaScript Proxy，则直接由 ownKeys、
setPrototypeOf 和 preventExtensions traps 提供同一结果。

D2 回归覆盖：

- related popup、普通 cross-origin child 与 detached child fallback 的 missing index get/descriptor/
  hasOwnProperty/`in` 全部抛访问方 `SecurityError`；
- unknown named get/descriptor/hasOwnProperty/`in`/set，以及 present/absent index/name 的 set/delete/
  define 全部拒绝；`location` navigation、allowed method 和 receiver check 保持；
- Object、Reflect、legacy proto setter 和 direct `__proto__` 的完整 null/non-null 矩阵，错误分别属于
  访问方 `DOMException` / `TypeError` realm；preventExtensions 失败后仍 extensible；
- exact string/symbol own names、index/then/symbol 顺序、named child 排除，以及
  `document` collision 可见、`focus` collision 被 exposed method 压住；
- ordinary Object/Reflect/legacy proto 操作仍委托 V8，设置 prototype 和冻结普通对象的结果不变。

D2 完成的是 `cross-origin-objects.html` 的 Window internal-method 静态矩阵。当时 related top-level
已由 Phase 5C 动态读取 target Page registry，但 generic nested cross-origin child 仍可能回落到
refresh-time index/name snapshot。D2 将该风险明确留给 D3 前的独立 owner 抽取；下面的 D2.5 完成了
该抽取，没有通过 access 时 drain/retry 修补。

#### Phase 5D2.5：generic nested live child projection

D2.5 把 Phase 5C 的 related-top 特例收敛为一次 WindowProxy callback 内有效的
`CrossOriginWindowChildRegistryOwner { host, parent }`。`parent=None` 表示 target Page 的 top-level
children，`parent=Some(DomHandle)` 表示 generic nested Window 的 direct children。owner 只从 target
Context 已有 host/handle slot 临时解析，不进入全局 cache，也不延长 `JsContextHost` 生命周期；nested
owner 只有在对应 browsing context 仍 live 且拥有 current Document handle 时成立，parked/retired facade
不会伪装成 live registry。

named/indexed WindowProxy handlers 和 `length` accessor 现在共享同一查询流程：

```text
foreign WindowProxy callback
  -> resolve target Context host + scoped parent
  -> synchronously project that Document subtree into the child registry
  -> index/count/name lookup in scoped registry
  -> return the existing stable child WindowProxy for that browsing-context id
  -> missing live entry => SecurityError（then 单独回落到 undefined fallback）
```

因此 get/query/descriptor、indexed enumerator/ownKeys 和 length 不再从 live surface 读取 child slot。
real LocalWindow 的 cross-origin access surface 现在只安装 exposed Window properties、Location、methods、
`then` fallback 和 symbols，child index/name 的物理 seed 为零；只有尚无 current LocalWindow 的预物化
facade或 navigation realm gap facade 保留 snapshot seed，作为没有 live registry owner 时的安全 fallback。
live owner 一旦存在，旧 seed 即使还附着在 proxy storage 上也不参与 named/indexed internal methods，
所以 rename/remove 后不会从 backing surface 复活 stale name/index。

该改动也暴露并修复了一个更底层的 stable WindowProxy 缺陷。未物化 nested child 原先通过
`instantiate_window_proxy_shell()` 预留 identity，但 cross-origin facade 错误复用了创建方 security
token，而且没有把 host/child handle 与 access surface 接回 facade Context。旧静态 index proxy 掩盖了
这一点；live registry 首次返回该 shell 时会直接看到创建方 raw global/intrinsics。现在预物化 shell：

- 保留 V8 unique default security token，不与创建方形成 same-origin alias；
- 在暴露前安装 exact context-host liveness slot 和 `ChildWindowProxyFacadeContextHandle`；
- 在 facade realm 同时初始化 stable global proxy 与独立 minimal access surface；
- handler data 在没有 live LocalWindow 时使用该 cross-origin proxy，后续 realm materialization 仍 detach
  同一 facade 并把 exact proxy 交给新 Context，identity 不复制也不替换。

回归在一个已经跨源 commit 的 child Document 内保留原始 3 个 nested WindowProxy，随后由 child 自己
rename 第 0 个 frame、移除第 1 个、append 一个 `name="then"` 的新 frame。parent 侧在不重新获取 outer
`contentWindow` 的前提下证明：第 0 个 identity 保持，原第 2 个移动到 index 1，新 child 同时等于
index 2 与 named `then`，旧 `nestedNamed` / `document` 的 get/has/descriptor 全部立即抛访问方
`SecurityError`，普通 named child 仍不进入 ownKeys，且三个 returned child 都继续拒绝 `.document`。
原有 detached fixture 进一步证明 context 尚未物化时不会泄漏 raw global。

D2.5 统一的是 child registry authority，不提前冒充 D3 membrane。它完成时，child WindowProxy 的
observer-relative 选择仍复用现有 top projection helper；不同 same-origin incumbent 的
Function/accessor prototype、wrapper cache 和异常 realm 随后已由 D3a 收敛，非-top observer 的 endpoint
projection 仍留给 D3b。

#### Phase 5D3a：per-accessing-Realm `CrossOriginPropertyDescriptorMap`

D3a 已完成 Function/accessor membrane，范围是 HTML
`CrossOriginGetOwnPropertyHelper` 返回的 Window/Location method 与 accessor wrapper。它没有为每个
target 复制一套 wrapper；缓存 key 是“访问方 Realm + interface member”，target identity 仍由 stable
WindowProxy/Location object 承担。这一点与 Chromium 的边界相同，也是复用 child-frame stable
WindowProxy/realm 基础后必须补上的访问方投影层。

##### Chromium / WPT 合同

本地 Chromium `a03603fe9af6` 的
`third_party/blink/renderer/platform/bindings/v8_cross_origin_property_support.cc` 在 isolate 的 current
Context 中取得 `ScriptState`，按 world 与 callback 缓存 `FunctionTemplate`，再对 current Context 调用
`GetFunction()`。template 还绑定对应 Window/Location interface signature，让 receiver brand 在 native
callback 和 WebIDL 参数转换之前成立。generated binding 入口位于
`third_party/blink/renderer/bindings/scripts/bind_gen/interface.py`。

对应 WPT 不只要求“属性可调用”，还要求：

- Window 的 `close`、`focus`、`blur`、`postMessage` 是 readonly data descriptor，name 分别等于属性名，
  length 分别为 `0/0/0/1`；
- Window 的 `location/window/frames/self/top/parent/opener/closed/length` 是 accessor descriptor，getter
  name 为 `get <name>`、length 为 0；只有 `location` 有 `set location`，length 为 1；
- Location 的 `replace` 是 length 1 的 readonly method，`href` 只有 `set href`、length 1；
- 同一 Realm 重复 `[[Get]]` / `[[GetOwnProperty]]` 得到同一个 function；两个 same-origin observer Realm
  得到不同 function，且各自继承本 Realm 的 `Function.prototype`；
- observer 不同不会复制 target：双方仍观察同一个 WindowProxy 和同一个 Location identity。

直接证据来自 Chromium checkout 中的
`cross-origin-objects-function-{common,caching,name,length}.html/js` 与 `cross-origin-objects.html`。

新增 core 回归构造 A（parent）与 B（same-origin `srcdoc` observer）两个 Realm，共同观察跨源 child C。
在接入 D3a 前，红灯输出精确暴露了旧 target-surface 模型：四个 method 和可见 accessor 的
`Function.prototype` 均不是访问方 Realm；`window/frames/self/top/parent/opener` descriptor 没有 getter；
readonly attribute 仍带 throwing setter；A/B 之间 method、getter、Window location setter、
Location.replace 与 href setter 的 identity 全部相同。该失败不是测试等待或 fixture 时序问题，而是 wrapper
确实由 target realm 唯一创建。

##### Realm-local cache owner

新模块 `cross_origin_property_descriptor_map.rs` 以 accessing Context 的 V8 hidden
extras-binding object 作为 cache owner，并用 isolate-wide private symbol 区分每个 member：

```text
cross-origin [[Get]] / [[GetOwnProperty]]
  -> incumbent Context（无 incumbent 时才回落 current Context）
  -> Context::get_extras_binding_object()
  -> member private slot
       hit  => 返回该 Realm 已有 native Function
       miss => 在该 Context 创建、设置 name/length、写回 slot
  -> descriptor/value 返回给访问方
```

这个 owner 选择同时满足三条 lifetime 不变量：

1. wrapper 只被 V8 tracing 持有，不在 Rust host/global cache 中形成强引用环；
2. stable WindowProxy navigation rebind 到新 Context 时，新 Realm 自然得到新的 extras object，不会复用旧
   LocalWindow generation 的 function；
3. 同一 Realm 观察多个 cross-origin target 时共享 HTML 规定的 member wrapper，function 本身不捕获某个
   target Page。

Window 四个 method、九个 getter、唯一 location setter，以及 Location 的 replace/href setter 都经过该
cache。`[[GetOwnProperty]]` 不再复用 target surface 的旧 data descriptor：九个 Window attribute 统一返回
non-enumerable/configurable accessor descriptor，readonly attribute 的 `set` 精确为 `undefined`；method 与
Location.replace 继续是 non-writable/non-enumerable/configurable data descriptor。

##### target-neutral wrapper 与 receiver-owned execution

per-Realm wrapper 不能把创建时的 ambient `JsContextHost` 当作 target。否则 opener 首次取得 popup.close
后调用该缓存 function，会关闭 opener；同理，postMessage 和 Location navigation 会进入错误 Page 的
queue/URL owner。D3a 因而把 native callback 划成两个阶段：

```text
accessing Realm
  -> receiver / WebIDL 参数 / exception Realm
  -> receiver 上的 stable child handle 或 related-top target marker
  -> target WindowProxy creation Context
  -> liveness-checked Context host slot
  -> target Page owner 执行 close / postMessage / Assign / Replace
```

- URL 的 USVString conversion 留在 accessing Realm；解析到 target 后才进入 target Context 和 navigation
  owner，避免 target realm 泄漏转换异常；
- `postMessage` 仍由既有 endpoint resolver 决定 top/child/popup endpoint，但排队 host 从 receiver 的 target
  Context 取得，`event.source` 继续是 stable source WindowProxy；
- related popup `close()` 只请求 target Page 的唯一 close transaction；本小节实施时 `focus()` / `blur()`
  仍是 receiver-brand no-op，后续 Phase 5L2 已让 `focus()` 进入 exact target Page transaction，并按 Chromium
  保留 top-level `blur()` no-op；
- attribute getter 先校验 receiver，再在 target access-surface Context 读取 live relation/scalar；
  `window/self/frames` 直接返回 exact receiver，避免创建等价但不相等的 identity；
- Chromium 依赖 V8 interface signature 拒绝错误 Location receiver；当前 Moli 的 minimal
  cross-origin object 没有对应 template signature，因此 cached `href` setter 与 `replace` 在参数转换前
  显式验证 Location proxy brand。扩大矩阵曾捕获 `hrefSetter.call(null, ...)` 被错误当成访问方 global 的
  回归，修复后又加入 `replace.call(null, ...)` 防回归。

双 Realm 回归最终证明：每个 Realm 内所有 wrapper 重复读取稳定，A/B wrapper 全部不同且原型分别属于
A/B，name/length/descriptor shape 一致，非法 receiver 的 `TypeError` 与 unknown property 的
`SecurityError` 都属于发起 Realm；同时 A/B 观察到的 target Window 和 Location identity 仍完全相同。
related popup 端到端回归还同时覆盖跨 Page message source、target Location assignment/replace、
target-only close 和 transport-failure error Document 后的 stable WindowProxy。

D3a 不改变 child registry authority，也不宣称完成 observer-relative endpoint。当时 generic nested child
lookup 已经 live，但从非-top same-origin observer 返回 child WindowProxy 时仍复用 top-oriented projection
helper。下面的 D3b 已把这条历史缺口闭合：observer/target pair 只用于本次 callback 的授权判断，最终仍
返回 browsing-context-owned stable target identity，而不是把 child wrapper 放进 Realm-local function cache。

#### Phase 5D3b：observer-relative child endpoint projection

D3b 修复的是一个三方关系，不能简化成“parent 是否和 child 同源”：A 是 top，B/C 是 A 的两个 direct
child；A 与 B/C 跨源，而 B 与 C 同源。B 通过跨源的 `parent.frames[1]` 或 named property 取得 C 时，应得到
C 的 stable WindowProxy 并拥有完整同源访问；A 对同一对象仍必须只看到 restricted surface。旧实现虽然
已经由 D2.5 live-resolve 到 C 的 browsing-context handle，最后却无条件调用
`child_browsing_context_window_proxy_for_top()`，把 A/C 的关系误当成所有 observer/C 的关系。

##### Chromium 边界

本地 Chromium `a03603fe9af6` 的
`third_party/blink/renderer/core/frame/{dom_window.cc,window_properties.cc}` 与
`third_party/blink/renderer/bindings/core/v8/window_proxy.cc` 没有为每个 observer 克隆 child Window：

- `DOMWindow::AnonymousIndexedGetter()` 从 `FrameTree::ScopedChild(index)` 取得 child 的 `DomWindow()`；
- `WindowProperties::AnonymousNamedGetter()` 同样解析 scoped child，再通过 current Realm 的
  `ToV8Traits<DOMWindow>::ToV8(...)` 投影；
- `DOMWindow::Wrap()` 最终返回 `WindowProxyManager` 持有的 `GetGlobalProxy()`，因此不同 observer 共享
  browsing-context identity；
- 访问是否展开由 current Realm / `BindingSecurity` 和 child security origin 决定，而不是由 target parent
  预先选一份永久 restricted/full wrapper。

WPT
`third_party/blink/web_tests/external/wpt/html/browsers/windows/nested-browsing-contexts/frameElement-siblings.sub.html`
也直接通过 `parent.frames[0]` 验证 sibling Window 的访问结果随 same-origin-domain 关系变化。D3b 沿用
这条边界：registry 决定“是哪一个 child”，observer-relative access 决定“本次能否 materialize/展开”，
stable WindowProxy 决定“对象 identity 是哪一个”。

##### Moli cutover

实现没有给 synthetic facade 增加第二套动态 index/name trap，而是把责任收回已成熟的 stable
WindowProxy/realm 基础：

```text
non-top observer B reads parent.frames[index/name]
  -> parent/top is A's real stable top-level WindowProxy
  -> cross-origin handler resolves A's live scoped child registry
  -> callback-local observer = incumbent B execution-context identity
  -> compare B origin with target child C dispatch-scope origin
       same origin => promote/reuse C's exact stable WindowProxy + LocalWindow realm
       cross origin => keep C's restricted facade
  -> V8 access check continues to evaluate every later observer against C
```

- child Realm 的 `parent` / `top` 在 main top Realm 存在时直接引用其 stable global proxy；只在尚无 current
  top Realm 的 bootstrap gap 保留旧 detached safe projection fallback。这样 B 的 `parent`、C 的
  `parent/top` 和 A 自身不再是三份 synthetic identity；
- `CrossOriginWindowChildRegistryOwner` 仍只持 callback-scoped target host/parent authority，但新增独立的
  `CrossOriginWindowObserver { host, identity }`。observer 从 incumbent Context 的 liveness slot 解析，不被
  target holder creation Context 覆盖，也不会跨 callback 缓存 raw host pointer；
- same-host 路径复用 `window_execution_context_can_access_dispatch_scope()`；related Page 路径把原先只允许
  top-to-top 的检查推广到 target dispatch scope。后者仍要求访问方 identity current、两 Page 属于同一个
  related script agent，并对 target 的 live origin 做比较；opaque origin 不因两个 host-local owner id 碰巧
  相同而放行；
- observer 有权限时，预物化 restricted facade 会 detach 并把 exact proxy 交给 C 的正式 Context。A 早先保存
  的引用严格相等，但在 C materialize 后读取 `document` 或页面 marker 仍由 V8 access check 拒绝；
- index、named get 与两种 descriptor 都经过同一个 owner/observer 决策。Realm-local cache 继续只保存
  D3a 的 method/accessor function，不保存 WindowProxy。

##### 回归证据

core 回归构造 `localhost` top A 与两个 `127.0.0.1` child B/C。A 先保存 C 的 restricted facade，随后 B
通过 `parent.frames[1]` 和 `parent.observerTarget` 访问 C。接入 D3b 前稳定红灯为：lookup 已返回对象，但
首次 marker write 抛访问方 `SecurityError`；A 侧 identity 与两个 denial 均正常，证明失败不是加载等待或
错误 target。修复后同时证明：

- index/name/getOwnPropertyDescriptor 都返回同一个 C proxy；
- B 可读写 C Document、Location、intrinsics，且 `document.defaultView === target`；
- C 的 `parent/top` 与 B 观察的真实 A proxy identity 一致；
- A 保存的引用不变，且 A 对 C 的 Document/marker 继续得到 `SecurityError`。

related Page 回归又把 opener A 与跨源 popup P 放在两个 `JsContextHost` 中，并让 P 的第一个 child C 导航到
A 的精确 origin。P 对 C 的 Document 被拒绝；A 经跨源 `popup[0]` 取得同一 C proxy 后可以完整读取 C 的
Document/realm，且 C 的 `parent/top` 都严格等于 popup stable WindowProxy。这覆盖了 related-agent
cross-host dispatch-scope access，不把同宿主 sibling 通过误当成跨 Page 证据。

#### Phase 5E1：非命名 `noopener` / `noreferrer` 的 Fresh Page single-owner

E1 先处理不需要把 WindowProxy 同步交回 creator 的最小 creation-policy 纵切：production
`window.open()` 的空 target / `_blank`，以及 hyperlink `_blank` 的 implicit 或显式 noopener。它们仍然
创建可被 CDP/BiDi 观察的 auxiliary top-level target，但 author 调用方只得到 `null`，所以没有理由先在
opener Page 内创建一份 lightweight Window、Document 和 loader，再等 protocol 创建第二份真实 Page。

##### Chromium / WPT 合同

本地 Chromium `a03603fe9af6` 的责任顺序很重要：

- `LocalDOMWindow::open()` 先从 entered Window 完成 URL 和 feature parsing；生成 referrer 时，只有
  `noreferrer` 选择 `kNever`，单独的 `noopener` 仍使用 entered Document 的 Referrer Policy；
- 同一个函数随后调用 `FrameTree::FindOrCreateFrameForNavigation()`，对返回的 frame 发起导航，最后才在
  普通 target 的 `noopener` 分支返回 `nullptr`。`_self` / `_parent` / `_top` 在该 null-return 判断之前
  返回 existing Window；
- `FrameTree` 先查 current tree / related Pages / existing named context，找不到才调用
  `CreateNewWindow()`。因此 `noopener` 不是“永远跳过 named lookup 并新建窗口”的同义词；
- `CreateNewWindow()` 在真正新建 auxiliary context 前检查 sandbox popup flag，并且只在
  `!features.noopener` 时 clone opener session-storage namespace。

对应 WPT 把容易混淆的边界拆得很清楚：

- `the-window-object/window-open-noopener.html` 要求第二次带 noopener 的 named `window.open()` 仍导航
  已有 target，但返回 `null`，原 target 的 opener 不被改写；special target 则忽略 null-return policy；
- `the-window-object/window-open-noreferrer.html` 要求新窗口 name 为空、`document.referrer` 为空、
  `window.opener` 为 `null`；
- `referrer-policy/generic/inheritance/popup-inheritance-about-blank.html` 要求普通 initial
  `about:blank` popup 的 `document.referrer` 保留 creator 完整 URL，不受 creator Document 的
  Referrer Policy 截短；
- `webstorage/storage_session_window_noopener.window.js` 要求新 noopener window 不复制 creator 的
  session storage；`storage_session_window_reopen.window.js` 则要求普通 named reopen 保留同一 Window；
- `windows/noreferrer-window-name.html` 同时证明两件事：新建的 named noreferrer windows 不应互相进入
  同一可复用 name group，但一个预先存在的 named iframe/window 仍可以被 noreferrer navigation 命中。

本纵切阅读了上述 Chromium source/WPT，没有编译 Chromium，也没有运行 upstream WPT；这里的 WPT
结果是合同对照，不是 Moli 新的通过声明。另用本地 `out/Default/chromedriver` 驱动
`out/Default/chrome`（`Chromium 147.0.7709.0`，headless），从 HTTP creator 分别打开
`about:blank`：

| 调用 | target realm 观察值 `[document.referrer, opener===null, href, name, origin]` |
| --- | --- |
| `window.open('about:blank', '_blank', 'noopener')` | `[creator 完整 URL, true, 'about:blank', '', 'null']` |
| `window.open('about:blank', '_blank', 'noreferrer')` | `['', true, 'about:blank', '', 'null']` |

这个 probe 直接证明 HTTP header eligibility、initial empty Document referrer 和 destination
Document referrer 不能共用一个字符串或一个计算入口。

##### 稳定红灯与旧 owner 违反路径

renderer owner 回归先在 production Page 上执行
`window.open("about:blank#fresh-agent", "_blank", "noopener")`。旧实现虽然预留了
`RendererScriptAgentAdmission::Fresh` Page，activation 中仍带 `popup_id = Some(2)`，证明 opener host
同时创建了 lightweight browsing-context identity。期望改为 `popup_id = None` 后稳定失败。

protocol 集成回归又用真实 HTTP server 观察 `/noopener`。旧实现得到两次请求：一次来自 opener 内的
lightweight loader，且没有 `Referer`；一次来自 target Page，带 creator URL。失败形状为：

```text
[("/noopener", None),
 ("/noopener", Some("http://127.0.0.1:<port>/opener"))]
```

这不是 redirect、retry 或测试 server 误计数，而是两个独立 loader。切掉 lightweight loader 后，请求数与
header 已转绿，但增强后的 target-session probe 继续稳定失败：网络 `Referer` 已正确，committed realm 的
`document.referrer` 仍为空。该第二阶段红灯把缺口定位到 main-Document commit fact，而不是再给请求层加
header patch。拆出 destination Document referrer 后，`about:blank` 又稳定暴露第三阶段红灯：它与 target
的 initial URL 相同，不发生 replacement commit，target realm 仍观察到空 referrer。加入
`about:blank#fragment` 后 same-document 路径同样失败，证明 initial empty Document 必须在默认 realm
创建前独立接收 creator referrer，不能等待 navigation commit 补写。

##### Moli cutover

E1 把 creation policy、Page reservation、导航与 Document commit 串成一份 typed transaction：

```text
entered creator Document
  -> resolve opener/referrer policy + destination URL
  -> reserve RendererPendingAuxiliaryPage(Fresh)
  -> freeze { initial-document, network, destination-document } referrers
  -> emit PendingPopupActivation { popup_id: None, exact referrers, reservation }
  -> protocol creates one target and consumes that reservation
  -> fresh Page bootstrap installs initial-document referrer before its first realm
  -> target Page owns at most one destination navigation
  -> replacement commit installs destination-document referrer before the new realm
```

- `WindowOpenFeatures` 现在分别回答 `suppresses_opener()` 与 `suppresses_referrer()`；parser 仍保持
  `noreferrer ⇒ noopener`，但 `noopener` 不再错误清空 referrer；
- creator 在同一个 decision point 冻结三个不同结果：initial empty Document 使用 creator 完整 URL
  （`noreferrer` 时为空）；HTTP network referrer 额外受 header eligibility 约束；destination
  `document.referrer` 使用 navigation referrer policy，但 `about:blank` 保留 initial 值。
  `RendererPendingPopupActivation` 显式携带三者；`Some("")` 表示显式抑制，`None` 只留给尚未迁移、
  仍依赖 browser-context inference 的 producer；
- production 的非命名、可解析、非 `javascript:` suppress-opener 路径只预留 Fresh Page，不调用
  `open_lightweight_popup_window()`，不创建 opener-local WindowProxy/Document/loader，也不携带 creator
  session-storage snapshot；`window.open()` 同步返回 `null`；
- hyperlink `_blank` 使用同一边界。没有 `rel=opener` 时的 implicit noopener、`rel=noopener` 和
  `rel=noreferrer` 都进入 Fresh Page；只有 `noreferrer` 抑制 referrer。当前端到端回归直接覆盖 anchor，
  `<area>` 虽共享 hyperlink activation 路径但尚未单独运行 WPT；
- protocol 的 `PopupTargetCreation` 原样携带三个 referrer；initial 值在 fresh Page 默认 realm 创建前
  安装，network/destination 值继续进入 exact target-owner navigation claim。任何一项都不从新 target
  的 initial `about:blank` 或消费时的 current session 反推。target admission 后仍只有该 target Page
  发起 destination navigation；
- `NavigationDispatchState` 把 `document_referrer` 放在 heap-owned commit environment 中，与
  `request_headers` 分开冻结。Fetch interception 可以修改 transport header，但不能顺带改写已经接受的
  Document environment；随后
  `RendererMainDocumentCommitSeed → RendererMainDocumentCommit` 把值送进 renderer，在默认 realm 和
  document-start script 创建前安装到 `DocumentPolicyContainer`；
- fresh initial Page 使用独立的 `initial_document_referrer` bootstrap 输入；它只初始化 Document
  environment，不伪造 `MainDocumentCommit` observation。因此精确 `about:blank` 和
  `about:blank#fragment` 都能在没有 cross-document commit 时保持 Chromium referrer；
- 三个 popup referrer 收拢为 heap-owned typed bundle，target admission future 也在 generic renderer
  output projection 边界 `Box::pin`。destination Document referrer 与 source origin / secure-context
  则组成一个 heap-owned commit environment；这些结构既表达同一组冻结事实，也避免普通
  `Target.createTarget` 为未走到的 popup/navigation 分支预留大栈帧；
- 没有 production Page allocator 的 renderer standalone fixture 暂时保留 lightweight fallback，避免把
  单元测试适配器误当成真实 browser owner。production 回归要求 reservation 必须存在，因此该 fallback
  不会掩盖 CDP 双 loader。

这一纵切建立的窄不变量是：

1. 一个新建的非命名 suppress-opener auxiliary context 只有一个 browsing-context/Page identity 和一个
   destination loader；
2. `window.open()` 返回值、`window.opener`、script-agent admission、session-storage clone policy 和
   referrer policy 都来自同一 creator-side decision；
3. initial empty Document referrer、网络 `Referer` 与 destination `document.referrer` 在同一
   creator-side decision 中分别冻结；它们可以不同，而 `noreferrer` 明确把三者都置空；
4. target/session attach 观察的是上述 Fresh Page 的 committed realm，不是 opener-local mirror。

#### Phase 5E2A：related-page named `window.open()` 的 renderer group authority

E2A 处理 E1 有意跳过的第一类 named target：同一个 related script agent 中，由
`window.open()` 创建或复用的 top-level auxiliary context。这个范围先统一最重要的同步 identity：
新建 named popup 返回的 WindowProxy、creator 立即写入的 Document、protocol target 采纳的 Page，以及
下一次按 name 找到的 context 必须是同一实体。它不把 browser-context-wide target-name map 当成
browsing-context group，也不把所有 named producer 一次性塞进该 map。

##### Chromium / WPT 选择顺序

本地 Chromium `a03603fe9af6` 的 `LocalDOMWindow::open()` 与 `FrameTree` 给出以下责任边界：

1. 先在 renderer 的 frame tree / related Pages 中选择现有 target，找不到才请求创建新 Page；
2. current Page 的 frame tree 优先于 related top-level Page，因此 named iframe 不能被同名 popup 抢走；
3. closing frame 不参与查找；复用的是既有 frame/WindowProxy，导航不会制造第二个 browsing context；
4. existing target 且本次不 suppress opener 时更新该 target 的 opener；本次为 noopener/noreferrer 时仍导航
   existing target，但返回 `null`，并且不能用本次 suppressed edge 覆盖原有 opener；
5. 真正创建的新 noopener/noreferrer context 属于新的 group/name policy，不能因为 browser context 相同就被
   原 creator 再次按 name 命中。

`window-open-noopener.html` 直接覆盖第 4 点；`windows/noreferrer-window-name.html` 同时覆盖 existing
named target 可被命中与 newly-created noreferrer contexts 不应互相复用。E2A 沿用 E1 的源码/WPT 对照
边界：本轮没有编译 Chromium，也没有运行 upstream WPT，因此这里只声明 Moli 聚焦回归，不声明
上述 WPT 已通过。

##### 稳定红灯：同一个 name 的两套 owner

接入前，named popup 同时依赖两份 registry：

- renderer `JsContextHost::lightweight_popup_window_names` 返回 opener realm 中的 lightweight Window/Document；
- protocol `BrowserContext::target_window_names` 再选择一个独立 target Page。

新增 protocol 回归让 creator 执行：

```javascript
const popup = window.open("about:blank", "reportWindow");
popup.document.body.dataset.owner = "renderer-page";
```

creator 同步观察到 `reportWindow|renderer-page`，但 attach 新 target 后旧实现稳定得到：

```text
undefined||false
```

期望是 `renderer-page|reportWindow|true`。三个字段分别证明 target 看到了另一份 Document、另一份 name
状态和缺失的 opener edge；这不是 CDP attach timing。回归随后主动清空 protocol
`target_window_names`，再用动态改名后的 name 执行 noopener reuse，用来证明修复不能只是让两张 map
更勤快地同步。

第一次 workspace 门禁又捕获两条过期 characterization，而不是实现超时：

- renderer owner test 用 production named `window.open()` 后等待 opener-local popup loader。E2A 不再启动
  这份 mirrored loader，server 因而永久等不到请求；测试在 E2A 当时改为尚未迁移的 named hyperlink
  producer，只锁定 legacy popup terminal 的 stable Page route。E2C 迁移 hyperlink 后，这条 characterization
  同样过期并被删除：继续等待 opener-local response 已经与 single-owner 不变量相反，新的 renderer/protocol
  回归改为观察 typed activation、exact Page handoff 和 target realm；
- protocol background test 手工向 `target_window_names` 写入一个与 creator 无 related-page 关系的 target，
  然后期待 `window.open()` 被该 map 重定向。新回归反向锁定：renderer-selected named popup 必须创建自己的
  exact related Page，旧 background target 的 URL/Document 和 active target 都不变。

##### Moli cutover

E2A 的 renderer/protocol 流程如下：

```text
entered Window.open(url, name, features)
  -> current Page named-child lookup
       hit: navigate exact child; return Window or null according to suppress-opener
  -> related-page top-level group lookup
       hit: return stable proxy (or null), emit activation { exact renderer Page residence }
  -> no hit + opener preserved + non-javascript URL
       reserve RelatedAuxiliaryPage
       synchronously stage real initial PageVm/realm/Document with window.name
       return that Page's stable proxy
       emit activation { exact pending Page reservation }
  -> protocol projection
       resolved residence: navigate the target already owning that exact Page
       pending reservation: create one target and adopt that exact staged Page
```

具体 owner 变化如下：

- `RendererPageScriptEnvironment` 现在持有一个 `RendererRelatedPageGroup` 和一个
  `RendererRelatedPageTopLevelTargetState`。后者把 exact `{RendererOwnerLocalHostId, PageId}`、stable
  WindowProxy、Page-scoped opener edge、lifecycle 与 name 放在同一状态节点；group registry 只持
  `Weak`，不会用 name map 延长已关闭 Page 的 V8 lifetime；
- related auxiliary environment 从 live source environment clone group capability。它不在已经进入 V8 isolate
  时回借 isolate holder；首次实现确实被聚焦回归捕获为 `RefCell already mutably borrowed`，改为从 source
  capability 传递后消除了这条 reentrancy 路径；fresh Page 仍创建自己的 group；
- 初始 auxiliary realm bootstrap 在安装 WindowProxy/opener 的同时登记 `window.name`。公开 name setter 会
  原子地从旧 name bucket 移除并注册新 name；cross-document replacement 复用同一 Page state，并在新 realm
  bootstrap 后恢复 name。`Closing`/`Closed` 在 renderer 可观察时立即注销，lookup 也再次检查 lifecycle；
- lookup 对当前 top-level Page 本身优先，再按 group 注册顺序选择第一个 live top-level target。空 name 与
  `_self` / `_parent` / `_top` / `_blank` 不进入普通 name registry；special
  target 继续走既有 navigation authority；
- 新 named opener-preserving、非 `javascript:` popup 复用 E1 前已建立的 synchronous real initial realm
  staging，只是把 target name 带入该 realm，并移除“named 必须走 lightweight”分支。creator 的立即 DOM
  mutation 因而落在 target 后续采纳的 exact Document；
- existing related named target 不创建 `Page.windowOpen` 事件、不预留 Page，也不创建 lightweight record。
  非 suppress-opener 调用把 target 的 Page-scoped opener edge 更新为 entered Window；noopener/noreferrer
  调用不修改旧 edge、仍发出 exact-target navigation activation，并向 caller 返回 `null`；
- `RendererResolvedPopupTarget` 是 activation 上的 typed destination claim。protocol 通过 host id 与 Page id
  同时扫描 active/background target，找不到就 fail closed，不回退到 name；这避免同一个 renderer host 中
  多个 related Page 被误路由。migrated producer 还会显式设置 renderer-owned new-target disposition；只有
  该 fact 才让带新 Page reservation 的 activation 跳过 protocol name lookup。E2A 当时尚未迁移的 hyperlink
  producer 即使乐观预留了 Page，仍保留 legacy projection fallback，避免把后续 E2B/E2C 行为偷偷混入 E2A；
- `BrowserContext::target_window_names` 暂时保留，服务 DevTools projection 和未迁移 producer。E2A 回归在
  清空它后仍只导航原 target，证明它不再是 migrated related `window.open()` 的选择 authority。

这一纵切建立的窄不变量是：

1. 新建普通 named `window.open()` 只有一个 initial Page/realm/Document；creator 立即 mutation 与 CDP
   target evaluation 观察同一对象；
2. related top-level name lookup 返回同一 stable WindowProxy，并把 exact renderer Page residence 送到
   protocol，不靠 target name 二次选择；
3. 动态 `window.name`、Page navigation 和 close lifecycle 共享同一 renderer state；旧 name 或 closing
   target 不能继续被命中；
4. existing target 的选择与 noopener 返回/opener mutation policy 分开：noopener 仍导航 exact target、
   返回 `null` 且保留旧 opener；
5. named iframe lookup 保持在 related top-level lookup 之前，且 noopener 命中 existing iframe 时仍导航、
   返回 `null`，不会误建 popup。

E2A 本身仍不是完整 browsing-context-group 实现。多个 related Page/嵌套 frame 同名时的完整 Chromium
frame-tree ordering、`CanNavigate`、focus 后来分别由 E2E/M.1/L2 收口；local committed-response COOP group
switch 又由 G1 收口，跨 agent 的真正 remote endpoint 仍需后续纵切；
E2A 当时保留的新建 named noopener/noreferrer fresh-group policy 由下一节 E2B 接手。当前 related registry
仍只覆盖 related same-agent top-level contexts。

#### Phase 5E2B：新建 named suppress-opener 的 Fresh group/name handoff

E2B 处理 E2A 明确保留的另一半 named `window.open()`：renderer 已按 current frame tree、related Page
group 完成查找，但没有 existing target，且本次 `noopener` / `noreferrer` 抑制 opener。此时 target name
仍属于新 browsing context 的真实状态，却不能让这个新 Page 回到 creator 的 related group，也不能借
browser-context-wide name map 让两个本应隔离的 Page 互相复用。

##### Chromium / WPT 合同与本轮范围

本轮继续对照本地 `~/chromium/src`：

- Blink `LocalDOMWindow::open()` 先调用 named target lookup；existing target 仍被导航，只有返回给 caller
  的 handle 受 noopener policy 影响。查找失败后才创建新 Window；
- Blink `FrameTree` 的 lookup 顺序仍是 current tree、Page tree、related Pages，并排除 closing Page；
- Content `RenderFrameHostImpl` 在 opener 被 suppress 的新建路径分配新的 virtual browsing-context group /
  `BrowsingInstance`。因此“先查 existing target”与“新 target sever group”是两个连续决策，不可合并为
  browser-context name lookup；
- WPT `auxiliary-browsing-contexts/named-lookup-noopener.html` 要求连续两次使用同一普通 name 的
  noopener `window.open()` 创建两个不同窗口，同时每个新窗口自己的 `window.name` 仍等于请求 name；
- WPT `windows/noreferrer-window-name.html` 对 noreferrer 锁定同样的“不互相复用”，并再次要求预先
  existing 的 named iframe/window 仍可先被命中。

这里仍是源码/WPT 合同对照，没有编译 Chromium，也没有运行 upstream WPT。本纵切只迁移 production
`window.open()` 的可解析、非 `javascript:`、普通 named suppress-opener 新建路径；hyperlink 已在下一节
E2C 迁移，form named target 又在 E2D 迁移；完整 nested-frame ordering、sandbox/COOP/remote endpoint
继续保留。

##### 稳定红灯与违反路径

renderer owner 回归在同一个 production opener Page 中，用相同 `isolated-popup-name` 先后执行 named
`noopener` 与 `noreferrer`。旧实现两次都落入 `open_lightweight_popup_window()`；第一条断言稳定得到：

```text
popup_id: left Some(2), right None
```

这证明即使 reservation 已标为 `RendererScriptAgentAdmission::Fresh`，opener host 仍额外拥有一份 caller
永远拿不到的 lightweight Window/Document identity。protocol 回归随后用同一 name 连续创建两个 target；
旧实现把后一个 target 写入 `BrowserContext::target_window_names`，稳定失败为：

```text
left: Some("TID-2")
right: None
```

这个 map 会把不相关 Fresh group 暴露成 browser-context-wide named target。代码审计同时确认 target name
只存在于 lightweight/protocol projection，fresh Page 的首个真实 realm 没有 creator-frozen name 输入。

##### Moli cutover

E2B 把 group policy 与首个 realm name 作为 renderer creation decision 的一部分：

```text
entered Window.open(url, ordinaryName, suppress-opener)
  -> current frame / related Page named lookup
       hit: navigate exact existing target; return null; preserve its opener edge
  -> no hit
       reserve RendererPendingAuxiliaryPage(Fresh)
       emit activation {
         popup_id: None,
         new_target_disposition: FreshNamed,
         target_name: ordinaryName,
         exact referrers + reservation
       }
  -> protocol creates one target, never consults/publishes the global name projection
  -> fresh Page bootstrap installs ordinaryName in the real Window slot and Page-group state
     before document-start scripts
  -> lookup from that Page may resolve itself; creator/other Fresh groups cannot resolve it
```

- 原来的 `renderer_selected_new_target: bool` 提升为
  `RendererPopupNewTargetDisposition::{Related, FreshUnnamed, FreshNamed}`。renderer 在 lookup/creation
  decision point 同时冻结“是否新建”“属于哪个 group”“首 realm 是否携带普通 name”；protocol 只消费该
  fact，不从 `can_access_opener`、target string 或 name map 重建 policy；
- suppress-opener 的空 target / `_blank` 继续标记 `FreshUnnamed`；新建 ordinary named
  `noopener`/`noreferrer` 标记 `FreshNamed` 并直接预留 Fresh Page，不再调用 lightweight popup owner。
  opener-preserving staged Page 显式标记 `Related`；E2B 当时尚未迁移的 `javascript:` / hyperlink producer
  不冒充已完成的 renderer decision；
- protocol 仅在 disposition 缺失时保留 legacy target-name fallback。`FreshNamed` target 不写入
  browser-context-wide `target_window_names`；`Related` 仍可保留 DevTools/legacy projection，但 exact
  renderer residence 才是 migrated lookup authority；
- `initial_top_level_browsing_context_name` 沿 initial empty-Document Page build 传到 renderer。
  `ScriptVmDefaultWorldBootstrap` 在 `finish()` 和 document-start scripts 之前，同时更新真实 V8
  `WINDOW_NAME_SLOT` 与 `RendererPageScriptEnvironment` 的 top-level name。后续 cross-document navigation
  继续复用 E2A 的 stable Page/group state，不需要 protocol 补写 realm；
- related staged Page 的这一 bootstrap 输入保持 `None`，避免 protocol adoption 用初始 target string 覆盖
  creator 在同步 WindowProxy 上已经完成的动态 `window.name` 修改。

这一纵切建立的窄不变量是：

1. 同一 opener 对相同普通 name 连续执行新建 `noopener` / `noreferrer`，每次都得到不同 Fresh Page，且
   不创建 opener-local lightweight owner；
2. 每个 fresh Page 的真实 realm 都观察到请求 name 与 `window.opener === null`；name 在首 realm 创建和
   document-start script 执行之间安装，并随该 stable Page 的 navigation 保留；
3. fresh Page 不进入 browser-context-wide name projection，也不进入 creator/其他 fresh Page 的 related
   lookup；它仍可在自己的 private Page group 中按 live name 精确命中自己；
4. existing named child/related target 的 lookup 仍先于新建，suppress-opener 只改变返回值和本次 opener
   mutation policy，不把 existing target 错误 sever 到 Fresh group；
5. group/name/referrer/session-storage policy 与 exact Page reservation 继续来自同一 renderer activation，
   protocol target admission 不产生第二份 Window、Document 或 loader。

#### Phase 5E2C：ordinary named hyperlink 的 renderer group lookup/creation

E2C 迁移 `<a>` 与共享 hyperlink activation 的 `<area>` 普通命名 target。它不新建另一套 link popup
registry，而是让已有 full creator capability 的 hyperlink producer 复用 E2A 的 related Page live registry
和 E2B 的 typed group disposition。与 `window.open()` 不同，hyperlink 没有同步 WindowProxy 返回值；但
target 选择、opener/referrer policy、initial realm name、Page admission 和 destination navigation 仍必须在
同一个 renderer decision point 完成，不能因此退回 protocol name map。

##### Chromium / WPT 合同与当前差距

本轮直接对照本地 Chromium `a03603fe9af6`：

- `third_party/blink/renderer/core/html/html_anchor_element.cc` 的 `HandleClick()` 先构造
  `FrameLoadRequest`，调用 `AnchorElementUtils::HandleRelAttribute()` 冻结 link relation，再把同一 request
  交给 `FrameTree::FindOrCreateFrameForNavigation()`；anchor 并没有一条绕过 frame-tree lookup 的独立
  browser-process name-map 路径；
- `anchor_element_utils.cc` 中 `noreferrer` 同时设置 no-referrer/noopener，`noopener` 只设置 noopener，
  `_blank` 在没有显式 `rel=opener` 时隐式 noopener。这些 policy 在 target 查找前已存在，但并不禁止命中
  existing named frame/window；
- `core/page/frame_tree.cc::FindFrameForNavigationInternal()` 的顺序是 source subtree、当前 Page 剩余
  frame tree、每个 non-closing related Page 的整棵 frame tree，最后才询问 embedder；每个候选还经过
  `CanNavigate()`。命中另一 Page 后会 focus，再次检查 detach；查找失败才由 `CreateNewWindow()` 新建；
- `core/page/create_window.cc` 只在 `!features.noopener` 时 clone session-storage namespace。因而
  “是否命中 existing target”“新 Page 属于 Related 还是 Fresh group”“是否 clone storage”是相邻但不同
  的决策，不能由 protocol 在 target 创建时从 name 字符串重建。

对应 WPT 给出 hyperlink 特有的可观察合同：

- `windows/auxiliary-browsing-contexts/named-lookup-noopener.html` 连续点击两个相同普通 target name 的
  `rel=noopener` anchor，要求得到两个不同 Window，同时两个真实 realm 的 `window.name` 都保留请求值；
- `windows/noreferrer-window-name.html` 要求两个新建同名 `rel=noreferrer` link 不互相复用，但预先存在的
  named iframe 和 named auxiliary window 仍分别可被同一个 noreferrer link 命中；existing window 的
  opener 状态不能被本次 suppressed relation 重写；
- 这些 case 同时说明 noopener policy 不能被实现成“先新建，再让 browser-context-wide name map 决定是否
  合并”。lookup 必须先在 source 可见的 browsing-context namespace 内完成，只有 miss 才执行 group split。

本轮没有编译 Chromium，也没有运行 upstream WPT，因此上述内容仍是源码/WPT 合同对照。Moli 当前
先覆盖 top-level 或 related auxiliary source 中具有完整 creator capability 的普通 named、可解析、非
`javascript:` hyperlink。现有 `navigate_hyperlink_target_browsing_context()` 仍保证当前 Page 的 named
iframe 先于 related top-level lookup；但 child-frame source 的完整 subtree/Page/related-Pages ordering、
related peer nested frame、`CanNavigate`、focus/detach transaction 尚未达到 Chromium 的完整算法。

##### 稳定红灯与违反路径

renderer 回归先让 production Page 点击一个 `target=related-hyperlink-name rel=opener` 的 link。旧实现虽
预留 auxiliary Page，却没有声明 renderer 已完成 group decision，稳定失败为：

```text
new_target_disposition: left None, right Some(Related)
```

同一回归随后用 `rel=noreferrer` 再导航相同 name，并要求 activation 携带第一次 reservation 的 exact
`{owner_local_host_id, page_id}`；最后连续点击两个相同 `isolated-hyperlink-name` 的 noopener/noreferrer
link，要求得到两个不同 Fresh reservation。

protocol 回归把违反路径拆成两个独立观察：

```text
related target realm: left "|false|#related-two"
                      right "relatedLinkName|true|#related-two"

same-name suppress-opener links: left 1 Target.targetCreated
                                 right 2 Target.targetCreated
```

第一条说明 opener host 的 named lightweight realm 与 target 采纳的 Page 仍是两份 identity；第二条说明
第二次 Fresh link 被 `BrowserContext::target_window_names` 合并进第一个 target。回归还主动清空 protocol
name projection，再执行 existing related target 的 `rel=noreferrer` 导航；只有 renderer group lookup
仍能精确复用原 Page，并保持它已有的 name/opener edge。

##### Moli cutover

E2C 将 hyperlink 路径改为下面的 owner 顺序：

```text
activate hyperlink(url, ordinaryName, rel)
  -> resolve source Document + named iframe lookup
       hit: navigate exact child
  -> freeze {opener exposure, three referrers, creator policy}
  -> related top-level Page lookup (independent of rel=noopener/noreferrer)
       hit: emit activation { exact renderer Page residence, no Page.windowOpen }
  -> no hit + opener preserved
       stage one real initial Page/realm with ordinaryName and opener
       emit activation { Related, popup_id, exact reservation }
  -> no hit + opener suppressed
       reserve one Fresh Page without opener-local lightweight owner
       emit activation { FreshNamed, popup_id: None, ordinaryName, exact reservation }
  -> protocol adopts/navigates the renderer-selected Page without name lookup
```

具体改动与边界如下：

- 原来只服务 `window.open()` 的 helper 更名为
  `related_page_named_target_for_navigation()`。`window.open()` 仍可传入 replacement opener；hyperlink
  lookup 始终传 `None`，所以 `rel=noopener/noreferrer` 命中 existing target 时只影响本次 source/referrer
  policy，不覆盖 target 已有的 Page-scoped opener edge；
- ordinary named hyperlink 在新建前调用同一 `RendererRelatedPageGroup` lookup。hit activation 只携带
  `RendererResolvedPopupTarget` 和 creator-frozen referrers，不预留 Page、不创建 lightweight record、也不
  产生 `Page.windowOpen`；protocol 通过 exact renderer residence 路由 navigation；
- opener-preserving miss 让 `open_lightweight_popup_window()` 启用 named real-Page staging。这里保留
  `popup_id` 只是同步 Window/initial auxiliary state 的 typed owner identity；staged Page、真实 realm、
  `window.name`、opener、session-storage clone 与 target admission 均复用 E2A 路径，并显式标记
  `RendererPopupNewTargetDisposition::Related`；
- suppress-opener miss 对 `_blank` 或 ordinary name 的非 `javascript:` URL 直接 reserve Fresh Page。
  `_blank` 现在显式标记 `FreshUnnamed`，ordinary name 标记 `FreshNamed`；两者都不创建 opener-local
  lightweight owner，Fresh target 也不写入 browser-context-wide name projection；
- `rel=opener target=_blank` 的新建 Related Page 同样得到显式 `Related` disposition。这让 E1 早期接入的
  `_blank` 两种 group admission 不再依赖 protocol 从 `exposes_opener` 推断；
- staged Related Page 的 session-storage store 与 initial storage key 直接取自 creation result，再以旧
  lightweight record lookup 作为 legacy fallback。真实 Page staging 不需要留下一份 mirrored record，
  因而不能在创建后反查那份本不应存在的状态；
- E2C 入库时，source 缺少完整 creator capability、URL 无法解析或为 `javascript:` 时仍走 legacy carrier。
  Chromium 的实际合同是同步选择/创建 selected target，但把脚本作为该 target Document 的 networking task
  异步执行，并使用 target realm/CSP/currentness；这不能靠 protocol async Page admission 机械迁移。full-creator
  `javascript:` 缺口现已由 E2N 完成，缺少 creator capability 的 compatibility fallback 仍保留；
- 旧 `owner_scheduler_applies_legacy_hyperlink_popup_terminal_from_stable_page_route` 被删除。它等待
  opener-local loader 请求并从 mirrored Document 回写 opener，恰好要求已迁移路径保留第二 owner；新的
  renderer activation 和 protocol target-realm 回归覆盖正确责任边界。

这一纵切建立的窄不变量是：

1. full-creator ordinary named hyperlink 与 `window.open()` 使用同一 renderer Page-group name authority；
   protocol name projection 不能创建、合并或重定向 migrated target；
2. existing named iframe 仍先于 related top-level Page；existing related Page 精确复用且不产生第二个
   target，`rel=noreferrer` 不覆盖它已有的 opener edge；
3. opener-preserving miss 只创建一个 Related Page/realm/Document/loader；suppress-opener miss 每次创建
   不同 Fresh Page，同时每个真实 realm 都保留 ordinary `window.name` 且 opener 为 null；
4. `Related` / `FreshNamed` / `FreshUnnamed` 与 exact Page reservation 在 renderer creation point 一次冻结；
   referrer、session-storage 和 realm bootstrap 不由 protocol name map 反推；
5. reuse 不产生 `Page.windowOpen`，new target 只产生一次；target attach 观察的是被 renderer 选中的真实
   realm，而不是 opener-local mirror。

#### Phase 5E2D：form named / `_blank` 的 target + request 一体化迁移

E2D 迁移 HTML form submission 的 full-creator auxiliary target。这里不能把 E2C 的 hyperlink
helper 直接当作“打开 URL”：form target 选择完成时，HTTP method、encoded body、Content-Type、
submitter override、form data、referrer policy 和目标 Frame/Page 已经属于同一次 submission。若
renderer 只把 URL 交给 protocol，POST 会静默变成 GET；若只保留 request 而让 protocol 再查 name，
同名 Fresh/Related group 又会被 browser-context projection 错误合并。

##### Chromium / WPT 合同与旧实现违反路径

本轮继续使用本地 Chromium `a03603fe9af6`，直接核对以下 owner：

- `third_party/blink/renderer/core/loader/form_submission.cc::FormSubmission::Create()` 先复制 form
  attributes，再按“submitter attribute 是否存在”覆盖 `formaction` / `formenctype` / `formmethod` /
  `formtarget`。它随后构造一份 `ResourceRequest`；POST 在同一对象上设置 method、encoded body 和
  Content-Type；
- effective target 不是简单的 `form.target`：copied target 为空时使用 `Document::BaseTarget()`，再由
  `FrameLoadRequest::CleanNavigationTarget()` 清理。因而 `formtarget=""` 会覆盖 form 自己的非空
  target，并继续落到 `<base target>`，不能错误回退到 form target；
- form 的 `noreferrer` 同时设置 no-referrer/noopener，`noopener` 只抑制 opener，`_blank` 在没有
  `rel=opener` 时隐式 noopener。之后同一 `FrameLoadRequest` 和 effective target 进入
  `FrameTree::FindOrCreateFrameForNavigation()`；
- target lookup 返回的 `target_frame` 与完整 `resource_request_` 一起存入 `FormSubmission`。
  `HTMLFormElement::ScheduleFormSubmission()` 使用 target frame 的 scheduler，处理 target-local
  Navigation API / client-navigation cancellation；最终 `FormSubmission::Navigate()` 仍从保存的同一
  request 导航保存的同一 frame；
- WPT `form-submission-target/rel-{form,input,button,base}-target.html` 覆盖显式 form target、submitter
  `formtarget`、`<base target=_blank>` 与动态 rel；`resources/reltester.js` 要求 noopener 保留 referrer、
  noreferrer 清空 referrer，默认 `_blank` 不暴露 opener；
- `form-target-request-header.html` 明确向 `_blank` POST，并由服务端要求 Content-Type；
  `form-submission-0/submit-entity-body.html` 又覆盖 urlencoded、multipart、text/plain 的 exact entity
  body。E2D 没有运行 upstream WPT，因此这些仍是源码/WPT 合同对照，不是通过率声明。

Moli 旧路径在 form owner 内发生了两个稳定分叉：

```text
ordinary name
  -> only try named iframe
  -> miss: return false, no auxiliary Page/target

POST + _blank / other non-current target
  -> submit_post_form_to_top_level_browsing_context()
  -> navigate opener Page, ignore selected auxiliary target
```

接入前 renderer production 回归提交一个 `target=related-form-name rel=opener` 的 GET form，失败为
`popup activations: left 0, right 1`。两条 protocol HTTP 回归分别要求 ordinary named target creation
和 `<base target=_blank>` POST creation，均稳定失败为 `Target.targetCreated` 消息为空。后者同时说明旧
POST carrier 没有到达新的 target；它不是单纯少发了一个 CDP event。

##### effective target 与 existing-frame 优先级

form owner 现在先一次性计算 effective target：

```text
submitter has formtarget attribute ? exact formtarget value
                                 : form target attribute
  -> selected value empty/missing ? source Document first live base target
                                  : selected value
  -> still empty/missing => current browsing context
```

target lookup 顺序保持窄且与 E2C 一致：

1. ordinary name 先查现有 named iframe；命中后继续使用原有 deferred child request、FormData
   `NavigateEvent`、per-form pending-child cancellation 和 exact child handle；
2. named iframe miss 后才进入 shared element auxiliary selector；
3. ordinary name 在 renderer related Page group 中查 exact live top-level Page，不依赖 protocol
   `target_window_names`；
4. related hit 携带 `RendererResolvedPopupTarget`，不创建 Page、不产生 `Page.windowOpen`；
5. miss 且保留 opener 时 staging 一个真实 Related initial Page；miss 且抑制 opener 时 reserve
   `FreshNamed` 或 `_blank` 的 `FreshUnnamed` Page；
6. E2D 入库时，source 没有 full creator capability、`javascript:` 或其它尚未迁移条件继续 fail closed 到
   legacy carrier，不能借本纵切声称 child-source 完整 ordering 已经完成；后续 E2N 已迁移 full-creator
   `javascript:` producer，但没有扩大“缺少 creator capability”的 fallback。

`rel=noopener/noreferrer` 仍不改变 existing-target lookup。命中已有 related Page 时，本次 source 的
opener exposure/referrer policy 只进入 navigation activation；目标 Page 既有的 Page-scoped opener edge
不被改写。新建时 relation 才决定 Related/Fresh admission、initial opener、session-storage clone 和首
realm name。

##### 一个 typed request 穿过 renderer / protocol authority

原来 popup activation 只有 `url: String`，protocol target navigation 固定调用 GET helper。E2D 把已有
top-level location request 抽成公共 `RendererTopLevelNavigationRequest`：

```text
RendererTopLevelNavigationRequest
  { url, method, raw body bytes, explicit headers, navigation kind }

form target selection
  -> RendererPendingPopupActivation { boxed exact request, referrers, Page decision }
  -> PagePreparedPopupActivation
  -> PopupTargetCreation
  -> PopupTargetNavigationClaimIdentity
       { exact TargetPageResidenceIdentity, boxed exact request, referrers, kind }
  -> Held -> Published -> Consumed
  -> request-aware stable Page navigation
```

GET/window.open/hyperlink producers 仍通过 `RendererTopLevelNavigationRequest::get()` 产生相同默认行为；
form POST 改用 `new()` 保存 raw encoded bytes 和 `Content-Type`。activation 的 URL accessor、
`Page.windowOpen`、target URL projection 与最终 request 全部读取同一个 carrier，并用 invariant 拒绝
target-selection URL 与 request URL 分裂。

protocol existing-target reuse 与 new-target admission 都把 request 整体交给
`PopupTargetNavigationOwnerAction::capture()`。claim 发布/消费仍先校验 exact browser context、target、
Page residence generation 和 initial/named-reuse kind；验证通过后才调用 request-aware renderer navigation
entry。由此 wait-for-debugger held action、background target、named reuse 和 stale Page rejection 不会只
保留 URL 而丢失 POST 元数据。Network/Fetch 观察到的 method、postData、Content-Type 与服务端实体来自
同一个 target Page loader。

##### form-specific target event 与 referrer

shared element selector 只共用 target/group/referrer/creation primitive，没有删除 form owner：

- POST serialization 与 `formdata` event 仍在 form submission owner 完成；
- named iframe hit 继续走原 form-specific child path；
- related top-level hit 会在精确 target Window/realm 同步派发 cross-document `NavigateEvent`，POST
  `event.formData` 保留 source entries，source element 和 user-initiated fact 一并传入；目标
  `preventDefault()` 后 submission 返回 accepted/canceled，不生成 popup activation，也不启动网络请求；
- creator policy 仍一次冻结 initial empty Document referrer、HTTP Referer 和 destination
  `document.referrer`。noopener 只切 opener/group，noreferrer 同时把三者置空；
- new `_blank` 默认使用 FreshUnnamed，但保留网络/document referrer；`rel=opener` 则使用 Related。

这个 event 接入只覆盖已经能由 renderer group 精确解析的 related top-level hit。它不等于完整实现
Blink `Frame::ScheduleFormSubmission()`：同一 form 的跨 task supersession、target loader
`CancelClientNavigation()`、parser cancellation、RemoteFrame scheduler 与 sandbox `allow-forms` 当时仍需在
通用 form/navigation owner 后续补齐，不能靠删除旧 activation 或 drain queue 伪装正确；其中 local/related
child cancellation 已由 E2G 完成，source-Document `allow-forms` 双门禁已由 E2L 完成。

##### 本纵切建立的不变量与证据边界

E2D 建立以下窄不变量：

1. form effective target、relation、exact request 和 renderer Page decision 在一个同步 owner 中冻结；
2. existing named iframe 优先且旧 FormData/cancellation 行为不回归；ordinary related Page reuse 不依赖
   protocol name projection；
3. named/`_blank` GET 与 POST 使用同一 Related/Fresh target algorithm，POST 不再导航 opener 或退化为
   GET；
4. `RendererTopLevelNavigationRequest` 从 activation 到 exact target-local claim 不拆 URL/method/body/header；
5. existing related target 可在自己的 realm 观察/cancel form navigation；new target 只产生一个 Page、
   Document、loader 和服务端副作用；
6. `<base target>` 与空 submitter override 使用 source Document 的 live base-target authority。

接入前/后的聚焦证据：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(per_page_isolate_policy_keeps_window_open_routes_page_owned)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 接入前：1 failed，named form popup activation left 0/right 1。
# 接入后：1 passed；覆盖 Related creation、exact named POST reuse、target NavigateEvent/FormData cancellation、
# 两个 same-name Fresh form、base target=_blank、空 submitter formtarget override 和 exact request fields。

cargo nextest run -p moli-protocol \
  -E 'test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request) | test(base_target_blank_form_post_creates_fresh_target_with_exact_request)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 接入前：2 failed，两个 case 的 Target.targetCreated 都为空。
# 接入后：2 passed；HTTP server 与 Network.requestWillBeSent 同时验证 POST、raw body、Content-Type、
# Referer/noreferrer、Related reuse、Fresh _blank、opener 保留/抑制、window.name 与唯一 response Document。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E 'test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(form_target_blank_reloads_rel_opener_policy_for_each_submission) | test(canceled_post_form_navigation_aborts_signal_without_synthetic_timer) | test(detached_child_form_submit_targets_named_iframe_without_shadow_controls) | test(formdata_event_appended_entries_are_submitted_to_named_iframe) | test(distinct_forms_keep_distinct_pending_child_target_submissions) | test(programmatic_form_submit_keeps_successive_distinct_child_targets) | test(form_top_and_parent_targets_queue_plain_top_level_navigation) | test(renderer_top_level_form_post_preserves_request_through_document_commit) | test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request) | test(base_target_blank_form_post_creates_fresh_target_with_exact_request) | test(named_opener_hyperlink_creation_and_reuse_are_owned_by_renderer_group) | test(named_suppress_opener_hyperlinks_create_distinct_fresh_groups_with_live_names) | test(noopener_and_noreferrer_popups_have_one_real_navigation_with_creator_referrer_policy)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 14 passed；覆盖 form current/child/auxiliary 三条 request owner 与相邻 hyperlink/referrer/group 路径。
```

#### Phase 5E2E：child-source 与 related nested frame-tree named resolver

E2A-E2D 的 renderer group authority 仍有一个结构性缺口：name-indexed registry 只描述 related
top-level Page，当前 Page 的 child lookup 又从整棵树根开始扫描。只要 source 本身是 child，或相同 name
出现在 related Page 的 nested frame 中，选择顺序和导航 owner 就会分裂。E2E 不再给这些调用方各加一次
fallback，而是把查找提升为 renderer-owned、source-relative 的 frame-tree resolver。

##### Chromium / WPT 合同

本轮继续直接对照本地 Chromium `a03603fe9af6`：

- `core/page/frame_tree.cc::FindFrameForNavigationInternal()` 先从 source frame 本身开始 preorder 遍历其
  subtree；再从当前 Page main frame 开始遍历整棵树，但排除刚才已经检查的 source descendants；最后按
  `Page::RelatedPages()` 顺序遍历每个 non-closing related Page 的 main frame 与完整 descendants。每个
  name match 都在原位置调用 `current_frame->CanNavigate(*frame, url)`，不能先选中再在调用方补权限；
- `core/frame/local_frame.cc::CanNavigate()` 的普通 nested-frame 路径允许 source 与 target 本身或 target
  任一 ancestor 同源；`javascript:` URL 更严格，必须与 target 本身同源。该函数还包含 sandbox navigation
  flags、top-level opener/user-activation、top navigation、fenced frame 等分支，E2E 没有把这些未建模的
  policy 伪装成已完成；
- WPT `browsing-context-names/duplicate-name-order.html` 构造同名 source descendant、current Page sibling
  和多个 popup，依次要求 `ChildA`、`SiblingB`、`PopupC`；
- WPT `windows/targeting-cross-origin-nested-browsing-contexts.html` 从 opener 尝试导航一个 cross-origin
  related Page 内的 nested name。因为 source 不能访问 target 及其任何 ancestor，旧 nested candidate
  必须被跳过，最终打开同名 top-level context；回传的 `isTop` 必须为 `true`。

本轮没有编译 Chromium，也没有运行 upstream WPT；以上仍是源码与 WPT 合同对照。Moli 的本地
回归复现相同的树顺序和普通 origin/ancestor 决策，不把它表述为完整 `LocalFrame::CanNavigate()`。

##### 稳定红灯与违反路径

首条 renderer 回归从一个 nested requester 执行五次 `window.open()`。旧实现稳定表现为：

```text
sourceSubtree  -> earlier-current-sibling      # 错过 requester descendant
currentTop     -> child-colliding-with-current-top
currentRemainder -> current-page-remainder     # 唯一正确项
relatedNested  -> null
relatedPageOrder -> null
```

nextest run `13005909-fbfa-4adc-a01c-30b8ccd0c0c8` 因此失败。违反路径有三条：

1. current Page lookup 从 main Document 的全局 child registry 起点扫描，不知道 source subtree；
2. `RendererRelatedPageGroup::named_targets` 只能返回同名 top-level Page，无法遍历该 Page 的 child registry；
3. hyperlink 命中不了 related nested child 后落入 auxiliary creation，navigation 不在目标 child 所属 Page
   执行。

调试过程中还暴露一个独立但同属访问 authority 的错误：production staged Page 的三个 nested handle
已经存在，name 也匹配，但普通 `CanNavigate` 仍返回 false。初始 `about:blank` 的 V8 security token、storage
origin 和 Window runtime state 都已继承 creator，Rust access check 却重新从 `document_url=about:blank`
构造一个 host-local opaque origin。修正后才可能让 frame-tree resolver 与 stable WindowProxy 的既有
same-origin 事实一致。

##### renderer owner cutover

E2E 建立以下 owner 链：

```text
ordinary named navigation(source Window/element, destination URL)
  -> resolve exact source WindowExecutionContextIdentity + child/top dispatch scope
  -> source subtree preorder
  -> current Page top + remaining preorder
  -> each live related Page in group order
       top WindowProxy
       current target Context -> target JsContextHost -> complete child preorder
  -> candidate-local CanNavigate filter
  -> typed result {current top | current child | related top | related child}
  -> navigate through the selected context's owner
```

具体实现边界如下：

- `RendererRelatedPageGroup` 在原有 top-level name index 之外保存 weak、按 Page admission 顺序排列的
  top-level target。weak entry 只有在 Page lifecycle 为 `Active`、stable main WindowProxy 存在且 current
  default Context 已绑定时才参与查找；close/discard 与尚未完成 bootstrap 的 Page 不会成为候选；
- 每个 top-level target 保存当前 `v8::Context`，main default realm 在 native bridge host slot 安装后绑定，
  navigation replacement 会覆盖为新 Context。stable WindowProxy 的 creation context 可能仍是 opener-side
  facade，不能作为 target `JsContextHost` 地址；current Context slot 才是当前 Document owner 的权威定位；
- source child 由 `window.open()` receiver 的 stable child marker 或 hyperlink source node 的 owner Document
  解析，而不是假设 callback 的 entered scope 必然是 top。当前 Page child handles 继续使用 live DOM/document
  order，subtree membership 由 child-parent registry 逐级判断；
- related Page 先从 current Context slot 解析 target host，再同步该 host 的 live child subtree。resolver
  返回的 related-child raw host pointer 只在当前 V8 callback 内存在；Page group 持久状态保存的是 V8
  `Global<Context>` 与 typed Page residence，不保存裸指针；
- 普通 nested candidate 只有在 source 可访问 target 或其任一 ancestor 时才命中；`javascript:` candidate
  额外要求 source 可访问 target 本身。通过筛选后，child WindowProxy 仍按 source observer realm 决定
  same-origin wrapper 或 restricted proxy，不把 target raw global 泄漏给 caller；
- related child navigation 调用 target host 的 child navigation owner；same-document / same-origin target
  event 和 cross-document child request 都由该 Page 自己产生。related top-level 结果仍携带 exact
  `RendererResolvedPopupTarget` 进入 E2A-E2D 的 Page activation/claim 路径；
- initial Document 的 effective serialized origin 现在与 loader fetch context、Window runtime state 和 V8
  token 一起传入 `JsContextHost` / main `FrameOwnerStore`。普通 URL 仍由 response URL 产生 origin；tuple
  origin 的 inherited initial Document 不再错误地从 `about:blank` URL 重建 opaque 身份；
- E2E 当时接入的 producer 是 ordinary-name `window.open()` 与 hyperlink；form E2D 的
  request/scheduler owner 当时尚未切到这份 typed result，避免只替换 name lookup。后续 E2F 已完成 local
  target owner cutover，跨 Page cancellable scheduler identity 仍按下节边界保留。

##### 本纵切建立的不变量与证据

E2E 建立以下窄不变量：

1. named lookup 顺序由 source-relative frame tree 与 related Page order决定，不由 name-indexed top-level map
   或 protocol target projection 决定；
2. current top、current child、related top 与 related child 都保留精确 owner；命中 related nested target
   不创建第四个 auxiliary Page，hyperlink 在 target Page 内导航原 child；
3. candidate name match 不代表可用；普通 nested target 必须通过 target/ancestor origin check，失败后继续
   搜索或创建新 context，且不得修改被拒绝 candidate；
4. tuple-origin initial inherited `about:blank` 的 Rust access origin 与已继承的 V8/security/storage
   authority 一致；
5. Page group 不持久化 `JsContextHost` raw pointer，replacement 后 lookup 只从 current Context 重新解析 host。

接入过程中与最终聚焦证据：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_named_frame_lookup_follows_chromium_frame_tree_order)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 13005909-fbfa-4adc-a01c-30b8ccd0c0c8：接入前 1 failed，稳定暴露上述四个错误选择。
# run 46408fd0-0683-4ab8-b5db-c9637796afd1：origin/owner 修正后 1 passed。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_named_frame_lookup_follows_chromium_frame_tree_order) | \
      test(named_frame_lookup_skips_candidate_the_source_cannot_navigate)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run c3857b05-7b03-4184-91a6-1598b1f51407：2 passed。
# 第二条由 data: opaque child 发起 lookup；同名 current-Page candidate 及 top ancestor 均不可访问，
# 因而必须创建新 auxiliary Page，并验证旧 candidate marker/URL 未变化。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E 'test(related_page_named_frame_lookup_follows_chromium_frame_tree_order) | test(named_frame_lookup_skips_candidate_the_source_cannot_navigate) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(window_open_noopener_navigates_existing_named_iframe_and_returns_null) | test(hyperlink_target_blank_reloads_rel_opener_policy_for_each_activation) | test(hyperlink_javascript_url_csp_checks_the_source_document_before_target_selection) | test(named_opener_hyperlink_creation_and_reuse_are_owned_by_renderer_group) | test(named_suppress_opener_hyperlinks_create_distinct_fresh_groups_with_live_names) | test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request) | test(base_target_blank_form_post_creates_fresh_target_with_exact_request)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 067d4968-3fc3-4692-b512-5c38faa76cf5：10 passed；覆盖 E2E 与 E2A-E2D/JavaScript-CSP 邻接面。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_named_frame_lookup_follows_chromium_frame_tree_order) | \
      test(named_frame_lookup_skips_candidate_the_source_cannot_navigate)' \
  --stress-count 50 --flaky-result fail --test-threads 8 --no-fail-fast
# run fe5b9c3c-c503-4c60-8b55-b6586dddae3a：50/50 iterations passed，每轮 2/2。
```

#### Phase 5E2F：form exact request 消费 typed frame-tree resolver

E2D 已把 form 的 effective target、method/body/Content-Type/referrer 和 target-realm
`NavigateEvent` 冻结到同一 submission carrier，E2E 又建立 source-relative typed resolver；但两者仍未
接通。旧 ordinary-name form 路径只先查 current Page named iframe，miss 后直接进入 auxiliary selector。
因此 related Page nested frame 即使已经由 E2E 找到，也没有机会接收 POST request，更无法由目标 Page 的
child scheduler 执行。

##### Chromium 合同与本轮责任边界

本轮继续对照本地 Chromium `a03603fe9af6`：

- `core/html/forms/form_submission.cc::FormSubmission::Create()` 把 target、method、encoded body、headers、
  referrer policy 与 source form state 冻结进一次 `FormSubmission`；`Navigate()` 把同一 request 交给
  已选择的 target Frame，不在 target owner 内重新从 DOM 拼装；
- `core/html/forms/html_form_element.cc::HTMLFormElement::ScheduleFormSubmission()` 先通过
  `FindFrameForNavigation()` 取得精确 Frame，再让 target LocalFrame 的 scheduler 安排 navigation；同时
  target loader client navigation 会被取消，form 保存的上一份 cancellable closure 会在新 submission
  到来时作废；
- `core/frame/frame.cc::Frame::ScheduleFormSubmission()` 与
  `core/page/frame_tree.cc::FindOrCreateFrameForNavigation()` 共同说明 lookup result、request 与 scheduler
  owner 不能拆成三次晚绑定；RemoteFrame 可以有不同 scheduler endpoint，但仍消费同一个 selection；
- 这份合同不等价于“目标 entry 覆盖旧 pending request”。同一 form 从 target A 改投 target B 时，source
  form 持有的 cancellation identity 仍应取消 A；这是 E2F 明确保留到下一纵切的边界。

本轮没有编译 Chromium，也没有运行 upstream WPT。源码对照证明 owner 关系；本地回归只覆盖 local
Frame、同源 direct response 与当前已有的 Page scheduler，不把它外推成 RemoteFrame 或完整 sandbox
form submission。

##### 违反路径、测试校正与证据强度

接入前的实际路径是：

```text
ordinary named form
  -> current-Page named iframe lookup
  -> miss
  -> E2D auxiliary related-top selector
  -> related nested frame 不可表示
  -> 创建新的 auxiliary top-level target
```

最初诊断 run `d67dd295-4bef-4427-af76-4308c9764d11` 的 provisional 回归确实观察到新 popup，但随后确认
测试使用的 `create_related_test_html_page_for_script_agent_experiment()` 只共享 script agent/isolate，
没有加入 production browsing-context group；该 run 因 setup 不成立而废弃，不能当作浏览器语义红灯。
最终回归改为真实 `window.open()` 同步创建 initial realm，消费 activation 中的 exact Page reservation，
再 adopt staged `about:blank` Page 并在其中建立 named child。没有为这份校正后的 setup 保留可信的
pre-cutover run；接入前证据因此是上述可审计源码路径而不是红测 run id，强度低于 E2E 的稳定红灯。

第一轮完整邻接集 run `a4cb34d9-bf0a-44d7-a28e-282cacc85931` 为 16 passed / 2 failed，暴露了两个真实
回归，而不是用 drain/retry 掩盖：

1. typed GET request 让 standalone upstream fixture 从旧 URL fast path 变成无条件 async，nested target
   已命中、event 已允许且 request 已入队，但 fixture 不再同步 materialize；
2. HTTP `Referer` 已正确来自 source Page，child entry 应用 loaded policy 时却漏拷贝
   `document_referrer`，最终 response Document 仍观察到旧 initial `about:blank` referrer。

修复分别落在 request-aware fixture materialization 与 child policy commit owner，而不是 form caller 或
测试等待循环。

##### typed owner cutover

E2F 现在使用以下单向 owner 链：

```text
form submission owner
  -> freeze RendererTopLevelNavigationRequest {URL, method, body, headers, kind}
  -> E2E source-relative resolver
       CurrentTopLevel   -> exact current Context + Page pending-location owner
       CurrentPageChild  -> exact child handle + current Page child scheduler
       RelatedTopLevel   -> current target Context + exact RendererResolvedPopupTarget
       RelatedPageChild  -> callback-scoped target JsContextHost + exact child handle
       miss              -> E2D Related/Fresh auxiliary creation
  -> dispatch NavigateEvent/FormData in selected target realm
  -> queue the same immutable request through that target's owner
```

关键实现不变量如下：

- ordinary named GET/POST 不再各自做 name lookup。`FormSubmissionMethod` 先转成 E2D 已有的
  `RendererTopLevelNavigationRequest`，method、raw encoded body、Content-Type 与 browser navigation kind
  在 resolver 前冻结；
- typed top-level result 除 stable WindowProxy 外显式携带 current target `v8::Context`。stable proxy 的
  creation context 可能仍是 opener-side facade，target-realm `NavigateEvent` 和 target host 定位只能使用
  current Context slot；
- child-source form 的 creator 直接复用现有 child stable WindowProxy、base URL 与 policy container。
  related-top 命中不会因为旧 helper 只识别 root/lightweight Document 而退回无 reservation 的 popup action；
- current/related child 都消费 `ChildBrowsingContextNavigationRequest`。该 carrier 额外保存 source initiator、
  policy-filtered `Referer` 和目标 Document 应观察的 referrer；target loader 禁止再次按 target parent 推导
  referrer，避免 cross-Page handoff 后改写 source；
- inherited `about:blank` / `about:srcdoc` source 不能把字面 `about:` URL 当 tuple-origin authority。source
  carrier 从 child policy container 读取 creator URL，与现有 stable WindowProxy/security token 的继承事实
  对齐；
- child network response、local URL snapshot 与 request-aware GET fixture 都把 referrer 写回同一
  `DocumentPolicyContainer`。因此 request header、entry snapshot 和最终 `document.referrer` 不再由三份状态
  分别决定；POST 或非 fixture request 仍走真实 async loader；
- current-Page child 保留既有 per-form cancellation：submitter activation 取消该 form 的全部旧 child
  target，programmatic submit 取消同一 target 的旧 request，成功 queue 后才登记 pending target；
- related-child raw host pointer 仍只在一次 V8 callback 内使用。跨 Page cancellation 没有把它持久化到 form
  state；后续必须保存 typed Page/Frame scheduler identity，而不是泄漏 host pointer。

##### 本纵切建立的回归与验证

production-style renderer 回归锁住：

- real related Page 中的 nested named frame 消费 form POST，不产生第二个 popup activation；
- target child realm 观察 exact destination、`FormData`、FORM source element 与 `userInitiated=false`；
- target Page 自己完成 child lifecycle/Document commit，server 同时观察 POST path、urlencoded
  Content-Type、source-derived `Referer` 与 raw body；response child `document.referrer` 与 source URL 一致；
- E2E 原 frame-tree 回归又从 nested requester 提交两个可取消 POST，分别命中 related top 与 related child
  的精确 realm，证明 child-source 也复用了同一 resolver/WindowProxy 基础；
- 既有 current-child formdata、同 form/不同 form supersession、detached child fixture、Related/Fresh
  top-level protocol handoff 与 hyperlink/referrer 邻接行为保持不变。

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(detached_child_form_submit_targets_named_iframe_without_shadow_controls) | \
      test(related_page_named_form_post_uses_nested_target_owner_and_exact_request)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 4980908e-6d47-43ae-b615-4543169a5164：2 passed；覆盖 request-aware fixture 与
# related-child HTTP/Referer/document.referrer commit。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E 'test(related_page_named_form_post_uses_nested_target_owner_and_exact_request) | test(related_page_named_frame_lookup_follows_chromium_frame_tree_order) | test(named_frame_lookup_skips_candidate_the_source_cannot_navigate) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(form_target_blank_reloads_rel_opener_policy_for_each_submission) | test(canceled_post_form_navigation_aborts_signal_without_synthetic_timer) | test(detached_child_form_submit_targets_named_iframe_without_shadow_controls) | test(formdata_event_appended_entries_are_submitted_to_named_iframe) | test(submit_button_click_supersedes_programmatic_submit_after_target_change) | test(distinct_forms_keep_distinct_pending_child_target_submissions) | test(programmatic_form_submit_keeps_successive_distinct_child_targets) | test(form_top_and_parent_targets_queue_plain_top_level_navigation) | test(renderer_top_level_form_post_preserves_request_through_document_commit) | test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request) | test(base_target_blank_form_post_creates_fresh_target_with_exact_request) | test(named_opener_hyperlink_creation_and_reuse_are_owned_by_renderer_group) | test(named_suppress_opener_hyperlinks_create_distinct_fresh_groups_with_live_names) | test(noopener_and_noreferrer_popups_have_one_real_navigation_with_creator_referrer_policy)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 90b62837-12de-4bbd-95b3-136a83d35c32：18 passed。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_named_frame_lookup_follows_chromium_frame_tree_order) | \
      test(related_page_named_form_post_uses_nested_target_owner_and_exact_request)' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail
# run 3e035af5-4873-418b-9a2c-5fb201f70907：20/20 iterations passed，每轮 2/2。
```

E2F 有意没有声称完成 Chromium 的整个 scheduling contract：跨 Page same-form cancellation、target loader
`CancelClientNavigation()`、parser cancellation、RemoteFrame scheduler、sandbox `allow-forms` 与 top-level
`CanNavigate()` 当时仍未实现；前两项 local/related child owner 已由 E2G 收敛，`allow-forms` source gate 已由
E2L 收敛。child-source 命中 current top 虽已保留 method/body/headers 和 exact target owner，
但当时 top-level protocol carrier 仍只记录 root Document lifecycle，尚未携带 child source/referrer identity（后由
E2H 解决）；这与
redirect、cross-origin/downgrade referrer 再计算一起需要单独纵切，不能由本轮 related-child direct-response
回归外推。

#### Phase 5E2G：跨 Page same-form cancellable scheduler identity

E2F 已经把 request 交给 exact target child owner，但 cancellation state 仍是 source-host-local 的
`HashMap<form DomHandle, Vec<target DomHandle>>`。related child queue 成功后没有登记，因为 callback-scoped
`target_host_ptr` 不能持久化；同一 form 随后由 submitter 从 related target A 改投 B 时，A 的 child task、
main-resource loader 与 parser ledger 会继续存活。与此同时，旧 local map 只比较 target handle：如果 A 的
form navigation 已被普通 `location` navigation 替换，稍后的 submitter 仍会把这份较新的无关 navigation
一起清掉。

##### Chromium 合同、本轮边界与红测

本轮继续以本地 Chromium `a03603fe9af6` 为基线：

- `HTMLFormElement::PrepareForSubmission()` 在 user/submission path 调用保存的
  `cancel_last_submission_`；`submitFromJavaScript()` 则直接进入 `ScheduleFormSubmission()`；
- `HTMLFormElement::ScheduleFormSubmission()` 取得 exact target Frame 后，使用 target LocalFrame scheduler，
  并在提交前执行 target `CancelPendingJavaScriptUrls()` 与 loader `CancelClientNavigation()`；
- `Frame::ScheduleFormSubmission()` 保存 `form_submit_navigation_task_version_`，返回的 cancellation closure
  只有 version 仍匹配时才取消 target Frame 的 task。也就是说 cancellation identity 必须同时限定 target
  Frame 与 scheduler generation，不能只保存一个可复用的 DOM owner handle；
- RemoteFrame 会退回 source scheduler，因此本轮 local child binding 不能外推为完整 remote endpoint 设计。

实现前的 focused run 同时固定两条失败：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(submitter_cancels_previous_same_form_navigation_in_a_related_page_child) | \
      test(submit_button_does_not_cancel_a_newer_non_form_navigation_in_the_previous_target)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 3a65beae-5ac1-4f24-98a8-b675ad5c768e：0 passed、2 failed。
# local case 中 replacement URL pending state 被旧 form 记录清空；related case 中 A/B 都未受 source-side
# exact cancellation 约束。
```

这两条红测只证明本地违反路径，不声称等价覆盖 Chromium 的所有 form task timing。最终 related 回归又被
加强为：先让 A 的 GET 真正进入 libcurl transport 并阻塞 response，再从 source Page 同步 click retarget 到
B；server 必须观察 A connection 被 cancel、B 是唯一完成 commit 的 response。

##### typed route 与 exact target-owner cancellation

E2G 的 owner 链现在是：

```text
source HTMLFormElement state
  -> PendingFormSubmissionChildNavigation {
       target:
         CurrentPage { BrowsingContextId }
         | RelatedPage {
             RendererResolvedPopupTarget,
             target root RendererDocumentLifecycleIdentity,
             BrowsingContextId
           },
       FrameDocumentNavigationLoadBinding
     }
  -> later submission takes the applicable source-owned route
  -> related Page residence resolves its current Context/JsContextHost
  -> root Document identity + BrowsingContextId resolve the exact live child
  -> target owner cancels only if current navigation-load binding still matches
```

关键不变量如下：

- persisted form state 不再保存 target `DomHandle` 或 `JsContextHost*`。`DomHandle` 只在当前 callback 内从
  stable `BrowsingContextId` 反查；related Page host 只通过 stable Page residence 的 current Context slot
  重新取得；
- related route 额外冻结 target root Document lifecycle identity。相同 Page residence 在 main Document
  replacement 后不能让旧 route 命中新 host 中恰好碰撞的 child/navigation allocator 值；
- `FrameDocumentNavigationLoadBinding` 同时限定 target Document task owner、navigation id 与 load-delay
  token。相同 child 已被普通 navigation 替换时，旧 form route 会从 source state 移除，但 target cleanup
  必须 no-op；
- submitter path 在新 target 是 child、top-level 或 miss/create 时都先消费该 form 的既有 child routes；
  programmatic path 保留既有回归合同，只替换相同 target 的 pending form navigation，不误删发往不同 child
  的 programmatic submissions；
- queue 成功后由 target owner 返回 exact navigation-load binding，再由 source form 登记。失败 queue 不会
  产生一个看似可取消、实际没有 scheduler generation 的 route；
- exact target cleanup 依次撤销 pending Window/entry seed、reserved service-worker client、child commit
  task、当前 Document parser/script ledger 与 exact pending main-resource load，随后 settle 同一个
  navigation/load-delay owner 并同步 child Window state；
- `NavigationResourceLoader::cancel()` 现在在 pending child load 被 owner 删除前显式调用。resource task
  持有的 clone 因而不能让 libcurl transport 在 exact form-cancel ledger 已清空后继续运行。普通 navigation
  supersession 仍保留既有 historical Network terminal，不套用这条主动 transport cancellation；
- related target commit 无法反向借用 source host 清理 form map。完成后的 route 可能暂时留到该 form 的
  下一次相关提交，但 exact root/child/load 三重校验会使它安全 no-op；每个 target identity 只保留一项，
  不按重复提交无限追加。

##### 本纵切建立的回归与当前证据

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(submitter_cancels_previous_same_form_navigation_in_a_related_page_child) | \
      test(submit_button_does_not_cancel_a_newer_non_form_navigation_in_the_previous_target)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 18d9c0d9-bcae-4c49-88ab-f38dfa5cb5a2：2 passed。
# related case 已是 in-flight HTTP 版本：A transport close、A 保持 about:blank、B request/Document commit
# 与 response body 均被断言；local case 证明 stale form token 不会清掉 replacement navigation。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E '<E2G 两条回归 + E2F/form/popup 邻接矩阵 18 条>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 865e048e-dc64-4084-9d59-09947ce6d1ca：20 passed。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(submitter_cancels_previous_same_form_navigation_in_a_related_page_child) | \
      test(submit_button_does_not_cancel_a_newer_non_form_navigation_in_the_previous_target) | \
      test(child_module_producer_boundaries_require_exact_task_owner)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run a3cc7c41-c243-46cc-82a2-f7df67d7eb76：3 passed；第三条锁住 stale owner 不得清除
# replacement Document 的 module/parser ledger，exact current owner 可以清除。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(submitter_cancels_previous_same_form_navigation_in_a_related_page_child) | \
      test(submit_button_does_not_cancel_a_newer_non_form_navigation_in_the_previous_target)' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run af92a077-2cf8-4ff5-815c-6bab74f2e9d7：20/20 iterations passed，每轮 2/2。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run 91820a56-3831-4bdf-94a0-7b7bcc049827：16011 passed、3 failed、18 skipped。
# 三条 failure 都要求普通 supersession 的 stale child response 继续产生 historical Network terminal；首版
# 将主动 loader.cancel() 错误放进通用 child-load clear，边界过宽。修复保留通用 historical terminal，
# 只在 exact form-cancel binding 命中时关闭 transport。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(stale_same_root_terminal_does_not_settle_newer_exact_navigation) | \
      test(response_for_replaced_child_document_is_historical_network_only) | \
      test(nested_stale_child_response_retains_producer_captured_parent_frame) | \
      test(submitter_cancels_previous_same_form_navigation_in_a_related_page_child) | \
      test(submit_button_does_not_cancel_a_newer_non_form_navigation_in_the_previous_target)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run a70995ca-0f6d-4e53-9337-dad7870c0707：5 passed；同时锁住 historical Network 与 exact cancel。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run fbfb803b-8a02-4621-9c3d-d45abc71b5ab：16014 passed、18 skipped；执行阶段 106.353s。

cargo fmt --all --check
# passed。

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 37s。
```

parser cancellation 复用 child Document owner 已有的 exact-ledger cleanup；它会清除 classic/module
scheduler、parser store、ready tasks 与 load-delay。当前证据包含该 owner 边界的既有
`child_module_producer_boundaries_require_exact_task_owner` 回归，但本轮 HTTP probe 直接锁住的是 main-resource
loader transport，不把它夸大成“阻塞 external script socket 一定同步关闭”。完整 Blink
`CancelClientNavigation()`（包括任意来源在新 target 上已有的 client navigation）、self-target
`CancelParsing()`、RemoteFrame scheduler endpoint 仍需后续纵切。

#### Phase 5E2H：child-source current-top causal/referrer carrier

E2G 解决了 target child 的 scheduler cancellation，但 child form 命中 `_top` 时，request 到达 target
Page 后仍会重新读取 target root 的 lifecycle/referrer。method/body 因 E2F carrier 得以保留，initiator
却发生了替换：source child 的 URL、Referrer Policy 与 exact Window/Document identity 没有跨过
同步 `Location` setter 和 renderer-to-protocol handoff。direct request 可能因此发送 root `Referer`；redirect、
cross-origin/downgrade 与 Fetch URL override 更会继续基于错误 source 或一条过早冻结的 header。

##### Chromium 合同与失败基线

本轮继续对照 Chromium `a03603fe9af6`：

- `FrameLoadRequest(LocalDOMWindow* origin_window, ...)` 在 request 构造时保存 `origin_window_`，并从该
  Window 的 `OutgoingReferrer()` / `GetReferrerPolicy()` 生成 request referrer；target Frame 并不替换它；
- `FormSubmission` 用 `Member<LocalDOMWindow> origin_window_` 跨过 target selection/scheduler，最终
  `Navigate()` 仍以这一个 origin Window 构造 `FrameLoadRequest`；
- `FrameLoader` 后续继续用 `GetOriginWindow()` 选择 requestor origin、fetch client settings、CSP 与
  navigation policy 输入。也就是说 source 是 causal/security input，target Frame/Page 才是 scheduler、
  loader 与 commit owner，两者不能合成一项“当前 root”；
- Chromium 的 `ResourceRequest` 保存 referrer URL/policy，由 network navigation 对实际 destination 处理；
  因而 Moli 也不能把 initial URL 的最终 `Referer` 字符串当作 redirect/DevTools URL override 的
  authoritative transport input。

实现前的两个 protocol 回归分别固定 direct + redirect/cross-origin 与 Fetch URL override 的违反路径：

```bash
cargo nextest run -p moli-protocol \
  -E 'test(child_form_top_navigation_keeps_source_referrer_across_redirect) | \
      test(child_form_top_navigation_recomputes_source_referrer_after_fetch_url_override)' \
  --no-fail-fast
# 初始 red：run 137178af-b7af-4ad1-9a1b-c83fc4d99261 中 direct request 使用 top `/source`；
# run 6990993a-6bb0-41c1-8e2e-29ffc68627af 中 Fetch pause 没有 child Referer。
# 首版只在 Location callback 内捕获 source 后，run b7a662d5-015a-4434-95da-bdde201f964b 仍为 0/2：
# callback 已进入 target realm，两条 request 都错误捕获 top `/source`。这证明 source 必须在 target
# Window setter 之前冻结，并通过同步 callback scope 显式传入。
```

##### typed source、target owner 与三阶段 referrer 投影

E2H 新增 `RendererTopLevelNavigationSource`，它与完整 method/body/header request 一起进入
`RendererTopLevelNavigationRequest`：

```text
source element / entered Window
  -> RendererTopLevelNavigationSource {
       root_document,
       window: RootFrame | ChildFrame { frame_id, local_window_id, document_id }
             | LightweightPopup { popup_id, popup_document_id },
       source_url,
       referrer_policy,
       suppress_referrer
     }
  -> target selection / synchronous target Location setter
  -> target Page pending top-level slot + exact Page handoff
  -> protocol NavigationDispatchState
       preflight event projection
       network transport policy input
       final Document commit seed
```

边界与不变量如下：

- source capture 发生在 hyperlink/form node owner 或 `window.open()` entered Window 上；initial inherited
  `about:` Document 的 outgoing source URL 读取其 `DocumentPolicyContainer.document_referrer`，不把
  `about:blank` 本身当作 HTTP referrer；
- target Page 继续唯一拥有 pending slot、handoff、navigation currentness、loader 与 response commit。
  carrier 只保存 causal facts，不把 source Page 变成第二个 scheduler；
- child `_top`/`_parent` 或 named-current-top 会同步进入另一个 Window 的 Location setter。caller 在该次
  V8 setter 周围安装可嵌套、立即恢复的 source scope；callback 生成的 target-root source 必须被这个显式
  source 覆盖。scope 不跨 task 保存，也不持有跨 re-entry 的 Rust mutable borrow；
- renderer publication 同时移动完整 request 和 typed source。`source_document` 继续提供 Page output 的
  causal root identity；`window` variant 再精确到 child LocalWindow/Document，不能用 URL 相同来冒充
  `RootFrame`；
- protocol preflight 为 `Fetch.requestPaused` / Network event 临时投影按 initial destination 计算的
  `Referer`，并标记 `SourcePolicyGenerated`。真正构建 libcurl request 时只移除这种 generated header，
  将 source URL、document policy 与 inference flag 交给共享 network `Request`；每个 actual URL/redirect
  hop 因而重新计算；
- `Fetch.continueRequest` 只改 URL 时保留 generated mode，新 URL 会重新计算。调用方显式提供 headers 时
  切为 `ExplicitOverride`：这些 headers 原样进入 transport，缺少 `Referer` 也不会被自动补回；
- `RendererMainDocumentCommitSeed` 保存同一 source。response final URL（transport error Document 则使用
  unreachable URL）确定后，再独立计算 `document.referrer`，不复用 HTTP header eligibility 或 preflight
  字符串；
- 普通 browser/CDP initiated navigation 没有 typed renderer source，继续走原有 target-session preflight，
  不因本轮改动改变 referrer owner。

##### 本纵切建立的回归与当前证据

```bash
cargo nextest run -p moli-protocol \
  -E 'test(child_form_top_navigation_keeps_source_referrer_across_redirect) | \
      test(child_form_top_navigation_recomputes_source_referrer_after_fetch_url_override)' \
  --no-fail-fast
# run e928ceb1-531b-4483-bcf0-ede5d8d75869：2 passed。
# 第一条同时断言同源 direct `/redirect`、跨 port redirect final 与最终 document.referrer 都是
# unsafe-url policy 下的完整 child URL；第二条断言 Fetch pause、URL-overridden transport 与 commit
# Document 都保留同一 child source。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E 'test(child_form_top_target_carries_exact_child_window_document_source) | \
      test(child_window_open_top_carries_exact_source_and_noreferrer_policy) | \
      test(child_form_top_navigation_keeps_source_referrer_across_redirect) | \
      test(child_form_top_navigation_recomputes_source_referrer_after_fetch_url_override)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 111a9b5f-af30-4895-8859-66dce7558312：4 passed。
# renderer 两条不是 URL-only 断言：它们比较 exact frame id/local-window id/document id，并锁住
# window.open(..., `_top`, `noreferrer`) 的 suppression bit。

cargo nextest run -p moli-protocol \
  renderer_navigation_source_recomputes_default_policy_for_actual_destination \
  --no-fail-fast
# run e0753570-aed5-4b57-8e6e-62c0b9f2bcb1：1 passed；默认 policy 的 same-origin full URL、
# cross-origin origin-only、HTTPS→HTTP downgrade 清空，以及 noreferrer/explicit-header inference gate
# 均在 carrier 边界锁定。

cargo nextest run -p moli-fetch -p moli-renderer-v8 -p moli-protocol \
  -E '<E2H 五条核心回归 + form/popup/Fetch/auth/response-stream 邻接矩阵 13 条>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run e78b5f4e-eefb-4635-8380-fddb15fcf3bf：18 passed。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E '<E2H 五条核心回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run f673a21b-a2ed-4c37-8a12-dbb643260909：20/20 iterations passed，每轮 5/5。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run 2cc1656c-c45d-4842-ae86-8df49c33acac：16021 passed、2 failed、18 skipped。
# 两条失败分别是 websocket parser-script Network/DCL backlog 与 file-chooser document-replacement
# shared-id 观察；均不在本轮改动路径，但由于涉及 lifecycle/currentness，继续按 flaky 规则复跑。

cargo nextest run -p moli -p moli-protocol \
  -E 'test(websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded) | \
      test(file_chooser_opened_renderer_backend_node_id_is_scoped_to_document_replacement)' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 5f5aa263-5d9f-4a8c-9545-2e43862fb272：20/20 iterations passed，每轮 2/2。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run 9051ae3c-80e1-4f03-a0f8-eab5112f2f37：16023 passed、18 skipped；执行阶段 102.298s。

cargo fmt --all --check
# passed。

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 34s。

git pull -r origin master
# Current branch popup-refactor is up to date；origin/master 没有基线漂移，HEAD 未重写。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E '<E2H 五条核心回归>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# 同步后 run ccdb722e-7685-4e10-96a3-e381b9d19b0a：5 passed。
```

本轮没有把共享 fetch runtime 扩张成完整 navigation Fetch Standard 实现。redirect response 自身通过
`Referrer-Policy` 改写后续 hop、Fetch response-stage fulfill/continueResponse 与显式 `Referer` 的全部
Chromium 关系仍需独立 probe；source security origin/CSP、sandbox/top-navigation permission 和
`javascript:` target-realm execution 在 E2H 入库时也不能从这个 URL/policy carrier 外推；该执行 owner 现已由
E2N 单独实现。当前 enum 中保留
`LightweightPopup` 只是迁移期兼容 source identity，不是 Phase 6 可以删除双栈的信号。

#### Phase 5E2I：sandbox new-auxiliary creation admission

E2H 之后，DOM producer 已能在 renderer 内精确区分 existing target 与 new auxiliary target，但 sandbox
policy 仍只有 origin/scripts/escape 的局部投影。一个没有 `allow-popups` 的 sandbox child 会照常创建
lightweight record、Page reservation、popup activation 和 `Page.windowOpen` observation；response CSP
`sandbox` 也不会阻止 root `window.open()`。这不仅是返回值错误，还会在被浏览器策略拒绝的动作上留下隐藏
Page/target/storage work，违反本章的 creation transaction 不变量。

##### Chromium/WPT 合同与失败基线

本轮继续固定 Chromium `a03603fe9af6`：

- `FrameTree::FindOrCreateFrameForNavigation()` 先调用 `FindFrameForNavigationInternal()`；只有没有命中
  existing context 时才进入 `CreateNewWindow()`。因此 sandbox popup gate 不能提前阻断 `_self` / `_parent` /
  `_top` 或 ordinary existing name；
- `CreateNewWindow()` 在真正创建 auxiliary Page 前检查 `WebSandboxFlags::kPopups`，失败返回 `nullptr`，并且
  不进入 embedder `CreateWindow()`；这也是 `window.open()` 返回 `null` 的 owner 边界；
- `services/network/public/cpp/web_sandbox_flags.cc` 将 `allow-popups` 映射为移除 `kPopups`（以及 custom
  protocol 相关 flag），而 `allow-popups-to-escape-sandbox` 只移除
  `kPropagatesToAuxiliaryBrowsingContexts`。后者不会隐式授予创建权限；
- 本地 WPT `html/browsers/sandboxing/sandbox-disallow-popups.html` 断言 blocked `window.open()` 的返回值为
  `null` 且 destination 不加载；`iframe_sandbox_popups_nonescaping-*` 与
  `iframe_sandbox_popups_escaping-*` 则分别要求 allowed popup 继承或逃离 sandbox。

renderer 回归先固定三个违反路径：

```bash
cargo nextest run -p moli-renderer-v8 sandbox_without_allow_popups
# run 4628abb3-0e05-4911-961b-58a860618203：2 failed。
# window.open(_blank/named-miss) 都返回非 null；anchor/form 也留下 popup activation。
# 同一 sandbox child 以自己的 ordinary name 为 target 已正确复用 existing context，证明失败点只在 new target。

cargo nextest run -p moli-renderer-v8 \
  response_csp_sandbox_requires_allow_popups_for_auxiliary_creation
# run 0eed2bb2-408b-40df-82d4-7257a370679a：1 failed；`sandbox allow-scripts` 仍创建 popup。
```

首版测试没有 `allow-same-origin`，parent test realm 对 opaque child 调用测试专用 `eval` 时先被正确的
cross-origin access check 拒绝；这不是产品失败证据。回归随后改为
`allow-scripts allow-same-origin [allow-forms]`，只移除测试 harness 的跨源干扰，仍保留 popup sandbox flag。

##### typed admission 与 owner 顺序

E2I 在 `DocumentPolicyContainer` owner 中新增一次性构造的
`AuxiliaryBrowsingContextCreationPolicy`。调用顺序是：

```text
source Window/element policy snapshot
  -> special / ordinary existing-target lookup
  -> DocumentPolicyContainer::into_auxiliary_browsing_context_creation_policy()
       sandbox active && !allow-popups => BlockedBySandbox
       allowed && propagates           => target policy keeps sandbox flags
       allowed && escape               => target policy uses unsandboxed defaults
  -> Page reservation / initial realm staging / popup activation / Page.windowOpen
```

具体边界如下：

- `DocumentSandboxPolicy` 现在显式保存 `allows_popups`。无 sandbox 时默认允许；iframe attribute 按 ASCII
  case-insensitive token 解析；多个 enforced response CSP 中只有每一个 active `sandbox` directive 都包含
  `allow-popups` 才允许。owner attribute 与 response CSP 合并时使用交集，不会被后来的宽松 policy 放大；
- `allow-popups-to-escape-sandbox` 保持独立交集。admission 先检查 `allow-popups`，再决定 accepted target
  是否继承 sandbox；因此 escape-only policy 必须拒绝，不能借 escape token 绕过 popup gate；
- `window.open()` 在 special/current-page/related-page name lookup 返回之后才构造 typed policy。child receiver
  的 policy 从已经选定的 `OwnerDispatchScope` 读取，而不是仅依赖 entered-realm marker；红测曾证明后者在
  `child.contentWindow.eval()` 嵌套调用中会错误回退到 root policy；
- hyperlink/form 的 full-creator path 在 current/related target hit 之后消费同一 typed policy；兼容
  `queue_popup_target_navigation()` 也从 exact dispatch scope 做同一 gate。blocked default action 返回 handled，
  但不会创建 lightweight record、reserve Page、记录 activation 或发布 `Page.windowOpen`；
- `open_lightweight_popup_window()` 的参数从原始 `DocumentPolicyContainer` 收紧为 admitted typed policy，避免
  当前 DOM 或非 DOM caller 绕过 gate。Service Worker `clients.openWindow()` 与 notification navigation 没有
  Document sandbox source，必须显式构造 browser-context/default admission；本轮没有把它们伪装成 root
  Window action；
- existing target 不消费 creation admission。回归中 sandbox child 仍可用自己的 ordinary frame name 命中
  self 并完成 same-document navigation，且不产生 popup activation。

##### 当前证据与明确未完成项

```bash
cargo nextest run -p moli-renderer-v8 sandbox_without_allow_popups
# run 734056d9-4ad8-4817-93be-2061ae7d33af：2 passed。

cargo nextest run -p moli-renderer-v8 sandbox
# 首次实现后 run f736c117-5418-4a76-827e-ccbfad47c86d：24 passed。
# 补入 direct-URL WPT 等价回归后 run e38bfe19-78a8-484a-987c-50f9d66df6b2：27 passed。
# 覆盖 attribute/CSP parser、window.open/link/form block、response CSP、既有 non-escape/escape popup、
# nested top-navigation denial、document.domain 与 sandbox storage 邻接行为。

cargo nextest run -p moli-renderer-v8 \
  auxiliary_creation_policy_separates_popup_admission_from_sandbox_escape
# run 45b184dc-66b5-47e4-922f-9a457cc281de：1 passed；escape-only 拒绝、propagating/escaping policy 分叉。

cargo nextest run -p moli-renderer-v8 -E '<E2I 七条 admission/inheritance 核心回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 772090fe-b0e7-46bc-9f3c-1169a8feefbb：20/20 iterations passed，每轮 7/7。
```

本轮也用当前 release binary 跑了三个最贴近的 upstream WPT，但结果不能被折算成全绿：

```bash
uv run --project moli-benchmark python -m moli_benchmark.wpt_cross \
  --wpt-root ../wpt --engine moli --mode cli \
  --moli-bin target/release/moli \
  --case html/browsers/sandboxing/sandbox-disallow-popups.html \
  --case html/semantics/embedded-content/the-iframe-element/iframe_sandbox_popups_nonescaping-1.html \
  --case html/semantics/embedded-content/the-iframe-element/iframe_sandbox_popups_escaping-1.html
# /tmp/moli-wpt-popup-sandbox-e2i-20260805：fail=1、timeout=2。
```

为了避免把旧的 `wpt-cross-current` pass 分类或 fixture 限制误当作本轮产品回归，又从本轮未修改起点
`128647d91cd2` 构建了独立 release binary，使用同一 runner/case set 做 A/B：基线同样是
`fail=1、timeout=2`。逐例证据进一步区分了失败边界：

- 基线 `sandbox-disallow-popups.html` 在约 54ms 直接失败，`window.open()` 返回
  `SecurityError: Blocked a frame ...`，没有满足 WPT 的 `null` 合同；当前实现已经越过该
  `assert_equals(e.data, "null")`，约 12s 后才因 `stash-take.py` 响应不是 JSON 而失败。本仓库
  `wpt_cross` fixture server 明确只实现白名单 Python handler，当前没有实现
  `/fetch/api/resources/stash-take.py` / `stash-put.py`，所以该 fail 的第二层“目标 URL 未加载”断言还不能作为
  产品证据；
- 两条 allowed-popup inheritance WPT 在 `128647d91cd2` 与当前 binary 上都 timeout，因此不是 E2I
  引入的回归，但也不能沿用 8 月 2/3 日更早全量快照中的 pass 作为当前验收。renderer 内增加了与 helper
  相同的 direct `window.open(location.href)` 回归，non-escape/escape 两条均通过，并进入上述 20 次 stress；
  CLI 路径仍暴露 auxiliary Page task/lifecycle pumping 的独立缺口，需后续用 protocol trace 收口。

因此，E2I 的强证据是 renderer owner admission、无 reservation/event 副作用、direct-URL realm 行为以及
基线/当前的返回值差分；“stash 证明 destination 绝未请求”和“当前 CLI inheritance WPT pass”仍是明确的
证据债，不能写成已完成。

提交前 workspace 门禁：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run 283da5a2-ae6e-473f-a593-a0a04f4439c0：16030 passed、18 skipped；执行阶段 100.014s。

cargo fmt --all --check
# passed。

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 30s。

git pull -r origin master
# origin/master 从 d4070fec16 前进到 780d9fe8ed；43 个 topic commit 无冲突完成 rebase。

cargo nextest run -p moli-renderer-v8 sandbox \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# rebase 后 run 5efdca76-c6a2-4ddd-b19f-5b1a162833d0：27 passed。

cargo nextest run -p moli-renderer-v8 -E '<E2I 七条 admission/inheritance 核心回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# rebase 后 run 85e6aa59-bf56-404c-ac5c-556ab50f70bb：20/20 iterations passed，每轮 7/7。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# rebase 后 run a36f4e37-5172-4fc1-9dde-2b96ba8710c3：16030 passed、18 skipped；执行阶段 135.592s。

cargo fmt --all --check
# rebase 后 passed。

cargo clippy --workspace --all-targets -- -D warnings
# rebase 后 passed；1m 34s。
```

这轮只完成 **new auxiliary admission**，不把它写成完整 sandbox/activation transaction：

- E2I 入库时，opener-preserving staged initial realm 已消费 admitted policy，而 Fresh/no-local-proxy Page
  reservation 尚未把 accepted sandbox flags 作为 public renderer-to-Page carrier 交给实际 initial Document
  build；该缺口已由下节 E2J 补齐，且没有让 protocol 根据 opener/target name 反推；
- `allow-forms` 是 form submission 自身的 permission，不等于 `allow-popups`；该独立 source-Document owner
  已由 E2L 接入。完整 sandbox top-navigation flags、opener relation、activation exception、fenced/remote 分支
  仍属于 `CanNavigate`；
- 当前 `userGesture` 只是 protocol observation，没有持久 transient activation 或消费 ledger。本轮 gate 在
  reservation/event 之前，因此不会新增错误消费，但也没有实现 popup blocker 或“allowed creation consumes
  activation”的 Chromium 语义；
- Chromium 的 sandbox console diagnostic、`javascript:` 最终 target realm/CSP/currentness，以及 COOP group
  sever 均未在 E2I 实现；其中 full-creator JavaScript URL owner 现已由 E2N 完成。

E2I 入库时确定的下一最小纵切是 **Fresh auxiliary accepted-sandbox carrier**，让 implicit noopener
anchor/form 与显式 noopener `window.open()` 的 initial Page 使用 renderer 已冻结的同一 policy。该纵切现已由
E2J 完成；popup blocker 与 transient activation 消耗又已由 E2K 接入同一 creation transaction；form
source-Document `allow-forms` 双门禁已由 E2L 完成。下一步先补 direct `form.submit()` 晚门禁之前的
creation-only target carrier；该纵切已由 E2L.1 完成，下一步进入完整 `CanNavigate`。

#### Phase 5E2J：Fresh auxiliary accepted-sandbox carrier

E2I 只把 sandbox 决策推进到“是否允许创建”与“是否逃离 sandbox”。opener-preserving Related Page 能在同步
staging 时直接消费 accepted policy，但 Fresh/no-local-proxy activation 只把 Page reservation 交给 protocol；
`noopener` 又有意不保留 DOM opener。因此如果 target owner 在创建事务里没有单独持有 frame policy，initial
`about:blank` 与真实 destination 都会回退到 URL/default policy，`allow-popups-to-escape-sandbox` 未出现时也会
错误逃离 sandbox。

##### Chromium 合同与 owner 选择

本轮继续以 Chromium `a03603fe9af6` 为固定基线，关键源码边界是：

- `third_party/blink/renderer/core/page/create_window.cc` 的 `CreateNewWindow()` 在 popup admission 之后，独立于
  `features.noopener` 计算 `sandbox_flags`：creator 仍受
  `kPropagatesToAuxiliaryBrowsingContexts` 约束时传递 active flags，否则传 `kNone`；
- `content/browser/web_contents/web_contents_impl.cc` 的 `CreateWithOpener()` 即使收到
  `opener_suppressed=true`，仍先用 `opener_rfh->active_sandbox_flags()` 设置新 root 的 pending frame policy；
  `SetOpenerForNewContents()` 只在 `!opener_suppressed` 时建立可脚本访问的 opener edge。也就是说“谁创建了
  auxiliary context”和“新 Window 是否暴露 opener”是两条不同关系；
- `content/browser/renderer_host/render_frame_host_impl.cc` 在 initial empty Document 上把 effective frame policy
  与 CSP sandbox 合并。后续 Document 也以 browsing-context/frame policy 为底，再叠加本次 response CSP；
  sandbox 不是从 opener URL、target name 或当前 `Window.opener` 反推的单次导航参数。

据此，本轮选择 **target Page slot 是跨 Document 的长期 owner，renderer 是 policy 的唯一解释者**：protocol
需要持有一个可复制的 typed value，以便重建 navigation wrapper 或 active/background target 切换后继续交回
renderer，但不能看到或分支判断 `DocumentSandboxPolicy` 的内部 flags。把 carrier 放进短命
`NavigationEngine` 会在 target runtime 重建时丢失；把它留在 opener relation 又会与 noopener 的正确 sever
冲突。

##### typed carrier 与 Document commit 路径

新增的 `RendererAuxiliaryBrowsingContextPolicy` 对 core/protocol 公开类型身份，但 sandbox payload 和解释方法
保持 renderer-private。完整流向是：

```text
creator DocumentPolicyContainer
  -> AuxiliaryBrowsingContextCreationPolicy（E2I admission/escape decision）
  -> RendererAuxiliaryBrowsingContextPolicy
  -> RendererPendingPopupActivation（仅 Fresh disposition 必须携带）
  -> PopupTargetCreation（opaque transport）
  -> TargetPageSlot（browsing-context lifetime owner）
       +-> initial empty Document build 的显式 input
       `-> 每次 TargetNavigationLoadInputs
             -> RendererMainDocumentCommit
             -> replacement Document build
  -> PageVm/ScriptVm realm bootstrap 与 response-policy merge
```

边界约束如下：

- explicit noopener `window.open()` 与 implicit/explicit noopener hyperlink/form 的 Fresh producer，在 Page
  reservation 之后、设置 `FreshUnnamed` / `FreshNamed` disposition 之前附上 frozen policy。activation 断言
  `Fresh iff carrier present`；Related activation 不携带它，因为 live staged Page 已经拥有自己的 initial
  environment；
- `PopupTargetCreation`、`BrowserContext::stage_popup_background_target()` 和 `TargetRuntimeSlot` 只搬运 opaque
  value。carrier 必须与 renderer Page reservation 同生；没有 reservation 却出现 carrier 会立即触发 owner
  invariant，而不是静默创建一份 protocol-side sandbox projection；
- `TargetPageSlot` 在 `replace_loaded_page()`、background/active promotion 与 navigation-engine wrapper replacement
  之间保留 carrier。每次 load inputs 都从 exact target slot snapshot，再写入该次
  `RendererMainDocumentCommit`；initial empty build 则走独立显式 input。renderer owner 会拒绝同一 Document 同时
  收到互相冲突的 explicit/commit policy；
- renderer owner 在 realm bootstrap 前先以 inherited frame sandbox 和本次 enforced response sandbox 求交集，
  把 effective policy 同时用于 initial security origin、Window runtime origin、storage key 和 script execution
  gate；随后 response CSP 安装进 `DocumentRuntime` 时再次以原 accepted frame policy 合并，避免 response setter
  覆盖继承 flags。强制 opaque origin 时，top-level storage key 使用 browser-context 单调分配的 opaque nonce；
  每个 replacement Document 获得自己的 nonce，不把同 URL 的两个 opaque Document 合并；
- `Location.origin` 的 top-level projection 也改为先读取 authoritative current Document policy，再做 URL 自身
  是否 opaque 的 fallback；否则 policy 已正确禁止 `document.domain` 时仍会错误暴露 tuple origin；Window
  runtime 的 replaceable `origin` slot 同样从 bootstrap 时已经安装的 Document sandbox policy 初始化，不能再次
  从 URL 覆盖 opaque origin；
- escape-sandbox admission 会冻结 default/unsandboxed carrier，因此 noopener 仍创建 Fresh group，但不会被错误
  施加 creator sandbox。carrier 本身不恢复 opener、不共享 creator tuple origin，也不改变 session-storage 的
  noopener 分叉。

##### 红测、回归与证据边界

protocol 红测从带 `Content-Security-Policy: sandbox allow-scripts allow-popups` 的 opener 创建显式 noopener
Fresh target，随后在 target realm 同时观察 `location.origin` 与 `document.domain`：

```bash
cargo nextest run -p moli-protocol \
  noopener_popup_retains_creator_sandbox_policy_across_document_navigations \
  --no-fail-fast
# red run b1abfc80-23bd-4bc8-aa51-addfc66cad5b：1 failed；
# 实际为 http://127.0.0.1:<port>|allowed，预期 null|SecurityError。
```

首版 carrier 接通后 `document.domain` 已抛 `SecurityError`，但 `location.origin` 仍按 URL 返回 HTTP origin；这份
中间差分证明 policy 已到达 Document owner，同时暴露了 Location 的旧 URL-only 投影。补测“creator 允许 escape，
target response 自己施加 CSP sandbox”时又得到 `Window.origin=http://...`、`location.origin=null`、
`document.domain=SecurityError`，进一步证明 effective response policy 必须在 realm bootstrap 前形成，Window
runtime state 不能随后按 URL 覆盖它。

修复两处投影后，最终回归覆盖五个状态：显式 noopener `window.open(non-empty)` 的首个 destination 和第二次
`Page.navigate`，implicit noopener anchor 创建的 initial `about:blank` 和其后 destination，以及 escape
creator 创建、由 target response CSP 重新 sandbox 的 Fresh realm。前四者锁住 creator frame-policy persistence，
第五个锁住 response intersection 的 bootstrap 时序；全部必须返回
`origin|location.origin|domain = null|null|SecurityError`，且 anchor 与 form 共享同一个 element Fresh producer：

```bash
cargo nextest run -p moli-protocol \
  -E 'test(noopener_popup_retains_creator_sandbox_policy_across_document_navigations) or test(fresh_noopener_popup_applies_response_sandbox_before_realm_observation)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 8e1a6134-991b-43ab-9c0e-4620a9787d0b：2 passed。

cargo nextest run -p moli-renderer-v8 \
  auxiliary_creation_policy_separates_popup_admission_from_sandbox_escape \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run ead96a1d-3b87-4f29-8755-1544d4c25268：1 passed；
# inherited/escaped typed carrier 与 E2I policy container 的 sandbox 完全一致。

cargo nextest run -p moli-renderer-v8 sandbox \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 7d65f712-aa7b-4a52-a5ba-2ef83a3b7d89：27 passed；覆盖 E2I admission、
# inheritance/escape 与相邻 origin/script/document.domain/storage/top-navigation sandbox 行为。

cargo nextest run -p moli-protocol \
  -E 'test(noopener_popup_retains_creator_sandbox_policy_across_document_navigations) or test(fresh_noopener_popup_applies_response_sandbox_before_realm_observation) or test(noopener_and_noreferrer_popups_have_one_real_navigation_with_creator_referrer_policy) or test(anchor_blank_target_uses_implicit_noopener)' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 27497ade-33a5-4f11-8a10-877109b5f8e1：20/20 iterations passed，每轮 4/4；
# 同时锁住 sandbox carrier、E2H referrer/single-owner 与 E1 implicit-noopener 邻接路径。
```

本轮没有把历史 `fail=1、timeout=2` 的三条 sandbox WPT 重新包装成 E2J 的通过证据：本地
`iframe_sandbox_popups_nonescaping-*` / `escaping-*` helper 使用 opener-preserving `window.open()` 或
`rel=opener`，走的是 E2I 已覆盖的 Related path，不会命中本轮 Fresh/no-local-proxy carrier；而旧 WPT runner
仍有 stash fixture 与 auxiliary task pumping 的已知限制。E2J 的强证据因此是 protocol target 的 initial /
replacement realm 观测、renderer typed-policy 单测和下述完整门禁；直接对应 Fresh noopener sandbox inheritance
的 upstream WPT 仍是证据债，不能沿用旧关键字快照声称已通过。

提交前 workspace 门禁：

```bash
cargo nextest run --no-fail-fast
# run 5e361013-7531-4e35-8414-2d114e5913a7：16032 passed、18 skipped；
# 执行阶段 101.504s。

cargo fmt --all --check
# passed。

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 33s。
```

同步 `origin/master` 后的基线与复验：

```bash
git pull -r origin master
# origin/master 从 93ceb0ed1f 前进到 08d0340f0c；44 个 topic commit 完成 rebase。
# 最早 popup 设计提交与 master 已重写的 cdp-runtime-renderer-runtime-current.md 有一处冲突；
# 保留 master 的 2026-08-05 current-owner 文本，popup 文档仍由 docs/README、child-context 和 isolate 文档导航。
```

master 已把 `ParsedCdpCommand` 的旧 `traits` 收进 `CdpRendererCommandPolicy`。旧 navigation-continuation
topic commit 重放后还有一个 getter 读取已删除字段，属于 Git 未能识别的语义冲突；同步适配是在 typed policy
上补只读 accessor，并让 scheduler getter 从同一 ingress-frozen policy 读取，没有恢复第二份 method 分类。

```bash
cargo nextest run -p moli-protocol \
  -E 'test(noopener_popup_retains_creator_sandbox_policy_across_document_navigations) or test(fresh_noopener_popup_applies_response_sandbox_before_realm_observation) or test(noopener_and_noreferrer_popups_have_one_real_navigation_with_creator_referrer_policy) or test(anchor_blank_target_uses_implicit_noopener)' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# rebase 后 run 366ae4fe-9554-498b-aa8c-2e914548f16f：20/20 iterations passed，每轮 4/4。

cargo nextest run --no-fail-fast
# rebase 后首次 run 234b8b36-8923-4327-834e-93d2ad5f9261：16046 passed、3 failed、18 skipped。
# 三条失败分别为 parser-script network backlog、replacement Document file chooser backend-node scope
# 和 per-Page isolate churn SIGABRT；均不在 popup carrier 路径。

cargo nextest run \
  -E 'test(websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded) or test(file_chooser_opened_renderer_backend_node_id_is_scoped_to_document_replacement) or test(per_page_isolate_navigation_churn_disposes_replaced_page_vms)' \
  --stress-count 50 --flaky-result fail --test-threads 3 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 64718878-96b0-4806-9e42-e26c1adc8804：50/50 iterations passed，每轮 3/3；
# 不用 sleep/retry 或降低 workspace 并发修改产品行为。

cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# rebase 后默认并发复跑 run a5064be3-7b7f-473c-a9dd-a525bc490970：
# 16049 passed、18 skipped；执行阶段 120.199s。

cargo fmt --all --check
# rebase 后 passed。

cargo clippy --workspace --all-targets -- -D warnings
# rebase 后 passed；1m 41s。
```

E2J 入库时确定的下一最小纵切是 renderer-owned transient activation ledger：existing-target navigation 不消费，
真正通过 sandbox admission 的 new-context creation 才执行 blocker decision 和至多一次消费，并把
`Page.windowOpen.userGesture` 降为该 transaction 的 observation。该纵切现已由 E2K 完成；form
`allow-forms` 的 source-Document 双门禁又已由 E2L 完成。下一步处理 direct `form.submit()` 晚门禁之前的
creation-only target side effect；该纵切已由 E2L.1 完成。后续依次处理完整 sandbox/top-level `CanNavigate`
和 `javascript:` 最终 target-realm execution；其中当前 owner 可表达的 local `CanNavigate` 已由 E2M.1
完成，full-creator `javascript:` final-target Page task/realm 又已由 E2N 完成。

#### Phase 5E2K：transient user activation 与 popup creation transaction

E2I/E2J 已经回答“sandbox 是否允许创建”和“accepted frame policy 由谁跨 Document 持有”，但此前
`userGesture` 仍只是 `JsContextHost` 上的命令栈深度：`Runtime.evaluate(userGesture=true)` 进入 V8 前加一，
命令返回后立即减一；`navigator.userActivation.isActive` 与 `hasBeenActive` 读取同一个瞬时布尔值。结果是
gesture 不能跨 protocol command 保留，new popup 不消费，existing target 与 new context 也没有统一的
blocker/consume 顺序。真实 Input dispatch 则完全不会授予 activation。

##### Chromium / WPT 固定合同

本轮仍以 Chromium `a03603fe9af6` 和 upstream WPT checkout
`db95fafd1fcef8428805e41eb5705d444e8c67ce` 为固定对照，关键边界如下：

- `third_party/blink/renderer/core/page/frame_tree.cc` 的
  `FrameTree::FindOrCreateFrameForNavigation()` 先执行 named/special target lookup；只有没有 frame 才进入
  `CreateNewWindow()`。已有 frame 随后只做 `CanNavigate` / focus，因此 existing-target navigation 不应进入
  popup blocker，也不应消费 activation；
- `content/browser/renderer_host/render_frame_host_impl.cc` 的 `CreateNewWindow()` 先计算 effective transient
  activation，再调用 embedder `CanCreateWindow()`；denial 在 consume 前返回。admitted path 随后在创建 Page、
  处理 opener-suppressed return 之前消费 browser/frame-tree activation；
- `content/renderer/render_frame_impl.cc` 只有在 browser 返回非 blocked/non-reuse 结果后才消费 initiating
  renderer 的 transient activation；即使最终是 noopener/ignore，也已完成 consume；
- `third_party/blink/public/common/frame/user_activation_state.h` 与
  `third_party/blink/common/frame/user_activation_state.cc` 分开保存 sticky bit 和 transient expiry timestamp。
  普通 build 的 lifespan 是 5 秒（MSan 特例 60 秒），expiry 边界为 inclusive；`ConsumeIfActive()` 只成功一次；
- `third_party/blink/renderer/core/inspector/thread_debugger_common_impl.cc` 的 `beginUserGesture()` 调用
  `LocalFrame::NotifyUserActivation()`，并不是 inspector command scope 的临时 flag；
- WPT `html/browsers/windows/consume-user-activation/window-open.html` 明确要求：创建新 window 后
  `isActive == false`，重新打开 existing named window 后 `isActive == true`。multi-global 变体又要求从 entry /
  incumbent / relevant globals 中选对被消费的 frame-tree activation。

这些事实还区分了两项不能揉成一个布尔值的策略：**是否有 transient activation** 是 renderer/frame-tree
动态状态，**没有 activation 时 embedder 是否仍允许 popup** 是 browser-context policy。Chrome 的 UI profile
通常用前者驱动 popup blocker，但 content embedder、content setting、testing/automation 可以放行；放行不代表
已有 activation 可以被重复使用。

##### renderer-owned ledger 与 typed admission

`JsContextHost` 现在持有 `TransientUserActivationLedger`，而不是
`protocol_user_gesture_activation_depth`。ledger 保存：

- 单调 `generation`，每次 trusted/protocol notification 创建一个新 identity 并刷新 5 秒 expiry；
- transient grant：未过期且未消费时由 `navigator.userActivation.isActive` 与 activation-gated WebAPI 读取；
- sticky bit：本 Document/frame lifetime 曾经激活后保持为真，由 `hasBeenActive` 单独读取；
- consume：用同一个 `Instant` 冻结 observed generation 并清除它，避免 expiry 边界把 observation 与消费拆成
  两个不同结果。

创建事务的实际责任链是：

```text
DevTools userGesture / trusted input
  -> Page/frame-tree TransientUserActivationLedger

DOM popup producer
  -> special / named current-Page / related-Page / legacy named lookup
       -> existing hit: navigate exact target; no admission, no consume
       `-> miss:
            DocumentPolicyContainer
              -> sandbox allow-popups admission
              -> RendererPopupBlockerPolicy decision
              -> observe + consume exact transient generation
              -> AuxiliaryBrowsingContextCreationAdmission
                   + accepted sandbox policy
                   ` RendererPopupCreationUserActivation
              -> reserve/create auxiliary Page
              -> RendererPendingPopupActivation owner action
              ` Page.windowOpen observation of frozen pre-consumption state
```

`AuxiliaryBrowsingContextCreationAdmission` 把 E2I 的 accepted sandbox policy 与本次
`RendererPopupCreationUserActivation` 收进同一 renderer-only value。后者同时保存 observed/consumed generation，
构造时要求两者完全相同；无 gesture 的 embedder bypass 则两者都为 `None`。这不是 protocol 可重算的字段：
`RendererPendingPopupActivation` 仅保留 typed result 用于 owner invariant/诊断，`Page.windowOpen` 事件从同一
frozen result 取得 `userGesture`，发布边界会断言两者一致。

`RendererBrowserContextRuntime` 新增公开的 `RendererPopupBlockerPolicy`：

- `AllowWithoutTransientActivation` 是 Moli 默认值，保持现有 headless automation/抓取工作负载不会因
  新 blocker 突然少建 target；但如果创建时确实存在 activation，仍必须消费；
- `RequireTransientActivation` 提供严格 Chromium-like admission，可由 embedder/browser profile 配置；没有
  active grant 时 `window.open()` 返回 `null`，且不会 reserve Page、创建 lightweight record、发布 owner action
  或 `Page.windowOpen`。

顺序由 `JsContextHost::admit_new_auxiliary_browsing_context()` 唯一负责：先消费
`DocumentPolicyContainer::into_auxiliary_browsing_context_creation_policy()` 的 sandbox verdict，再读取 blocker
policy，最后才 observe/consume ledger。sandbox denial 和 blocker denial 都发生在消费前；一旦 embedder 接受，
即使后续是 noopener/Fresh return 或具体 Page 创建失败，也不能把 grant 退回给脚本，这与 Chromium
browser-side consume 时机一致。

##### existing target 与所有 DOM producer

`window.open()`、hyperlink、form 和 compatibility `queue_popup_target_navigation()` 都改为调用同一个 admission。
其中 target lookup 必须严格先行：

- renderer-owned current/related Page 与 nested frame 继续消费 E2A-E2F 的 typed resolver；命中时 activation
  action 不携带 creation result；
- 尚未删除的 named lightweight record 增加显式 preflight reopen。它在 sandbox/blocker/consume 之前导航旧
  record，并发布 existing-target owner action但不发布 `Page.windowOpen`；`open_lightweight_popup_window()` 内部
  仍保留同一 lookup 作为非 DOM compatibility fallback；
- 真正 miss 后，Related、Fresh、legacy lightweight 和 protocol fallback new-context action 都携带同一
  creation result。Fresh path 仍额外携带 E2J accepted frame policy；没有 Page reservation 的旧 compatibility
  path 不伪造 Fresh policy carrier。

因此同一个 activated command 可以先复用 named target，再创建一个新 target：第一次 lookup 后 activation
仍为 active，第二次 creation 才消费。反过来，同一 activation 连续请求两个新 target，在严格 policy 下只有
第一个被创建；默认 automation policy 会创建两个，但第二个 `Page.windowOpen.userGesture` 必须是 `false`。

Service Worker `clients.openWindow()` 与 notification navigation 仍属于 browser-context/non-DOM producer；本轮
没有把它们伪装成某个 root Document 的 user activation，也没有迁移其 lightweight owner。这保持了 Phase 6
删除清单的真实边界。

##### activation source 与 WebAPI observation

V8 inspector `userGesture` 现在只在 command 开始时 `notify_user_activation()`，不再在 command 返回时清除；后续
task/command 在 expiry 前可观察并消费同一 grant。Input dispatcher 按 HTML activation-triggering input 规则在
author listener/default action 之前通知：mouse `mousedown`、non-mouse pointer 对应的 `touchend`、以及非
`Escape` 的 `keydown`。脚本 `element.click()` 不经过这条 trusted-input ingress，因此不会绕过严格 blocker。

WebDriver BiDi 的 `script.callFunction` / `script.evaluate` `userActivation=true` 也复用这条 protocol ingress。
这不是 command-local override：当前 WebDriver BiDi Editor's Draft 的执行算法要求在 author function/expression
前运行 HTML **activation notification steps**，没有在返回时恢复旧状态；upstream
`webdriver/tests/bidi/script/{call_function,evaluate}/user_activation.py` 也会在每个参数用例前显式调用
`window.open()` 消耗上一条 grant。因而 BiDi、CDP 和 trusted input 产生的是同一种可跨 command 保留、直到
消费或 expiry 才失效的 transient state；page global 不能伪造它，而消费后 sticky state 仍为 true。

原先读取 command-depth flag 的 pointer lock、storage access、vibrate、clipboard/editing command 等入口统一
读取 persistent transient state。`navigator.userActivation.hasBeenActive` 则从 sticky bit 读取，不再随 popup
consume 变回 false。当前每个 local child realm 都由同一 Page `JsContextHost` 编排，所以 ledger 是 local
frame-tree aggregate；这与 popup consume-all-local-frame-tree 的结果一致，但还不是 Chromium 对 OOPIF/remote
proxy 的逐 frame replication 模型。

##### 红测、聚焦回归与协议证据

实现前先加入两条 owner-level 红测：DevTools gesture 必须跨 command 保留直到 new context 消费，以及 existing
named target reuse 不消费、后续 new context 才消费：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(protocol_user_gesture_persists_until_new_auxiliary_creation_consumes_it) or test(existing_named_target_does_not_consume_popup_user_activation)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# red run 06567c3a-daa8-4856-9551-85ba549aead2：2 failed。
# command 结束后实际 [false,false]；existing reuse + new creation 后实际仍 active=true。
```

typed ledger/admission 接通后的首个同集 run：

```bash
# run 5ead0a45-9b45-4e8c-a0ff-052df896a64b：2 passed。
```

随后回归扩展到 5 秒 inclusive expiry、single-consume/new generation、严格 blocker、sandbox-before-consume、
synthetic vs trusted mouse，以及 Escape/non-Escape keyboard 与 touch release：

```bash
cargo nextest run -p moli-renderer-v8 -E '<E2K 八条 ledger/admission/input 回归>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 770ad35c-b82d-4b9e-9259-f5395b1bea71：8 passed。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(popup) or test(window_open)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run e1968a1a-c186-4977-9441-ffa93df539d2：111 passed。
```

协议层新增一个 `Runtime.evaluate(userGesture=true)` 命令连续创建两个 `_blank` 的回归。Moli 默认策略允许
两个 target 都创建，但 expression 在第一次之后观察到 `[isActive, hasBeenActive] == [false,true]`，两个
`Page.windowOpen.userGesture` 必须依次为 `true,false`。该用例与其他六条 owner 回归一起通过：

```bash
cargo nextest run -E '<E2K renderer + protocol 七条首轮回归>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run a5ab5842-3a2c-4c71-8429-58a82e7b5e64：协议/ledger 6 passed；
# 唯一失败是低层 parsed fixture 未安装 root lifecycle，trusted input 本身已正确显示 active=true。
# 给该产品级 popup 测试安装 production-shaped root lifecycle 后，
# run 92636417-2b22-4a4f-945c-4605cb35239b：该用例 passed。
```

跨 command 的 protocol adoption 另有独立回归：第一次 activated named creation 返回
`opened=true,active=false,sticky=true`；第二个 `userGesture=true` command 重开同名 target，必须复用原
WindowProxy、不发布第二个 target/event，并返回 `reused=true,active=true,sticky=true`。它与上述双 `_blank`
event observation 一起复跑：

```bash
cargo nextest run -p moli-protocol \
  -E 'test(page_window_open_observes_pre_consumption_activation_for_each_new_context) or test(window_open_named_target_reuses_existing_popup_target)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 94124b0c-0868-4886-8667-98034a19b569：2 passed。
```

首轮 workspace gate 还发现一条早于 E2K 的 BiDi 回归把 `userActivation=true` 错写成 command-local scope：

```bash
cargo nextest run --no-fail-fast
# run 80d60f73-df77-437f-9bca-d00282811575：
# 16058 tests run，16057 passed / 1 failed / 18 skipped；唯一失败为
# websocket_bidi_call_function_user_activation_controls_navigator_and_copy。

cargo nextest run -p moli \
  -E 'test(websocket_bidi_call_function_user_activation_controls_navigator_and_copy)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run a3bcc91e-7fb4-4e0d-870f-e54a6d35edf2：1 passed。
```

该回归现在要求 activation 跨下一条无 gesture command 保留，再用 `window.open()` 精确消费 transient grant，
同时验证 `hasBeenActive` 保持 true；消费后设置同名 page global 也不能令 `isActive` 复活。这与 BiDi 规范算法和
upstream WPT 的显式 pre-consume 结构一致，避免为了兼容旧断言重新引入 command-end clear。

修正旧回归后的本轮 pre-commit workspace gate：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail \
  --failure-output immediate
# run 31b6c1d4-afb2-49b2-9b93-e95e473b2193：
# 16058 passed、18 skipped；执行阶段 116.726s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 46s。
```

当前 release binary 也跑了 upstream `consume-user-activation/window-open.html`，但结果揭示的是 WPT bridge
限制，不能写成产品通过：

```bash
cargo build --release -p moli
# passed；1m 30s；binary sha256
# e26ecd53a37532422425bf26df09752127dc1f008375a796c30f2cb977433571。

uv run --project moli-benchmark python -m moli_benchmark.wpt_cross \
  --wpt-root ../wpt --engine moli --mode cdp \
  --moli-bin target/release/moli \
  --output-dir /tmp/moli-wpt-popup-activation-e2k-20260805-1 \
  --case html/browsers/windows/consume-user-activation/window-open.html
# fail=1；harness OK；2 subtests 中 1 pass / 1 fail。
# fail: Opening an existing window should not consume user activation。

# 同一 binary/matrix 的 CLI mode：
# /tmp/moli-wpt-popup-activation-e2k-20260805-cli-1；同样 1 pass / 1 fail。
```

这个 fail 不能反推 production existing-target consume 仍错误：仓库
`moli-benchmark/moli_benchmark/wpt_cross/server.py` 注入的
`test_driver_internal.click()` 使用页面内 `dispatchEvent(new MouseEvent(...))`，所以事件必然
`isTrusted == false`，按规范与 E2K 实现都不能授予 activation。第一个“new window consumes”subtest 只是从未
active 的 `false` 得到弱通过，第二个 bless 后仍为 false 才暴露 bridge 缺口。用同一 runner 跑系统 Chromium
145.0.7632.116 也不是有效 comparator：第一条通过后 popup cleanup 失败，harness ERROR，第二条 NOTRUN；输出在
`/tmp/chrome-wpt-popup-activation-e2k-20260805-1`。因此本轮强证据是 owner/protocol/trusted-input 回归；要让该
upstream case 成为验收门禁，WPT CDP driver 必须把 `test_driver.click()` 接到真实 `Input.dispatchMouseEvent`
或 WebDriver action，而不是在页面内伪造 trusted event。

本轮实现提交推送后按 topic 约定同步 master；远端 master 没有新增提交，因此 rebase 是 no-op、没有冲突或
commit rewrite。仍在同步后的精确 HEAD 上复验所有 E2K owner 边界与 workspace gate：

```bash
git pull -r origin master
# Current branch popup-refactor is up to date.

cargo nextest run -E '<E2K 11 条 ledger/admission/input/protocol/BiDi 回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 332ce20f-910c-4df4-a4b9-2e81c27ff6b5：20/20 iterations passed，
# 每轮 11/11；覆盖 expiry/single-consume、sandbox/blocker order、trusted input、
# Page.windowOpen pre-consumption observation、named reuse 与 BiDi persistence/consume。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail \
  --failure-output immediate
# run eba0efa4-aac5-4486-bfdf-18f8f91b8417：
# 16058 passed、18 skipped；执行阶段 100.650s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；incremental 0.19s。
```

##### 有意保留的边界与下一步

E2K 不是完整 User Activation v2 或 popup UI 完成标志：

- strict policy 目前是 `RendererBrowserContextRuntime` API，还没有 content-setting、CLI/CDP option、site exception
  或 popup-blocked UI/console diagnostic；默认仍为 automation-friendly allow；
- local child 使用 Page aggregate ledger，尚未实现 Chromium 的 notification same-origin visibility、ancestor /
  descendant propagation、RemoteFrame/browser replication、restricted extension activation 与 history activation；
- expiry 采用读取时惰性判定，不需要 timer/sleep；Document replacement 创建新 host 时会建立新 ledger。后续若
  引入 BFCache/Page restore，需要显式定义 activation snapshot，而不能复用旧 host 残留；
- E2K 提交时 Service Worker/notification producer 与 popup blocked diagnostics 仍按各自 Phase 5E/6 边界处理；
  full-creator DOM `javascript:` 的最终 target Page/realm/CSP/currentness 后由 E2N 完成，非 DOM producer 没有
  冒充 Window source，并已在 E3 以 browser-context source 迁入 Fresh Page；
- 当前 upstream user-activation WPT bridge 只能合成 untrusted event；把真实 protocol input 接入 testdriver 是
  harness 证据债，不应通过给 synthetic event 授权来让 case 变绿；
- E2K 入库时的下一最小纵切是 form `allow-forms`。该 source-Document 双门禁现已由 E2L 完成；它与 popup
  blocker 不同，也能阻止向 **existing** target 提交，因此没有错误塞进只为 new-context 构造的 admission。

#### Phase 5E2L：source-Document `allow-forms` 双门禁

E2I/E2J 已经能回答一个 sandboxed Document 是否允许**新建** auxiliary context，并把 accepted frame policy
交给实际 target Page；但 form submission 自身一直没有 `allow-forms` owner。此前一个无 `allow-forms` 的 child
可以向 existing named iframe/Page 排队 GET/POST，也可以先进入 `_blank` creation admission，因而
`allow-popups`、popup blocker 和 target resolver 被迫替 form permission 做错误的间接判断。response CSP
`sandbox` 也只投影 scripts/origin/popups，无法阻止 root Document form submission。

##### Chromium 合同、双门禁与一个可观察实现细节

本轮仍以 Chromium `a03603fe9af6` 为固定源码基线。关键边界不是 `LocalFrame::CanNavigate()`，而是
`HTMLFormElement` 自己：

- `third_party/blink/renderer/core/html/forms/html_form_element.cc::PrepareForSubmission()` 在 connected 检查后、
  constraint validation 与 `submit` event 前检查 `WebSandboxFlags::kForms`；因此 click/implicit submit/
  `requestSubmit()` 被拒绝时不会触发 validation、`invalid`、`submit` 或 `formdata`；
- `HTMLFormElement::ScheduleFormSubmission()` 在 `FormSubmission::Create()` 完成 entry-list construction/
  `formdata` 后再次检查 connected，再处理 dialog method，最后对普通 GET/POST 做第二次 `kForms` 检查；直接
  `form.submit()` 绕过第一道 gate，所以仍会触发一次 `formdata`，但不得启动 destination navigation；
- `third_party/blink/renderer/core/loader/form_submission.cc::FormSubmission::Create()` 冻结 action/target、构造
  entry list，并调用 `FrameTree::FindOrCreateFrameForNavigation()`。也就是说，Blink 当前实现会在第二道 forms
  gate **之前**做 target selection；这与“late gate 之前没有任何 target side effect”不是同一合同；
- `services/network/public/cpp/web_sandbox_flags.cc` 将 `allow-forms` 与 `allow-popups`、
  `allow-top-navigation` 分别映射，不能因为允许 form 就允许新窗口，也不能把 forms gate 推迟到 new-context
  popup admission；
- Chromium legacy web tests `fast/frames/form-submission-early-return-for-sandboxed-iframes.html`、
  `fast/frames/sandboxed-iframe-forms.html` 与 dynamic 变体覆盖早门禁、allowed/disallowed form；固定 upstream
  WPT checkout `db95fafd1fcef8428805e41eb5705d444e8c67ce` 没有找到能单独锁住 direct-submit target-selection
  side effect 的 focused case，因此不能把 WPT 关键字快照当作这项语义的强证据；Chromium
  `fast/frames/sandboxed-iframe-parsing-space-characters.html` 另用于锁定 attribute tokenizer 的 FF/VT 边界。

源码审计后又用同一 `out/Default/chrome` 做了最小 headless/CDP target probe：sandbox iframe 设置
`allow-scripts allow-popups`，其中 direct `form.submit()` 投向新 named target。在关闭 popup blocker 后，缺少
`allow-forms` 的运行确实多出一个 URL 仍为空的 page target；加入 `allow-forms` 后相同 target 导航到
`about:blank?#destination`。这证明 Chromium 的 direct-submit denial 可以留下 **creation-only initial empty
popup**，而不是“拒绝前完全没有 target”。

##### typed policy 与唯一 source owner

`DocumentSandboxPolicy` 现在显式保存 `allows_forms`：

- 无 sandbox 时默认 `true`；iframe `sandbox` attribute 只有出现 ASCII case-insensitive `allow-forms` token 才
  为 true；attribute tokenization 统一使用 HTML space characters，U+000C form feed 是 delimiter，Chromium
  明确判为非法组合的 U+000B vertical tab 不会再被 Rust `split_ascii_whitespace()` 错当 delimiter 并授予权限；
- 每个 response CSP policy 若没有 `sandbox` directive 不增加限制；只要任一 active `sandbox` directive
  缺少 `allow-forms`，所有 policy 的交集就为 false；
- creator attribute policy 与 response policy 在 Document build 时继续取交集；Fresh/no-local-proxy Page 的
  E2J typed sandbox carrier 自然携带新字段，尚未删除的 lightweight popup inheritance 也显式做 AND，protocol
  不解析或重算该 flag。

form owner 通过 `owner_dispatch_scope_for_node(form)` 在操作发生时解析 source Document，并读取当前 policy
snapshot：

```text
form node
  -> owner Document
       -> root Page DocumentPolicyContainer
       -> child browsing-context DocumentPolicyContainer
       `-> legacy lightweight popup DocumentPolicyContainer
  -> sandbox.allows_forms
```

这是 source permission，不是 target property。missing/detached owner 不凭空构造 sandbox denial；existing
connected/currentness checks 与后续 navigation owner 仍负责淘汰 detached/stale work。

Moli 当前两条执行链如下：

```text
requestSubmit / trusted activation
  -> connected
  -> source Document allow-forms EARLY GATE
  -> constraint validation
  -> submit event
  -> freeze action/target + construct entry list / formdata
  -> connected recheck
  -> typed target selection
       -> existing target: no creation consume
       `-> miss: popup admission + initial empty Page creation
  -> source Document allow-forms LATE GATE
       -> denied: no destination request
       `-> allowed: attach exact request / navigation

direct form.submit()
  -> connected
  -> freeze action/target + construct entry list / formdata
  -> connected recheck
  -> typed target selection / possible initial empty Page creation
  -> source Document allow-forms LATE GATE
       -> denied: preserve creation, omit destination request
       `-> allowed: attach exact request / navigation
```

因此早门禁不会创建 target、进入 popup blocker 或消费 E2K transient activation；晚门禁会保留 direct-submit
可观察的 `formdata`。E2L.1 之后，existing hit 被拒绝时仍不碰 loader/parser/scheduler；new miss 则保留已准入的
initial empty Page、window-open observation 与 activation consume，但没有 destination request。`formdata` handler
若移除 form，connected recheck 仍先退出；`allow-forms` 允许 existing target submission，但 `_blank` miss 仍必须
独立通过 E2I `allow-popups` 和 E2K blocker/activation transaction。

##### 红测与聚焦证据

最初四条 owner 回归在实现前得到 clean semantic red：

```bash
cargo nextest run -p moli-renderer-v8 -E '<最初四条 allow-forms owner 回归>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 95a8449a-ede2-4d46-814d-ec2155b75469：1 passed / 3 failed。
# requestSubmit + direct submit 实际为 submit|formdata|formdata；response CSP sandbox 仍允许 navigation；
# direct _blank probe 还进入 creation/consume。
```

其中最初把 direct `_blank` denial 写成“绝不创建/绝不消费”的断言，在上述 Chromium probe 后被删除：它会把
Moli 在 E2L 时尚缺 creation-only carrier 的行为误写成浏览器合同。E2L 当时的 owner 回归改为只要求
`requestSubmit()` 的早门禁在 target work 前保留 activation；direct submit 则锁住 `formdata` 后不导航
existing target。最终 10 条 policy/form/tokenizer 回归、更宽的 form/sandbox slice 与 popup/window-open 邻接
slice 均通过：

```bash
cargo nextest run -p moli-renderer-v8 -E '<E2L 十条 form/CSP/attribute/tokenizer owner 回归>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 62754038-2ec1-4d08-838b-7591a2acabea：10 passed。

cargo nextest run -p moli-renderer-v8 -E 'test(form) or test(sandbox)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run fac06157-0d1d-4dcd-b842-6a4a380946f5：327 passed。

cargo nextest run -p moli-renderer-v8 -E 'test(popup) or test(window_open)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 3e746aa3-2dfd-4171-9324-da916cd4a022：113 passed。
```

首轮 workspace gate 让 6 条既有 named-child fixture 稳定变红：这些 fixture 从未把动态 form 接入 Document，
却期待 `form.submit()` 导航。Chromium 的 connected check 会在 entry-list construction 前拒绝这条路径；因此
产品 owner 补了 direct-submit 首道 connected gate，仓库自有 runtime/WPT probe 与本地 Lightpanda upstream
mirror 只做最小 harness 修正——append form 后继续测试原本的 named-target 行为，而不是放宽产品语义：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail \
  --failure-output immediate
# run 21d69b08-63c1-44a4-a84c-05a1eb0a6bfa：16058 passed / 8 failed / 18 skipped。
# 6 条确定性失败均为 disconnected form fixture；另有 parser backlog 与 V8 isolate teardown 两条并发失败。

cargo nextest run -E '<六条修正 fixture + disconnected form owner 回归>' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 4d7cd63c-6989-4693-bf73-4289d45048fb：7 passed。
```

两条并发失败没有触达本轮 form/policy 调用图。按 flaky 规则复核时，parser backlog 单独 30/30 通过（run
`182af909-4653-4cfe-bbbb-c69d762f8542`）；它与 isolate churn 两条直接并跑曾有 5/20 iteration 失败（run
`7a6b6de8-ca05-4444-a6be-24c5ab4468fd`），而仓库既有 parser/file-chooser/isolate 邻接矩阵又 20/20、
每轮 3/3 通过（run `31224cde-45b3-4151-813b-445b39607390`），isolate churn 本身也在前一矩阵中 20/20
通过。这说明失败与并发进程时序有关，不能外推成 E2L 语义证据，也没有用 sleep、retry 或产品并发降级
修改行为。最终精确源码的 workspace gate：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail \
  --failure-output immediate
# run 5421969d-1548-4d69-9d52-2e96e7f24f93：16067 passed、18 skipped；执行阶段 101.445s。

cargo fmt --all --check
# passed。

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 33s。
```

本轮实现提交并首次推送后按 topic 约定同步 master；远端 master 没有新增提交，因此 rebase 是 no-op、没有冲突
或代码 rewrite。仍在同步后的精确 HEAD 上复验 E2L owner 与 workspace gate：

```bash
git pull -r origin master
# Current branch popup-refactor is up to date.

cargo nextest run -p moli-renderer-v8 -E '<E2L 十条 owner 回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 75cbfdfd-4b70-4008-af58-6deea4913fb4：20/20 iterations passed，每轮 10/10。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail \
  --failure-output immediate
# run 9f73b8f6-072e-498b-9581-4c889888cf78：16067 passed、18 skipped；执行阶段 101.632s。

cargo fmt --all --check
# passed。

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 29s。
```

##### E2L 入库时有意保留的边界

E2L 完成的是 source-Document policy 与 actual navigation gate，不把以下内容伪装成已经对齐：

- **E2L.1 creation-only target carrier**：direct `form.submit()` 在 Blink 中先 target selection，再晚门禁；new
  target 可以只留下 initial empty Page，并可能经过 popup blocker/activation consume。E2L 入库时 Moli
  仍在晚门禁前直接返回；该缺口现已由下节 E2L.1 完成；
- E2L 入库时完整 sandbox/top-level `CanNavigate` 属于后续 E2M，不能塞进 form owner；其中 local
  sandbox navigation flags、`allow-top-navigation[-by-user-activation]`、opener/ancestor relation 与
  destination-origin/site exception 现已由 E2M.1 完成，fenced/remote/file-local/embedder 分支仍保留；
- Chromium 的 security console diagnostic 尚未接入，当前只安静拒绝；E2L 入库时 `javascript:` form target 的
  最终 target realm/CSP/currentness 仍走 legacy，full-creator 路径现已由 E2N 收口；
- focused upstream WPT 缺口仍在；后续若新增/找到标准 case，应先确认 WPT driver 的 trusted-input/popup
  lifecycle 能力，再把它升级为提交门禁。

#### Phase 5E2L.1：direct-submit creation-only target carrier

E2L 已经让 late `allow-forms` gate 阻止 destination navigation，但当时的调用链在 gate 失败时直接返回，因而
丢失 Blink 已经完成的 target-selection side effect。不能用一条 synthetic GET `about:blank` 补这个差异：那会
凭空创建 navigation id、history/load 生命周期和第二个 loader owner，也会把“初始空 Document”错误建模成
“目标导航提交”。E2L.1 因此只拆开两个此前被绑死的事实：**请求创建/选择哪个 target**，以及**是否存在要交给
该 target 的 destination request**。

##### 固定 Chromium 合同与本地可观察证据

本轮继续使用 `/home/donoughliu/chromium/src@a03603fe9af6`：

- `HTMLFormElement::ScheduleFormSubmission()` 先调用 `FormSubmission::Create()`，取得其中冻结的
  `TargetFrame()`，之后才执行普通 GET/POST 的第二道 `WebSandboxFlags::kForms` 检查；
- `core/loader/form_submission.cc::FormSubmission::Create()` 在构造 entry list 和 request 后调用
  `FrameTree::FindOrCreateFrameForNavigation()`；existing target 在这里完成选择，miss 则进入
  `CreateNewWindow()`；
- `core/page/create_window.cc` 在创建窗口前以 form action URL 调用 `probe::WindowOpen(...)`，所以 DevTools
  `Page.windowOpen.url` 仍是 action URL；这不表示 destination navigation 随后一定发生；
- E2L 的本地 headless/CDP probe 已证明：`sandbox="allow-scripts allow-popups"` 的 direct submit 指向新
  named target 时会多出一个 URL 为空的 page target；加入 `allow-forms` 后同一 target 才导航到
  `about:blank?#destination`。

由此固定以下不变量：

```text
target selection result         late allow-forms      observable result
existing target                 denied                target 不变；无 NavigateEvent / cancellation / load
new target admitted + created   denied                WindowOpen + initial Page；无 destination request
existing target                 allowed               exact target owner 执行 request
new target admitted + created   allowed               initial Page 后执行 exact replacement request
```

`requestSubmit()` 的 E2L early gate 保持在 validation/submit/formdata/target work 之前，因此不进入上述
creation-only 分支，也不消费 transient activation。只有绕过 early gate 的 direct `submit()`，或 early gate 后
source policy 在 entry-list 阶段发生变化的路径，才可能到达 late decision。

##### renderer target owner 与 typed carrier

`RendererPendingPopupActivation` 不再以 mandatory request 代替整项 popup action，而是显式保存：

```text
requested_url: String
destination_request: Option<RendererTopLevelNavigationRequest>
```

`requested_url` 是同步 target selection 和 `Page.windowOpen` 已观察到的 action URL；只有 `Some(request)` 才
授权 target Page 启动 navigation。`without_destination_navigation()` 产生的不是 GET `about:blank`，而是 typed
no-destination action；GET/POST method、body、headers 与 source/referrer carrier 在 allowed path 仍保持整份。

form target owner 现在以 `ElementPopupDestinationPolicy::SourceFormSandbox` 标识需要 late gate 的请求：

- ordinary named lookup 先解析 current/child/related target。existing hit 后才读取 source form 当前 owner policy；
  denial 在 form-specific target `NavigateEvent`、same-form cancellation 和 loader/parser/scheduler 之前返回；
- miss 与 `_blank` 先走 E2I/E2K admission，完成 blocker decision、activation consume 和 Fresh/Related Page
  reservation 或 renderer-owned initial WindowProxy/Page 创建；之后再读取 source policy；
- form 新建路径同步创建时固定使用 plain `about:blank`，避免 action 本身是 `about:blank#fragment` 时把 fragment
  误提交进 initial Document；allowed path 的 fragment/GET query 仍由后续 exact request 处理；
- denial 仍记录原 action URL 的 `RendererPendingWindowOpenEvent` 和 popup owner action，但后者不含 destination
  request。初始 referrer、name、sandbox policy、session-storage namespace 与 stable WindowProxy reservation 都
  继续属于同一个创建事务。

legacy lightweight name map 目前仍作为兼容 lookup 存在；E2L.1 为它增加 selection-only live-id 查询，denied
existing hit 不再调用会启动 loader 的 `reopen_existing_lightweight_popup_window()`。这不是扩大 lightweight
模型，而是 Phase 6 删除双栈前避免旧 fallback 破坏新 owner 顺序。

##### protocol target owner：空 URL、真实 initial Page、零 navigation work

`PopupTargetCreation` 同样携带 `requested_url + Option<destination_request>`。new creation 的公共 target/attach
事务保持唯一，但两条后续路径明确分开：

- `Some(request)` 继续安装 `Held → Published → Consumed` 的 exact Page-residence navigation claim；
- `None` 把 DevTools target identity URL 设为 Chromium 可观察的空串，同时以 `about:blank` 构造唯一 initial
  Document，并安装 `NoDestination(TargetPageResidenceIdentity)` tombstone；不 capture、stage 或 publish
  `PopupTargetNavigationOwnerAction`。

该 tombstone 很重要：target URL 空串与 internal initial URL `about:blank` 不相等。如果不保留 typed
no-destination authority，后续 `Page.enable`、isolated-world 等通用入口会把差异误判成尚未开始的 initial
navigation，并从 mutable target URL 重建请求。E2L.1 让这些入口稳定返回“不应启动”，同时不制造 fake request。
target discovery、automation lifecycle、tab/page auto-attach 与 initial Page materialization 由共享的
`finish_popup_target_creation()` 完成，因此 creation-only 不是缺事件的旁路。

allowed path 还暴露并修正了一个既有 projection/owner 混淆：form GET 会把空 entry list 序列化为
`about:blank?#destination`，而 DevTools target URL 在 typed claim 执行前就已更新为该 destination。如果
same-document classifier 读取 target URL，它会把“当前 projection 已等于 destination”误判成重复 fragment
navigation；真实 initial Page 仍是 plain `about:blank`，renderer 随后会正确拒绝这条假 same-document 命令。
现在分类只读取已安装 Page 的 `final_url()`，并把带 query/fragment 的 parsed `about:blank` 交给 synthetic
Document loader。这样 allowed form 走真实 initial-Document replacement，denied form 仍保持零 navigation work，
两条路径都不依赖 lightweight popup 预先写入 action URL。

##### 聚焦回归与阶段证据

实现阶段先锁住三层 owner：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(sandbox_without_allow_forms_blocks_existing_target_navigation) or test(sandboxed_request_submit_denial_precedes_popup_activation_consumption) or test(sandboxed_direct_submit_creates_only_the_initial_empty_popup_before_late_denial) or test(sandboxed_direct_form_submit_creates_related_initial_page_without_destination)' \
  --no-fail-fast
# 最终实现复跑 run d20ac427-d134-41a0-ba24-423233a9a8a7：4 passed。

cargo nextest run -p moli-protocol \
  -E 'test(creation_only_popup_keeps_initial_document_without_scheduling_or_url_rescan) or test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request) or test(inline_about_blank_navigation_preserves_query_and_fragment)' \
  --no-fail-fast
# 最终实现复跑 run b0cb8662-2f38-451b-9b5a-56f3c33078bd：3 passed。

cargo nextest run -p moli-renderer-v8 -E 'test(form) or test(sandbox)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# 最终实现复跑 run 0136b84d-6f7d-4e24-ad52-2fe51af1510e：329 passed。

cargo nextest run -p moli-renderer-v8 -E 'test(popup) or test(window_open)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# 最终实现复跑 run eb44d99b-e87a-4e71-a48e-5a15ad2a78f7：114 passed。

cargo nextest run -p moli-protocol \
  -E 'test(popup) or test(same_document) or test(renderer_fragment_navigation_preserves_initial_document_residence) or test(inline_about_blank_navigation_preserves_query_and_fragment)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# 最终实现复跑 run eca682f6-e911-41a2-be26-d9ca4d8d7a16：68 passed。

cargo nextest run -E '<E2L.1 六条 denied/allowed renderer/protocol owner 回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 12fdf757-0618-4623-8d6f-b8ded248bc61：20/20 iterations passed，每轮 6/6。
```

renderer unit 回归证明 `formdata` 已发生、shared transient activation 已被创建事务消费、carrier URL 保留但
request 为 `None`；真实 Page 回归进一步证明 `WindowOpen` URL/name、Related Page reservation、stable initial
`about:blank`、referrer 与 name。protocol 回归证明 `Target.targetCreated.url == ""`、唯一 loaded Page 仍为
initial `about:blank`、scheduler event 为空，并主动调用通用 initial-URL scan 验证不能补出导航；allowed 对照则
证明 target-local authority 仍存在、plain initial Page 最终被 exact `about:blank?#...` request 替换，且 parsed
`about:blank` query/fragment 不会落入网络错误页。

提交前 workspace 门禁为：

```bash
TMPDIR=<repo>/tmp/e2l1-gate cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run c3f9dfbb-e675-4c00-8d4b-718977886365：16071 passed，18 skipped。

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# 均通过。
```

第一次 full-nextest 尝试 run `4746c10b-b985-4f08-adb7-4630e614c8eb` 在系统 `/tmp` tmpfs 已满时启动，首批
99 个失败均为 `ENOSPC` 或其下载落盘连锁超时，运行在 2769 passed 后主动中止；这不是行为门禁证据。没有删除
来源不明的历史 `/tmp/moli-*` 内容，而是把第二次完整运行隔离到仓库文件系统的临时目录；有效 run 全量
通过后已清理本轮 132 KiB fixture。

随后执行 `git pull -r origin master`，成功 rebase 到 `origin/master@815b44cbf0`。本次 master 增量是 TCP
keepalive 与 WebDriver/CDP smoke 诊断 3 个提交，没有修改 popup/form/navigation owner；仍按 Rust 基线执行完整
复验：

```bash
TMPDIR=<repo>/tmp/e2l1-post-rebase cargo nextest run \
  -E '<E2L.1 六条 denied/allowed renderer/protocol owner 回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 9ee5a260-7867-4d82-98a3-732a8876e240：20/20 iterations passed，每轮 6/6。

TMPDIR=<repo>/tmp/e2l1-post-rebase cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 8a453466-abc4-4cc8-93e5-87b64a6afc11：16072 passed，18 skipped。

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# 均通过。
```

两次隔离运行各自只残留 132 KiB 测试 fixture，均在验证后按本轮创建的精确目录清理；系统 `/tmp` 的历史内容
仍未改动。

##### E2L.1 有意保留的边界与下一步

- E2L.1 只对齐 target selection / creation 与 late forms decision，不实现 security console message；
- E2L.1 入库时 sandbox/top-level `CanNavigate` 仍属于 E2M；其中 local token/activation、opener/ancestor 与
  destination relation 现已由 E2M.1 统一，fenced/remote/file-local/embedder fallback 仍需后续 owner；
- `javascript:` form/popup 必须同步完成最终 target 选择/创建，再在该 target Document 的 networking task 中
  异步执行并使用其 CSP/origin/currentness，不能套用本节 HTTP(S)/`about:blank` destination carrier；该
  full-creator 路径现已由 E2N 独立实现；
- `NoDestination` 是 target-local typed tombstone，不是通用 navigation cancellation API；G1 已完成 local
  committed-response COOP group sever 与 disconnected facade，G4 已补 group-qualified endpoint generation 和
  local/disconnected operation currentness；G5 已补 same-group cross-agent top-level scheduler/transport，真正
  cross-process RemoteFrame 仍按其 owner 继续推进。

#### Phase 5E2M.1：renderer-owned local `CanNavigate` authority

E2E 在 named frame resolver 内实现过一份窄的 self / `javascript:` exact-origin / target-ancestor origin
过滤，Location、special target 和 POST form 则各自仍有局部 sandbox 判断。继续在这些调用方追加
`allow-top-navigation` 或 opener 例外会产生多份顺序不同的安全算法。E2M.1 把判断收回
`JsContextHost::can_navigate_browsing_context()`：调用方只提供 committed source execution-context identity、
typed target host/scope 和最终 destination URL，owner 返回稳定 denial reason；选择、报告和真正 scheduling 仍由
各 API 自己负责。

这里的 **local** 指 Moli 当前拥有的 `Top` / `Child(DomHandle)` /
`LightweightPopup(id)`，以及 shared related-agent 中另一个真实 Page 的这些 local endpoint。它不表示本轮已经
虚构出 Chromium 的 `RemoteFrame`、fenced-frame root 或 browser embedder decision。

##### 固定 Chromium 合同与判断顺序

本轮固定对照 `/home/donoughliu/chromium/src@a03603fe9af6`：

- `third_party/blink/renderer/core/frame/local_frame.cc::LocalFrame::CanNavigate()` 从 source==target 开始，随后依次
  处理 `javascript:` exact-origin、sandbox navigation/popup/top flags、普通 target-or-ancestor origin、outermost
  opener relation，以及 child-to-own-top 的 activation/destination/content-setting exception；
- `services/network/public/cpp/web_sandbox_flags.cc::ParseWebSandboxPolicy()` 分别清除
  `kTopNavigation` 与 `kTopNavigationByUserActivation`。因此多个 attribute/CSP policy 必须按 restriction union
  求交，`allow-top-navigation` 与 `allow-top-navigation-by-user-activation` 不能在不同 policy 之间拼成更强权限；
- `content/browser/renderer_host/navigation_request.cc::CommitNavigation()` 先继承 parent policy-container 的
  `can_navigate_top_without_user_gesture`，同源 top commit 放宽；cross-origin child 若 frame owner 没有显式
  `allow-top-navigation` 则收紧。response CSP 即使携带同名 token，也不能伪造 frame-owner provenance；
- destination relation 先比较 target current security origin，再比较相同 protocol 下非空
  domain-and-registry；IP、single-label host 和 public suffix 不进入后者；
- `CanAccessAncestor()` 还包含 file/local-origin compatibility 分支，`LocalFrame::CanNavigate()` 也会发送
  `DidBlockNavigation`、console diagnostic/use-counter。Moli 本轮没有伪装这些尚无 owner 的分支已完成。

本地 authority 按同一顺序实现以下可表达矩阵：

```text
1. source/target generation currentness；stale endpoint fail closed
2. exact same browsing context -> allow
3. javascript: 且 source 不能访问 exact target origin -> deny
4. sandbox navigation：descendant / outermost popup / own top 三类关系
5. own top token：unconditional、transient-only、committed sticky guard
6. 普通 target-or-ancestor origin access
7. outermost source-opener / target-opener-ancestor relation
8. child -> own top：sticky activation、destination exact origin、same-protocol
   registrable site、或显式 allow-without-activation embedder policy
9. 其余 unrelated target -> deny
```

`BrowsingContextNavigationDenial` 只是一组 typed reason，不决定 JS surface。sandbox reason 供 Location 保持
同步 `SecurityError`；ordinary target selection 对任何 denial 都安静跳过 candidate；element/window.open special
target 保留“已选中 existing WindowProxy，但 navigation 被拒绝”的结果，不错误 fall through 到新 popup。

##### committed policy-container，而不是动态 attribute 查询

`DocumentSandboxPolicy` 新增并区分：

```text
sandboxes_navigation
allows_top_navigation
allows_top_navigation_by_user_activation
frame_owner_explicitly_allows_top_navigation
```

attribute parser 与 response-CSP parser 分别生成 restriction，合并时对两类 top token 独立求交；只有 frame-owner
attribute 能设置 provenance bit。`DocumentPolicyContainer` 另持有 inverse
`top_navigation_without_user_gesture_is_restricted`，默认 permissive。每次真实 child Document commit 完成 origin 与
sandbox policy 安装后，target owner冻结：

```text
committed origin same-origin with top     -> unrestricted
cross-origin + owner explicit allow-top  -> inherit parent restriction
cross-origin + no owner provenance       -> restricted
```

比较 committed raw origin 时移除 `document.domain` relaxation，避免脚本事后改变 policy-container decision；动态
iframe `sandbox` mutation也不会重写已提交 Document 的 bit，只影响下一次 frame policy refresh/commit。网络成功、
initial/local bootstrap、failed-start fallback 与 `javascript:` string-result replacement 的 commit 入口都调用同一
freeze boundary。

Fresh/legacy popup 的 propagated sandbox record 同步携带 navigation/top flags。response CSP 仍只能收紧；其
`allow-top-navigation` 不会变成 frame-owner exception。`moli-site` 作为 renderer 的直接依赖，只用于
Chromium domain-and-registry fallback，并显式排除 IP、localhost/single-label 与 public suffix。

##### stable endpoint、source base 与调用入口

opener relation 不再通过 V8 wrapper object identity 猜测：

- 真实 related top Page 从 stable WindowProxy 解析 target host + dispatch scope；
- compatibility lightweight popup 从 `LightweightPopupBrowsingContextRecord.opener` 的 typed
  `PendingWindowMessageEndpoint` 读取 exact source scope；
- source lightweight popup 导航其 own typed opener 也走同一 outermost relation；
- source identity 与 target current owner 在判断前复核，closed/replaced endpoint 不会被 name lookup 复活。

跨源 Location 的 relative URL 另修正一项因果边界。Blink 按 entered/source Window 的 API base URL 解析，而不是
target Location 的 Document base。cross-origin observer 现在从 stable
`WindowExecutionContextIdentity` 取得 Top/Child/Lightweight source Document base，先解析成绝对 URL，再将**同一个**
URL交给 `CanNavigate` 与 target queue。这样 target-origin/site exception 不会批准 A，而真正提交 B。

E2M.1 已接入的生产入口如下：

| 入口 | authority 接点 | refusal surface |
| --- | --- | --- |
| ordinary named `window.open()` / hyperlink / form | current subtree、current Page、ordered related Page 的每个 candidate-local filter | 跳过不可导航 candidate；miss 再按既有 creation policy 处理 |
| `_self` / `_parent` / `_top` element 与 `window.open()` | final selected WindowProxy 解析出的 host/scope | existing target 保持 selected；静默不 scheduling、不创建替代 popup |
| same-origin/general Location setter | incumbent source realm + Location target owner | sandbox reason 抛 `SecurityError`；其他 refusal 静默终止 |
| cross-origin Window/Location membrane | callback-scoped observer identity + receiver-owned target endpoint | sandbox reason 抛 `SecurityError`；stale/unrelated fail closed |
| POST form current top | source form owner Document + top target | late policy refusal 视为 handled，零 Page navigation request |

GET/special form 已复用 element special-target authority；ordinary named GET/POST 早已由 E2F resolver 消费。self child
navigation、browser/CDP navigation 与 Service Worker `Client.navigate()` 是 target/browser 自身入口，不把它们
错误套成另一个 Document 发起的 named-target `CanNavigate`。

##### 聚焦证据

实现阶段先用三条 red-to-green owner 回归锁住最容易被 wrapper/marker 掩盖的边界：

```bash
TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 \
  -E '<frame-owner/CSP token intersection | cross-origin source-base relative Location | typed lightweight opener>' \
  --no-fail-fast
# run 1acdcf89-6093-4fa6-9267-c9d17b11ab9d：3 passed。
```

随后把 10 条新增 policy/entry tests 与 7 条已有 named/form/special-target/lifecycle 回归组成最终
owner-focused slice：

```bash
TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 \
  -E '<E2M.1 10 条新增 + 7 条既有 owner 回归>' --no-fail-fast
# run 431a1261-bb2b-4b27-b691-f93f0d3dd0ee：17 passed。

TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 sandbox --no-fail-fast
# run 98e7feea-44c5-4adf-ac82-7b06e943f311：44 passed。

for iteration in $(seq 1 20); do
  TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 \
    -E '<同一 17 条 owner-focused 回归>' --no-fail-fast
done
# first run 9f54b8fc-5bc1-4570-b0f5-0982a0baf21c，
# final run 8a7ef587-cf72-45d7-8ebb-2f862d145232；20/20 轮均为 17 passed。
```

destination matrix 覆盖 exact target origin、same registrable site、unrelated site、scheme mismatch、sticky activation、
IP different port、localhost different port、public suffix different port，以及 source `<base>` 下相对 URL 的
allow/deny。sandbox matrix覆盖 no token、activation token、有/无 transient activation、unconditional token、
committed attribute mutation、frame-owner/CSP provenance、special-target 四入口和 non-descendant named sibling。

第一次提交前全量 run `1b4dfe89-d49f-4e2e-a5e6-dcf95dc9a8e5` 是 16081 passed、1 failed、18 skipped；唯一
failure 是旧 `window_open_about_blank_returns_lightweight_popup_window` 在先调用 `popup.close()`、同步退休 legacy
Document owner 后，仍期待后续 Location fallback 改写 facade URL。中央 currentness authority 把它识别成
`StaleContext`，没有再复活已关闭 record。对照 Blink `DOMWindow::close()` 的 `Page::CloseSoon()` 与
`Location::SetLocation()` 的 attached-frame guard，真实 Page 在 deferred final teardown 前仍可被观察；而本测试用的
standalone lightweight callback 已执行 final teardown。fixture 因此改为先验证 live popup navigation，再验证
`close()` 返回/shape，并确认 close 后的第二次 setter 不会复活 owner，未放宽 stale-owner 判定。精确回归 run
`fdf1972d-88ee-461d-8f87-761ec2941c58` 为 1 passed。

##### E2M.1 有意保留的边界与下一步

- `RemoteFrame`、fenced-frame root、remote embedder decision 与 activation replication 尚无本地 target kind，本轮
  明确不以 `Top` 分支冒充；
- Chromium file/local-origin descendant compatibility 尚未建模；当前 origin owner 对 file opaque/local identity
  的表达仍需先统一，不能在 `CanNavigate` 调用方硬编码 URL scheme；
- popup blocker 的 `AllowWithoutTransientActivation` 目前是 browser-context policy 对 Chromium
  `DocumentLoader::ContentSettings::allow_popup` 的显式本地映射，不等于已经实现 content-setting/CDP 配置面；
- transient/sticky ledger 当前是 local Page/frame-tree 聚合 owner；RemoteFrame/per-frame replication、history
  activation 与 visibility 长尾仍需独立补齐；
- security console message、`DidBlockNavigation` reason、use counter 尚未接入；API refusal 只有既有异常/静默表面；
- central authority 已拒绝 cross-origin `javascript:` candidate；E2M.1 入库时最终 target 的同步选择、
  target-Document 异步 task execution、CSP/origin/currentness 仍走 legacy，现已由 E2N 完成；
- `navigate_named_iframe_target_from_document()` 只剩 current-host compatibility wrapper，生产
  window.open/hyperlink/form resolver 已不依赖其权限判断；Phase 6 删除双栈时一并移除，不能继续扩展；
- COOP browsing-context-group sever、disconnected/remote WindowProxy endpoint 仍按后续 owner 阶段推进；
  本地真实 Page 的 close/unload transaction 已由 Phase 5L1 完成，local focus transaction 已由 Phase 5L2 完成。

#### Phase 5E2N：`javascript:` final-target Page task / realm

E2M.1 已能拒绝 source 无权导航的 cross-origin target，但 production `window.open()`、hyperlink 与 form
此前仍把 `javascript:` 当成“不能进入真实 Page migration”的特殊 scheme。新建 target 可能继续得到
opener-host lightweight record；existing related target 则可能把 URL 留给 protocol 或 legacy facade 解释。
这会同时违反三个已经建立的不变量：最终 target realm 不是执行 owner、protocol 可能把 renderer-only URL
当作 browser navigation、旧 Document task 可能在 replacement realm 中复活。

E2N 不新建第三套 JavaScript URL executor。它把请求交给已经由 child-frame/stable WindowProxy 纵切成熟的
真实 Page/Document owner，并把“同步 target selection”与“异步 script execution”明确拆开。

##### 固定 Chromium / WPT 合同：同步选 target，异步执行 task

本轮继续固定 `/home/donoughliu/chromium/src@a03603fe9af6`，并校正了旧章节中“JavaScript URL 在 target
realm 同步执行”的不准确表述：

- `third_party/blink/renderer/core/frame/local_dom_window.cc::LocalDOMWindow::open()` 先通过
  `FindOrCreateFrameForNavigation()` 同步选中或创建最终 `Frame`，随后才对该 frame 调用 `Navigate()`；因此
  `window.open()` 的返回值和 stable WindowProxy 可同步观察，但脚本结果不可同步观察；
- `third_party/blink/renderer/core/loader/frame_loader.cc::FrameLoader::StartNavigation()` 对 current-tab
  `javascript:` 不进入 browser/network loader，而调用最终 target
  `Document::ProcessJavaScriptUrl()`；new-context/no-opener 且 opaque cross-origin 的分支会故意不执行脚本；
- `third_party/blink/renderer/core/dom/document.cc::Document::ProcessJavaScriptUrl()` 把 URL 追加到
  `pending_javascript_urls_`，并只发布一个可取消的 `TaskType::kNetworking` task；
  `ExecuteJavaScriptUrls()` 在 task 开始时 swap 出当时的 FIFO batch。多个已排队 URL 都执行；若前一条只启动
  provisional navigation，batch 继续；若 Document commit/detach 使旧 `Document` 失去 frame，则停止；
- `Document::CancelPendingJavaScriptUrls()` 在 Document replacement/teardown 清掉未选中的 task；
  `HTMLFormElement` 对后来接受的普通 form submission 还会显式取消 target Document 的 pending JS URL 与
  client navigation；
- `third_party/blink/renderer/bindings/core/v8/script_controller.cc::ExecuteJavaScriptURL()` 使用 target Window 的
  base URL、CSP 和 Trusted Types pre-navigation check，在 target realm 执行。异常/空结果/非字符串不替换；
  字符串 completion 只有在**本次执行**没有启动 navigation 时才 clone/commit replacement Document；
- upstream WPT
  `html/browsers/browsing-the-web/navigating-across-documents/javascript-url-task-queuing.html`
  显式断言 `window.open("javascript:...")` 和 opened-window Location 两种入口都不能同步执行。

为确认容易从源码误读的 FIFO/cancellation 顺序，本轮还用同一 checkout 的
`out/Default/chrome`（Chrome 147.0.7709.0，WebKit revision `a03603fe9af6`）跑了临时 CDP 最小探针；它不是
Moli 提交门禁，但结果与上述 owner 一致：

```text
同一 target 连续 JS URL       -> queued-first, queued-second
JS URL 后接 ordinary window  -> JS 不执行
JS URL 后接 ordinary link    -> JS 不执行
JS URL 后接 ordinary form    -> JS 不执行
第一条 JS 内启动 navigation  -> first, second；batch 继续
第一条 JS 返回 string        -> first；replacement 退休 second
第一条 JS throw              -> first, second；异常不退休 batch
ordinary navigation 后接 JS  -> 后来的 JS task 仍执行
```

##### 最终 target 的单向 owner 链

E2N 的 production 链路为：

```text
source window.open / hyperlink / form
  -> source CSP + E2M.1 CanNavigate
  -> synchronous final target resolution/creation
       existing current/child/related Page
       or one staged Related initial Page
       or one private Fresh Page
  -> Related target JsContextHost pending queue
       {handoff, source_document, target_document, navigation_source, URL}
  -> explicit cross-Page target-owner wake
  -> popup activation {exact Page residence, destination request = None}
  -> protocol only adopts/reuses target; never loads javascript: URL
  -> target Page networking owner snapshots current Document FIFO batch
  -> target Document currentness + target CSP
  -> execute in target default realm
  -> optional string-result Document replacement on the same stable Page
```

具体责任边界如下：

- `open_lightweight_popup_window()` 的 real initial Page staging 不再排除 `javascript:`。full-creator 的 Related
  miss 因而同步得到真实 initial `about:blank` Page/realm/Document，返回给 opener 的就是 protocol 随后 adopt
  的 stable WindowProxy；existing related hit 直接解析 target Window 的 `JsContextHost`；
- `window_open_callback()` 与 element auxiliary selector 都把完整
  `RendererTopLevelNavigationRequest`（包含 E2H source carrier）直接写入最终 target host。popup activation 对
  `javascript:` 强制 `NoDestination`，所以 URL 不会再由 protocol/browser 创建第二次 navigation；
- `NoDestination` 与 DevTools target-creation URL 是两个 typed fact：JavaScript popup 不发布 browser destination，
  但新建 target 仍按 Chromium `Target.targetCreated` 报告 requested `javascript:` URL；late-denied creation-only
  form 则继续报告空 URL。protocol 只消费 renderer 冻结的 projection flag，不从 scheme 反推执行策略；
- existing Related Page 的写入发生在 source turn，不能借用 target host 的 ordinary-turn active flag。E2N 因此为
  exact handoff 显式发布 target-owner wake；尚未 resident 的 staged initial Page 即使先收到一次早期 wake，adoption
  仍会从同一 pending queue 发现请求，既不丢 task，也不制造第二个 owner；
- hyperlink 和 form 复用同一 typed related Page resolver。form 现在在 entry-list construction 与 connected
  recheck 后也执行 source-Document inline-navigation CSP，`_self`、ordinary name 与 `_blank` 不再绕过 source
  check；最终 target 在执行 task 时再做自己的 CSP check；
- `PendingLocationNavigation` 新增 `target_document`。它与 initiator `source_document` 是两个独立 identity：前者
  决定 task 是否仍属于 current target Document，后者继续保存因果/referrer carrier，不能互相替代；
- `JsContextHost` 的 ordinary/history navigation 保持 replace-only；同一 target Document 的 JavaScript URL
  改为 FIFO queue。owner 选择 networking turn 时一次冻结当时的 JS batch，后来接受的 ordinary navigation
  清空整批未选 task；已经选出的 batch 则按 Chromium 顺序执行；
- 每个 batch item 执行前复核 `target_document`。task 排队后发生 `document.open()`、string-result commit 或其它
  Document replacement 时，旧 item 只完成 cancellation，不进入 replacement realm；
- string completion 的 currentness 不再读取“执行后是否存在任意 pending navigation”这一模糊布尔值，而是比较
  执行前后的 exact navigation handoff。只有本次脚本新建 handoff 才忽略 completion；batch 中更早脚本已启动的
  navigation 不会误判为本次脚本启动；
- JS exception 被 target realm 正常报告，但不会让 Page owner command 失败或替换 Document；non-string completion
  也只完成 task。string completion 使用既有 `JavascriptDocumentReplacement` lifecycle/commit owner，保持 stable
  WindowProxy 与 Page residence；
- existing Related target 随后接受普通 `window.open()` / hyperlink / form navigation 时，调用方不复制 loader
  cancellation，而只通知 target host 清掉它自己的 pending JS queue。实际 ordinary request 仍由 protocol/target
  loader 的唯一 owner 消费；
- suppress-opener 的 `_blank`/ordinary-name JavaScript URL 仍 reserve 一份真实 Fresh Page，并返回 `null`。由于
  没有 opener relation、initial Document 是独立 opaque origin，activation 没有 destination，creator 脚本不会
  被错误注入 Fresh realm。这与 Blink `FrameLoader` 的 intentional no-op 分支一致。

##### 回归与当前证据

新增/扩展的 owner 回归覆盖：new Related initial target、existing named reuse、WindowProxy stable identity、
window.open/hyperlink/POST form 三类 producer、同步返回/异步执行、source/target CSP、exception/non-string/string
completion、执行中启动 navigation、FIFO batch、ordinary supersession、Document replacement cancellation、
Fresh noopener no-execution，以及 protocol activation 的 exact residence / `NoDestination`。

```bash
TMPDIR=<repo>/tmp/e2n-build cargo nextest run -p moli-renderer-v8 \
  javascript_popup_producers_queue_the_final_related_target_page_realm --no-fail-fast
# run 5aeecb11-4910-41a6-806e-79e323a6e9f2：1 passed；含无需外部 target command 的
# cross-Page owner wake，以及 Window 与 element ordinary supersession。

TMPDIR=<repo>/tmp/e2n-build cargo nextest run -p moli-renderer-v8 javascript \
  --no-fail-fast
# 最终 run f7b0d01d-7deb-4acf-8f34-e4994ce11583：32 passed。

TMPDIR=<repo>/tmp/e2n-build cargo nextest run -p moli-renderer-v8 \
  -E '<同一 4 条核心回归>' \
  --stress-count 20 --flaky-result fail --test-threads 4
# 最终 run e97c108f-3e18-424f-bd92-b8cf7868a04f：20/20 iterations passed，每轮 4/4。

TMPDIR=<repo>/tmp/e2n-build cargo nextest run -p moli-protocol \
  rust_cdp_chromium_target_window_open_javascript_url_still_reports_popup_target \
  creation_only_popup_keeps_initial_document_without_scheduling_or_url_rescan \
  --no-fail-fast
# run 9f080bb7-9886-47fc-8bc1-83b15adaf9b7：2 passed；锁住 requested URL projection
# 与 destination execution authority 的分离。
```

本轮阅读了 upstream WPT 源码，但没有把临时 Chromium CDP probe 或旧
`wpt-cross-current` 关键字快照伪装成 Moli WPT pass。最终 full workspace gate 与 rebase 后证据记录在本文
统一验证章节。

##### E2N 有意保留的边界与下一步

- E2N 提交时尚缺 target `CheckAndGetJavascriptUrl()` 的 Trusted Types pre-navigation check、form 的 Chromium
  compatibility `form-action` check，以及 new/existing target 的 source policy 分岔；这些历史边界现已由 E2O
  完成，不能再作为当前缺口；
- E2N 只迁移有完整 renderer creator/target capability 的 DOM producer。它提交时保留的 Service Worker
  `clients.openWindow()` 与 notification navigation 已由 E3 单独迁移为 browser-context source + Fresh Page，
  没有伪装成 root Window 的 source Document；
- already-published ordinary destination 后同一 source turn 又排队 JS URL 的 renderer/protocol causal ordering
  已由 E3 的真实 protocol apply/commit integration 回归补齐；
- G1 后 local committed-response COOP group sever 已进入 Page replacement owner。该阶段尚不能表达的 related
  remote endpoint 与 isolated-world source 已由 G5/G6 和 P6R8 接入 typed route。fenced target 与真实
  cross-process scheduler 仍是独立基础设施边界；
- E2N 提交时 legacy lightweight JS executor 仍服务 standalone fixture/compatibility fallback。P6R4 已沿依赖顺序
  删除该 owner，tracked Rust 的宽口径旧模型扫描也已归零；
- E2O 后规划的非 DOM creation producer 与 focused protocol evidence 已由 E3 完成，local close/unload 又由 L1
  完成。后续 focus、group/remote 与 identity/lifetime 中，local owner 已依次由 L2、G1-G6、P6R2/P6R3 收口；
  当前进入 Phase 6 compatibility reachability 与依赖层删除。

#### Phase 5E2O：target Trusted Types、`form-action` 与 source-selection ordering

E2O 不是在 producer 上叠加三个独立布尔门禁。它补齐的是 E2N target task 两端的 policy transaction：

```text
existing target hit
  -> resolve exact Page/Frame + stable WindowProxy
  -> source Document inline-navigation CSP
  -> target-owned navigation task
  -> target Document CSP
  -> target Realm Trusted Types pre-navigation check
  -> execute rewritten/original source in that target Realm

new-context miss
  -> source Document CSP
  -> source Realm Trusted Types creation preflight
  -> auxiliary admission + exact initial Page/Document creation
  -> target-owned navigation task
  -> target Document CSP + target Realm Trusted Types
  -> execute only if the target Document is still current
```

existing hit 的 source Trusted Types **不能**提前运行；最终选中的 target realm 才决定 default policy、CSP
reporting realm 和脚本字符串。new-context miss 则不同：Blink 在创建 auxiliary context 前先用 source
`CheckAndGetJavascriptUrl()` 做一次 non-empty gate，因此 source default policy 可以允许或拒绝创建，但其 rewrite
不是 target 的执行字符串；target task 仍会用 target policy 检查原始 URL。这个差异也决定了 API 返回：source CSP
拒绝 existing named target 时，`window.open()` 仍返回已选中的 stable WindowProxy（`noopener` 则仍返回
`null`），且不能落入 new-popup fallback；真正的 miss 被 source CSP/Trusted Types 拒绝时才返回 `null` 且完全不
创建 target。

##### Chromium `a03603fe9af6` 的调用顺序证据

| Chromium owner | 固定基线位置 | E2O 采用的事实 |
| --- | --- | --- |
| source inline check | `core/frame/local_dom_window.cc:514-535` | `AllowInlineJavascriptUrl()` 只做 source CSP，并明确说明 Trusted Types 留给最终 `ExecuteJavaScriptURL()` |
| full pre-navigation check | `core/frame/local_dom_window.cc:538-572` | `CheckAndGetJavascriptUrl()` 先 source/target CSP，再执行 Trusted Types pre-navigation check，返回可能改写或为空的 source |
| new auxiliary creation | `core/page/create_window.cc:275-298` | `CreateNewWindow()` 在任何 window creation 前对 `javascript:` 调 source `CheckAndGetJavascriptUrl()`，empty 直接失败 |
| lookup/create split | `core/page/frame_tree.cc:202-226` | 先 `FindFrameForNavigationInternal()`；只有 miss 才进入 `CreateNewWindow()`，existing candidate 不走 source TT creation preflight |
| existing navigation | `core/loader/frame_loader.cc:545-562` | 已选中的 frame 在 `AllowRequestForThisFrame()` 中只调用 origin/source Window 的 `AllowInlineJavascriptUrl()` |
| final target execution | `bindings/core/v8/script_controller.cc:248-282` | target Window 在 execution turn 调自己的 `CheckAndGetJavascriptUrl()`，并以 target base URL/realm 执行 |
| Trusted Types helper | `core/trustedtypes/trusted_types_util.cc:301-380,841-847` | navigation 使用 dummy exception state；default `createScript` 参数为 `TrustedScript` / `Location href`；invalid reconstructed `javascript:` URL 按 enforce/report-only 决定 block/continue |
| `window.open()` return/opener | `core/frame/local_dom_window.cc:2396-2450` | selection/create 先完成，`Navigate()` 后 special target 总返回 Window；ordinary existing target 即使 `noopener` 返回 null，也不会变成 creation miss |
| form target selection | `core/loader/form_submission.cc:372-394` | `FormSubmission::Create()` 已执行 `FindOrCreateFrameForNavigation()` 并冻结 exact target |
| form late policy/schedule | `core/html/forms/html_form_element.cc:741-824` | selection 后才复核 connected、dialog/action、`allow-forms` 与 JavaScript compatibility `form-action`；target 存在才进入 navigation/scheduler |

对应的 upstream WPT source 包括 `trusted-types/navigate-to-javascript-url-001..005,008.html`、
`trusted-types/trusted-types-navigation.html`、
`content-security-policy/form-action/form-action-src-javascript-blocked.sub.html`、
`form-action-src-javascript-prevented.html` 和
`content-security-policy/script-src/javascript-window-open-blocked.html`。这些文件用于确定行为矩阵；本轮仍不把
“阅读 upstream source”写成 Moli WPT pass。

##### Moli owner 与实现

- `trusted_types.rs` 新增 navigation 专用 string conversion。它在当前 entered target realm 调 default
  `createScript(value, "TrustedScript", "Location href")`，消费 callback exception；enforce 返回 `None`，
  report-only dispatch violation 后继续原 source，成功 rewrite 则先验证重建的 `javascript:` URL。navigation API
  不同步抛出 default-policy exception；
- E2N 的 real target Page queue 没有另起 executor。`ScriptVm` 在选中 task 后先用 current target Document 做
  CSP，再在 target default context 做 Trusted Types，PageVM 只执行返回的 rewritten source。普通 navigation、
  target Document replacement 和 FIFO currentness 继续复用 E2N owner；
- generic child 不借 top realm 代检。`FrameScriptJobKind::JavascriptUrl` 在 exact stable child FrameRealm 中依次
  做 child effective CSP 与 Trusted Types，并把 rewrite 写回该 job；child response/meta enforced + report-only
  policy 与 violation event 也从 child owner 读取。这正是复用既有 child-frame stable WindowProxy/realm 基础的
  价值：policy、global、base URL、Document lifetime 与执行 realm 天然指向同一个 owner；
- `window.open()`、hyperlink 和 form 都先用 typed resolver 选 existing target。existing Page/child、special
  target 与 live lightweight compatibility target 在 selection 后运行 source CSP；denial 只阻止 target queue，
  不触发 popup admission、activation consumption 或 fallback creation。related existing top 仍保留 Chromium 的
  existing-opener update；
- resolver miss/`_blank` 才运行 source `CSP → Trusted Types` creation preflight，并且发生在 auxiliary admission、
  Page reservation、`Page.windowOpen` 和 initial Document creation之前。source default-policy rewrite 只作为 non-empty
  admission fact，不会污染 target task source；
- `form-action` 进入 source Document policy owner，覆盖 top、stable child 和 compatibility popup，并同时 dispatch
  report-only/enforced violation。directive 不回退到 `default-src`，也不套用 script nonce/`strict-dynamic` 规则；
- ordinary named form 的顺序固定为 `target selection → connected/allow-forms → source JS CSP → form-action → exact
  target scheduler`。new target 的 creation preflight 和 initial Page 已发生在 late allow-forms/form-action 之前，
  所以 late denial 继续保留 E2L.1 creation-only target，但不产生 destination loader/parser work；
- `form-action` 拒绝不会先取消同一 form 已有的 pending target。只有新 submission 真正进入 target scheduling 后，
  E2G 的 typed Page/Frame generation 才取消旧 task/loader/parser，避免 policy-denied submission 误杀既有导航。

##### 回归与当前证据

新增/扩展的回归覆盖以下矩阵：target enforce/no synchronous throw、target default-policy rewrite、report-only
default-policy exception continuation、invalid reconstructed URL、stable child target realm、source TT 只阻止 miss、
source CSP 命中 existing target 仍返回相同 proxy 但不 queue、`_self` return identity、form-action 无
`default-src` fallback、preventDefault 不触发 policy、existing/new creation-only order，以及 E2N real related Page
producer handoff。

```bash
cargo nextest run -p moli-renderer-v8 \
  popup_policy_checks_keep_existing_and_new_target_order_distinct \
  window_open_javascript_url_source_csp_preserves_the_selected_self_target \
  new_javascript_popup_uses_source_trusted_types_only_as_a_creation_preflight \
  iframe_javascript_url_uses_the_stable_child_target_trusted_types_policy \
  form_action_csp_runs_after_new_target_selection_and_skips_prevented_submission
# run 30aa06df-c30f-4862-b85f-23bf5b9401f1：5 passed。

cargo nextest run -p moli-renderer-v8 \
  javascript_location_navigation_enforces_target_trusted_types_without_throwing \
  javascript_location_navigation_uses_the_target_default_policy_rewrite \
  javascript_location_navigation_report_only_default_policy_exception_continues_original \
  form_action_csp_runs_after_new_target_selection_and_skips_prevented_submission \
  popup_policy_checks_keep_existing_and_new_target_order_distinct \
  iframe_javascript_url_uses_the_stable_child_target_trusted_types_policy
# run 574ba472-336a-4404-b6ac-4b35049c7135：6 passed。

cargo nextest run -p moli-renderer-v8 \
  javascript_popup_producers_queue_the_final_related_target_page_realm \
  sandboxed_direct_form_submit_creates_related_initial_page_without_destination \
  form_javascript_url_csp_checks_the_source_document_before_target_selection \
  iframe_javascript_url_string_completion_replaces_child_document \
  submitter_cancels_previous_same_form_navigation_in_a_related_page_child
# run 32901ca0-40d6-4edd-8dbc-d6d651efbe14：5 passed。

cargo clippy -p moli-renderer-v8 --all-targets -- -D warnings
# passed。

cargo nextest run -p moli-renderer-v8 \
  javascript_navigation_default_policy_rejects_an_invalid_reconstructed_url \
  form_action_is_navigation_specific_and_has_no_default_src_fallback \
  report_only_default_policy_transforms_or_preserves_by_callback_outcome \
  rejected_default_policy_reports_both_dispositions_and_enforces_once
# run 64f85d7d-1fc8-4245-8e7a-790d223a7fb4：4 passed。
```

##### E2O 检查点当时保留的边界

- Service Worker `clients.openWindow()`、notification navigation 等非 DOM producer 当时仍显式进入 lightweight
  owner；E3 已按这一边界把它们改成 browser-context source + Fresh Page，没有复用 root Window 作为假 source；
- Moli 对 ordinary form scheme 也在 renderer 做 `form-action`，因为当前没有 Chromium browser-process
  navigation policy continuation。redirect chain、response-stage URL override 与每 hop 的 form-action 语义仍需随
  browser/network navigation policy owner 补齐；
- isolated-world source 与 related remote top/child target 已由 P6R8 接入 source-qualified typed route。
  Moli 当前只有 Document 级 CSP 与 Trusted Types policy，尚未模拟 Chromium extension world 的独立 policy；
- E2O 提交时 legacy lightweight executor 仍服务 standalone fixture 和 compatibility fallback。P6R3/P6R4
  随后补齐真实 realm lifetime 并删除该 executor；
- already-published ordinary destination 后同一 turn 又排队 JavaScript URL 的 protocol apply/commit integration
  已由 E3 完成；focused upstream WPT slice 和 CSP reporting endpoint 网络上报仍是外部证据债。

#### Phase 5E3：非 DOM producer、精确 `openWindow()` terminal 与 local creation-policy exit

E3 不是把 SW API 名字改成另一个 popup helper。它统一的是三个此前分离的责任边界：谁创建 Page、谁持有
navigation 的因果 source，以及谁有权完成 worker Promise。阶段验收目标为：

1. DOM、Service Worker 与 notification 的 production auxiliary creation 都交给真实 renderer Page；
2. 非 DOM producer 不制造 source Window/Document，不暴露 opener，也不进入 related-name lookup；
3. `clients.openWindow()` 只由 exact reserved Page 的最终 navigation terminal 完成一次；
4. 普通 popup navigation 的 commit 不得越过同一 target Page 已经排队的 JavaScript URL task；
5. SW/notification production caller 不再直接调用 lightweight owner。Phase 6 的 compatibility facade 删除不在
   这项 exit criterion 内。

##### Chromium `a03603fe9af6` 的非 DOM owner 证据

| Chromium owner | 固定基线位置 | E3 采用的事实 |
| --- | --- | --- |
| Blink URL/activation gate | `third_party/blink/renderer/modules/service_worker/service_worker_clients.cc:229-264` | `openWindow()` 以 worker location 解析 URL，执行可显示性与 window-interaction gate，消费 interaction 后通过 ServiceWorker host 请求新 tab；没有 renderer Window opener |
| browser canonicalization/security | `content/browser/service_worker/service_worker_version.cc:2070-2111` | browser 把任意 accepted `about:` canonicalize 为 `about:blank`，再次执行 process URL 权限检查，再进入统一 `OpenWindow()` |
| browser creation source | `content/browser/service_worker/service_worker_client_utils.cc:498-550` | 新 WebContents 使用 worker script URL 生成 referrer/initiator origin，并标记 service-worker open-window；不是借当前 tab 的 root Document 充当 initiator |
| commit/current client lookup | `content/browser/service_worker/service_worker_client_utils.cc:410-447,698-748` | navigation 必须先 commit；随后按 RenderFrameHost 查 exact WindowClient，等待 execution-ready；Page 已销毁或 client 不可见时成功返回 null client |

Moli 保留自身无 browser process 的结构差异，但对齐同一可观察 owner 语义：renderer browser-context runtime
持有 Page/client registry，protocol target admission 仍是唯一 browser owner，worker runtime 只接收最终 typed
completion。

##### 统一 producer transaction

`record_service_worker_auxiliary_navigation()` 现在是两个非 DOM producer 的共享入口。它完成以下不可拆分事务：

- 用 worker script URL 和 destination 冻结 `RendererTopLevelNavigationSource::BrowserContext`、network referrer 与
  destination `document.referrer`；initial empty Document 的 referrer 为空；
- reserve 一个 `RendererScriptAgentAdmission::Fresh` Page，选择 `FreshUnnamed`，携带 default/non-sandboxed
  auxiliary frame policy，不产生 popup id、Window opener 或 session-storage clone；
- `http` / `https` 原样进入 target Page，所有 `about:` canonicalize 为 `about:blank`；SW 的其他 scheme 返回
  `TypeError`，notification action 则不创建 target；
- notification navigation 到这里结束；`clients.openWindow()` 额外安装 move-only continuation，后续不能从 URL、
  active target 或 target-name projection猜测完成对象。

因此三个 producer 的差异只保留在必要的 source/policy 表面：

| producer | causal source | group/opener | completion |
| --- | --- | --- | --- |
| Window/hyperlink/form | exact source Window/Document | Related 或 policy-selected Fresh；按既有规则暴露/切断 opener | 同步返回 stable WindowProxy 或 `null`，destination 异步 |
| SW `clients.openWindow()` | worker script browser-context source | 强制 Fresh unnamed；无 opener | exact worker Promise continuation |
| notification navigation | notification/worker script browser-context source | 强制 Fresh unnamed；无 opener | fire-and-forget owner action |

##### `clients.openWindow()` 的 exact terminal

continuation 同时冻结 `expected_page_id + request_id + source_version_id + source_generation`，并用共享 atomic once
状态保证 clone、错误分支与 Drop fallback 只结算一次。它从 activation 移入 target-local navigation action，再随
`NavigationDispatchState` 穿过 request/response Fetch interception、redirect、background load 和最终 commit；
target slot 中长期保留的 claim tombstone不持有 Promise authority，避免 Page lifetime 反向保活 worker request。

最终状态映射为：

| navigation/Page terminal | worker 可观察结果 |
| --- | --- |
| exact reserved Page commit，client 仍 current、execution-ready 且与 worker script same-origin | 返回该 Page 的 `WindowClient` snapshot |
| Page id mismatch、client 已消失/冻结、尚未 execution-ready、cross-origin 或 `about:blank` | Promise 成功 resolve `null` |
| 204/205、download、transport/fetch failure 未形成可暴露的 same-origin `WindowClient`（包括只提交 error Document），或 authority 在途中被丢弃 | Promise 成功 resolve `null` |
| URL/activation/host 在创建前被 API gate 拒绝 | 保留对应 `TypeError` / `InvalidAccessError`，不创建 Page |

这里“请求失败后 null”不是用当前 target 扫描补结果：unresolved carrier 的 Drop 也只向独立 completion queue 写入
typed null，queue 唤醒 exact ServiceWorker owner lane；source version/generation 过期时由 worker runtime 拒绝旧
completion 修改新 worker execution。

##### ordinary → JavaScript URL 的跨 publication 因果顺序

协议回归最初证明一个容易忽略的事实：同一 source V8 turn 内连续的 ordinary `window.open(url, name)` 与
`window.open(javascript:..., name)`，会因跨 Page handoff 分裂成两个 renderer publication。仅在单批 activation
内回看或按 URL/name 配对都不成立。

E3 把不变量放到 destination owner：每个 Window-origin ordinary popup navigation action 都携带
`drain_pending_javascript_tasks_before_commit`。protocol 先启动普通 browser navigation并发布 provisional start，
然后在该 cross-Document navigation 尚 suspended、background completion 尚不能被同一 actor 应用时，通过
interruptible exact-Page access 派发
`RunPendingJavascriptUrlTasksBeforeBrowserNavigation`。renderer owner 命令只在当前 pending scheme 确为
`javascript:` 时进入既有 JavaScript navigation lifecycle/FIFO/currentness；没有 task 时为 unit no-op。普通
navigation 若在 renderer selection 时已经 supersede 更早的 JS task，该 task 已被 target scheduler 取消，命令
不会复活它。这样锁定的是 `ordinary start → already-queued target JS task → ordinary commit`，没有 sleep、yield、
retry 或第二个 JavaScript executor。

##### E3 聚焦证据

```bash
TMPDIR=<repo>/tmp/phase5e cargo nextest run -p moli-renderer-v8 \
  browser_navigation_causal_command_drains_pending_javascript_url_tasks \
  clients_open_window_page_completion_requires_exact_reserved_page_identity \
  service_worker_clients_open_window_request_records_popup_activation \
  service_worker_popup_client_survives_javascript_reopen \
  service_worker_clients_open_window_about_url_canonicalizes_to_fresh_page \
  service_worker_clients_open_window_cross_origin_keeps_worker_referrer_source \
  navigator_service_worker_notification_action_navigate_records_popup_activation \
  service_worker_open_window_admits_about_url_to_parent_and_rejects_file_scheme
# run 4902d822-c14e-4114-bd4e-5937bf7359a8：8 passed。

TMPDIR=<repo>/tmp/phase5e cargo nextest run -p moli-protocol \
  service_worker_auxiliary_producers_use_fresh_pages_and_navigation_terminals \
  clients_open_window_continuation_survives_fetch_fulfill \
  ordinary_popup_navigation_then_javascript_url_preserves_renderer_protocol_order
# run 71d96402-b911-4814-b648-3939da462365：3 passed。
```

第二组是完整 protocol apply/commit 证据，不只是 renderer carrier 单测：它覆盖 SW/notification target create、
same-origin WindowClient、cross-origin/null、204/no-commit、transport error、Fetch fulfill continuation，以及 stable
WindowProxy + 单 target 的 ordinary→JS 顺序。外部 Chrome/focused upstream WPT 本轮尚未运行，不能把这 11 条
Rust 回归写成 WPT pass。

##### Phase 5E local exit 后的边界

- production SW/notification caller 已没有 `open_lightweight_popup_window()`；当前该符号只剩 DOM compatibility
  facade 与 lightweight record 内部 reopen。2026-08-23 全仓更宽的 lightweight 静态扫描仍为 112 个 tracked Rust 文件、1492 处
  命中（包含测试、注释和兼容投影），所以 E3 不是 Phase 6 删除完成证明；
- Phase 5L1 已把 script-closable、local subtree beforeunload/unload、renderer close ACK/timeout 收进真实 Page
  transaction；Phase 5L2 又完成本地 `focus()` 与 browser-context active/focused Page 事务。Chromium top-level
  `blur()` 保持指标-only no-op，没有伪造对称失焦事务；
- Phase 5G1 已完成本地 committed-response COOP browsing-context-group/script-agent split、JS opener/name sever、
  old-group disconnected facade 与同一 Page/CDP target/session continuity；Phase 5G2 又完成 redirect-chain
  enforced/report-only status、virtual group、Reporting endpoint/request，以及普通/Fetch override/error Document
  的同一 commit carrier；Phase 5G3 再完成 sandbox+enforced-COOP response sanitation、redirect pre-follow stop、
  ordinary/Fetch blocked terminal、CDP `blockedReason` 与 error-Document forced real+virtual sever；Phase 5G4 又
  由 group owner 分配 typed WindowProxy endpoint generation，删除 V8 surface 对目标 V8 object 的 private-slot
  直连，并把 message/location/close/focus/child projection 的 currentness 收进唯一 endpoint resolver。COOP/close
  后的 stale endpoint 不会回退到 incumbent Page 或命中同 residence 的替代 realm。Phase 5G5 又把 target state
  与 per-agent V8 projection 分离，完成 same-group 跨源 agent replacement、remote opener/name projection、typed
  message/Location/focus/close command、protocol exact target Page ACK/currentness，以及 remote named top-level reuse。
  Phase 5G6A 再发布 agent-neutral remote child tree，建立 root-Document-qualified frame token、observer-local stable
  proxy、remote nested name/order、Location/postMessage、exact frame request/scheduler 与 same-form cancellation，并让
  protocol ACK 有 deadline、target Page teardown 后 retained endpoint disconnected。Phase 5G6B1 又把 top/frame
  command、replicated policy和 structured-clone attachment变成 strict versioned bytes，增加 execution-channel
  generation与 actor-admission前 queued cancellation，并按 Chromium锁住 cross-agent Wasm `messageerror`。真正 renderer
  process/channel/crash/restart、browser capability broker、agent reunification、fenced/embedder 完整
  replication 仍是 Chromium infrastructure gap；当前 Moli popup exit 只要求实际支持面上的 remote
  `CanNavigate`/activation/focus/unload 与 Reporting 行为有明确实现或明确降级；
- P6R2 已把 group-safe opaque-origin nonce 收进 top/child LocalWindow、related Page 与 strict remote frame
  replication；P6R3 又让被 JS 强引用的 detached top/child Document、Node、function 与 realm 继续由原 native
  owner 服务，并让无引用 realm 在 GC 后精确释放。P6R8 又补齐当前产品的 isolated-world source 与 remote
  `javascript:`。redirect-time browser-process `form-action`、extension world 独立 policy、file-local/diagnostic
  和 focused WPT 仍是明确长尾。

##### 距离最终架构的剩余工作量（2026-08-23）

P6R10 已满足这里定义的 Moli 单进程 popup owner exit。production lightweight 双栈、ghost entered-child
marker 与旧 async DynamicClassic scheduler 均已物理删除，stable group/lifetime owner 和 direct Browser
cross-document history handoff 已经完成。remote DOM form carrier 与没有 production producer 的 OOPIF descendant
ACK 不计入当前 exit。当前 81-case 结果为 40 pass / 26 fail / 15 timeout；剩余用例需要按实际责任方分类，
这组测试数字不等同于 popup 架构完成度。

| 大里程碑 | 必须形成的 exit condition | 规模/风险判断 |
| --- | --- | --- |
| 当前产品的 remote semantic closure | P6R8 已完成 remote `javascript:` 与 isolated-world source；P6R9 确认 ordinary/form request wire 已保存实际可传输的 source facts，本地 target Page 已拥有 descendant lifecycle；Reporting/file-local/diagnostic 按产品支持面分级 | 当前单进程 exit 已满足；OOPIF/process 聚合留给可选基础设施 |
| identity/lifetime closure | P6R2 已完成 group-safe opaque-origin nonce；P6R3 已完成真实 local top/child Document realm 的 retain/detach/GC owner 协同、执行资源退休与安全 V8 handle 释放。remote endpoint/process capability lifetime 继续归 remote milestone | 当前产品 local exit 已完成；remote 长尾随上一行收口 |
| Phase 6 readiness/removal | P6R4 已物理删除 record、realm alias、`with(window)` wrapper、mirrored parser/loader/lifecycle 与 protocol compatibility fallback；tracked Rust 宽口径旧模型扫描为零。P6R10 又删除 ghost marker 与旧 async DynamicClassic stack，并完成 focused WPT/CDP 重新分类 | 当前产品 exit 已完成；后续保持零回退门禁 |
| reachable realm semantics | Location 已区分 entry base 与 incumbent source；P6R10 完成 `window.open()` receiver/entry/accessing owner 拆分，并让动态 inline child script 同步执行；multiple-globals 四例 CLI/CDP 均通过 | 当前产品 exit 已完成 |
| 可选 Chromium infrastructure | 真实 renderer process/channel/crash/restart、browser capability broker、agent reunification、fenced/guest 完整 replication | 仅在产品决定采用多进程/OOPIF 时另立项目，不计入当前 popup exit |

P6R4 证明先按 reachability 和 owner dependency 拆除的顺序成立。P6R10 已完成此前保留的两个 exit，
包括 realm owner facts、child script timing 和 focused WPT/CDP 重分类。后续工作从 26 个 fail 与 15 个 timeout
中按 history、opener、name、testharness completion 等责任方建立新计划。真实多进程 lifecycle 只在独立产品
决策下推进。

##### E1-E2O/E3 完成后的 Phase 5E 范围

E3 是 local/Fresh creation-policy exit，不是 COOP/remote group model 或 Phase 6 删除完成标志。以下边界仍不能
套用“非命名直接 Fresh Page”或“所有 name 都查 related same-agent registry”的捷径：

- E2A 已让 `window.open()` 的 existing related top-level target 先执行 renderer group lookup；E2B 已让
  新建 named noopener/noreferrer context 使用 private Fresh group，并只在该 group 内保留/查找 live name；
  E2C 已让 full-creator ordinary named hyperlink 复用两项 decision，E2D 已把 full-creator form 的
  effective target 与 exact request 接入同一 decision；E2E 已让 `window.open()` / hyperlink 的 child
  source、related nested frame、完整 local frame-tree collision order 和普通 origin/ancestor filter 进入
  renderer resolver；E2F 已让 ordinary named form 的 exact request 消费同一 typed result，并由命中的
  current/related child owner 执行；E2G 已让同一 source form 跨 Page 保存 stable target route 与 exact
  scheduler generation，并由目标 child owner 取消 task/load/parser work；E2H 已让 current-top
  `window.open()`、hyperlink/form 保存 source Window/Document 与 referrer policy，同时保持 target Page
  的唯一 scheduler/loader authority；E2I/E2J 已把 new-context sandbox admission 与 Fresh target 的
  跨 Document frame-policy handoff 分开建模；E2K 又把 existing-target bypass、sandbox/blocker admission、
  exact activation consume 与 protocol observation 收进同一事务；E2L 再把 source-Document `allow-forms`
  双门禁和 `formdata` 后 connected recheck 放回 form owner；E2L.1 又把 late gate 前的 target selection/new
  initial Page creation 与 optional destination request 收进同一 action；E2M.1 再把 local candidate/special
  target/Location 的 permission decision 收进唯一 renderer authority；E2N 最后把 full-creator JavaScript URL
  的同步 final-target selection、target-Document FIFO networking task、CSP/currentness 与 optional string-result
  replacement 收进同一真实 Page owner，并让 protocol 只观察 `NoDestination` create/reuse；E2O 再把 target
  Trusted Types、stable child realm、form-action 与 existing-hit/new-miss source policy ordering 收进同一
  transaction；E3 最后把 SW/notification browser-context producer、exact `openWindow()` terminal 与
  ordinary→JS protocol owner ordering 收进同一真实 Page pipeline；
- `_self` / `_parent` / `_top` 继续命中 existing context，并按 Chromium compatibility 行为返回 Window；E2M.1
  refusal 只阻止 navigation，不把已选中的 target 当成 miss；
- full-creator `javascript:` URL 已由 E2N 完成同步 target selection + 异步 target-Document task，E2O 已补 target
  Trusted Types、form-action CSP 和 source-selection ordering；E3 已完成 production 非 DOM producer 与
  protocol ordinary→JS ordering，剩余的是 compatibility executor、remote/isolated-world 与外部 WPT 证据；
- form named/`_blank` 的 target/request carrier、E2E resolver integration、local/related-child repeated
  submission cancellation 与 child-source current-top causal/referrer identity 已完成；完整 target
  `CancelClientNavigation()` 的 local/related cross-agent child scheduler 已由 G6A 以 source id→target load binding
  完成，A→B retarget 会精确取消 A；source terminal-completion 回传与真正跨进程 cancellation wire 仍未完成；
  source-Document sandbox forms gate 已由 E2L 完成，
  direct-submit denial 的 creation-only auxiliary target carrier 已由 E2L.1 完成；
- E2M.1 已把 E2E 的 local nested filter 提升为 current/related Page 共用的 local `CanNavigate` authority，G6A 又让
  related remote child 消费复制的 committed origin/document.domain/sandbox ancestor facts，
  并消费 E2K activation、sandbox navigation/top flags、typed opener relation 与 destination origin/site
  exception；fenced tree、完整 remote top/opener exception、file-local compatibility、browser embedder fallback 与
  diagnostic 仍未实现；E2L forms gate 与 navigation policy 保持独立；
- E2J 已为 sandboxed Fresh top-level 的每个 Document 分配 browser-context 唯一 storage nonce；P6R2 又把同一
  nonce 提升为 Rust `WindowAccessOrigin` 的 non-serialized identity，并让 inherited auxiliary `about:blank`
  通过 creator StorageKey 复制 exact nonce。related host 不再比较会碰撞的 `LocalWindowId`，也不再把所有 opaque
  equality 一律拒绝；独立创建的两个 `null` origin 因 nonce 不同继续 fail closed；
- E2I 已让 `allow-popups` / escape-sandbox 的 **new-context admission** 使用 typed policy；E2J 已补 Fresh
  no-local-proxy sandbox handoff；E2K 已补 transient/sticky activation 和 browser-context popup-blocker
  decision；E2L 已补 attribute/CSP/inherited `allow-forms` 和 source gate；E2L.1 已补 direct-submit
  creation-only side effect；E2M.1 已补 local sandbox/top-level navigation decision；G1 已补 local
   committed-response COOP group switch 和 old-proxy disconnected facade；G2 已补 redirect-chain
   enforced/report-only status、virtual group 与 report request，并让 Fetch override/error Document 继续消费同一
  commit；G3 已补 sandboxed popup blocked response 的 redirect stop、ordinary/Fetch effective response、
  authoritative error Document、CDP failure 与 forced real+virtual sever；G4 已补 group-qualified endpoint
  generation、删除 target V8-object marker，并统一 local/disconnected routed-operation currentness；G5 已补
  same-group cross-agent top-level replacement、remote `CanNavigate` 的 opener/sandbox 子集与 typed target
  command/ACK；G6A 已补 related remote child tree、nested `CanNavigate` 子集、exact frame route 与 scheduler
  cancellation。真正 process transport、fenced/embedder 完整 policy/lifecycle 仍没有统一；
- E2H 已覆盖同源 direct、跨 origin redirect hop、Fetch request-stage URL override、默认 downgrade policy
  与最终 `document.referrer`；redirect response policy mutation、Fetch response-stage override 和
  explicit header/document-referrer 的完整 Chromium 矩阵仍需单独 WPT/最小探针。

##### 已知差距与后续顺序

Phase 5C 已把 related top-level 的动态 child/opener 投影接回 owner，但还不是整份
`cross-origin-objects.html` 全通过。明确未完成项如下：

| 未完成项 | 当前事实 | 下一责任方 |
| --- | --- | --- |
| close/unload remote 长尾 | Phase 5L1 已统一真实 local Page 的 script-closable、subtree beforeunload、一次 dialog、network drain、pagehide/unload、renderer ACK/timeout 与 target teardown；G6A 的 remote child proxy 在 root/target teardown 后会 disconnected；P6R4 已删除 compatibility owner | 当前 Page 内的 descendant 已覆盖；跨 OOPIF/process descendant ACK 在出现真实 producer 后由 process owner 实现 |
| focus / active Page remote 长尾 | Phase 5L2 已完成 local Page/focused-frame authority、native `document.hasFocus()`、events/CSS、target activation/focus emulation/window-state/create/close promotion；G5 的 remote top-level focus 在 source 侧消费 activation/opener admission，并由 target Page ACK；G6A 没有伪造 nested focus，top-level `blur()` 保持 Chromium no-op | 当前产品不创建 remote child process；embedder/OOPIF activation endpoint 属于可选基础设施 |
| retained detached Document values | P6R3 已完成：stable WindowProxy 继续投影 current Window；被作者保存的旧 Document/Node/function 仍读写原 detached DOM，`defaultView`/Window execution authority 已关闭；最后引用删除并 GC 后 old top/child Context、host 与 native DOM 精确回到基线 | 当前产品 local exit 已满足；保留真实站点长跑 RSS 与未来不可信跨进程 capability lifetime 观察 |
| policy/group sever | E1-E2O 已统一 DOM local/Fresh creation、name/target/request/policy/activation/JavaScript URL；E3 又统一 SW/notification producer；G1-G6 已完成 local COOP、group endpoint、cross-agent top/child route、strict wire、channel currentness、ACK deadline 与 Page teardown disconnect；P6R8 已补 remote JavaScript URL、isolated-world source 与目标 main-realm execution；P6R9 已确认 remote form wire 无需携带 DOM/V8 value；P6R10 已完成 receiver/entry/accessing realm 区分与外部证据 | 当前单进程 exit 已满足；Reporting/file-local/diagnostic 和真实 process/fenced/guest 另行分级 |

下一批按以下顺序推进，避免把动态状态继续塞进静态 surface：

1. **Phase 5B/L1：close/unload transaction（已完成本地真实 Page 闭环）。** 已建立唯一
   browsing-context liveness authority；`window.close()`、`Target.closeTarget`、opener-side `.closed`、
   task cancellation、targetDestroyed 和 Page teardown 使用同一 typed transaction，并覆盖重复 close、
   target currentness 与早期 admission。L1 又补齐 `OpenedByDOM`/history/browser setting gate、root→descendant
   `beforeunload`、sticky activation/一次 dialog、network-drained barrier、pagehide/unload、renderer ACK/timeout
   和 command response causal fence；RemoteFrame/COOP 不在本地闭环中。
2. **Phase 5C：live relation/child projection（本纵切已完成）。** Page-scoped opener edge 与 top-level
   child/name registry 已取代静态 surface snapshot，覆盖动态 frames、named child、`then` / `open`
   shadow、opener setter/discard sever 与 navigation persistence；COOP/remote group sever 留给独立 group/remote
   milestone。
3. **Phase 5D：WPT internal methods/per-incumbent membrane（D1-D3b 已完成）。** D1 已完成 Location、D2
   已完成 Window 的 exact ownKeys、unknown/index、mutation、prototype/preventExtensions 静态矩阵，
   D2.5 已把 related/generic nested child 接回通用 live registry owner并修复预物化 restricted facade；
   D3a 已完成 Function/accessor 的 accessing-Realm prototype、identity、cache、异常 realm 与
   receiver-owned target dispatch；D3b 已完成 stable top identity cutover、callback-scoped observer/target
   child projection，以及 same-host / related-Page 两条访问矩阵。
4. **Phase 5E：local/Fresh creation policy（E1-E2O 与 E3 已完成）。** E1 已覆盖
   `window.open()` 非命名 noopener/noreferrer 与 hyperlink `_blank` implicit/explicit noopener 的
   single-owner/referrer commit；E2A 已覆盖 related top-level named `window.open()` 的真实 initial Page、
   live name/lifecycle registry 与 exact target reuse；E2B 已覆盖新建 named suppress-opener 的 Fresh
   Page/private group、首 realm name 与 self-only lookup；E2C 已把 ordinary named hyperlink 的 existing
   lookup、Related/Fresh creation 与 exact target handoff 收进同一 authority；E2D 又把 form effective
   target、named/`_blank`、POST body/Content-Type/referrer 与 target-realm NavigateEvent 收进同一 request
   carrier；E2E 又把 `window.open()` / hyperlink 的 source subtree、current Page、ordered related Pages
   完整 local frame-tree lookup、related-child target owner 与普通 nested `CanNavigate` 收进同一 resolver；
   E2F 再让 ordinary named form 的 exact request、target realm event 与 local child scheduler 消费该 typed
   result，并复用 child stable WindowProxy/policy container；E2G 又让 source form 保存 stable Page/child route
   与 exact navigation-load binding，跨 Page 取消由目标 owner 撤销 task、loader/parser ledger 且不会误杀
   replacement navigation；E2H 再把 current-top 的 source Window/Document、URL/policy 与 suppression 保留到
   redirect/Fetch URL override/commit，而不改变 target Page 的唯一 scheduler/loader ownership；E2I 已把
   attribute/response-CSP `allow-popups` admission 放到 existing lookup 之后、Page reservation/event 之前，并
   让 escape token 只控制 sandbox 继承；E2J 又让 Fresh target 的 initial 和 replacement Document 从 target
   Page slot 取得同一 renderer-owned accepted policy，且 noopener sever 与 sandbox inheritance 保持正交；
   E2K 再把 DevTools/trusted-input activation、existing-target bypass、sandbox/blocker order、single consume 和
   `Page.windowOpen` observation 放进同一 renderer transaction；E2L 又把 attribute/response-CSP/inherited
   `allow-forms` 收进 source-Document 双门禁，并保留 direct-submit `formdata` timing；E2L.1 再让 late denial
   保留 already-created initial Page/WindowOpen/activation transaction，同时以 optional request 明确不启动
   destination navigation；E2M.1 已把当前 local owner 可表达的 sandbox/top-level `CanNavigate` 顺序统一到
   committed policy 与 stable endpoint；E2N 又把 full-creator JavaScript URL 的同步 target selection、异步
   target-Document FIFO task、source/target CSP、currentness/string replacement 和 protocol `NoDestination`
   收进同一真实 Page；E2O 再补 target Trusted Types、stable child realm、form-action 与 existing/new source
   policy 分岔；E3 最后把 Service Worker/notification browser-context producer、exact Page/worker continuation
   terminal 与 ordinary→JS owner ordering 统一到真实 Page。Phase 5L1 已完成 close/unload closure，Phase 5L2
   已完成 local focus/active Page closure，Phase 5G1 已完成 local committed-response COOP group/agent sever 与
   stable Target/session continuity，Phase 5G2 已完成 redirect-chain enforced/report-only status、virtual group、
   report request、normal/Fetch/error commit carrier 与 exact Page output reservation，Phase 5G3 已完成 sandbox
   blocked-response sanitation、redirect stop、Fetch effective response、CDP terminal 与 forced real+virtual sever；
   Phase 5G4 已完成 group-qualified endpoint generation、删除 target V8-object marker，并统一 local/disconnected
   message/location/close/focus currentness；Phase 5G5 已完成 same-group cross-agent top-level replacement、per-agent
   WindowProxy/opener projection、remote named reuse 与 typed protocol target ACK；Phase 5G6A 已完成 agent-neutral
   remote child tree、stable nested proxy、related named target/Location/postMessage、exact scheduler cancellation、
   ACK deadline 与 Page teardown disconnect；Phase 5G6B1 已完成 versioned command/frame-policy/structured-clone
   wire、execution-channel generation与 queued ACK cancellation。P6R8 又完成 remote JavaScript URL、
   isolated-world source、target main-realm execution 与 denied-name no-fallback。下一步不再把真实 renderer process
   当作 popup blocker；P6R1-P6R3 已完成 creation caller graph 与当前产品 local identity/lifetime closure，P6R4 又按
   facade→loader/parser→protocol fallback 的依赖顺序物理删除 lightweight 双栈。P6R9 又按 Chromium wire
   与当前 producer 修正 remote form/descendant exit。P6R10 已完成 receiver/entry/accessing identity、child script
   timing 与 focused WPT/CDP 重新验收。后续按当前 26 fail / 15 timeout 的实际责任方处理，并继续分级
   Reporting/file-local。多进程 renderer/capability broker 仅在独立产品决策后继续。

Phase 5A 聚焦验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) | test(same_origin_child_window_migration_to_cross_origin_installs_denied_surface) | test(data_url_child_document_is_cross_origin_to_parent) | test(child_window_proxy_identity_survives_cross_origin_round_trip)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 4 passed

cargo nextest run -p moli-protocol \
  -E 'test(popup_transport_failure_commits_error_document_in_stable_auxiliary_page)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 1 passed；包含 error commit、Window allowlist、postMessage、location assign/replace

cargo nextest run \
  -E 'test(child_document_creation_freezes_document_start_script_registry) | test(add_script_run_immediately_creates_top_level_world_even_when_child_world_name_matches)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 2 passed；另在 concurrent core+renderer 负载下重复 protocol case 100 次通过
```

Phase 5B 聚焦验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(canceling_prepared_live_page_replacement_preserves_page_environment_and_output_stream) or test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) or test(related_page_script_agent_transfers_stable_window_proxy_objects_and_dom_wrappers) or test(related_page_window_close_is_synchronous_idempotent_and_disconnects_final_realm) or test(child_navigation_retires_runtime_binding_context_and_stale_function) or test(child_navigation_retires_local_window_owned_xhr) or test(child_navigation_aborts_fetch_and_detaches_keepalive)' \
  --no-fail-fast
# 7 passed

cargo nextest run -p moli-protocol \
  -E 'test(popup_window_close_retires_target_and_parks_stable_window_proxy) or test(target_close_parks_the_same_stable_popup_window_proxy) or test(popup_transport_failure_commits_error_document_in_stable_auxiliary_page)' \
  --no-fail-fast
# 3 passed

cargo nextest run -p moli-protocol \
  -E 'test(stale_window_close_termination_cannot_retire_current_page_residence)' \
  --no-fail-fast
# 1 passed；最终 termination continuation 会再次拒绝 stale Page generation
```

Phase 5C 聚焦验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_script_agent_experiment_shares_isolate_and_survives_source_close) or test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) or test(related_page_window_close_is_synchronous_idempotent_and_disconnects_final_realm) or test(related_page_script_agent_transfers_stable_window_proxy_objects_and_dom_wrappers)' \
  --no-fail-fast
# 4 passed；覆盖 live index/name/ownKeys、then/open shadow、显式 sever、opener discard、
# closed-popup opener retention 和两种 sever 的 navigation persistence
```

Phase 5D1 聚焦验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) or test(data_url_child_document_is_cross_origin_to_parent) or test(same_origin_child_window_migration_to_cross_origin_installs_denied_surface) or test(child_window_proxy_identity_survives_cross_origin_round_trip)' \
  --no-fail-fast
# 4 passed；覆盖 related/detached top-level Location、generic child Location、
# origin migration 和 stable WindowProxy navigation round-trip

cargo clippy -p moli-renderer-v8 --all-targets -- -D warnings
# passed

cargo nextest run -p moli-core \
  -E 'test(cross_origin_window_proxy_exposes_standard_noop_shape)' \
  --no-fail-fast
# 1 passed；同步淘汰 core 集成层中 denied Location descriptor/has 的旧预期
```

Phase 5D2 聚焦验证：

```bash
cargo nextest run \
  -E 'test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) | test(data_url_child_document_is_cross_origin_to_parent) | test(cross_origin_window_proxy_exposes_standard_noop_shape) | test(cross_origin_window_proxy_exposes_named_child_frames)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 4 passed；覆盖 related/generic/detached Window internal methods、ordinary intrinsic
# delegation、exact ownKeys、document/focus named collision 和 navigation 后 stale name 拒绝

cargo nextest run \
  -E 'test(cross_origin_window_proxy_exposes_standard_noop_shape) | test(cross_origin_top_window_proxy_length_tracks_top_child_lifecycle) | test(cross_origin_window_proxy_exposes_named_child_frames) | test(child_cross_origin_window_denials_use_the_child_dom_exception_realm) | test(same_origin_child_window_migration_to_cross_origin_installs_denied_surface) | test(child_window_proxy_identity_survives_cross_origin_round_trip) | test(child_browsing_context_cross_origin_post_message_reply_preserves_source_identity) | test(captured_cross_origin_content_window_matches_message_source_after_child_navigation) | test(captured_cross_origin_content_window_keeps_safe_surface_during_realm_gap) | test(data_url_child_document_is_cross_origin_to_parent) | test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 首次 10/11 passed；唯一失败是 document named-child collision 仍期待旧 denied accessor。
# 按 Chromium precedence 更新该回归后单独复跑通过；上述最终 4-case owner matrix 随后全通过。

cargo check -p moli-renderer-v8
# passed
```

Phase 5D2.5 聚焦验证：

```bash
cargo nextest run -p moli-core --test history_child \
  -E 'test(cross_origin_window_proxy_exposes_named_child_frames)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 新 mutation matrix 在旧实现上稳定失败：child 已回复完成 rename/remove/append，parent 读取
# renamedNested descriptor 得到 SecurityError；接入通用 owner 后 1 passed。

cargo nextest run \
  -E 'test(data_url_child_document_is_cross_origin_to_parent) | test(cross_origin_window_proxy_exposes_named_child_frames)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 2 passed；同时证明预物化 child shell 仍是 restricted facade、不会泄漏 raw global。

cargo nextest run \
  -E 'test(cross_origin_window_proxy_exposes_standard_noop_shape) | test(cross_origin_top_window_proxy_length_tracks_top_child_lifecycle) | test(cross_origin_window_proxy_exposes_named_child_frames) | test(child_cross_origin_window_denials_use_the_child_dom_exception_realm) | test(same_origin_child_window_migration_to_cross_origin_installs_denied_surface) | test(child_window_proxy_identity_survives_cross_origin_round_trip) | test(child_browsing_context_cross_origin_post_message_reply_preserves_source_identity) | test(captured_cross_origin_content_window_matches_message_source_after_child_navigation) | test(captured_cross_origin_content_window_keeps_safe_surface_during_realm_gap) | test(data_url_child_document_is_cross_origin_to_parent) | test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 11 passed；覆盖 related/generic live owner、pre-materialized shell、realm-gap snapshot、
# navigation round-trip、source identity、Location/Window internal-method matrix。

cargo check -p moli-renderer-v8
# passed
```

Phase 5D3a 聚焦验证：

```bash
cargo nextest run \
  -E 'test(data_url_child_document_is_cross_origin_to_parent) | test(cross_origin_property_wrappers_are_cached_per_accessing_realm)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 2 passed；覆盖两个 accessing Realm 的完整 wrapper 矩阵，以及 Location cached
# href/replace wrapper 的 receiver brand / WebIDL 边界。

cargo nextest run \
  -E 'test(cross_origin_property_wrappers_are_cached_per_accessing_realm) | test(cross_origin_window_proxy_exposes_standard_noop_shape) | test(cross_origin_top_window_proxy_length_tracks_top_child_lifecycle) | test(cross_origin_window_proxy_exposes_named_child_frames) | test(child_cross_origin_window_denials_use_the_child_dom_exception_realm) | test(same_origin_child_window_migration_to_cross_origin_installs_denied_surface) | test(child_window_proxy_identity_survives_cross_origin_round_trip) | test(child_browsing_context_cross_origin_post_message_reply_preserves_source_identity) | test(captured_cross_origin_content_window_matches_message_source_after_child_navigation) | test(captured_cross_origin_content_window_keeps_safe_surface_during_realm_gap) | test(data_url_child_document_is_cross_origin_to_parent) | test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) | test(popup_transport_failure_commits_error_document_in_stable_auxiliary_page) | test(cross_origin_location_proxy_only_allows_href_and_replace_navigation)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 首次 13/14 passed：唯一失败把 href setter 的 null receiver 当成访问方 global；
# 在 WebIDL conversion 前补 Location brand，并新增 replace null-receiver probe 后，最终 14 passed。

cargo clippy -p moli-renderer-v8 --all-targets -- -D warnings
# passed
```

Phase 5D3b 聚焦验证：

```bash
cargo nextest run -p moli-core --test history_child \
  -E 'test(cross_origin_child_endpoint_projection_is_relative_to_the_observer)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 旧实现稳定失败：B 经 parent.frames[1] 得到的 C 仍按 A/C origin 保持 restricted，
# 首次 marker write 抛 SecurityError；stable A-side identity 与 A-side denial 同时成立。
# stable top WindowProxy + observer/target projection 接入后 1 passed；随后加入 named lookup
# 与 named/indexed descriptor identity 后仍为 1 passed。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 1 passed；跨 host related opener 与 popup parent 分别对同一个 child 得到 allowed/denied，
# 并验证 child parent/top 指向 popup stable WindowProxy。

cargo nextest run \
  -E 'test(cross_origin_child_endpoint_projection_is_relative_to_the_observer) | test(cross_origin_property_wrappers_are_cached_per_accessing_realm) | test(cross_origin_window_proxy_exposes_standard_noop_shape) | test(cross_origin_top_window_proxy_length_tracks_top_child_lifecycle) | test(cross_origin_window_proxy_exposes_named_child_frames) | test(child_cross_origin_window_denials_use_the_child_dom_exception_realm) | test(same_origin_child_window_migration_to_cross_origin_installs_denied_surface) | test(child_window_proxy_identity_survives_cross_origin_round_trip) | test(child_browsing_context_cross_origin_post_message_reply_preserves_source_identity) | test(captured_cross_origin_content_window_matches_message_source_after_child_navigation) | test(captured_cross_origin_content_window_keeps_safe_surface_during_realm_gap) | test(data_url_child_document_is_cross_origin_to_parent) | test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) | test(popup_transport_failure_commits_error_document_in_stable_auxiliary_page) | test(cross_origin_location_proxy_only_allows_href_and_replace_navigation)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 15 passed；覆盖 D3a membrane、D3b same-host/cross-host endpoint、live registry、
# navigation/realm gap、source identity、popup error Document 与 Location/Window internal methods。

cargo check -p moli-renderer-v8
# passed
```

Phase 5E1 聚焦验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  -E 'test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(noreferrer_implies_noopener_and_last_value_wins) | test(hyperlink_target_blank_reloads_rel_opener_policy_for_each_activation) | test(window_open_noopener_lightweight_popup_uses_fresh_session_storage)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 4 passed；覆盖 production activation 不再携带 popup_id、Fresh agent、feature precedence、
# hyperlink 动态 rel policy 和 standalone fresh session-storage fallback。

cargo nextest run -p moli-protocol \
  -E 'test(noopener_and_noreferrer_popups_have_one_real_navigation_with_creator_referrer_policy) | test(anchor_blank_target_uses_implicit_noopener) | test(popup_initial_about_blank_adopts_renderer_page_and_related_script_agent)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 3 passed；覆盖 noopener/noreferrer/implicit-noopener 的 single request、network Referer、
# initial/destination document.referrer、精确 about:blank 与 fragment same-document、null opener、
# target attach，以及保留 opener 路径不回归；主矩阵内包含 6 条 activation case。

cargo nextest run -p moli-fetch \
  navigation_referrer_is_distinct_from_http_header_eligibility \
  --no-fail-fast --status-level fail --final-status-level fail
# 1 passed；证明非 HTTP destination 的 Document referrer 与 HTTP Referer eligibility 分离。

cargo nextest run -p moli-protocol \
  local_storage_mutations_fan_out_across_targets_without_leaking_session_storage \
  --no-fail-fast --status-level fail --final-status-level fail
# 默认 nextest 栈首次全量稳定 SIGABRT 后，detached HEAD 基线通过；heap-owned commit
# environment 修复后首次 + 连续 10 次聚焦复跑均通过。

cargo check -p moli-fetch -p moli-renderer-v8 -p moli-protocol --tests
# passed
```

Phase 5E2A 聚焦验证：

```bash
cargo nextest run -p moli-protocol \
  window_open_named_target_reuse_is_owned_by_the_renderer_page_group \
  --no-fail-fast --status-level fail --final-status-level fail
# 红灯：target 初次观察为 `undefined||false`，而 creator 已观察到
# `reportWindow|renderer-page`；接入 exact real Page/group 后 1 passed。
# 回归包含动态 window.name、主动清空 protocol target_window_names、noopener
# exact reuse、无新 Target.targetCreated，以及原 opener edge 保留。

cargo nextest run -p moli-protocol window_open_named_target \
  --no-fail-fast --status-level fail --final-status-level fail
# 4 passed；覆盖既有 named target、same-command reuse、renderer group authority
# 与旧 catchall/target projection 兼容面。

cargo nextest run -p moli-renderer-v8 \
  per_page_isolate_policy_keeps_window_open_routes_page_owned \
  --no-fail-fast --status-level fail --final-status-level fail
# 1 passed；首次 named activation 携带 RelatedAuxiliaryPage reservation，普通和
# noopener reuse 均携带同一 RendererResolvedPopupTarget，且不预留第二个 Page。

cargo nextest run -p moli-renderer-v8 \
  window_open_noopener_navigates_existing_named_iframe_and_returns_null \
  --no-fail-fast --status-level fail --final-status-level fail
# 1 passed；existing iframe 仍导航、返回 null，且不产生 popup activation。

cargo nextest run -p moli-protocol \
  protocol_name_projection_cannot_redirect_popup_to_unrelated_background_owner \
  --no-fail-fast --status-level fail --final-status-level fail
# 1 passed；手工写入的 protocol name projection 无权把 renderer 新建决定重定向到
# unrelated background Page，且不会 promote/导航该旧 target。

cargo check -p moli-renderer-v8 -p moli-protocol
# passed
```

Phase 5E2B 聚焦验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  per_page_isolate_policy_keeps_window_open_routes_page_owned \
  --no-fail-fast --status-level fail --final-status-level fail
# 接入前稳定失败：named suppress-opener activation 的 popup_id 为 Some(2)，预期 None。
# 接入后 1 passed；同时覆盖 FreshUnnamed/FreshNamed/Related typed disposition、
# noopener+noreferrer 各自的 Fresh admission、相同 name 的两次 reservation 使用不同 Page id。

cargo nextest run -p moli-protocol \
  named_suppress_opener_window_open_creates_distinct_fresh_groups_with_live_names \
  --no-fail-fast --status-level fail --final-status-level fail
# 接入前稳定失败：browser-context target_window_names 暴露后一个 Fresh target（Some("TID-2")）。
# 接入后 1 passed；覆盖两个 Target.targetCreated、无全局 name projection、两个真实 realm 的
# requested window.name/null opener、每个 private group 的 self-only exact reuse 与另一 target 不被导航。

cargo check -p moli-protocol
# passed
```

E2A 当时用于保护尚未迁移 hyperlink lightweight terminal 的 owner-scheduler characterization，已在
E2C 完成后删除；它要求 opener-local mirrored loader 发起请求，不再是合法的 green 条件。

Phase 5E2C 聚焦验证：

```bash
cargo nextest run -p moli-renderer-v8 \
  per_page_isolate_policy_keeps_window_open_routes_page_owned \
  --no-fail-fast --status-level fail --final-status-level fail
# 接入前稳定失败：named opener hyperlink 的 new_target_disposition 为 None，预期 Some(Related)。
# 接入后 1 passed；同时覆盖 existing target 的 exact renderer residence、noreferrer 不暴露/重写 opener，
# 以及两次同名 suppress-opener hyperlink 得到两个 FreshNamed Page reservation。

cargo nextest run -p moli-protocol \
  -E 'test(named_opener_hyperlink_creation_and_reuse_are_owned_by_renderer_group) | test(named_suppress_opener_hyperlinks_create_distinct_fresh_groups_with_live_names)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 接入前 2 failed：Related target realm 为 `|false|#related-two`，且两个同名 suppress-opener link
# 只产生一个 Target.targetCreated；接入后 2 passed。回归覆盖清空 protocol name projection 后 exact reuse、
# existing opener edge 保留、Fresh target 不发布全局 name，以及两个真实 realm 的 name/null opener。

cargo nextest run -p moli-protocol \
  -E 'test(window_open_named_target_reuses_existing_popup_target) | test(window_open_named_target_reuse_is_owned_by_the_renderer_page_group) | test(named_suppress_opener_window_open_creates_distinct_fresh_groups_with_live_names) | test(window_open_named_target_reused_in_same_command_emits_one_page_event) | test(anchor_blank_target_uses_implicit_noopener) | test(anchor_blank_target_with_rel_opener_preserves_exact_opener) | test(protocol_name_projection_cannot_redirect_popup_to_unrelated_background_owner)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 7 passed；覆盖相邻 window.open name authority、E2B Fresh split、hyperlink `_blank` 两种 opener policy
# 与 unrelated protocol projection 不回归。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(hyperlink_target_blank_reloads_rel_opener_policy_for_each_activation) | test(window_open_noopener_navigates_existing_named_iframe_and_returns_null)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 2 passed；覆盖动态 rel policy 与 existing named iframe 优先级。
```

Phase 5A 提交门禁结果：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# 15891 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5B 提交门禁结果：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# 15895 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5C 提交门禁结果：

```bash
cargo nextest run --no-fail-fast
# 15895 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5D1 提交门禁结果：

```bash
cargo nextest run --no-fail-fast
# 首次：15892 passed, 3 failed, 18 skipped。
# 其中 core cross_origin_window_proxy_exposes_standard_noop_shape 是本纵切应淘汰的旧
# denied Location descriptor/has 预期，更新后聚焦通过；另两个 websocket/parser backlog
# case 与本纵切路径不相交，在首次高并发 workspace 运行中失败。

for run in {1..5}; do
  cargo nextest run -p moli \
    -E 'test(websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded) or test(websocket_cdp_runtime_evaluate_uses_committed_page_while_parser_blocking_source_is_pending)' \
    --no-fail-fast || exit 1
done
# 5 rounds passed；每轮 2/2

cargo nextest run --no-fail-fast
# 最终：15895 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5D2 提交门禁结果：

```bash
cargo nextest run --no-fail-fast
# 首次：15894 passed, 1 failed, 18 skipped。唯一失败是
# webidl_callback_source_boundary_tests::direct_v8_call_inventory_is_frozen：D2 新增的原始
# Object/Reflect intrinsic delegate 尚未登记 source-level inventory。

cargo nextest run \
  -E 'test(direct_v8_call_inventory_is_frozen) | test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) | test(data_url_child_document_is_cross_origin_to_parent) | test(cross_origin_window_proxy_exposes_standard_noop_shape) | test(cross_origin_window_proxy_exposes_named_child_frames)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 将该调用按 captured native intrinsic 分类为 NativeForwardingOrScript 后，5 passed。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# 最终：15895 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5D2.5 提交门禁结果：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# 15895 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5D3a 提交门禁结果：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# 15896 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5D3b 提交门禁结果：

```bash
cargo nextest run --no-fail-fast
# rebase 到当时 origin/master 后最终为 15904 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5E1 提交门禁结果：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# rebase 前：15906 passed, 18 skipped。
# rebase 到 origin/master 后，前两次 workspace 高并发运行各出现一个互不相同的既有
# timing case：parser-script network backlog 一次、sandboxed blob/OPFS message 一次；
# 其余均为 15961 passed。两个失败分别连续 10 次聚焦复跑通过。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# rebase 后最终：15962 passed, 18 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5E2A 提交门禁结果：

```bash
cargo nextest run --no-fail-fast
# 首次：15962 passed, 2 failed, 18 skipped。一个失败是 protocol-only name map
# 仍被旧测试当作 renderer lookup authority；另一个旧 owner test 等待已迁移 named
# window.open 的 mirrored loader，380.779s 无进展后手动中断。两条均按上文责任边界
# 改写并分别聚焦通过。

cargo nextest run --no-fail-fast
# 接入提交 rebase 前：15964 passed, 18 skipped；99.385s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed

# `git pull -r origin master` 把 34 个 popup 提交重放到 ef44056fe9 后再次执行：
cargo nextest run --no-fail-fast
# 15992 passed, 18 skipped；100.406s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5E2B 提交门禁结果：

```bash
cargo nextest run --no-fail-fast
# 最终 typed API 收口后重跑：15993 passed, 18 skipped；100.737s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed
```

Phase 5E2C 提交门禁结果：

```bash
cargo nextest run --no-fail-fast
# 15994 passed, 18 skipped；执行阶段 100.077s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 29s。

# `git pull -r origin master` 把 36 个分支提交从 ef44056fe9 重放到 cac2e67294 后再次执行：
cargo nextest run --no-fail-fast
# 15994 passed, 18 skipped；执行阶段 100.649s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 30s。
```

Phase 5E2D 提交门禁结果：

```bash
cargo nextest run -p moli-core \
  -E 'test(wpt_compat_case_form_submitter_target_fallback_basic)' \
  --no-fail-fast --status-level fail --final-status-level fail
# 1 passed。首次全量门禁暴露本地 port 仍把显式空 formtarget 当作 missing；fixture 按当前
# Chromium owner 修正为“显式空值 -> 提交时冻结的 base target，缺失属性 -> form target”后聚焦通过。

cargo nextest run --no-fail-fast
# 最终重跑：15996 passed, 18 skipped；执行阶段 99.384s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；最终 Rust 改动下 1m 29s。

git diff --check
# passed
```

E2D rebase 后集成门禁：

```bash
git pull -r origin master
# 无文本冲突；把 37 个 popup 分支提交从 cac2e67294 重放到 b016375769，E2D 提交变为
# d209eb3430。

cargo nextest run -p moli -p moli-protocol \
  -E 'test(websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded) | \
      test(parser_tail_dom_mutations_precede_the_dcl_binding_refresh)' \
  --stress-count 100 --flaky-result fail --test-threads 8 --no-fail-fast
# 100/100 iterations；每轮 2/2 passed；22.816s。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run id a8546408-dd85-4300-8578-8ec4e4c21ee4；16000 passed, 18 skipped；98.000s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 36s。

git diff --check
# passed
```

这次 rebase 没有文本冲突，但 master 新增的 stable Page navigation 暴露了 Document/stream
identity 边界：Page-scoped output stream 会跨 replacement 保持 identity，DOM mutation batch
却必须属于 producer Document。纯 master 聚焦用例通过，逐提交二分首次落在
`67ca127c1b`；修复后每个 DOM batch 自带 exact Document agent token，protocol 只绑定匹配的
current attachment。全量并发随后又暴露两处测试采样歧义：held-parser 请求发出不等于 parser 已
到达该 live-tree 位置，初始 `about:blank` DCL 也不能代表目标 URL generation。fixture 现分别用
已执行的 inline `document.write()` 和目标 URL `Page.frameNavigated` 建立确定边界，没有增加
sleep、retry 或放宽事件顺序断言。

Phase 5E2E 提交门禁结果：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run ffc9f453-4050-420b-b9bb-ccc2d33121bf：16002 passed, 18 skipped；执行阶段 99.771s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 31s。

git diff --check
# passed
```

E2E rebase 后集成门禁：

```bash
git pull -r origin master
# origin/master 从 b016375769 前进到 744e161dad；39 个 popup 分支提交完成重放。
# 旧 continuation-fence 提交与新 master 在 moli-protocol-cdp/src/wire.rs 有一个内容冲突：
# 合并结果同时保留 master 的 Debugger/IO-route exceptions，以及旧提交的 Runtime control-method
# exceptions 和 Page.getNavigationHistory continuation fence。

cargo nextest run -p moli-protocol-cdp -p moli-renderer-v8 -p moli-protocol \
  -E 'package(moli-protocol-cdp) | test(related_page_named_frame_lookup_follows_chromium_frame_tree_order) | test(named_frame_lookup_skips_candidate_the_source_cannot_navigate) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(window_open_noopener_navigates_existing_named_iframe_and_returns_null) | test(hyperlink_target_blank_reloads_rel_opener_policy_for_each_activation) | test(hyperlink_javascript_url_csp_checks_the_source_document_before_target_selection) | test(named_opener_hyperlink_creation_and_reuse_are_owned_by_renderer_group) | test(named_suppress_opener_hyperlinks_create_distinct_fresh_groups_with_live_names) | test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request) | test(base_target_blank_form_post_creates_fresh_target_with_exact_request)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run ce377834-850c-48c9-8f2d-4b4f867db090：19 passed；包含 wire crate 全部单测和 10 条 popup 邻接回归。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run f83a2567-7fda-490e-bc18-63a6a7f73f8e：16011 passed, 18 skipped；执行阶段 100.329s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 31s。

git diff --check
# passed
```

Phase 5E2F 提交前门禁结果：

```bash
cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run 6cf8a817-01c0-4554-a5f2-f0771db5db3d：16009 passed、3 failed、18 skipped。
# 三个 failure 都是 charset/data URL form 用例仍断言 GET named iframe 必须使用旧 URL bootstrap；
# runtime 已正确产生 typed Request(GET, body=None)。断言改为同时验证编码 URL、method/body 和 Referer。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(form_submission_rewrites_charset_control_from_accept_charset) | \
      test(form_get_submission_uses_document_encoding_for_query) | \
      test(iso_2022_jp_get_form_data_url_target_posts_stateful_values)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 16862d74-5fbf-4f21-b48e-3ab142a6de83：3 passed。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run d09b90aa-93dc-4fad-8c6c-ba09b2165071：16012 passed、18 skipped；执行阶段 99.264s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 29s。

git diff --check
# passed
```

Phase 5E2F rebase 后集成门禁：

```bash
git pull -r origin master
# origin/master 从 744e161dad 前进到 768c70dfd7；40 个 popup 分支提交无冲突重放。
# master 新增 test(cdp): stabilize shadow DOM navigation fixtures，只修改 protocol DOM 测试 fixture，
# 不与 E2F form/child owner 代码重叠。

cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E 'test(related_page_named_form_post_uses_nested_target_owner_and_exact_request) | test(related_page_named_frame_lookup_follows_chromium_frame_tree_order) | test(named_frame_lookup_skips_candidate_the_source_cannot_navigate) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(form_target_blank_reloads_rel_opener_policy_for_each_submission) | test(canceled_post_form_navigation_aborts_signal_without_synthetic_timer) | test(detached_child_form_submit_targets_named_iframe_without_shadow_controls) | test(formdata_event_appended_entries_are_submitted_to_named_iframe) | test(submit_button_click_supersedes_programmatic_submit_after_target_change) | test(distinct_forms_keep_distinct_pending_child_target_submissions) | test(programmatic_form_submit_keeps_successive_distinct_child_targets) | test(form_top_and_parent_targets_queue_plain_top_level_navigation) | test(renderer_top_level_form_post_preserves_request_through_document_commit) | test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request) | test(base_target_blank_form_post_creates_fresh_target_with_exact_request) | test(named_opener_hyperlink_creation_and_reuse_are_owned_by_renderer_group) | test(named_suppress_opener_hyperlinks_create_distinct_fresh_groups_with_live_names) | test(noopener_and_noreferrer_popups_have_one_real_navigation_with_creator_referrer_policy)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run b226a25f-d8b8-4e8b-bc6f-17a35490c5e1：18 passed。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# run f08691b1-0ee2-4332-a3fb-3c4c0f6fbd8d：16012 passed、18 skipped；执行阶段 101.151s。

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 30s。

git diff --check
# passed
```

Phase 5E2M.1 提交前门禁结果：

```bash
TMPDIR=<repo>/tmp/e2m1-check cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# run df84c664-8487-46ef-96ed-bac2c8cab1b9：16082 passed、18 skipped；执行阶段 98.336s。

cargo fmt --all --check
# passed

TMPDIR=<repo>/tmp/e2m1-check cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 02s。

git diff --check
# passed
```

Phase 5E2M.1 rebase 后集成与门禁结果：

```bash
git pull -r origin master
# origin/master 从 abdb5e8cc4 前进到 d7fb86b60e；49 个 popup 分支提交完成重放。
# 唯一文本冲突位于 script_vm.rs import，合并后同时保留 master 的
# with_scoped_inspector_microtasks 与 popup 分支的 set_object_slot。

TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 \
  loaded_child_document_retains_exact_network_response_body --no-fail-fast
# run ec97fd16-bd36-4ff5-9d3a-0d8d05f06f66：1 passed。
# master 新增的 body-preservation fixture 调用分支已扩展的 child response API 时缺少
# document_referrer；该用例不构造 referrer，显式补 None，而 production caller 继续传
# exact referrer。这是 rebase 后的语义编译冲突，不是用旧签名绕开新 carrier。

TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 \
  -E '<E2M.1 10 条新增 + 7 条既有 owner 回归>' --no-fail-fast
# run adff7e28-bbe6-4a5a-93f7-4f73de32aa11：17 passed。

TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 sandbox --no-fail-fast
# run 352d8e23-7e14-498d-82d1-905222e8376d：44 passed。

for iteration in $(seq 1 20); do
  TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-renderer-v8 \
    -E '<同一 17 条 owner-focused 回归>' --no-fail-fast
done
# first run 36a54882-e0e9-4c4e-80af-ff12ad60b1ed，
# final run 39bb1512-fda3-487c-aeed-559d31af9c74；20/20 轮均为 17 passed。
```

第一次 rebase 后全量命令在进入测试前因工作区文件系统 `ENOSPC` 失败，不能算门禁结果。
当时 `target` 为 191 GiB，其中 `target/debug/deps` 为 165 GiB；先用
`cargo clean -p moli-renderer-v8 --dry-run` 精确确认范围，再执行同一 package clean，删除
316 个可重建 artifact、释放 23.4 GiB，没有清理源码、git 数据或用户目录。随后第一轮有效全量
run `39fd13c7-2e49-4027-8e16-44aac0fd08a3` 为 16102 passed、2 failed、18 skipped：

- file-chooser document-replacement shared-id 用例失败；它曾在旧全量中出现相同单次红灯，本轮单线程
  精确复跑通过，并与下面的 storage case 共同并发压力 20/20 通过；
- `local_storage_mutations_fan_out_across_targets_without_leaking_session_storage` 在默认 nextest
  栈确定性 SIGABRT。rebase 前生产 scheduler 已在 pending wait/completion 两侧使用 heap boundary，
  但 `TestContext` 的 scheduler mirror 仍把二者内嵌进自己的 async state。popup owner/continuation
  与 master commit state 合并后使该测试专用 future 越过栈预算。测试调度器现在与生产路径一致地在
  两侧 `Box::pin`；没有设置 `RUST_MIN_STACK`、新增大栈线程或 sleep/retry。

修正后的精确、压力与最终全量证据：

```bash
TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-protocol \
  local_storage_mutations_fan_out_across_targets_without_leaking_session_storage \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run 74a940e5-d3f6-4ba8-b9ee-37eff62fb3c7：1 passed。

TMPDIR=<repo>/tmp/e2m1-check cargo nextest run -p moli-protocol \
  -E 'test(file_chooser_opened_renderer_backend_node_id_is_scoped_to_document_replacement) | \
      test(local_storage_mutations_fan_out_across_targets_without_leaking_session_storage)' \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 349b0ae6-5312-4e4c-8a39-681f97a70b0d：20/20 iterations passed，
# 每轮 2/2；同时覆盖 file-chooser 单次红灯与 default-stack regression。

TMPDIR=<repo>/tmp/e2m1-check cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 44f5f352-7134-405e-9826-4a0bbc7ce89b：16104 passed、18 skipped；
# 执行阶段 114.590s。

cargo fmt --all --check
# passed；新增 scheduler heap boundary 经 rustfmt 后复检。

TMPDIR=<repo>/tmp/e2m1-check cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 39s。

git diff --check
# passed。
```

完成上述门禁、amend 和第一次 force-with-lease push 后，按本轮收尾约定再次执行
`git pull -r origin master`。第一次连接在 TLS handshake 阶段终止，没有修改 refs 或工作树；重试后发现
master 又从 `d7fb86b60e` 前进到 `106ac477d7`，49 个 popup 提交无冲突重放。该 master 增量只有
`docs/microtask-policy-checkpoint-risk-audit-2026-08-05.md` 一份 582 行文档；直接比较已验证并推送的
`5eeec64859` 与最终 rebase HEAD，tree 差异也只有这份新增文档。Rust、构建配置和测试代码完全相同，
因此不为 docs-only rebase 机械重跑 Rust 门禁；上面的 16104/16104、fmt 和 clippy 结果继续对应最终
代码内容。

Phase 5E2N 提交前集成与门禁结果：

```bash
TMPDIR=<repo>/tmp/e2n-build cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# 首次 run 7c364d36-d1c1-421e-8711-72f1e7fe8830：16109 passed、3 failed、18 skipped。
```

首次 full run 的三条 failure 分开处理，不能笼统归为“偶发”：

- Chromium-imported `rust_cdp_chromium_target_window_open_javascript_url_still_reports_popup_target`
  是 E2N 的真实回归。renderer 已正确保留 requested URL，但 protocol 把所有 `NoDestination` 新 target
  一律投影为空 URL。现已拆分 `destination_request` 与
  `reports_requested_url_without_destination`：JavaScript URL 仍不进入 browser loader，同时
  `Target.targetCreated.url` 为 requested URL；creation-only form 的空 URL/无 scheduler work 不变。上文
  2-case protocol 回归证明两面均成立；
- file-chooser replacement shared-id 与 child-frame unique-context-id 两条用例不经过本轮 popup queue、
  activation projection 或 JavaScript URL executor。它们在 full-workspace 高并发下各缺一次预期 protocol
  observation，但用完全相同 binary 做双用例、2 threads 的 20 轮压力复跑均通过。这个阶段只能证明
  “20 轮未复现”，不能证明 file-chooser 没有竞态；后续最终 rebase 门禁用 50 轮复现出 3 次失败并完成
  owner-boundary 修复，见下一节：

```bash
TMPDIR=<repo>/tmp/e2n-build cargo nextest run -p moli-protocol \
  file_chooser_opened_renderer_backend_node_id_is_scoped_to_document_replacement \
  emitted_child_frame_unique_context_id_selects_that_realm \
  --stress-count 20 --flaky-result fail --test-threads 2
# run 7da6bc55-0a0e-416a-a8a8-68e8e23ff1eb：20/20 iterations passed，每轮 2/2。
```

两条非 E2N failure 在这个历史检查点没有改生产代码或放宽断言；当时证据只支持“workspace 资源/调度下
未复现的既有 timing risk”，不足以声称已找到根因。file-chooser 的这项判断已被下一节更强的复现和修复
证据取代；child-frame unique-context-id 本轮仍没有再次失败。最终默认并发 full run 覆盖它们并全部通过：

```bash
TMPDIR=<repo>/tmp/e2n-build cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 739cc9c4-3140-42cd-8a4c-f6452fab9244：16112 passed、18 skipped；
# 执行阶段 101.818s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/e2n-build cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 43s。

git diff --check
# passed。
```

Phase 5E2N 最终 rebase、file-chooser owner 边界与门禁结果：

```bash
git pull -r origin master
# 第一轮把 popup 分支重放到 origin/master@f22f23462b。50 个分支提交中，旧的
# continuation-policy rebase 修复已被 master 等价包含，Git 自动丢弃该提交，最终保留 49 个。
```

第一轮 rebase 的文本冲突集中在同一批 ownership 演进，不是用一侧版本覆盖另一侧：合并结果同时保留
master 的 structured shutdown `BrowserContext` runtime owner/handle、main Document frozen security
identity 与 transport-error Document URL/security projection，以及 popup 分支的 stable Page/Document
continuation、sandbox policy 和 transient activation。两个 standalone/test-only caller 随新 API 补齐
optional creator/referrer/policy 参数，并像 production 一样保留 `ResourceRequestClientOwner` 与
`RendererBrowserContextRuntimeOwner`，只向 realm bootstrap 传 handle；owner 不再由临时表达式提前释放。

第一轮 rebase 后的相邻聚焦证据为：error-Document/security/lifecycle 5/5（run
`68c4c943-b7dd-4677-af87-b2063c43c4ba`）、renderer JavaScript 32/32（run
`4a83610c-54fb-42b4-a692-66c6b3aa4846`）、protocol JavaScript URL projection 2/2（run
`b3bc0aa6-7954-47fe-9b0e-6492921afb92`）、renderer sandbox/activation 8/8（run
`256c2acb-3041-4584-8ee5-5a0bf1ca5354`）和 protocol sandbox carrier 2/2（run
`260759a2-6e1f-4277-acab-23e831d35328`）。

第一次有效 full run `b4644ad3-9b19-4ba0-91f4-74426164aeca` 为 16133 passed、1 failed、
18 skipped；唯一 failure 是
`file_chooser_opened_renderer_backend_node_id_is_scoped_to_document_replacement`。这次没有沿用上一节
“未复现 timing risk”的结论，而是用相同 binary、4 threads 做 50 轮压力：run
`f0d8995c-c0bd-4a2d-9eb2-b19cdbda9b89` 为 47 passed、3 failed，失败落在第 32、36、43 轮，
确认是可重复的 scheduler race。

根因位于 file-chooser 的事件/DOM-agent 边界，不在 popup queue：renderer 在旧 input element 激活时已经
冻结 exact source Document、frame 和 renderer backend node id，随后同一段脚本可同步调用
`document.open()`。protocol 消费 owner action 时却异步向“当时的 current Document”查询/补登记 BiDi
shared-id，并把登记是否成功当作 automation event 是否携带 `element` 的条件。Document-scoped DOM-agent
binding 可能在 replacement reset 前登记后被清除，也可能在 reset 后落入新 Document，因此测试结果随调度
变化；旧 backend id 在 replacement Document 中不可解析这一核心合同始终成立。

修复没有增加 retry、sleep 或 protocol mirror registry，而是拆开两个不同 lifetime：

- file-chooser event 的 typed `element_shared_id` 永远由 activation 已冻结的 renderer backend identity
  推导并随事件保存；
- renderer DOM-agent binding 仍是 live Document/session 的唯一事实源，登记保持 best-effort，只负责节点仍
  current 时的后续 `input.setFiles` 等操作；`document.open()` 可以退休该 binding，但不能擦除已经接受的
  event identity；
- end-to-end 回归继续断言旧 backend id 对 replacement Document 返回 missing node；producer 回归改为直接
  检查 typed automation sidecar，避免再把 Document-scoped 内部映射的竞态时刻误当成事件合同。

修复后的证据：

```bash
cargo nextest run -p moli-protocol \
  -E 'test(file_chooser_opened_renderer_backend_node_id_is_scoped_to_document_replacement) or \
      test(file_chooser_opened_preserves_typed_automation_sidecar) or \
      test(document_open_replacement_preserves_causal_file_chooser_activation)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run d89a8ca5-dc41-4738-ae32-c13e98261c2f：3 passed。

cargo nextest run -p moli-protocol \
  file_chooser_opened_renderer_backend_node_id_is_scoped_to_document_replacement \
  --stress-count 50 --flaky-result fail --test-threads 4 --no-fail-fast
# run d34eae85-3c81-442d-8a65-8c4fe4f2a1cd：50/50 iterations passed。

TMPDIR=<repo>/tmp/e2n-build cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 567c0c47-dab8-4214-9a98-abde6c9c9738：16134 passed、18 skipped；
# 执行阶段 98.589s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/e2n-build cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 35s。

git diff --check
# passed。
```

按本轮约定，提交修复后再次执行 `git pull -r origin master`。fetch 发现 master 从
`413032799c` 前进到 `d74a055354`；最终相对第一轮 `f22f23462b` 多两项：CDP failed-navigation
error-Document Python smoke，以及 runtime/dynamic script 的 Document referer / CDP `Script` initiator
修复。51 个 popup 分支提交全部无冲突重放；`git range-diff f22f23462b..032a494799
origin/master..HEAD` 的 51 项均为 `=`。最终关键提交为 E2N `967f431ced`、structured test-owner 适配
`8c31e0bf15` 和 file-chooser identity 修复 `a728b5bb38`。

由于第二项 master 增量修改了 E2N 邻接的 document-script scheduler，最终 tree 重新执行 Rust 门禁，
没有把 clean rebase 当作行为证明：

```bash
TMPDIR=<repo>/tmp/e2n-build cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E 'test(runtime_script_uses_document_referer_and_script_cdp_initiator) or \
      test(prepared_runtime_script_start_captures_document_and_base_urls_at_prepare_time)' \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run c1171701-ca84-4797-a0d1-d5e0398c1e3c：2 passed。

TMPDIR=<repo>/tmp/e2n-build cargo nextest run -p moli-renderer-v8 javascript \
  --no-fail-fast --status-level fail --final-status-level fail --failure-output immediate
# run af1ff832-3535-42a0-9571-4c6323aaff64：32 passed。

TMPDIR=<repo>/tmp/e2n-build cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run e5f6c4be-bea3-4804-95d1-99e2255362b7：16135 passed、18 skipped；
# 执行阶段 98.733s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/e2n-build cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 33s。

git diff --check
# passed。
```

Chromium 对照基线仍是 `/home/donoughliu/chromium/src@a03603fe9af6`，本次 rebase 没有基线漂移；
也没有把 master 新增的 Python smoke 当作本轮 popup 验收或重新运行外部 Chrome/WPT。

Phase 5E2O 提交前集成与门禁结果：

```bash
cargo nextest run --no-fail-fast
# 首次 run 5ee77e9a-8af4-4d94-bcc3-6da87dd0ab41：16144 passed、1 failed、18 skipped。
```

唯一 failure 是 `moli-wpt-compat::concurrent_report_writes_never_expose_partial_payloads`。没有把它直接归类
为 flaky：聚焦复跑的 stderr 明确为 temporary report write `ENOSPC`；`df` 显示 `/tmp` 是独立 44 GB tmpfs，
当时可用空间只有 224 KB，而宿主磁盘仍有约 29 GB。该用例和 popup/Trusted Types/form-action 路径没有代码
交集。没有删除 `/tmp` 中来源不明的历史目录，而是创建仓库磁盘上的独立 TMPDIR，再用同一代码重跑：

```bash
TMPDIR=<repo>/tmp/e2o-gate.NW0yVs cargo nextest run -p moli-wpt-compat \
  concurrent_report_writes_never_expose_partial_payloads \
  --stress-count 20 --flaky-result fail --test-threads 4 --no-fail-fast \
  --failure-output immediate
# run aaf6fa25-faa4-4f41-8b77-ef61c9ece2b7：20/20 iterations passed。

cargo nextest run -p moli-renderer-v8 \
  window_open_named_lightweight_popup_reuses_without_recloning_session_storage \
  existing_named_target_does_not_consume_popup_user_activation \
  lightweight_popup_javascript_url_uses_inline_navigation_csp_not_eval_csp \
  popup_policy_checks_keep_existing_and_new_target_order_distinct \
  form_action_csp_runs_after_new_target_selection_and_skips_prevented_submission
# run 44ebba6f-29d3-4c5e-89e0-db5affd00e70：5 passed；覆盖 existing lightweight endpoint
# 不得退化为 creation miss 的最终审阅修正。

TMPDIR=<repo>/tmp/e2o-gate.NW0yVs cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# 最终 run 0d748d9c-1708-49f8-a68a-6246456dbcfe：16145 passed、18 skipped；
# 执行阶段 98.851s。

cargo fmt --all --check
# passed。

cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 30s。

git diff --check
# passed。
```

Chromium 对照仍固定为 `/home/donoughliu/chromium/src@a03603fe9af6`。本轮只阅读该 checkout 与 upstream WPT
source，没有运行外部 Chrome/WPT，因此 focused WPT slice 仍保留为后续证据债。

Phase 5E2O 最终 rebase 与相邻 owner-boundary 门禁结果：

```bash
git pull -r origin master
# origin/master 从 d74a055354 前进到 25326b1c12；53 个 popup 分支提交无冲突重放。

git range-diff d74a055354..a6edc570b1 25326b1c12..6f90fe1b3a
# 53 项全部为 `=`。
```

这次 master 增量是 `7b6328faac test(wpt): refresh cross-engine case baseline`、
`590ee6c64e fix(cache): remove response-path fsync` 与
`25326b1c12 test(cdp): cover fetch runtime callback teardown`。它们没有修改 popup/Trusted Types/form-action
实现；cache 聚焦回归 run `3b474bfd-9bd9-4e47-8c50-25ca01d937e9` 为 44 passed。rebase 后仍重新跑了
最终 tree，而没有把 clean range-diff 当成行为证明。

第一次 post-rebase full run `165d8b5b-59c4-47dc-b2df-0b2f97bfd336` 为 16144 passed、1 failed、
18 skipped。唯一 failure 是
`memory_cache_tee_drop_after_body_eof_cancels_pending_completion_and_releases_exact_runtime`；同一 binary 的
50 轮压力 run `5df9db25-3bdd-4656-b3ed-7ba3b5c12c9e` 在第 22 轮再次失败，不能归为一次性资源抖动。
根因位于 `StreamingRawResponse` 的 terminal/lifetime owner：字段声明顺序会先 drop completion receiver、后 drop
exact transport runtime lease，producer 因此可以观察到 completion 已关闭，但 retired runtime 尚不能 reap。
修复在 `StreamingRawResponse::Drop` 内先取消未完成传输、显式释放 lease，再允许字段析构关闭 completion；没有
增加 sleep、retry 或测试等待。fetch owner 新增单测直接记录 lease-drop 时 completion 是否仍开放，renderer
集成回归继续证明 memory-cache tee 会精确释放旧 runtime。修复后同一 renderer 回归 run
`118e2863-0459-466f-9c40-0b689984a0c8` 为 100/100 iterations passed。

第二次 full run `289ce9d4-db56-4424-ac93-952d4ddfafa4` 为 16144 passed、1 failed、18 skipped；唯一 failure
是 `emitted_child_frame_unique_context_id_selects_that_realm`。该用例在 E2N full workspace 中也曾单次失败，
但当时 20 轮未复现；本次相同 binary 的修改前压力 run
`3ca3f774-3e7e-4f87-b91d-7f19718f5986` 又是 100/100 passed，结合第二次 full failure 说明问题只在
workspace 调度竞争下暴露，不能继续保留“立即扫描测试缓冲区”的偶然假设。

生产 owner 已有 `Runtime.enable` 等待**已经排队**的 exact-Document child-realm task 的 barrier，并有
`runtime_enable_waits_for_queued_child_realm_before_reporting_contexts` 锁住该合同；这里竞态来自 fixture 的
`srcdoc` replacement realm 也可以在 enable response 后通过真实 renderer publication 变成 current。回归现在
在命令成功后从 `TestContext` 的真实 scheduler input 有界等待匹配 frame 的 current default-context event，再用
其 `uniqueId` 做 target-realm evaluation；已有 command-local replay 仍立即满足等待，尚未排队的 replacement
则走 live publication。没有 broad drain、poll budget、sleep 或伪造 context event。

最终证据：

```bash
TMPDIR=<repo>/tmp/e2o-gate.NW0yVs cargo nextest run \
  -p moli-fetch -p moli-renderer-v8 -p moli-protocol \
  -E 'test(dropping_raw_response_releases_lifetime_lease_before_completion_receiver) or \
      test(memory_cache_tee_drop_after_body_eof_cancels_pending_completion_and_releases_exact_runtime) or \
      test(emitted_child_frame_unique_context_id_selects_that_realm) or \
      test(runtime_enable_waits_for_queued_child_realm_before_reporting_contexts)' \
  --no-fail-fast
# run bd0108ae-f685-4669-be2a-056c128a788b：4 passed。

TMPDIR=<repo>/tmp/e2o-gate.NW0yVs cargo nextest run -p moli-protocol \
  emitted_child_frame_unique_context_id_selects_that_realm \
  --stress-count 100 --flaky-result fail --test-threads 4 --no-fail-fast
# run 81eb99a1-0fcf-4794-9731-5ab676f2a60c：100/100 iterations passed。

TMPDIR=<repo>/tmp/e2o-gate.NW0yVs cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run d06465e3-e29f-484c-a335-8d7fd1bd4363：16146 passed、18 skipped；
# 执行阶段 105.246s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/e2o-gate.NW0yVs cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 48s。

git diff --check
# passed。
```

Chromium 对照基线仍是 `/home/donoughliu/chromium/src@a03603fe9af6`；master 增量与相邻 owner 修复都没有改变
E2O 的 Chromium/WPT 行为矩阵。本轮没有运行外部 Chrome 或 focused upstream WPT，因此不能把上述 Rust
门禁写成 WPT pass。

#### Phase 5E3 提交门禁与并发风险

最终 Rust 树的提交门禁为：

```bash
TMPDIR=<repo>/tmp/phase5e cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 0804883b-b735-49ad-ad39-56c24d86fafb：16151 passed、18 skipped；
# 测试执行阶段 96.907s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/phase5e cargo clippy --workspace --all-targets -- -D warnings
# passed；43.65s。

git diff --check
# passed。

git pull -r origin master
# FETCH_HEAD / origin/master 为 25326b1c12；当前分支已经包含该提交，rebase 无冲突、无重写；
# Phase 5E3 实现提交仍为 6aae060677。同步后相对 master 落后 0、领先 57。
```

同步后 `/home/donoughliu/chromium/src` 仍为 `a03603fe9af6`，没有 Chromium 对照基线漂移。因为 rebase
没有改变任何代码或提交，最终树沿用上面的 full nextest/fmt/clippy 证据；本次只新增同步记录，不机械重跑
无关 Rust 门禁。

全量门禁中间有一项不能隐去的既有并发风险：run
`4cdda0f7-af3b-43e7-a865-d378f2151081` 为 16150 passed、1 failed、18 skipped，唯一失败是
`websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded` 未在目标 DCL 采样前观察到 parser
script 的 `Network.requestWillBeSent`。本轮没有修改 `moli` WebSocket fixture；该用例也曾在 E2H、E2J、D1
等 full-workspace 门禁中呈现相同的“workspace 并发失败、focused stress 通过”形状。当前复核为：

```bash
TMPDIR=<repo>/tmp/phase5e cargo nextest run -p moli \
  websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded \
  --stress-count 100 --flaky-result fail --test-threads 8 --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 6a6fceaa-c671-442c-ab65-c7b066536253：100/100 iterations passed。
```

随后两次默认并发 full run 均为 16151 passed、18 skipped；最后一次就是上面的提交门禁。现有证据只能说明它
依赖 workspace 全局调度竞争，不能证明 parser-network backlog 与 DCL publication 的 owner 不变量已经稳定；也
没有据此加入 sleep、retry、降低默认并发或放宽顺序断言。Phase 5L1 已为 dialog、network terminal、unload ACK 和
command response 建立 exact publication fence，并通过 pending Fetch/XHR close 回归；但本轮没有把上述 WebSocket
用例写成根因修复，也没有用 focused 100/100 掩盖剩余 parser-network 风险。

#### Phase 5L1：script-closable、beforeunload/unload 与 renderer ACK closure

这一纵切不是只给 `window.close()` 再加一个条件，而是把“是否接受关闭”和“接受后何时可以销毁 target”做成
完整 Page/browser transaction。它复用 Phase 5B 已有 stable Page liveness、WindowProxy closed facade 和
target teardown，不给 lightweight record 增加第二套 unload owner。

##### 固定 Chromium 对照

对照仍固定在 `/home/donoughliu/chromium/src@a03603fe9af6`：

- `third_party/blink/renderer/core/frame/dom_window.cc:617-677` 先检查 outermost main frame、incumbent
  `CanNavigate`，再以 `OpenedByDOM` / browser setting / history length 决定 script-closable；
  `ShouldClose()` 接受后才 `CloseSoon()` 并同步设置 closing；
- `third_party/blink/renderer/core/loader/frame_loader.cc:1624-1707` 先 root、再 local descendants 执行
  `beforeunload`，任一拒绝即取消；全部接受后才推进 will-unload 状态；
- `third_party/blink/renderer/core/dom/document.cc:4450-4567` 阻止 reentrant `beforeunload`，按
  `returnValue` / `preventDefault()`、sticky activation 和“一次 navigation 最多一个 confirmation”决定是否
  打开 dialog；`document.cc:4569` 以后与 `frame_loader.cc:377-420` 执行非可取消 unload lifecycle；
- `third_party/blink/renderer/core/frame/local_frame_mojo_handler.cc:1236-1264` 在 renderer 运行 unload 后才
  调用 completion callback；`content/browser/renderer_host/render_frame_host_impl.cc:7510-7574` 与
  `render_frame_host_impl.h:4771-4789` 把 browser close 保持在 running-unload 状态，等待 ACK 或 timeout 后才
  最终关闭；
- `dom_window.cc:680-777` 说明 `focus()` 还涉及 activation、opener、Page focus controller 和 embedder；
  `dom_window.cc:779-782` 的 top-level `blur()` 只记录 access metric。因此本轮没有实现一个错误的对称
  blur transaction；该 active Page milestone 后续已由 Phase 5L2 完成。

##### Moli owner 与不变量

1. `RendererPageReservationToken` 和跨 Document 保留的 `RendererPageScriptEnvironment` 现在携带
   `opened_by_dom`。DOM auxiliary 为 true，普通 browser/SW/notification Page 为 false；browser-context
   `allow_scripts_to_close_windows` 默认 false。`window.close()` 只有在 DOM-opened、browser override 或当前
   history length 不大于 1 时才能继续，browser-created multi-entry Page 会保持 live。
2. same-origin、cross-origin related `Window.close()` 都进入最终 target V8 Context。root 先于当前 local
   descendants、descendants 按 document order 接收 browser-created `BeforeUnloadEvent`；无 sticky activation
   时仍发事件但忽略 prompt 请求，一次 close 最多接受一个 confirmation。任一 dialog 拒绝时 Page 不进入
   `Closing`；handler 内 reentrant close 也不会递归取得第二份事务 authority。
3. close 被接受后，protocol 先取消该 target 的 navigation、Fetch/XHR、auth/response-stage 与 inspector await，
   让 terminal output 获得精确 renderer predecessor；随后 renderer 依次执行 root `pagehide`/`unload` 和各 local
   descendant lifecycle，且每个 Document 最多一次。`TopLevelCloseNetworkDrained` 与
   `TopLevelCloseUnloadAck` 是 typed owner action，不靠 sleep、drain loop 或猜测队列为空。
4. `window.close()`、`Page.close`、`Target.closeTarget` 最终仍进入同一个 target termination owner，但保留触发
   来源：renderer Window close 绑定 exact Page residence，晚到记录不能关闭 replacement Page；browser
   Page/Target close 对 target 保持 authority，页面不能靠 navigation 逃避 browser close。renderer unload ACK
   丢失、Page 消失或超过当前 1 秒有界 timeout 时才走 browser force-close fallback。
5. `Page.close` 的 beforeunload dialog 会先发布 exact output prefix，命令 continuation 绑定该 fence；直接
   DevTools `Target.closeTarget` 也把 renderer predecessor 带回统一 ingress。accepted `Page.close` 自身已经
   执行 network drain/unload，而其 stream 中仍有一条 `TopLevelClose(Page)` 记录；protocol 用 exact
   `TargetPageResidenceIdentity` 只抑制这一条重复 owner action，并在 Page replacement/target finalization 时
   清理标记，不能误吞旧 Page 或后续独立 close。
6. renderer publication 不再把 attached `sessionId` 当成 owner identity。可读 session id 会在多个仍存活的
   BrowserContext 中复用；Page stream 因此同时冻结 exact `CdpSessionRoute`，ingress 在该 route 下准备输出，
   `CommandOwnerScope` 再把它带过异步 scheduler turn。`TargetClose` finalizer 也按这个 scope 关闭 inactive
   BrowserContext 的 active/background target；只有当前 BrowserContext 才执行 active navigation-engine handoff
   和 loader refresh。这样旧 context 的 unload ACK 不会命中新 context，也不会把旧 preload/binding registry
   留给 replacement target。
7. HTTP `/json/close` 经 frontend-control actor 执行 `Target.closeTarget` 时，renderer unload 会产生真实
   predecessor。frontend control 现在把该 fence 交给统一 publication flush，并沿用现有 deferred Runtime reply
   与 adapter scheduler 路由，不再断言 frontend command 不可能携带 renderer work。

`Page.close` 会运行 `beforeunload` 并等待决定；`Target.closeTarget` 共用 network drain、unload ACK、timeout 和
最终 teardown，但不伪装成 `Page.close` 的 beforeunload preflight。这个来源差异是显式 contract，不是调用点分叉。

##### Phase 5L1 聚焦证据

新增/加强的边界回归包括：browser-created multi-entry script-close refusal；DOM-opened multi-entry related
popup close；beforeunload 拒绝后再次接受且 unload 恰好一次；无 sticky activation 时不允许 prompt veto；
root/child pagehide/unload 顺序；dialog prefix；三条 pending Fetch/XHR/auth response-stage close terminal；
direct DevTools 与 legacy target close 都等待 unload ACK cursor。

```bash
TMPDIR=<repo>/tmp/popup-lifecycle cargo nextest run \
  -E 'test(close) or test(beforeunload) or test(unload) or \
      test(session_owner_route_override_scope_selects_exact_owner_and_restores_previous_route) or \
      test(command_owner_scope_retains_an_attached_sessions_exact_route_after_ingress_scope) or \
      test(http_cdp_target_management_preserves_browser_discovery_events) or \
      test(http_cdp_target_management_uses_live_agent_host_directory)' \
  --no-fail-fast
# run 00a70b7c-3ac4-43b9-895d-190c968bd8bd：400 passed、15774 skipped；覆盖 workspace
# close/beforeunload/unload、HTTP frontend fence 和 exact attached-session owner scope。

TMPDIR=<repo>/tmp/popup-lifecycle cargo nextest run -p moli-protocol \
  -E 'test(close_aborts_paused_runtime_fetch_subresource) | test(close_aborts_paused_response_stage_runtime_xhr_subresource) | test(close_aborts_paused_runtime_xhr_auth_subresource) | test(devtools_command_executes_target_pending_activate_and_close) | test(devtools_target_legacy_close_drains_runtime_ready_events_without_serializing_them)' \
  --no-fail-fast
# run 81c1b29d-69a6-4f63-ad50-b9f2bd086839：5 passed。

TMPDIR=<repo>/tmp/popup-lifecycle cargo nextest run -p moli-protocol \
  -E 'test(session_owner_route_override_scope_selects_exact_owner_and_restores_previous_route) or \
      test(command_owner_scope_retains_an_attached_sessions_exact_route_after_ingress_scope) or \
      test(patchright_over_cdp_auto_attach_sweep_replacement_targets_keep_thin_handle_cleanup_isolated_per_browser_context_without_runtime_enable) or \
      test(patchright_over_cdp_auto_attach_sweep_replacement_targets_keep_thin_name_cleanup_isolated_per_browser_context_without_runtime_enable) or \
      test(patchright_over_cdp_auto_attach_sweep_replacement_targets_keep_thin_mixed_cleanup_isolated_per_browser_context_without_runtime_enable) or \
      test(patchright_over_cdp_auto_attach_sweep_crpage_replacement_targets_keep_cleanup_isolated_per_browser_context_without_runtime_enable) or \
      test(patchright_over_cdp_auto_attach_sweep_replacement_targets_keep_thin_crpage_cleanup_isolated_per_browser_context_without_runtime_enable)' \
  --no-fail-fast
# owner/Patchright 聚焦 run d3b022eb-8e3d-4f62-90ce-3e3fe3d10604：7 passed、3353 skipped；证明两个 BrowserContext
# 同时保留 Page/session 时，先关闭 inactive owner 不会遗留 target，replacement 仍恢复各自 preload/binding。

TMPDIR=<repo>/tmp/popup-lifecycle cargo nextest run -p moli-protocol \
  -E 'test(session_owner_route_override_scope_selects_exact_owner_and_restores_previous_route) | \
      test(command_owner_scope_retains_an_attached_sessions_exact_route_after_ingress_scope) | \
      test(close_background_target_emits_detached_events_and_clears_attached_sessions) | \
      test(close_target_removes_background_target_without_disturbing_active_target)'
# 最终 Rust 代码 run 4667deff-b4dd-4210-831c-6b88f0a8ea58：4 passed、3356 skipped；证明 attached
# session 与 sessionless route 都保留 exact owner，后台 target close 不扰动当前 active target。

TMPDIR=<repo>/tmp/popup-lifecycle cargo nextest run --no-fail-fast
# 最终 Rust 代码 run 719c05f3-61ec-4380-a006-edc5a4ebc5b8：16156 passed、18 skipped；
# 执行阶段 98.029s。

TMPDIR=<repo>/tmp/popup-lifecycle cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/popup-lifecycle cargo clippy --workspace --all-targets -- -D warnings
# passed；最终 Rust 代码下 45.93s。

git diff --check
# passed。
```

这两组是本地 renderer/protocol owner 回归，不是外部 WPT 结果。L1 当时明确不包含的 browser-context
`focus()` 已由下述 L2 完成；COOP/RemoteFrame beforeunload、lightweight compatibility unload 或 JS-retained
detached realm lifetime 当时都不会因为 local Page close 已闭环而自动成立。后者现已由 P6R3 独立完成。

#### Phase 5L2：browser-context active Page 与 focus transaction closure

这一纵切把 focus 从静态 `document.hasFocus = () => ...` surface override 提升为稳定 Page 的真实状态。它不是
单独补一个 `Window.focus()` callback：initial Page admission、frame focus、target promotion、window state、
emulation、activated creation 和 close 后自动提升必须观察同一个 owner，否则后台 target 仍会同时声称自己
focused，或者 active element identity 会在切换时被错误清空。

##### 固定 Chromium 对照

对照仍固定在 `/home/donoughliu/chromium/src@a03603fe9af6`：

- `third_party/blink/renderer/core/frame/dom_window.cc:680-782`：`focus()` 先检查 live Page/frame，消费
  incumbent Window 的 transient interaction；无 interaction 时只保留 opener exemption。top-level target 交给
  `LocalFrame::FocusPage()` / embedder，top-level `blur()` 本身只记录指标；
- `third_party/blink/renderer/core/dom/document.cc` 的 `Document::hasFocus()` 直接委托 Page
  `FocusController::IsDocumentFocused()`，不是从 JS `activeElement` 属性链猜测；
- `third_party/blink/renderer/core/page/focus_controller.cc` 把 Page active/focused、focused frame 与其 ancestor
  Documents 组合成 `hasFocus()`。Page 失焦保留 active element/focused frame identity，但使 `:focus` /
  `:focus-within` 失效；恢复时复用同一 identity；
- FocusController 的 Page transition 顺序是失焦时 element `blur` / `focusout` 后 Window `blur`，恢复时
  Window `focus` 后 element `focus` / `focusin`。本实现只对 local frame tree 固化该顺序；RemoteFrame/embedder
  分支留给下一阶段。

##### Moli owner 与不变量

1. `RendererPageReservationToken` 分别冻结 initial active-target 与 effective-focus，protocol 在 parser/author
   script 可观察前按 exact target slot 覆盖：active Page 通常为 active+focused，background/auxiliary Page
   通常为 inactive+unfocused。稳定 `RendererPageScriptEnvironment` 跨 Document replacement 保存这两个位，
   不从新 realm 重新推导；focus emulation 只允许 inactive+focused，不会伪造 active target。
2. `DomHost` 保存 Page-focused query state；native selector owner 因而统一控制 `:focus` / `:focus-within`。
   `JsContextHost` 另保存 focused Document，并通过 frame-owner ancestry 计算 root、focused child 与 sibling 的
   `Document.hasFocus()`；protocol 不再以生成脚本覆盖该方法。
3. browser-owned Page transition 保留 `document.activeElement` 与 focused-frame identity，只切换有效 focus 状态并
   触发上述 Chromium 顺序。关闭 active Page 时先执行一次幂等 Page blur，再进入既有 L1
   pagehide/unload/ACK transaction；后台 Page 本来 unfocused 时没有伪事件。
4. same-origin/cross-origin `Window.focus()` 都解析 receiver 的真实 target Context。admission 使用 incumbent
   transient activation 或 target opener edge；跨 Page action 写入 incumbent command/ordinary turn 的唯一 FIFO，
   但 payload 冻结 `RendererOwnerLocalHostId + PageId`。protocol 将它解析成 exact
   `TargetPageResidenceIdentity`，因此正常 source Runtime response 会等待 focus side effect，晚到 action 又不能激活
   同 target id 的 replacement Page。renderer owner 正同步阻塞在 modal prompt 时是明确例外：browser target
   activation 先完成，exact focus/surface command 仍保留在 owner FIFO，prompt settle 后按 Page identity 执行。
5. `Target.activateTarget`、`Page.bringToFront`、renderer `Window.focus()` 和 target close 后的自动 promotion
   共用 `promote_background_target_to_active_for_connection_async()`：旧 active Page 先在自己的 residence 中
   blur，navigation engine/slot 再交换，demoted visibility surface 随后更新，新 Page 最后 focus 并恢复 loader。
   对已 active target 的重复请求只同步 desired native state，不制造第二次 event transition。若共享 renderer
   owner lane 被任一 Page 的 modal prompt 阻塞，旧/新 Page 的 focus 与 visibility 命令均以 non-interruptible
   detached reply 形式排队，WebDriver switch 不会被原 Window prompt 卡死，也不会丢失最终状态变更。
6. `Emulation.setFocusEmulationEnabled` 与 window minimize/restore 不再只修改 JS getter；它们同时更新 surface
   override 和 native Page focus，并等待 Page command completion。显式 focus emulation 是允许 parked Page
   保持 focused/visible 的唯一本地例外；active 位仍为 false，所以带 user gesture/opener exemption 的
   `window.focus()` 会继续进入 promotion transaction。
7. protocol-neutral `CreateTarget { activate: true }` 在交换 slot 前启动带旧 Page generation 的 focus handoff，
   response 前完成旧 Page blur 和 parked visibility，再 materialize 新 active initial Page。这样 WebDriver/BiDi
   creation 不会绕过 CDP activation 的唯一事务；同步 immediate executor 仍只是测试用 staging primitive。

##### Phase 5L2 聚焦证据

新增回归分别锁住 native Page/frame 状态、cross-Page causal owner、三种 activation producer、emulation 与
modal-prompt owner barrier：

```bash
TMPDIR=<repo>/.tmp cargo nextest run \
  -p moli -p moli-protocol -p moli-renderer-v8 \
  -E 'test(top_level_page_focus_preserves_active_element_and_restores_effective_focus) | \
      test(related_cross_origin_window_focus_publishes_target_page_owner_action) | \
      test(document_has_focus_projects_the_page_focused_frame_ancestry) | \
      test(target_activation_moves_native_page_focus_and_preserves_active_element_identity) | \
      test(activated_target_creation_awaits_previous_page_focus_and_visibility_handoff) | \
      test(opener_window_handle_projects_the_renderer_owned_auxiliary_realm) | \
      test(pure_state_emulation_commands_complete_through_command_dispatch) | \
      test(focus_emulation_override_applies_to_document_start_surface) | \
      test(same_context_background_session_can_clear_its_own_touch_and_focus_before_promotion) | \
      test(webdriver_classic_switch_window_keeps_user_prompt_on_original_context)' \
  --no-fail-fast
# 最终 Rust 表示层收口后 run a70f44d8-0cd9-4522-838a-4e50c0343abb：
# 10 passed、11105 skipped。

TMPDIR=<repo>/.tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# 首轮 run 9b51a24a-5e84-4518-bf1c-c7dbe293a497：16160 passed、1 failed、18 skipped；
# 唯一 failure 是既有 parser/network backlog fixture 在全 workspace contention 下的一次调度失败。

TMPDIR=<repo>/.tmp cargo nextest run -p moli \
  -E 'test(websocket_cdp_parser_script_network_backlog_flushes_before_domcontentloaded)' \
  --stress-count 100 --flaky-result fail --test-threads 8 --no-fail-fast \
  --status-level fail --final-status-level fail
# run e55cb10f-e73e-41a1-bd0d-8796d510c452：100/100 passed。

TMPDIR=<repo>/.tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# 最终代码 run 9a709f2c-6d48-4874-9a2c-f4db6167d931：16161 passed、18 skipped；99.024s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/.tmp cargo clippy --workspace --all-targets -- -D warnings
# passed；最终代码下 1m 31s。

git diff --check
# passed。
```

其中 `opener_window_handle...` 同时证明 same-origin opener retained WindowProxy 的 action 在 source command
response 前完成、却激活 target Page；`related_cross_origin...` 证明 restricted Window callback 保留同一规则；
activated creation 回归同时检查旧 Page 的 `hasFocus/hidden/visibilityState`、active-element identity、CSS 和
event order，以及新 Page 的 focused/visible 初始状态。activation 回归还覆盖 parked Page 处于 focus-emulated
focused 状态时，`window.focus()` 仍必须把它提升为 active target；WebDriver 回归锁住 modal prompt 所在旧
Window 不得阻塞 switch，同时 prompt identity 继续留在原 target。

L2 的 exit condition 是 local Page/focused-frame authority 已唯一化，不是“所有浏览器 focus 都完成”。COOP
group switch、RemoteFrame focus endpoint、fenced/embedder activation 和跨进程 focus replication 仍属于下一
个 group/remote milestone；外部 focused WPT 本轮也尚未运行，不能把 Rust integration 回归写成 WPT pass。

#### Phase 5G1：local committed-response COOP browsing-context-group sever

G1 处理的是 group/remote 阶段的第一条完整纵切：renderer-owned top-level Page 收到最终 response、准备
replacement Document 时，若 enforced `Cross-Origin-Opener-Policy` 要求 BrowsingInstance swap，就在旧 realm
仍完整可用时预留一个新 group/agent/isolate，并在 commit 边界原子切断旧 group。它不是把 `opener` getter
改成 `null`，也不创建第二个 protocol Page；同一个 `PageId`、owner residence、CDP target/session 和 Page
scheduler 继续承载导航后的 Document。

##### 固定 Chromium 对照与本轮范围

对照仍固定在 `/home/donoughliu/chromium/src@a03603fe9af6`：

- `content/browser/security/coop/cross_origin_opener_policy_status.cc:32-88` 的
  `ShouldSwapBrowsingInstanceForCrossOriginOpenerPolicy()` 固定了五种 enforced policy 的完整决策：
  `unsafe-none`、`same-origin-allow-popups`、`same-origin`、`same-origin-plus-coep` 和
  `noopener-allow-popups`；它同时区分 initiator 是否仍是 initial empty Document；
- `content/browser/renderer_host/browsing_context_group_swap.cc:49-72` 明确只有 COOP swap 会在 commit 时
  clear proxies 和 `window.name`。security/proactive swap 虽然也可换 BrowsingInstance，却不能直接套用这两项
  side effect；
- Chromium 的 `CrossOriginOpenerPolicyStatus` 会沿 redirect hop 更新 enforced/report-only policy、reporter 和
  virtual browsing-context group。本纵切只实现**最终可提交 response 的 enforced policy**，不伪装成已经实现
  report-only/Reporting API 或 redirect-hop virtual group。

G1 当时只对 potentially trustworthy URL 接受 COOP；`same-origin + COEP` 提升为独立
`SameOriginPlusCoep` policy。其 opaque origin comparison 当时暂时 fail closed；P6R2 已用 group-safe
opaque-origin nonce 替换该临时状态，但 COOP 对 opaque/potentially-trustworthy 的完整外部矩阵仍待 focused WPT。

##### Moli owner、不变量与 commit 顺序

1. `BrowsingContextGroupId` 是 renderer owner 分配的 typed identity，与 `ScriptAgentId` 分离。普通 related
   auxiliary Page 同时加入 opener 的 related group 与 script agent；本轮也修正了旧 admission 只共享 isolate、
   却错误新建 related-group registry 的边界漂移。
2. 稳定 `RendererPageScriptEnvironment` 保存当前 top-level Document 的 enforced COOP value、serialized
   origin 与 initial-empty 位。initial `about:blank` 在 realm 标记后刷新该位，`document.open()` 退出 initial
   状态时再次刷新；related popup 的 initial Document 继承 opener policy，而不是无条件重置为
   `unsafe-none`。
3. response 已知但 replacement 尚未构造时，owner 用当前状态与 prospective final URL/headers 执行 Chromium
   matrix，产出 typed `PreserveBrowsingContextGroup` 或 `CrossOriginOpenerPolicyGroupSwitch`。204/205 等没有
   Document commit 的 terminal 不会仅凭 header 提前切 group。
4. preserve 路径继续借用现有 isolate、stable WindowProxy 和 output stream。switch 路径预留新
   `RendererDocumentIsolateHandle`、script-agent membership、related group、inspector agent、realm/WindowProxy
   和 renderer output stream，但继续使用稳定 Page 的 `PageRuntimeTaskSource` 与 consumer routes。这样 agent
   可以变，navigation handoff/generation 不能从 1 重新开始。
5. preparation cancel/supersession 只回收 provisional isolate/membership/output stream；旧 group、opener、name、
   proxy 和 Page scheduler 不发生变化。只有 replacement 通过 currentness/commit permit 后，旧 default inspector
   Context 才先 detach，旧 related registry 再注销 target、sever opener、断开 Document hosts，并把旧 global
   停驻为 `closed=true`、`opener=null` 的 disconnected facade。
6. 新环境从空 name、无 opener 的 fresh group 开始；旧 group 中已有 JS handle 的 proxy identity 不复活，也不能
   进入新 realm。旧 script-agent membership 和 output stream 只有在新 environment pin 安装到同一 Page slot
   后才退役，失败恢复不能留下“两个 active group”。
7. lifecycle journal 允许 output stream 改变的唯一例外是 typed Page-agent transition，并要求 old/new stream 的
   `RendererPageResidenceIdentity` 完全相同。旧 Document termination 仍从旧 agent stream 发出，新 Document
   lifecycle 从新 agent stream 继续；普通 replacement/adoption 仍禁止偷偷换 stream。
8. protocol 继续观察同一个 target/session：旧 Runtime contexts 被清除，新 default Context 与
   `Page.frameNavigated` 在原 session 发布，不产生第二次 `Target.targetCreated` 或
   `Target.targetDestroyed`。runtime memory diagnostic 新增 `browsingContextGroupId`，使 agent/group 是否同时
   按预期切换可直接断言。

实现过程中三个真实失败也固定了责任边界：related admission 的 group id 最初不相等，说明“共享 isolate”不等于
“同一 browsing-context group”；lifecycle journal 最初拒绝新 inspector stream，说明 stream rollover 必须由
typed same-Page transition 授权；第二次 same-policy navigation 最初复用了新 agent 却重建 Page scheduler，导致
handoff id 回退并被 currentness 拒绝。最终实现分别收回 related-group constructor、lifecycle binding owner 与
稳定 Page task source，而没有在测试或 protocol 调用点绕过断言。

##### G1 聚焦证据与明确保留项

```bash
TMPDIR=<repo>/.tmp cargo nextest run -p moli-renderer-v8 \
  cross_origin_isolation --no-fail-fast
# 8 passed；覆盖 header/trustworthiness/COEP 派生与 Chromium swap matrix。

TMPDIR=<repo>/.tmp cargo nextest run -p moli-renderer-v8 \
  coop_commit_switches_related_page_group_and_disconnects_old_window_proxy \
  --no-fail-fast
# 1 passed；覆盖 provisional cancel、group/agent/isolate/proxy switch、旧 proxy disconnect、
# 同 Page/owner、旧 membership 退役，以及第二次 same-policy navigation 保持 group/scheduler。

TMPDIR=<repo>/.tmp cargo nextest run -p moli-renderer-v8 \
  prepared_live_page_replacement --no-fail-fast
# 5 passed；普通 replacement/cancel 仍保持原 isolate、WindowProxy 与 output ownership。

TMPDIR=<repo>/.tmp cargo nextest run -p moli-renderer-v8 \
  related_page_script_agent_experiment_shares_isolate_and_survives_source_close \
  --no-fail-fast
# 1 passed；修正 related group constructor 后，source close 的既有 agent lifetime 不回归。

TMPDIR=<repo>/.tmp cargo nextest run -p moli-protocol \
  popup_coop_commit_keeps_target_session_and_severs_old_group_proxy \
  --no-fail-fast
# 1 passed；覆盖真实 HTTP COOP response、auto-attach waiting target、同 session context rollover、
# 无 target create/destroy、new opener/name 与 opener-held old proxy closed。

TMPDIR=<repo>/.tmp cargo nextest run -p moli-protocol \
  popup_coop_commit_keeps_target_session_and_severs_old_group_proxy \
  --stress-count 500 --test-threads 8 --flaky-result fail --max-fail 1
# run ecbca0b8-d8b6-41d7-ab95-45c87b2a7b61：500/500 passed。

TMPDIR=<repo>/.tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# 最终 run a19b990c-789d-4077-974a-fa812af654d7：16165 passed、18 skipped；97.806s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/.tmp cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 28s。

git diff --check
# passed。
```

第一次全仓 run 714b3a40-26f8-4ff8-910f-4caef693a79d 暴露了新 protocol 回归自己的等待条件错误：它在
`Runtime.executionContextCreated`/target `loaded_page` 后立即读取 body，但 parser 尚未到
`Page.loadEventFired`，因此约 1-2% 的重复运行会在新 realm 中读不到 `#coop-marker`。这不是通过 sleep 修复；
回归改为等待其实际断言所需的 load lifecycle，随后完成上述 500/500 和全仓通过。第二次全仓 run
47e1ef83-a827-44c3-b4e8-37f99dfdddb0 的唯一失败是未修改的
`add_script_run_immediately_creates_top_level_world_even_when_child_world_name_matches` 在 contention 下失败；
聚焦 run 190c814e-3403-489c-9e67-111dd1798e02 为 500/500，最终全仓复跑通过，未把无关 isolated-world
修复混入 G1。

##### G1 rebase：shared-isolate pause bridge 与 exact Debugger response ordering

G1 提交后按 topic 约定执行 `git pull -r origin master`，从 `origin/master@25326b1c12` rebase 到
`origin/master@45c532c5eb`，61 个 popup topic commit 被重放。master 新增
`fix(cdp): order debugger transitions after response`，它原先建立在“一份 pause bridge 只有一个 Page route”的
模型上；popup topic 则已经让 related Pages 共享 isolate/pause loop，并用
`RendererDevToolsAgentToken -> Page output journal` 保存多个 route。冲突不能选择任一侧覆盖另一侧，否则分别会
丢失 response ordering 或重新引入 opener/popup route replacement。

最终合并保持以下 ownership：

1. nested V8 pause loop 仍是 isolate/script-agent scope，一次只 dispatch 一个控制命令；
2. `Debugger.resume`/step transition 在 dispatch 时只 snapshot **发起命令 agent** 已报告 pause 的 frontend
   sessions。同一 Page 的 primary/attached sessions 共享 exact renderer call cause；related Page 即使同一 isolate，
   也不跨独立 Page output residence 认领该 cause；
3. `mark_command_response` 同时校验 agent token、Inspector session 和 renderer call id，避免两个 related target 的
   session-local call id 碰撞；
4. `finish_owner_turn` 从 isolate-global clear 改成 agent-scoped terminal。popup 的普通 owner settlement 不能提前
   清除 opener 的 step transition；发起 Page 到达 turn terminal 时仍会清除没有 repause 的 bounded cause；
5. session/Page detach 从 active/pending transition 精确移除对应 session；关闭 related Page 不终结其他 Page 的
   transition，关闭发起 Page 又不会留下永远等待的 cause；
6. protocol 继续维持“不同 Page stream 是独立 ordering domain，不用一条 command response 隐式 join 两个
   residences”的既有不变量。master 新增 barrier 只持有发起 Page 的 `AfterCommandResponse` batch。

rebase 还暴露一个测试侧签名冲突：master 新增 barrier 回归显式传入 `source_renderer_agent`，而 popup topic 已把
agent identity 收进 `RendererProtocolObservation::RuntimeInspector(batch)` 并删除重复参数。最终回归改用三参数
constructor，没有把旧 carrier 重新加回生产 API。

本次 post-rebase 聚焦证据：

```bash
TMPDIR=<repo>/.tmp cargo nextest run -p moli-renderer-v8 \
  -E 'test(/script_vm::inspector_pause::tests/)' --no-fail-fast
# run 4dc4c99b-c7be-4685-8bef-3a37699b49e3：17 passed。

TMPDIR=<repo>/.tmp cargo nextest run --no-fail-fast \
  -p moli-protocol-cdp -p moli-protocol -p moli \
  -E 'test(debugger_execution_controls_admit_exact_command_output_barriers) | \
      test(renderer_inspector_batches_keep_their_command_response_side) | \
      test(debugger_transition_messages_wait_for_the_exact_command_response) | \
      test(websocket_cdp_debugger_step_out_responds_before_resumed_and_caller_pause)'
# run 8d285de8-a8a3-421c-a009-390eee80ca22：4 passed。

TMPDIR=<repo>/.tmp cargo nextest run -p moli \
  websocket_cdp_debugger_step_out_responds_before_resumed_and_caller_pause \
  --stress-count 500 --test-threads 1 --flaky-result fail --max-fail 1
# run a1746b4a-3946-4fc0-af87-290151c1d33f：500/500 passed。

TMPDIR=<repo>/.tmp cargo nextest run --no-fail-fast \
  -p moli-renderer-v8 -p moli-protocol \
  -E 'test(coop_commit_switches_related_page_group_and_disconnects_old_window_proxy) | \
      test(popup_coop_commit_keeps_target_session_and_severs_old_group_proxy) | \
      test(/prepared_live_page_replacement/) | \
      test(related_page_script_agent_experiment_shares_isolate_and_survives_source_close) | \
      test(coop_group_swap_matrix_matches_chromium_for_committed_and_initial_empty_documents)'
# run 1b6cdb06-183c-4279-95c8-b57560c7f52e：9 passed。

TMPDIR=<repo>/.tmp cargo nextest run -p moli-protocol \
  popup_coop_commit_keeps_target_session_and_severs_old_group_proxy \
  --stress-count 500 --test-threads 8 --flaky-result fail --max-fail 1
# run c423cef4-674d-4a0f-a8a2-17c52a5392ee：500/500 passed。

TMPDIR=<repo>/.tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# run 16845c59-1653-47ef-9009-da8648fe4b95：16174 passed、18 skipped；98.943s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/.tmp cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 31s。

git diff --check
# passed。
```

Chromium 对照 checkout 仍是 `a03603fe9af6`；这次只同步 Moli master，没有对照基线漂移。post-rebase
workspace 门禁已按新 Rust 基线完整复跑，不能沿用 rebase 前的 16165/18 结果。

这些是 renderer/protocol integration evidence，不是 upstream WPT 结果。G1 的 exit condition 是“本地真实 Page
在最终 response commit 时有唯一 group/agent sever transaction”，不是完整 COOP/remote 结束。G1 入库时为后续
G2-G6 保留了以下范围；紧随其后的阶段记录说明哪些已经关闭、哪些仍在推进：

- redirect-hop enforced/report-only matrix、virtual group、Reporting endpoints 与 report dispatch；
- 对 navigation error Document、Fetch response-stage override、sandbox/COOP interaction 的 Chromium 最小探针；
- 不把 disconnected facade 等同于目标 V8 object：G4 已补 local/disconnected message/location/close/focus
  routing 与 endpoint generation，G5 又完成 same-group cross-agent top-level transport，真正 cross-process/
  RemoteFrame 进入 G6；
- RemoteFrame/fenced/embedder `CanNavigate`、跨进程 activation/focus/unload replication；
- protocol opener/group projection 是否随 sever 更新的明确契约；
- P6R2 已完成 group-safe opaque-origin nonce，P6R3 已完成 JS-retained detached DOM/realm lifetime；Phase 6
  现在进入 compatibility reachability 与依赖层删除。

#### Phase 5G2：redirect-chain COOP status、report-only virtual group 与 exact output reservation

G2 不再把 COOP 当作“最终 response headers 的一个布尔开关”。它把一次 top-level navigation 从当前
Document 出发、经过零到多个 redirect response、再到 terminal response/error Document 的全过程建模为一个
navigation-owned status。真实 browsing-context-group 是否切换、report-only virtual group 如何推进、上一 hop
与下一 hop 分别向哪个 endpoint 报告，以及最终 Document 要保存什么 COOP state，都从这份 status 产出；protocol
不能在 Fetch interception 或错误页路径重新推导第二份结果。

##### Chromium 对照与 G2 选择

对照仍固定在 `/home/donoughliu/chromium/src@a03603fe9af6`，本轮重点核对：

- `content/browser/security/coop/cross_origin_opener_policy_status.cc:145-178`：`SanitizeResponse()` 在普通
  header sanitation 之外，单独处理“继承 sandbox flags 的 popup 导航到非 `unsafe-none` COOP”这一阻断路径；
  即使 response 被阻断，也必须强制真实 BrowsingInstance 与 virtual group 同时切换；
- 同文件 `EnforceCOOP()` 的 redirect 累积规则：每个 response 都以“上一份 current policy/origin/reporter”为
  source 计算，真实 swap 使用 OR 累积，随后才把 current 更新成该 response；因此第一个 redirect mismatch
  不能被最终 response 与原 Document 恰好匹配所抵消；
- report-only 不能直接切真实 group。Chromium 同时计算 report-only→report-only、enforced→report-only 和
  report-only→enforced 三个 matrix，只有 deployment mismatch 成立时推进 virtual group；一旦真实 swap 已在
  earlier hop 成立，后续 response 也继续推进 virtual identity；
- `SetReportingEndpoints()` 与 `cross_origin_opener_policy_reporter.cc`：endpoint 只从 potentially trustworthy
  response source 建立，navigation-to/from report 需要按 origin/source 关系清除敏感 URL，并使用该 response
  reporter，而不是最终 Document 提交后再回读 headers。

Moli G2 实现上述 redirect-chain enforced swap、主 report-only virtual group 与 navigation report 的
可观察语义。Chromium 另有 feature-controlled `same-origin-allow-popups-by-default` 第二 virtual group、完整
ReportingService source/NetworkAnonymizationKey 生命周期和 Window access reports；这些没有被 G2 冒充完成，留在
后续 Reporting/remote closure。

##### 唯一 carrier、owner 与提交顺序

1. `RendererMainDocumentCommit` 新增 typed `navigation_redirect_chain`。网络 owner 把每个 redirect 的 URL、
   status/headers 与 terminal response 一起送进 renderer preparation；普通 buffered/streaming body、Fetch
   response-stage continue/fulfill 和 transport failure 构造的 browser-owned error Document 都保留同一 chain，
   不再只有 root lifecycle 知道 redirect 发生过。
2. renderer Page 保存完整 `TopLevelDocumentCrossOriginOpenerPolicy`：enforced/report-only value、已解析 endpoint、
   serialized origin、Document URL/referrer、initial-empty 位和 virtual group id。related popup initial empty
   Document 仍继承 opener state；真正 response commit 才安装 navigation 计算出的下一份 state。
3. `evaluate_cross_origin_opener_policy_navigation()` 先 snapshot 当前 committed state，然后依次消费每个
   redirect response 和 terminal response。每一 hop 都在更新 current 之前计算 enforced、report-only-to/from
   mismatch；真实 swap 只会由 `false` 累积为 `true`，virtual group 按 Chromium 条件分配下一 identity，最后
   产出一个 `CrossOriginOpenerPolicyCommit::Navigation { state, reports }`。
4. navigation-to/from COOP report 在相应 response endpoint 尚可解析时冻结为 typed fetch request：`POST`、
   `application/reports+json`、`no-cors`、same-origin credentials、redirect=`error`，并清除 URL credentials、
   fragment 和不应跨 origin 暴露的 previous/next URL。新 Document 安装 state 后由其唯一 loader 提交这些请求；
   loader 失败只形成 diagnostic，不改变已经决定的 commit/group transaction。
5. cumulative enforced swap 继续复用 G1 的 provisional group/agent/isolate/realm/output-stream preparation。
   redirect chain 中任何 hop 要求 swap，即使 terminal response headers 为 `unsafe-none`、被 Fetch fulfill 替换或
   最后成为 transport-error Document，commit 仍只在同一 Page/Target/session 上执行一次 sever；204/205 等没有
   Document commit 的 terminal 仍不会提前切 group。
6. Fetch fulfill 回归同时暴露了一个独立的 authoritative-URL bug：paused body 已携带 redirect 后的 final URL，
   synthetic fulfill 却重新使用 navigation requested URL。G2 改为从 `DocumentBodySource::final_url()` 生成 commit，
   所以 realm URL、COOP response origin 与 protocol frame URL 不会在 interception 后退回 redirect 起点。
7. COOP preparation 会在同一逻辑 Page 上短暂存在多个 provisional output stream。旧实现只用 `PageId` 保存
   protocol owner reservation；旧 preparation 的 delayed release 因而可能删掉更新 navigation 刚写入的 owner，
   让新 stream 在第一条 publication 时无 target。现在每次 bootstrap 先分配
   `RendererPageOutputOwnerReservationId`，reservation token、stream identity、protocol binding 与 release marker
   全程携带同一个 id；ingress 只允许 exact `(residence, reservation_id)` claim/release。generic Page adoption 只
   保留给已经构造完成且没有 creation token 的 compatibility 路径，production navigation 不再通过 Page identity
   猜 owner。

由此形成四条新的强不变量：

- redirect hop 的 enforced mismatch 是 monotonic navigation fact，不能被 final response 或 Fetch override 抹掉；
- report-only 只改变 virtual group/report，不触发真实 opener sever；真实 swap 后的后续 hop 仍推进 virtual identity；
- error Document 可以消费已决定的 group transaction，但没有 Document commit 的 terminal 不能消费它；
- owner reservation 的释放作用于一次 exact bootstrap，旧 generation 永远不能清除同 Page 的新 generation。

##### G2 聚焦证据与明确保留项

```bash
cargo nextest run -p moli-renderer-v8 \
  cross_origin_isolation --no-fail-fast
# run 671dbcee-044e-4f33-9ada-f4fd12dc05b5：11 passed；覆盖 header/endpoint 解析、enforced
# Chromium matrix、redirect 累积和 report-only virtual group/report。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(newer_live_page_replacement_reservation_supersedes_unconsumed_nonce) | \
      test(canceling_prepared_live_page_replacement_preserves_page_environment_and_output_stream)' \
  --no-fail-fast
# run 6db5be58-2f62-4af0-a48e-cb708c757e1d：2 passed；同一 Page 的 overlapping/canceled
# replacement 保持 exact reservation 与旧环境 currentness。

cargo nextest run -p moli-protocol \
  -E 'test(popup_coop_redirect_survives_fetch_response_override_and_severs_old_group_proxy) | \
      test(popup_coop_redirect_then_transport_error_still_severs_old_group_proxy) | \
      test(stale_page_release_cannot_clear_newer_same_page_owner_reservation) | \
      test(exact_reservation_release_clears_many_concurrent_page_owners)' --no-fail-fast
# run 021a2375-172e-409c-9fb4-0b53844be197：4 passed；真实 HTTP redirect 分别覆盖 Fetch
# response-stage fulfill 与 transport-error Document，均保持 stable Page/Target/session、new realm marker、
# opener sever/name clear 和 old proxy closed；另两条锁住 exact claim/release，旧 release 不误删 newer owner。

TMPDIR=<repo>/tmp/phase5g2-nextest cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# run b3069076-6fbd-42db-9a24-e8dfb2a9382c：16179 passed、18 skipped；99.835s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/phase5g2-nextest cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 30s。

git diff --check
# passed。
```

protocol Fetch integration 的同步 Page scheduler/loader 路径在 nextest 默认 2 MiB test thread 上会超过栈深，
而仓库既有 target integration harness 已统一用 8 MiB thread；回归复用同一个 `target_8mb_stack`，没有用 sleep、
retry 或放宽生命周期断言掩盖问题。外部 focused upstream WPT 本轮仍未运行，上述证据只能证明本仓
renderer/protocol 边界。

第一次 workspace run `ddaa94a1-ad3c-4f33-a41f-0ef055c14ccc` 使用系统 `/tmp`；该 44 GiB tmpfs 在 run
开始前已被历史临时目录占满，BiDi upload fixture 第一个可见失败为 `ENOSPC`。`df` 明确显示 `/tmp` 100%，因此
该 run 在 5569/16179 时中止，不作为代码结果；没有删除这些不属于本轮的临时目录。改用仓库磁盘上的独立
`TMPDIR` 后完成上述全量通过，避免把环境失败误报为产品回归或靠清理未知数据获得门禁。

G2 的 exit condition 是“redirect/terminal/Fetch/error 共享唯一 COOP status 与 exact Page owner”，不是完整
COOP/remote 完成。紧随其后的 G3 关闭了 sandbox sanitation，G4 又关闭了 local/disconnected endpoint
generation 与 routed-operation currentness，G5 再关闭 same-group cross-agent top-level replacement/command；
当前仍必须处理：

- 完整 ReportingService source、partition/NIK、queued delivery、access report 和 SOAP-by-default 第二 virtual group；
- 真正跨进程的 RemoteWindowProxy/RemoteFrame transport 与 process death；G5 已让 typed endpoint 跨本地
  script agent 路由，但没有伪装成 Mojo/IPC 或 remote child tree；
- RemoteFrame/fenced/embedder `CanNavigate`、跨进程 activation/focus/unload replication，以及 protocol
  opener/group projection 契约；
- P6R2/P6R3 已分别完成 group-safe opaque-origin nonce 与 JS-retained detached DOM/realm lifetime；Phase 6
  只再等待当前产品可达 remote 语义和 compatibility caller graph 收口。

#### Phase 5G3：sandboxed-COOP blocked response、redirect stop 与 authoritative error Document

G3 关闭的不是一个 header `if`，而是完整的 response-sanitation transaction。阻断决定必须在 redirect 目标请求、
download/no-content 分类、renderer Document preparation 和 COOP enforcement 之前发生；一旦阻断，原 response 的
Network terminal、browser-owned error Document、real/virtual group switch、旧 opener/proxy sever 与 stable
Page/Target/session 必须是同一次 navigation 的不同可观察面，不能让 protocol 和 renderer 各算一份结果。

##### Chromium/WPT 对照与边界选择

对照继续固定在 `/home/donoughliu/chromium/src@a03603fe9af6`，本轮核对：

- `content/browser/security/coop/cross_origin_opener_policy_status.cc:145-178`：`SanitizeResponse()` 先 sanitize
  COOP headers；若 enforced value 不是 `unsafe-none` 且 pending frame policy 有 sandbox flags，则在调用
  `EnforceCOOP()` 前返回
  `kCoopSandboxedIFrameCannotNavigateToCoopPage`，同时无条件设置 real BrowsingInstance swap 并分配新的 virtual
  browsing-context group；
- `content/browser/devtools/protocol/network_handler.cc` 与
  `content/browser/devtools/devtools_instrumentation.cc`：该阻断对 Network 域表现为
  `net::ERR_BLOCKED_BY_RESPONSE`，`blockedReason` 为
  `CoopSandboxedIframeCannotNavigateToCoopPage`；
- WPT `html/cross-origin-opener-policy/coop-csp-sandbox.https.html`：同一个 response 同时携带 CSP sandbox 与
  enforced COOP 时，popup 必须得到 network error，response body/script 不能成为 committed Document；
- WPT `coop-csp-sandbox-navigate.https.html`：前一个 Document 自己的 response CSP sandbox 只约束该
  Document/Window；它随后导航到没有 sandbox、只有 COOP 的 response 时必须正常 commit/sever，不能把旧 response
  policy 错存成跨 Document 的 frame policy。

G3 明确只实现 enforced COOP sanitation；report-only COOP 不阻断。DevTools CSP bypass 对当前 response CSP 生效，
但不能清除创建 auxiliary context 时已经冻结的 inherited sandbox。完整 Audits issue/console diagnostic、remote
frame policy replication 与 browser-process ReportingService 仍留给后续 milestone。

##### 唯一 sanitizer、transport gate 与 commit 事务

1. renderer 导出 opaque typed `RendererMainDocumentResponseBlock`。`sanitize()` 合并 target Page 保留的
   `RendererAuxiliaryBrowsingContextPolicy` 与**当前有效 response** 的 enforced CSP sandbox，再检查 potentially
   trustworthy response 上的 enforced COOP；protocol/fetch 只消费 typed 结果，不复制 sandbox token 或 COOP
   parser。
2. `moli-fetch::Request` 新增通用 `RedirectResponseFollowPolicy` callback。它不知道 COOP/sandbox，只保证
   callback 拒绝时把当前 redirect response 作为 terminal 返回，并且不发出下一 hop。buffered、HTML streaming、
   raw streaming 与两条 cache-hit redirect 路径都经过这一门槛；protocol 为 top-level navigation 冻结同一
   renderer sanitizer callback。fixture 用服务端原子计数证明 blocked redirect target 的请求数保持为 0。
3. protocol 的 `BlockedMainDocumentResponse` 按 redirect 顺序寻找第一个 blocked response，再检查 Fetch 后的
   effective terminal status/headers。普通 streaming/captured body、request/response-stage fulfill、cache/redirect
   terminal 共用这一步；判定发生在 204/205、download、XML prebuild 和 paused-response provisional Document 之前。
   Fetch 加上的 CSP+COOP 会阻断，先前 redirect 已经决定的阻断也不会被 final response 掩盖。
4. 被阻断 response 的原 URL/status/headers/redirect history 仍发布 `Network.responseReceived`，随后只发布一次
   `Network.loadingFailed(errorText=net::ERR_BLOCKED_BY_RESPONSE,
   blockedReason=CoopSandboxedIframeCannotNavigateToCoopPage)`。内部 error Document body 不再为同一 request
   追加伪 `loadingFinished`；没有 live Network listener 时，completed progress carrier 也保存原 response URL 与
   failed terminal，而不是把 `chrome-error://chromewebdata/` 误报成网络 response URL。
5. protocol 为同一个 navigation 构造唯一 `NetworkErrorPageNavigation::blocked_by_response`，并把 typed block
   写入 `RendererMainDocumentCommit`。renderer replacement admission 先消费 earlier redirect COOP status，然后对
   blocked terminal 强制 real swap + 一次 virtual group allocation；它直接安装 error Document 的 unsafe-none
   state，不把 browser error headers 当成第二次 ordinary COOP response，也不产生额外 navigation report。
6. commit 仍复用 G1/G2 的 provisional group/agent/isolate/realm/output reservation：成功时 Page scheduler
   identity、CDP Target 和 session 不变，新 error realm 的 `opener === null`、name 清空，旧 opener 持有的 stable
   proxy 进入 `closed=true` disconnected 状态；blocked response body/script 从未进入 authoritative Document。
7. response CSP sandbox 不写回 target Page 的 inherited auxiliary policy。反向回归先 commit 一个仅含 CSP
   sandbox 的 popup Document，再由它导航到无 sandbox 的 COOP response；后者正常 commit、sever 和执行 marker，
   锁住 WPT 所要求的 policy-container lifetime。

由此新增五条强不变量：

- response sanitation 早于 redirect follow，blocked hop 不能产生下一跳 cookie/cache/服务端副作用；
- enforced COOP + pending sandbox 才阻断，report-only COOP 不阻断；
- blocked network response 与 committed error Document 是同一 navigation 的两个 URL surface，不能互相冒充；
- blocked terminal 即使最终 commit 的 error Document 为 `unsafe-none`，也必须强制 real+virtual group sever；
- response CSP sandbox 只属于接收它的 Document；只有 auxiliary creation 冻结的 inherited sandbox 跨 replacement
  保留。

##### G3 聚焦证据与明确保留项

```bash
TMPDIR=<repo>/tmp/phase5g3-check cargo nextest run \
  -p moli-fetch -p moli-renderer-v8 -p moli-protocol \
  -E 'test(request_redirect_response_policy_can_stop_follow_before_next_exchange) | \
      test(coop_response_is_blocked_by_response_or_inherited_sandbox_before_commit) | \
      test(report_only_coop_and_bypassed_response_csp_do_not_block) | \
      test(blocked_response_forces_real_and_virtual_group_switch_without_enforcing_error_coop) | \
      test(popup_sandboxed_coop_redirect_is_blocked_before_follow_and_commits_one_error_document) | \
      test(popup_fetch_effective_sandboxed_coop_response_uses_the_same_blocked_terminal) | \
      test(popup_response_csp_sandbox_does_not_block_later_unsandboxed_coop_navigation)' \
  --no-fail-fast
# run 27641c98-fd15-493c-b699-0a3cb3ab43f3：7 passed；同时锁住 response CSP 不跨
# Document 持久化、原 blocked response URL/status、唯一 loadingFailed/no loadingFinished 和 redirect target 零请求。

TMPDIR=<repo>/tmp/phase5g3-check cargo nextest run -p moli-protocol \
  popup_response_csp_sandbox_does_not_block_later_unsandboxed_coop_navigation \
  --stress-count 100 --test-threads 8 --flaky-result fail --max-fail 1
# run 58e9343f-60be-4c96-ab09-474160d124af：100/100 passed。

TMPDIR=<repo>/tmp/phase5g3-nextest cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# pre-rebase run a0455a1d-4ab1-4b98-999b-228f356e2f09：16186 passed、18 skipped；99.088s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/phase5g3-nextest cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 30s。

git diff --check
# passed。
```

G3 提交后按 topic 约定执行 `git pull -r origin master`。`origin/master` 从 `45c532c5eb` 前进到
`2a79fed82e`，64 个 popup topic commit 全部重放，没有跳过提交。master 增量包含一项 renderer lazy
Window surface realm-owner 修复和五项 agent-episode benchmark/docs 改动。唯一文本冲突发生在
`moli-renderer-v8/src/runtime/tests.rs` 的 helper 插入点；合并结果同时保留 master 的
`assert_window_performance_surface_for_test` 和 popup topic 的 related-script-agent memory/accounting helpers，
没有选择性删除任一侧测试。因为该冲突涉及 Rust 测试结构，最终 tree 重新执行完整门禁：

```bash
TMPDIR=<repo>/tmp/phase5g3-rebase-nextest cargo nextest run --no-fail-fast
# run f020b220-fcc8-49ab-9871-2f1e6112d2af：16187 passed、18 skipped；101.347s。

cargo fmt --all --check
# passed。

TMPDIR=<repo>/tmp/phase5g3-rebase-clippy cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 32s。
```

最终分支相对 `origin/master@2a79fed82e` 落后 0、领先 64。Chromium 对照 checkout 仍固定在
`a03603fe9af6`，本次 Moli rebase 没有改变对照基线。

上述 protocol 回归使用真实 HTTP redirect、Fetch response-stage fulfill、Page/Runtime/Network CDP events 和真实
auxiliary Page replacement；它们不是纯 header parser 单测。外部 focused upstream WPT 本轮仍未运行，因此不能把
本地 7 条回归写成 WPT pass。G3 的 exit condition 是“sandbox+enforced-COOP blocked response 已成为完整 local
navigation transaction”，不是 remote/Reporting closure。紧随其后的 G4 已关闭 local/disconnected endpoint
generation/currentness，G5 又关闭 same-group cross-agent top-level replacement/command，但后续仍保留：

- 完整 ReportingService source/queue、partition/NIK、access report 与第二 virtual group；
- 真正跨进程 RemoteWindowProxy/RemoteFrame transport、process death 与 agent reunification；
- RemoteFrame/fenced/embedder `CanNavigate`、policy/activation/focus/unload replication；
- P6R2/P6R3 已分别完成 group-safe opaque-origin nonce 与 JS-retained detached DOM/realm lifetime；下一阶段进入
  Phase 6 compatibility reachability/removal。

全量门禁提供了两项额外证据。第一次 build 在 `moli-core` test target 发现新增可选 redirect-policy 参数的
两处 caller 漏传，以及两处机械 `None` 落到相邻 inline-page 调用；只修正四个测试签名后重新编译，production
crate 没有对应失败。随后 run `b2bbf80b-c59d-4ee5-884f-8565e5e4f299` 为 16185 passed、1 failed、18 skipped：
唯一失败是 WPT 反向控制看到 final `Page.frameNavigated` 后立即 evaluate，并发下尚未等到 final default realm/load，
测试退出还打断 background continuation。fixture 改为 `final frame → final default context → final load → scheduler
idle` 的确定边界，没有增加 sleep/retry 或修改 production lifecycle；100/100 压力通过后才得到上述最终 full
baseline。

#### Phase 5G4：group-qualified WindowProxy endpoint generation 与 disconnected routing

G1 的 parked facade 已经能让 old-group proxy 观察到 `closed=true`，但 owner 边界仍不正确：cross-origin surface 的
private slot 直接保存目标 `v8::Object`，每次调用再从该 object 的 creation context 反查 `JsContextHost`。这把“一个
group-visible WindowProxy endpoint”误写成“恰好还在同一 isolate 里的目标 V8 global”，既不能表达 generation，也
无法证明 Page residence 被 COOP replacement 复用后旧 proxy 不会路由到新 realm。`postMessage` 还有更具体的错误
风险：target-host 解析失败时会回退到 ambient host，把本应丢弃的 stale message 变成 opener 对自己的消息。

##### Chromium 对照与本轮边界

对照继续固定在 `/home/donoughliu/chromium/src@a03603fe9af6`。本轮核对：

- `third_party/blink/renderer/bindings/core/v8/remote_window_proxy.{h,cc}`：remote endpoint 创建独立
  `RemoteContext`，Local/Remote swap 转移或复用 outer global proxy，不能把旧 LocalWindow global 当 endpoint；
- `third_party/blink/renderer/core/frame/remote_dom_window.{h,cc}`：`RemoteDOMWindow` 本身没有 local
  `ExecutionContext`；`postMessage` 在 source task 接受后再 forward，target frame 已 detach 时丢弃；
- `third_party/blink/renderer/core/frame/remote_frame.cc`：navigation、postMessage、focus 和 opener replication 都
  通过 frame/token owner 路由，而不是从一个跨 Realm JS object 取 local host；
- `third_party/blink/renderer/core/frame/dom_window.cc` 与 `web_frame_test.cc`：close/focus 先做当前 endpoint admission，
  remote close 是 owner request；COOP 旧 proxy 可继续被 JS 强引用，但不再获得 replacement group 的 local authority。

Moli G4 关闭 typed identity/currentness 和 disconnected drop 这一层，不冒充已经有 Chromium 的跨进程 frame
IPC。G4 入库时 live related endpoint 仍只在同一 script agent 中 materialize；紧随其后的 G5 已把这个 admission
扩展到 same-group cross-agent top-level typed command/ACK。真正跨进程 transport、RemoteFrame/fenced policy
replication 和 process-death disconnect 仍明确留给 G6。

##### Endpoint owner、V8 projection 与统一路由

1. `TopLevelWindowProxyEndpointId` 是 `(BrowsingContextGroupId, generation)` 的 typed pair；generation 非零并由
   `RendererRelatedPageGroup` 单调分配。同一 group 的 top-level target 不会别名，另一个 group 即使复用 protocol
   Page residence 或相同 generation 数值也不是同一 endpoint。
2. group 同时持有 `generation → Weak<TopLevelTargetState>` registry。target state 保存 exact Page residence、stable
   outer proxy、current default Context、opener/name/lifecycle；resolver 只接受 pair 完全相等、state 为 `Active`、proxy
   与 current Context 都仍存在的目标。`Closing`、`Closed`、`Disconnected` 或 owner 已释放一律解析失败。
3. normal cross-document replacement 复用同一个 target state，因此 endpoint pair 与 WindowProxy identity 都保留；
   canceled provisional COOP replacement 不消费旧 endpoint。真正 COOP commit 创建 fresh group/state/pair，并在替换
   realm 可观察前把 old state 标记 `Disconnected`。diagnostic 新增
   `topLevelWindowProxyEndpointGeneration`，与已有 `browsingContextGroupId` 一起能验证 pair，而不拿 V8 identity hash
   充当 owner identity。
4. Page-owned cross-origin Window surface、outer proxy 与 Location proxy 的 target-routing private data 只保存两个
   不可由网页脚本访问的 BigInt wire parts，不再保存目标 V8 object。callback 从 incumbent/source Context 解析
   observer identity，再由它的 group registry 取得 exact active target；取到的 raw host pointer 只在该 callback
   内使用，不缓存、不泄漏到 task。没有 `RendererPageScriptEnvironment` 的 standalone `ScriptVm` 不属于 related
   Page group：它只安装 local-top boolean marker，并且只能从自身 creation Context 解析 local host；diagnostic
   generation 保持 `0`，不能为了复用 surface 人工制造 group endpoint。
5. child count/index/name projection、`closed`、动态 `opener`、Location assign/replace、`close()`、`focus()` 与
   `postMessage()` 共用这条 endpoint currentness。live target 继续复用既有 CanNavigate、close、focus 和 Page task
   owner；stale endpoint 表现为 `closed=true`、`length=0`，所有 routed operation 静默丢弃。`opener` 不被
   currentness fallback 重新推导：COOP sever 返回 `null`，普通 final close 仍保留既有 opener edge。
6. `postMessage` 对 related top endpoint 不再使用 `target_host.unwrap_or(ambient_host)`。endpoint marker 存在但 exact
   target 不可解析时立即返回，不能排入 opener 的 `TopWindow` queue；live related message 仍在 source callback
   structured-clone，并由 target Page 的 window-message task source 投递，已有 `event.source === opener` 语义不变。
7. protocol 没有重复推导 endpoint。真实 COOP redirect + Fetch response-stage override 回归在同一 stable CDP
   target/session 上让旧 proxy 依次调用 postMessage、Location assign/replace、close、focus，然后证明 opener 没收到
   self-message、replacement URL/Document/closed state 未变化、target 仍只有一个。

由此形成六条新的强不变量：

- endpoint identity 由 browsing-context group owner 分配；V8 object identity 和 Page residence 都不能替代它；
- normal Document replacement 保留 exact pair，COOP group switch 必须更换 pair；取消 preparation 两者都不改变；
- 所有 cross-origin top operation 先做一次 exact endpoint currentness，不能各自反查 target creation context；
- stale/disconnected endpoint 永远不能命中同 residence 的 replacement realm，也不能回退成 incumbent self-operation；
- group-qualified endpoint 只属于 Page-group owner；standalone realm 继续是明确的 local-only route，不能成为
  synthetic group 或绕过 exact endpoint admission 的 compatibility 后门；
- G4 的 local resolver 是未来 RemoteFrame transport 的 admission 边界，不是跨进程实现完成声明。

##### G4 聚焦与压力证据

```bash
cargo nextest run -p moli-renderer-v8 \
  coop_commit_switches_related_page_group_and_disconnects_old_window_proxy
# run 517c6133-b04e-43d7-a512-d8ecb49cfccd：1 passed；覆盖 canceled/same-group endpoint
# pair 保留、COOP fresh pair、四类 stale operation、closed/opener/length 和 replacement currentness。

cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_script_agent_exposes_chromium_cross_origin_window_proxy_surface) | \
      test(related_cross_origin_window_focus_publishes_target_page_owner_action) | \
      test(related_page_window_close_is_synchronous_idempotent_and_disconnects_final_realm)'
# run fe84ef7d-9ac5-44a1-8038-836568d5cc98：3 passed；证明 live endpoint 的 child/name、Location、
# focus 与 close 没有因 identity owner 抽取而退化。

cargo nextest run -p moli-protocol \
  popup_coop_redirect_survives_fetch_response_override_and_severs_old_group_proxy
# run 28cc7a25-33ed-48fe-afa9-b18b62f0ed1d：1 passed；真实 HTTP redirect + Fetch fulfill + CDP
# target/session/realm replacement，并执行 stale operation matrix。

cargo nextest run -p moli-renderer-v8 \
  coop_commit_switches_related_page_group_and_disconnects_old_window_proxy \
  --stress-count 100 --test-threads 8 --flaky-result fail --max-fail 1
# run c54a0a98-f57c-4ce0-8a10-457397c6db4b：100/100 passed。

cargo nextest run -p moli-protocol \
  popup_coop_redirect_survives_fetch_response_override_and_severs_old_group_proxy \
  --stress-count 50 --test-threads 8 --flaky-result fail --max-fail 1
# run c6469dcc-305d-47cd-aada-511ceeaac9be：50/50 passed。

cargo nextest run -p moli-renderer-v8 \
  module_runtime::graph::tests::external_module_root_uses_import_map_integrity_when_element_integrity_is_absent
# run c6dc0f05-b510-4a06-a17c-74b111b015b0：1 passed；证明 owner-less standalone realm
# 不要求或伪造 Page-group endpoint。

cargo nextest run -p moli-renderer-v8 --no-fail-fast \
  --status-level fail --final-status-level fail
# run 8ada8626-efc6-439c-a80c-b465a604a36f：7203 passed、4 skipped。

git pull -r origin master
# Current branch popup-refactor is up to date；相对 origin/master 为 0 behind、65 ahead。

cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
# rebase 后 run 059ae817-f392-444e-a142-2df80d09665d：16187 passed、18 skipped。

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# 均通过。
```

这些证据没有使用 sleep、retry 或 eventually-consistent target-name lookup。外部 focused upstream WPT 本轮尚未
运行，因此不能把本仓 endpoint regression 写成 RemoteFrame/WPT 通过。第一次 full gate 还否定了“每个 main
default realm 都必须有 Page-group endpoint”的错误硬前置：大量 module/WebIDL standalone harness 本来就没有
Page owner。最终实现把该要求收窄到 related Page surface，并用 renderer 全包与 workspace 全量同时锁住这条
边界。G5 已完成下面第一项的 same-process top-level 部分；完整 group/remote exit 仍需：

- 把同一个 typed endpoint admission 从 G5 的 cross-agent top-level owner route 扩展到真正 cross-process
  RemoteFrame/process-death command/ACK transport；
- 完整 RemoteFrame/fenced/embedder `CanNavigate`、activation/focus/unload 和 opener/group policy replication；
- ReportingService source/queue、partition/NIK、access report 与第二 virtual group；
- P6R2 已完成随后的 group-safe opaque-origin nonce，P6R3 已完成 JS-retained detached DOM/realm lifetime；下一阶段
  进入 Phase 6 compatibility reachability/removal。

#### Phase 5G5：same-group cross-agent top-level RemoteWindowProxy transport

G4 已经让一个 top-level WindowProxy 由 `(BrowsingContextGroupId, endpoint generation)` 定位，但 resolver 成功后
仍要求 target LocalWindow 与 observer 位于同一 isolate/script agent。这个限制在 related popup 第一次跨源 commit
时会暴露：如果继续共享 isolate，就没有 Chromium-shaped agent separation；如果只替换 isolate，opener 中已经保存的
proxy、target name、`window.opener`、`MessageEvent.source` 和 protocol Target/Page residence 又会全部丢失。G5
关闭的是这整个 top-level 纵切，而不是只让一个跨源 getter 返回值看起来正确。

##### Chromium 对照与本轮边界

对照继续固定在 `/home/donoughliu/chromium/src@a03603fe9af6`。本轮重点核对：

- `third_party/blink/renderer/bindings/core/v8/remote_window_proxy.cc`：每个 isolate 创建自己的
  `RemoteContext`，同一 projection 内复用 outer global proxy；LocalWindow 的 V8 Context 不是可跨 isolate 搬运的
  browsing-context identity；
- `third_party/blink/renderer/core/frame/remote_dom_window.cc`：`postMessage` 在 source script 返回后才 forward，
  target detach 后直接丢弃，不能同步进入另一个 LocalWindow；
- `third_party/blink/renderer/core/frame/remote_frame.cc`：remote navigation 把 initiator、method/body、referrer、
  user gesture 和 replace disposition 组装成 `OpenURL` owner request；message/focus/opener/name/sandbox/origin 也由
  frame token 与 replicated state 驱动；
- `content/browser/renderer_host/render_frame_proxy_host.cc`：browser owner 解析 exact target host，复核 target
  liveness/current origin/related SiteInstanceGroup，并把 source local token 翻译成 target process 的 remote token。

Moli 没有 Chromium 的 SiteInstance/Mojo process 拓扑。G5 因此实现的是同一 renderer/protocol runtime 内的
**cross-agent top-level transport seam**：V8 handle 已完全隔离，操作必须穿过 typed output、exact protocol target
Page 和 target renderer ACK。G5 交付时 carrier仍是进程内 Rust capability，也没有 remote child `FrameTree`；G6A
随后补 child owner，G6B1 再替换为 process-neutral versioned wire。真实 SiteInstance/Mojo process拓扑仍未实现。

##### Logical target、per-agent projection 与 commit transaction

1. `RendererRelatedPageTopLevelTargetState` 只保存 agent-neutral authority：exact Page residence、group-qualified
   endpoint、name、opened-by-DOM、lifecycle、active/focused、current URL/origin、committed COOP 与 opener endpoint。
   V8 global/context/opener handle 移入 `RendererRelatedPageTopLevelTargetProjection`，按 `ScriptAgentId` 注册；一个
   agent 对一个 logical target 最多有一个 projection。
2. 当前 Page environment 强持有自己的 LocalWindow projection；group registry 只持 weak projection。observer
   environment 强持有自己 materialize 的 remote facade。这样 canceled provisional agent 不会被 group 长期 pin，
   而网页仍持有的旧 agent proxy 会随 logical target 一起停驻为 remote projection。
3. related Page 从已有 HTTP(S) origin commit 到不同 origin，且当前 agent 还有 related peer 时，replacement
   isolation 选择 `PreserveBrowsingContextGroupWithRemoteAgent`：预留 fresh document isolate/script-agent membership、
   inspector backend 和 replacement realm，但复用 Page scheduler、output stream、group、endpoint 与 logical state。
   preparation/cancel 不触碰旧 projection；commit 才 detach old LocalWindow、把旧 stable proxy 重新接到 live remote
   facade，再安装新 agent LocalWindow。
4. COOP group switch 仍走独立 transaction：fresh group/fresh endpoint，旧 logical state 先 `Disconnected`。因此
   “same-group agent replacement” 与 “group sever” 不会因都更换 isolate 而混为一条分支；同一 Page residence 上的旧
   endpoint command 也不能穿透 COOP replacement。
5. `RendererScriptAgentPageMembership` 持有 admission 时冻结的 immutable `ScriptAgentId`。同步 `window.open()`
   在 source V8 callback 内构造 initial target 时不再为查询 agent id 重借已经 entered 的 isolate holder；这不是
   放宽 `RefCell` 检查，而是把身份 capability 放回其 owner 边界。

##### Remote surface、named lookup 与 opener projection

1. observer 第一次解析 remote endpoint 时，在自己的 isolate 创建 host-free restricted Window facade，只写入
   group-qualified endpoint；facade 不持有 target `Context`、target global 或 `JsContextHost*`。同一 observer agent
   后续解析复用同一 global proxy，保证保存值与 named lookup 的 JS identity 稳定。
2. old agent 的 target stable proxy 在 LocalWindow detach 后复用为 remote facade，`closed` 仍为 `false`，
   `window === self`、restricted Location/Window whitelist 和 opener projection 保持；只有 close/COOP/discard 才让
   shared logical lifecycle 使它断连。
3. logical opener 以 endpoint 复制。fresh target agent bootstrap 在 host 尚未绑定进 environment 前，直接使用已验证
   environment materialize opener projection；不能从 ambient host 或 source isolate V8 value 回推。普通 Document
   replacement 与 same-group agent transition 保留 opener，COOP sever 为 `null`。
4. named target resolver 先按 group page order 枚举 logical top-level target，再决定 local 或 remote projection。
   `window.open()`、hyperlink、form 命中 remote top-level 后冻结 exact Page residence 并复用既有 activation/navigation
   handoff，不创建第二个 auxiliary Page。remote child frame-tree 尚未复制，因此当前只能声明 top-level name collision
   正确；不能声明 remote nested named frame lookup 已完成。

##### Typed command、protocol route 与 target ACK

1. `RendererRemoteWindowProxyCommand` 是 move-owned typed carrier，固定 command id、target endpoint、exact target
   Page 和 operation kind。Navigate 还保存 Assign/Replace、绝对 URL 与 source navigation carrier；PostMessage 保存
   source endpoint/Page/origin、structured-clone payload 与 normalized target origin；Focus/Close 不携带 V8 authority。
2. source callback 完成本地 WebIDL/structured-clone、activation/opener 或 `CanNavigate` admission 后，只向 source
   Page output journal 追加 `RendererOwnerAction::RemoteWindowProxy`。它不进入 target isolate，也不预测 target ACK；
   例如 remote `close()` 返回后 source facade 仍 live，直到 target 接受 close transaction。
3. protocol ingress 用 frozen `RendererResolvedPopupTarget` 找 exact active/background target 与 loaded Page generation，
   在不持有 BrowserContext/target-slot borrow 的情况下等待 renderer command completion；ACK 返回后再次检查同一
   Page residence/generation，再由 exact Page finish typed reply。target renderer 最后复核自己的 endpoint、Page
   residence 与 browsing-context lifecycle，任何一层 stale 都返回 `false`。
4. Navigate 在 target Page 内临时安装 source navigation carrier，然后复用既有 cross-origin Location、
   `CanNavigate` 后半段、loader/history owner；PostMessage 在 target owner turn 再复核 source endpoint/Page 与
   target origin，排入 target window-message task；Focus/Close 复用 Phase 5L2/L1 的 exact Page transaction。
5. remote `MessageEvent.source` 在 target agent materialize source endpoint projection；若 source 正是 logical opener，
   event 复用 target realm 的 canonical `window.opener` projection，保持 `event.source === window.opener`、origin 与
   `closed` 观察一致。source Page residence 同 endpoint 一起验证，不能让回收后复用的 target 冒充消息来源。
6. renderer output transport memory accounting 包含 command URL、serialized origin、target-origin 和 structured-clone
   wire bytes；clone/转移 payload 仍由现有 message-port retirement owner 处理，不另建 remote 队列。

由此形成八条新的强不变量：

- logical browsing context 与 agent-local V8 projection 是两层 identity；Page residence、V8 object、script agent
  任一个都不能单独替代 group-qualified endpoint；
- same-group cross-origin commit 更换 script agent/isolate，但保留 Page/group/endpoint/name/opener；COOP commit 更换
  group/endpoint 并让旧 proxy 断连；
- canceled provisional agent 不改变旧 LocalWindow/proxy/opener，也不被 weak group registry 泄漏；
- observer isolate 永远不保存 target isolate 的 V8 handle 或 host pointer；remote operation 必须越过 typed owner
  action 与 target ACK；
- source admission 与 target execution 各做一次 currentness；protocol 等 ACK 时不能持有 target-slot borrow；
- named remote top-level 占据其原 group page-order 位置，不能因缺少 local target Context 而落入 new-popup fallback；
- `postMessage` 同时验证 source endpoint、source Page、target endpoint、target Page 与 target origin，不能回退到
  incumbent self-message；
- COOP/close 后的 stale/in-flight command 即使仍指向同一 Page residence 也必须失败。

##### G5 聚焦证据

```bash
cargo nextest run -p moli-renderer-v8 \
  cross_origin_related_page_commit_moves_local_window_to_remote_agent_and_routes_commands
# run 11b1a446-a050-47ac-888b-628fecb8ec8b：1 passed；覆盖 provisional cancel、fresh agent、
# stable group/endpoint/old proxy、opener、window.open/hyperlink/POST form remote named reuse、
# form body/header、structured-clone MessageEvent.source、focus、Location assign、close 与 target ACK。

cargo nextest run -p moli-renderer-v8 \
  coop_commit_switches_related_page_group_and_disconnects_old_window_proxy
# run ee0309a5-f53a-4536-8c3c-6da587eca76d：1 passed；新增同 Page residence 的 stale in-flight
# remote command，证明 COOP fresh endpoint 在 target 端返回 false。

cargo nextest run -p moli-protocol \
  cross_origin_popup_remote_window_proxy_routes_through_exact_background_target
# run 30fc750f-4517-466c-aae5-a58e29f1b71b：1 passed；两个真实 localhost origin、auto-attached
# popup Page.navigate、same-group agent split、单 background target/named reuse、typed protocol ingress、
# structured message 与 canonical opener source。

cargo nextest run -p moli-protocol \
  named_form_post_reuses_renderer_group_target_and_preserves_exact_request \
  --stress-count 100 --test-threads 8 --flaky-result fail --max-fail 1
# run f59c9a7b-a8d8-4046-977a-37eb56c87d2c：100/100 passed。
```

最后一个用例的异步 `Runtime.evaluate(awaitPromise=true)` 回包通过 production-shaped typed document
continuation scheduler 等待真实 scheduler input；没有 sleep、retry、测试专用 lifecycle drain 或同步直调 target。

```bash
TMPDIR=<repo>/.phase5g5-nextest.JpKlvb cargo nextest run --no-fail-fast
# pre-rebase run c36a3343-8e82-483f-bdcb-70e647759ff5：16189 passed、18 skipped。

git pull -r origin master
# 无冲突；Moli 基线更新到 origin/master@1464757bd8，topic 的 66 个提交完整重放。

TMPDIR=<repo>/.phase5g5-rebase-nextest.svCOJv cargo nextest run --no-fail-fast
# post-rebase run add4c59c-8cf5-43d8-89de-02b4f0d7e5ee：16241 passed、18 skipped。

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# post-rebase 均通过；clippy 1m43s。
```

第一次 workspace run `4c7af533-859a-4f91-8209-82d23fa383f0` 使用系统 `/tmp`；`df` 显示 44 GiB tmpfs
已 100% 占满，最早失败的 BiDi upload fixture 和后续 WPT importer 临时文件均为 `ENOSPC`。没有删除不属于本轮
的历史临时文件；改用仓库磁盘上的唯一 `TMPDIR` 后完整通过。外部 focused upstream WPT 未运行，不能把上述
回归写成 Chromium SiteInstance/RemoteFrame WPT pass。

补强 remote named hyperlink/POST form 覆盖后的第一次 full run
`5aa7f5bd-1326-4ab3-bb94-cf73c81e320e` 为 16188 passed、1 failed、18 skipped。唯一失败是既有 named-form
fixture 已显式启用 document continuation scheduler，却在 cross-agent commit 后仍把最终 inspector reply 当成
立即回包；单独运行通过，但 workspace 并发暴露 sent queue 里只有 lifecycle publication。fixture 改用
`process_and_wait_for_response_async()` 等待指定 command id 的真实 typed scheduler input，不增加 sleep/retry 或
页面 idle。随后 100/100 压力和 pre-rebase full run 均通过。最终 rebase 没有文本冲突；master 新增 52 个
测试，post-rebase full run、fmt 与 clippy 仍全部通过。Chromium 对照 checkout 仍是
`a03603fe9af6230a12f1b2fb2c18a7d003a0d937`，没有基线漂移。

##### G5 明确保留给 G6/identity 的范围

- Rust command carrier 尚未序列化为跨 OS process IPC；没有 process crash、channel disconnect、ACK timeout 与
  remote endpoint teardown 模型；
- remote Page 的 child frame tree、frame token、name/order、sandbox/origin/permissions policy 尚未复制，因而
  RemoteFrame/fenced/embedder `CanNavigate`、child postMessage、focus/unload 仍未完成；
- target 回到与 opener 同源时不会自动 reunify 到原 script agent；正确性当前依赖继续使用 remote projection，资源与
  Chromium agent clustering 仍有差距；
- postMessage 已覆盖 related top-level source/opener；arbitrary remote subframe source token、on-demand opener-chain
  projection、BFCache/pending-deletion delivery 仍属于 RemoteFrame 阶段；
- ReportingService source/queue、partition/NIK、access report 与第二 virtual group 没有因 typed transport 自动完成；
- P6R2 已完成 group-safe opaque-origin nonce，P6R3 已完成真实 local top/child Document/Node/realm lifetime；未来
  不可信跨进程 capability GC/崩溃回收仍属于可选 process-lifecycle milestone。

#### Phase 5G6A：agent-neutral RemoteFrame tree、exact nested route 与 transport teardown

G5 证明了同一 browsing-context group 内的 related top-level Page 可以跨 script agent 保留 stable
WindowProxy，但它当时只复制 main-frame target。remote Page 的任何 nested frame 都对 source agent 不可见：
`popup.length` 固定为 0，named resolver 看到 top-level name 不匹配后只能继续创建新 popup，保存的 child proxy、
frame-target postMessage、POST form scheduler 与 cancellation identity 都不存在。G6A 关闭的是这条完整的
**related remote child owner seam**，不是增加几个 cross-origin getter。

G6 总 exit 仍包括真正 OS process/IPC、process failure、agent reunification、fenced/embedder 和 Reporting。
所以本节明确命名为 G6A；它提供可以被未来 wire transport 复用的 identity/currentness/target-owner 边界，但不把
同一进程中的 Rust carrier 冒充 Chromium 的 SiteInstance/Mojo 拓扑。

##### Chromium 对照与 Moli 的取舍

对照继续固定在 `/home/donoughliu/chromium/src@a03603fe9af6`，本轮重新核对了以下 owner：

- `third_party/blink/renderer/core/page/frame_tree.cc` 的
  `FindFrameForNavigationInternal()` 先查 source subtree、当前 Page 全树，再按 related Page 顺序遍历每棵完整
  frame tree；每个同名 candidate 都要经过 `CanNavigate()`，fenced tree 会在 related-page lookup 前停止；
- `third_party/blink/public/mojom/frame/frame_replication_state.mojom` 与
  `content/browser/renderer_host/browsing_context_state.*` 把 name/unique-name、committed origin、active sandbox、
  frame policy、permissions policy、insecure-request state、activation/ad facts作为 browser-owned replication
  state广播给对应 `RenderFrameProxyHost`；动态 name 与 committed origin 不是 V8 wrapper 的私有快照；
- `third_party/blink/renderer/core/frame/remote_frame.cc` 的 `RemoteFrame::Navigate()` 在 source renderer 构造
  `OpenURLParams`，保留 initiator token/origin/base、method/body/headers、referrer、form/user-gesture、replace
  disposition 和 source location；`ForwardPostMessage()` 同样传 source local-frame token，而不是目标 V8 handle；
- `content/browser/renderer_host/render_frame_proxy_host.cc` 的 `OpenURL()` 在 browser owner 重新验证 URL、active
  Document、related SiteInstanceGroup 与 source token，再把请求交给 exact `FrameTreeNode` navigator；
  `RouteMessageEvent()` 重新验证 target liveness/origin/related group，并把 source local token 翻译成 target
  process 中的 RemoteFrame token，必要时按需创建 source/opener proxy；
- `RemoteFrame::DetachImpl()`、`BrowsingContextState::RenderProcessGone()` 与 proxy host liveness 分别处理 renderer
  facade detach、process-side proxy失联和 browser-owned logical frame state。IPC channel 断开不应把同数字 routing id
  解析成另一个 frame。

Moli 没有 SiteInstanceGroup、RenderFrameHost/ProxyHost 或 Mojo。G6A 因此选择最小但形状正确的分层：
logical related Page 共享 agent-neutral frame snapshots；每个 observer agent 只 materialize自己的 V8 facade；
operation 通过 source output journal/protocol target Page/target Frame scheduler 三段 owner route。G6A 交付时 snapshot
仍是进程内共享 Rust value，`V8StructuredClonePayload` 也可能带进程内 capability；后续 G6B1 已把两者替换为 strict
versioned process-neutral wire，但仍没有实际 Mojo/process channel。

##### Document-qualified frame identity 与 replicated tree

1. `RendererRemoteFrameToken` 由 `(top-level endpoint, root Document lifecycle identity,
   BrowsingContextId)` 组成。child browsing-context id 在当前实现中由 Document host 分配，root replacement 后可能
   重用数字；只用 Page id 或 child id 会让 retained/in-flight command 命中新 Document 的无关 frame。
2. target `JsContextHost` 按实际 document order 发布 `RendererRemoteFrameSnapshot`，只包含 owner-neutral facts：
   exact token、parent id、动态 name、committed URL/origin、`document.domain` override 和 committed policy container。
   本地 `DomHandle`、`JsContextHost*`、V8 Context/global、loader binding 都不允许进入 group state。
3. tree 在 child create/sync/drop、name change、child commit 与 `document.domain` change 后更新；只发布仍 live 且在
   Document tree 中的 browsing context。root Page transition、same-group agent replacement 和 COOP group switch
   在旧 Document teardown 前先清空 outgoing tree，避免新 Document 发布前的短窗口继续路由旧 child。
4. source resolver 每次读取 logical target 的最新 tree；target收到 command 后仍复核 exact Page residence、top
   endpoint、root Document lifecycle、child id 和 child liveness。source snapshot admission 与 target owner
   currentness 是两道独立门禁，不能用其中一道替代另一道。

##### Observer-local stable proxy 与 related named lookup

1. 每个 source script agent 维护 `(RendererRemoteFrameToken → projection id → V8 facade)` 的私有 registry；facade
   只保存 observer-local projection id，callback 再解析 token和最新 snapshot。同一 observer 对同一 token 始终返回
   同一 outer proxy，目标 agent 的 Context/global/host pointer 不会跨 isolate 泄漏。
2. remote top/child 共用既有 cross-origin Window/Location internal-method surface。`length`、numeric index、dynamic
   name、递归 child、`parent`/`top` 都读取 replicated tree；`self/window/frames` identity 仍指向 stable proxy。
   root tree 清空或 frame消失后，保存值 `closed === true`、child count 为 0，Location/postMessage 安全丢弃。
3. named resolver保留 Chromium-shaped order：source subtree/current Page 后，按 related Page order检查 top name，再按
   remote Page document order 检查所有 nested names；candidate 消费 replicated origin/domain/sandbox facts 的
   remote `CanNavigate` 子集。命中后 `window.open()`、hyperlink、form 直接返回/使用 exact remote frame proxy，
   不落入 new-popup fallback。
4. 当前 remote `CanNavigate` 已覆盖 source execution-context currentness、JavaScript URL cross-origin refusal、
   source sandboxed-ancestor refusal和 source 对 target/ancestor/top 的 origin-domain access；fenced root、guest/
   embedder fallback、file-local、完整 top/opener exception 与跨进程 snapshot revision 仍在 G6B/后续收口。

##### Exact navigation、same-form cancellation 与 postMessage

1. frame command在 G5 top endpoint 外增加 exact `target_frame`。Location、named hyperlink/window.open 和 form 都把
   `ChildBrowsingContextNavigationRequest` 交给 target owner；GET/POST method、encoded body、headers、source
   initiator、policy-filtered network referrer与最终 `document.referrer` carrier 不会在 protocol 重建。
   Assign/Replace 现在共用 typed request scheduler；Replace 只改变 history disposition，不再退回只传 URL 的
   target-parent referrer推导。
2. source form 为每次 remote child submission 分配 `RendererRemoteFrameNavigationId`，target Page 把这个 id绑定到
   自己的 `FrameDocumentNavigationLoadBinding`。同一 form retarget A→B 时，source owner action FIFO先发
   `CancelFrameNavigation(A,id)`，target 只在 token、root Document 和 exact load generation都匹配时取消 task、
   loader/parser ledger，然后再接受 B。owner-local `DomHandle`/load binding 从不回传 source Page。
3. target 映射会保留到 navigation load真正 terminal；pending→load commit 不能过早删除 cancellation identity。
   load settled/without-dispatch 后 target 清理映射。source 侧尚无 terminal-completion notification，因此对已经
   terminal 的旧 id再次取消可能得到 harmless `false`；跨进程 completion carrier 属于 G6B。
4. remote child `postMessage` 在 target owner复核 exact frame并确保其 realm存在后，排入该 child 的 window-message
   task；target origin仍在 dispatch时检查。source carrier可标记 source top或source child token/origin，target agent
   materialize自己的 source proxy，`MessageEvent.source` 不为 `null`且不会退回 incumbent self-message。
5. nested `focus()`/close 不被伪造：G5 top-level focus/close transaction只接受无 `target_frame` 的 command，child
   close按 Window 语义保持 no-op。P6R9 复核确认当前 target Page 已拥有本地 descendant lifecycle；实际 remote
   focus traversal、activation transfer 和 descendant unload 聚合只在 OOPIF/process producer 出现后实现。

##### Protocol ACK deadline 与 owner loss

1. protocol ingress仍用 `RendererResolvedPopupTarget` 找 exact active/background target和 loaded Page generation；
   在不持有 BrowserContext/target-slot borrow时等待 renderer ACK，返回后再复核同一 residence。
2. wait新增 5 秒有限 deadline。reply channel关闭、JavaScript dialog interruption、timeout、Page generation变化或
   invalid ACK都返回 `false`，不会永久占住 protocol output ingress。timeout只是释放 protocol wait，不声称已经有
   OS process supervisor或能强杀 hung renderer work。
3. protocol integration用 `Page.crash` 拆除 exact background popup Page后，source保留的 remote top proxy立即
   `closed === true`；后续 postMessage/Location安全 no-op，background target不复活、不 promotion，也不 alias active
   Page。这证明当前 Page-owner teardown边界；它不是外部 renderer OS process crash测试。

由此新增以下强不变量：

- nested route identity必须至少包含 group-qualified top endpoint、root Document generation和 browsing-context id；
- replicated tree永远不包含 target host pointer、V8 handle或 owner-local scheduler binding；
- observer proxy identity稳定，但每次 operation currentness动态读取 logical tree；稳定 identity不等于永久 live；
- named remote frame占据原 related Page/document-order位置，权限拒绝后才能继续查找，不能把 remote miss误判为
  top-level hit或创建新 popup；
- source scheduler id只能由 exact target Page映射到 owner-local load binding；取消失败不能猜测“当前 load”；
- Assign/Replace/form必须消费同一 typed request carrier，history disposition不能改变 initiator/referrer owner；
- remote message同时冻结 source top/frame identity与 origin，并在 target owner复核 target frame/origin；
- root replacement、COOP sever、Page crash或channel failure后，旧 command只能失败/no-op，不能按数字 id别名到
  replacement frame/Page。

##### G6A 聚焦证据

```bash
TMPDIR=<repo>/.g6-retest-* cargo nextest run -p moli-renderer-v8 \
  -E 'test(remote_agent_replicates_child_tree_and_routes_exact_remote_frame_commands)' \
  --no-fail-fast
# 最终 focused run ea066025-cf66-4050-8d3a-0d66098789b5：1 passed。
# 最终锁住 fresh remote agent、两 child stable projection、top/child source-token postMessage translation、
# Location.replace typed source/referrer、named target、POST request、source id→target load binding、
# same-form cancel(A)→navigate(B) 与 root replacement stale proxy。

TMPDIR=<repo>/.g6-stress-* cargo nextest run -p moli-renderer-v8 \
  -E 'test(remote_agent_replicates_child_tree_and_routes_exact_remote_frame_commands)' \
  --stress-count 100 --no-fail-fast
# run ce059f8f-52ac-4c92-9e48-997aae1e8ca1：100/100 iterations passed。

TMPDIR=<repo>/.g6-focused-* cargo nextest run \
  -p moli-renderer-v8 -p moli-protocol \
  -E 'test(remote_agent_replicates_child_tree_and_routes_exact_remote_frame_commands) | \
      test(cross_origin_related_page_commit_moves_local_window_to_remote_agent_and_routes_commands) | \
      test(cross_origin_popup_remote_window_proxy_routes_through_exact_background_target)' \
  --no-fail-fast
# 最终 run 6b246054-f885-4b6c-ac97-0e3db5a4868c：3 passed。
# 最终 renderer 用例还锁住 Location.replace typed source/referrer 与 POST form
# method/body/header/referrer；protocol用例锁住 exact background Page crash/teardown、retained proxy
# disconnected 与无 alias。

TMPDIR=<repo>/.g6-precommit-full.* cargo nextest run --no-fail-fast
# run 04eb1af8-db76-450f-ba0a-08da382ebe94：16242 passed、18 skipped。

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# 均通过；最终 clippy 1m04s。

git pull -r origin master
# origin/master 从 1464757bd8 前进到 0ce69cbdcf；rebase 完成后 topic 落后 0、领先 66。

TMPDIR=<repo>/.g6-postrebase-full.* cargo nextest run --no-fail-fast
# run 0197d696-6925-41b5-bd64-9bc8a9469ece：16251 passed、18 skipped。

cargo fmt --all --check
TMPDIR=<repo>/.g6-postrebase-lint.* cargo clippy --workspace --all-targets -- -D warnings
# 均通过；post-rebase clippy 1m32s。
```

第一次新增 same-form回归在 run `cb652198-a28a-43c7-a5be-4ac8ccb5267d` 暴露了真实 owner bug：target map
在 pending navigation转为 load时就被清除，cancel(A)因此 ACK `false`。修复不是增加 retry/drain，而是把映射寿命移到
`settle_child_navigation_load()` / `finish_child_frame_navigation_without_load_dispatch()` 两个 terminal owner。

提交前第一次 full run `9d522044-d335-450c-92a1-10b74a8c99ef` 为 16241 passed、1 failed、18 skipped；唯一失败是
新增 G6A 回归在 `Location.replace` 后的 named hyperlink ACK 得到 `false`。单跑
`a22cafb5-a739-410b-b81c-a3ad1259936f` 通过，但原始 fixture 把连续 navigation 指向不可解析的
`example.test`：stress run `37e8df6c-70ba-452d-a74d-3433b404d38e` 证明前三次可能在 DNS 尚未快速失败时通过，随后
transport terminal 会先清掉 A 的 navigation identity，测试已经不再处于它声称验收的“cancel pending A”状态。
fixture 因此改为把同源带端口 URL 通过 `FetchConfig::set_http_host_resolve()` 映射到一个本地、明确不响应的 listener；
它保留完整 same-origin Referer，并由新 navigation/Page teardown 真实取消 pending fetch，没有 sleep、retry、轮询或
放宽 ACK。最终 100/100 stress、focused integration 与 full workspace均通过。这里修的是测试前置条件，不把当前
child transport-error Document 长尾冒充 G6A 已完成。

最终 rebase 共处理 67 个 replay step。较早的 `feat(navigation): fence committed document continuations` 与新
master 的 preload owner-admission重叠，合并结果保留 master 的 CSP/meta gate、stylesheet queue和
`admit_pending_preloads()`，同时保留 topic 的 module-map单一 owner、实际 scheduled count与 deferred
document-start fence。另一个 topic 测试提交 `d97f1d5555` 已被 master 的 `34784812b5` 等价覆盖，冲突按 master
更新后的真实 scheduler wait保留，因此该提交重放为空而没有制造重复测试。post-rebase full/fmt/clippy均通过；
Chromium 对照仍固定在 `a03603fe9af6`，没有基线漂移。

外部 Chrome/WPT、真实多进程 crash/IPC fault injection尚未运行，不能把 G6A写成 Chromium SiteInstance/OOPIF
exit。

#### Phase 5G6B1：versioned process-neutral wire、channel generation 与 queued cancellation

G6A 的 route/currentness owner 已经可复用，但 carrier 当时仍能直接持有 Rust enum、policy struct 和 V8 structured-clone
attachment。这样的接口在同一进程内可以通过，却无法证明发送端与接收端没有共享 capability，也无法在 renderer
重启后区分“同一个 logical WindowProxy”与“旧 execution channel”。G6B1 关闭的是 G6B 的 wire/identity 前半段；它
刻意不创建假的 OS process abstraction，也不把尚不存在的 process supervisor、Mojo channel 或 crash recovery 写成
已完成。

##### Chromium 对照与本轮边界

对照继续固定在 `/home/donoughliu/chromium/src@a03603fe9af6`。除 G5/G6A 已核对的
`RemoteWindowProxy`、`RemoteFrame` 和 `RenderFrameProxyHost` 路径外，本轮重点核对：

- `third_party/blink/public/common/messaging/cloneable_message.h` 与
  `third_party/blink/public/mojom/messaging/transferable_message.mojom`：跨 renderer carrier 显式拥有 encoded message、
  Blob、ArrayBuffer、MessagePort/stream channel、sender origin/agent-cluster id 与 brokered FileSystemAccess token；它不把
  target V8 handle 或 `v8::CompiledWasmModule` 塞进 Mojo；
- `third_party/blink/renderer/bindings/core/v8/serialization/serialized_script_value.h`：
  `SerializedScriptValue::WasmModules()` 保存的是 process-local `v8::CompiledWasmModule`；
- `third_party/blink/renderer/core/frame/local_dom_window.cc`：locked message 的 source agent-cluster id 与接收 Window
  不一致时，在反序列化网页值前把事件变成 `messageerror`；
- `third_party/blink/public/common/messaging/cloneable_message.h` 的 FileSystemAccess 注释同时要求接收端 origin check 和
  browser-process `FileSystemAccessManager` 二次授权。Moli 尚无对应 broker，因此不能通过复制 renderer 内部
  handle 来伪造支持。

Moli 当前使用严格 JSON schema 作为进程中立 seam；选择 JSON 是当前可测试实现，不是未来 IPC 格式承诺。
G6B1 的 exit condition 是“carrier 可以仅靠 bytes + routing header 重建并拒绝伪造输入”，而不是“renderer 已经被
拆成独立 OS process”。

##### Versioned command wire 与 exact route

1. `RendererRemoteWindowProxyCommand` 现在只保留一份已验证 routing header 和 `Arc<[u8]>` encoded command。target
   owner 每次执行前从 wire 重新 decode；sender 端原始 Rust command enum、URL/request struct、structured-clone
   payload和 target host/V8 capability 都不会随 command 一起共享。
2. v1 schema 对 unknown field 使用 `deny_unknown_fields`，并同时校验 nonzero request id、endpoint、exact
   `(owner-local-host, Page)` residence、channel generation、root Document/frame token、operation/route kind、URL、
   method/body/header、source Page/frame 与 target-origin。serialized origin 只接受规范化 origin serialization 或
   `null`，不能把任意 URL/string 冒充安全源。
3. top navigate、frame navigate/cancel、postMessage、focus 与 close 共用同一 envelope；只有 frame operation允许
   `target_frame`，top operation携带 frame route会被 target decode拒绝。POST body和二进制 structured-clone attachment
   使用无 padding base64，不依赖 sender address space。
4. 单 command总 wire上限为 64 MiB，URL、header/string与 attachment各有更窄上限；renderer output memory accounting
   直接使用实际 encoded byte length，不再用手工估算 Rust value大小。

##### Logical endpoint 与 execution-channel generation

1. logical top-level endpoint仍由 `(BrowsingContextGroupId, endpoint generation)` 标识，并在 same-group Document/
   agent replacement中保持稳定。新增 `RendererRemoteWindowProxyChannel` 由 `(owner-local-host id, channel generation)`
   标识当前接收 execution owner；每条 top/frame command和 remote form scheduler identity都冻结两层 identity。
2. channel在 target创建时分配。same-group cross-agent transition只有在 commit 已停驻 outgoing proxy、即将安装新
   LocalWindow 时才旋转；provisional prepare/cancel不旋转。这样 old proxy仍指向同一 logical browsing context，
   但旧 agent队列中的 command不能因为 Page/endpoint没变而落到 replacement realm。
3. target decode和 owner dispatch同时校验 channel owner属于 exact Page residence且 generation等于 logical target
   当前值；diagnostic公开 owner/generation供协议回归观察。COOP sever仍额外更换 group endpoint，不能用 channel
   rotation替代 browsing-context-group split。

##### Revisioned remote-frame policy wire

1. 每个 `RendererRemoteFrameSnapshot` 独立编码为 v1 strict wire，包含 monotonic tree revision、top endpoint、root
   Document lifecycle、frame/parent id、动态 name、URL/origin、`document.domain` 和完整当前
   `DocumentPolicyContainer` 投影。group state只保存 encoded bytes，不保存 target host、DOM/V8 handle或 loader
   binding。
2. publication为整棵树分配同一非零 revision；reader每次重新 decode整个 snapshot，并复核 endpoint、root
   Document/Page和 revision一致。最多 4096 个 frame、单 snapshot 4 MiB、整树 64 MiB。
3. full-tree validator拒绝重复 frame id、缺失 parent、parent cycle、跨 Page root、非 canonical origin、非法
   credentialless nonce和超限 policy/string。稳定 observer proxy仍可保留，但任何一次 lookup/operation都必须消费
   最新完整 tree，而不能信任曾经 decode成功的单节点。
4. frame name、URL、policy 和树规模都可能受网页控制。publication 若发现非法或超限值，会先推进 revision、清空
   已发布 tree 并记录 warning；不会以 `assert` / `expect` 杀死 renderer，也不会让上一个合法 revision 继续被路由。

##### Structured clone 的 process-neutral attachment

1. remote message wire显式拥有 V8 encoded bytes、transferred ArrayBuffer bytes、MessagePort ids、Readable/
   Writable/TransformStream channel ids、Blob与普通 File bytes/metadata。decode验证总 retained bytes、attachment数、
   clone/transfer/port id唯一性、非零 id、finite `File.lastModified` 和有界无 NUL string。
2. Wasm遵循 Chromium的 agent-cluster failure，而不是把 compiled module转换成 bytes后在 target重编译。source
   `postMessage()`完成序列化且不抛同步异常；cross-agent wire只携带 locked/source metadata，不携带 compiled
   module。target在读取缺失 attachment前确定 exact remote-agent mismatch并派发 `messageerror`，其 `data` 为
   `null`，同时保留正确 origin与 canonical source proxy。
3. 普通 renderer-local `FileSystemHandle` 和 OPFS-backed `File` 在 remote serialization阶段同步抛
   `DataCloneError`，且发生在 ArrayBuffer/MessagePort/stream transfer side effect提交前。这是有意保守的不支持，
   不是 Chromium等价；要关闭差距必须先有 browser-owned transfer token、storage key/origin复核和接收端 broker。

##### ACK waiter cancellation 的线性化边界

1. protocol remote command使用 cancellable pending handle。timeout、dialog interruption、reply channel失败或 caller
   放弃 wait会 drop handle，并以 CAS把仍处于 `Pending` 的 command标为 `Cancelled`。
2. target actor在取得 owner lane后、任何 Page/currentness检查和副作用前，以 CAS执行 `Pending → Running`。若 waiter
   先取消，queued command直接拒绝；若 actor先取得 `Running`，该 owner turn继续到 terminal ACK，caller timeout
   不能回滚已经开始的 navigation/message/close事务。
3. 这里的线性化点是 target owner admission，不是 network commit。actor占有 owner lane后不会与同一 Page的 agent
   replacement交错，且 exact channel仍会在 operation执行前复核。G6B1因此解决“timeout后尚在队列中的命令晚执行”，
   但尚未解决真实 process断开后 browser保存跨重启 tombstone或回收 orphaned request。

由此新增以下强不变量：

- 跨 endpoint carrier只能由 schema bytes、数值 identity和进程中立 attachment组成；target host/V8 handle、Rust
  owner pointer与 process-local compiled module不能越过 wire seam；
- logical endpoint与 execution channel是两层 identity；same-group agent commit保留前者并旋转后者，canceled
  preparation两者都不改变；
- command routing header与 decoded body必须逐字段相等，unknown version/field、kind/route矛盾或 forged origin一律
  在 target副作用前拒绝；
- remote-frame tree必须以单一 revision通过完整 topology校验；验证单节点不能代表验证整棵树；
- caller丢弃 ACK wait只能取消尚未取得 target owner lane的命令，已经 `Running` 的事务不能伪装成可回滚；
- cross-agent Wasm source调用成功、target派发 `messageerror`；不能用同步 `DataCloneError` 或 target recompilation改变
  Chromium可观察顺序；
- 没有 browser capability broker的 FileSystemHandle/OPFS File必须在 transfer side effect前失败，不能把 renderer
  内部对象序列化成可伪造 token。

##### G6B1 聚焦证据

```bash
cargo check -p moli-renderer-v8 -p moli-core -p moli-protocol --tests
# 通过；覆盖本轮三个 Rust owner边界的完整 test build。

TMPDIR=<repo>/.g6b-focused.m2YE7E cargo nextest run \
  -p moli-renderer-v8 -p moli-protocol \
  -E 'test(remote_window_proxy_wire_rejects_unknown_versions_and_fields) | \
      test(remote_window_proxy_wire_round_trips_process_neutral_clone_attachments) | \
      test(remote_frame_replication_snapshot_uses_strict_versioned_wire) | \
      test(remote_wasm_message_is_rejected_before_missing_attachment_deserialization) | \
      test(dropped_remote_window_proxy_waiter_cancels_queued_target_dispatch) | \
      test(cross_origin_related_page_commit_moves_local_window_to_remote_agent_and_routes_commands) | \
      test(remote_agent_replicates_child_tree_and_routes_exact_remote_frame_commands) | \
      test(cross_origin_popup_remote_window_proxy_routes_through_exact_background_target)' \
  --no-fail-fast
# final run a269540f-67ff-4c7c-97d9-fab9af03edb3：8/8 passed。

TMPDIR=<repo>/.g6b-focused.m2YE7E cargo nextest run \
  --no-fail-fast --status-level fail --final-status-level fail
# final pre-commit run bef88e2e-0d94-4555-b399-aa51c8d14aea：16256 passed、18 skipped。

cargo fmt --all --check
TMPDIR=<repo>/.g6b-focused.m2YE7E cargo clippy --workspace --all-targets -- -D warnings
# 均通过；clippy 1m33s。
```

2026-08-22 把完整 popup commit series rebase 到 Moli master 后，又在最终工作树复核了一次，而没有沿用上面的
pre-rebase 结果：

```bash
TMPDIR=<repo>/target/tmp cargo nextest run \
  -p moli-renderer-v8 -p moli-protocol \
  -E 'test(remote_child_navigation_wire_rejects_oversized_header_fields) | \
      test(remote_frame_replication_snapshot_uses_strict_versioned_wire) | \
      test(remote_agent_replicates_child_tree_and_routes_exact_remote_frame_commands)' \
  --no-fail-fast
# run 8a8c81a7-0caa-4524-88c7-7f8fb4e89c7f：3/3 passed。

# G5/G6B joint matrix：run 1b03c300-50a9-4c73-aba5-c6dd6602e9c3，8/8 passed。
# G6A exact child route stress：run 8a560a50-c767-4fc0-9b22-85fdb49fd725，100/100 passed。

TMPDIR=<repo>/target/tmp cargo nextest run \
  --no-fail-fast --status-level fail --final-status-level fail
# final run b2fc95ed-85bc-48ef-b062-636d56ddb65a：16094 passed、14 skipped；101.376s。

cargo fmt --all --check
TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# 均通过；clippy 47.48s。
```

这次复核额外发现 publication 对网页可控 frame name/URL/policy/树规模仍使用 `assert` / `expect`，以及 child
navigation header 只限制数量而未限制单字段大小。最终实现已改为 invalid/oversize publication 推进 revision 并
清空旧树、wire decode 拒绝超限 method/header；相应 focused case 和 full gate 都在上述最终工作树上通过。

这组用例同时锁住 strict version/unknown-field/forged-origin/duplicate-port拒绝、合法 body 与 browser-validated route
header不一致时拒绝、正常 attachment round trip、
frame-tree parent cycle、Wasm target `messageerror`、waiter drop取消 queued target side effect、same-group agent
commit channel rotation/cancel保持、G6A exact child route，以及 protocol exact background popup target。它仍是
same-process fault seam；没有 fork renderer或注入 OS channel failure。

第一次 pre-commit clippy暴露 `contains_wasm_module == !wasm_modules.is_empty()` 的
`clippy::nonminimal_bool`；实现按等价表达式化简后重新执行 fmt、完整 clippy和 workspace nextest。最终证据对应修正后
工作树，不沿用修正前的测试结果。外部 Chromium/WPT与真实多进程 fault injection未运行。

##### G6B1 明确保留给 process lifecycle/identity 的范围

- 尚无真实 renderer OS process spawn、browser-owned IPC channel、process crash observer、restart/rebind、channel
  authentication与跨 process fault injection；当前 channel generation是形状正确的 execution-owner identity，不是
  process supervisor完成证明；
- 尚无 browser-side FileSystemAccess/OPFS transfer-token broker；structured clone也未覆盖 ImageBitmap、shared
  memory等未来需要 broker或平台资源的 attachment；
- snapshot仍不是 Chromium `FrameReplicationState` 全集：unique name、permissions/insecure-request、opaque-origin
  trust bit、per-frame activation/ad/focus、pending-vs-active policy和 fenced/guest/embedder状态仍缺；
- exact agent-cluster token/reunification尚未建模。target回到 opener同源时继续保留 remote projection，语义安全但
  资源和 agent clustering不等价；
- 实际 OOPIF/process descendant 的 beforeunload/unload/focus 聚合、fenced/guest/embedder `CanNavigate`/
  side-channel 规则、BFCache/pending-deletion delivery、on-demand opener chain 与 source terminal completion
  notification 仍缺。当前单进程 target Page 已拥有本地 descendant lifecycle，也没有 remote child process producer；
- remote form method/body/referrer/scheduler 已完整。Chromium 的 `RemoteFrame::Navigate()` wire 同样不携带
  source element 或 V8 `FormData`。target-realm event 只在 source Window 可访问本地 target 时接收 source element，
  所以跨 agent DOM carrier 不再列为缺口。remote `javascript:`/isolated-world 的历史缺口已由 P6R8 收口；
- P6R2 已完成 group-safe opaque-origin nonce，P6R3 已完成真实 local Document realm 的 JS-retained
  detach/GC owner 协同；ReportingService queue/source/partition/NIK 与未来不可信跨进程 capability lifetime
  仍属于后续 group/identity 长尾。

2026-08-22 复核后不再把 **G6B2 real renderer process lifecycle** 列为 popup 当前终态的下一刀。它只在 Moli
另行采用多进程 renderer 时启动，届时需要 browser owner真正持有 process/channel generation与 request tombstone，
建立 spawn/disconnect/crash/restart/rebind，并用 protocol fault injection证明 queued/running ACK、frame revision和
retained proxy不会跨 process generation串线。当前下一大纵切是 **Phase 6 compatibility reachability/removal**。
P6R1 已画清并关闭三个 compatibility creation 文件的 production caller，P6R2 收敛 group-safe opaque origin，
P6R3 又完成 JS-retained detached Document/Node/realm lifetime；P6R4 已按 facade → loader/parser → protocol fallback
的 owner 依赖顺序物理拆除旧栈。P6R8 又在同一 typed endpoint 上补齐 remote `javascript:` 与
isolated-world source。P6R9 按 Chromium wire 和当前 producer 重新分类 remote descendant 与 form DOM carrier。
P6R10 又完成 receiver/entry/accessing identity 与通用 child scheduler。当前长尾转为 81-case 矩阵的
owner-by-owner 分类，以及 Reporting/file-local。
fenced/guest/embedder、OOPIF lifecycle 和 browser capability broker 保持独立可选项目，不能阻塞 Moli 的
单进程 popup 收口。

### Phase 6：删除 lightweight 专用模型

E3 已达到“所有已迁移 production creation producer 都创建真实 auxiliary Page”，Phase 5L2 完成 local
focus/active Page closure，Phase 5G1-G6A 又完成 local committed-response/redirect-chain COOP group status、
sandbox blocked-response transaction、group-qualified disconnected endpoint routing、same-group cross-agent
top-level command/ACK 与 agent-neutral remote child route；Phase 5G6B1 再让 command/policy/clone carrier进程中立、
版本化且可严格拒绝伪造 route，并以 channel generation隔离 outgoing/replacement agent。真实 renderer process
lifecycle与 browser capability broker 不是删除旧栈的产品前置。P6R1 已关闭 production compatibility creation，
P6R2 已完成 group-safe opaque-origin identity，P6R3 已完成真实 local Document realm 的 JS-retained lifetime。
P6R3 结束时仍不适合整块盲删，因为测试与 standalone adapter 把旧类型当作行为夹具；当时按
`git grep -l -E 'lightweight_popup|LightweightPopup|lightweight popup' -- '*.rs'` 与对应 occurrence 统计，
宽口径扫描仍为 112 个 tracked Rust 文件、1492 处命中。P6R4 先恢复通用夹具、再沿 owner dependency 删除旧类型，
最终以 `rg -ni 'lightweight[ _-]?popup|LightweightPopup' --glob '*.rs'` 复核为零命中。这个前后差异也说明原判断
“不能按命中数机械删除”是正确的，但 reachability 前置满足后应直接物理删除，不能继续用 `cfg(test)` 保留整套旧 owner。

#### P6R1：production creation exit 与完整 initial Page handoff

对三个直接 caller 反向复核后的结论比旧文档更强：有 renderer owner 的 `window.open()`、hyperlink 和 form
new-context 路径此前虽调用名为 `open_lightweight_popup_window()` 的 facade，实际已经优先 reserve exact
`RendererPendingAuxiliaryPage`，并在 opener owner turn 中构造完整 initial Page/Document/realm。legacy record 只在
allocator 缺失或 staging 失败时回退创建；后者会重新引入第二套 loader/parser owner，且会让 unit test 在真实 Page
staging 回归时假通过。

P6R1 将这条边界改成结构性不变量：

- Chromium 对照仍固定在 `a03603fe9af6`：Blink `CreateNewWindow()` 从 embedder 得到 `Page` 后立即要求
  `page->MainFrame()` 并返回该 frame；`LocalDOMWindow::open()` 在 `FindOrCreateFrameForNavigation()` 选定/创建 exact
  frame 后才启动 navigation并返回它的 `DomWindow()`。Moli 因此不保留脱离 Page 的 production synthetic shell；
- production DOM caller 只调用 `open_renderer_owned_related_auxiliary_page()`；它必须同步创建完整 staged initial
  Page，并返回同一个 stable WindowProxy 与 exact Page reservation；
- owner-backed staging 失败统一 fail closed，随后只能发布不带 local proxy/reservation 的 browser-owned action，
  不能创建 lightweight record 兜底；
- `create_lightweight_popup_window()`、standalone Window shell/Navigator/storage aliases/close method、legacy named
  reopen 与两个 caller 中的 lookup branch 全部只在 `cfg(test)` 编译；test fallback 又显式拒绝任何已绑定 Page
  allocator 的 host，避免掩盖 production staging regression；
- 删除 `RendererStagedAuxiliaryWindowProxyRegistry` 及 protocol-created Page 的延迟消费分支。同步 handoff 只保留
  `staged_related_initial_empty_pages`，其中一个 entry 已经是完整 `PageVm`，protocol 以 exact reservation 消费一次；
- 仍保留 `LightweightPopupBrowsingContextRecord` 及其 mirrored loader/parser 供 standalone compatibility fixtures。
  因此本纵切证明“production 不再创建旧 owner”，不等于 Phase 6 record 删除完成。

#### P6R2：group-safe opaque-origin identity 与 LocalWindow lifetime

P6R1 后重新审计 `WindowAccessOrigin`、StorageKey、child owner transition、related group replication 与 legacy
compatibility record，确认原来的安全降级仍有两项结构性问题：跨 host opaque origin 一律拒绝会误伤合法 inherited
auxiliary `about:blank`；child storage nonce 只按 iframe `DomHandle` 缓存，又会在 replacement navigation 后误复用旧
opaque identity。直接把缓存改成 `FrameDocumentTaskOwner` 也不正确，因为 `document.open()` 会换 DocumentId、但按
Chromium 语义必须保留 LocalWindow 与 origin。

固定 Chromium `a03603fe9af6` 对照如下：

- `third_party/blink/renderer/platform/weborigin/security_origin.cc:173-210` 的 copy constructors 复制
  `nonce_if_opaque_`；`249-317` 区分 unique opaque、带既有 nonce reconstruction 与 sandbox 指定 nonce；
- `security_origin.cc:566-611` 的 `IsSameOriginWith()` / `IsSameOriginDomainWith()` 在任一 origin opaque 时只比较
  `nonce_if_opaque_`；两个公开序列化都为 `null` 的 origin 不因此相同；
- `security_origin.cc:708-712` 的 `DeriveNewOpaqueOrigin()` 必须生成新 nonce；
- `sandboxed_opaque_security_origin_creator.h:17-29` 只允许 DocumentLoader 以指定 nonce 构造 sandboxed origin，说明
  nonce 是 loader/owner carrier，不是 URL 字符串或 renderer-local Window 编号。

Moli 的单进程 owner 由此形成以下不变量：

1. `OpaqueOriginNonce` 由共享 `RendererBrowserContextRuntime::next_opaque_origin_nonce()` 非零单调分配，不再误命名为
   WebStorage-only allocator。top-level host 在构造时即绑定 identity；
   inherited auxiliary Page 从 creator StorageKey 复用 exact nonce，独立 opaque Page 分配新 nonce。
2. child own-opaque binding 保存 `(DomHandle, LocalWindowId, nonce)`。initial install 与 navigation commit 在脚本可观察
   前刷新；LocalWindow replacement 换 nonce，`document.open()` 的 Document-only replacement 保留 nonce，detach 删除
   binding。network partition 与 Window access 消费同一 identity，不再各自维护生命周期。
3. Rust `WindowAccessOrigin` 从 host-local `WindowExecutionContextOwner` 改为 `OpaqueOriginNonce`。related host 可以比较
   inherited exact nonce，独立 `null` origin 仍拒绝；V8 security token 继续是优化/VM boundary，不再是 Rust owner
   缺失时的唯一正确性来源。
4. related top-level state复制 current opaque nonce；remote frame snapshot/wire 增加 nonce并升为 v2。ingress严格拒绝
   missing/zero/mismatched nonce、tuple-with-nonce 和 opaque `document.domain`；remote target/ancestor `CanNavigate`
   comparison消费复制 identity。
5. legacy lightweight compatibility record仍编译的 opaque路径改为从它已有的 StorageKey 取 nonce，避免测试双栈继续
   制造另一套 owner identity；这不重新开放 production creation caller。

真实 related auxiliary 回归锁住 `data:` opener → staged initial `about:blank` → protocol adoption：adoption 前后 opener
都能访问同一个 popup Document；另一个独立 related `data:` Page 即使公开 origin 同为 `null` 仍抛 `SecurityError`。
child 回归同时锁住 `document.open()` 保留 nonce与普通 opaque navigation 换 nonce；strict-wire 回归覆盖合法 round-trip
和四类伪造输入。`MessageEvent.origin` 继续只暴露发送时的公开 `null`，没有把 nonce泄露给脚本，也没有错误地按 source
当前 Document重验 queued message。

P6R2 最终工作树的验证证据如下；focused matrix 同时覆盖 inherited child、真实 related auxiliary、独立 opaque
拒绝、LocalWindow lifetime 与 strict remote wire，完整 workspace gate 没有沿用修改前结果：

```bash
TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 \
  -E 'test(child_opaque_origin_nonce_follows_local_window_lifetime) | \
      test(related_auxiliary_page_inherits_exact_opaque_origin_nonce) | \
      test(remote_frame_replication_snapshot_uses_strict_versioned_wire) | \
      test(inherited_opaque_srcdoc_reuses_initial_empty_child_local_window) | \
      test(opaque_origin_access_requires_the_same_non_serialized_identity) | \
      test(related_pages_use_the_shared_browser_context_opaque_nonce)'
# final run 243757ed-b89f-41c7-a3a2-e6c523a91991：6/6 passed。

TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast
# final run 6c820911-5bd8-43bd-9cc8-c7295914c2c9：16096 passed、14 skipped；100.802s。

cargo fmt --all --check
TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# 均通过；clippy 1m31s。
```

这份 nonce 是当前可信单进程 browser-context 内的 identity，不是未来不可信 renderer IPC 的 authentication token。
若 Moli 另行采用多进程，需要像 Chromium `UnguessableToken` 一样由 browser owner签发并校验，而不能把单调 u64
当安全 capability。P6R3 已完成后续真实 local Document realm 的 JS-retained lifetime；未来若引入不可信 renderer
process，opaque identity 的签发和 retained remote capability 回收仍须由 browser owner 重新建立。

#### P6R3：JS-retained detached Document realm 与无 Oilpan GC owner

P6R2 后的 revisit 发现，旧的“Page teardown 就断开 Context host pointer”虽然避免 use-after-free，却比 Chromium
过早销毁语义：作者从 iframe/popup 旧 realm 保存的 function、Document 或 Node 会立刻 `TypeError`，而不是在最后
JS 引用消失前继续服务原 detached DOM。简单让 Context slot 强持 `JsContextHost` 也不成立，因为 Moli 没有 Oilpan；
任何 `Context → Rust slot/host → v8::Global → Context` 都会成为 V8 无法追踪的自保活环。

固定 Chromium `a03603fe9af6` 的实现对照与本地 Chromium 行为探针给出以下边界：

- `LocalWindowProxy`/`ScriptState` 把 outer stable WindowProxy 与 inner global/Document lifetime 分开；navigation 后保存的
  closure 仍执行在旧 inner realm，保存的 Document/Node 仍可读取和修改，但 outer `window` 继续指向 browsing context
  的 current WindowProxy；
- `Window.document` 是 `[LegacyUnforgeable, CachedAccessor=kWindowDocument]`。Blink 用
  `FunctionTemplate::NewWithCache` 和 realm-private cache 保存创建 realm 的 Document，不是每次从 stable outer proxy
  动态解析 current Document；
- 从 frame detach 后，保存的旧 Document/Node/function 仍可用，`Document.defaultView` 为 `null`，保存的旧 Window
  已 `closed`。detach 前注册的 Window event listener 不再被调度，旧 custom-element registry 也不会 upgrade 新建元素；
  这说明要保留 DOM/realm 值，同时退休 LocalDOMWindow/ExecutionContext 服务；
- retained old child `AbortSignal` 在 navigation 后仍同步更新 `aborted`/`reason`，也仍能作为 live parent target 的
  cancellation source；但旧 signal 的 direct listener/`onabort` 不再运行，`onabort` 读回 `null`。反方向上，旧 child
  创建并挂到 live parent `Document` 或 parent `AbortSignal` 的 callback 也不再运行，因此 callback owner 必须取 function
  creation realm，不能只看 event target/signal realm；
- 最后 JS 引用删除并 GC 后，old inner global、Document 与 native backing 可以一起回收。保留 active timer、observer、
  IndexedDB resolver 或 wrapper-cache strong Global 都会违反这条边界。

Moli 由此建立一组不依赖 tracing GC 的显式 owner 规则：

1. `ContextHostPointerSlot` 在可失败 bootstrap 阶段仍是 non-owning pointer；任何真实 Document realm 在对脚本或
   Inspector publication **之前**必须 promotion 为 Context-owned `Rc<JsContextHost>`。default world、child default
   world、isolated world 和 prebootstrap residence 都走同一强制边界，promotion 失败则整个 bootstrap fail closed。
2. host lifecycle 明确分为 `Active → Detached → Destroyed`。`ScriptVm` teardown 把原地址稳定的
   `Box<DocumentRuntime>` 转移给 retained host；因此旧 wrapper 的 raw runtime pointer 在最后 Context GC 前仍有效，
   临时 WindowProxy facade 则保持 non-owning 并在 detach 后 fail closed。
3. teardown 只退休 execution authority，不销毁 native DOM graph。timer、event callback、parser/document.write、
   module continuation、fetch/XHR/EventSource、worker、message/port、BroadcastChannel/WebSocket、observer、
   custom-element reaction、History/Navigation、media/rendering、service worker、IndexedDB 等 owner 都在进入 isolate
   时取消或清空；isolate 级
   IndexedDB checkpoint queue 按 exact retiring Context 删除，不能误清同 agent 的其他 related Page。
4. active wrapper/intrinsic/DOMException caches 使用 strong handle 保证 identity/expando；detach 时转换为 weak handle。
   Context-local IndexedDB table、AbortStore、pending network-body resolver/error reason 与 Resource Timing secondary buffer
   按 exact realm 拆环。跨 realm event/Abort callback 按 callback creation realm 退休；Page scheduler 已接纳的 exact
   Document task payload 仍由其 typed task owner 在 selected/discard turn 消费，不能与无 owner 的 host cache 混为一谈。
   slot 中仍含 Weak handle 的 Rust
   state 延迟到 Context annex finalization 后，再在普通 entered-isolate 栈上释放，避免从 V8 GC callback 直接 reset
   persistent handle；isolate shutdown 则在 `OwnedIsolate` 尚存活时完成最终 drain。
5. vendor V8 shim 暴露 `FunctionTemplate::new_with_cache()`，`window.document` 按 Blink cached-accessor 语义安装为
   enumerable、non-configurable、无 setter 的 own accessor。stable WindowProxy navigation 后投影 replacement
   Document，而旧 closure 里的 accessor 仍返回旧 Document，二者不会被一个动态 getter混为一体。cached value 只在
   stable proxy 仍由 exact current LocalWindow realm 注册时原位更新；真正的 LocalWindow replacement 先 detach 旧
   Context，再由 replacement Context 更新同一 outer proxy，不能在 commit 中间把新 Document 写进旧 inner global。

回归把保活和回收写成可精确计数的双向不变量：related top-level navigation/close 后，保存的旧 Document、Node、
function 能继续读取和修改；保存 child realm 值时 old top+child host 继续存活；删除最后引用并触发 GC 后 native
Document host、detached Context 和 wrapper strong-entry 数都回到操作前基线。另一个 case 在旧 top+child realm 中先
创建 custom elements、AbortController、MutationObserver、长 timer、Window listener 与 pending
`indexedDB.databases()`，teardown 后不保留任何额外 realm。child case 还同时保留 pending `Response.text()` resolver、
errored body reason 和 Resource Timing overflow entry，验证三类共享 host strong edge 在 exact realm teardown 后从
`(1, 1, 1)` 归零；live parent target 上由 old child 创建的 Abort callback 也必须被清掉。聚焦证据：

```bash
TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_script_agent_transfers_stable_window_proxy_objects_and_dom_wrappers) | \
      test(related_page_script_agent_releases_replaced_and_closed_peer_realms) | \
      test(related_page_script_agent_releases_detached_host_v8_roots_and_child_realm) | \
      test(related_page_script_agent_retains_and_releases_detached_child_dom_values) | \
      test(assigning_document_does_not_replace_legacy_unforgeable_document_alias) | \
      test(child_navigation_aborts_fetch_and_detaches_keepalive) | \
      test(child_navigation_retains_and_releases_detached_dom_realm) | \
      test(dom_wrapper_expando_survives_renderer_document_isolate_garbage_collection) | \
      test(context_wrapper_cache_is_weakened_on_script_vm_teardown) | \
      test(child_default_bridge_ref_is_released_on_child_context_teardown) | \
      test(detached_first_exposure_retires_prebootstrapped_child_realm) | \
      test(child_navigation_retires_websocket_execution_context) | \
      test(scalar_buffer_state_keeps_pending_and_capacity_orthogonal)' \
  --no-fail-fast --status-level fail --final-status-level fail
# run d0f5f632-a0b4-4318-af2d-3cac6ced8fdc：13/13 passed。

TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 -p moli-v8-util \
  -E 'test(/abort_signal/) | \
      test(indexed_db_detached_realm_methods_keep_receiver_realm_state) | \
      test(indexed_db_transaction_stays_active_through_creation_task_microtasks) | \
      test(related_page_script_agent_keeps_indexed_db_manager_routes_page_local) | \
      test(reused_global_proxy_keeps_intrinsic_maps_isolated_by_context)' \
  --no-fail-fast --status-level fail --final-status-level fail
# run 5ba7de6f-5cc5-4982-9df1-61764680eba8：20/20 passed。

TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 \
  -E 'test(related_page_script_agent_releases_replaced_and_closed_peer_realms) | \
      test(related_page_script_agent_releases_detached_host_v8_roots_and_child_realm) | \
      test(related_page_script_agent_retains_and_releases_detached_child_dom_values) | \
      test(child_navigation_retains_and_releases_detached_dom_realm)' \
  --stress-count 10 --flaky-result fail --test-threads 4 --no-fail-fast
# run 11e148f1-d64c-4a96-8bd6-c2c7d8e1dbae：10/10 iterations passed，40/40 case executions。
```

最终 workspace revisit 没有只沿用 focused 证据。第一次 full nextest 暴露并修正了三条共同边界：

- related Page 会在已 entered、已借用的 source isolate 回调内同步 bootstrap。deferred host-release queue 因此直接挂在
  可重入的 `RendererDocumentIsolateHandle` 上，不能为读取队列再次借用 isolate holder；popup/target 创建簇不再触发
  `RefCell already mutably borrowed`；
- initial-empty LocalWindow reuse 要原位刷新 `Window.document` cache，LocalWindow replacement 则必须保留旧 realm
  cache。刷新现在由 exact current realm registration 判定，同时覆盖 retained-old-function 与 initial-empty rebind 两组
  相反回归；
- Page teardown 在清掉 output journal 前先退休 DedicatedWorker 并发布 `Target.destroyed`。retained host 延长 native
  lifetime，不能改变 browser-observable worker lifecycle 的输出顺序。

修正后三者的 9-case combined retained/rebind/form matrix run
`134ba336-2424-41c6-b069-695e6eed927e` 为 9/9 passed；最终
`cargo nextest run --no-fail-fast --status-level fail --final-status-level fail` run
`da20df0e-a03f-4155-8d86-7f37c116d615` 为 16100/16100 passed、14 skipped；
`cargo fmt --all --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 通过。

P6R3 没有声称解决 V8/Rust 全仓所有内存问题，也没有用单元回归代替真实站点 RSS 长跑。AbortSignal 与 pending
network-body source 中已经不含 detached-realm strong edge 的纯 Rust/scalar record，仍可能存活到后续显式取消或 Page
host teardown；这是资源表 compaction/RSS 观察项，不再是 Context 自保活环。P6R3 完成的是 Phase 6
所需的 local Document lifetime owner：有 JS 引用时语义可用，无引用时不形成跨堆强环，Page execution resource
不会在 detached realm 中复活。多进程 remote capability 的认证/崩溃回收继续是可选基础设施。remote JavaScript URL
已由 P6R8 收口，descendant lifecycle 又由 P6R9 按当前 Page owner 与 OOPIF reachability 重新分类。
Reporting/file-local 等语义长尾继续按产品支持面分级。

#### P6R4：物理删除 compatibility owner 与 protocol fallback

P6R3 后对“把旧实现留在 `cfg(test)` 还是直接删掉”的 revisit 结论是分层处理。完整 browsing-context owner、realm alias、
parser/loader 和 lifecycle 不能因为 production caller 已断开就继续留在 test build；那会让 standalone `ScriptVm` 测试继续
验证一套产品永远不会执行的语义，也会让后续 owner 改动同时维护两套 currentness。只有不创建 owner、不调度任务的纯数据
构造器或 test-only 查询 accessor 可以保留 `cfg(test)`。因此 P6R4 没有把旧栈整体改成 conditional compilation，而是按
依赖层物理删除。

删除后的 owner 边界如下：

1. `context_host/popups.rs` 只保留真实 related auxiliary Page 的 reservation、initial empty Document/realm staging、
   sessionStorage snapshot、opaque storage scope 和 pending activation 输出。成功的 `OpenedAuxiliaryBrowsingContext` 现在必然
   携带 exact `RendererPendingAuxiliaryPage`、storage snapshot 与 initial storage key，不再暴露永远为 `true`/`Some` 的
   compatibility-shaped 字段。
2. 删除 `LightweightPopupBrowsingContextRecord` 及其 Document/LocalWindow/navigation token、active-scope alias、shared-context
   realm registration、`with(window)` global 扫描和 synthetic Window/Navigator/storage facade。timer、XHR、fetch、worker、
   message port、BroadcastChannel、WebSocket、IndexedDB、CSP、document.domain、focus/history/storage event 等 owner 分支只剩
   Page root 或 child-frame typed identity。
3. 删除 popup 专用 document/classic-script fetch target、resource completion terminal、DOM-manipulation/load-event queue、
   mirrored parser/script executor 和对应 `PageVm` arbitration 文件。一个 auxiliary URL 从 initial Page adoption 起只可能由
   target Page 的正常 loader/parser/lifecycle 提交。
4. protocol 删除 popup-only `RendererWindowDocumentSource`、remote wire source 和 JavaScript-dialog parking route。真实 auxiliary
   Page 的 top-level source就是该 Page 的 `RootFrame`；popup target 创建、attach、dialog 与 navigation 不再从 opener attachment
   猜测第二个 Document owner。
5. Classic WebDriver 的 popup Window reference 行为没有随旧 owner 一起删除。host bridge 改为从真实 auxiliary WindowProxy 的
   private reservation id 读取，并由 protocol target 的 `moli_popup_id` 映射到稳定 window handle；旧的
   `__moliHostLightweightPopupIdForObject` 名称和 host-record lookup 一并消失。
6. 旧 standalone self-loop 测试随其 owner 删除；真实 Page/PageVM/CDP/Classic WebDriver 测试继续作为权威覆盖。一次过宽的测试块
   删除曾带走通用 Service Worker HTTP server fixture，P6R4 在编译复核时只恢复这些纯夹具，没有恢复任何 legacy popup 行为。

第一次 workspace full nextest 还暴露了 17 条旧 fixture 债。其中 15 条会让无 owner 的 standalone `ScriptVm`/单 Page task
executor 自己创建、导航并驱动 destination popup Page，删除 fallback 后分别表现为 `window.open()` 返回 `null` 或永远等不到
目标 Page 网络/消息；给 test allocator 增加 staging owner 只会在 `cfg(test)` 重建被删除的双栈，因此这些用例连同三个独占
server/cache helper 一并删除。剩余两条仍有独立价值：WebIDL 参数转换用例改为断言无 Page owner 时有效 `open()` 返回 `null`，
direct-V8-call inventory 则删除已经消失的 popup callback entry。没有用 ignore、timeout 放宽或 test-only popup runtime 把全量门禁
改绿。

本轮 Rust 与同步文档改动的提交前 diff 为 149 个文件、1297 行新增、17896 行删除，净删除 16599 行。静态证据为：

```bash
rg -ni 'lightweight[ _-]?popup|LightweightPopup' --glob '*.rs' --glob '!target/**' .
# zero matches

TMPDIR=<repo>/target/tmp cargo check --workspace --all-targets --message-format short
# passed without warnings；删除旧 fixture 后的 package all-targets check 同样零 warning。
```

真实 owner 聚焦矩阵同时覆盖 opaque-origin inheritance、DOM 三入口 policy、initial Page adoption、stable opener WindowProxy、
204/205 no-commit、ordinary→`javascript:` ordering、Service Worker producer 和 Classic WebDriver Window reference；另外把 WebIDL
无 owner 返回值与 direct V8 call inventory 放进同一次聚焦复核：

```bash
TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 -p moli-protocol -p moli \
  -E 'test(related_auxiliary_page_inherits_exact_opaque_origin_nonce) | \
      test(javascript_popup_producers_queue_the_final_related_target_page_realm) | \
      test(popup_policy_checks_keep_existing_and_new_target_order_distinct) | \
      test(opener_window_handle_projects_the_renderer_owned_auxiliary_realm) | \
      test(popup_initial_about_blank_adopts_renderer_page_and_related_script_agent) | \
      test(popup_no_commit_responses_preserve_initial_document_before_redirect_replacement) | \
      test(ordinary_popup_navigation_then_javascript_url_preserves_renderer_protocol_order) | \
      test(service_worker_auxiliary_producers_use_fresh_pages_and_navigation_terminals) | \
      test(webdriver_classic_execute_script_round_trips_window_and_frame_references) | \
      test(window_dialog_and_open_arguments_use_webidl_conversion) | \
      test(direct_v8_call_inventory_is_frozen)' \
  --no-fail-fast --status-level fail --final-status-level fail
# run e166e582-e1bd-4c6c-aed9-408cc4d1d8af：11/11 passed，其中 9 条是真实多 Page popup 路径。

TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail
# clippy 等价化简后的最终 run 7f1f0832-47a2-4565-b264-d8bb583c2678：
# 15963/15963 passed，14 skipped；执行阶段 100.407s。

cargo fmt --all --check
# passed

TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# passed；1m 31s。
```

静态扫描、focused matrix 与 workspace 三门禁已经满足本地 Phase 6 compatibility owner 删除 exit。提交后的
`git pull -r origin master` 与必要复验仍按仓库流程执行；focused WPT/CDP slice 重新分类继续是外部兼容性证据债，不能被这次
Rust 删除和 nextest 机械替代。

#### P6R5：production direct Browser auxiliary Page owner

P6R4 删除旧 owner 后，protocol/CDP 路径仍有 `CdpConnection` 的 target scheduler 消费
`RendererOwnerAction`，但 CLI/WebFetch 直接构造的 `Browser` 没有对应 output ingress。这里不是 test fixture 缺口：`moli fetch`
是 release binary 的生产入口。renderer 会同步返回真实 related WindowProxy 并 stage 真实 Page，随后发布 popup activation；如果没有
browser owner 采纳 reservation、持有 `Page` 并驱动 target navigation，opener 看到的 proxy 虽然存在，destination、跨 Page
`Location`、`postMessage` 和 close transaction 都不会继续。P6R4 前旧 lightweight self-loop 曾遮住这条缺口，删除后首个真实 CLI WPT
把它暴露为稳定的 120 秒 timeout。

因此“整套放进 `cfg(test)` 还是删掉”的边界已经固定：

- 完整 browsing-context/Document/parser/loader/lifecycle owner 不能放进 `cfg(test)`；direct `Browser` owner 是 release production
  路径，必须与 protocol scheduler 一样消费 typed renderer output；
- 旧 lightweight record、realm alias、mirrored loader/parser 和 self-loop 已在 P6R4 物理删除，P6R5 没有恢复任何旧类型或兼容分支；
- 只有不创建 owner、不推进 Page task 的纯测试工厂、查询 accessor、同一 production output stream 的只读 fan-out observer 和断言
  helper 可以保留 `cfg(test)`。测试不能替换 BrowserContext 唯一 transport。

P6R5 的 owner 链如下：

```text
renderer Page output stream
  -> exact (RendererOwnerLocalHostId, PageId) residence router
     -> owner-local auxiliary actor: owns the non-Send core Page on a LocalSet
     -> externally held root Page: exact browser-owner Page command, no handle/lifetime transfer
```

具体收口内容：

1. `Browser::new()` 安装一个 shared-by-clones 的 output transport consumer；一个 dedicated current-thread Tokio `LocalSet` 持有所有
   direct-Browser auxiliary actor。每个 actor 采纳 renderer 已经 stage 的 reservation，创建或复用同一 initial `about:blank` Page，
   并串行处理 exact navigation、RemoteWindowProxy、focus 和 close/unload 命令。`BrowserLifetimeOwner` 在 renderer/network/storage
   owner 前先停掉这条 lane，避免 auxiliary task 越过 browser teardown。
2. destination 不再序列化成 synthetic `location = ...`。新的 crate-local Page command 携带冻结后的
   `RendererTopLevelNavigationRequest`，完整保留 method、body、headers、request kind 与 source/referrer carrier，再由 target Page
   自己的 standalone follow state machine完成唯一 loader/parser/Document replacement。Service Worker `clients.openWindow()` 的
   continuation也只在该 exact Page commit 后解析。
3. named reuse、`focus()` 或 related `postMessage` 命中调用方仍持有的 root Page 时，router 通过
   `(owner-local-host, PageId)` 进入 renderer owner，而不要求克隆/转移 `RendererPageHandle`。回归确认 named reuse 导航原 root 且
   owner registry 仍只有一个 Page；不会因为 local actor map miss 静默创建或丢掉 target。
4. `window.open()` 新建 target 时，请求 URL 为空或是无 query/fragment 的 `about:blank` 都直接暴露同步 stage 的 initial empty
   Document，不再 admission 后排第二次 `about:blank` navigation。`about:blank#fragment` 仍是 target Page 必须推进的 destination work；
   已有 named target 被 `window.open("about:blank", name)` 命中时也仍是一次真实 replacement navigation。exact blank、带
   fragment 及 new/reuse 三种语义不能共用一个 URL shortcut。
5. related opener 调用 target `Location` 时，执行 turn 属于 opener、navigation owner 属于 target。location callback 现在比较
   incumbent host 与 target host；跨 Page 时显式发布 target-residence wake，避免请求只留在 target pending slot，等待一次无关 target
   command 才偶然推进。
6. stage related Page 时同时安装 exact `RendererDocumentIsolateAllocator`。initial realm adoption 后发生 Document replacement 可以继续
   在同一 Page slot 创建 target-owned realm；缺少 allocator 不再把已 committed 的旧 Document 留成不可恢复空 shell。若 committed
   bootstrap 仍失败，owner restore 只对有 live `ScriptVm` 的 Page 建 output fence，让空 retiring shell 能确定性 teardown，而不会读取
   已消失的 output journal。
7. Related activation 继续使用 creator 冻结的 session-storage snapshot；Fresh/noopener activation 没有该 snapshot 时创建独立
   session-storage namespace，不再误用 partition root store。direct `Browser` 回归先在 root 写 marker，再通过 exact Page command 读取 Fresh
   target，确认结果为 `null`。
8. initial Page materialization 不再丢弃 `top_level_browsing_context_closing` creation diagnostic。`open(url); popup.close()` 在同一同步 turn
   完成时，owner 会先采纳可观察的 staged target、拒绝发布 destination navigation，再按该 Page 自己的 close FIFO 完成 teardown；回归用
   nonblocking destination listener 证明不是“已经发请求再取消”。若来源是 Service Worker，相应 `clients.openWindow()` continuation 明确解析为
   null。

本轮新增四个 direct `Browser` 回归。第一条从调用方 root 打开 initial `about:blank`，用相对 URL 完成首次 replacement，再跨源导航并
通过 exact RemoteWindowProxy 把 `postMessage` 送回 externally held opener，最后验证 `window.close()` 只 retire auxiliary Page。第二条
把 root 命名后执行 `window.open(destination, rootName)`，验证 exact named reuse 导航同一个 PageId、registry 保持一个 Page。renderer
邻接回归另验证 related WindowProxy 的 `Location` assignment 会唤醒并导航 exact standalone target。第三、四条分别锁住 Fresh
session-storage namespace 和同步 close 的零 destination-fetch 副作用。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run -p moli-core -p moli-renderer-v8 -p moli-protocol \
  -E '<P6R5 direct Browser 四条 + cross-Page wake + about:blank new/reuse + protocol 204 no-commit + named form>' \
  --no-fail-fast
# final focused run 8b71775e-2d50-44a9-b118-e630692548e2：8/8 passed。

TMPDIR=<repo>/target/tmp cargo nextest run -p moli-protocol \
  -E 'test(named_form_post_reuses_renderer_group_target_and_preserves_exact_request)' \
  --stress-count 20 --flaky-result fail --no-fail-fast
# run 78c91785-8ed2-4b8b-9466-e46ccf63d0ab：20/20 iterations passed。
```

真实 WPT 证据使用 debug binary SHA-256
`67217017b0b63df1aa93f60ec05643a5a4cd16fcc554f9313af63f8ba476013b`、Chromium/WPT
`a03603fe9af6230a12f1b2fb2c18a7d003a0d937`、固定 6 个
`initial-empty-document/window-open-*` case。单例 A/B 中，`window-open-aboutblank.html` 从 P6R4 后 CLI 的
120087 ms timeout 变成 2541 ms、2/2 subtest pass。完整六例结果为：

| case | CLI | CDP | 当前判断 |
| --- | --- | --- | --- |
| `window-open-aboutblank.html` | pass | pass | P6R5 production owner/wake 已闭环 |
| `window-open-history-length.html` | pass | pass | 新 target initial history 投影正确 |
| `window-open-nourl.html` | timeout | harness-stalled | 两入口共有的后续 navigation/message 或 history replacement 缺口 |
| `window-open-204.html` | timeout | harness-stalled | 204 no-commit 后的后续 navigation/message 缺口 |
| `window-open-204-fragment.html` | 2 fail | 2 fail | second relative Location 报 invalid URL；不是 CLI owner 特例 |
| `window-open-204-pushState-replaceState.html` | 2 fail | 2 fail | initial-empty History mutation 后 second Location 报 invalid URL |

CLI 汇总为 2 pass / 2 fail / 2 timeout，CDP 为 2 pass / 2 fail / 2 harness-stalled；两个 mode 的 failure names 与
invalid-URL 形状一致。这把旧的笼统“CLI pumping 缺口”重新分类成两层：P6R5 已解决 direct `Browser` 无 owner 的基础阻断，剩余四例是
protocol/direct-Browser 共享的 initial-empty/no-commit URL、history replacement 或 second-navigation currentness 语义债。后续应以这四例
为一个纵切修 renderer owner，不应恢复 lightweight fallback、放宽 timeout 或建立 test-only popup runtime。最终原始输出分别在
`/tmp/moli-popup-wpt-cli-owner-20260823-closing-storage` 和
`/tmp/moli-popup-wpt-cdp-owner-20260823-closing-storage`。

P6R5 提交时保留三个明确工程边界。当前一个 `Browser` 使用一条 dedicated owner thread，而不是每 popup 一条；后续若 direct root Page 本身
迁入统一 browser scheduler，可以合并线程，但不能丢掉 exact residence/actor ownership。其次，这条 owner只消费 direct API 需要推进的
Page coordination action；download/file chooser/dialog 等没有 direct-API policy surface 的动作仍沿用既有行为，需要在相应产品能力纵切中
定义默认策略，不能由 popup owner 猜测 protocol UI 行为。第三，cross-document `history.back()`/`forward()` 当时仍需要 protocol 已有的
session-history controller，direct owner 会忽略 `TopLevelHistoryTraversal` action。P6R7 已通过 renderer-frozen exact seed 与
root/auxiliary 共用的 Page command 收口这项边界，没有建立第二份 test-only history controller。

提交前完整门禁：

```bash
TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast
# run cd2a2771-8fa2-4a03-9d27-36e8691001ea：15968 passed、14 skipped。

cargo fmt --all --check
# passed

TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# passed
```

#### P6R6 initial-empty、target queue 与 history commit 收口

P6R5 上面的六例表保留为历史基线。那次结果已经把 direct `Browser` 缺少 production owner 的问题与 renderer
共同语义债分开，但后四例仍分别表现为 timeout、harness-stalled 和 invalid URL。P6R6 沿真实 Page owner 链回查后，
找到四个会互相放大的缺口。

##### Initial empty fallback base

`pushState()`、`replaceState()` 和 fragment 更新会改变
`Document.URL`，此前同一个 setter 也清掉了 creator 继承的 fallback base。后续相对 Location 因而以变化后的
`about:blank` 解析并报 invalid URL。`NativeDocument` 现在提供保留 fallback base 的窄更新入口，renderer 只在
current initial-empty same-document 更新时使用它。普通 Document URL replacement 仍会清掉旧 fallback，避免把
creator base 带进后续真实 Document。

这里还要区分 top-level 与 child 的 History URL 解析。top-level initial empty Document 已经通过
`pushState()` 或 `replaceState()` 改变可见 URL 后，下一次 History API 的相对可选 URL 以当前可见
`Document.URL` 解析，同时继续保留 creator fallback 供 Location 等 URL API 使用。child initial empty
Document 仍参与 joint session history，并继续使用继承的 parent base。第一次把 top-level 规则直接应用到
child 后，有 8 条既有 child history/hashchange 回归稳定失败。收窄分支后，这 8 条与新增 top-level 回归
一起恢复为 9/9 passed。

##### Target Page 唯一 destination queue

此前 activation 保留 `open(url)` 的请求，target Page 自己只持有
后来发生的 Location 请求。两者由不同 owner 排队，同一 author turn 内的 `open(old); popup.location = new` 可能先
提交 `new`，protocol 随后又 replay `old`。现在 renderer 在同步 stage target Page 时就把完整 destination request
放入该 Page 唯一 pending slot。成功排队后，popup activation 只负责 creation 和 requested-URL observation，不再携带
第二份 destination。later Location、form 或 JavaScript URL 按同一个 target-local replace/FIFO 规则竞争。

```text
creator turn
  -> reserve and stage the real related Page
  -> queue open(url) in the target Page pending slot
  -> a later Location request replaces that same slot
  -> Browser or protocol owner adopts the Page
  -> Page creation diagnostics transfer the winning request once
  -> protocol installs exact target-local Held authority
  -> exactly one winning request reaches loader and commit
```

最初实现只完成了 target Page 内的唯一 queue，却仍让 adopted Page 在稍后的普通 renderer output 中发布
winning request。该发布会和 debugger hold、Fetch interception、named reuse、`Page.navigate` 争夺同一
target。现在 staged related initial Page 带有明确的创建标记。owner 在 Page admission 的创建回复中取走
winning non-JavaScript request，并通过 `RendererPageCreationDiagnostics.initial_top_level_navigation` 交给
protocol。protocol 随后把完整 request、history seed、Page residence generation 与 target route 冻结到
`PopupTargetNavigationOwnerAction`，先进入 `Held`，再由同一 target owner 发布和消费。activation 与
target-local request 同时存在会被当成重复 authority 拒绝，不能再依靠相邻 output 的顺序猜测赢家。

`window.open()` 无 URL 后同一轮设置 `popup.location.href` 还暴露了一个更窄的旧条件。target 的观测 URL
仍为 `about:blank`，旧代码即使拿到 exact claim，也要求 target URL 已经改变，结果会把真实 claim
静默丢弃。claimed 路径现在直接比较 claim request 与 current initial URL，并继续检查 exact Page
currentness 和 pending cross-document navigation。两者不同才启动 replacement。显式 `about:blank`
仍留在唯一 initial Document，不会制造伪 replacement。

JavaScript URL 也遵守 target-local 顺序。已经同步 handoff 的 ordinary navigation 保留在队首，随后产生的
JavaScript URL task 排在其后。更晚的 ordinary navigation 会替换整个待执行序列。这样
`window.open(ordinary); window.open(javascript:, sameName)` 保持 ordinary 到 JavaScript 的因果顺序，
同时 `open(old); location = winning` 仍只保留 winning ordinary request。

##### Incumbent source

相对 URL base 已经改为读取 incumbent realm，但 Location request 的 source 一度仍从
target realm 重建。direct Browser 回归明确观察到第二次 opener-driven navigation 的 `document.referrer` 等于 popup
前一页 URL。Location 现在从 incumbent execution context 捕获 source Window、Document URL 和 Referrer Policy。
异步 RemoteWindowProxy command 继续使用命令中已有的 active typed source 覆盖 target-local projection。两条路径随后
进入同一个 `RendererTopLevelNavigationSource` carrier。

##### Renderer-authored history seed

renderer 的 `PendingLocationNavigation.entry_seed` 已正确表达 initial-entry replacement 与
后续 push，但发布 `RendererDocumentSourcedTopLevelLocationNavigation` 时曾丢掉它。protocol replacement 因此每次从
长度 1 重建，`window-open-nourl.html` 和 `window-open-204.html` 的第二次导航都得到错误的 `history.length`。现在 seed
沿以下路径传递，并在 replacement realm 的 document-start work 前安装。

```text
PendingLocationNavigation.entry_seed
  -> RendererDocumentSourcedTopLevelLocationNavigation
  -> RendererPageCreationDiagnostics for a staged related Page
  -> PopupTargetNavigationClaimIdentity
  -> NavigationDispatchState
  -> RendererMainDocumentCommitSeed
  -> RendererMainDocumentCommit
  -> HTML, streaming raw, and NativeDom Page build paths
```

这次也修正了 direct loader 的 referrer 边界。带 typed source 的 request 不再把 creator-policy 结果冻结成显式
`Referer` header。direct owner 把 source URL 设为 network initiator，把 source policy 交给共享 fetch path，使每个
redirect destination 都重新计算 HTTP `Referer`。最终 Document referrer 按实际 commit URL 计算。protocol 继续把
source-policy header 标记为 generated，并在 transport 前移除 preflight projection。只有没有 typed source 的遗留
producer 才使用 frozen explicit-header fallback。

验证过程保留了几次有定位价值的失败。direct Browser referrer 首次断言发现 incumbent source 仍被 target
realm 覆盖，run `5fa7b3fe-5972-400b-b400-f4faf4e1d7f0` 得到 8/9。修正 Location capture 后，最小复跑
`51a07db1-0c55-4b6f-b152-673ed15000a6` 得到 4/4。target admission 第一个 full run 又暴露 9 条 protocol
回归，原因是 winning staged request 仍作为较晚 renderer output 发布。改成 Page creation diagnostic transfer 后，
聚焦 run `0a81a363-8733-44d5-a4cc-fbaecc0342fc` 得到 19/19。

History URL base 最初被统一改成 visible Document URL，随后 full run
`c07e8aa8-e142-4975-b75d-54f97d57ad9b` 得到 15968 passed、8 failed、14 skipped。8 条失败全部属于
child initial-empty joint-history。收窄 top-level 分支后，run `8a631928-0cd0-47e4-8068-8b5397abeb7b`
得到 9/9，随后 full run `ead46364-8977-4440-8ce9-c00a0741fdbc` 得到 15976 passed、14 skipped。

真实 CDP WPT 最后发现一条 Rust 集成测试没有覆盖的边界。首次 final-code 六例得到 5 pass 和 1
`harness-stalled`，唯一失败为 `window-open-nourl.html`。独立单例仍在 120 秒稳定 stalled。target inspection
显示 opener 已 complete，popup 仍停在 `about:blank`，没有 pending navigation。新增 renderer delegated-diagnostics
回归证明 request 已完整离开 staged Page，新增 protocol 回归随后证明旧 target-URL heuristic 丢弃了 exact claim。
修正 claimed predicate 后，独立 CDP 单例恢复为 pass。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run \
  -p moli-dom -p moli-renderer-v8 -p moli-core -p moli-protocol \
  -E '<initial-empty fallback base + target admission + typed source/referrer + history seed 十九条>' \
  --no-fail-fast
# run 0a81a363-8733-44d5-a4cc-fbaecc0342fc
# 19/19 passed, 12797 skipped by the focused filter

TMPDIR=<repo>/target/tmp cargo nextest run -p moli-protocol \
  -E '<same-turn old-to-winning + no-URL Location + explicit about:blank sandbox carrier>' \
  --stress-count 20 --test-threads 4 --no-fail-fast
# run b9195258-65e6-43fa-a61c-878ece415ba4
# 20/20 stress iterations passed

TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run fdfd7204-0e66-4f55-ab2d-d50deea8d12e
# 15978 passed, 14 skipped

TMPDIR=<repo>/target/tmp cargo nextest run -p moli-core \
  -E '<standalone Fresh popup owner record + Fetch pause/continue 四条>' \
  --stress-count 20 --test-threads 4 --no-fail-fast
# run 0bc73f6c-2e2d-41c6-9f97-5779ae4169ac
# 20/20 stress iterations passed，每轮 4/4

TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run de913608-21cf-451a-8886-0d5783b57558
# 15978 passed, 14 skipped

cargo fmt --all --check
# passed

TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# passed
```

外部证据使用 debug binary SHA-256
`137c7a8772906f7c46ad629cc230fdd57110e9685d4dfe5cf39b4f3aeeedf0f1`。WPT checkout 固定为
`db95fafd1fcef8428805e41eb5705d444e8c67ce`，Chromium 对照仍为
`a03603fe9af6230a12f1b2fb2c18a7d003a0d937`。固定 case 顺序、每例 120 秒 timeout，CLI 使用 process-pool，
CDP 对六例各启一个 worker。两个 mode 均为 6/6 case、13/13 subtest 通过，无 fail、timeout、notrun、harness error
或 JS exception。

| case | CLI | CDP | P6R6 结果 |
| --- | --- | --- | --- |
| `window-open-aboutblank.html` | 2/2 pass | 2/2 pass | exact blank 仍复用唯一 initial Document |
| `window-open-history-length.html` | 3/3 pass | 3/3 pass | initial history projection 保持正确 |
| `window-open-nourl.html` | 2/2 pass | 2/2 pass | no-URL 后续 replacement 与 history seed 均正确 |
| `window-open-204.html` | 2/2 pass | 2/2 pass | 204 no-commit 后的下一次 commit 正确 |
| `window-open-204-fragment.html` | 2/2 pass | 2/2 pass | fragment 更新保留 creator fallback base |
| `window-open-204-pushState-replaceState.html` | 2/2 pass | 2/2 pass | History mutation 后相对 Location 与 replacement 均正确 |

原始输出保存在
`/tmp/moli-popup-wpt-cli-p6r6-final-owner-20260823-1` 和
`/tmp/moli-popup-wpt-cdp-p6r6-final-owner-20260823-1`。这组 focused 结果只证明列出的 initial-empty cases，不能替代
更宽的 popup、sandbox、COOP 与 remote WPT 分类。

`StandaloneAuxiliaryPageObservation` 及其 enum、sender、Browser 注册入口和 admission 前 hook 已全部删除。
Fresh popup 回归现在先等待 renderer owner 的 active Page records 出现第二个已采纳 Page，再按该 Page identity 发送 exact
Page command 检查 session storage，最后通过同一 identity 关闭目标。测试无法再用 reservation 已产生冒充 owner 已完成采纳。

三条 Fetch pause 回归改用一次绑定的 production renderer output transport。helper 按 exact Page residence 过滤 typed
`SubresourceFetchPause`，取得真实 `internal_id` 后调用 Page 的 production continue command。最初尝试在 Browser 已创建后把 transport
换给测试，聚焦 run `a5a44072-89e8-4a11-9e82-2bd182208fdf` 得到 1/4。三个失败都由
`one BrowserContext renderer output stream cannot change protocol transport` invariant 拒绝。最终测试改为先给独立
`NavigationEngine` 绑定 transport，再创建 Page。聚焦 run `19171115-ff2a-4a61-b09b-ba2252330c8b` 得到 4/4，随后
20 轮并发复跑全部通过。

这次清理没有重跑外部 WPT。release 行为只增加 renderer owner active Page records 的只读有序快照，popup admission、导航和
Fetch 执行没有改变。上面的 CLI/CDP 结果仍是前一笔 P6R6 browser-semantics commit 的证据，不表示新 commit 产生了相同 binary hash。

P6R6 结束时仍有两项明确工作。direct Browser 当时尚未拥有 cross-document `history.back()` / `forward()` controller，remote
JavaScript URL、descendant lifecycle 与更宽 focused WPT/CDP 也仍需单独验收。固定六例已经从 remaining-risk 列表移除，后续不能
继续沿用 P6R5 的 2 pass / 2 fail / 2 timeout 作为当前状态。history 已由 P6R7 收口，remote JavaScript URL 已由
P6R8 收口，descendant lifecycle 又由 P6R9 按 production reachability 重新分类。

#### P6R7 direct Browser cross-document history owner

P6R6 已经让 renderer-authored history seed 穿过 protocol replacement，但 direct `Browser` owner 仍有两处断点。
`RendererOwnerAction::TopLevelHistoryTraversal` 只带 delta，standalone owner 收到后直接忽略。它没有 protocol 的
`TargetNavigationHistoryState`，无法从 delta 独立找回目标 entry。另一条 Location owner action 虽然已经带有
`NavigationHistoryEntrySeed`，`StandaloneAuxiliaryPageCommand::Navigate` 和 target Page command 仍只传 request，seed
在进入 loader 前丢失。root Page 和 Browser-owned auxiliary Page 因而都可能在第一次 replacement 后只剩局部 history。

本轮把两条路径收进同一个 typed handoff。

1. 已知的 top-level cross-document History traversal 在 renderer 选中目标 entry 时，同时冻结 delta 与完整
   `NavigationHistoryEntrySeed`。seed 的 current entry 就是 destination，保留全部 entry URL、state、key、document id、
   navigation index 和 traverse activation。renderer 无法选出 entry 时仍只发布 delta。这类记录供拥有 browser history
   controller 的 consumer 处理，direct `Browser` 会把它当作 no-op。
2. standalone owner 按 action 所在的 exact Page residence 路由 destination。Browser 自己持有的 auxiliary Page 进入该
   Page actor 的串行 command queue，调用方仍持有的 root Page 进入 renderer browser-owner command。两者都使用现有
   `FollowTopLevelNavigationInStandaloneAdapter`，并把 request 与 seed 一起安装进 target Page 的唯一 pending navigation slot。
3. 普通 `TopLevelLocationNavigation` 走同一条 seed handoff。这个补口覆盖 Navigation API cross-document traverse，也覆盖
   opener 通过 stable RemoteWindowProxy 连续导航真实 popup 的路径。loader、parser、Document replacement、204/no-commit
   和 currentness 仍由 target Page 原有 state machine 负责。
4. protocol target 保留 browser session-history controller 的最终决定权。它只按 action delta 解析 browser entry，不允许
   renderer seed 覆盖或否决结果。browser 发起的连续 `Page.navigate` 可能让 browser history 比当前 renderer realm 的局部
   seed 更长。第一次实现把两边 index 当作强一致条件，focused run 中 protocol back 留在第二页，证明这种校验会把 direct
   fallback 错当成 protocol authority。删掉反向否决后，原有 browser-owned history 回归恢复。
5. renderer output transport 的 retained-memory charge 现在包含 Location 与 History action 携带的 serialized entries 和
   activation。长 history 不再按零字节或仅按 request URL 进入有界 transport。

生产链现在保持以下责任划分。

```text
History API selects a known cross-document entry
  -> renderer freezes delta plus exact destination seed
  -> direct Browser routes by exact Page residence
  -> target Page records request plus seed in one pending slot
  -> target Page loader commits one replacement Document

protocol consumer receives the same action
  -> browser session-history controller resolves delta
  -> renderer seed remains a direct-Browser fallback
  -> browser entry starts the existing protocol navigation transaction
```

回归覆盖四个边界。renderer 单元测试检查 back action 中的 destination URL、current index、完整 entry list 与 traverse
activation。direct root Page 回归先写入两端 `history.state`，再执行 back/forward，确认 PageId、history length 和两端 state
都保留。Browser-owned auxiliary Page 回归通过 stable opener proxy 连续完成两次真实 navigation，再在 exact popup Page 上
执行 back/forward，确认同一 Page residence 往返并最终通过 `window.close()` 退休。protocol 既有回归继续证明连续
`Page.navigate` 后的 renderer `history.back()` 服从 browser-owned history。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run \
  -p moli-renderer-v8 -p moli-core -p moli-protocol \
  -E '<typed traversal seed + direct root/auxiliary back-forward + protocol browser authority 四条>' \
  --no-fail-fast --status-level fail --final-status-level fail
# run 58754570-4f66-4a86-ab8d-15ecfb63b80e
# 4/4 passed, 12658 skipped by the focused filter

TMPDIR=<repo>/target/tmp cargo nextest run -p moli-core \
  -E '<direct root/auxiliary back-forward 两条>' \
  --stress-count 20 --test-threads 4 --no-fail-fast
# run 62e53c2f-abaf-4774-a530-67925c1da542
# 20/20 stress iterations passed，每轮 2/2
```

当前代码构建的 release binary SHA-256 为
`c37d11c6f71333c36015758519ce5a15f32445f435287d6c19de406c928409fa`。WPT checkout 固定为
`db95fafd1fcef8428805e41eb5705d444e8c67ce`，Chromium 对照仍为
`a03603fe9af6230a12f1b2fb2c18a7d003a0d937`。上游
`history_back_1.html` 和 `history_forward_1.html` 都由 opener 创建真实 popup，再检查跨文档 traversal 后回传的页面序列。
两例在 CLI 与 CDP mode 均为 2/2 case、2/2 subtest 通过，harness status 均为 OK，没有 fail、timeout 或 notrun。

```bash
TMPDIR=<repo>/target/tmp cargo build --release -p moli
# passed

uv run --project moli-benchmark python -m moli_benchmark.wpt_cross \
  --wpt-root /home/donoughliu/code/wpt --engine moli --mode cli \
  --moli-bin target/release/moli \
  --output-dir /tmp/moli-wpt-popup-history-p6r7-cli-20260823-1 \
  --case html/browsers/history/the-history-interface/history_back_1.html \
  --case html/browsers/history/the-history-interface/history_forward_1.html
# pass=2

uv run --project moli-benchmark python -m moli_benchmark.wpt_cross \
  --wpt-root /home/donoughliu/code/wpt --engine moli --mode cdp \
  --moli-bin target/release/moli \
  --output-dir /tmp/moli-wpt-popup-history-p6r7-cdp-20260823-1 \
  --case html/browsers/history/the-history-interface/history_back_1.html \
  --case html/browsers/history/the-history-interface/history_forward_1.html
# pass=2
```

提交前仓库门禁也已完成。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast \
  --status-level fail --final-status-level fail --failure-output immediate
# run 1edbe053-59e5-4c68-8251-93f39b0be1a2
# 15981 passed, 14 skipped

cargo fmt --all --check
# passed

TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# passed
```

P6R7 收口的是当前单进程 direct owner 的跨文档 traversal handoff。BFCache、POST history resubmission、scroll restoration
和 crash restore 属于整个 session-history 子系统，不能从这组 popup 回归外推为已经完成。remote JavaScript URL
现已由 P6R8 收口。P6R9 又确认本地 target Page 已拥有 descendant lifecycle，remote wire 不应传 DOM form value。
P6R7 结束时剩余的 receiver/entry identity、通用 child scheduler 与 focused WPT/CDP 已由 P6R10 收口。
前一笔已经删除的
`StandaloneAuxiliaryPageObservation` 不会以 history observer 或 test-only controller 的形式恢复。

#### P6R8 remote JavaScript URL 与目标 realm owner

G5/G6 已经让 related Page 在跨 script-agent replacement 后保留同一个 logical WindowProxy endpoint，也能把
ordinary Location、named navigation、form request 和 child scheduler 送到 exact target Page/Frame。这里仍有一条
生产可达的断路。目标跨源后再回到与 source 同源时仍保持 remote projection，`javascript:` 不能借 local V8 object
直达目标。旧分支会拒绝或静默丢弃这次导航。remote child 则只能把 URL 塞进 ordinary request，接收方会把它误认成
跨文档 load。

本轮复核固定了四个责任边界。

1. named lookup 先固定 exact target 并执行 `CanNavigate`。准入的 existing hit 随后在发布 command 前检查 source
   Document 的 inline-navigation CSP。main world 与 isolated world 都保留 exact `WindowExecutionContextIdentity`。
   isolated world 的 `grants_universal_access` 单独参与 Window access，不能被折叠成 source origin 相同。
2. 通过准入的 request 使用 `NavigateJavaScriptUrl` 或 `NavigateFrameJavaScriptUrl`。普通 `Navigate` 与
   `NavigateFrame` 在构造和 wire decode 两端都拒绝 `javascript:`，防止协议 carrier 绕过 source policy。
3. target owner 在 ACK 前复核 endpoint、Page residence、execution-channel generation、root Document、exact child token
   与当前 access origin。通过后只把任务放进 target Page 或 child Frame 的现有 networking-task queue。脚本始终在目标
   main realm 执行，source world 只保留 policy 和 access 语义。
4. target task 继续使用 E2O 已有的 current Document、target CSP、Trusted Types、异常和 completion owner。non-string
   completion 保留 Document/realm，string completion 才进入既有 replacement transaction。

source carrier 由以下 typed facts 组成。

```text
exact source root or child Window identity
  + source Page residence and related-group endpoint
  + root Document lifecycle identity
  + serialized tuple origin or group-safe opaque nonce
  + current document.domain override
  + main or isolated source world
  + isolated-world universal-access bit
  + referrer and suppression navigation source
```

`RemoteWindowProxy` wire 升到 version 2。decode 会验证 source 与 target 属于同一个 browsing-context group，source
root Document 与 source Page 相符，child source token 指向同一 endpoint 和 root Document，tuple/opaque identity 与
`document.domain` 形状一致，command kind 与 top/frame route 相符。target dispatch 再消费本地当前状态，wire 中冻结的
origin 不能替代 target 当前 origin。

当前 wire 由同一进程内已经验证的 source owner 生成，isolated world 的 universal-access bit 还不是 browser 签发的
安全 capability。未来若接入不可信 renderer process，这个权限必须由 browser broker 绑定 source endpoint 和 channel，
不能接受 renderer 自报。P6R8 只把当前产品已有的 trusted isolated-world identity 无损送过 owner 边界。

named target lookup 也补了一个独立但必要的语义。匹配到同名 remote top 或 child 后，`CanNavigate` 拒绝只会让这次
导航成为 no-op。resolver 仍返回已选中的 stable WindowProxy，`window.open()` 仍返回该对象，hyperlink/form 也不会把
拒绝解释成 name miss 后新建第二个 popup。这个结果同时避免错误消耗 popup activation。

`document.domain` 以前只在 remote frame-tree snapshot 中更新。same-origin remote top 因而可能在 source/target 已经
共同放宽 domain 后继续按旧 top origin 判断。本轮让 top Document 的 domain mutation 立即重发 top-level target state，
source carrier 也冻结发出时的 domain。target 在 command 到达前发生 domain mutation 时会返回 negative ACK，channel
generation 保持不变，后续 source 采用相同 domain 后的新 command 才能通过。

实现过程中聚焦回归暴露了一个 owner 分类错误。remote child form 需要保留 method、body、headers 与 scheduler id，
所以 wire 解码后使用 `ChildBrowsingContextBootstrap::Request`。原来的 JavaScript URL fast path 只识别
`ChildBrowsingContextBootstrap::Url`，导致 non-string completion 仍退休旧 child realm。第一次 G6 运行因此无法再用缓存的
execution context id。诊断把 child default realm inventory 从执行前后的 `[2, 3]` 与 `[3]` 对照出来。最终修复放在
bootstrap classification、pending-task query 与 history seed 三个 owner 边界，`Request(javascript:)` 现在与
`Url(javascript:)` 进入同一个 target task，不经过 loader/parser commit。

Chromium 对照固定在 `a03603fe9af6230a12f1b2fb2c18a7d003a0d937`。

| Chromium owner | 固定基线位置 | P6R8 采用的事实 |
| --- | --- | --- |
| source admission | `third_party/blink/renderer/core/loader/frame_loader.cc:545-565` | `FrameLoadRequest` 用 origin Window 与 `JavascriptWorld()` 检查 source browsing context CSP |
| target scheduling | `third_party/blink/renderer/core/loader/frame_loader.cc:825-854`、`third_party/blink/renderer/core/dom/document.cc:9613-9640` | 选中的 target Document 接收 URL，并在 networking task 中批量执行 |
| source/target policy split | `third_party/blink/renderer/core/frame/local_dom_window.cc:514-558` | source inline check 不执行 Trusted Types，最终 task 的 `CheckAndGetJavascriptUrl()` 再处理 target policy |
| target realm execution | `third_party/blink/renderer/bindings/core/v8/script_controller.cc:248-320` | classic script 对 target `LocalDOMWindow` 执行，并按 target provisional navigation 与 completion type 决定是否替换 Document |

Moli 当前的 isolated world 没有独立 CSP/Trusted Types policy container，因此 main/isolated source 都读取同一 source
Document policy，target task 也读取目标 Document policy。world kind 仍进入 wire，避免 universal access 与普通 isolated
source 混同，也为未来增加 world-specific policy 留下 typed 位置。这一阶段没有声称复制 Chromium extension world 的全部
bypass 行为。

回归矩阵覆盖以下可观察结果。

- cross-origin named target 被 main-world source 拒绝后仍返回同一 proxy，不发布 command、activation 或第二 Page。
- universal isolated source 可以命中 cross-origin remote target，target main realm 执行，target isolated realm 不变。
- target 回到 source origin 后仍保持 remote agent，Location、named hyperlink 与 isolated-world `window.open()` 都进入
  typed top command。
- command 发出后 target `document.domain` 改变会 negative ACK，source/target 采用相同 domain 后可以重新准入。
- source CSP 在发布 command 前拒绝，target CSP 在 positive ACK 后的 delayed target task 中拒绝。
- remote child Location、hyperlink 与 POST form 都使用 exact frame token。form 保留 scheduler id，三条路径都在 target
  child main realm 执行。
- non-string remote child JavaScript URL 执行前后 execution-context inventory 完全相同。
- wire round trip 保留 world/domain/source route，并拒绝 generic JavaScript URL、forged group 与 forged opaque nonce。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 \
  -E '<wire 两条 + remote top/child 两条 + G5 + local child JS/TT + local popup JS + source CSP 三条>' \
  --no-fail-fast
# run 4ac54d27-f725-4742-8e46-2eec9fa08615
# 11/11 passed, 7289 skipped by the focused filter
```

提交前 workspace 门禁也在同一代码工作树上通过。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast
# run 768204b1-a651-472b-88cc-9d2f62750026
# 15984 passed, 14 skipped

cargo fmt --all --check
# passed

TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# passed in 1m 32s
```

G6 的首次失败与修正后证据保留如下。

```text
72a04a2d-f290-48c7-8ca0-3a0ea7f83bd2  cached child context id became unknown
080da5de-fdd9-4e2e-acbc-8985364a2a91  realm inventory changed from [2, 3] to [3]
78e8d845-40a8-4eb9-add6-fb2f180230d4  owner fix passed the complete G6 case
```

这一纵切没有恢复 P6R4 删除的 observation seam 或 lightweight executor。完整 owner、loader、parser、realm alias 和
lifecycle state 应继续物理删除。仅供测试构造输入或查询 production state 的无副作用 fixture/accessor 才适合
`cfg(test)`。P6R9 已把 remote descendant 与 form DOM carrier 按 Chromium wire 和 production reachability
重新分类。P6R10 随后完成 receiver/entry/accessing identity、通用 child script scheduler 与更宽
focused WPT/CDP 分类。extension world 独立 policy 仍按产品支持面单独处理。

#### P6R9 child browsing-context name 与 entry/incumbent navigation identity

P6R9 不是继续扩展 popup facade。它复查现有真实 Page/Frame owner 在普通 child 场景中的两个 identity 漏洞，
并用 Chromium 与外部 WPT 校准 P6R8 留下的 exit condition。两处修复都落在 child browsing-context owner
和 navigation request owner，popup 只复用修正后的通用 primitive。

##### Browsing-context name owner

旧 `ChildBrowsingContextEntry::name` 同时表示 browsing-context name 和 `<iframe name>` 属性。
`refresh_child_browsing_context()` 每次同步 owner element 时都会重新读取 DOM attribute，再重建 entry。
脚本执行 `childWindow.name = "runtime-name"` 后，只要 parent 改动或删除 `<iframe name>`，下一次 refresh
就会覆盖 runtime name。后续 `_self`、ordinary named lookup 和跨文档 replacement 都可能丢失同一个 frame。

Chromium 基线 `a03603fe9af6230a12f1b2fb2c18a7d003a0d937` 给出的 owner 很清楚。

| Chromium 位置 | 采用的事实 |
| --- | --- |
| `third_party/blink/renderer/core/frame/local_dom_window.cc:1772-1784` | `window.name` 读写 `FrameTree` name，并按 frame-tree owner replication |
| `third_party/blink/renderer/core/html/html_frame_element_base.cc:173-175` | owner attribute 只在 frame 建立时初始化 frame name |
| `third_party/blink/renderer/core/html/html_frame_owner_element.cc:694-697` | subframe creation/navigation 显式接收已经确定的 `frame_name` |

本地 Chromium 最小探针依次执行 initial owner name `a`、owner rename `b`、`window.name = "c"`、
owner rename `d` 和 owner attribute removal，结果为 `a|a|c|c|c`。这证明 owner attribute mutation
不会回写已经存在的 browsing-context name。

Moli 现在把字段改为 `browsing_context_name`。新 entry 仍由初始 owner attribute 初始化，既有 entry
refresh 则保留 runtime name。frame identity snapshot、named lookup 和 `window.name` getter/setter 都读取
同一个字段。回归覆盖 owner rename、attribute removal、`window.open(blob, "_self")` replacement 与新 realm
继续观察 `runtime-name`。

上游 `html/browsers/windows/browsing-context-names/choose-_self-001.html` 在修复后的 release binary 上，
CLI 与 CDP 都由旧失败转为 pass。该用例从 child 将自己命名后调用 `window.open(..., "_self")`，能直接证明
修复进入 production target selection 和 replacement，不只是 test accessor 的状态变化。

##### Entry base 与 incumbent source

Location API 需要同时保存两类来源。相对 URL 按 entry settings object 的 base 解析，fetch client、referrer
与 navigation initiator 则来自 incumbent settings object。旧 `entered_window_api_base_url()` 使用
`get_incumbent_context()`，把两者折叠成一份 realm。local child cross-document Location 随后只排入
`ChildBrowsingContextBootstrap::Url`，即使调用点捕获了 source，也会在 target loader 前丢失。

P6R9 采用 V8 `get_entered_or_microtask_context()` 解析 entry execution-context identity，并从该 identity
读取 Document base。incumbent source 继续由现有 navigation-source capture 负责。HTTP(S) child Location
现在构造 `ChildBrowsingContextNavigationRequest`，统一冻结以下值。

```text
entry Window/Document base
  -> absolute target URL

incumbent Window/Document source URL
  + referrer policy
  + suppress-referrer decision
  -> network Referer
  + committed document.referrer
  + typed initiator source
```

request builder 已从 element activation 与 remote-frame caller 的重复实现上移到
`ChildBrowsingContextNavigationRequest`。local child、related child 与 remote frame 因而使用同一个 GET
carrier。Location 的 history increment 仍由原 owner 处理，request queue 不重写 renderer-authored seed。

这次升级只用于 HTTP(S)。`blob:` 等路径当前可在 child owner 中同步 materialize Document；把它们包装成
network-shaped Request 会改变 commit timing，并让 `_self` replacement 失去同步完成。P6R9 保留原 URL bootstrap，
后续若统一 non-network navigation carrier，需要先给 carrier 建立明确的 materialization kind，不能靠 scheme
外推异步 loader。

内部三 realm 回归建立 entry、incumbent 与 relevant sibling。entry realm 调用 top bridge，top 再调用 incumbent
函数，incumbent 最终写 relevant Location。断言同时锁住 entry base 解析出的
`https://multiple-globals.test/entry/target.html`，以及 incumbent `about:srcdoc` source identity。
修复前该用例拿到 URL-only bootstrap，修复后得到 typed Request。

##### Remote form carrier 的 reachability 修正

P6R8 文档把 remote target 的 `NavigateEvent.sourceElement` 与 V8 `FormData` carrier 列为当前缺口，
这项要求超出了 Chromium 行为，也违反 isolate ownership。

| Chromium 位置 | 采用的事实 |
| --- | --- |
| `third_party/blink/renderer/core/loader/frame_loader.cc:882-899` | 只有 origin Window 可以访问本地 target frame 时，target `NavigateEvent` 才接收 `source_element` |
| `third_party/blink/renderer/core/frame/remote_frame.cc:263-319` | remote wire 发送 URL、initiator、POST body、headers、referrer、form flag、gesture 与 frame tokens，不发送 DOM/V8 value |
| `third_party/blink/renderer/core/navigation_api/navigation_api.cc:815-833` | `FormData` 由 target 进程可见的本地 `source_element` 重建 |

Moli 不应为跨 agent form 构造伪 DOM facade。remote route 继续传 method、raw body、Content-Type、headers、
referrer、form flag、source scheduler id 和 exact target binding。source element 与 `FormData` 只在 same-agent
local route 中存在。这样既保留网络与取消语义，也不让一个 isolate 持有另一个 isolate 的 V8 object。

remote descendant lifecycle 也按相同原则重新分类。当前 remote projection 的 target 仍是一份真实 Page，
它自己的 `JsContextHost` 拥有本地 child subtree。Phase 5L1 已让该 Page 在 close transaction 中递归派发
beforeunload、pagehide、unload 并提交 renderer ACK。source agent 不需要再遍历 target 的 DOM。
只有产品引入真实 OOPIF 或 renderer process descendant 后，browser/process owner 才需要聚合多 endpoint ACK。

##### 外部 WPT 与失速归因

WPT checkout 固定为 `db95fafd1fcef8428805e41eb5705d444e8c67ce`。本轮先用改动前 release
跑 81 case 的 popup 关键切片，CLI 为 35 pass、27 fail、19 timeout，CDP 为 35 pass、27 fail、
19 harness-stalled。两种入口的 case 分类一致，说明当前主要差距来自 renderer 行为，没有出现 CLI/CDP
两套 popup owner 再次分叉。该结果只是宽口径基线，不能作为 P6R9 修复后通过率。

修复后的 release binary SHA-256 为
`92b464d6e6710c9a9dfb5b4609d03ddf85e4d42ecf4f6bb8dad945afe83435b4`。
四例 focused slice 的结果如下。

| 用例 | CLI | CDP | 结论 |
| --- | --- | --- | --- |
| `choose-_self-001.html` | pass | pass | browsing-context name 修复已由 production 外部路径证明 |
| `context-for-location.html` | timeout | harness-stalled | testharness 没有收到 result/completion callback |
| `context-for-location-assign.html` | timeout | harness-stalled | 同上 |
| `context-for-location-href.html` | timeout | harness-stalled | 同上 |

CDP 逐 frame 探针证明 entry、incumbent 与 relevant 都达到 `readyState = "complete"`，entry body 的
`onload` 也已派发。失败发生在 `context-helper.js:33`。entry onload 向 incumbent Document 插入 inline
classic script 后立刻调用 `incumbent.contentWindow.go()`，Moli 抛出
`TypeError: incumbent.contentWindow.go is not a function`。稍后检查时该函数已经出现。这说明动态插入到
child Document 的 inline classic script 没有在 `appendChild()` 返回前同步执行。

手工再次调用 entry onload 后导航会继续，但这次调用由 top-level CDP evaluation 进入，URL base 也随之变成
top-level entry realm，不能当作 multiple-globals 语义通过证据。P6R9 因此采用内部三 realm 回归证明
entry/incumbent carrier，把三个外部 timeout 记到通用 child dynamic-script scheduling，不把它们写成 popup
target 或 Location owner 已通过。

聚焦验证如下。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 \
  -E 'test(child_window_name_survives_owner_refresh_and_self_navigation) or test(cross_realm_location_uses_entry_base_and_incumbent_navigation_source)' \
  --no-fail-fast
# run f64cd39c-af81-4a25-ac20-bf894b474ba8
# 2 passed, 7300 skipped

TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 \
  -E '<P6R9 two regressions plus six adjacent child/window/location/source cases>' \
  --no-fail-fast
# run 68bd48a0-9037-48d6-808c-930bc0ccd489
# 8 passed

# P6R9 两例、form replacement、related Page projection 和跨源 named child projection
# 连续运行十轮
# run ba3b26fb-6b92-4395-8086-6d3008af104b
# 50 passed
```

第一轮 full nextest 暴露出三个仍把 owner attribute 当作 live browsing-context name 的旧 fixture/assertion，
以及一个随后单跑通过的 parser backlog 用例。fixture 已改为让 child 自己写 `window.name`，没有放宽
production 语义。修正后第二轮 full nextest 只出现一次
`related_popup_same_turn_retarget_admits_only_winning_initial_navigation` 失败。该用例随即单跑通过，之后连续
五十轮全部通过。最终门禁重新从头运行并全绿，保留这段中间结果是为了避免用一次成功掩盖时序风险。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast
# run bddb83d3-d505-4c6e-bda2-ddec4c31d2c2
# 15986 passed, 14 skipped

cargo fmt --all --check
# passed

TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# passed

TMPDIR=<repo>/target/tmp cargo build --release -p moli
sha256sum target/release/moli
# 92b464d6e6710c9a9dfb5b4609d03ddf85e4d42ecf4f6bb8dad945afe83435b4
```

外部产物保存在 `/tmp/moli-wpt-popup-p6r9-entry-source-cli-20260823-1` 和
`/tmp/moli-wpt-popup-p6r9-entry-source-cdp-20260823-1`。最终 release 哈希与运行这组 focused WPT 时
一致，后续仅修改了测试 fixture 和本文档，所以结果仍对应当前 production binary。

#### P6R10 Window call realm identity 与同步 child script owner

P6R10 直接完成 P6R9 留下的两个 exit。实现范围覆盖 `window.open()` 的 typed call identity、related Page
borrowed receiver、creator resource authority、top-level causal request carrier，以及 child Document 动态 inline
classic script 的同步 mutation semantics。没有增加 popup-specific state。

##### Chromium 的 receiver 与 entry 分工

固定 Chromium 基线的 `LocalDOMWindow::open()` 在
`third_party/blink/renderer/core/frame/local_dom_window.cc:2296-2450` 保留以下调用顺序。

1. generated binding 先完成 Window receiver brand/access check，并在当前 calling realm 中做 WebIDL argument
   conversion。conversion 可以同步执行 author getter。
2. native `LocalDOMWindow::open()` 把 `this` 保留为 receiver，同时读取 `EnteredDOMWindow(isolate)`。
3. entered Window 完成 URL，构造 `FrameLoadRequest`，提供 referrer policy、outgoing referrer 与 creator fetch client。
4. receiver frame 的 `FrameTree` 执行 `FindOrCreateFrameForNavigation()`。sandbox、popup admission、transient
   activation、opener relation、session storage clone 与 special/named target 都依附 receiver。
5. 选定的 frame 消费由 entered Window 创建的 request。已有 target 与新 auxiliary context 使用同一套 request
   facts，不会把 target root 重新解释成 initiator。

这里存在三个不可互换的身份。

| 身份 | 提供的事实 |
| --- | --- |
| receiver | `this` 对应的 exact LocalWindow、target tree、sandbox、activation、opener 与 creation transaction |
| entry | URL base、Document policy、CSP/Trusted Types、resource loader、referrer 与 destination request source |
| accessing/calling realm | receiver access check、WebIDL conversion 与异常所属 realm |

P6R10 没有继续使用 incumbent 或 current realm 猜测这三组事实。`current_realm_owner_dispatch_scope()`
现在返回 `Option<OwnerDispatchScope>`，无法找到 exact binding 时 fail closed，也不再隐式退回 Top。

##### Typed receiver capture 与 generation currentness

`WindowOperationReceiver` 在 argument conversion 前冻结 receiver 的 owner、dispatch scope、realm token 与
V8 context。conversion 返回后，`resolve_live_binding()` 只接受原 binding 仍是 current 的情况。稳定 child
handle 只用于找到 capture 时的 scope，不能在 iframe replacement 后重新绑定到新 LocalWindow。

回归 `window_open_receiver_generation_is_frozen_before_url_conversion` 让 URL `toString()` 删除并重新插入
receiver iframe。调用返回 `null`，replacement child 保持 initial `about:blank`。这锁住了一个容易被 stable
WindowProxy 掩盖的 generation race。

receiver host 与 accessing host 也不再共用一个 registry。same-host receiver 使用本地 dispatch-scope access
check，related Page receiver 使用 related script-agent membership 与两端 effective origin 检查。跨源 top/child
与 remote frame proxy 保持 fail closed。group-qualified related endpoint marker 同时存在于同源和跨源 proxy，
所以 marker 只参与形态识别，不能单独决定授权。

第一次加入粗粒度 marker 拒绝后，focused nextest run
`98c76f0a-36d4-4406-9df6-84888713f187` 出现 3 pass / 1 fail。同源 related popup 的 borrowed
`Window.open()` 被错误返回 `null`。删除这条粗粒度拒绝并保留 typed origin check 后，run
`58c2c017-b855-4f34-85d0-4616a741ead4` 的四条回归全部通过。这个中间失败证明测试检查了真实
跨 Page receiver route，没有只验证本地 child。

##### Creation 与 navigation 的两组 source

P6R10 进一步拆开 creation activation source 与 destination request source。

```text
receiver Page/Frame
  -> target lookup
  -> sandbox and transient activation
  -> opener/session storage relation
  -> RendererPendingPopupActivation source

entry Page/Frame
  -> URL/base and creator policy
  -> DocumentResourceLoader
  -> referrer/CSP/Trusted Types
  -> RendererTopLevelNavigationRequest source
```

所有 existing related hit、Fresh/noopener、staged Related fallback 与最终 browser-owned fallback 都显式携带
同一份 `RendererTopLevelNavigationRequest`。早期回归曾发现 referrer tuple 已正确，但 activation 丢失 causal
request source。补齐所有 activation branch 后，receiver sandbox admission 与 entry navigation source 可以同时
成立。

`open_renderer_owned_related_auxiliary_page()` 现在接收明确的 creator `DocumentResourceLoader`。调用方不再从
ambient current/receiver realm 二次发现 network authority。新 related initial policy 组合 entry Document
policy 与 receiver 已准入的 sandbox/escape 结果，避免把 raw sandbox flags 重算一次并破坏
`allow-popups-to-escape-sandbox`。

special/named target helper 也接收 exact receiver Window 与 identity。child `_self` 只有在 target scope 确实是
Top 时才进入 top-level Page queue；child target 继续由 Location/Frame owner 处理。显式 top-level navigation
carrier 优先于 incumbent-derived fallback，因此跨 child Location handoff 不会覆盖 entry source。

##### Child dynamic inline classic script 的同步边界

P6R9 的三条 Location WPT 在 target request 已正确后仍 timeout。最小探针显示 entry onload 向 incumbent child
Document 插入 inline classic script，`appendChild()` 返回时函数尚未定义，稍后才出现。错误属于通用
Document mutation/script scheduler。

P6R10 用 `RuntimeScriptStartCandidate` 保存 mutation turn 中的 script discovery 顺序。候选可以是 main Document
script，也可以是带 exact child handle 的 inline classic script。child candidate 在对应 owner scope 中完成
source、nonce、integrity、CSP 与 Trusted Types 准备，并在 mutation followup 返回前进入 child realm 执行。

microtask checkpoint 仍由最外层 DOM API turn 负责。nested dynamic insertion 不单独 drain microtask，
`document.close()` 路径继续按原 parser boundary drain。回归
`child_initial_about_blank_executes_dynamic_inline_script_synchronously` 同时检查三件事。

- inline body 在 append 返回前已经执行；
- script 内创建的 Promise 在外层 checkpoint 才执行；
- script 同步移除自身 iframe 时，不会让旧 owner 的后续任务进入 replacement 或 detached realm。

旧异步路径已经物理删除，包括 `PendingChildDynamicDocumentScript`、
`FrameDocumentDynamicClassic*`、`FrameDocumentUnboundScriptWork::DynamicClassic`、ready task、owner action 与
followup。编译器在第一次删除入口后暴露出剩余 dead enum branch，本轮继续删除这些 branch，没有用
`cfg(test)` 或 `allow(dead_code)` 隐藏。

##### 删除与 `cfg(test)` 的边界

P6R10 复核了两个 child-window private marker。`ENTERED_CHILD_WINDOW_HANDLE_SLOT` 没有 production writer，
读路径只能制造 ambient fallback，现已连同 getter/export 一起删除。`ACTIVE_CHILD_WINDOW_HANDLE_SLOT` 有真实
production enter/restore writer，继续服务 CSP、Trusted Types、script preparation 与 execution 的 exact child
scope。

这次判断沿用 P6R4 的规则。

- production 不再可达的完整 owner、scheduler、parser、loader、realm alias 与 lifecycle state 直接删除；
- 只构造 production 输入的 fixture 可以保留；
- 无副作用读取 production state 的 test accessor 与断言可以保留；
- 测试不能调用一套只在 `cfg(test)` 编译的旧 owner 来证明 release 行为。

当前 tracked Rust 对 `lightweight_popup|LightweightPopup|lightweight popup` 的宽口径扫描为零，
`ENTERED_CHILD_WINDOW_HANDLE_SLOT|entered_child_window_handle` 为零，旧 DynamicClassic stack 的专用符号也为零。

##### 聚焦回归、全量门禁与外部矩阵

receiver 与 entry 拆分完成后的第一组聚焦回归如下。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run -p moli-renderer-v8 \
  -E 'test(child_initial_about_blank_executes_dynamic_inline_script_synchronously) | test(window_open_receiver_generation_is_frozen_before_url_conversion) | test(cross_realm_window_open_uses_receiver_target_and_entry_request_policy) | test(related_window_proxy_location_wakes_the_exact_standalone_target_page)'
# run 58c2c017-b855-4f34-85d0-4616a741ead4
# 4 passed, 7299 skipped
```

`cross_realm_window_open_uses_receiver_target_and_entry_request_policy` 同时锁住 receiver target、receiver sandbox、
receiver activation source、entry URL/base 与 entry destination request source。related Page runtime 回归用无效 URL
要求 borrowed receiver 抛出 `SyntaxError`，证明调用已经到达 URL parsing，而没有把 live target 当作 stale/null。

第一次 workspace 门禁随后暴露了两类历史假设。旧测试仍要求 dynamic inline child script 经过
`DocumentScriptReady` 异步队列。这个 release 路径已经被同步 mutation candidate 取代，其中三个测试组成的
`runtime/page_vm/tests/child_document_script_ready.rs` 整文件删除，其余覆盖改写为同步 owner、typed realm
publication 与 detach lifetime 断言。旧 owner 没有搬进 `cfg(test)`。

同一轮还发现 sandboxed contextual fragment 会进入 child script preparation。共同入口
`start_connected_child_document_script()` 现在先检查 exact owner Document 是否允许 scripting，再准备 inline、
external 或 module script。修正后的五项 owner 回归在 run
`725ee5a6-b067-4d6d-8bf4-8b67c508e124` 中全部通过。

协议全包并发又揭示两条 same-turn popup 测试等待了 URL commit，却在目标 Document 到达 load 前读取
`document.title`。URL 与 history 已经是 winning navigation，标题仍可能来自空 Document。测试保留标题强断言，
attach 后改用 `Page.enable`，再等待 exact popup session 与 frame id 的 `Page.frameStoppedLoading`。无 URL
用例压力运行 50 次全过，run 为 `b1e85cc2-ef82-46fa-b93b-7e76f02fb106`。retarget 用例压力运行
100 次全过，run 为 `c24d9293-3ea8-4ac0-bb48-ef26a5161a11`。最终协议全包 run
`48dbd8f6-c74b-4ce2-9fc6-8908f3d00360` 为 3358 passed。

最终代码上的八项 owner 聚焦回归覆盖 receiver generation、跨 Page borrowed receiver、receiver 与 entry
policy、related target wake、同步 child inline script、sandbox gate、typed realm publication 与 detach
lifetime。run `05d8533a-b4c8-41ca-ba16-4aefe817ac30` 为 8 passed，7292 skipped。

提交前 workspace 门禁使用同一份最终 Rust 代码。

```bash
TMPDIR=<repo>/target/tmp cargo nextest run --no-fail-fast
# run a247565f-762c-4223-9b7a-6820321132ba
# 15984 passed, 14 skipped

cargo fmt --all --check
# passed

TMPDIR=<repo>/target/tmp cargo clippy --workspace --all-targets -- -D warnings
# passed in 1m 33s
```

最终 release binary SHA-256 为
`f193e206c22d7888940a7ea7ab6a15fbf8a2d8f8233f3f6914b34976b7599a1b`。四例 focused WPT 如下。

| WPT | CLI | CDP |
| --- | --- | --- |
| `multiple-globals/context-for-location.html` | pass | pass |
| `multiple-globals/context-for-location-assign.html` | pass | pass |
| `multiple-globals/context-for-location-href.html` | pass | pass |
| `multiple-globals/context-for-window-open.html` | pass | pass |

同一 binary 随后重跑固定 81-case popup 清单。CLI 为 40 pass / 26 fail / 15 timeout，CDP 为
40 pass / 26 fail / 15 harness-stalled。归一化后的逐 case status 完全一致。相对 P6R9 基线新增五个 pass，
没有丢失既有 pass。相对早一版 P6R10 binary 的逐 case status 与 failure-name 也没有变化。
focused 与完整矩阵产物分别保存在
`/tmp/moli-wpt-popup-p6r10-final2-multiple-globals-{cli,cdp}-20260823-1` 和
`/tmp/moli-wpt-popup-p6r10-final2-{cli,cdp}-20260823-1`。

P6R10 至此完成当前单进程 popup owner 计划。剩余 26 fail 与 15 timeout 是下一轮兼容性分类输入，
不能在未确认 owner 前继续扩展 popup transaction。真实 renderer process、OOPIF、fenced/guest 与完整 Reporting
仍按产品决策独立立项。

### 公开仓库迁移与 stable Page owner 适配（2026-08-24）

旧仓库 `popup-refactor` 的十二个内聚提交已经按原顺序迁移到公开仓库，公开提交范围为
`486e7d8bf` 到 `b8ba0364d`。迁移前的公开基线 `0e597fc37` 另保存在
`backup/popup-refactor-pre-import-20260823-0e597fc37`，因此导入前状态仍可独立检查。

公开仓库在这段时间已经建立了更严格的 stable Page / replacement Document owner。直接保留旧仓库的隐含
假设会让代码通过 cherry-pick，却在完整门禁中暴露错误，因此本轮同时完成以下适配：

1. `TargetNavigationLoadInputs` 用显式 `replace_stable_page` 表达 replacement admission，不再把
   `RendererMainDocumentCommitSeed` 的存在误当成 Page replacement identity。
2. `DocumentCommit` reply boundary 只发布命令响应；typed document continuation 一直保留到 exact
   post-parse lifecycle 完成并发布最终 Page state。`Pending`、`Blocked` 与中途 Document replacement 都不能提前
   settle continuation。
3. stable Page 更换 Document 时，把旧 Document 的 Network request-id correlation 移入 retiring output
   residence。旧 realm teardown 产生的 Fetch/XHR `ERR_ABORTED` terminal 因此仍在 successor Document 可观察前
   精确投影，不会因 Page residence 未变化而被清空。
4. inspector pause route 由“output journal 创建时的 agent”改成显式的 current
   `RendererDevToolsAgentToken -> stable Page output journal`。replacement Document 的新 agent 因而可以继续发布
   `Debugger.paused`，同时 Page output stream 保持稳定。
5. WebDriver Classic element frame target 在查找 frame owner 前先走已有的 attached-element 验证，stale element
   返回 stale element reference，不再错误降级为 no such frame。
6. 依赖 author script、binding 或 utility world 已完成的协议测试显式等待 document continuation；Fetch fixture
   先消费 initial empty Document 的 load event，避免把旧事件误认成新 navigation 的因果前缀。

公开仓库最终 Rust 状态使用以下门禁验收：

```bash
cargo nextest run --no-fail-fast
# run 36bfddaf-5b9f-401e-88e7-ae3bff58165f
# 16530 passed, 14 skipped

cargo fmt --all --check
# passed

cargo clippy --workspace --all-targets --all-features -- -D warnings
# passed in 51.69s
```

这组适配不恢复 lightweight popup 双栈，也不改变 P6R10 的产品边界；它把已经完成的 popup owner 设计接到
公开仓库当前的 typed replacement、output residence 和 inspector routing 基础上。

## 验收不变量

迁移完成至少要满足以下不变量。

### Identity / realm

- 同一 popup 的 WindowProxy 跨 navigation 身份稳定。
- popup 与 opener 是不同 realm：`popup.Array !== opener.Array`，global lexical state
  不共享。
- same-origin 同步读取、函数调用、DOM object access 作用于 popup 的真实 Document。
- cross-origin access 只暴露允许 surface，round trip 不复活旧 LocalWindow。
- initial empty same-origin 首次 commit 只在满足明确 policy 时复用 LocalWindow。
- top-level WindowProxy endpoint 的 `(group, generation)` 在 normal Document replacement 中保留，在 COOP group
  switch 中更换；stale pair 不能命中复用同一 Page residence 的 replacement realm。

### Ownership / lifecycle

- 一个 browsing context 只有一个 current Document owner、一个 history 和一个 loader。
- 一个 navigation token 只有一个 authoritative load；redirect 不算第二 owner。
- 旧 Document/realm 的 timer、fetch、module、worker、message 和 callback 不能修改新
  generation。
- popup 可独立于 opener 存活和关闭。
- named target、opener edge、close state 只有一个 registry source of truth。
- disconnected endpoint 的 message/location/close/focus 必须统一丢弃，且 `postMessage` 不得回退到 incumbent Page。

### CDP

- 一个 auxiliary top-level context 对应一个 target 和同一个 renderer Page residence。
- opener handle mutation 与 CDP `Runtime.evaluate` / DOM snapshot 观察同一个 Document。
- target create/attach/context-created/load/target-destroyed 顺序稳定。
- `waitForDebuggerOnStart` 不靠 sleep，且不会让 CLI 与 CDP 使用不同完成条件。
- Runtime object id、context id、binding、object group 和 exception event 按 target/session
  精确路由，即使多个 Page 共用 isolate。
- `Target.closeTarget` 与 `window.close()` 汇合到同一 close transaction。

### Policy / storage / network

- blocked popup 不创建隐藏 Page、target 或 storage namespace，也不误消耗 activation；
  allowed creation 按 Chromium 语义消耗 activation。
- `noopener` / `noreferrer` 返回值、opener、referrer、name/group 和 storage 行为由同一
  policy result 决定。
- session storage namespace 只分配/clone 一次。
- Fetch/Network 事件、cookie、cache、redirect 和服务端副作用来自唯一 loader。

## 测试建议

### 已有聚焦 nextest

popup 当前路径：

```bash
cargo nextest run -p moli-core -p moli-renderer-v8 -p moli-protocol \
  -E 'test(direct_browser_owner_drives_related_auxiliary_page_and_remote_opener_message) | test(direct_browser_owner_routes_named_reuse_to_externally_held_root_page) | test(direct_browser_owner_preserves_root_history_across_cross_document_back_and_forward) | test(direct_browser_owner_preserves_auxiliary_history_across_cross_document_traversal) | test(top_level_cross_document_history_traversal_carries_the_exact_destination_seed) | test(renderer_history_back_uses_browser_owned_navigation_history) | test(related_window_proxy_location_wakes_the_exact_standalone_target_page) | test(per_page_isolate_policy_keeps_window_open_routes_page_owned) | test(popup_no_commit_responses_preserve_initial_document_before_redirect_replacement)' \
  --no-fail-fast --status-level fail --final-status-level fail
```

child stable proxy / realm 基线：

```bash
cargo nextest run -p moli-renderer-v8 -p moli-protocol \
  -E 'test(initial_empty_same_origin_commit_reuses_local_window_exactly_once) | test(same_origin_child_window_migration_to_cross_origin_installs_denied_surface) | test(child_window_proxy_identity_survives_cross_origin_round_trip) | test(per_page_isolate_policy_uses_distinct_isolates_and_isolates_contexts) | test(per_page_isolate_policy_reuses_navigation_isolate_and_replaces_contexts) | test(popup_target_diagnostics_report_distinct_page_vm_document_isolates)' \
  --no-fail-fast --status-level fail --final-status-level fail
```

迁移后第二组中的 per-page-isolate 测试需要被更精确的 script-agent policy 测试替代，
不能简单删除隔离覆盖。

### WPT 优先簇

优先运行本地 Chromium WPT checkout 中这些目录/文件：

- `html/browsers/browsing-the-web/navigating-across-documents/initial-empty-document/window-open-*`
- `html/browsers/browsing-the-web/navigating-across-documents/multiple-globals/`
- `html/browsers/windows/auxiliary-browsing-contexts/`
- `html/browsers/windows/browsing-context-names/`
- `html/browsers/the-window-object/open-close/`
- `html/browsers/the-window-object/window-open-noreferrer.html`
- `html/browsers/origin/cross-origin-objects/`
- iframe sandbox popup cases；
- anchor/area/form `_blank` + opener/noopener/noreferrer cases。

每轮要记录 Moli commit、Chromium/WPT commit、binary build profile、case timeout、
parallelism、case list 和 subtest 详情。focused WPT 不应覆盖仓库 full baseline list。

### 合并前仓库验证

按照仓库约定：

```bash
cargo nextest run --no-fail-fast
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

禁止使用 `cargo test`。若阶段只跑 focused nextest，提交说明必须列出未跑 full suite 的
原因。

## 风险与停止条件

### 最高风险：fresh-by-default / selective-related script-agent policy

`PageVm` / `RendererPageScriptEnvironment` 默认仍为每个 fresh Page 建立 agent；只有显式
related auxiliary admission 才共享 isolate。已有大量
`per_page_isolate_policy_*` 测试保护：

- isolate/context 隔离；
- navigation 替换 context 但复用 Page isolate；
- timer/fetch/module/worker/IndexedDB 的 page-owned routing；
- CDP object id、context id、binding 和 object group 的 page-local scope。

推荐方案不是“删除隔离”，而是把隔离 key 从 `PageId` 提升为 `ScriptAgentId +
Page/Realm owner`：V8 memory heap 可共享，所有可观察路由仍必须精确到 Page/Document/
session。

### reentrancy / borrow 风险

`window.open()` 在 opener V8 callback 内发生，此时 opener `ScriptVm` 正在执行。同步创建
另一个 Context/Page 不能通过重入同一个 owner loop、临时释放借用或全局裸指针实现。
Phase 3 必须在已经验收的 selective agent 基础上确定安全的 `PendingAuxiliaryPage`
构造边界和 ownership transfer。

### inspector 风险

多个 target 共享 isolate 后，inspector backend、execution context id 和 remote object
registry 很容易错误地按 isolate 全局化。任何一个 target 能 evaluate/release 另一个
target 的 object 都是阻断问题。pause loop 是 isolate 级，但 target close、Page output
journal、queued command 和 frontend session 必须按 renderer agent/Page 路由；Phase 3
第一纵切已修正 close/command/session 路由，跨 target remote-object 隔离仍由 Phase 2B
矩阵保护。

### COOP / agent split 风险

V8 object 不能跨 isolate 搬迁。若初始 popup 与 opener 共享 isolate，COOP 导航后的
group/agent split 必须新建 realm并把旧 WindowProxy 切到 disconnected/remote endpoint，
不能尝试移动 context。G1 已用 provisional fresh isolate + commit-time old-proxy disconnect 完成 local
路径，并证明同一 Page scheduler/Target/session 可以跨 agent 保持；G4 已把 closed facade 演化为
group-qualified endpoint generation，并统一 local/disconnected operation currentness；G5 又完成 same-group
cross-agent top-level projection、typed protocol route与 target ACK。剩余风险是让真正 cross-process
RemoteFrame、process death、agent reunification、redirect/report-only 和 policy replication 也消费同一 endpoint
admission，不能绕过或重新推导该事务。

### 明确停止扩大修改

出现以下任一情况时，应停止继续补调用方并重新检查 owner 设计：

- 同一个 popup 修复要复制到 lightweight 和 target Page；
- 为了等 protocol 创建 target，需要 sleep、drain、retry 或无限轮询；
- CLI 成功而 CDP 失败，且两者等待的是不同 Page/loader；
- stale popup navigation/realm completion 能写入新 Page residence；
- shared isolate 只能靠裸指针、泄漏 cache 或仅 debug assertion 保持路由；
- “性能提升”来自跳过 target event、DOM、network 或正确性检查；
- popup 被建模成 child 后开始携带 parent/frameElement/load-blocker 特例。

## 决策记录

本次评估建议记录以下架构决定：

1. auxiliary popup 必须只有一个 authoritative Page/Document/navigation owner。
2. protocol target 绑定 renderer-created Page，不再复制 Page。
3. 采纳 child-frame stable WindowProxy/realm 基础，但先抽成通用 browsing-context
   primitive；不把 popup 当隐藏 child frame。
4. 保留 opener 的 popup 需要独立 realm 和同步 WindowProxy access，因此要引入可承载
   多 Page realm 的 script agent；第一版共享 isolate，后续可演化 remote endpoint。
5. popup 仍是独立 PageVM/CDP target，可在 opener 关闭后存活。
6. `noopener` / COOP 是 group/agent/endpoint policy，不只是 `window.opener = null`。
7. Phase 2B selective shared-agent 实验已经通过；Phase 3 第一纵切只为 renderer 明确
   新建的 auxiliary context 打开 production relationship admission。fresh Page 与
   `noopener` 仍隔离，不恢复 renderer-owner-wide sharing，也不先做大范围 popup 迁移。
8. 最终删除 lightweight popup 专用 loader/parser/script/realm alias，避免长期双栈。
9. COOP swap 在 response preparation 时选择并预留新 group/agent，只有 Document commit 才 sever old group；
   Page scheduler、PageId 与 protocol Target/session 稳定，realm/WindowProxy/inspector output stream 按 agent
   更换。取消或 supersede 不能修改旧 group。
10. top-level WindowProxy endpoint 由 browsing-context group 分配 `(group, generation)`；normal Document
    replacement 保留，COOP group switch 更换。V8 projection 只能携带这个 typed identity，不能保存目标 V8
    global 或 Page residence；所有 routed operation 必须在 owner resolver 中做 exact active/current check。

## 相关文档

- [Child Browsing Context Current Boundary](child-browsing-context-current.md)
- [V8: Isolate vs Context](v8-isolate-vs-context.md)
- [Chromium Context / Lazy WindowProxy / ScriptState](chromium-context-lazy-windowproxy-scriptstate-2026-06-15.md)
- [Popup Target and JavaScript Navigation Lifecycle](popup-target-and-javascript-navigation-lifecycle-2026-07-22.md)
- [CDP Target Engine and Initial Popup Document Case Study](cdp-target-engine-and-initial-popup-document-case-study-2026-05-24.md)
- [CDP Initial Empty Document Chromium Alignment Plan](cdp-initial-empty-document-chromium-alignment-plan-2026-06-18.md)
