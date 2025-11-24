use std::{str::FromStr, sync::Arc};

use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use uuid::Uuid;

use crate::{MessageContent, Tool, ToolDescription, blob::BlobStorage};

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
        let uuid = Uuid::from_str(&args.img_idx)?;
        // TODO retrive some metadata
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
        let uuid = Uuid::from_str(&args.asset_idx)?;
        // TODO retrive some metadata
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
