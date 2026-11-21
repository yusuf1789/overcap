//! Son 15 çeviriyi bellekte tutan, oyunun odağını almayan yerel geçmiş paneli.
//! Ana altyazıdan ayrı HWND kullanılır; her iki pencere de OCR yakalamasından hariçtir.

use anyhow::Result;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    VK_NEXT, VK_PRIOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, PostMessageW,
    RegisterClassW, SetWindowDisplayAffinity, SetWindowPos, ShowWindow, HWND_TOPMOST,
    MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE,
    WDA_EXCLUDEFROMCAPTURE, WM_APP, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND,
    WM_HOTKEY, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::config::OverlayStyle;
use crate::overlay::{NoticeLevel, OverlayHandle};
use crate::render::{measure_text_size, present, render_history_text};

pub const HISTORY_LIMIT: usize = 15;
const HK_OLDER: i32 = 101;
const HK_NEWER: i32 = 102;
const WM_REPOSITION: u32 = WM_APP + 20;

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub id: u64,
    pub timestamp: String,
    pub text: String,
}

/// Canlı logdan ayrı küçük halka, son 15 hedef metni ve sıralama kimliğini saklar.
#[derive(Default)]
pub struct HistoryBuffer {
    entries: VecDeque<HistoryEntry>,
    next_id: u64,
}

impl HistoryBuffer {
    pub fn push(&mut self, timestamp: String, text: String) {
        if text.trim().is_empty() {
            return;
        }
        self.next_id += 1;
        self.entries.push_back(HistoryEntry {
            id: self.next_id,
            timestamp,
            text,
        });
        while self.entries.len() > HISTORY_LIMIT {
            self.entries.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn snapshot(&self) -> Vec<HistoryEntry> {
        self.entries.iter().cloned().collect()
    }
}

/// Sıfır uzaklık son satırı takip eder. Kullanıcı yukarıdayken eklenen
/// satırların yüksekliği uzaklığa eklenerek okunan konum korunur.
#[derive(Default)]
struct ScrollPosition {
    gap: i32,
    max: i32,
    viewport: i32,
    tail: Option<u64>,
}

impl ScrollPosition {
    fn layout(&mut self, rows: &[(u64, i32)], viewport: i32) {
        if self.gap > 0 {
            if let Some(tail) = self.tail {
                self.gap = self.gap.saturating_add(
                    rows.iter()
                        .filter(|(id, _)| *id > tail)
                        .map(|(_, h)| h)
                        .sum::<i32>(),
                );
            }
        }
        self.viewport = viewport;
        self.max = (rows.iter().map(|(_, h)| h).sum::<i32>() - viewport).max(0);
        self.gap = self.gap.clamp(0, self.max);
        self.tail = rows.last().map(|(id, _)| *id);
    }

    fn scroll(&mut self, delta: i32) {
        self.gap = self.gap.saturating_add(delta).clamp(0, self.max);
    }
}

struct HistoryWindow {
    hwnd: isize,
    handle: OverlayHandle,
    visible: bool,
    position: RECT,
    scale: f32,
    scroll: ScrollPosition,
}

static WINDOW: Mutex<Option<HistoryWindow>> = Mutex::new(None);

/// Pencere, ana overlay ile aynı Win32 mesaj işçisinde oluşturulur.
/// Yakalama koruması kurulamazsa pencere hiç gösterilmez.
pub fn initialize(handle: OverlayHandle) -> Result<()> {
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = w!("OvercapHistoryWindow");
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class,
            w!("Overcap Translation History"),
            WS_POPUP,
            0,
            0,
            720,
            600,
            None,
            None,
            instance,
            None,
        )?;
        if let Err(error) = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
            let _ = DestroyWindow(hwnd);
            return Err(error.into());
        }
        *WINDOW.lock().unwrap() = Some(HistoryWindow {
            hwnd: hwnd.0 as isize,
            handle,
            visible: false,
            position: RECT::default(),
            scale: 1.0,
            scroll: ScrollPosition::default(),
        });
    }
    Ok(())
}

