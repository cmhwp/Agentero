# 知乎 Agent

内置「知乎」Agent：出现在右侧栏 Agent 面板的切换器里，无需安装任何 CLI。它把**知乎直答**与**按标题检索**两项能力接进对话面板，随论文阅读随时调用。

Host 契约见 [../backend/zhihu.md](../backend/zhihu.md)；完整设计（含未来的 CLI 后端与广场面板）见 [../development/zhihu.md](../development/zhihu.md)。

## 形态

内置 Agent，不是外部 ACP 进程：

- 注册表在启动时注入一条 `template: "zhihu"` 的 descriptor（`built-in-zhihu`），`available: true`，因此在切换器里无需探测/安装即可见。
- 选择后在右栏 Agent 面板正常对话：流式直答、`/搜索` 斜杠命令、模型配置条、历史项行为都与外部 Agent 一致。
- 它是 **chat-only**：不读 Vault、不跑 ACP、不做 `session/list`。因此它不出现在「翻译 / PDF Ask」的 Agent 选择器里，也不会把当前论文路径注入提示词（避免把本地路径发给第三方服务）。

## 能力

| 输入 | 行为 |
|---|---|
| 纯文本 | 知乎直答（`/v1/chat/completions`，流式），增量渲染为普通 Agent 消息 |
| `/搜索 <关键词>` | 按标题检索知乎，结果以 Markdown 链接列表返回（`/search` 同样可用） |

检索结果取开放平台 `RankingScore` 相关度 ≥ 0.5 的条目，按平台返回顺序展示，含作者与赞同数。

## 凭证

Access Secret 在 **设置 → Agent → 知乎** 配置：

- 输入框是 write-only：保存后不再回显明文，只显示可用状态点（未配置 / 检测中 / 可用 / 不可用）。
- 存储走 Host `settings.json`（0600），IPC 上始终是 `*` 掩码；前端从不持有明文。
- 保存后自动用一次最小检索探测连通性（消耗一次开放平台额度）；清除按钮删除本机凭证。
- 未配置时发送会返回可读错误并提示到设置页配置。

Secret 获取：登录 <https://developer.zhihu.com> → profile 生成。

## 代码

- UI：`src/components/settings/panes/agent/zhihu-secret-block.tsx`（凭证卡片）
- 前端库：`src/lib/zhihu.ts`（探测 / 掩码辅助）
- Agent 标识：`src/lib/agent/api.ts` 的 `AgentTemplate` / `isChatOnlyAgentTemplate`；`src/components/agent/agent-logo.tsx`

## 限制与后续

- 目前只做「直答 + 标题检索」；论文↔讨论的**精确索引**（预印本/正式版别名、作者 ORCID 绑定、作者参与优先排序）尚未接入，等待知乎侧提供对应服务端接口。
- 直答为单轮；多轮上下文依赖前端本地 transcript 回放（`buildLocalTranscriptPrompt`），非服务端会话。
- `/搜索` 的关键词需用户显式给出；暂未自动取当前论文标题。
