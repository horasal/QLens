use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
};

use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig};
use chat_ui::*;
use clap::Parser;
use futures::StreamExt;
use tracing::Level;

#[derive(clap::Parser)]
struct Argument {
    #[clap(short, long)]
    text: Option<String>,
    #[clap(short, long)]
    image: Option<PathBuf>,
    #[clap(short, long, default_value = "http://127.0.0.1:8080")]
    base_url: String,
    #[clap(short, long, default_value = "")]
    api_key: String,
    #[clap(short, long, default_value = ".")]
    output: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();
    let args = Argument::parse();
    let config = OpenAIConfig::new()
        .with_api_base(args.base_url)
        .with_api_key(args.api_key);
    let client = Client::with_config(config);
    tracing::info!("Created openai client.");

    let db = sled::Config::new()
        .temporary(false)
        .use_compression(true)
        .path("chat_data")
        .open()?;
    let image_db = db.open_tree("image")?;
    let history_db = db.open_tree("history")?;
    tracing::info!("DB started.");
    let zoom_tool = Box::new(ZoomInTool::new(image_db.clone()));
    let bbox_tool = Box::new(BboxDrawTool::new(image_db.clone()));
    let toolset = ToolSet::builder()
        .add_tool(zoom_tool)
        .add_tool(bbox_tool)
        .build();
    let llm = LLMProvider::new(client, history_db, image_db, toolset)?;
    tracing::info!("LLMProvider created.");

    let entry = llm.new_chat()?;
    let id = entry.id;
    tracing::info!("Create Chat -> {}@{}.", id, entry.date);
    let mut v = Vec::new();
    if let Some(ref p) = args.image {
        let image = {
            let mut file = File::open(p)?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            buf
        };
        let img_uuid = llm.save_image(&image)?;
        tracing::info!("Upload Image -> {}.", img_uuid);
        v.push(MessageContent::ImageRef(img_uuid, "".to_string()));
    }
    if let Some(ref s) = args.text {
        v.push(MessageContent::Text(s.to_owned()));
    }
    if v.is_empty() {
        return Err(anyhow!("No input"));
    }
    let stream = llm.send_chat_message(id, v, LLMConfig::default()).await?;

    tokio::pin!(stream);

    while let Some(event) = stream.next().await {
        let event = event?;
        match event {
            ChatEvent::ContentDelta(d) => print!("{}", d),
            ChatEvent::ReasoningDelta(d) => print!("{}", d),
            ChatEvent::ToolDelta(s) => print!("{}", s),
            ChatEvent::ToolCall(tool) => {
                println!("\nUseTool:{}", tool.function_name)
            }
            ChatEvent::ToolResult { tool_use, result } => {
                println!("Tool {} returns:", tool_use.function_name);
                for v in result {
                    println!("\t{}", v);
                    match v {
                        MessageContent::ImageBin(b, id, _) => {
                            let mut f = std::fs::File::create(format!(
                                "{}/{}.jpg",
                                args.output,
                                id.to_string()
                            ))?;
                            println!("\tTool returns image -> {}/{}.jpg", args.output, id);
                            f.write_all(&b)?;
                        }
                        _ => {}
                    }
                }
            }
            ChatEvent::StreamEnd {} => {}
        }
    }
    Ok(())
}
