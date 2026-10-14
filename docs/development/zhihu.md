# 知乎开放平台集成 — 设计稿

> 状态：**部分落地（2026-10-14 起）**。已实现「内置知乎 Agent + 设置页凭证 + 直答 / 标题检索」一条薄切片，见 [../frontend/zhihu.md](../frontend/zhihu.md)、[../backend/zhihu.md](../backend/zhihu.md)。本文档其余部分（CLI 后端、广场面板、划词插引用）仍为设计稿。
> **实现偏离 D2/D4**：首版走**直接 HTTP**（`crates/agentero-core::features::zhihu`），Access Secret 存本机 `settings.json`（redact 掩码），而非官方 `zhihu-cli` + Keychain。理由：用户要求「在配置页面配置 secret」，且首版只做直答 / 检索，直连 HTTP 更薄。CLI 后端保留为后续方向。
> 范围：`features/zhihu` 后端、Agent 设置卡片（内置 Agent）、广场「知乎」面板、论文页「找知乎讨论」、划词「插入知乎引用」。
> 相关：[`plaza.md`](plaza.md)、[`plaza-feeds.md`](plaza-feeds.md)、[`agent-dx.md`](agent-dx.md)、[`../frontend/index.md`](../frontend/index.md)。
> 依赖：知乎开放平台（<https://developer.zhihu.com>）Access Secret；CLI 方案另需 `zhihu-cli` 官方二进制。

## 0. 设计决策总表

| # | 议题 | 结论 |
|---|---|---|
| D1 | 定位 | 知乎 = **社区经验层**：arXiv/S2 给学术文献，知乎给真实讨论、解读与实践经验 |
| D2 | 后端形态 | ~~统一 CLI 后端~~ **首版直接 HTTP**（core `features::zhihu`）；CLI 后端为后续方向 |
| D3 | 虚拟 Agent | Agent 注册表新增内置条目（`template: "zhihu"`，Host 内建运行器，**非外部 ACP 进程**），大脑用知乎直答（流式 SSE） |
| D4 | 凭证边界 | ~~Agentero 永不接触 Access Secret~~ **首版存本机 `settings.json`（0600、redact 掩码）**；CLI+Keychain 方案保留为后续 |
| D5 | 已有 CLI 用户 | 探测官方安装目录（`~/Library/Application Support/zhihu-cli/current/`），**直接复用不重装** |
| D6 | 安装器 | Rust 重实现官方 `setup.sh` 逻辑：公开 manifest + HTTPS + SHA-256 + size + tar 单成员 + 版本自报六重校验 |
| D7 | 配置一次全局生效 | `ZhihuState` 单一事实源，Agent 卡片 / 广场空态 / composer Agent 选择器同步订阅 |
| D8 | 明确不做 | 知识库上传、直答作通用 chat、me 数据、iframe 直嵌、默认全网搜索（见 §7） |

## 1. 动机与定位

Agentero 是 local-first 科研文献工作台。学术链路（arXiv 发现 → S2/Crossref 引用 → 阅读笔记）已完整；缺的是**社区经验层**：「这篇论文大家怎么理解」「这个领域从业者在聊什么」。知乎的搜索 / 热榜 / 直答 API 提供带社交信号（赞同数、评论数、认证）的中文社区内容，与学术源互补。

原则（用户明确要求）：**能力克制，一个后端，N 个薄入口**。只做发现与引用，不做账号管理、内容发布。

## 2. 总体架构

```
                ┌─ 配置层（做一次）──────────────────────────┐
                │  src-tauri/src/features/zhihu/             │
                │  ├─ detector  探测官方目录 CLI + 版本比对    │
                │  ├─ installer 一键下载（manifest 六重校验）  │
                │  ├─ auth      封装 auth set --secret-stdin  │
                │  ├─ runner    统一 CLI spawn（JSON stdout） │
                │  └─ ZhihuState 单一状态源 → Tauri event     │
                └──────────────┬─────────────────────────────┘
                               │ 统一 CLI runner
        ┌───────────┬──────────┼─────────────┬───────────────┐
        ▼           ▼          ▼             ▼               ▼
   广场知乎面板   知乎虚拟Agent  论文找讨论    划词插引用     额度/状态显示
   热榜+搜索     对话/直答     chip注入      markdown引用块  (设置页)
   (发现层)      (对话层)     (对话层)      (写作层)
```

