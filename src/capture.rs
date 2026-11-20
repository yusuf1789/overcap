use anyhow::{anyhow, Result};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

use crate::config::Region;

pub struct CapturedFrame {
    pub width: i32,
    pub height: i32,
    pub bgra: Vec<u8>,
}

/// Captures a screen rectangle as top-down BGRA8 pixels.
/// Any window with WDA_EXCLUDEFROMCAPTURE is automatically omitted by Windows.
pub fn capture_region(region: Region) -> Result<CapturedFrame> {
    if region.width <= 0 || region.height <= 0 {
        return Err(anyhow!("Invalid capture region: {:?}", region));
    }

    let w = region.width;
    let h = region.height;
    let byte_len = (w as usize) * (h as usize) * 4;

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

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default(); 1],
        };

        let mut bits: *mut c_void = ptr::null_mut();
        let hbitmap = match CreateDIBSection(hdc_mem, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) => bmp,
            Err(e) => {
                let _ = DeleteDC(hdc_mem);
                ReleaseDC(None, hdc_screen);
                return Err(anyhow!("CreateDIBSection failed: {e}"));
            }
        };

        if bits.is_null() {
            let _ = DeleteObject(hbitmap);
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
            return Err(anyhow!("DIBSection buffer allocation failed"));
        }

        let old_obj = SelectObject(hdc_mem, hbitmap);
        let blt_res = BitBlt(
            hdc_mem,
            0,
            0,
            w,
            h,
            hdc_screen,
            region.left,
            region.top,
            SRCCOPY,
        );

        let mut bgra = vec![0u8; byte_len];
        if blt_res.is_ok() {
            ptr::copy_nonoverlapping(bits as *const u8, bgra.as_mut_ptr(), byte_len);
        }

        SelectObject(hdc_mem, old_obj);
        let _ = DeleteObject(hbitmap);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        blt_res.map_err(|e| anyhow!("BitBlt failed: {e}"))?;

        Ok(CapturedFrame {
            width: w,
            height: h,
            bgra,
        })
    }
}

/// Returns the fraction (0.0..=1.0) of sampled pixels where subtitle-like bright pixels
/// changed, ignoring pure dark/mid-tone background camera motion.
pub fn changed_pixel_ratio(prev: &[u8], curr: &[u8]) -> f32 {
    if prev.len() != curr.len() || prev.is_empty() {
        return 1.0;
    }

    // Sample every 2nd pixel (8 bytes stride)
    let mut bright_changed: u64 = 0;
    let mut any_changed: u64 = 0;
    let mut total: u64 = 0;
    let mut i = 0;
    let len = prev.len();

    while i + 2 < len {
        let l1 = (prev[i] as i32 + (prev[i + 1] as i32) * 2 + prev[i + 2] as i32) >> 2;
        let l2 = (curr[i] as i32 + (curr[i + 1] as i32) * 2 + curr[i + 2] as i32) >> 2;
        let delta = (l1 - l2).abs();
        if delta > 24 {
            any_changed += 1;
            // Subtitle glyphs in games/videos are almost always bright (>170 luma)
            let b1 = l1 >= 170;
            let b2 = l2 >= 170;
            if b1 != b2 || (b1 && b2 && delta > 36) {
                bright_changed += 1;
            }
        }
        total += 1;
        i += 8;
    }

    if total == 0 {
        return 0.0;
    }

    let bright_ratio = (bright_changed as f32) / (total as f32);
    // Fallback for rare dark-text subtitles on very bright scenes: if a huge portion changed, still allow
    let any_ratio = (any_changed as f32) / (total as f32);
    if bright_ratio >= 0.0025 {
        bright_ratio
    } else if any_ratio >= 0.12 {
        0.003
    } else {
        0.0
    }
}

/// 2x bilinear upscale for sharper Windows.Media.Ocr accuracy on small game/video subtitles.
/// Skips upscaling if the frame is already very large (> 1600px wide or > 300px tall).
pub fn upscale_for_ocr(frame: &CapturedFrame) -> CapturedFrame {
    if frame.width > 1600 || frame.height > 300 {
        return CapturedFrame {
            width: frame.width,
            height: frame.height,
            bgra: frame.bgra.clone(),
        };
    }

    let sw = frame.width as usize;
    let sh = frame.height as usize;
    let dw = sw * 2;
    let dh = sh * 2;
    let mut out = vec![0u8; dw * dh * 4];
    let src = &frame.bgra;

    for dy in 0..dh {
        let sy0 = (dy / 2).min(sh - 1);
        let sy1 = (sy0 + 1).min(sh - 1);
        let fy = (dy & 1) as u32; // 0 or 1 (0.0 or 0.5 weight)

        let row0 = sy0 * sw * 4;
        let row1 = sy1 * sw * 4;
        let dst_row = dy * dw * 4;

        for dx in 0..dw {
            let sx0 = (dx / 2).min(sw - 1);
            let sx1 = (sx0 + 1).min(sw - 1);
            let fx = (dx & 1) as u32;

            let p00 = row0 + sx0 * 4;
            let p10 = row0 + sx1 * 4;
            let p01 = row1 + sx0 * 4;
            let p11 = row1 + sx1 * 4;
            let d = dst_row + dx * 4;

            for c in 0..3 {
                let v00 = src[p00 + c] as u32;
                let v10 = src[p10 + c] as u32;
                let v01 = src[p01 + c] as u32;
                let v11 = src[p11 + c] as u32;

                let top = if fx == 0 { v00 } else { (v00 + v10) >> 1 };
                let bot = if fx == 0 { v01 } else { (v01 + v11) >> 1 };
                let val = if fy == 0 { top } else { (top + bot) >> 1 };
                out[d + c] = val as u8;
            }
            out[d + 3] = 255;
        }
    }

    CapturedFrame {
        width: dw as i32,
        height: dh as i32,
        bgra: out,
    }
}
