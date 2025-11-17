use anyhow::{anyhow};
use image::{self, Rgba, RgbaImage};
use resvg::tiny_skia;
use resvg::usvg;
use schemars::JsonSchema;
use schemars::schema_for;
use serde::Deserialize;
use std::cmp::max;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

use crate::parse_tool_args;
use crate::tools::FONT_DATA;
use crate::tools::utils::save_image_to_db;
use crate::{MessageContent, Tool, ToolDescription};

const MEMO_KEY: &str = "current_session_memo_uuid";
const DEFAULT_WIDTH: u32 = 1024;
const DEFAULT_HEIGHT: u32 = 1024;

#[derive(Deserialize, JsonSchema)]
struct ImageMemoWriteArgs {
    content: WriteContent,
    #[schemars(description = "Optional label for this operation.")]
    label: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
enum WriteContent {
    #[schemars(description = "Draw SVG content onto the memo.")]
    SVG(SvgMemo),
    #[schemars(description = "Copy a region from another image (local UUID) onto the memo.")]
    CopyImage(ImageMemoCopy),
}

#[derive(Deserialize, JsonSchema)]
struct SvgMemo {
    #[schemars(description = "SVG string content.")]
    svg: String,
    #[schemars(
        description = "Target bounding box [x1, y1, x2, y2] on the memo. The SVG will be resized to fit this box.",
    )]
    target_bbox: [f64; 4],
}

#[derive(Deserialize, JsonSchema)]
struct ImageMemoCopy {
    #[schemars(description = "The local UUID of the source image.")]
    img_idx: String,
    #[schemars(description = "Source region [x1, y1, x2, y2] to copy from.")]
    source_bbox: [f64; 4],
    #[schemars(description = "Target region [x1, y1, x2, y2] on the memo to paste to.")]
    target_bbox: [f64; 4],
}

#[derive(Deserialize, JsonSchema)]
enum ImageMemoArgs {
    #[schemars(description = "Read the current memo.")]
    Read {
        #[schemars(description = "Optional bounding box [x1, y1, x2, y2] to read. If omitted, reads full memo.")]
        bbox: Option<[f64; 4]>,
    },
    #[schemars(description = "Write content (SVG or Image) to the memo.")]
    Write(ImageMemoWriteArgs),
    #[schemars(description = "Clear the memo (reset to blank).")]
    Clear,
}

pub struct ImageMemoTool {
    db: sled::Tree,
}

impl ImageMemoTool {
    pub fn new(ctx: sled::Tree) -> Self {
        Self { db: ctx }
    }

    fn get_current_memo(&self) -> Result<RgbaImage, anyhow::Error> {
        if let Some(uuid_bytes) = self.db.get(MEMO_KEY)? {
            let uuid_str = String::from_utf8(uuid_bytes.to_vec())?;
            let uuid = Uuid::from_str(&uuid_str)?;
            if let Some(img_data) = self.db.get(uuid.as_bytes())? {
                let img = image::load_from_memory(&img_data)?.to_rgba8();
                return Ok(img);
            }
        }
        Ok(RgbaImage::from_pixel(DEFAULT_WIDTH, DEFAULT_HEIGHT, Rgba([255, 255, 255, 255])))
    }

    fn save_memo(&self, img: &RgbaImage) -> Result<Uuid, anyhow::Error> {
        let mut output_buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut output_buf);
        img.write_to(&mut cursor, image::ImageFormat::Png)?;

        let uuid = save_image_to_db(&self.db, &output_buf)?;
        self.db.insert(MEMO_KEY, uuid.to_string().as_bytes())?;
        Ok(uuid)
    }
}

