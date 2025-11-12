use image::{
    DynamicImage, GenericImageView, ImageError, ImageFormat, error::ParameterError, imageops,
};
use std::io::Cursor;
use std::str::FromStr;
use uuid::Uuid;

use crate::schema::MessageContent;
use crate::tools::{Tool, ToolDescription};
use anyhow::{Error, anyhow};
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;

// Tool

#[derive(Deserialize, JsonSchema)]
struct ZoomArgs {
    #[schemars(
        description = "The bounding box of the region to zoom in, as [x1, y1, x2, y2], where (x1, y1) is the top-left corner and (x2, y2) is the bottom-right cornerrelative coordinates.",
        length(equal = 4)
    )]
    bbox_2d: [f64; 4],

    #[schemars(description = "The name or label of the object in the specified bounding box")]
    label: String,

    #[schemars(description = "The uuid of input image")]
    img_idx: String, // 使用更具体的数字类型
}

pub struct ZoomInTool {
    db: sled::Tree,
}

impl ZoomInTool {
    pub fn new(ctx: sled::Tree) -> Self {
        Self { db: ctx }
    }
}

impl Tool for ZoomInTool {
    fn name(&self) -> String {
        "image_zoom_in_tool".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "image_zoom_in_tool".to_string(),
            name_for_human: "图像局部裁切/放大工具(image crop and zoom-in tool)".to_string(),
            description_for_model: "Crop and zoom in on a specific region of an image by cropping it based on a bounding box (bbox) and an optional object label".to_string(),
            parameters: serde_json::to_value(schema_for!(ZoomArgs)).unwrap(),
            args_format: "输入格式必须是JSON，其中图片必须用其UUID指代。".to_string(),
        }
    }
    fn call(&self, args: &str) -> Result<MessageContent, Error> {
        let args: ZoomArgs = serde_json::from_str(args)?;
        let id = Uuid::from_str(&args.img_idx)?;
        let image = self.db.get(id)?.ok_or(anyhow!("Empty Image"))?;
        let bbox = BBox {
            x1: args.bbox_2d[0],
            y1: args.bbox_2d[1],
            x2: args.bbox_2d[2],
            y2: args.bbox_2d[3],
        };
        let cropped_img = image_zoom_in(&image, bbox)?;
        let mut uuid = Uuid::new_v4();
        for _ in 0..10 {
            match self
                .db
                .compare_and_swap(uuid, None::<&[u8]>, Some(cropped_img.clone()))?
            {
                Ok(()) => break,
                Err(_) => {
                    uuid = Uuid::new_v4();
                }
            }
        }
        Ok(MessageContent::ImageRef(uuid, args.label))
    }
}

/// 用于智能调整大小的辅助结构体
/// 移植自 Python 版本的 `smart_resize` 及其辅助函数
struct ImageResizer {
    factor: f64,
    min_pixels: f64,
    max_pixels: f64,
}

impl ImageResizer {
    /// * `factor`: 缩放因子，通常为 32
    /// * `min_pixels`: 最小像素总数
    /// * `max_pixels`: 最大像素总数
    fn new(factor: u32, min_pixels: u64, max_pixels: u64) -> Self {
        Self {
            factor: factor as f64,
            min_pixels: min_pixels as f64,
            max_pixels: max_pixels as f64,
        }
    }

    /// (helper) `round_by_factor` 的 Rust 实现
    fn round_by_factor(&self, number: f64) -> f64 {
        (number / self.factor).round() * self.factor
    }

    /// (helper) `ceil_by_factor` 的 Rust 实现
    fn ceil_by_factor(&self, number: f64) -> f64 {
        (number / self.factor).ceil() * self.factor
    }

    /// (helper) `floor_by_factor` 的 Rust 实现
    fn floor_by_factor(&self, number: f64) -> f64 {
        (number / self.factor).floor() * self.factor
    }

    /// `smart_resize` 的 Rust 实现
    ///
    /// # Arguments
    ///
    /// * `height`: 图像高度
    /// * `width`: 图像宽度
    ///
    /// # Returns
    ///
    /// * `(u32, u32)`: (新的高度, 新的宽度)
    fn smart_resize(&self, height: u32, width: u32) -> (u32, u32) {
        let height_f = height as f64;
        let width_f = width as f64;

        let mut h_bar = self.factor.max(self.round_by_factor(height_f));
        let mut w_bar = self.factor.max(self.round_by_factor(width_f));

        let current_pixels = h_bar * w_bar;

        if current_pixels > self.max_pixels {
            let beta = (height_f * width_f / self.max_pixels).sqrt();
            h_bar = self.floor_by_factor(height_f / beta);
            w_bar = self.floor_by_factor(width_f / beta);
        } else if current_pixels < self.min_pixels {
            let beta = (self.min_pixels / (height_f * width_f)).sqrt();
            h_bar = self.ceil_by_factor(height_f * beta);
            w_bar = self.ceil_by_factor(width_f * beta);
        }

        (h_bar as u32, w_bar as u32)
    }
}

