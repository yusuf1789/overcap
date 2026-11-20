use anyhow::Result;
use std::sync::Mutex;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, BitBlt, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_SWISS, FW_BOLD, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes,
    ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, IDC_CROSS, LWA_ALPHA, MSG,
    SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_SHOW,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONDOWN, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::config::Region;

struct SelectorState {
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
    dragging: bool,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    selected: Option<Region>,
}

static STATE: Mutex<Option<SelectorState>> = Mutex::new(None);

fn get_xy(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    (x, y)
}

unsafe extern "system" fn selector_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = get_xy(lparam);
            if let Ok(mut guard) = STATE.lock() {
                if let Some(st) = guard.as_mut() {
                    st.dragging = true;
                    st.x0 = x;
                    st.y0 = y;
                    st.x1 = x;
                    st.y1 = y;
                }
            }
            SetCapture(hwnd);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = get_xy(lparam);
            let mut should_redraw = false;
            if let Ok(mut guard) = STATE.lock() {
                if let Some(st) = guard.as_mut() {
                    if st.dragging {
                        st.x1 = x;
                        st.y1 = y;
                        should_redraw = true;
                    }
                }
            }
            if should_redraw {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = get_xy(lparam);
            let _ = ReleaseCapture();
            let mut done = false;
            if let Ok(mut guard) = STATE.lock() {
                if let Some(st) = guard.as_mut() {
                    if st.dragging {
                        st.dragging = false;
                        let rx1 = st.x0.min(x);
                        let ry1 = st.y0.min(y);
                        let rx2 = st.x0.max(x);
                        let ry2 = st.y0.max(y);
                        let w = rx2 - rx1;
                        let h = ry2 - ry1;
                        if w > 20 && h > 15 {
                            st.selected = Some(Region {
                                left: st.vx + rx1,
                                top: st.vy + ry1,
                                width: w,
                                height: h,
                            });
                            done = true;
                        }
                    }
                }
            }
            if done {
                let _ = DestroyWindow(hwnd);
            } else {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let (vw, vh, dragging, rect) = if let Ok(guard) = STATE.lock() {
                if let Some(st) = guard.as_ref() {
                    (
                        st.vw,
                        st.vh,
                        st.dragging,
                        RECT {
                            left: st.x0.min(st.x1),
                            top: st.y0.min(st.y1),
                            right: st.x0.max(st.x1),
                            bottom: st.y0.max(st.y1),
                        },
                    )
                } else {
                    (0, 0, false, RECT::default())
                }
            } else {
                (0, 0, false, RECT::default())
            };

            if vw > 0 && vh > 0 {
                let mem_dc = CreateCompatibleDC(hdc);
                let mem_bmp = CreateCompatibleBitmap(hdc, vw, vh);
                let old_bmp = SelectObject(mem_dc, mem_bmp);

                let bg_brush = CreateSolidBrush(COLORREF(0x00000000));
                let full_rect = RECT {
                    left: 0,
                    top: 0,
                    right: vw,
                    bottom: vh,
                };
                FillRect(mem_dc, &full_rect, bg_brush);
                let _ = DeleteObject(bg_brush);

                // Instruction banner at top center
                let font = CreateFontW(
                    -24,
                    0,
                    0,
                    0,
                    FW_BOLD.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET.0 as u32,
                    OUT_DEFAULT_PRECIS.0 as u32,
                    CLIP_DEFAULT_PRECIS.0 as u32,
                    CLEARTYPE_QUALITY.0 as u32,
                    (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
                    w!("Segoe UI"),
                );
                let old_font = SelectObject(mem_dc, font);
                SetBkMode(mem_dc, TRANSPARENT);
                SetTextColor(mem_dc, COLORREF(0x0088FF00)); // #00FF88 (BGR)
                let (vx, vy) = STATE
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|s| (s.vx, s.vy)))
                    .unwrap_or((0, 0));
                let primary_w = GetSystemMetrics(SM_CXSCREEN).max(640);
                let mut banner = RECT {
                    left: -vx,
                    top: (-vy) + 40,
                    right: (-vx) + primary_w,
                    bottom: (-vy) + 90,
                };
                let mut msg_w: Vec<u16> =
                    "Drag to select subtitle region   (ESC or Right-Click to cancel)"
                        .encode_utf16()
                        .collect();
                DrawTextW(
                    mem_dc,
                    &mut msg_w,
                    &mut banner,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                SelectObject(mem_dc, old_font);
                let _ = DeleteObject(font);

                if dragging && rect.right > rect.left && rect.bottom > rect.top {
                    let fill_brush = CreateSolidBrush(COLORREF(0x00334422));
                    FillRect(mem_dc, &rect, fill_brush);
                    let _ = DeleteObject(fill_brush);

                    let border_brush = CreateSolidBrush(COLORREF(0x0088FF00));
                    for pad in 0..3 {
                        let r = RECT {
                            left: rect.left + pad,
                            top: rect.top + pad,
                            right: rect.right - pad,
                            bottom: rect.bottom - pad,
                        };
                        FrameRect(mem_dc, &r, border_brush);
                    }
                    let _ = DeleteObject(border_brush);

                    let _ = CreatePen(PS_SOLID, 1, COLORREF(0x0088FF00));
                }

                let _ = BitBlt(hdc, 0, 0, vw, vh, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, old_bmp);
                let _ = DeleteObject(mem_bmp);
                let _ = DeleteDC(mem_dc);
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Opens a fullscreen semi-transparent overlay allowing the user to drag-select a screen region.
pub fn select_region() -> Result<Option<Region>> {
    unsafe {
        let hmodule = GetModuleHandleW(None)?;
        let hinstance = HINSTANCE(hmodule.0);
        let class_name = w!("OvercapRegionSelector");

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(selector_wnd_proc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_CROSS)?,
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        if let Ok(mut guard) = STATE.lock() {
            *guard = Some(SelectorState {
                vx,
                vy,
                vw,
                vh,
                dragging: false,
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
                selected: None,
            });
        }

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            class_name,
            PCWSTR::null(),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            None,
            None,
            hinstance,
            None,
        )?;

        SetLayeredWindowAttributes(hwnd, COLORREF(0), 100, LWA_ALPHA)?;
        let _ = ShowWindow(hwnd, SW_SHOW);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let selected = STATE.lock().ok().and_then(|mut g| g.take()?.selected);
        Ok(selected)
    }
}