pub fn toggle() {
    if let Ok(mut state) = WINDOW.lock() {
        if let Some(window) = state.as_mut() {
            if window.visible {
                window.hide();
            } else {
                window.scroll = ScrollPosition::default();
                window.place();
                window.visible = true;
                window.handle.history_visible.store(true, Ordering::SeqCst);
                window.draw();
                if !window.visible {
                    return;
                }
                // Sayfa kısayolları yalnızca panel açıkken alınır; oyun kapalı panelden etkilenmez.
                let mods = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0);
                unsafe {
                    for (id, key) in [(HK_OLDER, VK_PRIOR), (HK_NEWER, VK_NEXT)] {
                        if let Err(error) =
                            RegisterHotKey(HWND(window.hwnd as _), id, mods, key.0 as u32)
                        {
                            window.handle.push_notice(
                                NoticeLevel::Warning,
                                "history",
                                format!("History page shortcut unavailable: {error}"),
                            );
                        }
                    }
                }
            }
        }
    }
}

pub fn refresh() {
    if let Ok(mut state) = WINDOW.lock() {
        if let Some(window) = state.as_mut() {
            if window.visible {
                window.draw();
            }
        }
    }
}

/// HWND yok edilirken mesaj işlevi yeniden çağrılabilir; kilit önce bırakılır.
pub fn shutdown() {
    let window = WINDOW.lock().ok().and_then(|mut s| s.take());
    if let Some(mut window) = window {
        window.hide();
        unsafe {
            let _ = DestroyWindow(HWND(window.hwnd as _));
        }
    }
}

impl HistoryWindow {
    fn hide(&mut self) {
        self.visible = false;
        self.handle.history_visible.store(false, Ordering::SeqCst);
        unsafe {
            let hwnd = HWND(self.hwnd as _);
            let _ = UnregisterHotKey(hwnd, HK_OLDER);
            let _ = UnregisterHotKey(hwnd, HK_NEWER);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    /// Seçili altyazının monitörü tercih edilir; seçim yoksa öndeki pencerenin monitörü kullanılır.
    fn place(&mut self) {
        unsafe {
            let region = self.handle.current_region();
            let monitor = if region.width > 0 && region.height > 0 {
                MonitorFromPoint(
                    POINT {
                        x: region.left + region.width / 2,
                        y: region.top + region.height / 2,
                    },
                    MONITOR_DEFAULTTONEAREST,
                )
            } else {
                MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST)
            };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return;
            }
            self.scale = (GetDpiForWindow(HWND(self.hwnd as _)).max(96) as f32 / 96.0).min(4.0);
            self.position = panel_bounds(info.rcWork, self.scale);
            let _ = SetWindowPos(
                HWND(self.hwnd as _),
                HWND_TOPMOST,
                self.position.left,
                self.position.top,
                self.position.right - self.position.left,
                self.position.bottom - self.position.top,
                SWP_NOACTIVATE,
            );
        }
    }

    fn draw(&mut self) {
        let entries = self
            .handle
            .history
            .lock()
            .map(|b| b.snapshot())
            .unwrap_or_default();
        let w = self.position.right - self.position.left;
        let h = self.position.bottom - self.position.top;
        if w <= 0 || h <= 0 {
            return;
        }
        let frame = render_panel(
            &entries,
            &mut self.scroll,
            w,
            h,
            self.scale,
            self.handle.history_turkish.load(Ordering::SeqCst),
        );
        let result = present(
            HWND(self.hwnd as _),
            self.position.left,
            self.position.top,
            w,
            h,
            &frame,
        );
        if let Err(error) = result {
            self.hide();
            self.handle.push_notice(
                NoticeLevel::Error,
                "history",
                format!("Could not draw history: {error}"),
            );
            return;
        }
        unsafe {
            let _ = SetWindowPos(
                HWND(self.hwnd as _),
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
            );
        }
    }
}

