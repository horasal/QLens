use crate::blob::BlobStorage;
use crate::schema::MessageContent;
use crate::tools::{FONT_DATA, Tool, ToolDescription};
use crate::{ImageResizer, parse_tool_args};
use ab_glyph::PxScale;
use anyhow::Result;
use image::{GenericImageView, Pixel, Rgba, RgbaImage, imageops};
use imageproc::drawing::{draw_hollow_rect_mut, draw_text_mut, text_size};
use imageproc::rect::Rect;
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Cursor;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize, JsonSchema)]
struct BboxDrawArgs {
    #[schemars(description = "list of bounding boxes")]
    bboxes: Vec<Bbox>,

    #[schemars(description = "The local uuid of the image to be drawn on")]
    img_idx: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Bbox {
    #[schemars(
        description = "The bounding box of the region as [x1 ,y1, x2, y2], values are cornerrelative coordinates in [0,1000]",
        length(equal = 4)
    )]
    bbox_2d: [f64; 4],
    #[schemars(description = "The name or label of the object")]
    label: Option<String>,
}

pub struct BboxDrawTool {
    db: Arc<dyn BlobStorage>,
}

impl BboxDrawTool {
    pub fn new(ctx: Arc<dyn BlobStorage>) -> Self {
        Self { db: ctx }
    }
}

#[async_trait::async_trait]
impl Tool for BboxDrawTool {
    fn name(&self) -> String {
        "image_draw_bbox_2d_tool".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "image_draw_bbox_2d_tool".to_string(),
            name_for_human: "图像标记工具(bbox marker tool)".to_string(),
            description_for_model: "Draw boxes on specific regions of an image based on given bounding boxes (bbox_2d) and an optional object label".to_string(),
            parameters: serde_json::to_value(schema_for!(BboxDrawArgs)).unwrap(),
            args_format: "必须是一个YAML或JSON对象，其中图片必须用其对应的UUID指代。".to_string(),
        }
    }
    async fn call(&self, args: &str) -> Result<Vec<MessageContent>> {
        let args: BboxDrawArgs = parse_tool_args(args)?;
        let id = Uuid::from_str(&args.img_idx)?;
        let image = self
            .db
            .get(id)?
            .ok_or(anyhow::anyhow!("Image does not exist"))?;
        let cropped_img = draw_bboxes_rgba(&image, &args.bboxes)?;
        let uuid = self.db.save(&cropped_img)?;
        Ok(vec![MessageContent::ImageRef(uuid, "".to_string())])
    }
}

const COLOR_MAP: &[Rgba<u8>] = &[
    Rgba([255, 0, 0, 255]),   // 1. 红色 (Red)
    Rgba([0, 255, 0, 255]),   // 2. 绿色 (Green)
    Rgba([0, 0, 255, 255]),   // 3. 蓝色 (Blue)
    Rgba([255, 255, 0, 255]), // 4. 黄色 (Yellow)
    Rgba([0, 255, 255, 255]), // 5. 青色 (Cyan)
    Rgba([255, 0, 255, 255]), // 6. 品红 (Magenta)
    Rgba([255, 128, 0, 255]), // 7. 橙色 (Orange)
    Rgba([128, 0, 255, 255]), // 8. 紫色 (Purple)
    Rgba([0, 128, 0, 255]),   // 9. 深绿 (Dark Green)
    Rgba([0, 128, 128, 255]), // 10. 蓝绿色 (Teal)
    Rgba([128, 128, 0, 255]), // 11. 橄榄色 (Olive)
    Rgba([255, 0, 128, 255]), // 12. 玫瑰红 (Rose)
    Rgba([255, 165, 0, 255]), // 13. 亮橙色 (Bright Orange)
    Rgba([128, 0, 0, 255]),   // 14. 栗色 (Maroon)
    Rgba([0, 0, 128, 255]),   // 15. 海军蓝 (Navy)
    Rgba([170, 110, 40, 255]), // 16. 棕色 (Brown)
    Rgba([250, 190, 212, 255]), // 17. 粉色 (Pink)
    Rgba([70, 240, 240, 255]), // 18. 亮天蓝 (Light Sky Blue)
    Rgba([245, 130, 48, 255]), // 19. 杏色 (Apricot)
    Rgba([128, 128, 128, 255]), // 20. 灰色 (Gray)
];

