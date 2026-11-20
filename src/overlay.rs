use anyhow::Result;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateRoundRectRgn,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC, InvalidateRect,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor, SetWindowRgn, UpdateWindow,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CALCRECT, DT_CENTER,
    DT_NOPREFIX, DT_WORDBREAK, FF_SWISS, FW_BOLD, FW_NORMAL, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    VK_A, VK_DOWN, VK_H, VK_Q, VK_S, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMessageW, GetSystemMetrics, PostMessageW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, HICON, HWND_TOPMOST, LWA_ALPHA, MSG, SM_CXSCREEN,
    SM_CYSCREEN, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WDA_EXCLUDEFROMCAPTURE, WM_APP,
    WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_PAINT, WM_RBUTTONUP,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};

use crate::config::{parse_hex_colorref, AppConfig, Region};
use crate::ocr::SourceBounds;
use crate::profiles::foreground_process_name;
use crate::selector::select_region;
use crate::tray::{self, TrayAction, WM_APP_TRAY};

const WM_APP_UPDATE: u32 = WM_APP + 1;
const WM_APP_STYLE: u32 = WM_APP + 2;
const WM_APP_RESELECT: u32 = WM_APP + 3;
const WM_APP_TOGGLE_PAUSE: u32 = WM_APP + 4;
const WM_APP_QUIT: u32 = WM_APP + 5;

const MAX_LOG_ENTRIES: usize = 100;

const HK_QUIT: i32 = 1;
const HK_TOGGLE: i32 = 2;
const HK_RESELECT: i32 = 3;
const HK_FONT_UP: i32 = 4;
const HK_FONT_DOWN: i32 = 5;
const HK_SETTINGS: i32 = 6;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub total_ms: u128,
    pub ocr_ms: u128,
    pub engine: String,
    pub src: String,
    pub dst: String,
}

#[derive(Debug, Clone)]
pub struct OverlayStyle {
    pub font_family: String,
    pub font_size: i32,
    pub font_bold: bool,
    pub text_color: String,
    pub bg_color: String,
    pub opacity: u8,
    pub corner_radius: i32,
    pub text_shadow: bool,
    pub fit_to_text: bool,
    pub padding_x: i32,
    pub padding_y: i32,
}

impl OverlayStyle {
    pub fn from_config(cfg: &AppConfig) -> Self {
        Self {
            font_family: cfg.font_family.clone(),
            font_size: cfg.font_size,
            font_bold: cfg.font_bold,
            text_color: cfg.text_color.clone(),
            bg_color: cfg.bg_color.clone(),
            opacity: cfg.opacity,
            corner_radius: cfg.corner_radius,
            text_shadow: cfg.text_shadow,
            fit_to_text: cfg.fit_to_text,
            padding_x: cfg.padding_x,
            padding_y: cfg.padding_y,
        }
    }
}

pub struct SharedOverlayState {
    pub text: String,
    pub source_bounds: Option<SourceBounds>,
    pub style: OverlayStyle,
    pub paused: bool,
    pub region: Region,
    pub active_app: Option<String>,
    pub config: AppConfig,
}

