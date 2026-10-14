//! 知乎 Access Secret 的连通性探测命令。

use agentero_core::features::zhihu::ZhihuClient;
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
