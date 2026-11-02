use ab_glyph::{FontRef, PxScale};
use image::codecs::png::PngEncoder;
use image::{ImageEncoder, Rgba, RgbaImage};
use imageproc::drawing::{draw_text_mut, text_size};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");

pub fn stamp_screenshot(path: &Path, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut img: RgbaImage = image::open(path)?.to_rgba8();
    let (width, height) = img.dimensions();

    // Ensure fully opaque (alpha = 255) for all pixels so semi-transparent windows
    // (e.g. Kitty terminal with background_opacity 0.8) are rendered as solid opaque surfaces
    // without any transparency or wallpaper showing through.
    for p in img.pixels_mut() {
        p[3] = 255;
    }

    let font = FontRef::try_from_slice(FONT_DATA)?;
    let scale = PxScale::from(22.0);

    let (text_w, text_h) = text_size(scale, &font, text);

    let margin_x = 12i32;
    let margin_y = 12i32;

    let text_pos_x = margin_x;
    let text_pos_y = (height as i32) - margin_y - (text_h as i32);

    if text_pos_y >= 0 && text_pos_x >= 0 && (text_pos_x + text_w as i32) <= (width as i32) {
        // Outline/shadow for readability without any background box
        for dx in -1..=1 {
            for dy in -1..=1 {
                if dx != 0 || dy != 0 {
                    draw_text_mut(
                        &mut img,
                        Rgba([0, 0, 0, 230]),
                        text_pos_x + dx,
                        text_pos_y + dy,
                        scale,
                        &font,
                        text,
                    );
                }
            }
        }

        // White text: RGBA (255, 255, 255, 255)
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
