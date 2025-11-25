use mime::Mime;
use reqwest::header::CONTENT_TYPE;
use schemars::JsonSchema;
use schemars::schema_for;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use crate::MessageContent;
use crate::Tool;
use crate::ToolDescription;
use crate::blob::BlobStorage;
use crate::convert_svg_to_png;
use crate::parse_tool_args;

const MAX_TEXT_LEN: usize = 10 * 1024;

#[derive(Deserialize, JsonSchema)]
struct FetchArgs {
    #[schemars(description = "Target URL")]
    url: String,

    #[schemars(
        description = "HTTP method(default:Get)"
    )]
    method: Option<FetchMethod>,

    #[schemars(
        description = "Keep <script> tags in HTML? Default: false"
    )]
    keep_script: Option<bool>,

    #[schemars(description = "POST body.")]
    post_content: Option<String>,
    #[schemars(
        description = "POST MIME type(default:application/json)"
    )]
    post_content_type: Option<String>,

    #[schemars(description = "label for this request")]
    label: Option<String>,
}

#[derive(Deserialize, JsonSchema, Copy, Clone)]
enum FetchMethod {
    #[serde(alias = "GET", alias = "get")]
    Get,
    #[serde(alias = "POST", alias = "post")]
    Post,
}

pub struct FetchTool {
    image : Arc<dyn BlobStorage>,
    asset: Arc<dyn BlobStorage>,
    client: reqwest::Client,
}

impl FetchTool {
    pub fn new(image: Arc<dyn BlobStorage>, asset: Arc<dyn BlobStorage>) -> Self {
        Self { image: image,
            asset: asset,
            client: reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(40))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

#[async_trait::async_trait]
impl Tool for FetchTool {
    fn name(&self) -> String {
        "curl_url".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "curl_url".to_string(),
            name_for_human: "网页抓取工具(curl_url)".to_string(),
            description_for_model: "Fetch remote URL.
    - HTML -> Converted to Markdown automatically.
    - Binary/Image -> Downloaded to Asset/Image DB, returns UUID.
    - Text -> Returned as-is.
    Supports GET/POST.".to_string(),
            parameters: serde_json::to_value(schema_for!(FetchArgs)).unwrap(),
            args_format: "JSON Object.".to_string(),
        }
    }

    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let args: FetchArgs = parse_tool_args(args)?;
        let mut req_builder = match args.method.unwrap_or(FetchMethod::Get) {
            FetchMethod::Get => self.client.get(&args.url),
            FetchMethod::Post => self.client.post(&args.url),
        };

        if let Some(content) = args.post_content {
            if matches!(args.method, Some(FetchMethod::Post)) {
                req_builder = req_builder
                    .header(
                        CONTENT_TYPE,
                        args.post_content_type
                            .unwrap_or("application/json".to_string()),
                    )
                    .body(content);
            }
        }
        let res = req_builder.send().await?;
        let status = res.status();
        if !status.is_success() {
            return Ok(vec![MessageContent::Text(format!(
                "Failed to fetch URL. HTTP Status: {}",
                status
            ))]);
        }
        let mime_type = if let Some(content_type) = res.headers().get(CONTENT_TYPE) {
            let content_type_str = content_type.to_str().unwrap_or("");
            match content_type_str.parse::<Mime>() {
                Ok(m) => {
                    if m == mime::APPLICATION_OCTET_STREAM {
                        if let Ok(parsed_url) = reqwest::Url::parse(&args.url) {
                            mime_guess::from_path(parsed_url.path()).first_or_octet_stream()
                        } else {
                            mime_guess::from_path(&args.url).first_or_octet_stream()
                        }
                    } else {
                        m
                    }
                }
                Err(_) => mime::APPLICATION_OCTET_STREAM, // 解析失败就当做二进制
            }
        } else {
            mime_guess::from_path(&args.url).first_or_octet_stream()
        };
        match (mime_type.type_(), mime_type.subtype()) {
            (mime::TEXT, mime::HTML) => {
                let html = res.text().await?;
                let mut skip_tags = vec!["style"];
                // 除非显式要求保留 script，否则移除
                if args.keep_script != Some(true) {
                    skip_tags.push("script");
                }

                let markdown = htmd::HtmlToMarkdownBuilder::new()
                    .skip_tags(skip_tags)
                    .build()
                    .convert(&html)?;

                Ok(vec![MessageContent::Text(markdown)])
            }
            (mime::TEXT, _)
            | (mime::APPLICATION, mime::JSON)
            | (mime::APPLICATION, mime::JAVASCRIPT)
            | (mime::APPLICATION, mime::XML) => Ok(vec![MessageContent::Text(res.text().await?)]),

            (mime::IMAGE, sub_type) => {
                let uuid = if sub_type.as_str().to_lowercase().contains("svg") {
                    self.image.save(&convert_svg_to_png(&res.text().await?)?)?
                } else {
                    let bytes = res.bytes().await?.to_vec();
                    self.image.save(&super::convert_to_png(bytes)?)?
                };
                Ok(vec![MessageContent::ImageRef(
                    uuid,
                    args.label.unwrap_or(args.url),
                )])
            }

            _ => {
                if let Some(suffix) = mime_type.suffix() {
                    if suffix == mime::JSON || suffix == mime::XML {
                        return Ok(vec![MessageContent::Text(res.text().await?)]);
                    }
                }
                let bytes = res.bytes().await?.to_vec();

                match String::from_utf8(bytes.clone()) {
                    Ok(text) if text.len() < MAX_TEXT_LEN => Ok(vec![MessageContent::Text(text)]),
                    _ => {
                        let uuid = self.asset.save(&bytes)?;
                        tracing::info!("Blob fetched and saved as asset {}", uuid);
                        Ok(vec![MessageContent::AssetRef(uuid, mime_type.to_string())])
                    }
                }
            }
        }
    }
}
