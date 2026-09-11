# Fingerprint Pro：Windows 身份对照的进展与阻塞

## 本次结论

本次没有修改产品默认身份，也没有将 Linux UA override 当成 Windows
浏览器。**真正的 Windows Chrome 对照仍未完成**：当前会话只有 Linux
主机，尚未获得可用的 Windows 机器或已授权端点。没有据此猜测、修改
Navigator、字体或商业站点的结果。

已新增独立采集入口
`moli_cdp_smoke.diagnostics.fingerprint_identity`，Windows 上的运行方法见
`moli-cdp-smoke/DIAGNOSTICS.md`。`--require-windows` 会拒绝 Linux/macOS；
输出保留宿主 OS、浏览器版本、可执行文件 SHA-256、原生 Navigator/UA-CH
和服务端结果的白名单字段。没有伪装 OS 或读取用户现有 Chrome profile。

## 2026-09-12 的同机补充采样

二进制由 `2264ea6db` 加本次 InputEvent 改动构建，Moli release SHA-256：
`e4f3d498a836436f0325bb42779688a805c23726456be8e9c695ea62bd2ba382`。
对照为 `/usr/bin/chromium` 145.0.7632.116，xvfb headed，非 headless。
两端均新进程、新 context，CDP Runtime/Page/Network 开启，无 UA override，
无代理，保留 TLS 校验。DOMContentLoaded 后观察 12 秒，然后才读取 identity。

| 项目 | Moli 默认 Windows identity，Linux 后端 | Chromium 原生 Linux |
| --- | --- | --- |
| 有效结果 | HTTP 200，约 4.70 秒收到 | HTTP 200，约 3.00 秒收到 |
| bot | not_detected | not_detected |
| tampering / suspect_score | true / 22 | false / 12 |
| tampering_ml_score | 0.9998 | 0 |
| 七项 font_preferences | 149.3125、149.3125、144.015625、133.0625、149.3125、9.34375、162 | 全部相同 |
| touch_event / touch_start / max_touch_points | false / false / 0 | 相同 |
| fonts | 空列表 | 空列表 |

字段顺序为 default、serif、sans、mono、apple、min、system。这里的 fonts
是本次 Fingerprint Pro 的输出，不是之前 CreepJS 的字体数量。

仍存在的观察差别包括平台、语言、screen、hardwareConcurrency、audio 和
math 的结果；这些不是已经证实的触发原因。尤其两端的 V8 版本也不同，不能
将任何一个 hash 的差别直接解释成 Windows 身份矛盾。其他会话当时仍有
Cargo 构建，环境文件保留 load average；本轮不是隔离负载的 timing 实验。

历史 OS-only 对照见 `cdp-fingerprint-identity-status-2026-09-10.md`：
改变 OS identity 这一组能够改变判定，但并没有定位商业模型的单一规则。
本轮字体偏好与触摸字段已经一致而 verdict 仍不同，再次说明不能继续把
“调整这七项宽度”当作已有证据支持的修复。

## 下一步需要的证据

1. 在真正 Windows Chrome 上运行采集器，记录 Windows/Chrome 版本和字体环境。
2. 比较原生 UA-CH 与站点实际收到的 raw attributes，先列可复现差异。
3. 对具体 Web API 用最小离线探针验证；只修确定的兼容性问题。
4. 保持 Moli 的 Windows 默认身份，不覆盖站点检测脚本、请求或结果。

Windows 启动路径尚未做真实执行验证。Linux 下通过的 guard、字段筛选单测
不能替代这一步，也不能声称本任务已取得 Windows 基线。

## 采样与工具验证记录

- `docs/inner/fp-native-moli-20260912-direct/`：本次 Moli 有效结果。
- `docs/inner/fp-native-chromium-20260912/`：本次 Chromium 有效结果。
- `docs/inner/fp-native-moli-20260912-input-event/`：首次工具启动受驱动 HTTP
  discovery 的代理环境影响，连接失败，**尚未导航到站点**；保留为工具失败，
  不计为网站样本。修正为使用已直连发现的 WebSocket URL 后才执行上述采样。
- 采集器白名单、API URL 筛选、真实 Windows guard 的三个专项单元测试通过。

原始目录仅留本地。提交的文档不包含 visitor ID、IP、cookies、SDK token
或未筛选的站点响应。
