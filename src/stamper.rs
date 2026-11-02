use ab_glyph::{FontRef, PxScale};
use image::codecs::png::PngEncoder;
use image::{ImageEncoder, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut, text_size};
use imageproc::rect::Rect;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");

pub fn stamp_screenshot(path: &Path, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut img: RgbaImage = image::open(path)?.to_rgba8();
    let (width, height) = img.dimensions();

    let font = FontRef::try_from_slice(FONT_DATA)?;
    let scale = PxScale::from(22.0);

    let (text_w, text_h) = text_size(scale, &font, text);

    let padding_x = 8i32;
    let padding_y = 4i32;

    let margin_x = 10i32;
    let margin_y = 10i32;

    let box_w = text_w as i32 + padding_x * 2;
    let box_h = text_h as i32 + padding_y * 2;

    let box_x = margin_x;
    let box_y = (height as i32) - margin_y - box_h;

    if box_y >= 0 && box_x >= 0 && (box_x + box_w) <= (width as i32) {
        // Semi-transparent dark background: RGBA (15, 15, 15, 200)
        let rect = Rect::at(box_x, box_y).of_size(box_w as u32, box_h as u32);
        draw_filled_rect_mut(&mut img, rect, Rgba([15, 15, 15, 200]));

        // White text: RGBA (255, 255, 255, 255)
        let text_pos_x = box_x + padding_x;
        let text_pos_y = box_y + padding_y;
        draw_text_mut(
            &mut img,
            Rgba([255, 255, 255, 255]),
            text_pos_x,
            text_pos_y,
            scale,
            &font,
            text,
        );
    }

    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    let encoder = PngEncoder::new(writer);
    encoder.write_image(&img, width, height, image::ExtendedColorType::Rgba8)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stamp() {
        let img = RgbaImage::new(400, 200);
        let path = std::env::temp_dir().join("test_stamp.png");
        img.save(&path).unwrap();
        stamp_screenshot(&path, "2026-11-02 21:45:00").unwrap();
        let stamped = image::open(&path).unwrap().to_rgba8();
        let mut non_black = 0;
        for p in stamped.pixels() {
            if p[0] != 0 || p[1] != 0 || p[2] != 0 {
                non_black += 1;
            }
        }
        println!("Non-black pixels: {non_black}");
        assert!(non_black > 0);
        let _ = std::fs::remove_file(path);
    }
}
