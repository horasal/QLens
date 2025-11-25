use std::{str::FromStr, sync::Arc};

use schemars::{JsonSchema, schema_for};
use serde::Deserialize;

use crate::{MessageContent, Tool, ToolDescription, AssetId, blob::BlobStorage};

fn bytes_preview(b: &[u8]) -> String {
    b.iter()
        .take(32)
        .map(|c| format!("{:02x}", c))
        .collect::<Vec<String>>()
        .join(" ")
}

#[derive(Deserialize, JsonSchema)]
pub struct ImageArgs {
    #[schemars(description = "Image UUID")]
    img_idx: String,
}

pub struct ImageTool(Arc<dyn BlobStorage>);

impl ImageTool {
    pub fn new(image: Arc<dyn BlobStorage>) -> Self {
        Self(image)
    }
}

#[async_trait::async_trait]
impl Tool for ImageTool {
    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let args: ImageArgs = serde_json::from_str(args)?;
        let uuid = AssetId::from_str(&args.img_idx)?;
        Ok(match self.0.get(uuid)? {
            None => {
                vec![MessageContent::Text("Image does not exist.".to_string())]
            }
            Some(v) => {
                self.0.retain(uuid)?;
                vec![MessageContent::ImageRef(
                    uuid,
                    format!("FileSize:{},Preview:{}", v.len(), bytes_preview(&v)),
                )]
            }
        })
    }

    fn description(&self) -> super::ToolDescription {
        ToolDescription {
            name_for_human: "View Image".to_string(),
            name_for_model: "Image".to_string(),
            description_for_model: "View Image".to_string(),
            parameters: serde_json::to_value(schema_for!(ImageArgs)).unwrap(),
            args_format: "JSON".to_string(),
        }
    }

    fn name(&self) -> String {
        "Image".to_string()
    }

    fn visible_to_human(&self) -> bool {
        true
    }

    fn visible_to_model(&self) -> bool {
        false
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct AssetArgs {
    #[schemars(description = "Asset UUID")]
    asset_idx: String,
}

pub struct AssetTool(Arc<dyn BlobStorage>);

impl AssetTool {
    pub fn new(asset: Arc<dyn BlobStorage>) -> Self {
        Self(asset)
    }
}

#[async_trait::async_trait]
impl Tool for AssetTool {
    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let args: AssetArgs = serde_json::from_str(args)?;
        let uuid = AssetId::from_str(&args.asset_idx)?;
        Ok(match self.0.get(uuid)? {
            None => {
                vec![MessageContent::Text("Asset does not exist.".to_string())]
            }
            Some(v) => {
                self.0.retain(uuid)?;
                vec![MessageContent::AssetRef(
                    uuid,
                    format!("FileSize:{},Preview:{}", v.len(), bytes_preview(&v)),
                )]
            }
        })
    }

    fn description(&self) -> super::ToolDescription {
        ToolDescription {
            name_for_human: "View Asset".to_string(),
            name_for_model: "Asset".to_string(),
            description_for_model: "View Asset".to_string(),
            parameters: serde_json::to_value(schema_for!(AssetArgs)).unwrap(),
            args_format: "JSON".to_string(),
        }
    }

    fn name(&self) -> String {
        "Asset".to_string()
    }

    fn visible_to_human(&self) -> bool {
        true
    }

    fn visible_to_model(&self) -> bool {
        false
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct InspectArgs {
    uuid: String,
    #[serde(rename = "type", alias = "Type", alias = "TYPE")]
    ty: Option<ResourceType>,
}

#[derive(Deserialize, JsonSchema)]
pub enum ResourceType {
    #[serde(alias = "IMAGE", alias = "image")]
    Image,
    #[serde(alias = "ASSET", alias = "asset")]
    Asset,
}

pub struct ResourceInspector {
    image: Arc<dyn BlobStorage>,
    asset: Arc<dyn BlobStorage>,
}

const PEEK_SIZE: usize = 2048;

impl ResourceInspector {
    pub fn new(image: Arc<dyn BlobStorage>, asset: Arc<dyn BlobStorage>) -> Self {
        Self { image, asset }
    }

    fn try_read(
        &self,
        id: AssetId,
        ty: Option<ResourceType>,
    ) -> Result<(Option<(Vec<u8>, usize)>, ResourceType), anyhow::Error> {
        match ty {
            Some(t) => {
                let storage = match t {
                    ResourceType::Asset => &self.asset,
                    ResourceType::Image => &self.image,
                };
                storage
                    .peek(id, PEEK_SIZE)
                    .map_err(|e| e.into())
                    .map(|v| (v, t))
            }
            None => self
                .image
                .peek(id, PEEK_SIZE)
                .map(|v| (v, ResourceType::Image))
                .or(self
                    .asset
                    .peek(id, PEEK_SIZE)
                    .map(|v| (v, ResourceType::Asset)))
                .map_err(|e| e.into()),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ResourceInspector {
    fn name(&self) -> String {
        "ResourceInspector".into()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "ResourceInspector".into(),
            name_for_human: "Resource Inspector".into(),
            description_for_model: "Preview Asset or Image".into(),
            parameters: serde_json::to_value(schema_for!(InspectArgs)).unwrap(),
            args_format: "JSON".into(),
        }
    }

    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let args: InspectArgs = serde_json::from_str(args)?;
        let uuid = AssetId::from_str(&args.uuid)?;

        let ((data, size), ty) = match self.try_read(uuid, args.ty)? {
            (Some(d), ty) => (d, ty),
            (None, _) => return Ok(vec![MessageContent::Text("Resource not found.".into())]),
        };

        let mime = infer::get(&data)
            .map(|t| t.mime_type())
            .unwrap_or("application/octet-stream");

        let mut details = String::new();
        let mut v = Vec::new();

        if mime.starts_with("image/") {
            if let Ok(reader) =
                image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format()
            {
                if let Ok((w, h)) = reader.into_dimensions() {
                    details = format!("Dimensions: {}x{}\n", w, h);
                }
                match ty {
                    ResourceType::Image => v.push(MessageContent::ImageRef(uuid, "".into())),
                    ResourceType::Asset => v.push(MessageContent::AssetRef(
                        uuid,
                        "Image in asset store, can not be directly viewed".into(),
                    )),
                }
            }
        } else if mime.starts_with("text/")
            || mime == "application/json"
            || mime == "application/csv"
            || String::from_utf8(data.clone()).is_ok()
        {
            let preview = String::from_utf8_lossy(&data);
            let lines: Vec<&str> = preview.lines().take(10).collect();
            details = format!(
                "Head (First 10 lines):\n```\n{}\n...\n```",
                lines.join("\n")
            );
        } else {
            let hex: String = data
                .iter()
                .take(32)
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ");
            details = format!("Hex Head: {}", hex);
        }

        let info = format!(
            "Resource Info:\n- UUID: {}\n- Size: {} bytes\n- Mime: {}\n{}",
            uuid, size, mime, details
        );
        v.push(MessageContent::Text(info));

        Ok(v)
    }
}