- 配置状态全局共享：任一入口完成「安装 + 配置凭证」，全部入口就绪。
- 广场面板、划词等入口不引入任何独立配置，只有空态引导卡。

## 3. 后端：`features/zhihu`

仿 `discovery/feeds/` 的 feature 结构（commands + XDG state），核心四个模块：

### 3.1 detector（探测，零副作用）

```
1. macOS: ~/Library/Application Support/zhihu-cli/current/zhihu-cli
   Linux:  ${XDG_DATA_HOME:-~/.local/share}/zhihu-cli/current/zhihu-cli
2. 存在 → spawn `zhihu-cli version` 读版本 → 与本 feature 维护的 MIN_CLI_VERSION 比对
3. 兼容 → Ready 复用；低于 → 提示升级；损坏 → 修复入口
```

版本比较逻辑照抄官方 `run.sh` 的 `version_ge`（点分三段 + prerelease 语义）。

### 3.2 installer（一键下载）

官方 skill 的 `setup.sh` 逻辑透明可审计，Rust 重实现（reqwest + sha2 + tar，约百行）：

1. `GET https://developer-cdn.zhihu.com/zhihu-cli/releases/beta/manifest.json`
2. 选平台 artifact（darwin-arm64/amd64、win-amd64、linux-amd64/arm64 五平台）
3. 校验：HTTPS-only、artifact host 必须等于 manifest host、SHA-256、size ≤ 128MiB、tar 归档**有且仅有**根级单成员 `zhihu-cli`、二进制自报版本等于 manifest 版本
4. 原子落盘官方目录（`.zhihu-cli.new.$$` → rename），不动 PATH、不用 sudo
5. 进度经 Tauri event 推前端

### 3.3 auth（凭证，Agentero 零接触）

| 动作 | 命令 | 说明 |
|---|---|---|
| 写入 | `zhihu-cli auth set --secret-stdin` | secret 走 **stdin**（绝不进 argv，避免 `ps` 泄露）；CLI 内含一次在线验证 |
| 本地状态 | `zhihu-cli auth status` | 不联网、不耗额度；返回脱敏值 + `last_verified_at` |
| 在线验证 | `zhihu-cli auth status --verify` | 仅手动「重新检测」/ 换钥后调用；耗一次 creator 额度 |
| 清除 | `zhihu-cli auth logout` | 仅删本机 Keychain 条目 |

Agentero 侧**不存 secret**：settings.json 只存探测缓存（版本、脱敏值、last_verified_at）。

### 3.4 ZhihuState 与事件

```rust
struct ZhihuState {
    phase: Phase, // NotInstalled | Installing{progress} | NeedsSecret | Ready | Invalid{reason}
    version: Option<String>,
    secret_masked: Option<String>, // "b30d...e94c"，来自 auth status
    last_verified_at: Option<String>,
}
// emit "zhihu://state-changed"；前端 zustand store 订阅
```

### 3.5 Tauri commands（规划）

`zhihu_probe` / `zhihu_install` / `zhihu_set_secret`（stdin 桥接）/ `zhihu_logout` /
`zhihu_hot` / `zhihu_search` / `zhihu_answer`（流式 event）/ `zhihu_quota`

## 4. 配置层：Agent 设置卡片

复用现有 agent registry 卡片样式，新增状态点 + Secret 输入框。建议抽出通用 `SecretInputCard` 组件（知乎为首个消费者，后续 API 型服务复用）。

### 4.1 卡片状态机

