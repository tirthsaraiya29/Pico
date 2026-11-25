use crate::palette::Rgb;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// System sans-serif font loader with geometric bitmap glyph fallback for headless containers.
pub struct FontEngine {
    regular: Option<Arc<FontVec>>,
    bold: Option<Arc<FontVec>>,
}

impl FontEngine {
    pub fn load() -> Self {
        let regular_candidates: &[&str] = if cfg!(target_os = "windows") {
            &[
                r"C:\Windows\Fonts\segoeui.ttf",
                r"C:\Windows\Fonts\arial.ttf",
                r"C:\Windows\Fonts\calibri.ttf",
            ]
        } else {
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
            ]
        };

        let bold_candidates: &[&str] = if cfg!(target_os = "windows") {
            &[
                r"C:\Windows\Fonts\seguisb.ttf",
                r"C:\Windows\Fonts\segoeuib.ttf",
                r"C:\Windows\Fonts\arialbd.ttf",
            ]
        } else {
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            ]
        };

        let load_first = |list: &[&str]| -> Option<Arc<FontVec>> {
            for p in list {
                if let Ok(bytes) = std::fs::read(p) {
                    if let Ok(f) = FontVec::try_from_vec(bytes) {
                        return Some(Arc::new(f));
                    }
                }
            }
            None
        };

        let regular =
            load_first(regular_candidates).or_else(|| Self::fc_match_fallback().map(Arc::new));
        let bold = load_first(bold_candidates).or_else(|| regular.clone());
        Self { regular, bold }
    }

    #[cfg(target_os = "linux")]
    fn fc_match_fallback() -> Option<FontVec> {
        let out = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", "sans"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        std::fs::read(path)
            .ok()
            .and_then(|b| FontVec::try_from_vec(b).ok())
    }

    #[cfg(not(target_os = "linux"))]
    fn fc_match_fallback() -> Option<FontVec> {
        None
    }

    fn font(&self, bold: bool) -> Option<&FontVec> {
        if bold {
            self.bold.as_deref().or(self.regular.as_deref())
        } else {
            self.regular.as_deref()
        }
    }
}