/// Çalışma alanı negatif ekran koordinatlarında da olabilir; panel görev çubuğunu örtmez.
fn panel_bounds(work: RECT, scale: f32) -> RECT {
    let margin = (20.0 * scale) as i32;
    let width = ((720.0 * scale) as i32).min((work.right - work.left - 2 * margin).max(1));
    let height = ((600.0 * scale) as i32).min((work.bottom - work.top - 2 * margin).max(1));
    let right = (work.right - margin).max(work.left + width);
    let top = (work.top + margin).min(work.bottom - height);
    RECT {
        left: right - width,
        top,
        right,
        bottom: top + height,
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Tıklama veya tekerlek kullanımında oyun odağı geçmiş paneline geçmez.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            // Konum güncellemesi sırasında gelen eşzamanlı DPI mesajı kilidi yeniden alamaz.
            let _ = PostMessageW(hwnd, WM_REPOSITION, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }
        WM_REPOSITION => {
            if let Ok(mut state) = WINDOW.lock() {
                if let Some(window) = state.as_mut() {
                    if window.visible {
                        window.place();
                        window.draw();
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_HOTKEY => {
            if let Ok(mut state) = WINDOW.lock() {
                if let Some(window) = state.as_mut() {
                    if window.visible {
                        let delta = if msg == WM_MOUSEWHEEL {
                            ((wparam.0 >> 16) as u16 as i16 as i32) * (48.0 * window.scale) as i32
                                / 120
                        } else {
                            let page =
                                (window.scroll.viewport - (40.0 * window.scale) as i32).max(1);
                            if wparam.0 as i32 == HK_OLDER {
                                page
                            } else {
                                -page
                            }
                        };
                        window.scroll.scroll(delta);
                        window.draw();
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE | WM_RBUTTONUP => {
            hide();
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            if let Ok(mut state) = WINDOW.lock() {
                if let Some(window) = state.as_mut() {
                    let width = window.position.right - window.position.left;
                    if x >= width - (52.0 * window.scale) as i32 && y < (56.0 * window.scale) as i32
                    {
                        window.hide();
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn hide() {
    if let Ok(mut state) = WINDOW.lock() {
        if let Some(window) = state.as_mut() {
            window.hide();
        }
    }
}

/// Yeni fontlar veya çizim bağımlılıkları eklenmez; altyazının mevcut GDI maskesi kullanılır.
fn text_style(size: i32, color: &str, background: &str) -> OverlayStyle {
    OverlayStyle {
        font_family: "Segoe UI".into(),
        font_size: size,
        font_bold: false,
        text_color: color.into(),
        bg_color: background.into(),
        opacity: 255,
        corner_radius: 0,
        padding_x: 14,
        padding_y: 6,
        text_shadow: false,
        outline_width: 0,
        ..Default::default()
    }
}

/// Opak grafik parçaları yalnızca görünür satırlarda kopyalanır; metin panel dışına taşmaz.
fn copy_rows(
    target: &mut [u8],
    width: i32,
    source: &[u8],
    source_width: i32,
    x: i32,
    y: i32,
    clip_top: i32,
    clip_bottom: i32,
) {
    let source_height = source.len() as i32 / (source_width * 4);
    for row in 0..source_height {
        let dest_y = y + row;
        if dest_y < clip_top || dest_y >= clip_bottom {
            continue;
        }
        let count = source_width.min(width - x).max(0) as usize * 4;
        let dest = ((dest_y * width + x) * 4) as usize;
        let src = (row * source_width * 4) as usize;
        if dest + count <= target.len() {
            target[dest..dest + count].copy_from_slice(&source[src..src + count]);
        }
    }
}

fn render_panel(
    entries: &[HistoryEntry],
    scroll: &mut ScrollPosition,
    width: i32,
    height: i32,
    scale: f32,
    turkish: bool,
) -> Vec<u8> {
    let px = |n: f32| (n * scale).round() as i32;
    let header = px(56.0).min(height);
    let footer = px(42.0).min((height - header).max(0));
    let body_end = height - footer;
    let viewport = (body_end - header).max(0);
    let margin = px(12.0).min((width / 4).max(0));
    let row_width = (width - 2 * margin).max(1);
    let body_style = text_style(px(19.0), "#EBEBEB", "#1C1C1C");
    let meta_style = text_style(px(12.0), "#A0A0A0", "#1C1C1C");
    let title_style = text_style(px(17.0), "#EBEBEB", "#141414");
    let footer_style = text_style(px(12.0), "#A0A0A0", "#141414");
    let rows: Vec<_> = entries
        .iter()
        .map(|entry| {
            let (_, text_height) =
                measure_text_size(&entry.text, &body_style, (row_width - 28).max(1));
            (entry.id, px(26.0) + text_height + 12 + px(10.0))
        })
        .collect();
    scroll.layout(&rows, viewport);
    let mut frame = vec![0; width as usize * height as usize * 4];
    for pixel in frame.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[20, 20, 20, 255]);
    }
    let title = format!(
        "{}  ·  {} / {}",
        if turkish {
            "ÇEVİRİ GEÇMİŞİ"
        } else {
            "TRANSLATION HISTORY"
        },
        entries.len(),
        HISTORY_LIMIT
    );
    let title_frame = render_history_text(&title, &title_style, (width - px(52.0)).max(1), header);
    copy_rows(
        &mut frame,
        width,
        &title_frame,
        (width - px(52.0)).max(1),
        0,
        0,
        0,
        height,
    );
    let close_width = px(52.0).min(width);
    let close = render_history_text("X", &title_style, close_width, header);
    copy_rows(
        &mut frame,
        width,
        &close,
        close_width,
        width - close_width,
        0,
        0,
        height,
    );
    let hint = if turkish {
        "Ctrl+Shift+L: Kapat  |  Ctrl+Shift+PgUp/PgDn: Kaydır"
    } else {
        "Ctrl+Shift+L: Close  |  Ctrl+Shift+PgUp/PgDn: Scroll"
    };
    let hint_frame = render_history_text(hint, &footer_style, width, footer);
    copy_rows(
        &mut frame,
        width,
        &hint_frame,
        width,
        0,
        body_end,
        0,
        height,
    );
    let mut y = header - (scroll.max - scroll.gap);
    for (entry, (_, row_height)) in entries.iter().zip(&rows) {
        let painted_height = row_height - px(10.0);
        if y + painted_height > header && y < body_end {
            let meta_height = px(26.0);
            let meta = render_history_text(&entry.timestamp, &meta_style, row_width, meta_height);
            copy_rows(
                &mut frame, width, &meta, row_width, margin, y, header, body_end,
            );
            let text_height = (painted_height - meta_height).max(1);
            let text = render_history_text(&entry.text, &body_style, row_width, text_height);
            copy_rows(
                &mut frame,
                width,
                &text,
                row_width,
                margin,
                y + meta_height,
                header,
                body_end,
            );
        }
        y += row_height;
    }
    if entries.is_empty() {
        let empty = if turkish {
            "Henüz çeviri yok. Çevrilen son 15 replik burada görünecek."
        } else {
            "No translations yet. The last 15 translated lines will appear here."
        };
        let pixels = render_history_text(empty, &title_style, width, viewport);
        copy_rows(
            &mut frame, width, &pixels, width, 0, header, header, body_end,
        );
    }
    // İnce kaydırma göstergesi en yeni satırın altta olduğunu görsel olarak belirtir.
    if scroll.max > 0 && viewport > 0 {
        let thumb = ((viewport as i64 * viewport as i64 / (viewport + scroll.max) as i64) as i32)
            .max(px(20.0))
            .min(viewport);
        let top = header + (scroll.max - scroll.gap) * (viewport - thumb) / scroll.max;
        for y in top..top + thumb {
            for x in (width - px(5.0)).max(0)..width - 1 {
                let index = ((y * width + x) * 4) as usize;
                frame[index..index + 4].copy_from_slice(&[100, 100, 100, 255]);
            }
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sınırlı tampon metni ve zaman damgasını kronolojik sırada korumalıdır.
    #[test]
    fn buffer_keeps_last_fifteen_and_clear_does_not_reuse_ids() {
        let mut buffer = HistoryBuffer::default();
        buffer.push("empty".into(), " ".into());
        for i in 1..=19 {
            buffer.push(format!("00:00:{i:02}"), format!("Replik {i}"));
        }
        let entries = buffer.snapshot();
        assert_eq!(entries.len(), 15);
        assert_eq!(entries[0].text, "Replik 5");
        assert_eq!(entries[14].timestamp, "00:00:19");
        buffer.clear();
        assert!(buffer.snapshot().is_empty());
        buffer.push("later".into(), "Yeni".into());
        assert!(buffer.snapshot()[0].id > entries[14].id);
    }

    /// Baştaki satır düşse de yeni sonuçlar okunan mevcut repliği yerinden oynatmamalıdır.
    #[test]
    fn scroll_preserves_reading_position_and_follows_latest_only_at_bottom() {
        let mut scroll = ScrollPosition::default();
        let rows: Vec<_> = (1..=15).map(|id| (id, 100)).collect();
        scroll.layout(&rows, 300);
        assert_eq!(scroll.gap, 0);
        assert_eq!(scroll.max, 1200);
        scroll.scroll(200);
        let newer: Vec<_> = (2..=16).map(|id| (id, 100)).collect();
        scroll.layout(&newer, 300);
        assert_eq!(scroll.gap, 300);
        scroll.layout(&newer, 300);
        assert_eq!(scroll.gap, 300);
        scroll.scroll(-10000);
        scroll.layout(&(3..=17).map(|id| (id, 100)).collect::<Vec<_>>(), 300);
        assert_eq!(scroll.gap, 0);
        scroll.scroll(10000);
        assert_eq!(scroll.gap, scroll.max);
        scroll.layout(&[], 300);
        assert_eq!(scroll.gap, 0);
        assert_eq!(scroll.max, 0);
    }

    /// İkinci monitör birincinin solunda olsa da konum ve boyut çalışma alanını aşmamalıdır.
    #[test]
    fn placement_fits_negative_and_small_monitor_work_areas() {
        for work in [
            RECT {
                left: -1920,
                top: -200,
                right: 0,
                bottom: 880,
            },
            RECT {
                left: 0,
                top: 0,
                right: 300,
                bottom: 220,
            },
        ] {
            let panel = panel_bounds(work, 2.0);
            assert!(panel.left >= work.left && panel.top >= work.top);
            assert!(panel.right <= work.right && panel.bottom <= work.bottom);
            assert!(panel.right > panel.left && panel.bottom > panel.top);
        }
    }

    /// Görsel kontrol dosyaları doğrudan üretim çizicisinden alınır; başlık, uzun
    /// Türkçe satırlar, boş durum ve kaydırma sınırları aynı testte doğrulanır.
    #[test]
    fn render_history_preview_for_visual_verification() {
        let entries: Vec<_> = (1..=15).map(|id| HistoryEntry {
            id, timestamp: format!("12:30:{id:02}"),
            text: format!("Replik {id}: Buradan hemen çıkmalıyız. Onlar bizi bulmadan önce birlikte güvenli bir yere ulaşalım. Şimdi beni takip et!")
        }).collect();
        let mut scroll = ScrollPosition::default();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        for (name, frame) in [
            (
                "history-empty.bmp",
                render_panel(&[], &mut ScrollPosition::default(), 720, 600, 1.0, false),
            ),
            (
                "history-latest.bmp",
                render_panel(&entries, &mut scroll, 720, 600, 1.0, true),
            ),
        ] {
            assert_eq!(frame.len(), 720 * 600 * 4);
            assert!(frame.chunks_exact(4).all(|pixel| pixel[3] == 255));
            crate::diagnostics::write_bmp(
                &mut std::fs::File::create(path.join(name)).unwrap(),
                720,
                600,
                &frame,
            )
            .unwrap();
        }
        assert!(scroll.max > 0);
        scroll.scroll(scroll.max);
        let older = render_panel(&entries, &mut scroll, 720, 600, 1.0, true);
        crate::diagnostics::write_bmp(
            &mut std::fs::File::create(path.join("history-older.bmp")).unwrap(),
            720,
            600,
            &older,
        )
        .unwrap();
    }

    /// Gerçek HWND üzerinde yakalama koruması ve aç/kapat sırasında odak davranışı doğrulanır.
    #[test]
    fn native_window_excludes_capture_and_never_activates() {
        use crate::config::{AppConfig, Region};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowDisplayAffinity, GetWindowLongPtrW, IsWindowVisible, GWL_EXSTYLE,
        };
        let handle = crate::overlay::create_handle(
            AppConfig::default(),
            Region {
                left: 0,
                top: 0,
                width: 0,
                height: 0,
            },
        );
        initialize(handle.clone()).unwrap();
        let hwnd = HWND(WINDOW.lock().unwrap().as_ref().unwrap().hwnd as _);
        unsafe {
            let mut affinity = 0;
            GetWindowDisplayAffinity(hwnd, &mut affinity).unwrap();
            assert_eq!(affinity, WDA_EXCLUDEFROMCAPTURE.0);
            assert_ne!(
                GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0,
                0
            );
            let foreground = GetForegroundWindow();
            toggle();
            assert_eq!(GetForegroundWindow(), foreground);
            assert!(IsWindowVisible(hwnd).as_bool());
            assert!(handle.history_visible.load(Ordering::SeqCst));
            toggle();
            assert!(!IsWindowVisible(hwnd).as_bool());
            assert!(!handle.history_visible.load(Ordering::SeqCst));
            assert_eq!(GetForegroundWindow(), foreground);
        }
        shutdown();
    }
}
