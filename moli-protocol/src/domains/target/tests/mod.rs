use super::*;
use crate::testing::TestContext;
use axum::{
    Router,
    http::{
        HeaderMap, StatusCode,
        header::{CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::{IntoResponse, Redirect},
    routing::{any, get},
};
use moli_cookie_jar::NetworkCookieRequestContext;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

mod test_support;
mod tests_background_auto_attach;
mod tests_background_staging;
mod tests_browser_context;
mod tests_cdp_chromium_imports;
mod tests_cdp_smoke_chromium;
mod tests_cdp_smoke_fixture;
mod tests_cdp_smoke_playwright;
mod tests_control_plane;
mod tests_patchright;
mod tests_patchright_runtimeless;
mod tests_playwright_cdp;
mod tests_popup_close_lifecycle;
mod tests_popup_document_location;
mod tests_popup_response_ownership;
mod tests_same_context_bindings;
mod tests_same_context_overrides;
mod tests_same_context_state;
mod tests_same_context_target_local;
mod tests_target_attachment;
mod tests_target_basics;
mod tests_target_creation;
mod tests_target_management;

use test_support::*;

pub(super) async fn target_8mb_stack<F, Fut>(name: &'static str, build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    let result = std::thread::Builder::new()
        .name(name.into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("8mb-stack target test runtime should build")
                .block_on(build());
        })
        .expect("8mb-stack target test thread should spawn")
        .join();

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
