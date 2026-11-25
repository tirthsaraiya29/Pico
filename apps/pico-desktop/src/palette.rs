use image::imageops::FilterType;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: (self.r as f32 + (other.r as f32 - self.r as f32) * t).round() as u8,
            g: (self.g as f32 + (other.g as f32 - self.g as f32) * t).round() as u8,
            b: (self.b as f32 + (other.b as f32 - self.b as f32) * t).round() as u8,
        }
    }

    pub fn to_argb_u32(self, alpha: u8) -> u32 {
        ((alpha as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

/// Contrast-verified media card palette derived from album artwork.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtPalette {
    pub bg_left: Rgb,
    pub bg_right: Rgb,
    pub accent: Rgb,
    pub text_primary: Rgb,
    pub text_secondary: Rgb,
}

impl Default for ArtPalette {
    fn default() -> Self {
        Self {
            bg_left: Rgb::new(22, 27, 38),
            bg_right: Rgb::new(28, 38, 54),
            accent: Rgb::new(82, 168, 255),
            text_primary: Rgb::new(244, 246, 250),
            text_secondary: Rgb::new(198, 206, 219),
        }
    }
}

/// Computes WCAG 2.1 relative luminance (0.0 .. 1.0).
pub fn relative_luminance(c: Rgb) -> f32 {
    fn linearize(v: u8) -> f32 {
        let s = v as f32 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linearize(c.r) + 0.7152 * linearize(c.g) + 0.0722 * linearize(c.b)
}

/// Computes WCAG 2.1 contrast ratio (1.0 .. 21.0).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Enforces dark companion backdrop stops so crisp `#F4F6FA` / `#C6CEDB` labels strictly pass >= 4.5:1.
pub fn contrast_safe_palette(dom_primary: Rgb, dom_secondary: Rgb) -> ArtPalette {
    let base_dark = Rgb::new(17, 20, 28);
    let fg_primary = Rgb::new(244, 246, 250);
    let fg_secondary = Rgb::new(198, 206, 219);

    let darken_for_contrast = |candidate: Rgb, weight: f32| -> Rgb {
        let mut mixed = base_dark.lerp(candidate, weight);
        // Progressively step toward base_dark until both primary and secondary text hit >= 4.8:1
        for step in 0..=16 {
            if contrast_ratio(fg_primary, mixed) >= 7.0
                && contrast_ratio(fg_secondary, mixed) >= 4.8
            {
                break;
            }
            let factor = (1.0 - (step as f32 * 0.06)).max(0.05);
            mixed = base_dark.lerp(candidate, weight * factor);
        }
        mixed
    };

    let bg_left = darken_for_contrast(dom_primary, 0.34);
    let bg_right = darken_for_contrast(dom_secondary, 0.28);

    // Brighten accent slightly if too dark against obsidian cards
    let accent = if relative_luminance(dom_primary) < 0.14 {
        dom_primary.lerp(Rgb::new(140, 200, 255), 0.55)
    } else {
        dom_primary
    };

    ArtPalette {
        bg_left,
        bg_right,
        accent,
        text_primary: fg_primary,
        text_secondary: fg_secondary,
    }
}

/// Decoded square thumbnail ready for crisp software blitting + derived palette.
#[derive(Clone)]
pub struct CachedArtwork {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub palette: ArtPalette,
}

/// Bounded LRU artwork cache (6 entries) avoiding repeated image decodes on track redraws.
pub struct ArtworkCache {
    capacity: usize,
    order: VecDeque<u64>,
    entries: HashMap<u64, CachedArtwork>,
}

impl ArtworkCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(2),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    pub fn get_or_decode(&mut self, raw_bytes: &[u8]) -> Option<&CachedArtwork> {
        let key = fnv1a_bytes(raw_bytes);
        if !self.entries.contains_key(&key) {
            let decoded = decode_and_extract_palette(raw_bytes)?;
            while self.order.len() >= self.capacity {
                if let Some(evicted) = self.order.pop_front() {
                    self.entries.remove(&evicted);
                }
            }
            self.order.push_back(key);
            self.entries.insert(key, decoded);
        }
        self.entries.get(&key)
    }
}

fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn decode_and_extract_palette(raw_bytes: &[u8]) -> Option<CachedArtwork> {
    let img = image::load_from_memory(raw_bytes).ok()?;
    let rgba_img = img.resize_to_fill(60, 60, FilterType::Triangle).to_rgba8();
    let (dom1, dom2) = extract_dominant_colors(rgba_img.as_raw());
    let palette = contrast_safe_palette(dom1, dom2);
    Some(CachedArtwork {
        width: 60,
        height: 60,
        rgba: rgba_img.into_raw(),
        palette,
    })
}

#[derive(Default)]
struct ColorBucket {
    weight: f32,
    count: u32,
    r_sum: u64,
    g_sum: u64,
    b_sum: u64,
}

/// Quantizes pixels into chromatic buckets preferring vibrant non-neutral hues.
pub fn extract_dominant_colors(rgba: &[u8]) -> (Rgb, Rgb) {
    let mut counted: HashMap<(u8, u8, u8), ColorBucket> = HashMap::new();
    for px in rgba.as_chunks::<4>().0 {
        if px[3] < 32 {
            continue;
        }
        let (r, g, b) = (px[0], px[1], px[2]);
        let max_c = r.max(g).max(b) as f32;
        let min_c = r.min(g).min(b) as f32;
        let sat = if max_c > 0.0 {
            (max_c - min_c) / max_c
        } else {
            0.0
        };
        let e = counted.entry((r >> 4, g >> 4, b >> 4)).or_default();
        e.weight += 0.25 + sat * 2.4;
        e.count += 1;
        e.r_sum += r as u64;
        e.g_sum += g as u64;
        e.b_sum += b as u64;
    }

    if counted.is_empty() {
        return (Rgb::new(66, 128, 220), Rgb::new(40, 64, 112));
    }

    let mut sorted: Vec<ColorBucket> = counted.into_values().collect();
    sorted.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let to_rgb = |bucket: &ColorBucket| -> Rgb {
        let c = bucket.count.max(1) as u64;
        Rgb::new(
            (bucket.r_sum / c) as u8,
            (bucket.g_sum / c) as u8,
            (bucket.b_sum / c) as u8,
        )
    };

    let c1 = to_rgb(&sorted[0]);
    let c2 = sorted
        .iter()
        .skip(1)
        .map(to_rgb)
        .find(|cand| {
            let dr = (cand.r as i16 - c1.r as i16).abs();
            let dg = (cand.g as i16 - c1.g as i16).abs();
            let db = (cand.b as i16 - c1.b as i16).abs();
            dr + dg + db > 45
        })
        .unwrap_or_else(|| Rgb::new(c1.r / 2, c1.g / 2, c1.b / 2));

    (c1, c2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contrast_safe_palette_passes_wcag_even_on_pure_white_cover() {
        let p = contrast_safe_palette(Rgb::new(255, 255, 255), Rgb::new(255, 240, 40));
        assert!(contrast_ratio(p.text_primary, p.bg_left) >= 7.0);
        assert!(contrast_ratio(p.text_secondary, p.bg_left) >= 4.5);
        assert!(contrast_ratio(p.text_primary, p.bg_right) >= 7.0);
        assert!(contrast_ratio(p.text_secondary, p.bg_right) >= 4.5);
    }
}
