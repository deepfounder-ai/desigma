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

/// What the brush marked on a picture: one byte per cell of a grid the size of the preview
/// (0 keeps the picture, 255 lets the model change it).
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl Mask {
    pub fn new(w: u32, h: u32) -> Self {
        let (w, h) = (w.max(1), h.max(1));
        Mask { w, h, data: vec![0; (w * h) as usize] }
    }
    pub fn is_empty(&self) -> bool {
        self.data.iter().all(|v| *v == 0)
    }
    pub fn clear(&mut self) {
        self.data.fill(0);
    }
    /// Marks a round-ended stroke from `a` to `b` (grid cells) of radius `r`.
    pub fn stroke(&mut self, a: (f32, f32), b: (f32, f32), r: f32) {
        let r = r.max(0.5);
        let (x0, x1) = ((a.0.min(b.0) - r).floor().max(0.0) as u32, ((a.0.max(b.0) + r).ceil().max(0.0) as u32).min(self.w.saturating_sub(1)));
        let (y0, y1) = ((a.1.min(b.1) - r).floor().max(0.0) as u32, ((a.1.max(b.1) + r).ceil().max(0.0) as u32).min(self.h.saturating_sub(1)));
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
                if (px - cx).powi(2) + (py - cy).powi(2) <= r * r {
                    self.data[(y * self.w + x) as usize] = 255;
                }
            }
        }
    }
    /// The box around everything marked.
    pub fn bounds(&self) -> Option<Region> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (i, v) in self.data.iter().enumerate() {
            if *v != 0 {
                let (x, y) = (i as u32 % self.w, i as u32 / self.w);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        (x0 != u32::MAX)
            .then(|| Region::new((x0 as f32 / self.w as f32, y0 as f32 / self.h as f32), ((x1 + 1) as f32 / self.w as f32, (y1 + 1) as f32 / self.h as f32)))
    }
}

/// `generated` at the size of `original`.
pub fn fit_to(original: &RgbaImage, generated: &RgbaImage) -> RgbaImage {
    let (w, h) = original.dimensions();
    if generated.dimensions() == (w, h) {
        generated.clone()
    } else {
        imageops::resize(generated, w, h, imageops::FilterType::Lanczos3)
    }
}

/// `original` with the marked part taken from `generated` (scaled to the original's size).
/// Pixels outside the mark are the original's; inside, a soft edge hides the seam.
pub fn paste_mask(original: &RgbaImage, generated: &RgbaImage, mask: &Mask) -> RgbaImage {
    let (w, h) = original.dimensions();
    let generated = fit_to(original, generated);
    let hard = image::GrayImage::from_raw(mask.w, mask.h, mask.data.clone()).unwrap_or_else(|| image::GrayImage::new(mask.w, mask.h));
    // the blurred mark is 1 deep inside and about one half on the edge: only its upper half is
    // used, so the fade stays inside the mark
    let soft = imageops::blur(&hard, 3.0);
    let hard = imageops::resize(&hard, w, h, imageops::FilterType::Triangle);
    let soft = imageops::resize(&soft, w, h, imageops::FilterType::Triangle);
    let mut out = original.clone();
    for (x, y, px) in out.enumerate_pixels_mut() {
        let inside = hard.get_pixel(x, y).0[0] as f32 / 255.0;
        if inside == 0.0 {
            continue;
        }
        let a = inside * ((soft.get_pixel(x, y).0[0] as f32 / 255.0 - 0.5) * 2.0).clamp(0.0, 1.0);
        let g = generated.get_pixel(x, y).0;
        for (o, g) in px.0.iter_mut().zip(g) {
            *o = (*o as f32 * (1.0 - a) + g as f32 * a).round() as u8;
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
    fn pasting_changes_only_what_the_brush_marked() {
        let original = flat(200, 100, [10, 10, 10, 255]);
        let generated = flat(100, 50, [250, 250, 250, 255]);
        let mut mask = Mask::new(100, 50);
        // a horizontal stroke in the right half, 10 cells thick each way: cells 50..90 by 15..35
        mask.stroke((60.0, 25.0), (80.0, 25.0), 10.0);
        let out = paste_mask(&original, &generated, &mask);
        assert_eq!(out.dimensions(), (200, 100));
        for (x, y) in [(0, 0), (60, 50), (150, 10), (150, 90), (199, 50)] {
            assert_eq!(out.get_pixel(x, y).0, [10, 10, 10, 255], "outside the mark at {x},{y}");
        }
        assert_eq!(out.get_pixel(150, 50).0, [250, 250, 250, 255], "the middle of the stroke is the generated image");
        let edge = out.get_pixel(150, 33).0[0];
        assert!(edge > 10 && edge < 250, "the edge of the stroke is blended: {edge}");
    }

    #[test]
    fn masks_know_their_bounds_and_emptiness() {
        let mut m = Mask::new(100, 50);
        assert!(m.is_empty() && m.bounds().is_none());
        m.stroke((50.0, 25.0), (50.0, 25.0), 5.0);
        let b = m.bounds().unwrap();
        assert!((b.x0 - 0.45).abs() < 0.02 && (b.x1 - 0.55).abs() < 0.02 && (b.y0 - 0.4).abs() < 0.03 && (b.y1 - 0.6).abs() < 0.03, "{b:?}");
        // a stroke that starts and ends outside the grid is clipped to it
        m.stroke((-20.0, 25.0), (200.0, 25.0), 4.0);
        let b = m.bounds().unwrap();
        assert_eq!((b.x0, b.x1), (0.0, 1.0));
        m.clear();
        assert!(m.is_empty());
        assert_eq!(fit_to(&flat(8, 4, [0; 4]), &flat(2, 2, [9, 9, 9, 255])).dimensions(), (8, 4));
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
