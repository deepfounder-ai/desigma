//! Pixel work for AI images: shrinking references, and pasting the edited part of a generated
//! image back into the original so nothing outside the chosen region changes.

use image::{imageops, RgbaImage};

/// Longest side of an image sent as a reference.
pub const MAX_REFERENCE_SIDE: u32 = 2048;

pub fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    image::load_from_memory(bytes).map(|i| i.to_rgba8()).map_err(|e| format!("Cannot decode image: {e}"))
}

pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("Cannot encode image: {e}"))?;
    Ok(out.into_inner())
}

/// `img` as PNG bytes, scaled down so its longest side is at most [`MAX_REFERENCE_SIDE`].
pub fn reference_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let (w, h) = img.dimensions();
    let long = w.max(h);
    if long <= MAX_REFERENCE_SIDE {
        return encode_png(img);
    }
    let k = MAX_REFERENCE_SIDE as f32 / long as f32;
    encode_png(&imageops::resize(img, ((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1), imageops::FilterType::Triangle))
}

/// A region of an image as fractions of its width and height (0..=1), `x0 < x1`, `y0 < y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Region {
    pub fn new(a: (f32, f32), b: (f32, f32)) -> Self {
        let c = |v: f32| v.clamp(0.0, 1.0);
        Region { x0: c(a.0.min(b.0)), y0: c(a.1.min(b.1)), x1: c(a.0.max(b.0)), y1: c(a.1.max(b.1)) }
    }
    /// Too small to be meant as a region (a click, not a drag).
    pub fn is_tiny(&self) -> bool {
        self.x1 - self.x0 < 0.02 || self.y1 - self.y0 < 0.02
    }
    /// The region in words, for the instruction sent to the model.
    pub fn percent(&self) -> String {
        let p = |v: f32| (v * 100.0).round() as i32;
        format!("from {}% to {}% of the width and from {}% to {}% of the height", p(self.x0), p(self.x1), p(self.y0), p(self.y1))
    }
}

/// `original` with `region` taken from `generated` (scaled to the original's size). Pixels
/// outside the region are the original's; a soft edge inside the region hides the seam.
pub fn paste_region(original: &RgbaImage, generated: &RgbaImage, region: Region) -> RgbaImage {
    let (w, h) = original.dimensions();
    let scaled;
    let generated = if generated.dimensions() == (w, h) {
        generated
    } else {
        scaled = imageops::resize(generated, w, h, imageops::FilterType::Lanczos3);
        &scaled
    };
    let (x0, y0) = ((region.x0 * w as f32).floor() as u32, (region.y0 * h as f32).floor() as u32);
    let (x1, y1) = (((region.x1 * w as f32).ceil() as u32).min(w), ((region.y1 * h as f32).ceil() as u32).min(h));
    let feather = (x1.saturating_sub(x0).min(y1.saturating_sub(y0)) as f32 / 8.0).clamp(1.0, 24.0);
    let mut out = original.clone();
    for y in y0..y1 {
        for x in x0..x1 {
            // distance to the nearest region edge, in pixels; edges on the image border do not fade
            let mut d = f32::MAX;
            for (edge, at_border) in [(x - x0, x0 == 0), (x1 - 1 - x, x1 == w), (y - y0, y0 == 0), (y1 - 1 - y, y1 == h)] {
                if !at_border {
                    d = d.min(edge as f32 + 0.5);
                }
            }
            let a = (d / feather).clamp(0.0, 1.0);
            let (o, g) = (original.get_pixel(x, y).0, generated.get_pixel(x, y).0);
            let mut px = [0u8; 4];
            for c in 0..4 {
                px[c] = (o[c] as f32 * (1.0 - a) + g[c] as f32 * a).round() as u8;
            }
            out.put_pixel(x, y, image::Rgba(px));
        }
    }
    out
}

/// The choices of the dialog; "Auto" leaves it to the provider.
pub const ASPECTS: [&str; 8] = ["Auto", "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"];

/// Index in [`ASPECTS`] of the ratio closest to `w`:`h`.
pub fn nearest_aspect(w: f32, h: f32) -> usize {
    let want = (w.max(1.0) / h.max(1.0)).ln();
    let ratio = |s: &str| s.split_once(':').map(|(x, y)| (x.parse::<f32>().unwrap_or(1.0) / y.parse::<f32>().unwrap_or(1.0)).ln()).unwrap_or(0.0);
    (1..ASPECTS.len()).min_by(|a, b| (ratio(ASPECTS[*a]) - want).abs().total_cmp(&(ratio(ASPECTS[*b]) - want).abs())).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn flat(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn pasting_changes_only_the_region_and_fades_at_its_edges() {
        let original = flat(200, 100, [10, 10, 10, 255]);
        let generated = flat(100, 50, [250, 250, 250, 255]);
        let out = paste_region(&original, &generated, Region::new((0.25, 0.2), (0.75, 0.8)));
        assert_eq!(out.dimensions(), (200, 100));
        for (x, y) in [(0, 0), (49, 50), (150, 50), (100, 19), (100, 80), (199, 99)] {
            assert_eq!(out.get_pixel(x, y).0, [10, 10, 10, 255], "outside the region at {x},{y}");
        }
        assert_eq!(out.get_pixel(100, 50).0, [250, 250, 250, 255], "the middle is the generated image");
        let edge = out.get_pixel(50, 50).0[0];
        assert!(edge > 10 && edge < 250, "the edge is blended: {edge}");
    }

    #[test]
    fn a_region_on_the_image_border_does_not_fade_there() {
        let out = paste_region(&flat(64, 64, [0, 0, 0, 255]), &flat(64, 64, [200, 200, 200, 255]), Region::new((0.0, 0.0), (1.0, 1.0)));
        assert!(out.pixels().all(|p| p.0 == [200, 200, 200, 255]), "the whole image is replaced");
    }

    #[test]
    fn regions_normalize_and_aspects_snap() {
        let r = Region::new((0.9, 1.4), (0.1, -0.2));
        assert_eq!(r, Region { x0: 0.1, y0: 0.0, x1: 0.9, y1: 1.0 });
        assert!(Region::new((0.5, 0.5), (0.505, 0.9)).is_tiny());
        assert_eq!(r.percent(), "from 10% to 90% of the width and from 0% to 100% of the height");
        assert_eq!(ASPECTS[nearest_aspect(1920.0, 1080.0)], "16:9");
        assert_eq!(ASPECTS[nearest_aspect(500.0, 510.0)], "1:1");
        assert_eq!(ASPECTS[nearest_aspect(300.0, 450.0)], "2:3");
    }

    #[test]
    fn large_references_are_scaled_down() {
        let png = reference_png(&flat(4096, 1024, [1, 2, 3, 255])).unwrap();
        assert_eq!(decode(&png).unwrap().dimensions(), (2048, 512));
        let small = reference_png(&flat(10, 20, [1, 2, 3, 255])).unwrap();
        assert_eq!(decode(&small).unwrap().dimensions(), (10, 20));
    }
}
