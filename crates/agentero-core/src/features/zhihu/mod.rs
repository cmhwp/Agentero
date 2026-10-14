//! 知乎开放平台客户端：直答（流式）+ 标题检索。
//!
//! Access Secret 由 Host 从 `settings.json` 读出后作为参数传入；本模块不接触
//! 存储，也不感知 Agent 层。鉴权沿用开放平台约定：`Authorization: Bearer` +
//! 秒级 `X-Request-Timestamp`。

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// 知乎开放平台基址。
pub const BASE: &str = "https://developer.zhihu.com";

/// 直答默认模型。
pub const DEFAULT_MODEL: &str = "zhida-fast-1p5";

/// 检索请求超时。
const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);
/// 直答是流式长连接，超时放宽。
const CHAT_TIMEOUT: Duration = Duration::from_secs(180);

/// 相关度下限。原型实测：真实相关讨论 ≈1.6+，相邻噪声 0.5–0.7，无关 <0.4。
/// 检索始终会返回内容，所以必须以阈值兜底。
pub const SEARCH_SCORE_FLOOR: f64 = 0.5;

/// 知乎开放平台客户端。构造即持有一次调用的凭证。
#[derive(Debug, Clone)]
pub struct ZhihuClient {
    secret: String,
}

/// 检索结果条目（对齐开放平台 `zhihu_search` 的字段）。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ZhihuSearchItem {
    /// `answer` | `article`
    pub kind: String,
    pub title: String,
    pub url: String,
    pub excerpt: String,
    pub author: String,
    pub votes: u64,
    /// 开放平台的 `RankingScore`。
    pub score: f64,
}

impl ZhihuClient {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.secret)
    }

    /// 按标题检索知乎内容。`Code != 0` 视为失败（开放平台自身的信封）。
    pub async fn search(
        &self,
        query: &str,
        count: usize,
    ) -> Result<Vec<ZhihuSearchItem>, AppError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(AppError::message("知乎检索关键词为空"));
        }
        let client = crate::http::client(SEARCH_TIMEOUT)?;
        let url = format!("{BASE}/api/v1/content/zhihu_search");
        let res = client
            .get(url)
            .header("Authorization", self.bearer())
            .header("X-Request-Timestamp", now_secs())
            .query(&[
                ("Query", query.to_string()),
                ("Count", count.clamp(1, 10).to_string()),
            ])
            .send()
            .await
            .map_err(|e| AppError::message(format!("知乎检索请求失败：{e}")))?;
        let status = res.status();
        let text = res
            .text()
            .await
            .map_err(|e| AppError::message(format!("知乎检索响应读取失败：{e}")))?;
        if !status.is_success() {
            return Err(AppError::message(format!(
                "知乎检索 HTTP {status}: {}",
                crate::http::http_err_snippet(&text)
            )));
        }
        parse_search_response(&text)
    }

    /// 一次非流式直答，把英文标题译成简短中文（知乎提问标题上限 50 字）。
    /// 任何失败都返回 `None`，调用方回退用原标题。
    pub async fn translate_title(&self, title: &str) -> Option<String> {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        let prompt = format!(
            "将下面这篇论文的英文标题译成简洁的中文标题。只输出译文，不加引号、书名号或句号，不超过 28 个字。\n{title}"
        );
        let body = serde_json::json!({
            "model": DEFAULT_MODEL,
            "stream": false,
            "messages": [{ "role": "user", "content": prompt }],
        });
        let client = crate::http::client(SEARCH_TIMEOUT).ok()?;
        let res = client
            .post(format!("{BASE}/v1/chat/completions"))
            .header("Authorization", self.bearer())
            .header("X-Request-Timestamp", now_secs())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        let value: serde_json::Value = res.json().await.ok()?;
        let raw = value["choices"][0]["message"]["content"].as_str()?.trim();
        let out = raw
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(|c| matches!(c, '「' | '」' | '"' | '\'' | '《' | '》' | '。'))
            .trim()
            .to_string();
        if out.is_empty() || out.chars().count() > 40 {
            None
        } else {
            Some(out)
        }
    }

    /// 知乎直答（OpenAI 兼容 `/v1/chat/completions`），流式把增量交给 `on_delta`，
    /// 返回完整文本。
    pub async fn chat_stream(
        &self,
        model: &str,
        prompt: &str,
        mut on_delta: impl FnMut(&str) + Send,
    ) -> Result<String, AppError> {
        let model = if model.trim().is_empty() {
            DEFAULT_MODEL
        } else {
            model.trim()
        };
        let client = crate::http::client(CHAT_TIMEOUT)?;
        let body = serde_json::json!({
            "model": model,
            "stream": true,
            "messages": [{ "role": "user", "content": prompt }],
        });
        let res = client
            .post(format!("{BASE}/v1/chat/completions"))
            .header("Authorization", self.bearer())
            .header("X-Request-Timestamp", now_secs())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::message(format!("知乎直答请求失败：{e}")))?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(AppError::message(format!(
                "知乎直答 HTTP {status}: {}",
                crate::http::http_err_snippet(&text)
            )));
        }

        let mut res = res;
        let mut buf = String::new();
        let mut full = String::new();
        'outer: loop {
            let Some(bytes) = res
                .chunk()
                .await
                .map_err(|e| AppError::message(format!("知乎直答流读取失败：{e}")))?
            else {
                break;
            };
            buf.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(idx) = buf.find('\n') {
                let line = buf[..idx].trim_end_matches('\r').to_string();
                buf.drain(..=idx);
                let Some(payload) = sse_payload(&line) else {
                    continue;
                };
                if payload == "[DONE]" {
                    break 'outer;
                }
                if let Some(delta) = sse_delta(payload)? {
                    full.push_str(&delta);
                    on_delta(&delta);
                }
            }
        }
        Ok(full)
    }
}