```
① 未安装      ○  [ ⬇ 一键安装 ]
② 安装中      下载进度 ··· 62% · ✓ SHA-256 校验通过
③ 待配置凭证  ◐  Secret 输入框 + [保存并验证] + 前往 developer.zhihu.com/profile ↗
④ 就绪        ●  v0.6.0 · 密钥 b30d••••e94c · 额度 97/100 · 验证于 2m 前
              [更换密钥] [检查更新]；[⋯] 菜单：重新检测 / 清除本机凭证 / 复制诊断
⑤ 凭证失效    ●⚠ AUTH_INVALID · 密钥可能已轮换 → [重新输入] [前往个人中心重新生成 ↗]
```

### 4.2 绿点 Probe 策略（默认本地、零额度）

| 时机 | 命令 | 联网 | 耗额度 |
|---|---|---|---|
| App 启动 / 打开设置页 / 安装完成 | `version` + `auth status` | ❌ | ❌ |
| 保存 / 更换密钥后 | `auth set --secret-stdin`（内含在线验证） | ✅ | 一次 creator |
| 手动「重新检测」 | `auth status --verify` | ✅ | 一次 creator |
| 就绪态额度行 | `quota --api-id ...` | ✅ | ❌ 查询免费 |

颜色：🟢 = installed && compatible && auth.configured；🟡 = 待配密钥 / 探测中（呼吸动画）/ 网络不可达；🔴 = verify 失败 / 版本不兼容 / 安装损坏。tooltip 显示 `last_verified_at`。

### 4.3 Secret 输入安全清单

- `type="password"`；保存后立即从 React state 清除；不进日志、不进 settings.json
- stdin 传递；卡片只回显 CLI 返回的脱敏值
- 「更换密钥」= 重新展开输入框，流程同首次

## 5. 入口层

### 5.1 广场「知乎」面板（发现层，主入口）

`PLAZA_SOURCES` 加一条 `{ id: "zhihu", panel: "zhihu", ... }` + `plaza-zhihu-view.tsx`（壳抄 `PlazaArxivRecView`）。两个 tab：

| Tab | CLI 调用 | 呈现 |
|---|---|---|
| 热榜 | `zhihu-cli hot --limit 20` | 卡片流（标题/摘要/缩略图） |
| 搜索 | `zhihu-cli search zhihu --query ... --count 10` | 卡片带赞同数/评论数/作者/认证 |

卡片动作：「问 Agent」（复用 `buildPlazaAskPrompt`，`src/lib/plaza/ask-prompt.ts`）+「打开」（跳外部浏览器）。**不做**收藏、评论、全文抓取（知乎登录墙，`resolve_body` 不可靠）。空态 = 复用配置层引导卡；`ZhihuState` 变 Ready 时已打开面板经 event 自动切换。

### 5.2 知乎虚拟 Agent（对话层）

- `AgentDescriptor` 扩展 `kind: "builtin"`：registry 不 spawn 外部 ACP 进程，路由到 host 内建适配器
- prompt 路由表：

| 输入 | CLI 调用 | ACP 呈现 |
|---|---|---|
| 纯文本 | `answer --query <text> --stream --output sse` | 流式 AgentMessage（直答 LLM：zhida-fast/thinking/agent） |
| `/search 关键词` | `search zhihu --count 10` | ToolCall 块 + markdown 卡片 |
| `/hot` | `hot --limit 20` | 热榜卡片流 |
| `/topic 主题` | `question recommend --query --count 5` | 推荐问题列表 |

- 全部只读、无权限请求；`session/cancel` = kill 子进程
- **「问 Agent」兜底路由**：未连接任何 LLM Agent 时，广场卡片的「问 Agent」默认路由到知乎 Agent（开箱即用）

### 5.3 论文页「找知乎讨论」（对话层，最高价值/成本比）