const TEXT_COLOR: Rgba<u8> = Rgba([255, 255, 255, 255]); // 纯白色
const TEXT_BG_ALPHA: u8 = 128;

fn draw_bboxes_rgba(image_data: &[u8], bboxes: &[Bbox]) -> Result<Vec<u8>, anyhow::Error> {
    let image = image::load_from_memory(image_data)?;
    let (width, height) = image.dimensions();

    let (width, height, image) = if width < 128 || height < 128 {
        let resizer = ImageResizer::new(8, 262144, 12845056);
        let (new_w, new_h) = resizer.smart_resize(width, height);
        let resized_image = image.resize_exact(new_w, new_h, imageops::FilterType::Lanczos3);
        (new_w, new_h, resized_image)
    } else {
        (width, height, image)
    };

    let mut image_buffer: RgbaImage = image.to_rgba8();

    let font = ab_glyph::FontRef::try_from_slice(FONT_DATA)?;

    let font_size = 40.0;
    let border_thickness = 3_i32;
    let text_padding = 4_i32; // 文本在背景矩形内的边距

    let mut label_to_color = HashMap::new();
    let mut next_color_index = 0;

    for item in bboxes {
        let color = *label_to_color
            .entry(item.label.clone().unwrap_or_default())
            .or_insert_with(|| {
                let color = COLOR_MAP[next_color_index % COLOR_MAP.len()];
                next_color_index += 1;
                color
            });

        // 坐标转换
        let bbox = &item.bbox_2d;
        let x1 = ((bbox[0] / 1000.0) * width as f64) as i32;
        let y1 = ((bbox[1] / 1000.0) * height as f64) as i32;
        let x2 = ((bbox[2] / 1000.0) * width as f64) as i32;
        let y2 = ((bbox[3] / 1000.0) * height as f64) as i32;

        if (x2 - x1) <= 0 || (y2 - y1) <= 0 {
            continue;
        }

        for i in 0..border_thickness {
            let rect = Rect::at(x1 + i, y1 + i).of_size(
                (x2 - x1 - 2 * i).max(0) as u32,
                (y2 - y1 - 2 * i).max(0) as u32,
            );
            if rect.width() > 0 && rect.height() > 0 {
                draw_hollow_rect_mut(&mut image_buffer, rect, color);
            }
        }

        if let Some(ref text) = item.label {
            let (text_w, text_h) = text_size(PxScale::from(font_size), &font, text);

            let bg_w = (text_w + text_padding as u32 * 2) as u32;
            let bg_h = (text_h + text_padding as u32 * 2) as u32;

            let try_y_above = y1 - bg_h as i32;
            let (bg_x, bg_y, text_x, text_y);

            if try_y_above < 0 {
                // 空间不足，画在 Bbox 内部左上角
                bg_x = x1;
                bg_y = y1;
                text_x = x1 + text_padding;
                text_y = y1 + text_padding;
            } else {
                // 默认，画在 Bbox 外部左上角
                bg_x = x1;
                bg_y = try_y_above;
                text_x = x1 + text_padding;
                text_y = try_y_above + text_padding;
            }

            let bg_color = Rgba([color[0], color[1], color[2], TEXT_BG_ALPHA]);

            // 裁剪背景矩形以确保在图像边界内
            let bg_x_start = bg_x.max(0) as u32;
            let bg_y_start = bg_y.max(0) as u32;
            let bg_x_end = (bg_x + bg_w as i32).min(width as i32) as u32;
            let bg_y_end = (bg_y + bg_h as i32).min(height as i32) as u32;

            if bg_x_start < bg_x_end && bg_y_start < bg_y_end {
                for y in bg_y_start..bg_y_end {
                    for x in bg_x_start..bg_x_end {
                        let p = image_buffer.get_pixel_mut(x, y);
                        p.blend(&bg_color);
                    }
                }
            }

            // 确保文本起始点在图像内
            if text_x >= 0 && text_y >= 0 && text_x < width as i32 && text_y < height as i32 {
                draw_text_mut(
                    &mut image_buffer,
                    TEXT_COLOR,
                    text_x,
                    text_y,
                    font_size,
                    &font,
                    text,
                );
            }
        }
    }

    let mut output_buffer = Vec::new();
    let mut cursor = Cursor::new(&mut output_buffer);
    image_buffer.write_to(&mut cursor, image::ImageFormat::Png)?;

    Ok(output_buffer)
}
