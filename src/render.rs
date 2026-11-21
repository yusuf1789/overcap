//! Software compositing for the overlay bitmap.
//!
//! The overlay is a per-pixel-alpha layered window (`UpdateLayeredWindow`), so the background
//! box can be translucent while the text stays fully opaque. GDI cannot draw antialiased text
//! into an alpha bitmap, so the glyphs are rendered as a grayscale coverage mask (white on
//! black) and composited here: background -> shadow -> outline -> text.
//!
//! All pixel buffers are top-down, premultiplied BGRA8, which is what `UpdateLayeredWindow`
//! with `AC_SRC_ALPHA` expects.

use anyhow::{anyhow, Result};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, GdiFlush,
    GetDC, ReleaseDC, SelectObject, SetBkMode, SetTextColor, AC_SRC_ALPHA, AC_SRC_OVER,
    ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CALCRECT, DT_CENTER, DT_NOPREFIX,
    DT_WORDBREAK, FF_SWISS, FW_BOLD, FW_NORMAL, HFONT, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

use crate::config::{parse_hex_colorref, OverlayStyle, MAX_OUTLINE_WIDTH};

/// Drop-shadow offset in pixels (down and to the right).
const SHADOW_OFFSET: i32 = 2;

/// Per-pixel coverage, 0 (empty) ..= 255 (fully covered).
#[derive(Clone)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Mask {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            data: vec![0; w * h],
        }
    }
}

/// A solid color painted through a mask, optionally shifted by `(dx, dy)`.
pub struct Layer<'a> {
    pub mask: &'a Mask,
    pub color: [u8; 3],
    pub dx: i32,
    pub dy: i32,
}

fn hex_to_rgb(hex: &str, fallback_colorref: u32) -> [u8; 3] {
    let v = parse_hex_colorref(hex, fallback_colorref);
    [
        (v & 0xFF) as u8,
        ((v >> 8) & 0xFF) as u8,
        ((v >> 16) & 0xFF) as u8,
    ]
}

unsafe fn create_font(style: &OverlayStyle) -> HFONT {
    let mut face: Vec<u16> = style.font_family.encode_utf16().collect();
    face.push(0);
    let weight = if style.font_bold {
        FW_BOLD.0
    } else {
        FW_NORMAL.0
    } as i32;
    CreateFontW(
        -style.font_size,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
        PCWSTR(face.as_ptr()),
    )
}

/// Size of `text` wrapped to `max_width`, using the style's font.
pub fn measure_text_size(text: &str, style: &OverlayStyle, max_width: i32) -> (i32, i32) {
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return (max_width, style.font_size + 12);
        }
        let font = create_font(style);
        let old_font = SelectObject(hdc, font);

        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: max_width.max(40),
            bottom: 0,
        };
        DrawTextW(
            hdc,
            &mut wide,
            &mut rc,
            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT,
        );

        SelectObject(hdc, old_font);
        let _ = DeleteObject(font);
        ReleaseDC(None, hdc);

        ((rc.right - rc.left).max(20), (rc.bottom - rc.top).max(20))
    }
}

fn bitmap_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [Default::default(); 1],
    }
}

/// Draws `text` as white-on-black antialiased glyphs into a `w` x `h` coverage mask,
/// centered inside the padding the same way the overlay lays it out.
pub fn render_text_mask(text: &str, style: &OverlayStyle, w: i32, h: i32) -> Mask {
    render_text_mask_aligned(text, style, w, h, true)
}

