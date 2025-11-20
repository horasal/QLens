use std::{
    collections::HashMap,
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
};

use anyhow::{Error, Result};
use async_openai::{Client, config::OpenAIConfig};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Multipart, Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{
        HeaderMap, StatusCode, Uri,
        header::{self, CONTENT_TYPE},
    },
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use chat_ui::*;
use clap::Parser;
use futures::{
    SinkExt, Stream, StreamExt,
    stream::{AbortHandle, AbortRegistration, Abortable},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{
    mpsc::{self, UnboundedSender},
};
use tokio_util::sync::CancellationToken;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
};
use tracing::Level;
use uuid::Uuid;

#[derive(Debug, Clone, clap::Parser, Serialize, Deserialize)]
struct Arguments {
    #[arg(
        long,
        default_value = "http://127.0.0.1:8080",
        help = "Endpoint of LLM server, without \"/v1\""
    )]
    provider: String,
    #[arg(long, default_value = "")]
    api_key: String,

    #[arg(short, long, default_value = "127.0.0.1", help = "Server address")]
    addr_serve: String,
    #[arg(short, long, default_value = "3000")]
    port_serve: u16,

    #[arg(
        short,
        long,
        default_value = "chat_data",
        help = "path to folder where chat data are saved"
    )]
    database_path: String,

    #[arg(
        short,
        long,
        help = "load config from file and overwrite all terminal settings"
    )]
    config_file: Option<std::path::PathBuf>,

    #[arg(long, help = "ID of the model to use")]
    model: Option<String>,
    #[arg(long, default_value = "0.8", help = "temperature between 0.0 and 2.0")]
    temp: Option<f32>,
    #[arg(long, help = "stream results from llm", default_value = "true")]
    stream: Option<bool>,
    #[arg(
        long,
        default_value = "1.0",
        help = "Number between -2.0 and 2.0. Positive values penalize new tokens based on their existing frequency in the text so far, decreasing the model's likelihood to repeat the same line verbatim."
    )]
    frequency_penalty: Option<f32>,
    #[arg(
        long,
        help = "Number between -2.0 and 2.0. Positive values penalize new tokens based on whether they appear in the text so far"
    )]
    presence_penality: Option<f32>,
    #[arg(long, help = "nucleus sampling, recommend altering this or `temp`")]
    top_p: Option<f32>,
    #[arg(long, help = "Unique identifier representing your end user")]
    user: Option<String>,
    #[arg(long)]
    seed: Option<i64>,
    #[arg(
        long,
        help = "Upper bound for the number of tokens that can be generated for a completion, including vision and reasoning"
    )]
    max_completion_tokens: Option<u32>,
    #[arg(
        long,
        default_value = "false",
        help = "Allow LLM to call different tool once in a time"
    )]
    parallel_function_call: bool,

    #[clap(
            long,
            value_delimiter = ',',
            num_args = 1..,
            default_values_t = vec![ToolKind::ZoomIn, ToolKind::JsInterpreter, ToolKind::DrawBbox, ToolKind::Curl],
            help = "Tools can be used by Qwen."
        )]
    tools: Vec<ToolKind>,

    #[clap(long, value_enum, default_value_t = PromptLanguage::English)]
    system_prompt_language: PromptLanguage,

    #[clap(
        long,
        help = "Dump current config values to json and exit",
        default_value_t = false
    )]
    #[serde(skip)]
    dump_config: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Deserialize, Serialize)]
enum PromptLanguage {
    Auto,
    English,
    Chinese,
    Korean,
    Japanese,
}

impl PromptLanguage {
    fn to_lang(&self) -> Option<whatlang::Lang> {
        match self {
            PromptLanguage::Auto => None,
            PromptLanguage::Chinese => Some(whatlang::Lang::Cmn),
            PromptLanguage::English => Some(whatlang::Lang::Eng),
            PromptLanguage::Korean => Some(whatlang::Lang::Kor),
            PromptLanguage::Japanese => Some(whatlang::Lang::Jpn),
        }
    }
}

impl Into<LLMConfig> for Arguments {
    fn into(self) -> LLMConfig {
        LLMConfig {
            model: self.model,
            temp: self.temp,
            stream: self.stream,
            frequency_penalty: self.frequency_penalty,
            presence_penality: self.presence_penality,
            top_p: self.top_p,
            user: self.user,
            seed: self.seed,
            max_completion_tokens: self.max_completion_tokens,
            parallel_function_call: self.parallel_function_call,
            system_prompt_lang: self.system_prompt_language.to_lang(),
        }
    }
}

struct AppState {
    llm: LLMProvider<OpenAIConfig>,
    config: LLMConfig,
}
#[derive(rust_embed::Embed, Clone)]
#[folder = "frontend_clean/build"]
struct Assets;


