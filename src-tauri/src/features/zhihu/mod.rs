//! 内置「知乎」Agent：Host 内建适配器。
//!
//! 不 spawn 外部 ACP 进程，而是直连知乎开放平台，把结果转成前端既有的
//! `agent:*` 事件（stream / completed / failed / models / commands），因此
//! 右侧栏 Agent 面板无需任何特殊分支即可渲染。凭证来自 App settings。
//!
//! 能力：
//! - 默认：知乎直答（流式）。
//! - `/搜索 <关键词>`：按标题检索知乎内容（原型的「查索引」，真后端待接入）。

pub mod commands;

use std::sync::{Arc, Mutex};

use agentero_core::features::zhihu::{ZhihuClient, DEFAULT_MODEL};

use crate::features::agent::models::{
    AgentCommand, AgentCommandInput, AgentCommandsEvent, AgentFailedEvent, AgentModelChoice,
    AgentModelsEvent, AgentResultPayload, AgentStreamEvent, AgentStreamKind,
};
use crate::features::agent::AgentEventEmitter;

/// 内置 Agent 的模型（展示名对齐开放平台）。
pub const ZHIHU_MODEL_ID: &str = DEFAULT_MODEL;
const ZHIHU_MODEL_NAME: &str = "知乎直答";

/// 引擎侧对话模型选择器的 config id。
const MODEL_CONFIG_ID: &str = "model";

/// 一次直答 / 检索运行所需的一切（已从 `agent_run_once` 解构）。
pub struct ZhihuRunOnce {
    pub app: AgentEventEmitter,
    pub session_id: String,
    pub message_id: String,
    pub agent_id: String,
    pub prompt: String,
    /// 续聊时前端回传的 provider session id；用于保持同一历史项。
    pub resume_session_id: Option<String>,
    pub model_id: Option<String>,
    pub secret: Option<String>,
    pub cancellation: tokio::sync::watch::Receiver<bool>,
}

/// 模型目录事件（warm 与首次运行都会广播，供配置条展示）。
pub fn models_event(agent_id: &str, session_id: &str, current: &str) -> AgentModelsEvent {
    AgentModelsEvent {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        config_id: MODEL_CONFIG_ID.to_string(),
        current_id: current.to_string(),
        models: vec![AgentModelChoice {
            id: ZHIHU_MODEL_ID.to_string(),
            name: ZHIHU_MODEL_NAME.to_string(),
            group: None,
        }],
    }
}

/// 斜杠命令清单（供 composer `/` 菜单）。
pub fn builtin_commands() -> Vec<AgentCommand> {
    vec![AgentCommand {
        name: "搜索".to_string(),
        description: "按标题检索知乎内容".to_string(),
        input: Some(AgentCommandInput {
            hint: "关键词".to_string(),
        }),
    }]
}

/// 广播模型目录 + 斜杠命令（面板打开 / 切换 Agent 时 warm 调用）。
pub fn emit_warm_events(app: &AgentEventEmitter, agent_id: &str, model_id: Option<&str>) {
    let current = model_id
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(ZHIHU_MODEL_ID);
    let _ = app.emit("agent:models", models_event(agent_id, "warm", current));
    let _ = app.emit(
        "agent:commands",
        AgentCommandsEvent {
            session_id: "warm".to_string(),
            agent_id: agent_id.to_string(),
            commands: builtin_commands(),
        },
    );
}

/// 运行一轮：按 prompt 前缀分流到检索或直答；错误统一走 `agent:failed`。
pub async fn run_once(params: ZhihuRunOnce) {
    let provider_session_id = params
        .resume_session_id
        .clone()
        .unwrap_or_else(|| params.session_id.clone());
    match execute(&params, &provider_session_id).await {
        Ok(()) => {}
        Err(error) => {
            let _ = params.app.emit(
                "agent:failed",
                AgentFailedEvent {
                    session_id: params.session_id.clone(),
                    error: error.to_string(),
                },
            );
        }
    }
}

async fn execute(
    params: &ZhihuRunOnce,
    provider_session_id: &str,
) -> Result<(), agentero_core::error::AppError> {
    let Some(secret) = params
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Err(agentero_core::error::AppError::domain(
            "zhihuNoSecret",
            "尚未配置知乎 Access Secret。请在 设置 → Agent → 知乎 中填入。",
        ));
    };
    let client = ZhihuClient::new(secret);
    let model = params
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(ZHIHU_MODEL_ID);

    match parse_route(&params.prompt) {
        Route::Search(query) => run_search(params, &client, provider_session_id, &query).await,
        Route::Chat(prompt) => run_chat(params, &client, provider_session_id, model, &prompt).await,
    }
}

enum Route {
    Search(String),
    Chat(String),
}

/// `/搜索` / `/search` 前缀 → 检索；其余 → 直答。命令与关键词之间必须空白分隔，
/// 避免 `/搜索历史` 被误判。
fn parse_route(prompt: &str) -> Route {
    let trimmed = prompt.trim();
    for prefix in ["/搜索", "/search"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return Route::Search(rest.trim().to_string());
            }
        }
    }
    Route::Chat(trimmed.to_string())
}

