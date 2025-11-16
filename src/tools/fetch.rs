use mime::Mime;
use reqwest::header::CONTENT_TYPE;
use schemars::JsonSchema;
use schemars::schema_for;
use serde::Deserialize;
use std::time::Duration;

use crate::MessageContent;
use crate::Tool;
use crate::ToolDescription;
use crate::tools::utils::save_image_to_db;
use crate::tools::utils::save_svg_to_db;

#[derive(Deserialize, JsonSchema)]
struct FetchArgs {
    #[schemars(description = "The target URL to fetch content from.")]
    url: String,

    #[schemars(
        description = "HTTP method. Use 'Post' only when submitting data. Defaults to 'Get'."
    )]
    method: Option<FetchMethod>,

    #[schemars(
        description = "Set to true ONLY when you need to analyze Javascript code specifically. Defaults to false (scripts are removed for cleaner reading)."
    )]
    keep_script: Option<bool>,

    #[schemars(description = "string content for POST requests. Ignored for GET requests.")]
    post_content: Option<String>,
    #[schemars(
        description = "ContentType for `post_content`. Default to `application/json` and ignored for GET requests."
    )]
    post_content_type: Option<String>,

    #[schemars(description = "Optional label for this request")]
    label: Option<String>,
}

#[derive(Deserialize, JsonSchema, Copy, Clone)]
enum FetchMethod {
    Get,
    Post,
}

pub struct FetchTool {
    db: sled::Tree,
    client: reqwest::blocking::Client,
}

impl FetchTool {
    pub fn new(ctx: sled::Tree) -> Self {
        Self { db: ctx,
            client: reqwest::blocking::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(40))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new()),
        }
    }
}

impl Tool for FetchTool {
    fn name(&self) -> String {
        "curl_url".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "curl_url".to_string(),
            name_for_human: "网页抓取工具(curl_url_tool)".to_string(),
            description_for_model:
"Access and retrieve content from a specific URL.
* Allow to fetch image binary and any text-base content.
* If remote content is an image, the content of this image and its actual uuid will be returned; the image format may be converted for rendering purpose.
* If remote content is HTML, it will be automatically converted to Markdown and all links are preserved as remote url.
* Other text-based content will be returned as-is.".to_string(),
            parameters: serde_json::to_value(schema_for!(FetchArgs)).unwrap(),
            args_format: "输入格式必须是JSON。".to_string(),
        }
    }

    fn call(&self, args: &str) -> Result<MessageContent, anyhow::Error> {
        let args: FetchArgs = serde_json::from_str(args)?;
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
        let res = req_builder.send()?;
        let status = res.status();
        if !status.is_success() {
            return Ok(MessageContent::Text(format!(
                "Failed to fetch URL. HTTP Status: {}",
                status
            )));
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
                let html = res.text()?;
                let mut skip_tags = vec!["style"];
                // 除非显式要求保留 script，否则移除
                if args.keep_script != Some(true) {
                    skip_tags.push("script");
                }

                let markdown = htmd::HtmlToMarkdownBuilder::new()
                    .skip_tags(skip_tags)
                    .build()
                    .convert(&html)?;

                Ok(MessageContent::Text(markdown))
            }
            (mime::TEXT, _)
            | (mime::APPLICATION, mime::JSON)
            | (mime::APPLICATION, mime::JAVASCRIPT)
            | (mime::APPLICATION, mime::XML) => Ok(MessageContent::Text(res.text()?)),

            (mime::IMAGE, sub_type) => {
                let uuid = if sub_type.as_str().to_lowercase().contains("svg") {
                    save_svg_to_db(&self.db, &res.text()?)?
                } else {
                    let bytes = res.bytes()?.to_vec();
                    save_image_to_db(&self.db, &super::convert_to_png(bytes)?)?
                };
                Ok(MessageContent::ImageRef(
                    uuid,
                    args.label.unwrap_or(args.url),
                ))
            }

            _ => {
                if let Some(suffix) = mime_type.suffix() {
                    if suffix == mime::JSON || suffix == mime::XML {
                        return Ok(MessageContent::Text(res.text()?));
                    }
                }

                match res.text() {
                    Ok(text) => Ok(MessageContent::Text(text)),
                    Err(_) => Ok(MessageContent::Text(format!(
                        "Unsupported Binary Content-Type: {}",
                        mime_type
                    ))),
                }
            }
        }
    }
}