/// 取 SSE `data:` 载荷；非 data 行（含 `: keep-alive` 注释）返回 None。
fn sse_payload(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

/// 解析一个 SSE data 载荷：空/心跳 → None；服务端错误 → Err；否则取增量文本。
fn sse_delta(payload: &str) -> Result<Option<String>, AppError> {
    if payload.is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|e| AppError::message(format!("知乎直答流解析失败：{e}")))?;
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("知乎直答返回错误");
        return Err(AppError::message(msg.to_string()));
    }
    let delta = value["choices"][0]["delta"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    Ok(if delta.is_empty() { None } else { Some(delta) })
}

#[derive(Deserialize)]
struct SearchEnvelope {
    #[serde(rename = "Code", default)]
    code: i64,
    #[serde(rename = "Message", default)]
    message: String,
    #[serde(rename = "Data", default)]
    data: Option<SearchData>,
}

#[derive(Deserialize)]
struct SearchData {
    #[serde(rename = "Items", default)]
    items: Vec<SearchRawItem>,
}

#[derive(Deserialize)]
struct SearchRawItem {
    #[serde(rename = "ContentType", default)]
    content_type: String,
    #[serde(rename = "Title", default)]
    title: String,
    #[serde(rename = "Url", default)]
    url: String,
    #[serde(rename = "ContentText", default)]
    content_text: String,
    #[serde(rename = "AuthorName", default)]
    author_name: String,
    #[serde(rename = "VoteUpCount", default)]
    vote_up_count: u64,
    #[serde(rename = "RankingScore", default)]
    ranking_score: f64,
}

