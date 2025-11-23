use std::{str::FromStr, sync::Arc};

use anyhow::anyhow;
use resvg::{tiny_skia, usvg};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    MessageContent, Tool, ToolDescription, blob::BlobStorage, get_usvg_options, parse_tool_args,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoState {
    // 画布基础尺寸 (可以动态增长)
    pub width: u32,
    pub height: u32,
    // 自动布局的游标位置 (y坐标)
    pub cursor_y: u32,
    // 图层列表 (从底向上渲染)
    pub layers: Vec<Layer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: Uuid,
    pub kind: LayerKind,
    // 图层在画布上的位置和尺寸
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerKind {
    ImageRef(Uuid),
    SvgContent(String),
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ImageMemoArgs {
    Read {
        #[serde(default = "default_true")]
        grid: bool,
    },
    Add {
        content: MemoContentInput,
        layout: LayoutMode,
    },
    Undo,
    Clear,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoContentInput {
    #[schemars(description = "Image UUID.")]
    Image(String),
    #[schemars(description = "SVG string.")]
    Svg(String),
    #[schemars(description = "Raw text (auto-wrap).")]
    Text(String),
}

#[derive(Deserialize, JsonSchema)]
pub enum LayoutMode {
    #[schemars(description = "Auto-stack at bottom.")]
    Append { height: Option<u32> },

    #[schemars(description = "[x1,y1,x2,y2] (Normalized 0-1000)")]
    Absolute { bbox: [f64; 4] },
}

fn default_true() -> bool {
    true
}

pub struct ImageMemoTool {
    image_db: Arc<dyn BlobStorage>,
    memo_db: Arc<dyn BlobStorage>,
}

#[async_trait::async_trait]
impl Tool for ImageMemoTool {
    fn name(&self) -> String {
        "Memo".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "Memo".to_string(),
            name_for_human: "笔记工具".to_string(),
            description_for_model: r##"Visual Scratchpad (Persistent).
**Usage:**
1. **Complex Reasoning**: Draw diagrams/relations.
2. **Comparison**: Copy images side-by-side.
3. **State**: Save intermediate results.
**Note:** Context is persistent across turns."##.to_string(),
            parameters: serde_json::to_value(schema_for!(ImageMemoArgs)).unwrap(),
            args_format: "JSON.".to_string(),
        }
    }

    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let args: ImageMemoArgs = parse_tool_args(args)?;
        let mut state = self.get_state()?;

        match args {
            ImageMemoArgs::Add { content, layout } => {
                let (kind, src_w, src_h) = match content {
                    MemoContentInput::Svg(s) => (LayerKind::SvgContent(s), 200, 200),
                    MemoContentInput::Image(uuid_str) => {
                        let uuid = Uuid::from_str(&uuid_str)?;
                        let bytes = self.image_db.get(uuid)?.ok_or(anyhow!("Img not found"))?;
                        let meta = image::load_from_memory(&bytes)?;
                        (LayerKind::ImageRef(uuid), meta.width(), meta.height())
                    }
                    MemoContentInput::Text(txt) => {
                        let (svg, h) = wrap_text_to_svg(&txt, state.width); // 宽度铺满画布
                        (LayerKind::SvgContent(svg), state.width, h)
                    }
                };

                let (x, y, w, h) = match layout {
                    LayoutMode::Append { height } => {
                        let target_h = height.unwrap_or(src_h);

                        let scale = target_h as f32 / src_h as f32;
                        let final_w = (src_w as f32 * scale) as u32;

                        let y = state.cursor_y;

                        state.cursor_y += target_h + 20; // +20 padding

                        (20, y as i32 + 20, final_w, target_h) // x=20 padding
                    }
                    LayoutMode::Absolute { bbox } => {
                        let abs_box = to_abs_bbox(bbox, state.width, state.height);
                        (abs_box[0] as i32, abs_box[1] as i32, abs_box[2] - abs_box[0], abs_box[3] - abs_box[1])
                    }
                };

                let required_h = (y + h as i32) as u32 + 50;
                if required_h > state.height {
                    state.height = required_h;
                }

                state.layers.push(Layer {
                    id: Uuid::new_v4(),
                    kind,
                    x,
                    y,
                    width: w,
                    height: h,
                });

                self.save_state(&state)?;

                Ok(vec![MessageContent::Text("Layer added.".into())])
            }

            ImageMemoArgs::Read { grid } => {
                let png_data = self.render_view(&state, grid)?;
                let uuid = self.image_db.save(&png_data)?;
                Ok(vec![
                    MessageContent::Text("✅ Read Success".to_string()),
                    MessageContent::ImageRef(uuid, "Memo Snapshot".into())])
            }

            ImageMemoArgs::Undo => {
                if let Some(l) = state.layers.pop() {
                    state.cursor_y = state
                        .layers
                        .iter()
                        .map(|l| l.y as u32 + l.height)
                        .max()
                        .unwrap_or(0)
                        + 20;

                    self.save_state(&state)?;
                    Ok(vec![MessageContent::Text("Undone last action.".into())])
                } else {
                    Ok(vec![MessageContent::Text("Nothing to undo.".into())])
                }
            }

            ImageMemoArgs::Clear => {
                self.memo_db.remove(b"current")?;
                Ok(vec![MessageContent::Text("Memo cleared.".into())])
            }
        }
    }
}

impl ImageMemoTool {
    pub fn new(image_db: Arc<dyn BlobStorage>, memo_db: Arc<dyn BlobStorage>) -> Self {
        Self {
            image_db: image_db,
            memo_db: memo_db,
        }
    }

    fn get_state(&self) -> Result<MemoState, anyhow::Error> {
        if let Some(data) = self.memo_db.get_by_key(b"current")? {
            Ok(serde_json::from_slice(&data)?)
        } else {
            Ok(MemoState {
                width: 1024,
                height: 1024,
                cursor_y: 0,
                layers: vec![],
            })
        }
    }

    fn save_state(&self, state: &MemoState) -> Result<(), anyhow::Error> {
        let data = serde_json::to_vec(state)?;
        self.memo_db.insert(b"current", &data)?;
        Ok(())
    }

    fn render_view(&self, state: &MemoState, show_grid: bool) -> Result<Vec<u8>, anyhow::Error> {
        let header_height = 40;
        let total_height = state.height + header_height;
        let mut canvas = tiny_skia::Pixmap::new(state.width, total_height)
            .ok_or(anyhow!("Failed to create canvas"))?;
        canvas.fill(tiny_skia::Color::WHITE);

        let content_transform = tiny_skia::Transform::from_translate(0.0, header_height as f32);

        for layer in &state.layers {
            let transform = content_transform.pre_translate(layer.x as f32, layer.y as f32);

            match &layer.kind {
                LayerKind::SvgContent(svg_data) => {
                    let usvg_options = get_usvg_options();
                    let tree = usvg::Tree::from_str(svg_data, &usvg_options)?;

                    let size = tree.size();
                    let scale_x = layer.width as f32 / size.width();
                    let scale_y = layer.height as f32 / size.height();
                    let scale = scale_x.min(scale_y);

                    let render_ts = transform.post_scale(scale, scale);
                    resvg::render(&tree, render_ts, &mut canvas.as_mut());
                }
                LayerKind::ImageRef(uuid) => {
                    if let Some(img_bytes) = self.image_db.get(*uuid)? {
                        let src_pixmap = tiny_skia::Pixmap::decode_png(&img_bytes)?;

                        let scale_x = layer.width as f32 / src_pixmap.width() as f32;
                        let scale_y = layer.height as f32 / src_pixmap.height() as f32;

                        let render_ts = transform.post_scale(scale_x, scale_y);

                        canvas.draw_pixmap(
                            0,
                            0,
                            src_pixmap.as_ref(),
                            &tiny_skia::PixmapPaint::default(),
                            render_ts,
                            None,
                        );
                    }
                }
            }
        }
        let mut header_paint = tiny_skia::Paint::default();
        header_paint.set_color_rgba8(240, 240, 240, 255); // 浅灰色背景
        canvas.fill_rect(
            tiny_skia::Rect::from_xywh(0.0, 0.0, state.width as f32, header_height as f32).unwrap(),
            &header_paint,
            tiny_skia::Transform::default(),
            None,
        );

        let header_svg = format!(
            r###"<svg><text x="10" y="25" font-family="sans-serif" font-size="16" fill="#555" font-weight="bold">Visual Notebook (Session ID: {}) - Cursor Y: {}</text></svg>"###,
            "Current", state.cursor_y
        );
        let tree = usvg::Tree::from_str(&header_svg, &get_usvg_options())?;
        resvg::render(&tree, tiny_skia::Transform::default(), &mut canvas.as_mut());
        if show_grid {
            self.draw_grid(&mut canvas)?;
        }

        Ok(canvas.encode_png()?)
    }

    fn draw_grid(&self, canvas: &mut tiny_skia::Pixmap) -> Result<(), anyhow::Error> {
        let width = canvas.width();
        let height = canvas.height();
        let step = 100; // 网格间距

        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);

        svg.push_str(
            r#"
            <defs>
                <style>
                    .grid { stroke: #e0e0e0; stroke-width: 1; }
                    .axis { stroke: #ff0000; stroke-width: 1; stroke-dasharray: 4; }
                    .label { font-family: monospace; font-size: 10px; fill: #999; opacity: 0.7; }
                </style>
            </defs>
        "#,
        );

        for x in (0..width).step_by(step) {
            svg.push_str(&format!(
                r#"<line x1="{}" y1="0" x2="{}" y2="{}" class="grid" />"#,
                x, x, height
            ));
            svg.push_str(&format!(
                r#"<text x="{}" y="12" class="label">{}</text>"#,
                x + 2,
                x
            ));
        }

        for y in (0..height).step_by(step) {
            svg.push_str(&format!(
                r#"<line x1="0" y1="{}" x2="{}" y2="{}" class="grid" />"#,
                y, width, y
            ));
            svg.push_str(&format!(
                r#"<text x="2" y="{}" dy="-2" class="label">{}</text>"#,
                y,
                y
            ));
        }

        svg.push_str("</svg>");

        let tree = usvg::Tree::from_str(&svg, &get_usvg_options())?;
        resvg::render(&tree, tiny_skia::Transform::default(), &mut canvas.as_mut());

        Ok(())
    }
}

fn wrap_text_to_svg(text: &str, width: u32) -> (String, u32) {
    let line_height = 30;
    let padding = 20;

    // 如果需要完美排版，需要引入 text_layout 库，这里为了不引入新依赖做简易版
    let max_chars_per_line = (width - padding * 2) / 12;

    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut current_line = String::new();
        let mut width_counter = 0;

        for c in paragraph.chars() {
            let char_width = if c.is_ascii() { 1 } else { 2 };
            if width_counter + char_width > max_chars_per_line {
                lines.push(current_line);
                current_line = String::new();
                width_counter = 0;
            }
            current_line.push(c);
            width_counter += char_width;
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }

    let height = (lines.len() as u32 * line_height) + padding * 2;

    let mut svg_content =
        String::from(r#"<g font-family="monospace" font-size="24" fill="black">"#);

    for (i, line) in lines.iter().enumerate() {
        let y = padding + (i as u32 + 1) * line_height - 5;
        // 注意：需要对 line 进行 XML 转义 (replace < with &lt; 等)，此处简略
        let safe_line = line
            .replace("&", "&amp;")
            .replace("<", "&lt;")
            .replace(">", "&gt;");
        svg_content.push_str(&format!(
            r#"<text x="{}" y="{}">{}</text>"#,
            padding, y, safe_line
        ));
    }
    svg_content.push_str("</g>");

    let svg = format!(
        r###"<svg width="{}" height="{}" viewBox="0 0 {} {}" xmlns="http://www.w3.org/2000/svg">
            <rect width="100%" height="100%" fill="#f0f0f0" stroke="#ccc" stroke-width="1"/>
            {}
           </svg>"###,
        width, height, width, height, svg_content
    );

    (svg, height)
}

pub fn normalize_to_pixel(rel_val: f64, max_pixel: u32) -> u32 {
    let ratio = rel_val.clamp(0.0, 1000.0) / 1000.0;
    (ratio * max_pixel as f64).round() as u32
}

/// 处理 Bbox [x1, y1, x2, y2] 的转换
pub fn to_abs_bbox(rel_bbox: [f64; 4], w: u32, h: u32) -> [u32; 4] {
    [
        normalize_to_pixel(rel_bbox[0], w),
        normalize_to_pixel(rel_bbox[1], h),
        normalize_to_pixel(rel_bbox[2], w),
        normalize_to_pixel(rel_bbox[3], h),
    ]
}
