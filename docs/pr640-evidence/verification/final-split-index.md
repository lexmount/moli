# PR #640 分区与验收对应

功能验收范围是 1.1.9 的 34 个失败任务，加上 1.1.8 已失败的 386，共 35 题。历史 11 类标签保持不变；类别描述失败现象，PR 按共享实现责任组织，两者不要求一一对应。历史 67 的传输超时单独保留，不认领为功能修复。

所有分区基于 main `2793b2407fc7805afcb531e94fc02da4ac7bf33d`。输入与网络依赖布局，布局、文本、停止加载与表单依赖工程前置。

| 分区 | PR | 独立验收任务 | HEAD |
|---|---|---|---|
| 工程可靠性 | [#734](https://github.com/lexmount/moli/pull/734) | 专项功能契约，无历史任务认领 | `2e9c2852` |
| 布局几何与失效 | [#735](https://github.com/lexmount/moli/pull/735) | 29, 66, 108, 156, 163, 196, 314, 349, 374, 389, 399, 404, 415, 418, 441, 448, 506, 547, 576, 585, 590, 620, 640, 645, 658, 666, 714 | `b0f237b0` |
| 输入激活与导航 | [#736](https://github.com/lexmount/moli/pull/736) | 274, 386, 465, 731 | `fa1c36ef` |
| 网络记录与响应证据 | [#737](https://github.com/lexmount/moli/pull/737) | 339, 595, 676 | `db091f69` |
| 文本响应解析 | [#740](https://github.com/lexmount/moli/pull/740) | 784 | `ca2a8d39` |
| 停止加载 | [#738](https://github.com/lexmount/moli/pull/738) | 专项功能契约，无历史任务认领 | `b43a213d` |
| 表单命名属性 | [#739](https://github.com/lexmount/moli/pull/739) | 专项功能契约，无历史任务认领 | `7919ed09` |

原始 [#640](https://github.com/lexmount/moli/pull/640) 与所有原分支保留。旧拆分 PR 的讨论通过替代关系可追溯，未改写原远端提交。新的 7 个 PR 作者及全部新增提交的 author/committer 均已通过 GitHub API 核验为 `lanyue-llk`，见 [账号关联](final-pr-github-attribution.json)。

595 在旧布局版本 `48d78ac6` 中完成交互但因网络记录缺少 Referer 而失败，因此由包含布局前置的网络分区承担最终验收；旧结果仍保留为 28 题中 27 通过、1 失败。详见 [595 证据](case-595-dependency.json)。

[逐题历史分类及归属](historical-failure-scope-v2.json)覆盖全部 35 题；[原 11 类](historical-11-categories.csv)保留 1.1.8 与 1.1.9 的差异。每个最终 PR 的二进制只执行其对应任务，动作、评分和输入文件保持冻结；此前整体 90 题结果不能替代分区验收。当前逐题运行结果以各 PR 发布的精确 HEAD、二进制哈希及审计文件为准。
