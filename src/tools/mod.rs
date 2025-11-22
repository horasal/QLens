use crate::{blob::BlobStorage, schema::*};
use anyhow::Error;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use strum::{Display, EnumIter, EnumString, IntoEnumIterator};
use uuid::Uuid;

mod prompt_template;

mod zoomin;
pub use zoomin::ZoomInTool;

mod bbox;
pub use bbox::BboxDrawTool;

mod image_memo;
pub use image_memo::ImageMemoTool;

mod code_interpreter;
pub use code_interpreter::JsInterpreter;

mod fetch;
pub use fetch::FetchTool;

mod utils;
pub use utils::*;

#[allow(dead_code)]
type ToolTrait = Box<dyn Tool + Send + Sync>;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Display, EnumIter, Serialize, Deserialize,
)]
#[strum(serialize_all = "snake_case")]
pub enum ToolKind {
    #[strum(serialize = "zoom_in")]
    ZoomIn,
    #[strum(serialize = "image_memo")]
    ImageMemo,
    #[strum(serialize = "draw_bbox")]
    DrawBbox,
    #[strum(serialize = "js_interpreter")]
    JsInterpreter,
    #[strum(serialize = "curl")]
    Curl,
}

impl ToolKind {
    pub fn create_tool(&self, image: Arc<dyn BlobStorage>, asset: Arc<dyn BlobStorage>) -> Box<dyn Tool + Send + Sync> {
        match self {
            ToolKind::ZoomIn => Box::new(ZoomInTool::new(image)),
            ToolKind::ImageMemo => Box::new(ImageMemoTool::new(image)),
            ToolKind::DrawBbox => Box::new(BboxDrawTool::new(image)),
            ToolKind::JsInterpreter => Box::new(JsInterpreter::new(image, asset)),
            ToolKind::Curl => Box::new(FetchTool::new(image, asset)),
        }
    }

    pub fn default_tools_str() -> String {
        ToolKind::iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDescription {
    pub name_for_model: String,
    pub name_for_human: String,
    pub description_for_model: String,
    pub parameters: serde_json::Value, // 使用 serde_json::Value 来表示 JSON Schema
    pub args_format: String,           // 例如: "此工具的输入应为JSON对象。"
}

#[async_trait::async_trait]
pub trait Tool {
    fn name(&self) -> String;
    fn description(&self) -> ToolDescription;
    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, Error>;

    fn get_function_description(&self) -> String {
        let desc = self.description();
        let template = "### {name_for_human}\n{name_for_model}: {description_for_model} 输入参数：{parameters} 其他说明：{args_format}\n\n";

        template
            .replace("{name_for_human}", &desc.name_for_human)
            .replace("{name_for_model}", &desc.name_for_model)
            .replace("{description_for_model}", &desc.description_for_model)
            .replace("{parameters}", &desc.parameters.to_string()) // 序列化JSON
            .replace("{args_format}", &desc.args_format)
            .trim()
            .to_string()
    }
}

pub struct ToolSet {
    tools: HashMap<String, Box<dyn Tool + Send + Sync>>,
}

impl std::fmt::Display for ToolSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for k in self.tools.keys() {
            write!(f, "{},", k)?;
        }
        write!(f, "]")
    }
}

pub struct ToolSetBuilder {
    tools: HashMap<String, Box<dyn Tool + Send + Sync>>,
}

impl ToolSetBuilder {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn add_tool(mut self, tool: Box<dyn Tool + Send + Sync>) -> Self {
        let name = tool.name();
        if self.tools.insert(name.clone(), tool).is_some() {
            tracing::warn!("Overwrite tool '{}'", name);
        }
        self
    }

    pub fn build(self) -> ToolSet {
        ToolSet { tools: self.tools }
    }
}

impl ToolSet {
    pub fn builder() -> ToolSetBuilder {
        ToolSetBuilder::new()
    }

    pub fn list_tools(&self) -> Vec<ToolDescription> {
        self.tools.values().map(|v| v.description()).collect()
    }

