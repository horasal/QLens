use crate::schema::*;
use anyhow::Error;
use std::collections::HashMap;


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

type ToolTrait = Box<dyn Tool + Send + Sync>;

pub fn get_tool<T: AsRef<str>>(value: T, db: sled::Tree) -> Option<ToolTrait> {
    match value.as_ref() {
        "zoom_in" => Some(Box::new(ZoomInTool::new(db))),
        "image_memo" => Some(Box::new(ImageMemoTool::new(db))),
        "draw_bbox" => Some(Box::new(BboxDrawTool::new(db))),
        "js_interpreter" => Some(Box::new(JsInterpreter::new(db))),
        "curl" => Some(Box::new(FetchTool::new(db))),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct ToolDescription {
    pub name_for_model: String,
    pub name_for_human: String,
    pub description_for_model: String,
    pub parameters: serde_json::Value, // 使用 serde_json::Value 来表示 JSON Schema
    pub args_format: String,           // 例如: "此工具的输入应为JSON对象。"
}

pub trait Tool {
    fn name(&self) -> String;
    fn description(&self) -> ToolDescription;
    fn call(&self, args: &str) -> Result<MessageContent, Error>;

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

    pub fn add_tool(&mut self, tool: Box<dyn Tool + Send + Sync>) -> &mut Self {
        let name = tool.name();
        if self.tools.insert(name.clone(), tool).is_some() {
            tracing::warn!("Overwrite tool '{}'", name);
        }
        self
    }

    pub async fn use_tool_async(&self, tool_name: String, args: String) -> Message {
        self.use_tool(&tool_name, &args)
    }

    pub fn use_tool(&self, tool_name: &str, args: &str) -> Message {
        let result_content = match self.tools.get(tool_name) {
            None => {
                // 错误处理：工具未找到
                let error_msg = format!("错误：未找到名为 '{}' 的工具。", tool_name);
                MessageContent::Text(error_msg)
            }
            Some(tool) => {
                // 找到工具，执行它
                match tool.call(args) {
                    // 工具成功执行
                    Ok(content) => content,

                    // 错误处理：工具执行失败
                    Err(e) => {
                        let error_msg = format!("工具 '{}' 执行失败：{}", tool_name, e);
                        MessageContent::Text(error_msg)
                    }
                }
            }
        };

        // 无论成功还是失败，都打包成一个 Message 返回
        // LLM 需要这个结果（无论是数据还是错误信息）来继续下一步
        Message {
            owner: Role::Tools,
            content: vec![result_content],
            reasoning: vec![], // 工具结果没有 reasoning
            tool_use: vec![],  // 这不是一个 "tool_use" 动作，而是 "tool_use" 的结果
        }
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
        let assistant_prompt = templates.assistant_desc_template
            .replace("{CURRENT_DATE}", &chrono::Local::now().format("%Y-%m-%d").to_string());

        format!( r##"{}\n{}\n\n{}"##, assistant_prompt, tool_info, tool_fmt)
    }
}

pub const FN_TAG: &str = "✿";
pub const FN_MAX_LEN: usize = FN_NAME.len() * 2 + 6;
pub const FN_NAME: &str = "✿FUNCTION✿";
pub const FN_ARGS: &str = "✿ARGS✿";
pub const FN_RESULT: &str = "✿RESULT✿";
pub const FN_EXIT: &str = "✿RETURN✿";
pub const FN_STOP_WORDS: [&str; 4] = [FN_NAME, FN_ARGS, FN_RESULT, FN_EXIT];

#[test]
fn test_builder() {
    let db = sled::Config::new()
        .temporary(true)
        .path("./tmp")
        .open()
        .unwrap();
    let zoom_tool = Box::new(ZoomInTool::new(db.open_tree("image").unwrap()));
    let bbox_tool = Box::new(BboxDrawTool::new(db.open_tree("image").unwrap()));
    let toolset = ToolSet::builder()
        .add_tool(zoom_tool)
        .add_tool(bbox_tool)
        .build();
    println!("{}", toolset.system_prompt(whatlang::Lang::Cmn, true))
}
