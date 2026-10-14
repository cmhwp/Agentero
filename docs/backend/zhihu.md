# 知乎接入 Host

内置「知乎」Agent 的 Host 侧实现：直连知乎开放平台，把结果转成既有 `agent:*` 事件，复用 Agent 面板而不新增前端分支。前端说明见 [../frontend/zhihu.md](../frontend/zhihu.md)。

## 分层

```text
crates/agentero-core/src/features/zhihu/   # tauri 无关：开放平台客户端
src-tauri/src/features/zhihu/              # Host：内置 Agent 运行器 + 探测命令
```

| 模块 | 职责 |
|---|---|
| `agentero-core::features::zhihu::ZhihuClient` | 直答（SSE 流式）、标题检索、信封解析、相关度阈值 |
| `features::zhihu::run_once` | 路由（直答 / `/搜索`）、emit `agent:stream` / `agent:completed` / `agent:failed`、取消 |
| `features::zhihu::emit_warm_events` / `models_event` / `builtin_commands` | 模型目录与斜杠命令（供配置条 / `/` 菜单） |
| `features::zhihu::commands::zhihu_probe` | 设置页连通性探测（一次最小检索） |

## 内置 Agent 的接线

内置 Agent 不是外部 ACP 进程，Host 侧三处专门处理：

1. **注册**：`AgentRegistry::load()` 注入 `template: "zhihu"` 的 descriptor（id `builtin-zhihu`，`available: true`），使 `agent_list_agents` 无需探测即可返回；`probe_or_bundled` 对内置模板直接置为可用。
2. **运行分流**：`service::accept_run_once` 解析出内置模板后不走 `to_acp_agent` / `run_once`，而是注册取消通道后交给 `features::zhihu::run_once`，并提前把 `agent:status(starting)` 发到前端。
3. **旁路空实现**：`agent_warm` 广播模型目录 + 斜杠命令并直接返回 ok（不 spawn）；`agent_probe` 直接报可用；`agent_list_sessions` / `agent_load_session` 返回空结果。远程目标解析对内置 Agent 跳过。

前端因此零改动即可渲染流式直答与工具面板；取消仍走 `agent_cancel_run`（内置运行器 `select!` 监听取消通道并回 `stop_reason: "cancelled"`）。

## 凭证

Access Secret 存 `AppSettings.zhihu_access_secret`（XDG `settings.json`，0600）。读路径：`AppSettingsStore::zhihu_access_secret()`；与其它 BYOK key 共用 redact（`*` 掩码）/ `merge_secrets`（掩码回填）逻辑，因此 WebView 与广播都不含明文。未配置时运行器返回 `zhihuNoSecret` 域错误。

## 端点

| 能力 | 方法 / 路径 | 说明 |
|---|---|---|
| 标题检索 | `GET https://developer.zhihu.com/api/v1/content/zhihu_search` | `Query` / `Count`（≤10）；信封 `{Code,Message,Data}`，`Code != 0` 抛错 |
| 直答 | `POST https://developer.zhihu.com/v1/chat/completions` | OpenAI 兼容；`stream: true`，逐块解析 `data:` SSE，`[DONE]` 结束 |

鉴权：`Authorization: Bearer <secret>` + 秒级 `X-Request-Timestamp`。

## 限制与后续

- 直答为单轮，多轮上下文靠前端 transcript 回放。
- 精确论文↔讨论索引（别名、ORCID 绑定、作者优先排序）未接入，需知乎侧服务端接口；`/搜索` 目前是标题/关键词检索兜底。
- 额度由开放平台按能力组限制；错误消息原样透传（30001 频率 / 30002 配额等）。
