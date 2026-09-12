# 拖放 InputEvent 收尾

接续 `bfdf315f1` 的普通文本编辑事件修复。这次只补既有拖放编辑链路，
不改 Windows 默认身份、同步布局策略或 WebGL，也不宣称新增一个通过的检测站点。

## 实测差异

对照 Debian `/usr/bin/chromium` 145.0.7632.116、xvfb headed 模式，
通过真实 CDP `Input.dispatchDragEvent` 的 dragEnter → dragOver → drop。
使用本地空白页面，不访问真实账号。探针不取消 dragover，让浏览器执行
原生编辑；取消 beforeinput 的独立用例仍验证编辑被阻止。

| 目标 | beforeinput | input |
| --- | --- | --- |
| input / textarea | data="hello"，dataTransfer=null | 同左 |
| contenteditable=true，纯文本或 HTML | data=null，附带只读 DataTransfer | 与 beforeinput 共用同一个 DataTransfer |
| contenteditable=plaintext-only | data="hello"，dataTransfer=null | data=null，附带 DataTransfer；不插入 HTML 节点 |

这些 beforeinput/input 都是可信 InputEvent，bubbles/composed=true，
isComposing=false，只有 beforeinput 可取消。编辑事件的 transfer 与
drop 事件的 transfer 不是同一个对象，编辑事件在派发后仍能读取数据。

旧 Moli 的富文本 drop 使用普通 Event，缺少 data/inputType/isComposing，
InputEvent 构造器本身也缺少 dataTransfer。此外，缺少 selection 时的成功
append 路径会直接 return，漏掉后续 input。

## 实现

- 原生 InputEvent helper 增加 transfer 载荷，与字符串载荷共用构造流程。
  继续使用保存的 intrinsic constructor，不调用页面替换的 InputEvent。
- InputEventInit 增加 nullable DataTransfer 的品牌校验；与 DragEvent
  共用读取规则，拒绝普通对象和伪造 prototype 的对象，保留 getter 异常。
- 编辑前复制 DataTransfer 的原生 item 状态，冻结其写权限。
  setData/clearData/items.clear 不修改，items.add 返回 null，items.remove
  抛 InvalidStateError；effectAllowed 不可更改。beforeinput/input 共用此副本。
- item store 的统一写入边界检查权限。普通 new DataTransfer 和既有拖拽
  对象仍保留原有可写行为，不在这次顺带改变整个拖拽访问权限状态机。
- 插入成功后统一派发 input，取消 beforeinput 时不执行插入。
  plaintext-only 的输入载荷和禁止插入 HTML 按实测行为处理。

副本会分配新的 item 包装和字符串；File/目录 entry 保留引用，不复制文件
字节。这样原拖拽列表后续变动不会破坏编辑事件的数据，但不是完整实现
Blink 的 DataObject/访问权限生命周期模型。

## 可执行验证

- `groups/drop_input.py` 已接入默认 dom-input smoke。六种场景覆盖文本控件、
  富文本、plaintext-only、取消、载荷、只读操作、对象身份和派发后读取。
  同一函数已在 xvfb Chromium 和本次 Moli debug 公开 CDP 服务通过。
- 原有 keypress 和 native text InputEvent smoke 在两端同样通过。
- Rust 的 `input/tests/drop_events.rs` 覆盖完整协议到原生编辑链路；
  renderer 测试补充 nullable 转换、保留 getter 异常，以及复制原生 item
  不调用页面 getter、源列表清空后文本/File 仍可读取。
- Python smoke 项目 46 项单元测试通过。

本轮原始探针和环境信息保存在
`/tmp/moli-drop-input-20260912.zWqtH6fo/`：
`chromium-native`、`chromium-plaintext`、`moli-before` 为差异探针，
`chromium-smoke`、`moli-smoke` 为相同 E2E 函数的结果。
临时目录不作为长期测试依赖；可重复执行的断言在仓库 smoke 和 Rust 测试内。

## 提交前门禁

保持 HEAD `bfdf315f1` 不变，从仓库根目录完成：

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
NEXTEST_TEST_THREADS=16 cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
```

Clippy 最终通过；nextest **17545/17545 通过**，耗时 146.470 秒，
13 项为仓库原有 skip。未增加重试、放宽断言或延长测试超时。
使用已有独立 Cargo target cache，未在构建或测试期间提交代码。
后续只补本文验证记录，未再修改实现和测试代码。

nextest run ID：`86d4c42b-b7d8-4e5d-aba7-0ce985a3bd7c`。
本地日志：`docs/inner/drop-input-clippy-final-20260912.log`、
`docs/inner/drop-input-nextest-20260912.log`。日志不随提交发布。

## 未扩大范围

没有补全 IME、getTargetRanges、富文本编辑器所有算法和所有 InputEvent
构造器属性描述符。也没有补齐 DragEvent 的完整 protected/read-only/neutered
生命周期；例如 Chromium 派发结束后清空 drop transfer 的访问能力，Moli
既有拖拽对象仍保留可访问状态。这与此次长期可读的编辑事件副本是两件事。

Fingerprint Pro 的真实 Windows 基线仍需要 Windows 机器或已授权端点；
本机 Linux Chromium 不能替代该证据。DeviceAndBrowser 交互页的上轮
共同 suspiciousClientSideBehavior 判定仍见独立取证报告，不能把本次本地
API 回归通过解释成该网站已转绿。
