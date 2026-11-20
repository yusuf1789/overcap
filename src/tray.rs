use anyhow::{anyhow, Result};
use std::ffi::c_void;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyMenu, GetCursorPos, PostMessageW,
    SetForegroundWindow, TrackPopupMenu, HICON, ICONINFO, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_APP, WM_NULL,
};

pub const WM_APP_TRAY: u32 = WM_APP + 10;

const IDM_OPEN: usize = 1001;
const IDM_RESELECT: usize = 1002;
const IDM_PAUSE: usize = 1003;
const IDM_QUIT: usize = 1004;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenSettings,
    SelectRegion,
    TogglePause,
    Quit,
}

/// Generates a minimal geometric RGBA icon: dark graphite square (`#1A1A1A`)
/// with a 1px `#444444` border and two crisp white subtitle bars near the bottom.
pub fn generate_icon_rgba(size: u32) -> Vec<u8> {
    let s = size.max(8);
    let mut pixels = vec![0u8; (s * s * 4) as usize];

    let bar_h = (s / 8).max(2);
    let gap = (s / 16).max(1);
    let bottom_margin = (s / 6).max(3);
    let margin_x = (s / 5).max(3);
    let margin_x2 = (margin_x + (s / 10).max(1)).min(s / 2 - 1);

    let bar2_y0 = s.saturating_sub(bottom_margin + bar_h);
    let bar2_y1 = s.saturating_sub(bottom_margin);
    let bar1_y0 = bar2_y0.saturating_sub(gap + bar_h);
    let bar1_y1 = bar2_y0.saturating_sub(gap);

    for y in 0..s {
        for x in 0..s {
            let is_border = x == 0 || y == 0 || x == s - 1 || y == s - 1;
            let in_bar1 = !is_border
                && y >= bar1_y0
                && y < bar1_y1
                && x >= margin_x
                && x < s - margin_x;
            let in_bar2 = !is_border
                && y >= bar2_y0
                && y < bar2_y1
                && x >= margin_x2
                && x < s - margin_x2;

            let (r, g, b, a) = if is_border {
                (0x44, 0x44, 0x44, 0xFF)
            } else if in_bar1 || in_bar2 {
                (0xFF, 0xFF, 0xFF, 0xFF)
            } else {
                (0x1A, 0x1A, 0x1A, 0xFF)
            };

            let idx = ((y * s + x) * 4) as usize;
            pixels[idx] = r;
            pixels[idx + 1] = g;
            pixels[idx + 2] = b;
            pixels[idx + 3] = a;
        }
    }

    pixels
}

/// Builds a Win32 `HICON` from `generate_icon_rgba` using `CreateDIBSection` and `CreateIconIndirect`.
pub fn create_hicon(size: i32) -> Result<HICON> {
    let s = size.max(8);
    let rgba = generate_icon_rgba(s as u32);

    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: s,
                biHeight: -s, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm_color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        if bits.is_null() {
            let _ = DeleteObject(hbm_color);
            return Err(anyhow!("CreateDIBSection returned null pixel buffer"));
        }

        // Convert RGBA to BGRA for Win32 32-bit DIBs.
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (s * s * 4) as usize);
        for (src_px, dst_px) in rgba.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            dst_px[0] = src_px[2];
            dst_px[1] = src_px[1];
            dst_px[2] = src_px[0];
            dst_px[3] = src_px[3];
        }

        let hbm_mask = CreateBitmap(s, s, 1, 1, None);
        if hbm_mask.is_invalid() {
            let _ = DeleteObject(hbm_color);
            return Err(anyhow!("CreateBitmap failed for icon mask"));
        }

        let ii = ICONINFO {
            fIcon: BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let hicon = CreateIconIndirect(&ii);

        let _ = DeleteObject(hbm_color);
        let _ = DeleteObject(hbm_mask);

        Ok(hicon?)
    }
}

/// Registers the system tray icon for `hwnd`.
pub fn add_tray_icon(hwnd: HWND, hicon: HICON) -> Result<()> {
    let mut tip = [0u16; 128];
    for (i, ch) in "Overcap".encode_utf16().enumerate().take(127) {
        tip[i] = ch;
    }

    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: hicon,
        szTip: tip,
        ..Default::default()
    };

    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            return Err(anyhow!("Shell_NotifyIconW(NIM_ADD) failed"));
        }
    }
    Ok(())
}

/// Removes the system tray icon associated with `hwnd`.
pub fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn to_wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Displays the tray context menu at the cursor position and returns the chosen action.
pub fn show_tray_menu(hwnd: HWND, is_paused: bool) -> Option<TrayAction> {
    unsafe {
        let hmenu = CreatePopupMenu().ok()?;

        let open_w = to_wide_z("Open Overcap");
        let reselect_w = to_wide_z("Select Region (Ctrl+Shift+S)");
        let pause_w = to_wide_z(if is_paused { "Resume" } else { "Pause" });
        let quit_w = to_wide_z("Quit (Ctrl+Shift+Q)");

        let _ = AppendMenuW(hmenu, MF_STRING, IDM_OPEN, PCWSTR(open_w.as_ptr()));
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_RESELECT, PCWSTR(reselect_w.as_ptr()));
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_PAUSE, PCWSTR(pause_w.as_ptr()));
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_QUIT, PCWSTR(quit_w.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            hmenu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(hmenu);

        match cmd {
            IDM_OPEN => Some(TrayAction::OpenSettings),
            IDM_RESELECT => Some(TrayAction::SelectRegion),
            IDM_PAUSE => Some(TrayAction::TogglePause),
            IDM_QUIT => Some(TrayAction::Quit),
            _ => None,
        }
    }
}
