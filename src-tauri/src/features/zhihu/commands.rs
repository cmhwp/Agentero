//! 知乎相关 Tauri 命令：凭证探测 + 提问草稿。

use agentero_core::features::zhihu::QuestionDraftInput;
use agentero_core::features::zhihu::{question_draft, ZhihuClient, ZhihuQuestionDraft};
use tauri::{AppHandle, Manager};

use crate::core::error::ApiResult;
use crate::features::system::settings::AppSettingsStore;

/// 探测已配置的知乎 Access Secret：发起一次最小检索，成功即视为可用。
/// 未配置或请求失败返回 false。
#[tauri::command]
#[specta::specta]
pub async fn zhihu_probe(app: AppHandle) -> ApiResult<bool> {
    let store = app.state::<AppSettingsStore>();
    let Some(secret) = store.zhihu_access_secret() else {
        return ApiResult::ok(false);
    };
    let client = ZhihuClient::new(secret);
    ApiResult::ok(client.search("知乎", 1).await.is_ok())
}

/// 生成知乎提问草稿：标题优先用直答译成中文（未配凭证/失败时回退原标题），
/// 正文为论文信息 + 摘要。桌面端无法像扩展那样直接填知乎提问弹窗，前端拿到草稿后
/// 复制到剪贴板并打开知乎。
#[tauri::command]
#[specta::specta]
pub async fn zhihu_question_draft(
    app: AppHandle,
    input: QuestionDraftInput,
) -> ApiResult<ZhihuQuestionDraft> {
    let zh_title = match app.state::<AppSettingsStore>().zhihu_access_secret() {
        Some(secret) if !input.title.trim().is_empty() => {
            ZhihuClient::new(secret).translate_title(&input.title).await
        }
        _ => None,
    };
    ApiResult::ok(question_draft(&input, zh_title.as_deref()))
}
