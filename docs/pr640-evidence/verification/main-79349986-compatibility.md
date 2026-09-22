# main@79349986 与七分区的合入兼容性

结论：只读源码审查及隔离对象库虚拟合并均未发现合入冲突或拆分内容丢失。新 main 的 11 个文件与七个 PR 的实际增量文件没有交集。该结论是源码/补丁兼容性结论，未运行新组合的编译或测试，既有二进制结果仍只对应原精确 HEAD。

## 固定对象与全树证明

| 对象 | SHA |
|---|---|
| 原 main | `2793b2407fc7805afcb531e94fc02da4ac7bf33d` |
| 新 main | `793499865736c5e91eb5dc52ee501beea8db0912` |
| 原组合 commit | `ed844e7cf8c2f1f7d79a6215936c6055b1eb3b65` |
| 原组合 tree | `6fe6dd8f43edca8585379cd23b957057d902f0f1` |
| 新 main 虚拟合并 tree | `98be4e73eda6286445ea57d462edd9828cc3e53f` |

自然 merge-base 已核对为原 main 2793。`git merge-tree --write-tree` exit 0，无文本冲突。Git 写入仅发生在单独临时 `GIT_OBJECT_DIRECTORY`，通过 alternates 只读主仓库和 combined-final-source 对象；未创建/移动任何现有 ref，未改 worktree。

- 原组合→虚拟合并的文件集合恰为新 main 的 11 个文件；这些文件的最终 blob 全部逐个等于新 main 对应 blob。
- 上游两提交增量与合并新增量的 stable patch-id 均为 `0153c50a8b9ac14aa900b07b51f5740d13231640`。
- 原 main→原组合，以及新 main→虚拟合并的 split stable patch-id 均为 `df007ca6857a37e9ab208640ff10fe916ccd79ab`。
- 七个独立 PR 分别与上游 11 文件求交，结果全部为空。
- 合并树中旧 API `prepare_xhr_send_body_from_args` 引用为零；导出、Window 和 Worker 调用侧共同切换到新 API。

逐文件 blob、全部路径及隔离对象目录见 `main-79349986-compatibility.json`。无需改造 split 源码解决本次合入。

## 上游实际语义变化与关联路径

新增提交：`d63b4e7e5`（XHR send body先转换再状态检查）、`793499865`（Document识别为native union分支）。

- `network_host/xhr/send/request.rs:115–137,201–237`：把WebIDL参数转换与方法决定的正文提取分开。普通值先转USVString；native Document/Blob/FormData/URLSearchParams/BufferSource保持native分类；GET/HEAD在prepare阶段不提取正文；shared/resizable backing store显式拒绝。
- `network_host/xhr/send.rs:42–76`：Window先转换，再验证send状态，之后读取实际method并prepare；因此用户toString中的重入open/send会影响最终请求状态，而非使用转换前缓存method。Worker在 `worker/global_scope/xhr.rs:247–274` 做同样处理。
- `network_host/xhr/bindings/prototype.rs:13`：send声明增加原生XMLHttpRequest receiver检查，避免非法receiver先执行body转换。

**网络分区：** 上游影响XHR请求发出之前的参数转换和异常顺序；split网络记录、redirect逐跳规则、有界响应体及durable session所有权均完整保留。真实HTTP/CDP旧测试仍是原db091二进制证据，不能自动称为此新组合验证。

**文本分区：** MIME响应选择、decoder、共享plaintext parser、子文档snapshot/live初始化未改。Document作为send参数的native分支与“将响应作为text/plain/JSON解析”处于不同边界。不要把本次native union识别描述成新增完整Document序列化能力；POST最终body.prepare仍进入既有提取路径。

**输入分区：** 点击/键盘/默认激活路由没有更改。事件处理器若调用XHR.send，会观察到新的WebIDL转换/重入顺序，这是上游刻意改变的语义；没有分区重复实现或函数接口交叉修改。

**停止加载：** 取消所有者、parser退役和load gate修改逐patch保留。上游在send前失败时不会进入发请求的路径；本次diff未重排已有传输取消实现。未据此声称重入取消场景已实测。

## 后续验证边界

没有运行cargo、浏览器任务或新组合smoke，也没有重建任何二进制。若实际将这两提交纳入待发布/待合入分支，必须在新精确HEAD满足仓库Rust门禁；原HEAD的CI和功能验收不能换标签。

上游随代码新增 `window_xhr_send_body_converts_union_before_state_and_method`、`worker_xhr_send_body_converts_union_before_state_and_method`、`xhr_send_body_reentrant_conversion_controls_request` 及共享JS fixture；它们覆盖转换先后和重入请求。新组合验证应保留这些检查，并复用原组合的文本DOM/网络原始正文/停止后新导航smoke，检查相邻边界；本报告不宣称这些新组合检查已经运行。
