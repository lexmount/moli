# Text Nextest 重试证据审计

固定 HEAD `ca2a8d39572644de1211e3aba30d239abf0ffb17`，[job 106628518773](https://github.com/lexmount/moli/actions/runs/35691182688/job/106628518773)，Nextest 0.9.132，内部 run UUID `95ea08da-b9ed-4545-bf7d-8c9771dfec00`。

结论：这次确实有一个测试失败后重试成功，但已检查的 GitHub 记录没有测试名称或首次失败内容，无法从这些记录恢复。没有证据能把它归因为环境、text 修改或日志列出的慢测试。

## 已核查的记录

- 重新下载 job 原始日志，共 85,511 字节，SHA256 `378672840e029ebb055f3023cd8d63193d8c6a898d09414934523f179ea84eae`，与已有日志逐字节相同。不是现有文件截断或只读摘要造成的信息缺失。
- 原始日志第 974–990 行：启动 18,458 测试、139 binaries；仅列出 13 个慢测试，随后汇总 18,458 passed（13 slow、1 flaky）、13 skipped。无 RETRY、失败断言、panic 或重试测试名。慢测试名不能推断为 flaky 测试。
- Check output 的 title、summary、text 均为 null。唯一 annotation 是 Node.js 20 弃用警告，与测试失败无关。
- run 产物 API 返回四项：`moli-release-head`、`moli-release-base`、`cdp-smoke-diagnostics`、`webdriver-smoke-diagnostics`。精确 HEAD 的 workflow 可确认它们来自其他 jobs，只打包 release 或对应 smoke 目录。Nextest job 没有 upload/report/summary 步骤，没有 JUnit 配置或显式 recording 配置。
- job 已完成且执行了容器清理。不是尚未上传的 Nextest 报告。

## 信息缺失的原因

精确 HEAD 的 `.config/nextest.toml:1–6` 配置 `retries=2`、`status-level="fail"`、`final-status-level="fail"`、`success-output="never"`。状态等级 `fail` 不展示 `retry` 等级的恢复测试；因此整体 SUCCESS 与 1 flaky 可以同时成立。这里确定的是报告可观测性缺失，不是该测试首次失败的运行根因。[Nextest 状态等级说明](https://nexte.st/docs/reporting/#status-levels)

## 最低成本恢复路径

唯一值得先做的零测试执行恢复，是在本次 self-hosted runner 的保留目录中查**该 UUID 的录制数据**。Nextest 支持保存完整尝试及输出，但须提前启用；本次命令、仓库配置、环境日志没有启用证据。可先只读检查 runner 为 job 挂载的临时 HOME 中 `~/.config/nextest/config.toml`、`~/.local/state/nextest` / `~/.cache/nextest`（不同版本目录），以及同工作区可能保留的 `target/nextest` 报告。若精确 UUID 记录存在，用该版本 `cargo nextest replay --help` 确认参数后，仅 replay 并打开 retry 状态及 failure 输出；无需重跑测试。[录制需预先开启](https://nexte.st/docs/features/record-replay-rerun/)

该路径是条件性恢复方案，不是已经存在记录的声明。任务期间没有 runner 主机访问，也没有修改 CI 去取得访问。如果没有提前录制或另存输出，现存 GitHub 数据不能逆推出首次失败；重跑即使再次失败，也属于新证据，不能冒认恢复了这次失败。

后续正常测试运行的最小可观测性修正，是命令增加 `--status-level retry --final-status-level retry --failure-output immediate-final`；这保留原重试语义，只显露恢复名称和失败尝试输出。本审计未应用该修正，未重跑 18k 测试，未修改分支、CI 设置或源码。
