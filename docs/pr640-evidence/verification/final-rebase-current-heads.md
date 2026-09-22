# PR #640 拆分后的当前提交与验收边界

七个 PR 的当前提交均基于 main `a54ba82a7ac6462429d2f60cd0dbd0d67c589e84` 或其直接父 PR。每个分区只重放自身提交；七个分区的 `range-diff` 全部等价，稳定 patch-id、修改文件集合及提交顺序均保持，作者和提交者均为关联 GitHub 账号 `lanyue-llk` 的邮箱。文本分区保留原有三项提交，其余各一项。当前分支与原已测提交的逐项哈希见[机器可核对清单](final-rebased-prs.json)。

| PR | 职责 | 当前 HEAD | 直接父 PR | 已测 HEAD 归档 |
|---|---|---|---|---|
| [#734](https://github.com/lexmount/moli/pull/734) | 工程与验证基础 | `746e376f0179` | main | `codex/pr640-tested-20260922-engineering` |
| [#735](https://github.com/lexmount/moli/pull/735) | 布局 | `62c07ae866e8` | #734 | `codex/pr640-tested-20260922-layout` |
| [#736](https://github.com/lexmount/moli/pull/736) | 输入 | `9818501903e1` | #735 | `codex/pr640-tested-20260922-input` |
| [#737](https://github.com/lexmount/moli/pull/737) | 网络 | `15faebd1683b` | #735 | `codex/pr640-tested-20260922-network` |
| [#738](https://github.com/lexmount/moli/pull/738) | 停止加载 | `e9c3b70ceb73` | #734 | `codex/pr640-tested-20260922-stop` |
| [#739](https://github.com/lexmount/moli/pull/739) | 表单命名属性 | `4e85980f809e` | #734 | `codex/pr640-tested-20260922-forms` |
| [#740](https://github.com/lexmount/moli/pull/740) | 文本解析 | `c6d78aeb4289` | #734 | `codex/pr640-tested-20260922-text` |

原 #640 与历史 PR/分支仍保留；[七个已测 HEAD 的远端归档清单](final-tested-head-archives.json)也独立保留。建议合入顺序是 #734 → #735 → #736 → #737 → #738 → #739 → #740；实际依赖为 #734 → #735 → {#736、#737}，以及 #734 → {#738、#739、#740}。

[五图审查指南](../../pr640-review/reviewer-guide.md)与[35 题逐项验收总表](final-declared-acceptance.json)记录了**已归档精确二进制**的真实任务结果：布局 v3 的 26 题加 v4 单独 547、输入原合同 4 题、网络显式就绪合同 3 题、文本原合同 1 题。其余三个 PR 有独立专项测试。新 main 添加了 XHR send body 和 CSSOM 修复；布局分区与新增 CSSOM 测试位于同一文件，但分区补丁自动合并且 `range-diff` 等价。源码补丁等价不等于新二进制已重新执行 35 题；旧 receipt 均标为旧 HEAD，不挪用到新 HEAD。当前 HEAD 的 CI 由各 PR 单独显示。

PR 合入采用仓库允许的线性方式。合入父 PR 后，子 PR 仍只应携带其自身直接父提交之后的增量；如果 GitHub squash 改写了父提交 SHA，合入者需按该明确边界重放子 PR，再核对 diff 与 CI。原已测分支与证据不因合入而删除。
