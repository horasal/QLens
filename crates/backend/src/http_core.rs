use std::{collections::HashMap, sync::Arc};

use anyhow::{Error, Result};
use axum::{
    Json,
    extract::{
        Multipart, Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use chat_ui::*;
use futures::{
    SinkExt, Stream, StreamExt,
    stream::{AbortHandle, AbortRegistration, Abortable},
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_util::sync::CancellationToken;

use uuid::Uuid;

use crate::AppState;

struct TaskControl {
    abort: AbortHandle,
    token: CancellationToken,
}
#[derive(Deserialize)]
pub struct ToolCallRequest {
    args: String,
}

pub async fn list_tools_handler(State(state): State<Arc<AppState>>) -> Response {
    Json(state.llm.list_tools()).into_response()
}

pub async fn call_tool_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(payload): Json<ToolCallRequest>,
) -> Response {
    let tool_use = ToolUse {
        use_id: Uuid::new_v4(),
        function_name: name,
        args: payload.args,
    };
    let result_message = state.llm.call_tool(tool_use).await;
    Json(result_message).into_response()
}

pub async fn chat_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ClientRequest {
    /// 发送新消息
    Chat {
        request_id: Uuid, // 前端生成的唯一ID
        chat_id: Uuid,
        content: Vec<MessageContent>,
        config: Option<LLMConfig>,
    },
    Regenerate {
        request_id: Uuid,
        chat_id: Uuid,
        message_id: Uuid, // 用户点击的那个消息 ID
        config: Option<LLMConfig>,
    },
    Edit {
        request_id: Uuid,
        chat_id: Uuid,
        message_id: Uuid,
        new_content: Vec<MessageContent>, // 用户修改后的新内容
        config: Option<LLMConfig>,
    },
    /// 终止生成
    Abort { request_id: Uuid, chat_id: Uuid },
}

#[derive(serde::Serialize)]
struct StreamPacket {
    chat_id: Uuid,
    request_id: Uuid,
    #[serde(flatten)]
    event: ChatEvent,
}

// 内部循环事件
enum LoopEvent {
    // Worker 线程生成好 JSON 字符串发给主线程
    InternalMsg(String),
    // 任务完成/失败信号，用于清理 Map
    TaskFinished(Uuid),
}
async fn handle_stream(
    chat_id: Uuid,
    request_id: Uuid,
    tx: UnboundedSender<LoopEvent>,
    stream: Result<impl Stream<Item = Result<ChatEvent, Error>>, Error>,
    abort_reg: AbortRegistration,
) {
    match stream {
        Ok(stream) => {
            tokio::pin!(stream);
            let mut stream = Abortable::new(stream, abort_reg);

            while let Some(event_result) = stream.next().await {
                match event_result {
                    Ok(event) => {
                        let packet = StreamPacket {
                            chat_id,
                            request_id,
                            event,
                        };
                        if let Ok(json) = serde_json::to_string(&packet) {
                            // 发送给主循环，如果通道已关(主循环挂了)则退出
                            if tx.send(LoopEvent::InternalMsg(json)).is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Stream error in task {}: {}", request_id, e);
                        let error_packet = StreamPacket {
                            chat_id,
                            request_id,
                            event: ChatEvent::Error(e.to_string()),
                        };
                        if let Ok(json) = serde_json::to_string(&error_packet) {
                            let _ = tx.send(LoopEvent::InternalMsg(json));
                        }
                        break;
                    }
                }
            }

            // 发送结束包 (StreamEnd)
            let end_packet = StreamPacket {
                chat_id,
                request_id,
                event: ChatEvent::StreamEnd {},
            };
            if let Ok(json) = serde_json::to_string(&end_packet) {
                let _ = tx.send(LoopEvent::InternalMsg(json));
            }
        }
        Err(e) => {
            tracing::error!("Failed to initialize stream for {}: {}", request_id, e);
            let error_packet = StreamPacket {
                chat_id,
                request_id,
                event: ChatEvent::Error(e.to_string()),
            };
            if let Ok(json) = serde_json::to_string(&error_packet) {
                let _ = tx.send(LoopEvent::InternalMsg(json));
            }
        }
    }
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    tracing::info!("New websocket Connection");

    let (mut sender, mut receiver) = socket.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<LoopEvent>();

    let mut tasks: HashMap<Uuid, TaskControl> = HashMap::new();

    loop {
        tokio::select! {
            //Branch A: 处理 WebSocket 发来的消息 (用户输入)
            ws_msg = receiver.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        let req: ClientRequest = match serde_json::from_str(&text) {
                            Ok(req) => req,
                            Err(e) => {
                                tracing::error!("Failed to parse client ws message: {}", e);
                                continue;
                            }
                        };

                        match req {
                            ClientRequest::Abort { request_id, chat_id } => {
                                tracing::info!("Abort request received for req: {}, chat: {}", request_id, chat_id);
                                if let Some(control) = tasks.remove(&request_id) {
                                    control.token.cancel();
                                    control.abort.abort();
                                }
                            }
                            ClientRequest::Chat { request_id, chat_id, content, config } => {
                                tracing::info!("Start generation for req: {}, chat: {}", request_id, chat_id);


                                let state = state.clone();
                                let tx = tx.clone();
                                let token = CancellationToken::new();

                                let llm_config = config.map(|x| x.merge_in(state.config.clone())).unwrap_or(state.config.clone());

                                let (abort_handle, abort_reg) = AbortHandle::new_pair();
                                tasks.insert(request_id, TaskControl { abort: abort_handle, token: token.clone() });

                                tokio::spawn(async move {
                                    let stream_result = state.llm.send_chat_message(chat_id, content, llm_config, token).await;
                                    handle_stream(chat_id, request_id, tx.clone(), stream_result, abort_reg).await;
                                    let _ = tx.send(LoopEvent::TaskFinished(request_id));
                                });
                            }
                            ClientRequest::Regenerate { request_id, chat_id, message_id, config } => {
                                tracing::info!("Regenerate request: {}, msg: {}", request_id, message_id);

                                let state = state.clone();
                                let tx = tx.clone();

                                let token = CancellationToken::new();
                                let (abort_handle, abort_reg) = AbortHandle::new_pair();
                                let llm_config = config.map(|x| x.merge_in(state.config.clone())).unwrap_or(state.config.clone());
                                tasks.insert(request_id, TaskControl { abort: abort_handle, token: token.clone() });
                                tokio::spawn(async move {
                                        let stream_result = state.llm.regenerate_at(
                                            chat_id,
                                            message_id,
                                            llm_config,
                                            token
                                        ).await;
                                        handle_stream(chat_id, request_id, tx.clone(), stream_result, abort_reg).await;
                                        let _ = tx.send(LoopEvent::TaskFinished(request_id));
                                });
                            }
                                ClientRequest::Edit { request_id, chat_id, message_id, new_content, config } => {
                                        tracing::info!("Edit request: {}", message_id);
                                        let state = state.clone();
                                        let tx = tx.clone();

                                        let token = CancellationToken::new();
                                        let (abort_handle, abort_reg) = AbortHandle::new_pair();
                                        tasks.insert(request_id, TaskControl { abort: abort_handle, token: token.clone() });
                                        let llm_config = config.map(|x| x.merge_in(state.config.clone())).unwrap_or(state.config.clone());

                                        tokio::spawn(async move {
                                            let stream_result = state.llm.edit_message_and_regenerate(
                                                chat_id,
                                                message_id,
                                                new_content,
                                                llm_config,
                                                token
                                            ).await;

                                            handle_stream(chat_id, request_id, tx.clone(), stream_result, abort_reg).await;
                                            let _ = tx.send(LoopEvent::TaskFinished(request_id));
                                        });
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!("Client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::warn!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {} // 忽略 Binary/Ping/Pong 等其他消息
                }
            }

            // Branch B: 处理内部任务发回的消息
            internal = rx.recv() => {
                match internal {
                    Some(LoopEvent::InternalMsg(json)) => {
                        if let Err(e) = sender.send(Message::Text(json.into())).await {
                            tracing::warn!("Failed to send message to client: {}, closing connection", e);
                            break;
                        }
                    }
                    Some(LoopEvent::TaskFinished(req_id)) => {
                        // 任务自然结束或出错结束，清理 Map
                        tasks.remove(&req_id);
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }

    // 循环退出（连接断开），强制终止所有还在运行的任务
    if !tasks.is_empty() {
        tracing::info!("Cleaning up {} active tasks", tasks.len());
        for (_, control) in tasks {
            control.abort.abort();
        }
    }
}

pub async fn model_list_handler(State(state): State<Arc<AppState>>) -> Response {
    match state.llm.get_model_names().await {
        Ok(list) => Json(list).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list models, {}", e),
        )
            .into_response(),
    }
}

pub async fn new_chat_handler(State(state): State<Arc<AppState>>) -> Response {
    match state.llm.new_chat() {
        Ok(entry) => {
            tracing::info!("New chat created via API: {}", entry.id);
            Json(entry).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create new chat entry: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create chat").into_response()
        }
    }
}
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn get_history_handler(
    State(state): State<Arc<AppState>>,
    Query(param): Query<PaginationParams>,
) -> Response {
    match state.llm.get_history_list(param.limit, param.offset) {
        Ok(e) => Json(e).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error {}", e),
        )
            .into_response(),
    }
}

pub async fn delete_chat_handler(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<Uuid>,
) -> Response {
    match state.llm.delete_chat(uuid) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => {
            tracing::error!("Failed to delete chat entry: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

pub async fn get_chat_handler(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<Uuid>,
) -> Response {
    match state.llm.get_chat(uuid) {
        Ok(Some(chat)) => Json(chat).into_response(),
        Ok(None) => {
            // 没找到，这是客户端错误 (404)
            tracing::warn!("Chat entry not found: {}", uuid);
            (StatusCode::NOT_FOUND, "Chat not found").into_response()
        }
        Err(e) => {
            // 数据库IO错误，这是服务器错误 (500)
            tracing::error!("Failed to get chat entry: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct UploadResponse {
    file: String,
    uuid: AssetId,
}

pub async fn download_image(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<AssetId>,
) -> impl IntoResponse {
    match state.llm.get_image(uuid) {
        Ok(Some(bytes)) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                CONTENT_TYPE,
                guess_content_type(&bytes)
                    .unwrap_or("image/jpeg")
                    .parse()
                    .unwrap(),
            );
            (headers, bytes).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Image not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to retrieve image {}: {}", uuid, e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

pub async fn download_asset_handler(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<AssetId>,
) -> impl IntoResponse {
    match state.llm.get_asset(uuid) {
        Ok(Some(bytes)) => {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, "application/octet-stream".parse().unwrap());
            (headers, bytes).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Asset not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to retrieve asset {}: {}", uuid, e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

pub async fn upload_asset_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Response {
    let mut responses = Vec::new();
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let file_name = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown_file".to_string()); // 备用名称

                tracing::debug!("开始接收文件: {}", file_name);

                let data = match field.bytes().await {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::warn!("Failed to read stream {}: {}", file_name, e);
                        let error_msg = format!("Failed to read data for {}: {}", file_name, e);
                        return (StatusCode::BAD_REQUEST, error_msg).into_response();
                    }
                };
                let uuid = match state.llm.save_asset(&data) {
                    Ok(uuid) => uuid,
                    Err(e) => {
                        tracing::error!("Unable save {} to database: {}", file_name, e);
                        let error_msg = "Failed to save asset to database".to_string();
                        return (StatusCode::INTERNAL_SERVER_ERROR, error_msg).into_response();
                    }
                };

                responses.push(UploadResponse {
                    file: file_name,
                    uuid,
                });
            }
            Ok(None) => {
                break;
            }
            Err(e) => {
                // 客户端发送的 multipart stream 格式错误或连接中断
                tracing::warn!("Failed to parse multipart stream : {}", e);
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid multipart stream: {}", e),
                )
                    .into_response();
            }
        }
    }

    Json(responses).into_response()
}

fn guess_content_type(input_data: &[u8]) -> Result<&str, anyhow::Error> {
    let format = image::guess_format(&input_data)?;
    Ok(format.to_mime_type())
}

pub async fn upload_image(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Response {
    let mut responses = Vec::new();
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let file_name = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown_file".to_string()); // 备用名称

                tracing::debug!("开始接收文件: {}", file_name);

                let data = match field.bytes().await {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::warn!("Failed to read stream {}: {}", file_name, e);
                        let error_msg = format!("Failed to read data for {}: {}", file_name, e);
                        return (StatusCode::BAD_REQUEST, error_msg).into_response();
                    }
                };
                let data = match chat_ui::convert_to_png(data.to_vec()) {
                    Ok(png_bytes) => png_bytes.to_vec(),
                    Err(e) => {
                        tracing::warn!("Failed to convert image to PNG: {}, keeping original", e);
                        data.to_vec()
                    }
                };
                let uuid = match state.llm.save_image(&data) {
                    Ok(uuid) => uuid,
                    Err(e) => {
                        tracing::error!("Unable save {} to database: {}", file_name, e);
                        let error_msg = "Failed to save image to database".to_string();
                        return (StatusCode::INTERNAL_SERVER_ERROR, error_msg).into_response();
                    }
                };

                responses.push(UploadResponse {
                    file: file_name,
                    uuid,
                });
            }
            Ok(None) => {
                break;
            }
            Err(e) => {
                // 客户端发送的 multipart stream 格式错误或连接中断
                tracing::warn!("Failed to parse multipart stream : {}", e);
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid multipart stream: {}", e),
                )
                    .into_response();
            }
        }
    }

    Json(responses).into_response()
}