async fn run_chat(
    params: &ZhihuRunOnce,
    client: &ZhihuClient,
    provider_session_id: &str,
    model: &str,
    prompt: &str,
) -> Result<(), agentero_core::error::AppError> {
    if prompt.trim().is_empty() {
        return Err(agentero_core::error::AppError::message(
            "请输入要提问的内容",
        ));
    }

    let content = Arc::new(Mutex::new(String::new()));
    let content_cb = Arc::clone(&content);
    let emitter = params.app.clone();
    let session_cb = params.session_id.clone();
    let stream = client.chat_stream(model, prompt, move |delta| {
        if let Ok(mut buf) = content_cb.lock() {
            buf.push_str(delta);
        }
        let _ = emitter.emit(
            "agent:stream",
            AgentStreamEvent {
                session_id: session_cb.clone(),
                chunk: delta.to_string(),
                kind: AgentStreamKind::Message,
            },
        );
    });

    let mut cancel = params.cancellation.clone();
    tokio::select! {
        result = stream => result.map(|_| ())?,
        () = wait_cancel(&mut cancel) => {
            emit_completed(params, provider_session_id, take(&content), Some("cancelled"));
            return Ok(());
        }
    }

    emit_completed(params, provider_session_id, take(&content), None);
    Ok(())
}

async fn run_search(
    params: &ZhihuRunOnce,
    client: &ZhihuClient,
    provider_session_id: &str,
    query: &str,
) -> Result<(), agentero_core::error::AppError> {
    if query.is_empty() {
        return Err(agentero_core::error::AppError::message(
            "请提供检索关键词，例如 /搜索 希尔伯特第六问题",
        ));
    }
    let mut items = client.search(query, 10).await?;
    items.retain(|item| item.score >= agentero_core::features::zhihu::SEARCH_SCORE_FLOOR);
    let text = format_search_results(query, &items);
    let _ = params.app.emit(
        "agent:stream",
        AgentStreamEvent {
            session_id: params.session_id.clone(),
            chunk: text.clone(),
            kind: AgentStreamKind::Message,
        },
    );
    emit_completed(params, provider_session_id, text, None);
    Ok(())
}

fn format_search_results(
    query: &str,
    items: &[agentero_core::features::zhihu::ZhihuSearchItem],
) -> String {
    if items.is_empty() {
        return format!("知乎上暂无与「{query}」相关的讨论。");
    }
    let mut out = format!("**知乎检索：{query}**\n\n");
    for (idx, item) in items.iter().enumerate() {
        let kind = if item.kind == "answer" {
            "回答"
        } else {
            "文章"
        };
        out.push_str(&format!(
            "{}. [{}]({}) — {} · {} 赞同（{}）\n",
            idx + 1,
            item.title,
            item.url,
            item.author,
            item.votes,
            kind
        ));
        let excerpt = item.excerpt.trim();
        if !excerpt.is_empty() {
            out.push_str(&format!("   > {excerpt}\n"));
        }
    }
    out
}

fn emit_completed(
    params: &ZhihuRunOnce,
    provider_session_id: &str,
    content: String,
    stop_reason: Option<&str>,
) {
    let _ = params.app.emit(
        "agent:completed",
        AgentResultPayload {
            session_id: params.session_id.clone(),
            message_id: params.message_id.clone(),
            content,
            reasoning: None,
            sources: Vec::new(),
            stop_reason: stop_reason.map(str::to_string),
            provider_session_id: Some(provider_session_id.to_string()),
        },
    );
}

fn take(content: &Arc<Mutex<String>>) -> String {
    content.lock().map(|g| g.clone()).unwrap_or_default()
}

async fn wait_cancel(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_slash_search_and_default_chat() {
        match parse_route("/搜索 希尔伯特第六问题") {
            Route::Search(q) => assert_eq!(q, "希尔伯特第六问题"),
            _ => panic!("expected search"),
        }
        match parse_route("/search Ricci flow") {
            Route::Search(q) => assert_eq!(q, "Ricci flow"),
            _ => panic!("expected search"),
        }
        // No whitespace separator → treated as a plain question, not a command.
        match parse_route("/搜索历史") {
            Route::Chat(q) => assert_eq!(q, "/搜索历史"),
            _ => panic!("expected chat"),
        }
        match parse_route("这篇论文讲了什么") {
            Route::Chat(q) => assert_eq!(q, "这篇论文讲了什么"),
            _ => panic!("expected chat"),
        }
    }

    #[test]
    fn search_markdown_lists_links() {
        let item = agentero_core::features::zhihu::ZhihuSearchItem {
            kind: "answer".into(),
            title: "如何理解".into(),
            url: "https://www.zhihu.com/question/1/answer/2".into(),
            excerpt: "摘要".into(),
            author: "某人".into(),
            votes: 12,
            score: 1.5,
        };
        let md = format_search_results("关键词", &[item]);
        assert!(md.contains("[如何理解](https://www.zhihu.com/question/1/answer/2)"));
        assert!(md.contains("12 赞同"));
        assert_eq!(
            format_search_results("x", &[]),
            "知乎上暂无与「x」相关的讨论。"
        );
    }
}