fn parse_search_response(text: &str) -> Result<Vec<ZhihuSearchItem>, AppError> {
    let envelope: SearchEnvelope = serde_json::from_str(text)
        .map_err(|e| AppError::message(format!("知乎检索响应解析失败：{e}")))?;
    if envelope.code != 0 {
        let message = if envelope.message.trim().is_empty() {
            "知乎检索失败".to_string()
        } else {
            envelope.message
        };
        return Err(AppError::message(message));
    }
    Ok(envelope
        .data
        .map(|d| d.items)
        .unwrap_or_default()
        .into_iter()
        .map(|it| ZhihuSearchItem {
            kind: if it.content_type.eq_ignore_ascii_case("answer") {
                "answer".to_string()
            } else {
                "article".to_string()
            },
            title: it.title.replace(" - 知乎", ""),
            url: it.url.split('?').next().unwrap_or("").to_string(),
            excerpt: it.content_text.chars().take(160).collect(),
            author: it.author_name,
            votes: it.vote_up_count,
            score: it.ranking_score,
        })
        .collect())
}

fn now_secs() -> String {
    chrono::Utc::now().timestamp().to_string()
}

/// 知乎提问标题上限（平台限制）。
pub const QUESTION_TITLE_MAX: usize = 50;

/// 正文里摘要的最大字符数，避免长摘要把草稿撑爆。
const DRAFT_ABSTRACT_MAX: usize = 800;

/// 生成提问草稿所需的论文信息（均为公开元数据，前端从 catalog 取）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QuestionDraftInput {
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub doi: String,
    #[serde(default)]
    pub arxiv_id: String,
    /// 论文主页链接；为空时按 DOI / arXiv 推导。
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub abstract_text: String,
}

/// 交给用户发布（或复制到剪贴板）的知乎提问草稿。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ZhihuQuestionDraft {
    pub title: String,
    pub body: String,
}

