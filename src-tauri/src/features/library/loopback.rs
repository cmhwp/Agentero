//! Loopback that receives the better-auth session token from the system browser.
//! Binds 127.0.0.1 only. The token is taken from the POST body and is not logged.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::Form;
use axum::response::Html;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::session::acceptable_token;
use crate::core::error::AppError;

#[derive(Deserialize)]
struct TokenForm {
    token: String,
    nonce: String,
}

pub struct PendingSignIn {
    pub port: u16,
    pub nonce: String,
    token_rx: Option<oneshot::Receiver<String>>,
    stop_tx: Option<oneshot::Sender<()>>,
}

impl PendingSignIn {
    pub async fn bind() -> Result<Self, AppError> {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let (token_tx, token_rx) = oneshot::channel();
        let slot = Arc::new(Mutex::new(Some(token_tx)));
        let expected = nonce.clone();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| AppError::message(format!("sign-in port: {error}")))?;
        let port = listener
            .local_addr()
            .map_err(|error| AppError::message(format!("sign-in port: {error}")))?
            .port();
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let app = Router::new().route(
            "/callback",
            post(move |Form(form): Form<TokenForm>| {
                let slot = slot.clone();
                let expected = expected.clone();
                async move {
                    if form.nonce != expected || !acceptable_token(&form.token) {
                        return Html(
                            "<!doctype html><meta charset=\"utf-8\"><title>Agentero</title><p>That session token was rejected.</p>"
                                .to_string(),
                        );
                    }
                    if let Some(sender) = slot.lock().ok().and_then(|mut guard| guard.take()) {
                        let _ = sender.send(form.token);
                    }
                    Html(
                        "<!doctype html><meta charset=\"utf-8\"><title>Agentero</title><p>Signed in. You can close this window.</p><p>已登录，可以关闭此窗口。</p>"
                            .to_string(),
                    )
                }
            }),
        );
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = stop_rx.await;
            });
            if let Err(error) = server.await {
                log::warn!(target: "agentero::library", "sign-in loopback ended: {error}");
            }
        });
        Ok(Self {
            port,
            nonce,
            token_rx: Some(token_rx),
            stop_tx: Some(stop_tx),
        })
    }

    pub async fn wait(mut self) -> Result<String, AppError> {
        let token_rx = self
            .token_rx
            .take()
            .ok_or_else(|| AppError::message("Sign-in was cancelled"))?;
        let result = tokio::time::timeout(Duration::from_secs(180), token_rx).await;
        self.stop();
        match result {
            Ok(Ok(token)) => Ok(token),
            Ok(Err(_)) => Err(AppError::message("Sign-in was cancelled")),
            Err(_) => Err(AppError::message("Sign-in timed out")),
        }
    }

    fn stop(&mut self) {
        if let Some(sender) = self.stop_tx.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for PendingSignIn {
    fn drop(&mut self) {
        self.stop();
    }
}