/// Geçmişte uzun paragraflar soldan okunur; altyazının mevcut ortalama
/// davranışı ayrı giriş noktasıyla aynen korunur.
fn render_text_mask_aligned(
    text: &str,
    style: &OverlayStyle,
    w: i32,
    h: i32,
    centered: bool,
) -> Mask {
    if w <= 0 || h <= 0 {
        return Mask::new(0, 0);
    }
    let mut mask = Mask::new(w as usize, h as usize);
    if text.trim().is_empty() {
        return mask;
    }
    let byte_len = mask.w * mask.h * 4;

    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return mask;
        }
        let hdc = CreateCompatibleDC(hdc_screen);
        if hdc.is_invalid() {
            ReleaseDC(None, hdc_screen);
            return mask;
        }

        let bmi = bitmap_info(w, h);
        let mut bits: *mut c_void = ptr::null_mut();
        let dib = match CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(hdc);
                ReleaseDC(None, hdc_screen);
                return mask;
            }
        };
        ptr::write_bytes(bits as *mut u8, 0, byte_len);
        let old_bmp = SelectObject(hdc, dib);

        let font = create_font(style);
        let old_font = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(0x00FF_FFFF));

        let pad_x = style.padding_x.clamp(4, 60);
        let pad_y = style.padding_y.clamp(2, 40);
        let mut wide: Vec<u16> = text.encode_utf16().collect();

        let mut calc_rect = RECT {
            left: pad_x,
            top: 0,
            right: (w - pad_x).max(pad_x + 10),
            bottom: 0,
        };
        let alignment = if centered {
            DT_CENTER
        } else {
            Default::default()
        };
        let text_h = DrawTextW(
            hdc,
            &mut wide,
            &mut calc_rect,
            alignment | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT,
        );
        let top_y = ((h - text_h) / 2).max(pad_y);
        let mut draw_rect = RECT {
            left: pad_x,
            top: top_y,
            right: (w - pad_x).max(pad_x + 10),
            bottom: h - pad_y,
        };
        DrawTextW(
            hdc,
            &mut wide,
            &mut draw_rect,
            alignment | DT_WORDBREAK | DT_NOPREFIX,
        );
        let _ = GdiFlush();

        // Grayscale antialiasing: all color channels are equal, so green is the coverage.
        let src = std::slice::from_raw_parts(bits as *const u8, byte_len);
        for (dst, px) in mask.data.iter_mut().zip(src.chunks_exact(4)) {
            *dst = px[1];
        }

        SelectObject(hdc, old_font);
        let _ = DeleteObject(font);
        SelectObject(hdc, old_bmp);
        let _ = DeleteObject(dib);
        let _ = DeleteDC(hdc);
        ReleaseDC(None, hdc_screen);
    }

    mask
}

/// Grows every covered pixel into a roughly circular neighborhood of `radius` pixels,
/// keeping the antialiased edge softness (maximum of overlapping coverage).
pub fn dilate(mask: &Mask, radius: i32) -> Mask {
    let r = radius.clamp(0, MAX_OUTLINE_WIDTH);
    if r == 0 {
        return mask.clone();
    }

    let mut offsets: Vec<(i32, i32)> = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r + 1 {
                offsets.push((dx, dy));
            }
        }
    }

    let (w, h) = (mask.w as i32, mask.h as i32);
    let mut out = Mask::new(mask.w, mask.h);
    for y in 0..h {
        for x in 0..w {
            let m = mask.data[(y * w + x) as usize];
            if m == 0 {
                continue;
            }
            for &(dx, dy) in &offsets {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let idx = (ny * w + nx) as usize;
                if out.data[idx] < m {
                    out.data[idx] = m;
                }
            }
        }
    }
    out
}

/// Anti-aliased rounded-rectangle coverage (0..=255) of the pixel at `(x, y)`.
fn rounded_coverage(x: usize, y: usize, w: usize, h: usize, radius: f32) -> u32 {
    if radius <= 0.0 {
        return 255;
    }
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (wf, hf) = (w as f32, h as f32);
    let cx = px.clamp(radius, wf - radius);
    let cy = py.clamp(radius, hf - radius);
    let dist = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    ((radius - dist + 0.5).clamp(0.0, 1.0) * 255.0).round() as u32
}

fn div255(v: u32) -> u32 {
    (v + 127) / 255
}

/// Builds the premultiplied BGRA frame: a rounded background box of `bg_alpha` opacity with
/// the given layers painted over it. Layer pixels are always opaque where covered, regardless
/// of `bg_alpha`.
pub fn compose(
    w: usize,
    h: usize,
    bg_rgb: [u8; 3],
    bg_alpha: u8,
    radius: i32,
    layers: &[Layer],
) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    if w == 0 || h == 0 {
        return out;
    }

    // `clamp` below panics if min > max, so keep the radius within half of each side.
    let max_radius = (w.min(h) as f32) / 2.0;
    let radius = (radius.max(0) as f32).min(max_radius);

    for y in 0..h {
        for x in 0..w {
            let coverage = rounded_coverage(x, y, w, h, radius);
            let a = div255(bg_alpha as u32 * coverage);
            let i = (y * w + x) * 4;
            out[i] = div255(bg_rgb[2] as u32 * a) as u8;
            out[i + 1] = div255(bg_rgb[1] as u32 * a) as u8;
            out[i + 2] = div255(bg_rgb[0] as u32 * a) as u8;
            out[i + 3] = a as u8;
        }
    }

    let (wi, hi) = (w as i32, h as i32);
    for layer in layers {
        let (mw, mh) = (layer.mask.w as i32, layer.mask.h as i32);
        for my in 0..mh {
            for mx in 0..mw {
                let m = layer.mask.data[(my * mw + mx) as usize] as u32;
                if m == 0 {
                    continue;
                }
                let (x, y) = (mx + layer.dx, my + layer.dy);
                if x < 0 || y < 0 || x >= wi || y >= hi {
                    continue;
                }
                let i = ((y * wi + x) * 4) as usize;
                let inv = 255 - m;
                // Premultiplied "source over": src = color * m, src alpha = m.
                let blend = |src: u8, dst: u8| -> u8 {
                    (div255(src as u32 * m) + div255(dst as u32 * inv)).min(255) as u8
                };
                out[i] = blend(layer.color[2], out[i]);
                out[i + 1] = blend(layer.color[1], out[i + 1]);
                out[i + 2] = blend(layer.color[0], out[i + 2]);
                out[i + 3] = blend(255, out[i + 3]);
            }
        }
    }

    out
}