/// 组装知乎提问草稿：标题 `如何评价论文《…》？`（≤50 字），正文含论文信息与摘要。
/// `zh_title` 为直答译文，缺失时回退原标题。
pub fn question_draft(input: &QuestionDraftInput, zh_title: Option<&str>) -> ZhihuQuestionDraft {
    let frame = |t: &str| format!("如何评价论文《{t}》？");
    let base = zh_title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(input.title.trim());
    let mut title = frame(base);
    if title.chars().count() > QUESTION_TITLE_MAX {
        let room = QUESTION_TITLE_MAX.saturating_sub(frame("").chars().count() + 1);
        let clipped: String = base.chars().take(room).collect();
        title = frame(&format!("{clipped}…"));
    }

    let venue = if !input.venue.trim().is_empty() {
        input.venue.trim().to_string()
    } else if !input.arxiv_id.trim().is_empty() {
        "arXiv".to_string()
    } else {
        String::new()
    };
    let link = if !input.doi.trim().is_empty() {
        format!("https://doi.org/{}", input.doi.trim())
    } else if !input.url.trim().is_empty() {
        input.url.trim().to_string()
    } else if !input.arxiv_id.trim().is_empty() {
        format!("https://arxiv.org/abs/{}", input.arxiv_id.trim())
    } else {
        String::new()
    };

    let mut lines: Vec<String> = Vec::new();
    if !input.title.trim().is_empty() {
        lines.push(format!("论文：{}", input.title.trim()));
    }
    let authors: Vec<&str> = input
        .authors
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect();
    if !authors.is_empty() {
        lines.push(format!("作者：{}", authors.join(", ")));
    }
    if !venue.is_empty() {
        let year = input.year.trim();
        let suffix = if year.is_empty() {
            String::new()
        } else {
            format!("（{year}）")
        };
        lines.push(format!("期刊：{venue}{suffix}"));
    }
    if !link.is_empty() {
        lines.push(format!("链接：{link}"));
    }
    let abstract_text = input.abstract_text.trim();
    if !abstract_text.is_empty() {
        let clipped: String = abstract_text.chars().take(DRAFT_ABSTRACT_MAX).collect();
        lines.push(String::new());
        lines.push(format!("摘要：{clipped}"));
    }

    ZhihuQuestionDraft {
        title,
        body: lines.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_payload_skips_non_data_lines() {
        assert_eq!(sse_payload(": keep-alive"), None);
        assert_eq!(sse_payload("event: message"), None);
        assert_eq!(sse_payload("data: {\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_payload("data:[DONE]"), Some("[DONE]"));
    }

    #[test]
    fn sse_delta_extracts_incremental_content() {
        let payload = r#"{"choices":[{"delta":{"content":"你好"}}]}"#;
        assert_eq!(sse_delta(payload).unwrap().as_deref(), Some("你好"));
        assert_eq!(sse_delta("").unwrap(), None);
        assert_eq!(sse_delta(r#"{"choices":[{"delta":{}}]}"#).unwrap(), None);
    }

    #[test]
    fn sse_delta_surfaces_error_frame() {
        let err = sse_delta(r#"{"error":{"message":"quota exceeded"}}"#).unwrap_err();
        assert!(err.to_string().contains("quota exceeded"));
    }

    #[test]
    fn search_response_maps_items_and_envelope_errors() {
        let ok = r#"{"Code":0,"Message":"success","Data":{"Items":[{"ContentType":"Answer","Title":"标题 - 知乎","Url":"https://www.zhihu.com/question/1/answer/2?utm=1","ContentText":"正文","AuthorName":"某人","VoteUpCount":12,"RankingScore":1.8}]}}"#;
        let items = parse_search_response(ok).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "answer");
        assert_eq!(items[0].title, "标题");
        assert_eq!(items[0].url, "https://www.zhihu.com/question/1/answer/2");
        assert_eq!(items[0].votes, 12);
        assert_eq!(items[0].score, 1.8);

        let bad = r#"{"Code":30001,"Message":"频率超限","Data":null}"#;
        let err = parse_search_response(bad).unwrap_err();
        assert!(err.to_string().contains("频率超限"));
    }

    fn draft_input() -> QuestionDraftInput {
        QuestionDraftInput {
            title: "Hilbert's sixth problem".into(),
            authors: vec!["Yu Deng".into(), "Zaher Hani".into()],
            venue: "arXiv".into(),
            year: "2025".into(),
            doi: "10.1000/xyz".into(),
            arxiv_id: "2503.01800".into(),
            url: String::new(),
            abstract_text: "A derivation of fluid equations.".into(),
        }
    }

    #[test]
    fn question_draft_frames_title_and_prefers_translation() {
        let input = draft_input();
        let english = question_draft(&input, None);
        assert_eq!(english.title, "如何评价论文《Hilbert's sixth problem》？");
        assert!(english.body.contains("论文：Hilbert's sixth problem"));
        assert!(english.body.contains("作者：Yu Deng, Zaher Hani"));
        assert!(english.body.contains("期刊：arXiv（2025）"));
        assert!(english.body.contains("链接：https://doi.org/10.1000/xyz"));
        assert!(english
            .body
            .contains("摘要：A derivation of fluid equations."));

        let translated = question_draft(&input, Some("希尔伯特第六问题"));
        assert_eq!(translated.title, "如何评价论文《希尔伯特第六问题》？");
    }

    #[test]
    fn question_draft_caps_title_at_platform_limit() {
        let input = QuestionDraftInput {
            title: "x".repeat(200),
            ..QuestionDraftInput::default()
        };
        let draft = question_draft(&input, None);
        assert!(draft.title.chars().count() <= QUESTION_TITLE_MAX);
        assert!(draft.title.ends_with("》？"));
        assert!(draft.title.contains('…'));
    }

    #[test]
    fn question_draft_falls_back_to_arxiv_link() {
        let input = QuestionDraftInput {
            title: "T".into(),
            arxiv_id: "2503.01800".into(),
            ..QuestionDraftInput::default()
        };
        let draft = question_draft(&input, None);
        assert!(draft.body.contains("期刊：arXiv"));
        assert!(draft
            .body
            .contains("链接：https://arxiv.org/abs/2503.01800"));
        assert!(!draft.body.contains("摘要："));
    }
}
