use std::{io::Cursor, sync::Arc};

use anyhow::anyhow;
use image::ImageFormat;
use resvg::{tiny_skia, usvg};
use uuid::Uuid;

pub const FONT_DATA: &'static [u8] = include_bytes!("../../font.ttf");

pub fn convert_to_png(input_data: Vec<u8>) -> Result<Vec<u8>, anyhow::Error> {
    let format = image::guess_format(&input_data)?;
    match format {
        ImageFormat::Jpeg | ImageFormat::Png => Ok(input_data),
        _ => {
            let img = image::load_from_memory(&input_data)?;
            let mut png_data = Vec::new();
            let mut cursor = Cursor::new(&mut png_data);
            img.write_to(&mut cursor, ImageFormat::Png)?;
            Ok(png_data)
        }
    }
}

pub fn save_svg_to_db(db: &sled::Tree, svg_data: &str) -> Result<Uuid, anyhow::Error> {
    let mut font_db = usvg::fontdb::Database::new();
    font_db.load_font_data(FONT_DATA.to_vec());

    let usvg_options = usvg::Options {
        fontdb: Arc::new(font_db),
        font_family: "MapleMonoNormal-NF-CN-Regular".into(),
        ..Default::default()
    };

    let tree = usvg::Tree::from_str(svg_data, &usvg_options)?;

    let svg_size = tree.size();
    let width = svg_size.width().ceil() as u32;
    let height = svg_size.height().ceil() as u32;

    if width == 0 || height == 0 {
        return Err(anyhow!("Either width or height is 0"));
    }

    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(anyhow!(
        "Unable to create Pixmap with size {}x{}",
        width,
        height
    ))?;

    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());

    let output_buf = pixmap.encode_png()?;
    save_image_to_db(db, &output_buf)
}

pub fn save_image_to_db(db: &sled::Tree, img: &[u8]) -> Result<Uuid, anyhow::Error> {
    let mut uuid = Uuid::new_v4();
    for _ in 0..10 {
        match db.compare_and_swap(uuid, None::<&[u8]>, Some(img))? {
            Ok(()) => break,
            Err(_) => {
                uuid = Uuid::new_v4();
            }
        }
    }
    Ok(uuid)
}

/// 用于智能调整大小的辅助结构体
/// 移植自 Python 版本的 `smart_resize` 及其辅助函数
pub struct ImageResizer {
    factor: f64,
    min_pixels: f64,
    max_pixels: f64,
}

impl ImageResizer {
    /// * `factor`: 缩放因子，通常为 32
    /// * `min_pixels`: 最小像素总数
    /// * `max_pixels`: 最大像素总数
    pub fn new(factor: u32, min_pixels: u64, max_pixels: u64) -> Self {
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
    pub fn smart_resize(&self, height: u32, width: u32) -> (u32, u32) {
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
