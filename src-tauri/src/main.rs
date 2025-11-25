// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use async_openai::{config::OpenAIConfig, Client};
use backend::get_http_router;
use chat_ui::{LLMConfig, LLMProvider, StorageKind, ToolKind};
use tauri::Manager;

#[tokio::main]
async fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let app_dir = app.path().app_data_dir().unwrap();
            std::fs::create_dir_all(&app_dir).unwrap();
            let db_path = app_dir.join("chat.redb");
            tauri::async_runtime::spawn(async move {
                let config = OpenAIConfig::new() .with_api_base("http://localhost:8080") .with_api_key("");
                let client = Client::with_config(config);
                let llm = LLMProvider::new(client, db_path, StorageKind::Redb, &ToolKind::default_list())
                    .expect("Can not start llm service");
                let api_router = get_http_router(llm, LLMConfig::default());
                let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
                    .await
                    .unwrap();
                axum::serve(listener, api_router).await.unwrap()
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
