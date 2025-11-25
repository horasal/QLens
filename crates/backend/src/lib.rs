mod http_core;
use http_core::*;
use std::sync::Arc;

use async_openai::config::OpenAIConfig;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use chat_ui::{LLMConfig, LLMProvider};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
};
struct AppState {
    llm: LLMProvider<OpenAIConfig>,
    config: LLMConfig,
}

pub fn get_http_router(llm: LLMProvider<OpenAIConfig>, config: LLMConfig) -> Router {
    let llm = AppState { llm, config };

    Router::new()
        .route("/api/tools", get(list_tools_handler))
        .route("/api/tools/{name}", post(call_tool_handler))
        .route("/api/models", get(model_list_handler))
        .route("/api/chat", get(chat_handler))
        .route("/api/chat/new", post(new_chat_handler))
        .route("/api/history", get(get_history_handler))
        .route(
            "/api/history/{id}",
            get(get_chat_handler).delete(delete_chat_handler),
        )
        .route("/api/asset/{id}", get(download_asset_handler))
        .route("/api/asset", post(upload_asset_handler))
        .route("/api/image/{id}", get(download_image))
        .route("/api/image", post(upload_image))
        .with_state(Arc::new(llm))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(CompressionLayer::new())
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(20 * 1000 * 1000))
}
