# DeviceAndBrowser 交互页：请求生命周期与两端事件数据

## 结论

本轮完成 Moli release 与 xvfb Chromium 的同动作对照。两端都发出最终
POST、收到 HTTP 200、读取到 JSON，且页面展示同一判定。
**两端仍然都命中 `suspiciousClientSideBehavior`，不能把这个共同结果
归因为 Moli 漏发事件。** InputEvent 修复是独立的兼容性改进，没有让本站转绿。

之前 Chromium 的空结果已经定位到更具体的阶段：最终 POST 在记录中存在，
但原观察窗口没有记录到该请求的响应头或失败事件。不是“没有点到提交”，
也没有证据说明“响应已收到，但页面没有显示”。仅凭旧日志仍不能区分
链路、服务端等待等进一步原因；本轮成功不能倒填成旧轮通过。

## 取证方法

入口：`moli_cdp_smoke.diagnostics.device_browser_behavior`。
使用网站允许的 demo 表单和虚构凭据，不访问真实账号。

- 新进程、新 context；Runtime/Page/Network 开启，无 UA override、无代理，TLS 验证保留。
- 与原 12 站动作相同：DOMContentLoaded 后等 20 秒，点击邮箱、固定 80 ms
  节奏输入、点击密码并输入、点击 Login。没有“拟人化”或改变事件内容。
- 被动观察 `POST https://deviceandbrowserinfo.com/fingerprint_bot_test` 的
  request、response headers、loadingFinished/loadingFailed，以及对应的 JSON。
- 不将 `fingerprint-scan.com` 的辅助探测回包当成本站最终判定。
- 提交后 20 秒先冻结 baseline，再读取 `#jsonResult`；另等 40 秒记录 extended。
  deep-copy 的 baseline 不会被迟到的事件修改。每次 DOM 读取时间另外记录。
- 仅保留 interactions 中的数值/布尔字段、timing 摘要和服务端布尔判定。
  原生函数 stack 只记录数量，不保存原始字符串；不保存账号、IP、token。

当前 Moli 是 `2264ea6db` 加本次 InputEvent 补丁的 release，SHA-256 为
`e4f3d498a836436f0325bb42779688a805c23726456be8e9c695ea62bd2ba382`。
Chromium 为 `/usr/bin/chromium` 145.0.7632.116，xvfb headed。
这是一次同动作诊断，不是 12 站整套重跑；其他会话有构建/测试负载，
不能把下面毫秒级差别解读为隔离环境下的性能回归。

## 本轮完整链路

以下时间是 collector 起始后的秒数。

| 观察 | Moli | Chromium |
| --- | --- | --- |
| 最终 POST | 26.754 | 26.919 |
| 收到响应头 | 27.991，HTTP 200 | 27.821，HTTP 200 |
| loadingFinished | 27.992 | 27.821 |
| baseline cutoff | 46.493 | 46.641 |
| baseline DOM 展示 | isBot=true | isBot=true |
| extended cutoff | 86.540 | 86.688 |
| extended DOM 展示 | 未改变 | 未改变 |

两端 API JSON 与 DOM 中的判定一致。Moli 只有 behavior 项为 true；
Chromium 另外命中 `hasInconsistentTimingResolution`、`isAutomatedWithCDP`、
`isAutomatedWithCDPInWebWorker`。这只描述本轮实际结果，不承诺所有 Chromium
版本或负载条件都会命中这些项。

## 网站实际收到的交互字段

| 字段 | Moli | Chromium |
| --- | --- | --- |
| 邮箱 / 密码 numKeys | 25 / 20 | 25 / 20 |
| typeAtCharacter | true | true |
| numSuspiciousKeyEventsEmail / Password | 0 / 0 | 0 / 0 |
| hasUntrustedEvent / cdpMouseLeak | false / false | false / false |
| 邮箱、密码、提交按钮有 click | 全部 true | 全部 true |
| 三次点击都在精确中心 | 全部 true | 全部 true |
| nativeFunctionsStackTraceCount | 5 | 5 |
| 邮箱平均键间隔 / 标准差，ms | 84.329 / 0.527 | 86.367 / 0.912 |
| 密码平均键间隔 / 标准差，ms | 84.105 / 0.398 | 86.111 / 1.090 |
| timeToFill，ms | 3975.3 | 3962.9 |

这些统计与脚本使用固定节奏、locator 默认中心点击的事实一致。没有黑盒
服务端的单变量证据，不能声称其中某个字段就是触发 `suspiciousClientSideBehavior`
的唯一规则；更不能据此改 renderer 去隐藏正常产生的事件或篡改统计。

## 旧轮为什么不能算通过

旧记录位于
`cdp-survey-2264ea6db-20260912.7fKqHCAJ/sites-chromium/device-browser-behavior/`。

- 26.290 秒已经记录最终 POST，`interaction.submitted=true`。
- 原 verdict cutoff 为 46.031 秒；该请求只有 requestWillBeSent，没有
  responseReceived 或 loadingFailed。旧采集器没有订阅 loadingFinished。
- 页面 `#jsonResult` 仍为空，保存的辅助服务 JSON 不能代替最终判定。

所以旧轮应继续标记为“观察窗口内无有效 verdict”。当前 collector 增加
终态和独立扩展窗口，正是为了以后能区分请求未发、等待头部、等待 body、
网络失败、JSON 解析失败和页面显示问题，而不是统一记为“没通过”。

## 验证与产物

- Moli：`docs/inner/dab-behavior-moli-20260912-input-event/`。
- Chromium：`docs/inner/dab-behavior-chromium-20260912-input-event/`。
- 每端一轮，均为 baseline 与 extended 同进程连续观察，无重试取绿。
- 五个专项测试覆盖请求阶段分类、数据白名单、合法布尔判定、迟到响应
  不得修改 baseline、辅助探测不能冒充最终 verdict。

本提交是诊断和记录，不更改产品交互节奏、浏览器身份、WebGL 或冻结布局策略。
