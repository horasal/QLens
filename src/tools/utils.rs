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
