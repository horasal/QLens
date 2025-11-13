use std::{
    io::{Cursor, Read},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
};

use anyhow::Result;
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
    SinkExt, StreamExt,
    stream::{AbortHandle, Abortable},
};
use image::ImageFormat;
use serde::{Deserialize, Serialize};
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

    #[clap(long, default_value = "zoom_in,image_memo,js_interpreter")]
    tools: String,
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

fn initialize_provider(arg: &Arguments) -> Result<LLMProvider<OpenAIConfig>> {
    let config = OpenAIConfig::new()
        .with_api_base(&arg.provider)
        .with_api_key(&arg.api_key);
    let client = Client::with_config(config);
    tracing::info!("Created openai client.");

    let db = sled::Config::new()
        .temporary(false)
        .path(&arg.database_path)
        .use_compression(true)
        .open()?;
    let image_db = db.open_tree("image")?;
    let history_db = db.open_tree("history")?;
    tracing::info!("DB started.");
    let toolset = arg
        .tools
        .split(',')
        .filter_map(|v| {
            let t = get_tool(v.trim(), image_db.clone());
            if t.is_none() {
                tracing::warn!("Unknown tool {}", v);
            }
            t
        })
        .fold(ToolSet::builder(), |ts, t| ts.add_tool(t))
        .build();
    let llm = LLMProvider::new(client, history_db, image_db, toolset)?;
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

#[derive(serde::Deserialize, Debug)]
struct ClientWSMessage {
    chat_id: Uuid,
    content: Vec<MessageContent>,
}

#[derive(serde::Serialize)]
struct StreamPacket {
    chat_id: Uuid,
    // 使用 serde(flatten) 将 ChatEvent 的字段直接嵌入到 JSON 根部
    #[serde(flatten)]
    event: ChatEvent,
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    tracing::info!("New websocket Connection");
    let (mut sender, mut receiver) = socket.split();

    tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                let client_msg: ClientWSMessage = match serde_json::from_str(&text) {
                    Ok(msg) => msg,
                    Err(e) => {
                        tracing::error!("Failed to parse client ws message: {}", e);
                        // 可以向客户端发送一个错误
                        continue; // 跳过此消息
                    }
                };

                tracing::info!("Received message for chat {}.", client_msg.chat_id,);

                let chat_id_for_stream = client_msg.chat_id;

                match state
                    .llm
                    .send_chat_message(client_msg.chat_id, client_msg.content, state.config.clone())
                    .await
                {
                    Ok(stream) => {
                        tokio::pin!(stream);
                        let (abort_handle, abort_reg) = AbortHandle::new_pair();
                        let mut stream = Abortable::new(stream, abort_reg);
                        while let Some(event) = stream.next().await {
                            match event {
                                Ok(chat_event) => {
                                    let packet = StreamPacket {
                                        chat_id: chat_id_for_stream,
                                        event: chat_event,
                                    };
                                    match serde_json::to_string(&packet) {
                                        Ok(json_event) => {
                                            if sender
                                                .send(Message::Text(json_event.into()))
                                                .await
                                                .is_err()
                                            {
                                                abort_handle.abort();
                                                tracing::info!(
                                                    "Disconnected by client (on data packet)"
                                                );
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "FATAL: Failed to serialize data packet: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    abort_handle.abort();
                                    tracing::error!("Stream error: {}", e);
                                    break;
                                }
                            }
                        }
                        let end_packet = StreamPacket {
                            chat_id: chat_id_for_stream,
                            event: ChatEvent::StreamEnd {},
                        };
                        match serde_json::to_string(&end_packet) {
                            Ok(json_event) => {
                                if sender.send(Message::Text(json_event.into())).await.is_err() {
                                    tracing::info!("Disconnected by client (on StreamEnd)");
                                    abort_handle.abort();
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "FATAL: Failed to serialize StreamEnd packet: {}",
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to send message: {}", e);
                    }
                }
            }
        }
        tracing::info!("WebSocket client disconnected.");
    });
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
    Json(
        state
            .llm
            .history
            .iter()
            .filter_map(|v| v.ok())
            .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
            .collect(),
    )
}

async fn delete_chat_handler(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<Uuid>,
) -> Response {
    match state.llm.history.remove(uuid) {
        Ok(Some(ivec)) => match serde_json::from_slice::<ChatEntry>(&ivec) {
            Ok(entry) => {
                for e in entry.messages.into_iter() {
                    match e.owner {
                        Role::Tools => {
                            for c in e.content.into_iter() {
                                match c {
                                    MessageContent::ImageBin(_, img_id, _)
                                    | MessageContent::ImageRef(img_id, _) => {
                                        match state.llm.image.remove(img_id) {
                                            Ok(None) => tracing::warn!(
                                                "Image {} referenced by {} but not exist while deleting.",
                                                img_id,
                                                uuid
                                            ),
                                            Err(e) => tracing::error!(
                                                "Failed to delete image {} from database, {}",
                                                img_id,
                                                e
                                            ),
                                            Ok(_) => {}
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
                StatusCode::OK.into_response()
            }
            Err(e) => {
                tracing::error!("Failed to deserialize chat entry {}: {}", uuid, e);
                StatusCode::OK.into_response()
            }
        },
        Ok(None) => StatusCode::OK.into_response(),
        Err(e) => {
            tracing::error!("Failed to get chat entry: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

async fn get_chat_handler(State(state): State<Arc<AppState>>, Path(uuid): Path<Uuid>) -> Response {
    let db_result = match state.llm.history.get(uuid) {
        Ok(Some(ivec)) => Ok(ivec),
        Ok(None) => {
            // 没找到，这是客户端错误 (404)
            tracing::warn!("Chat entry not found: {}", uuid);
            return (StatusCode::NOT_FOUND, "Chat not found").into_response();
        }
        Err(e) => {
            // 数据库IO错误，这是服务器错误 (500)
            tracing::error!("Failed to get chat entry: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error"))
        }
    };

    // 如果 db_result 是 Err，提前返回
    let data = match db_result {
        Ok(data) => data,
        Err(response) => return response.into_response(),
    };

    match serde_json::from_slice::<ChatEntry>(&data) {
        Ok(entry) => Json(entry).into_response(),
        Err(e) => {
            // 数据损坏，这是服务器错误 (500)
            tracing::error!("Failed to deserialize chat entry: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Corrupted data").into_response()
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
    match state.llm.image.get(uuid) {
        Ok(Some(ivec)) => {
            let bytes: Vec<u8> = ivec.to_vec();
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, "image/jpeg".parse().unwrap());
            (headers, bytes).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Image not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to retrieve image {}: {}", uuid, e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

pub fn convert_to_png(input_data: Vec<u8>) -> Result<Vec<u8>, anyhow::Error> {
    let format = image::guess_format(&input_data)?;
    match format {
        ImageFormat::Jpeg | ImageFormat::Png => Ok(input_data),
        _ => {
            let img = image::load_from_memory(&input_data)?;
            let mut png_data = Vec::new();
            let mut cursor = Cursor::new(&mut png_data);
            img.write_to(&mut cursor, ImageFormat::Png)?;
            Ok(png_data)
        }
    }
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
                let data = match convert_to_png(data.to_vec()) {
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