impl Tool for ImageMemoTool {
    fn name(&self) -> String {
        "image_memo".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "image_memo".to_string(),
            name_for_human: "Visual Notebook (image_memo)".to_string(),
            description_for_model: r##"A persistent visual notebook.
Use this to:
1. Organize thoughts by drawing diagrams (SVG).
2. Collect evidence by clipping parts of images (CopyImage).
3. Layout information visually for complex reasoning.
The memo persists across tool calls in this session.
IMPORTANT: Use coordinate format [x1, y1, x2, y2] for all bounding boxes.
"##.to_string(),
            parameters: serde_json::to_value(schema_for!(ImageMemoArgs)).unwrap(),
            args_format: "YAML或JSON".to_string(),
        }
    }

    fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let args: ImageMemoArgs = parse_tool_args(args)?;

        match args {
            ImageMemoArgs::Read { bbox } => {
                let mut memo = self.get_current_memo()?;
                if let Some(rect) = bbox {
                    let x = rect[0] as u32;
                    let y = rect[1] as u32;
                    let w = max(1, (rect[2] - rect[0]) as u32);
                    let h = max(1, (rect[3] - rect[1]) as u32);

                    let (mw, mh) = memo.dimensions();
                    if x + w > mw || y + h > mh {
                         return Ok(vec![MessageContent::Text("Error: Read bbox out of bounds.".to_string())]);
                    }
                    let sub_img = image::imageops::crop(&mut memo, x, y, w, h).to_image();
                    let temp_uuid = save_image_to_db(&self.db, &image_to_bytes(&sub_img)?)?;
                    return Ok(vec![MessageContent::ImageRef(temp_uuid, "Memo View (Cropped)".to_string())]);
                }

                // Read 全图时，依然保存一次 current memo 确保有最新的 uuid（虽然内容没变）
                // 这个UUID跟Chat Session是关联的，chat被删除后就会自动消失，不需要担心空间问题
                let uuid = self.save_memo(&memo)?;
                Ok(vec![MessageContent::ImageRef(uuid, "Current Memo".to_string())])
            },
            ImageMemoArgs::Clear => {
                let empty = RgbaImage::from_pixel(DEFAULT_WIDTH, DEFAULT_HEIGHT, Rgba([255, 255, 255, 255]));
                let uuid = self.save_memo(&empty)?;
                Ok(vec![MessageContent::Text(format!("Memo cleared. New UUID: {}", uuid))])
            },
            ImageMemoArgs::Write(write_args) => {
                let mut memo = self.get_current_memo()?;

                let (req_w, req_h) = match &write_args.content {
                    WriteContent::SVG(s) => (s.target_bbox[2] as u32, s.target_bbox[3] as u32),
                    WriteContent::CopyImage(c) => (c.target_bbox[2] as u32, c.target_bbox[3] as u32),
                };

                let (curr_w, curr_h) = memo.dimensions();
                if req_w > curr_w || req_h > curr_h {
                    let new_w = max(curr_w, req_w);
                    let new_h = max(curr_h, req_h);
                    let mut new_memo = RgbaImage::from_pixel(new_w, new_h, Rgba([255, 255, 255, 255]));
                    image::imageops::overlay(&mut new_memo, &memo, 0, 0);
                    memo = new_memo;
                }

                match write_args.content {
                    WriteContent::SVG(svg_args) => {
                        let x1 = svg_args.target_bbox[0];
                        let y1 = svg_args.target_bbox[1];
                        let w = (svg_args.target_bbox[2] - x1) as u32;
                        let h = (svg_args.target_bbox[3] - y1) as u32;

                        let svg_img = render_svg_to_image(&svg_args.svg, w, h)?;
                        image::imageops::overlay(
                            &mut memo,
                            &svg_img,
                            x1 as i64,
                            y1 as i64
                        );
                    },
                    WriteContent::CopyImage(copy_args) => {
                        let src_uuid = Uuid::from_str(&copy_args.img_idx)?;
                        if let Some(src_data) = self.db.get(src_uuid.as_bytes())? {
                            let mut src_img = image::load_from_memory(&src_data)?.to_rgba8();

                            let sx = copy_args.source_bbox[0] as u32;
                            let sy = copy_args.source_bbox[1] as u32;
                            let sw = (copy_args.source_bbox[2] - copy_args.source_bbox[0]) as u32;
                            let sh = (copy_args.source_bbox[3] - copy_args.source_bbox[1]) as u32;

                            let dw = (copy_args.target_bbox[2] - copy_args.target_bbox[0]) as u32;
                            let dh = (copy_args.target_bbox[3] - copy_args.target_bbox[1]) as u32;

                            let crop = image::imageops::crop(&mut src_img, sx, sy, sw, sh).to_image();

                            let resized = image::imageops::resize(
                                &crop,
                                dw,
                                dh,
                                image::imageops::FilterType::Lanczos3
                            );

                            image::imageops::overlay(
                                &mut memo,
                                &resized,
                                copy_args.target_bbox[0] as i64,
                                copy_args.target_bbox[1] as i64
                            );
                        } else {
                            return Err(anyhow!("Source image not found"));
                        }
                    }
                }

                let uuid = self.save_memo(&memo)?;
                Ok(vec![MessageContent::ImageRef(
                    uuid,
                    write_args.label.unwrap_or("Memo Updated".to_string())
                )])
            }
        }
    }
}

fn render_svg_to_image(svg_data: &str, target_w: u32, target_h: u32) -> Result<RgbaImage, anyhow::Error> {
    let mut font_db = usvg::fontdb::Database::new();
    font_db.load_font_data(FONT_DATA.to_vec());

    let usvg_options = usvg::Options {
        fontdb: Arc::new(font_db),
        font_family: "MapleMono-NF-CN-Regular".into(),
        ..Default::default()
    };

    let tree = usvg::Tree::from_str(svg_data, &usvg_options)?;

    let size = tree.size();

    let fit_w = if size.width() > 0.0 { size.width() } else { target_w as f32 };
    let fit_h = if size.height() > 0.0 { size.height() } else { target_h as f32 };

    let scale_x = target_w as f32 / fit_w;
    let scale_y = target_h as f32 / fit_h;

    let transform = tiny_skia::Transform::from_scale(scale_x, scale_y);

    let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)
        .ok_or(anyhow!("Failed to create pixmap"))?;

    resvg::render(
        &tree,
        transform,
        &mut pixmap.as_mut(),
    );

    let img = RgbaImage::from_raw(target_w, target_h, pixmap.data().to_vec())
        .ok_or(anyhow!("Failed to convert pixmap to image"))?;

    Ok(img)
}


fn image_to_bytes(img: &RgbaImage) -> Result<Vec<u8>, anyhow::Error> {
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, image::ImageFormat::Png)?;
    Ok(buf)
}