/// CPU-rendered 32-bit XRGB framebuffer canvas with anti-aliased SDF rounded primitives.
pub struct Canvas<'a> {
    pub buf: &'a mut [u32],
    pub width: i32,
    pub height: i32,
    pub scale: f32,
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut [u32], width: i32, height: i32, scale: f32) -> Self {
        Self {
            buf,
            width,
            height,
            scale: scale.max(1.0),
        }
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    fn sf(&self, v: f32) -> f32 {
        v * self.scale
    }

    pub fn clear(&mut self, color: Rgb) {
        let px = color.to_argb_u32(255);
        self.buf.fill(px);
    }

    #[inline]
    fn blend_pixel(&mut self, px: i32, py: i32, src: Rgb, alpha: f32) {
        if px < 0 || py < 0 || px >= self.width || py >= self.height || alpha <= 0.002 {
            return;
        }
        let idx = (py * self.width + px) as usize;
        if idx >= self.buf.len() {
            return;
        }
        let a = alpha.clamp(0.0, 1.0);
        if a >= 0.996 {
            self.buf[idx] = src.to_argb_u32(255);
            return;
        }
        let dst = self.buf[idx];
        let dr = ((dst >> 16) & 0xff) as f32;
        let dg = ((dst >> 8) & 0xff) as f32;
        let db = (dst & 0xff) as f32;
        let r = (dr + (src.r as f32 - dr) * a).round() as u32;
        let g = (dg + (src.g as f32 - dg) * a).round() as u32;
        let b = (db + (src.b as f32 - db) * a).round() as u32;
        self.buf[idx] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
    }

    /// Evaluates signed distance to rounded box center (negative = inside).
    #[inline]
    fn sd_rounded_box(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
        let qx = (px - cx).abs() - hw + r;
        let qy = (py - cy).abs() - hh + r;
        let out_dist = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
        let in_dist = qx.max(qy).min(0.0);
        out_dist + in_dist - r
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Rgb, alpha: f32) {
        self.fill_rounded_gradient(rect, radius, color, color, alpha);
    }

    pub fn fill_rounded_gradient(
        &mut self,
        rect: Rect,
        radius: f32,
        left_color: Rgb,
        right_color: Rgb,
        alpha: f32,
    ) {
        let x0 = self.s(rect.x).max(0);
        let y0 = self.s(rect.y).max(0);
        let x1 = self.s(rect.x + rect.w).min(self.width);
        let y1 = self.s(rect.y + rect.h).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let fx0 = self.sf(rect.x as f32);
        let fy0 = self.sf(rect.y as f32);
        let fw = self.sf(rect.w as f32);
        let fh = self.sf(rect.h as f32);
        let cx = fx0 + fw * 0.5;
        let cy = fy0 + fh * 0.5;
        let hw = fw * 0.5;
        let hh = fh * 0.5;
        let r = self.sf(radius).min(hw).min(hh).max(0.0);
        let same_color = left_color == right_color;

        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                let sd = Self::sd_rounded_box(fx, fy, cx, cy, hw, hh, r);
                if sd >= 0.75 {
                    continue;
                }
                let aa = if sd <= -0.75 { 1.0 } else { (0.75 - sd) / 1.5 };
                let c = if same_color {
                    left_color
                } else {
                    let t = ((fx - fx0) / fw.max(1.0)).clamp(0.0, 1.0);
                    left_color.lerp(right_color, t)
                };
                self.blend_pixel(px, py, c, alpha * aa);
            }
        }
    }

    pub fn stroke_rounded_rect(&mut self, rect: Rect, radius: f32, color: Rgb, thickness: f32) {
        let x0 = (self.s(rect.x) - 1).max(0);
        let y0 = (self.s(rect.y) - 1).max(0);
        let x1 = (self.s(rect.x + rect.w) + 1).min(self.width);
        let y1 = (self.s(rect.y + rect.h) + 1).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let fw = self.sf(rect.w as f32);
        let fh = self.sf(rect.h as f32);
        let cx = self.sf(rect.x as f32) + fw * 0.5;
        let cy = self.sf(rect.y as f32) + fh * 0.5;
        let hw = fw * 0.5;
        let hh = fh * 0.5;
        let r = self.sf(radius).min(hw).min(hh).max(0.0);
        let half_t = (self.sf(thickness) * 0.5).max(0.5);

        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                let sd = Self::sd_rounded_box(fx, fy, cx, cy, hw, hh, r);
                let edge_dist = (sd + half_t).abs();
                if edge_dist <= half_t + 0.65 {
                    let aa = ((half_t + 0.65 - edge_dist) / 1.3).clamp(0.0, 1.0);
                    self.blend_pixel(px, py, color, aa);
                }
            }
        }
    }

    pub fn blit_rounded_rgba(
        &mut self,
        rect: Rect,
        radius: f32,
        src_w: u32,
        src_h: u32,
        rgba: &[u8],
    ) {
        if src_w == 0 || src_h == 0 || rgba.len() < (src_w * src_h * 4) as usize {
            return;
        }
        let x0 = self.s(rect.x).max(0);
        let y0 = self.s(rect.y).max(0);
        let x1 = self.s(rect.x + rect.w).min(self.width);
        let y1 = self.s(rect.y + rect.h).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let fx0 = self.sf(rect.x as f32);
        let fy0 = self.sf(rect.y as f32);
        let fw = self.sf(rect.w as f32).max(1.0);
        let fh = self.sf(rect.h as f32).max(1.0);
        let cx = fx0 + fw * 0.5;
        let cy = fy0 + fh * 0.5;
        let r = self.sf(radius).min(fw * 0.5).min(fh * 0.5);

        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            let sy = (((fy - fy0) / fh) * (src_h as f32)).clamp(0.0, (src_h - 1) as f32) as u32;
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                let sd = Self::sd_rounded_box(fx, fy, cx, cy, fw * 0.5, fh * 0.5, r);
                if sd >= 0.75 {
                    continue;
                }
                let aa = if sd <= -0.75 { 1.0 } else { (0.75 - sd) / 1.5 };
                let sx = (((fx - fx0) / fw) * (src_w as f32)).clamp(0.0, (src_w - 1) as f32) as u32;
                let si = ((sy * src_w + sx) * 4) as usize;
                let c = Rgb::new(rgba[si], rgba[si + 1], rgba[si + 2]);
                let a = (rgba[si + 3] as f32 / 255.0) * aa;
                self.blend_pixel(px, py, c, a);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_clipped(
        &mut self,
        fonts: &FontEngine,
        text: &str,
        x: i32,
        y: i32,
        max_w: i32,
        size_px: f32,
        bold: bool,
        color: Rgb,
    ) {
        if text.is_empty() || max_w <= 4 {
            return;
        }
        let Some(font) = fonts.font(bold) else {
            self.draw_bitmap_fallback(text, x, y, max_w, color);
            return;
        };

        let px_scale = PxScale::from(self.sf(size_px));
        let scaled = font.as_scaled(px_scale);
        let origin_x = self.sf(x as f32);
        let limit_x = self.sf((x + max_w) as f32);
        let baseline_y = self.sf(y as f32) + scaled.ascent();

        let ellipsis_w = scaled.h_advance(font.glyph_id('.')) * 3.0;
        let mut cursor_x = origin_x;
        let mut prev_glyph: Option<ab_glyph::GlyphId> = None;

        // First measure if clipping needed.
        let mut needs_clip = false;
        {
            let mut probe_x = origin_x;
            let mut prev = None;
            for ch in text.chars() {
                let g = font.glyph_id(ch);
                if let Some(pg) = prev {
                    probe_x += scaled.kern(pg, g);
                }
                probe_x += scaled.h_advance(g);
                if probe_x > limit_x {
                    needs_clip = true;
                    break;
                }
                prev = Some(g);
            }
        }

        let stop_x = if needs_clip {
            (limit_x - ellipsis_w).max(origin_x)
        } else {
            limit_x
        };

        let mut render_char = |ch: char, cx: &mut f32, pg_opt: &mut Option<ab_glyph::GlyphId>| {
            let gid = font.glyph_id(ch);
            if let Some(pg) = *pg_opt {
                *cx += scaled.kern(pg, gid);
            }
            let adv = scaled.h_advance(gid);
            let glyph = gid.with_scale_and_position(px_scale, ab_glyph::point(*cx, baseline_y));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, cov| {
                    let px = bounds.min.x.floor() as i32 + gx as i32;
                    let py = bounds.min.y.floor() as i32 + gy as i32;
                    if px >= origin_x.floor() as i32 && (px as f32) < limit_x {
                        self.blend_pixel(px, py, color, cov);
                    }
                });
            }
            *cx += adv;
            *pg_opt = Some(gid);
        };

        for ch in text.chars() {
            let gid = font.glyph_id(ch);
            let next_w = scaled.h_advance(gid);
            if needs_clip && cursor_x + next_w > stop_x {
                for _ in 0..3 {
                    render_char('.', &mut cursor_x, &mut prev_glyph);
                }
                break;
            }
            render_char(ch, &mut cursor_x, &mut prev_glyph);
        }
    }

    fn draw_bitmap_fallback(&mut self, text: &str, x: i32, y: i32, max_w: i32, color: Rgb) {
        let mut cx = x;
        for ch in text.chars() {
            if cx + 6 > x + max_w {
                break;
            }
            if ch != ' ' {
                let box_r = Rect::new(cx, y + 2, 4, 8);
                self.fill_rounded_rect(box_r, 1.0, color, 0.85);
            }
            cx += 6;
        }
    }
}