/// Renders the complete overlay frame (`w` x `h`) for `text` in `style`.
pub fn render_overlay(text: &str, style: &OverlayStyle, w: i32, h: i32) -> Vec<u8> {
    let text_mask = render_text_mask(text, style, w, h);
    render_with_mask(style, w, h, text_mask)
}

/// Aynı premultiplied BGRA çizicisi geçmiş panelinin sola hizalı metnini de üretir.
pub fn render_history_text(text: &str, style: &OverlayStyle, w: i32, h: i32) -> Vec<u8> {
    render_with_mask(
        style,
        w,
        h,
        render_text_mask_aligned(text, style, w, h, false),
    )
}

fn render_with_mask(style: &OverlayStyle, w: i32, h: i32, text_mask: Mask) -> Vec<u8> {
    let outline_width = style.outline_width.clamp(0, MAX_OUTLINE_WIDTH);
    let outline_mask = (outline_width > 0).then(|| dilate(&text_mask, outline_width));

    let mut layers: Vec<Layer> = Vec::new();
    if style.text_shadow {
        layers.push(Layer {
            mask: outline_mask.as_ref().unwrap_or(&text_mask),
            color: [0, 0, 0],
            dx: SHADOW_OFFSET,
            dy: SHADOW_OFFSET,
        });
    }
    if let Some(mask) = &outline_mask {
        layers.push(Layer {
            mask,
            color: hex_to_rgb(&style.outline_color, 0x0000_0000),
            dx: 0,
            dy: 0,
        });
    }
    layers.push(Layer {
        mask: &text_mask,
        color: hex_to_rgb(&style.text_color, 0x00FF_FFFF),
        dx: 0,
        dy: 0,
    });

    compose(
        w.max(0) as usize,
        h.max(0) as usize,
        hex_to_rgb(&style.bg_color, 0x000C_0A0A),
        style.opacity,
        style.corner_radius.clamp(0, 48),
        &layers,
    )
}

