# 原生文本编辑 InputEvent 补齐

## 改动

本次针对已有的非 IME 文本编辑链路，不改变输入节奏或网站统计。

- 共用原生 `InputEvent` 构造器。文本输入携带 `data` 和 `insertText`；
  textarea Enter 使用 `insertLineBreak`；Backspace/Delete 分别使用
  `deleteContentBackward` / `deleteContentForward`。换行和删除的 data 为 null。
- 非合成的 beforeinput/input 为可信 InputEvent，继承 UIEvent；两者
  bubbles/composed=true，只有 beforeinput 可取消，isComposing=false。
- 使用保存的 intrinsic constructor，不调用页面替换的 `window.InputEvent`。
- 删除操作在 beforeinput 之后才扩展删除范围。取消事件不会先修改光标/选区。
- 文本控件拖入文本的已有替换链路携带 insertFromDrop；execCommand 的
  既有文本插入/删除事件也使用对应类型。
- checkbox、select 等非文本控件事件和 blur 后的 change 仍为普通 Event。

边界：这不是完整编辑器实现。没有增加 IME、富文本拖放的 DataTransfer、
getTargetRanges、contenteditable 的完整删除/段落算法。富文本 drop 的
既有独立链路尚未迁入本 helper，不声称所有 beforeinput 入口已完全一致。
没有改变同步布局冻结策略、Windows 默认身份或 WebGL。

## 可执行验证

`moli-protocol/src/domains/input/tests/keyboard_events.rs` 的新测试覆盖：

1. input、textarea、contenteditable 的 keyDown/char/Input.insertText 元数据。
2. input、textarea 双向删除的值、方向和 null data。
3. 取消 beforeinput 后值和光标保持不变。
4. textarea Enter 的换行类型与 null data。
5. 页面覆盖 InputEvent 构造器不影响原生事件；change 不变成 InputEvent。

扩充的 `dom-input` CDP smoke 已在真实 Chromium 145.0.7632.116（xvfb）
和本次 Moli debug、release 二进制上通过。测试使用真实 Input.* 命令和
Playwright keyboard，未仅验证 JS 自行构造的 InputEvent。

本地证据目录：

- `docs/inner/input-event-chromium-extended-20260912/`
- `docs/inner/input-event-moli-debug-20260912/`
- `docs/inner/input-event-moli-release-20260912/`

## 门禁完整记录

`cargo fmt --all`、全 workspace/all-targets/all-features Clippy
（`-D warnings`）通过；额外 `cargo fmt --all --check` 通过。

不隐去提交前的失败运行：

| 全量运行 | 结果 | 失败情况 |
| --- | --- | --- |
| 首次 | 17537/17541 通过 | 磁盘耗尽使三个 WPT 写报告失败；另有 realtime audio 赋值 880 后读回 440 |
| 恢复空间后 | 17540/17541 通过 | 既有 follows_post_parse_timeout_location_assign_during_page_load 的 2 秒等待未到目标页 |
| 首次限制并发 | 17540/17541 通过 | 编译期间提交取证文档，使 version 测试的编译期 HEAD 与运行期 HEAD 不同；属于操作问题，不是产品修复 |
| 固定 HEAD，最终验证 | **17541/17541 通过** | 147.880 秒，13 项为仓库原有 skip |

没有修改这两个无关测试/生产实现。音频单测有界 stress 10/10 通过；
所在 eval 模块 5/5 轮、每轮 12 项通过；导航单测 stress 10/10 通过。
这些不证明间歇问题已修复。并发构建/测试负载下的导航 deadline 和实时
音频共享值仍是已记录风险，不用窄测试通过掩盖全量失败。

最终保持 HEAD `5c53546e1` 不动，重新按顺序完成以下验证（从仓库根目录，
Cargo 使用独立的可重建 target cache）：

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
NEXTEST_TEST_THREADS=16 cargo nextest run --no-fail-fast --status-level fail --final-status-level fail
```

使用同一 workspace 和断言，只限制并发；没有跳过用例、加重试、增加超时
或放宽断言。最终 nextest run ID 为 `27d134e7-b626-4160-89d6-0ada2fc06273`。
日志为 `docs/inner/input-event-clippy-final-20260912.log` 和
`docs/inner/input-event-nextest-final-stable-head-20260912.log`。
Python smoke 项目全部 46 项单元测试也通过。

## 网站收益边界

DeviceAndBrowser 的完整对照见 `device-browser-behavior-2026-09-12.md`。
两端均正常提交且收到最终 verdict；邮箱/密码 numKeys 均为 25/20，
仍共同命中 suspiciousClientSideBehavior。本改动修复 API 兼容性，
不能算作新增一个通过的检测站点。
