use std::{io::Read, net::{IpAddr, Ipv4Addr, SocketAddr}, str::FromStr};

use async_openai::{Client, config::OpenAIConfig};
use axum::{http::{StatusCode, Uri, header}, response::{Html, IntoResponse, Response}};
use chat_ui::{LLMConfig, LLMProvider, StorageKind, ToolKind};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tracing::Level;

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
            default_values_t = vec![ToolKind::ZoomIn, ToolKind::JsInterpreter, ToolKind::DrawBbox, ToolKind::Curl, ToolKind::ResourceInspector],
            help = "Tools can be used by Qwen."
        )]
    tools: Vec<ToolKind>,

    #[clap(long,default_value_t = StorageKind::Sled, help = "Backend Storage")]
    backend: StorageKind,

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
            parallel_function_call: Some(self.parallel_function_call),
            system_prompt_lang: self.system_prompt_language.to_lang(),
            custom_system_prompt: None,
        }
    }
}

const INDEX_HTML: &str = "index.html";

pub async fn static_handler(uri: Uri) -> impl IntoResponse {
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

pub async fn index_html() -> Response {
    match Assets::get(INDEX_HTML) {
        Some(content) => Html(content.data).into_response(),
        None => not_found().await,
    }
}

fn initialize_provider(arg: &Arguments) -> Result<LLMProvider<OpenAIConfig>, anyhow::Error> {
    let config = OpenAIConfig::new()
        .with_api_base(&arg.provider)
        .with_api_key(&arg.api_key);
    let client = Client::with_config(config);
    tracing::info!("Created openai client.");
    let llm = LLMProvider::new(client, &arg.database_path, arg.backend, &arg.tools)?;
    tracing::info!("LLMProvider created.");
    Ok(llm)
}

#[derive(rust_embed::Embed, Clone)]
#[folder = "../../frontend/build"]
struct Assets;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
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
    let llm = initialize_provider(&args)?;
    let app = backend::get_http_router(llm, args.clone().into()).fallback(static_handler);
    let addr = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::from_str(&args.addr_serve)?),
        args.port_serve,
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Serving at {}:{}", args.addr_serve, args.port_serve);
    axum::serve(listener, app).await?;

    Ok(())
}