论文页动作（命令面板或 References 侧栏旁）：当前论文标题 → `search zhihu` → 结果作为 composer 上下文 chip 注入对话。复用现有 chip / 选区 pin 机制。

### 5.4 划词「插入知乎引用」（写作层）

笔记内选中文字 → 划词菜单加「找知乎」（现有翻译/⌘K/⌘L 菜单旁）→ 浮出结果 → 点选插入：

```markdown
> **[标题](url)** — 作者（👍 128）
> 摘要文本…
```

先例：Cool Paper 笔记按钮（`paper_coolpapers_notes` 追加进当前笔记）。

### 5.5 零代码：vault Skill

`.agents/skills/zhihu/`（纯 markdown）教用户连接的 ACP Agent（Claude Code 等）直接调本机 `zhihu-cli`；可挂入广场 Skill 推荐面板（`skill-catalog.ts`）。与虚拟 Agent 共享同一 CLI 安装。

## 6. 技术选型记录：为什么是 CLI 后端

知乎侧可用通道有三：官方 `zhihu-cli` 二进制、官方 MCP 服务（zhihu_search / global_search / hot_list / zhida，SSE + Streamable HTTP）、裸 HTTP API。选 CLI：

1. **凭证边界**：MCP / HTTP 要求 Agentero 自存 secret、自管 Bearer；CLI 模式下凭证归 Keychain，Agentero 零鉴权代码
2. **流式**：`answer --stream` 有 SSE 增量；zhida 的 MCP 版等完整输出才返回
3. **无状态**：spawn 即用，崩了重跑；MCP over SSE 要管长连接 / sessionId
4. **错误处理现成**：CLI 已封装鉴权、时间戳、重试、错误码语义（`AUTH_REQUIRED` / 30001 频率 / 30002 配额等）

官方 MCP 留作未来彩蛋（如给外部客户端供工具）；裸 HTTP 仅在 CLI 后续废弃时兜底。

## 7. 明确不做

| 项 | 理由 |
|---|---|
| 知识库上传（RAG 云端） | 本地笔记推第三方云端，与广场「不注入登录态」隐私立场冲突，方向相反 |
| 直答作通用 chat | BYOA 架构下不引入第二套通用 LLM；直答仅作知乎 Agent 的检索增强大脑。唯一保留可能：未来作为翻译功能的免费 OpenAI 兼容 provider 备选 |
| me 数据（创作/关注/收藏） | 创作者场景，与文献工作台无关 |
| iframe 直嵌知乎 | 登录墙 / XFO，且代理模式不注入登录态 |
| 默认全网搜索 | Agent 自带 web 能力，重复；仅作搜索面板 Filter 彩蛋 |

## 8. 边界与风险

| 风险 | 缓解 |
|---|---|
| 直答多轮：CLI `answer` 单轮（HTTP 才支持多轮 messages） | 适配器拼接最近 N 轮进 query；体验不佳再评估直连 HTTP（需重议 D4） |
| 直答无引用链接（OpenAI 格式仅 content） | 引导「要来源用 /search」；或适配器组合：先 search 拿来源卡片再 answer |
| 额度：各能力组 100 次/日（未实名 10 次） | 设置页只读额度行（查询免费）；30001/30002 错误停止重试并提示 |
| 版本耦合：CLI 接口演进 | Rust 侧维护 `MIN_CLI_VERSION`；`upgrade --check` 供「检查更新」 |
| 用户预期 | Agent 描述文案明确「知乎社区检索增强问答」，非通用 chat |

## 9. 实施阶段

```
P1  features/zhihu 骨架：detector + installer + auth + ZhihuState + Agent 卡片（含 SecretInputCard）
P2  广场知乎面板（热榜 + 搜索 + 空态引导）
P3  知乎虚拟 Agent（builtin ACP 适配器 + answer 流式 + 斜杠路由）
P4  论文页「找知乎讨论」chip
P5  划词「插入知乎引用」
随时  vault Skill（零代码，可先行发布试用）
```