    pub fn add_tool(&mut self, tool: Box<dyn Tool + Send + Sync>) -> &mut Self {
        let name = tool.name();
        if self.tools.insert(name.clone(), tool).is_some() {
            tracing::warn!("Overwrite tool '{}'", name);
        }
        self
    }

    pub async fn use_tool_async(&self, tool_use: ToolUse) -> (ToolUse, Message) {
        let result_content = match self.tools.get(&tool_use.function_name) {
            None => {
                let error_msg = format!("错误：未找到名为 '{}' 的工具。", tool_use.function_name);
                vec![MessageContent::Text(error_msg)]
            }
            Some(tool) => {
                match tool.call(&tool_use.args).await {
                    Ok(content) => content,
                    Err(e) => {
                        let error_msg = format!("工具 '{}' 执行失败：{}", tool_use.function_name, e);
                        vec![MessageContent::Text(error_msg)]
                    }
                }
            }
        };

        let origin = tool_use.use_id.clone();
        (tool_use,
        Message {
            id: Uuid::new_v4(),
            owner: Role::Tools(origin),
            content: result_content,
            reasoning: vec![],
            tool_use: vec![],
        })
    }

    pub fn system_prompt(&self, lang: whatlang::Lang, parallel_function_calls: bool) -> String {
        let tool_descs = self
            .tools
            .values()
            .map(|tool| tool.get_function_description())
            .collect::<Vec<String>>()
            .join("\n\n");

        let tool_names = self
            .tools
            .values()
            .map(|tool| tool.name())
            .collect::<Vec<String>>()
            .join(",");

        let templates = prompt_template::get_templates(lang);
        let tool_info = templates
            .tool_info_template
            .replace("{tool_descs}", &tool_descs);

        let tool_fmt_string = if parallel_function_calls {
            templates.parallel_call_template
        } else {
            templates.single_call_template
        };
        let tool_fmt = tool_fmt_string
            .replace("{tool_names}", &tool_names)
            .replace("{FN_NAME}", FN_NAME)
            .replace("{FN_ARGS}", FN_ARGS)
            .replace("{FN_RESULT}", FN_RESULT)
            .replace("{FN_EXIT}", FN_EXIT);
        let assistant_prompt = templates.assistant_desc_template.replace(
            "{CURRENT_DATE}",
            &chrono::Local::now().format("%Y-%m-%d").to_string(),
        );

        format!(r##"{}\n{}\n\n{}"##, assistant_prompt, tool_info, tool_fmt)
    }
}

pub const FN_TAG: &str = "✿";
pub const FN_MAX_LEN: usize = FN_NAME.len() * 2 + 6;
pub const FN_NAME: &str = "✿FUNCTION✿";
pub const FN_ARGS: &str = "✿ARGS✿";
pub const FN_RESULT: &str = "✿RESULT✿";
pub const FN_EXIT: &str = "✿RETURN✿";
pub const FN_RAWHTML: &str = "✿RAWHTML✿";
pub const FN_RAWSVG: &str = "✿RAWSVG✿";
pub const FN_STOP_WORDS: [&str; 4] = [FN_NAME, FN_ARGS, FN_RESULT, FN_EXIT];

#[test]
fn test_builder() {
    let db = sled::Config::new()
        .temporary(true)
        .path("./tmp")
        .open()
        .unwrap();
    let tree = Arc::new(db.open_tree("image").unwrap());
    let zoom_tool = Box::new(ZoomInTool::new(tree.clone()));
    let bbox_tool = Box::new(BboxDrawTool::new(tree.clone()));
    let js_tool = Box::new(JsInterpreter::new(tree.clone() ,tree.clone()));
    let curl_tool = Box::new(FetchTool::new(tree.clone(), tree.clone()));
    let mem_tool = Box::new(ImageMemoTool::new(tree.clone()));
    let toolset = ToolSet::builder()
        .add_tool(zoom_tool)
        .add_tool(bbox_tool)
        .add_tool(js_tool)
        .add_tool(curl_tool)
        .add_tool(mem_tool)
        .build();
    println!("{}", toolset.system_prompt(whatlang::Lang::Cmn, false))
}