struct TaskControl {
    abort: AbortHandle,
    token: CancellationToken,
}

fn initialize_provider(arg: &Arguments) -> Result<LLMProvider<OpenAIConfig>> {
    let config = OpenAIConfig::new()
        .with_api_base(&arg.provider)
        .with_api_key(&arg.api_key);
    let client = Client::with_config(config);
    tracing::info!("Created openai client.");
    let llm = LLMProvider::new(client, &arg.database_path, &arg.tools)?;
    tracing::info!("LLMProvider created.");
    Ok(llm)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();
    let args = Arguments::parse();
    if args.dump_config {
        eprintln!("{}", serde_json::to_string_pretty(&args)?);
        return Ok(());
    }
    let args = if let Some(ref p) = args.config_file {
        tracing::info!("Load config from file {}.", p.to_string_lossy());
        let mut v = Vec::new();
        let mut f = std::fs::File::open(p)?;
        f.read_to_end(&mut v)?;
        serde_json::from_slice(&v)?
    } else {
        args
    };

    let llm = AppState {
        llm: initialize_provider(&args)?,
        config: args.clone().into(),
    };

    let app = Router::new()
        .route("/api/chat", get(chat_handler))
        .route("/api/chat/new", post(new_chat_handler))
        .route("/api/history", get(get_history_handler))
        .route(
            "/api/history/{id}",
            get(get_chat_handler).delete(delete_chat_handler),
        )
        .route("/api/image/{id}", get(download_image))
        .route("/api/image", post(upload_image))
        .fallback(static_handler)
        .with_state(Arc::new(llm))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(CompressionLayer::new())
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(20 * 1000 * 1000));
    let addr = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::from_str(&args.addr_serve)?),
        args.port_serve,
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Serving at {}:{}", args.addr_serve, args.port_serve);
    axum::serve(listener, app).await?;

    Ok(())
}

const INDEX_HTML: &str = "index.html";

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == INDEX_HTML {
        return index_html().await;
    }

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();

            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => {
            if path.contains('.') {
                return not_found().await;
            }

            index_html().await
        }
    }
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "404").into_response()
}

async fn index_html() -> Response {
    match Assets::get(INDEX_HTML) {
        Some(content) => Html(content.data).into_response(),
        None => not_found().await,
    }
}

async fn chat_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
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
    },
    Regenerate {
        request_id: Uuid,
        chat_id: Uuid,
        message_id: Uuid, // 用户点击的那个消息 ID
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
            // 这里可以构造一个 Error 类型的 Packet 发回给前端
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
                                // 可选：发送错误回执给前端
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
                            ClientRequest::Chat { request_id, chat_id, content } => {
                                tracing::info!("Start generation for req: {}, chat: {}", request_id, chat_id);

                                let state = state.clone();
                                let tx = tx.clone();
                                let token = CancellationToken::new();

                                let (abort_handle, abort_reg) = AbortHandle::new_pair();
                                tasks.insert(request_id, TaskControl { abort: abort_handle, token: token.clone() });

                                tokio::spawn(async move {
                                    let stream_result = state.llm.send_chat_message(chat_id, content, state.config.clone(), token).await;
                                    handle_stream(chat_id, request_id, tx.clone(), stream_result, abort_reg).await;
                                    let _ = tx.send(LoopEvent::TaskFinished(request_id));
                                });
                            }
                            ClientRequest::Regenerate { request_id, chat_id, message_id } => {
                                tracing::info!("Regenerate request: {}, msg: {}", request_id, message_id);

                                let state = state.clone();
                                let tx = tx.clone();

                                let token = CancellationToken::new();
                                let (abort_handle, abort_reg) = AbortHandle::new_pair();
                                tasks.insert(request_id, TaskControl { abort: abort_handle, token: token.clone() });
                                tokio::spawn(async move {
                                        let stream_result = state.llm.regenerate_at(
                                            chat_id,
                                            message_id,
                                            state.config.clone(),
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

async fn new_chat_handler(State(state): State<Arc<AppState>>) -> Response {
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
async fn get_history_handler(State(state): State<Arc<AppState>>) -> Json<Vec<ChatMeta>> {
    Json(state.llm.get_history_list())
}

async fn delete_chat_handler(
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

async fn get_chat_handler(State(state): State<Arc<AppState>>, Path(uuid): Path<Uuid>) -> Response {
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
pub struct UploadImageResponse {
    file: String,
    uuid: Uuid,
}

async fn download_image(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<Uuid>,
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

fn guess_content_type(input_data: &[u8]) -> Result<&str, anyhow::Error> {
    let format = image::guess_format(&input_data)?;
    Ok(format.to_mime_type())
}

async fn upload_image(State(state): State<Arc<AppState>>, mut multipart: Multipart) -> Response {
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

                responses.push(UploadImageResponse {
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
