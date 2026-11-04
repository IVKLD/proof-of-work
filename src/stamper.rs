use ab_glyph::{FontRef, PxScale};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ImageEncoder, Rgba, RgbaImage};
use imageproc::drawing::{draw_text_mut, text_size};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");

pub fn stamp_screenshot(
    path: &Path,
    text: &str,
    blocked_rects: &[[u32; 4]],
    jpeg_quality: u8,
    max_width: Option<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut img: RgbaImage = image::open(path)?.to_rgba8();
    let (mut width, mut height) = img.dimensions();
    if width == 0 || height == 0 {
        return Err("Image dimensions are zero".into());
    }

    // 1. Black out all forbidden application windows
    for &[bx, by, bw, bh] in blocked_rects {
        if bx < width && by < height {
            let max_x = bx.saturating_add(bw).min(width);
            let max_y = by.saturating_add(bh).min(height);
            for y in by..max_y {
                for x in bx..max_x {
                    img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                }
            }
        }
    }

    // 2. Ensure fully opaque (alpha = 255) for all pixels so semi-transparent windows
    // (e.g. Kitty terminal with background_opacity 0.8) are rendered as solid opaque surfaces
    // without any transparency or wallpaper showing through.
    for p in img.pixels_mut() {
        p[3] = 255;
    }

    // 3. Optional downscale for screens larger than max_width (e.g. 4K down to 1080p for ~500KB)
    if let Some(mw) = max_width
        && width > mw && mw > 0
    {
        let target_w = mw;
        let target_h = ((height as u64 * mw as u64) / width as u64) as u32;
        img = image::imageops::resize(
            &img,
            target_w,
            target_h,
            image::imageops::FilterType::Triangle,
        );
        width = target_w;
        height = target_h;
    }

    let font = FontRef::try_from_slice(FONT_DATA)?;
    let font_size = if height >= 2000 { 36.0 } else if height >= 1400 { 28.0 } else { 22.0 };
    let scale = PxScale::from(font_size);

    let (text_w, text_h) = text_size(scale, &font, text);

    let margin_x = if height >= 2000 { 20i32 } else { 12i32 };
    let margin_y = margin_x;

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

    let pid = std::process::id();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("screenshot");
    let tmp_path = path.with_file_name(format!("{file_name}.tmp.{pid}"));
    let encode_res = (|| -> Result<(), Box<dyn std::error::Error>> {
        let file = File::create(&tmp_path)?;
        let writer = BufWriter::new(file);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "jpg" || ext == "jpeg" {
            let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();
            let mut encoder = JpegEncoder::new_with_quality(writer, jpeg_quality);
            encoder.encode(
                rgb_img.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgb8,
            )?;
        } else {
            let encoder = PngEncoder::new(writer);
            encoder.write_image(&img, width, height, image::ExtendedColorType::Rgba8)?;
        }
        Ok(())
    })();

    if let Err(e) = encode_res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    std::fs::rename(&tmp_path, path)?;

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
        stamp_screenshot(&path, "2026-11-02 21:45:00", &[], 80, None).unwrap();
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

    #[test]
    fn test_stamp_jpeg() {
        let img = image::RgbImage::new(400, 200);
        let path = std::env::temp_dir().join("test_stamp.jpg");
        img.save(&path).unwrap();
        stamp_screenshot(&path, "2026-11-02 21:45:00", &[], 80, None).unwrap();
        let stamped = image::open(&path).unwrap().to_rgba8();
        let (w, h) = stamped.dimensions();
        assert_eq!(w, 400);
        assert_eq!(h, 200);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_stamp_downscale() {
        let img = image::RgbImage::new(3840, 2160);
        let path = std::env::temp_dir().join("test_stamp_downscale.jpg");
        img.save(&path).unwrap();
        stamp_screenshot(&path, "2026-11-02 21:45:00", &[], 60, Some(1920)).unwrap();
        let stamped = image::open(&path).unwrap().to_rgba8();
        let (w, h) = stamped.dimensions();
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_blocked_blackout() {
        let mut img = RgbaImage::new(400, 200);
        for p in img.pixels_mut() {
            *p = Rgba([255, 255, 255, 255]); // white
        }
        let path = std::env::temp_dir().join("test_blackout.png");
        img.save(&path).unwrap();
        stamp_screenshot(&path, "2026-11-02 21:45:00", &[[50, 50, 100, 100]], 80, None).unwrap();
        let stamped = image::open(&path).unwrap().to_rgba8();
        assert_eq!(stamped.get_pixel(60, 60), &Rgba([0, 0, 0, 255]));
        assert_eq!(stamped.get_pixel(10, 10), &Rgba([255, 255, 255, 255]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_saved_screenshot_integrity() {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if let Ok(home) = std::env::var("HOME") {
            let day_dir = std::path::PathBuf::from(home).join("worklog").join(today);
            if let Ok(entries) = std::fs::read_dir(day_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "png" || e == "jpg" || e == "jpeg") {
                        let img = image::open(&path).expect("Failed to open saved screenshot");
                        let (w, h) = (img.width(), img.height());
                        assert!(w > 0 && h > 0);
                        break;
                    }
                }
            }
        }
    }
}