/// Shows `bgra` (premultiplied, `w` x `h`) in the layered window at screen position `(x, y)`.
pub fn present(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, bgra: &[u8]) -> Result<()> {
    let len = (w.max(0) as usize) * (h.max(0) as usize) * 4;
    if len == 0 || bgra.len() != len {
        return Err(anyhow!("invalid overlay frame {w}x{h}"));
    }

    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return Err(anyhow!("GetDC failed"));
        }
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        if hdc_mem.is_invalid() {
            ReleaseDC(None, hdc_screen);
            return Err(anyhow!("CreateCompatibleDC failed"));
        }

        let bmi = bitmap_info(w, h);
        let mut bits: *mut c_void = ptr::null_mut();
        let dib = match CreateDIBSection(hdc_screen, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(hdc_mem);
                ReleaseDC(None, hdc_screen);
                return Err(anyhow!("CreateDIBSection failed"));
            }
        };
        ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, len);
        let old_bmp = SelectObject(hdc_mem, dib);

        let pt_dst = POINT { x, y };
        let pt_src = POINT { x: 0, y: 0 };
        let size = SIZE { cx: w, cy: h };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let result = UpdateLayeredWindow(
            hwnd,
            hdc_screen,
            Some(&pt_dst),
            Some(&size),
            hdc_mem,
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(dib);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        result.map_err(|e| anyhow!("UpdateLayeredWindow failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(frame: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    }

    fn mask_with(w: usize, h: usize, points: &[(usize, usize, u8)]) -> Mask {
        let mut m = Mask::new(w, h);
        for &(x, y, v) in points {
            m.data[y * w + x] = v;
        }
        m
    }

    #[test]
    fn text_stays_opaque_when_background_is_invisible() {
        let mask = mask_with(8, 8, &[(4, 4, 255)]);
        let layers = [Layer {
            mask: &mask,
            color: [200, 100, 50],
            dx: 0,
            dy: 0,
        }];
        let frame = compose(8, 8, [10, 20, 30], 0, 0, &layers);

        // BGRA, fully opaque, exact text color.
        assert_eq!(pixel(&frame, 8, 4, 4), [50, 100, 200, 255]);
        // Everything else is fully transparent.
        assert_eq!(pixel(&frame, 8, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn background_uses_its_own_opacity_premultiplied() {
        let frame = compose(4, 4, [255, 0, 0], 128, 0, &[]);
        let [b, g, r, a] = pixel(&frame, 4, 1, 1);
        assert_eq!(a, 128);
        assert_eq!((b, g), (0, 0));
        assert_eq!(r, 128, "premultiplied red = 255 * 128 / 255");
    }

    #[test]
    fn rounded_corners_are_transparent_and_center_is_not() {
        let frame = compose(40, 40, [0, 0, 0], 255, 12, &[]);
        assert_eq!(pixel(&frame, 40, 0, 0)[3], 0, "corner pixel is cut off");
        assert_eq!(pixel(&frame, 40, 20, 20)[3], 255);
        assert_eq!(
            pixel(&frame, 40, 20, 0)[3],
            255,
            "straight edge stays solid"
        );
    }

    #[test]
    fn oversized_radius_does_not_panic() {
        let frame = compose(10, 6, [0, 0, 0], 255, 48, &[]);
        assert_eq!(frame.len(), 10 * 6 * 4);
    }

    #[test]
    fn partial_coverage_blends_over_background() {
        let mask = mask_with(4, 4, &[(2, 2, 128)]);
        let layers = [Layer {
            mask: &mask,
            color: [255, 255, 255],
            dx: 0,
            dy: 0,
        }];
        let frame = compose(4, 4, [0, 0, 0], 0, 0, &layers);
        let [b, _, _, a] = pixel(&frame, 4, 2, 2);
        assert_eq!(a, 128);
        assert_eq!(b, 128);
    }

    #[test]
    fn dilate_grows_but_respects_bounds_and_zero_radius() {
        let mask = mask_with(7, 7, &[(3, 3, 255), (0, 0, 200)]);
        assert_eq!(dilate(&mask, 0).data, mask.data);

        let grown = dilate(&mask, 2);
        assert_eq!(grown.data[3 * 7 + 5], 255, "2px to the right is covered");
        assert_eq!(grown.data[0], 200);
        assert_eq!(grown.data[7 * 7 - 1], 0, "far corner stays empty");
    }

    #[test]
    fn shifted_shadow_layer_moves_pixels() {
        let mask = mask_with(6, 6, &[(1, 1, 255)]);
        let layers = [Layer {
            mask: &mask,
            color: [9, 9, 9],
            dx: 2,
            dy: 2,
        }];
        let frame = compose(6, 6, [0, 0, 0], 0, 0, &layers);
        assert_eq!(pixel(&frame, 6, 3, 3)[3], 255);
        assert_eq!(pixel(&frame, 6, 1, 1)[3], 0);
    }

    #[test]
    fn gdi_text_mask_has_coverage_inside_the_box() {
        let style = OverlayStyle::default();
        let mask = render_text_mask("Hello", &style, 200, 60);
        assert_eq!((mask.w, mask.h), (200, 60));
        assert!(mask.data.iter().any(|&v| v > 200), "glyph cores are solid");
        assert!(mask.data.iter().any(|&v| v == 0), "background is empty");
    }

    #[test]
    fn full_overlay_with_invisible_bg_still_has_opaque_text() {
        let style = OverlayStyle {
            opacity: 0,
            outline_width: 2,
            ..OverlayStyle::default()
        };
        let frame = render_overlay("Hello", &style, 200, 60);
        assert_eq!(frame.len(), 200 * 60 * 4);
        assert!(
            frame.chunks_exact(4).any(|p| p[3] == 255),
            "text must stay opaque at 0% background opacity"
        );
        assert_eq!(pixel(&frame, 200, 0, 0)[3], 0);
    }
}