#[derive(Clone)]
pub struct OverlayHandle {
    hwnd_raw: Arc<Mutex<isize>>,
    pub state: Arc<Mutex<SharedOverlayState>>,
    pub region_changed: Arc<AtomicBool>,
    pub stop_flag: Arc<AtomicBool>,
    pub gui_open_requested: Arc<AtomicBool>,
    pub engine_reload_requested: Arc<AtomicBool>,
    pub last_latency_ms: Arc<AtomicU64>,
    pub logs: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl OverlayHandle {
    fn post_msg(&self, msg: u32) {
        if let Ok(raw) = self.hwnd_raw.lock() {
            if *raw != 0 {
                unsafe {
                    let _ = PostMessageW(HWND(*raw as *mut _), msg, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    pub fn set_text(&self, text: String) {
        self.set_subtitle(text, None);
    }

    pub fn set_subtitle(&self, text: String, source_bounds: Option<SourceBounds>) {
        let changed = if let Ok(mut st) = self.state.lock() {
            if st.text == text && !text.is_empty() {
                false
            } else {
                st.text = text;
                st.source_bounds = source_bounds;
                true
            }
        } else {
            false
        };
        if changed {
            self.post_msg(WM_APP_UPDATE);
        }
    }

    pub fn apply_style(&self, style: OverlayStyle, save_to_config: bool) {
        if let Ok(mut st) = self.state.lock() {
            st.style = style.clone();
            if save_to_config {
                st.config.font_family = style.font_family;
                st.config.font_size = style.font_size;
                st.config.font_bold = style.font_bold;
                st.config.text_color = style.text_color;
                st.config.bg_color = style.bg_color;
                st.config.opacity = style.opacity;
                st.config.corner_radius = style.corner_radius;
                st.config.text_shadow = style.text_shadow;
                st.config.fit_to_text = style.fit_to_text;
                st.config.padding_x = style.padding_x;
                st.config.padding_y = style.padding_y;
                let _ = st.config.save();
            }
        }
        self.post_msg(WM_APP_STYLE);
    }

    pub fn current_region(&self) -> Region {
        self.state.lock().map(|s| s.region).unwrap_or(Region {
            left: 0,
            top: 0,
            width: 100,
            height: 50,
        })
    }

    pub fn is_paused(&self) -> bool {
        self.state.lock().map(|s| s.paused).unwrap_or(false)
    }

    pub fn snapshot_config(&self) -> AppConfig {
        self.state
            .lock()
            .map(|s| s.config.clone())
            .unwrap_or_default()
    }

    pub fn update_config_and_save(&self, new_cfg: AppConfig) -> Result<()> {
        let style = OverlayStyle::from_config(&new_cfg);
        new_cfg.save()?;
        if let Ok(mut st) = self.state.lock() {
            st.style = style;
            st.config = new_cfg;
        }
        self.post_msg(WM_APP_STYLE);
        Ok(())
    }

    pub fn set_active_app(&self, app: Option<String>) {
        if let Ok(mut st) = self.state.lock() {
            st.active_app = app;
        }
    }

    pub fn active_app(&self) -> Option<String> {
        self.state.lock().ok().and_then(|s| s.active_app.clone())
    }

    pub fn set_region_from_profile(&self, region: Region) {
        let changed = if let Ok(mut st) = self.state.lock() {
            if st.region == region {
                false
            } else {
                st.region = region;
                st.config.region = Some(region);
                true
            }
        } else {
            false
        };
        if changed {
            self.region_changed.store(true, Ordering::SeqCst);
            self.post_msg(WM_APP_STYLE);
        }
    }

    pub fn push_log(&self, entry: LogEntry) {
        self.last_latency_ms
            .store(entry.total_ms as u64, Ordering::SeqCst);
        if let Ok(mut logs) = self.logs.lock() {
            while logs.len() >= MAX_LOG_ENTRIES {
                logs.pop_front();
            }
            logs.push_back(entry);
        }
    }

    pub fn recent_logs(&self) -> Vec<LogEntry> {
        self.logs
            .lock()
            .map(|logs| logs.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear_logs(&self) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.clear();
        }
    }

    pub fn request_open_gui(&self) {
        self.gui_open_requested.store(true, Ordering::SeqCst);
    }

    pub fn request_engine_reload(&self) {
        self.engine_reload_requested.store(true, Ordering::SeqCst);
    }

    pub fn trigger_reselect(&self) {
        self.post_msg(WM_APP_RESELECT);
    }

    pub fn toggle_pause(&self) {
        self.post_msg(WM_APP_TOGGLE_PAUSE);
    }

    #[allow(dead_code)]
    pub fn request_quit(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.post_msg(WM_APP_QUIT);
    }

    pub fn delete_profile(&self, app: &str) -> Result<()> {
        if let Ok(mut st) = self.state.lock() {
            st.config.profiles.remove(app);
            if st.active_app.as_deref() == Some(app) {
                st.active_app = None;
            }
            st.config.save()?;
        }
        Ok(())
    }

    pub fn set_test_preview(&self, show: bool, text: Option<String>) {
        if show {
            let preview = text.unwrap_or_else(|| "Overcap live subtitle preview".to_string());
            self.set_subtitle(preview, None);
        } else {
            self.set_subtitle(String::new(), None);
        }
    }
}

static OVERLAY_HANDLE: Mutex<Option<OverlayHandle>> = Mutex::new(None);
static TRAY_ICON: Mutex<isize> = Mutex::new(0);

fn measure_text_size(text: &str, style: &OverlayStyle, max_width: i32) -> (i32, i32) {
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return (max_width, style.font_size + 12);
        }
        let mut face_w: Vec<u16> = style.font_family.encode_utf16().collect();
        face_w.push(0);
        let weight = if style.font_bold { FW_BOLD.0 } else { FW_NORMAL.0 } as i32;

        let font = CreateFontW(
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
            CLEARTYPE_QUALITY.0 as u32,
            (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
            PCWSTR(face_w.as_ptr()),
        );
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

fn apply_window_shape_and_alpha(hwnd: HWND, w: i32, h: i32, opacity: u8, radius: i32) {
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), opacity, LWA_ALPHA);
        if radius > 0 {
            let r = radius.clamp(0, 48);
            let hrgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, r * 2, r * 2);
            let _ = SetWindowRgn(hwnd, hrgn, true);
        } else {
            let _ = SetWindowRgn(hwnd, None, true);
        }
    }
}

fn apply_visibility_and_redraw(hwnd: HWND) {
    let snapshot = OVERLAY_HANDLE.lock().ok().and_then(|g| {
        let h = g.as_ref()?;
        let st = h.state.lock().ok()?;
        Some((
            st.paused,
            st.text.clone(),
            st.source_bounds,
            st.style.clone(),
            st.region,
        ))
    });

    let Some((paused, text, source_bounds, style, region)) = snapshot else {
        return;
    };

    let should_show = !paused && !text.trim().is_empty();

    unsafe {
        if should_show {
            // If no capture region has been selected yet, fall back to bottom-center of the primary screen
            // so Overlay Preview works safely without clamping panics.
            let region = if region.width >= 60 && region.height >= 32 {
                region
            } else {
                let sw = GetSystemMetrics(SM_CXSCREEN).max(800);
                let sh = GetSystemMetrics(SM_CYSCREEN).max(600);
                let rw = (sw * 60 / 100).clamp(480, 1200);
                let rh = 110;
                Region {
                    left: (sw - rw) / 2,
                    top: (sh - rh - 80).max(40),
                    width: rw,
                    height: rh,
                }
            };

            let pad_x = style.padding_x.clamp(4, 60);
            let pad_y = style.padding_y.clamp(2, 40);

            let (box_x, box_y, box_w, box_h) = if style.fit_to_text {
                let max_text_w = (region.width - pad_x * 2).max(60);
                let (tw, th) = measure_text_size(text.trim(), &style, max_text_w);

                // Ensure the overlay box is at least large enough to fully cover the
                // underlying source subtitle (e.g. 2-line EN under 1-line TR).
                let (min_cover_w, min_cover_h, anchor_cx, anchor_cy) = match source_bounds {
                    Some(sb) => (
                        sb.width + 18,
                        sb.height + 14,
                        region.left + sb.x + sb.width / 2,
                        region.top + sb.y + sb.height / 2,
                    ),
                    None => (
                        0,
                        0,
                        region.left + region.width / 2,
                        region.top + region.height / 2,
                    ),
                };

                let bw = (tw + pad_x * 2 + 10).max(min_cover_w).min(region.width).max(60);
                let bh = (th + pad_y * 2 + 4).max(min_cover_h).min(region.height).max(32);

                let max_x = (region.left + region.width - bw).max(region.left);
                let max_y = (region.top + region.height - bh).max(region.top);
                let bx = (anchor_cx - bw / 2).clamp(region.left, max_x);
                let by = (anchor_cy - bh / 2).clamp(region.top, max_y);
                (bx, by, bw, bh)
            } else {
                (region.left, region.top, region.width, region.height)
            };

            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                box_x,
                box_y,
                box_w,
                box_h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            apply_window_shape_and_alpha(hwnd, box_w, box_h, style.opacity, style.corner_radius);
            let _ = InvalidateRect(hwnd, None, false);
            let _ = UpdateWindow(hwnd);
        } else {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn handle_quit(hwnd: HWND) {
    if let Ok(guard) = OVERLAY_HANDLE.lock() {
        if let Some(h) = guard.as_ref() {
            h.stop_flag.store(true, Ordering::SeqCst);
        }
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn handle_toggle_pause(hwnd: HWND) -> bool {
    let paused = if let Ok(guard) = OVERLAY_HANDLE.lock() {
        if let Some(h) = guard.as_ref() {
            if let Ok(mut st) = h.state.lock() {
                st.paused = !st.paused;
                st.paused
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };
    apply_visibility_and_redraw(hwnd);
    paused
}

fn handle_reselect(hwnd: HWND) {
    let detected_app = foreground_process_name();
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    if let Ok(Some(new_region)) = select_region() {
        println!("[Region] New region: {:?}", new_region);
        if let Ok(guard) = OVERLAY_HANDLE.lock() {
            if let Some(h) = guard.as_ref() {
                if let Ok(mut st) = h.state.lock() {
                    st.region = new_region;
                    st.config.region = Some(new_region);
                    if let Some(ref app) = detected_app {
                        st.active_app = Some(app.clone());
                        st.config.profiles.insert(app.clone(), new_region);
                        println!("[Profile] Saved region for {}", app);
                    }
                    let _ = st.config.save();
                }
                h.region_changed.store(true, Ordering::SeqCst);
            }
        }
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                new_region.left,
                new_region.top,
                new_region.width,
                new_region.height,
                SWP_NOACTIVATE,
            );
        }
    }
    apply_visibility_and_redraw(hwnd);
}

fn handle_open_gui() {
    if let Ok(guard) = OVERLAY_HANDLE.lock() {
        if let Some(h) = guard.as_ref() {
            h.request_open_gui();
        }
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_APP_UPDATE | WM_APP_STYLE => {
            apply_visibility_and_redraw(hwnd);
            LRESULT(0)
        }
        WM_APP_RESELECT => {
            handle_reselect(hwnd);
            LRESULT(0)
        }
        WM_APP_TOGGLE_PAUSE => {
            let _ = handle_toggle_pause(hwnd);
            LRESULT(0)
        }
        WM_APP_QUIT => {
            handle_quit(hwnd);
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let event = lparam.0 as u32;
            match event {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    handle_open_gui();
                }
                WM_RBUTTONUP => {
                    let paused = OVERLAY_HANDLE
                        .lock()
                        .ok()
                        .and_then(|g| g.as_ref().map(|h| h.is_paused()))
                        .unwrap_or(false);
                    if let Some(action) = tray::show_tray_menu(hwnd, paused) {
                        match action {
                            TrayAction::OpenSettings => handle_open_gui(),
                            TrayAction::SelectRegion => handle_reselect(hwnd),
                            TrayAction::TogglePause => {
                                let _ = handle_toggle_pause(hwnd);
                            }
                            TrayAction::Quit => handle_quit(hwnd),
                        }
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            let id = wparam.0 as i32;
            match id {
                HK_QUIT => {
                    println!("[Hotkey] Ctrl+Shift+Q -> Shutting down...");
                    handle_quit(hwnd);
                }
                HK_TOGGLE => {
                    let paused = handle_toggle_pause(hwnd);
                    println!(
                        "[Hotkey] Ctrl+Shift+H -> {}",
                        if paused { "Paused (hidden)" } else { "Resumed" }
                    );
                }
                HK_RESELECT => {
                    println!("[Hotkey] Ctrl+Shift+S -> Selecting new subtitle region...");
                    handle_reselect(hwnd);
                }
                HK_FONT_UP | HK_FONT_DOWN => {
                    let delta = if id == HK_FONT_UP { 2 } else { -2 };
                    if let Ok(guard) = OVERLAY_HANDLE.lock() {
                        if let Some(h) = guard.as_ref() {
                            if let Ok(mut st) = h.state.lock() {
                                st.style.font_size = (st.style.font_size + delta).clamp(14, 64);
                                st.config.font_size = st.style.font_size;
                                let _ = st.config.save();
                                println!("[Hotkey] Font size: {}", st.style.font_size);
                            }
                        }
                    }
                    let _ = InvalidateRect(hwnd, None, false);
                    let _ = UpdateWindow(hwnd);
                }
                HK_SETTINGS => {
                    println!("[Hotkey] Ctrl+Shift+A -> Opening Overcap window...");
                    handle_open_gui();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let w = client.right - client.left;
            let h = client.bottom - client.top;

            let (text, style) = OVERLAY_HANDLE
                .lock()
                .ok()
                .and_then(|g| {
                    let handle = g.as_ref()?;
                    let st = handle.state.lock().ok()?;
                    Some((st.text.clone(), st.style.clone()))
                })
                .unwrap_or_else(|| (String::new(), OverlayStyle::from_config(&AppConfig::default())));

            if w > 0 && h > 0 {
                let mem_dc = CreateCompatibleDC(hdc);
                let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
                let old_bmp = SelectObject(mem_dc, mem_bmp);

                let bg_cr = parse_hex_colorref(&style.bg_color, 0x000C0A0A);
                let bg = CreateSolidBrush(COLORREF(bg_cr));
                FillRect(mem_dc, &client, bg);
                let _ = DeleteObject(bg);

                if !text.trim().is_empty() {
                    let mut face_w: Vec<u16> = style.font_family.encode_utf16().collect();
                    face_w.push(0);
                    let weight = if style.font_bold { FW_BOLD.0 } else { FW_NORMAL.0 } as i32;

                    let font = CreateFontW(
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
                        CLEARTYPE_QUALITY.0 as u32,
                        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
                        PCWSTR(face_w.as_ptr()),
                    );
                    let old_font = SelectObject(mem_dc, font);
                    SetBkMode(mem_dc, TRANSPARENT);

                    let pad_x = style.padding_x.clamp(4, 60);
                    let pad_y = style.padding_y.clamp(2, 40);
                    let mut wide: Vec<u16> = text.encode_utf16().collect();

                    let mut calc_rect = RECT {
                        left: pad_x,
                        top: 0,
                        right: (w - pad_x).max(pad_x + 10),
                        bottom: 0,
                    };
                    let text_h = DrawTextW(
                        mem_dc,
                        &mut wide,
                        &mut calc_rect,
                        DT_CENTER | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT,
                    );

                    let top_y = ((h - text_h) / 2).max(pad_y);

                    if style.text_shadow {
                        SetTextColor(mem_dc, COLORREF(0x00000000));
                        let mut shadow_rect = RECT {
                            left: pad_x + 2,
                            top: top_y + 2,
                            right: (w - pad_x + 2).max(pad_x + 12),
                            bottom: h - pad_y + 2,
                        };
                        DrawTextW(
                            mem_dc,
                            &mut wide,
                            &mut shadow_rect,
                            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
                        );
                    }

                    let fg_cr = parse_hex_colorref(&style.text_color, 0x00FFFFFF);
                    SetTextColor(mem_dc, COLORREF(fg_cr));
                    let mut draw_rect = RECT {
                        left: pad_x,
                        top: top_y,
                        right: (w - pad_x).max(pad_x + 10),
                        bottom: h - pad_y,
                    };
                    DrawTextW(
                        mem_dc,
                        &mut wide,
                        &mut draw_rect,
                        DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
                    );

                    SelectObject(mem_dc, old_font);
                    let _ = DeleteObject(font);
                }

                let _ = BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, old_bmp);
                let _ = DeleteObject(mem_bmp);
                let _ = DeleteDC(mem_dc);
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = UnregisterHotKey(hwnd, HK_QUIT);
            let _ = UnregisterHotKey(hwnd, HK_TOGGLE);
            let _ = UnregisterHotKey(hwnd, HK_RESELECT);
            let _ = UnregisterHotKey(hwnd, HK_FONT_UP);
            let _ = UnregisterHotKey(hwnd, HK_FONT_DOWN);
            let _ = UnregisterHotKey(hwnd, HK_SETTINGS);
            tray::remove_tray_icon(hwnd);
            if let Ok(mut icon_guard) = TRAY_ICON.lock() {
                if *icon_guard != 0 {
                    let _ = DestroyIcon(HICON(*icon_guard as *mut _));
                    *icon_guard = 0;
                }
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn create_handle(config: AppConfig, region: Region) -> OverlayHandle {
    let style = OverlayStyle::from_config(&config);
    OverlayHandle {
        hwnd_raw: Arc::new(Mutex::new(0)),
        state: Arc::new(Mutex::new(SharedOverlayState {
            text: String::new(),
            source_bounds: None,
            style,
            paused: false,
            region,
            active_app: None,
            config,
        })),
        region_changed: Arc::new(AtomicBool::new(false)),
        stop_flag: Arc::new(AtomicBool::new(false)),
        gui_open_requested: Arc::new(AtomicBool::new(false)),
        engine_reload_requested: Arc::new(AtomicBool::new(false)),
        last_latency_ms: Arc::new(AtomicU64::new(0)),
        logs: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES))),
    }
}

pub fn run_overlay_loop(handle: OverlayHandle) -> Result<()> {
    let region = {
        let st = handle.state.lock().unwrap();
        st.region
    };

    if let Ok(mut guard) = OVERLAY_HANDLE.lock() {
        *guard = Some(handle.clone());
    }

    unsafe {
        let hmodule = GetModuleHandleW(None)?;
        let hinstance = HINSTANCE(hmodule.0);
        let class_name = w!("OvercapOverlayWindow");

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let ex_style = WS_EX_TOPMOST
            | WS_EX_LAYERED
            | WS_EX_TRANSPARENT
            | WS_EX_NOACTIVATE
            | WS_EX_TOOLWINDOW;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            w!("Overcap"),
            WS_POPUP,
            region.left,
            region.top,
            region.width,
            region.height,
            None,
            None,
            hinstance,
            None,
        )?;

        if let Ok(mut raw) = handle.hwnd_raw.lock() {
            *raw = hwnd.0 as isize;
        }

        // CRITICAL: Exclude this overlay window from screen capture so OCR always sees
        // the underlying original subtitle underneath our translated overlay.
        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)?;
        let (opacity, radius) = {
            let st = handle.state.lock().unwrap();
            (st.style.opacity, st.style.corner_radius)
        };
        apply_window_shape_and_alpha(hwnd, region.width, region.height, opacity, radius);

        let mods = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0);
        let _ = RegisterHotKey(hwnd, HK_QUIT, mods, VK_Q.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_TOGGLE, mods, VK_H.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_RESELECT, mods, VK_S.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_FONT_UP, mods, VK_UP.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_FONT_DOWN, mods, VK_DOWN.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_SETTINGS, mods, VK_A.0 as u32);

        if let Ok(hicon) = tray::create_hicon(32) {
            if tray::add_tray_icon(hwnd, hicon).is_ok() {
                if let Ok(mut icon_guard) = TRAY_ICON.lock() {
                    *icon_guard = hicon.0 as isize;
                }
            } else {
                let _ = DestroyIcon(hicon);
            }
        }

        // Start hidden until a subtitle appears
        let _ = ShowWindow(hwnd, SW_HIDE);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    handle.stop_flag.store(true, Ordering::SeqCst);
    Ok(())
}