/// 内部用于表示绝对像素坐标（浮点数）的 BBox
#[derive(Debug, Clone, Copy)]
struct AbsolutePixelBBox {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

impl AbsolutePixelBBox {
    fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// `maybe_resize_bbox` 的 Rust 实现
    fn validate_and_resize(self, img_width: u32, img_height: u32) -> Self {
        let img_width_f = img_width as f64;
        let img_height_f = img_height as f64;

        let left = self.x1.max(0.0);
        let top = self.y1.max(0.0);
        let right = self.x2.min(img_width_f);
        let bottom = self.y2.min(img_height_f);

        let height = bottom - top;
        let width = right - left;

        if height < 32.0 || width < 32.0 {
            let center_x = (left + right) / 2.0;
            let center_y = (top + bottom) / 2.0;
            let ratio = 32.0 / height.min(width);

            let new_half_height = (height * ratio * 0.5).ceil();
            let new_half_width = (width * ratio * 0.5).ceil();

            let new_left = (center_x - new_half_width).floor();
            let new_right = (center_x + new_half_width).ceil();
            let new_top = (center_y - new_half_height).floor();
            let new_bottom = (center_y + new_half_height).ceil();

            let new_left = new_left.max(0.0);
            let new_top = new_top.max(0.0);
            let new_right = new_right.min(img_width_f);
            let new_bottom = new_bottom.min(img_height_f);

            let new_height = new_bottom - new_top;
            let new_width = new_right - new_left;

            if new_height > 32.0 && new_width > 32.0 {
                return Self::new(new_left, new_top, new_right, new_bottom);
            }
        }

        Self::new(left, top, right, bottom)
    }

    /// 将浮点 BBox 转换为 `image::crop_imm` 所需的 (x, y, width, height) 整数参数
    ///
    /// PIL 的 `crop((l, t, r, b))` 会对所有坐标执行 floor 操作。
    /// width = floor(r) - floor(l)
    /// height = floor(b) - floor(t)
    fn to_crop_args(&self) -> (u32, u32, u32, u32) {
        let left = self.x1.floor() as u32;
        let top = self.y1.floor() as u32;
        let right = self.x2.floor() as u32;
        let bottom = self.y2.floor() as u32;

        let width = right.saturating_sub(left);
        let height = bottom.saturating_sub(top);

        (left, top, width, height)
    }
}

/// 边界框（BBox）结构体
///
/// 根据 Python 逻辑，这些坐标是相对坐标（0-1000 范围）
#[derive(Debug, Clone, Copy)]
pub struct BBox {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// 将图像（PNG 二进制数据）按给定的 BBox 放大
///
/// # Arguments
///
/// * `image_data`: 原始图像的 PNG 二进制数据 (`&[u8]`)
/// * `bbox`: `BBox` 结构体，包含 (x1, y1, x2, y2) 相对坐标 (0-1000)
///
/// # Returns
///
/// * `Result<Vec<u8>, ImageError>`:
///   - `Ok(Vec<u8>)`: 包含裁剪和缩放后图像的 PNG 二进制数据
///   - `Err(ImageError)`: 如果图像处理（加载、裁剪、保存）失败
///
/// 返回新分配的、包含 PNG 数据的 `Vec<u8>`。
pub fn image_zoom_in(image_data: &[u8], bbox: BBox) -> Result<Vec<u8>, ImageError> {
    let img: DynamicImage = image::load_from_memory(image_data)?;
    let (img_width, img_height) = img.dimensions();

    let abs_bbox = AbsolutePixelBBox::new(
        bbox.x1 / 1000.0 * (img_width as f64),
        bbox.y1 / 1000.0 * (img_height as f64),
        bbox.x2 / 1000.0 * (img_width as f64),
        bbox.y2 / 1000.0 * (img_height as f64),
    );

    let validated_bbox = abs_bbox.validate_and_resize(img_width, img_height);

    //    `image` crate 使用 (x, y, width, height)
    //    而 PIL 使用 (left, top, right, bottom)
    let (crop_x, crop_y, crop_width, crop_height) = validated_bbox.to_crop_args();

    if crop_width == 0 || crop_height == 0 {
        return Err(ImageError::Parameter(ParameterError::from_kind(
            image::error::ParameterErrorKind::DimensionMismatch,
        )));
    }

    let cropped_image = img.crop_imm(crop_x, crop_y, crop_width, crop_height);

    //    Python 代码使用: min_pixels=256 * 32 * 32 = 262144
    //    和 max_pixels=12845056 (默认值)
    let resizer = ImageResizer::new(32, 262144, 12845056);

    let (new_h, new_w) = resizer.smart_resize(crop_height, crop_width);

    //    Python 的 `Image.BICUBIC` 对应于 `image::imageops::FilterType::CatmullRom`
    let resized_image = cropped_image.resize_exact(new_w, new_h, imageops::FilterType::Lanczos3);

    let mut output_buffer: Vec<u8> = Vec::new();
    let mut cursor = Cursor::new(&mut output_buffer);

    resized_image.write_to(&mut cursor, ImageFormat::Png)?;

    Ok(output_buffer)
}
