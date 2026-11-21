use anyhow::Result;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    VK_A, VK_DOWN, VK_H, VK_L, VK_Q, VK_S, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, PostMessageW, PostQuitMessage, RegisterClassW, SetWindowDisplayAffinity,
    SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HICON, HWND_TOPMOST, MSG,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY,
    WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_PAINT, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::config::{AppConfig, OutputMode, OverlayStyle, Region, MAX_OUTLINE_WIDTH};
use crate::ocr::SourceBounds;
use crate::profiles::foreground_process_name;
use crate::render::{measure_text_size, present, render_overlay};
use crate::selector::{select_region_with_options, SelectorOptions, OUTPUT_BORDER_COLORREF};
use crate::tray::{self, TrayAction, WM_APP_TRAY};

const WM_APP_UPDATE: u32 = WM_APP + 1;
const WM_APP_STYLE: u32 = WM_APP + 2;
const WM_APP_RESELECT: u32 = WM_APP + 3;
const WM_APP_TOGGLE_PAUSE: u32 = WM_APP + 4;
const WM_APP_QUIT: u32 = WM_APP + 5;
const WM_APP_HISTORY_TOGGLE: u32 = WM_APP + 6;
const WM_APP_HISTORY_REFRESH: u32 = WM_APP + 7;
const WM_APP_STREAM_MODE: u32 = WM_APP + 8;
const WM_APP_SELECT_OUTPUT: u32 = WM_APP + 9;

const MAX_LOG_ENTRIES: usize = 100;
const MAX_NOTICES: usize = 50;
const MAX_NOTICE_CHARS: usize = 300;

const HK_QUIT: i32 = 1;
const HK_TOGGLE: i32 = 2;
const HK_RESELECT: i32 = 3;
const HK_FONT_UP: i32 = 4;
const HK_FONT_DOWN: i32 = 5;
const HK_SETTINGS: i32 = 6;
const HK_HISTORY: i32 = 7;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub total_ms: u128,
    pub ocr_ms: u128,
    /// Bekleme, OCR tamamlandıktan çeviri çağrısının başlamasına kadar ölçülür.
    pub wait_ms: u128,
    pub translation_ms: u128,
    pub report: crate::translator::TranslationReport,
    pub engine: String,
    pub src: String,
    pub dst: String,
}

/// Kaydedilen tercihten bağımsız gerçek işçi durumu; model anahtarı içermez.
#[derive(Clone, Debug, Default)]
pub struct EngineStatus {
    pub ready: String,
    pub model: String,
    pub loading: bool,
    pub message: Option<String>,
    pub last_result: Option<crate::translator::TranslationReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Warning,
    Error,
}

/// A problem worth showing to the user (config reset, engine fallback, save failure, ...).
/// Release builds have no console, so this is the only place such errors become visible.
#[derive(Debug, Clone)]
pub struct Notice {
    /// Increases with every new (non-merged) notice; lets the GUI track what was dismissed.
    pub id: u64,
    pub timestamp: String,
    pub level: NoticeLevel,
    pub source: String,
    pub message: String,
    /// How many times this identical notice happened in a row.
    pub count: u32,
}

pub struct SharedOverlayState {
    pub text: String,
    pub source_bounds: Option<SourceBounds>,
    pub style: OverlayStyle,
    pub paused: bool,
    pub region: Region,
    /// Foreground app right now; `None` while Overcap's own window (or nothing usable) is focused.
    pub active_app: Option<String>,
    /// Most recent foreground app other than Overcap, so the GUI can offer per-app styles
    /// even though Overcap itself is focused while the user edits.
    pub last_app: Option<String>,
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
    /// Nesil sayacı, motor değişirken devam eden eski sonuçların sunulmasını engeller.
    pub engine_revision: Arc<AtomicU64>,
    pub engine_status: Arc<Mutex<EngineStatus>>,
    pub ocr_diagnostics: Arc<AtomicBool>,
    pub last_latency_ms: Arc<AtomicU64>,
    pub logs: Arc<Mutex<VecDeque<LogEntry>>>,
    /// Oyun içi görünüm yalnızca son 15 hedef metni tutar; diske yazılmaz.
    pub history: Arc<Mutex<crate::history::HistoryBuffer>>,
    pub history_visible: Arc<AtomicBool>,
    pub history_turkish: Arc<AtomicBool>,
    pending_output_target_app: Arc<Mutex<Option<String>>>,
    notices: Arc<Mutex<VecDeque<Notice>>>,
    next_notice_id: Arc<AtomicU64>,
}

impl OverlayHandle {
    /// Teşhis yalnızca zaten gösterilen metnin bir kopyasını alır.
    pub fn snapshot_text(&self) -> String {
        self.state
            .lock()
            .map(|s| s.text.clone())
            .unwrap_or_default()
    }
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

    /// Applies `style` live and, if `save_to_config`, stores it as the global style.
    pub fn apply_style(&self, style: OverlayStyle, save_to_config: bool) {
        if let Ok(mut st) = self.state.lock() {
            if save_to_config {
                style.write_to_config(&mut st.config);
                self.save_config_or_notice(&st.config);
            }
            st.style = style;
        }
        self.post_msg(WM_APP_STYLE);
    }

    /// Applies `style` live and stores it as the style for `app` (lowercase `.exe` name).
    pub fn apply_app_style(&self, app: &str, style: OverlayStyle) {
        if let Ok(mut st) = self.state.lock() {
            st.config
                .profile_styles
                .insert(app.to_string(), style.clone());
            self.save_config_or_notice(&st.config);
            st.style = style;
        }
        self.post_msg(WM_APP_STYLE);
    }

    /// Removes the per-app style; the overlay falls back to the global style.
    pub fn remove_app_style(&self, app: &str) {
        if let Ok(mut st) = self.state.lock() {
            if st.config.profile_styles.remove(app).is_some() {
                self.save_config_or_notice(&st.config);
                st.style = OverlayStyle::from_config(&st.config);
            }
        }
        self.post_msg(WM_APP_STYLE);
    }

    /// Switches the live style to the one that belongs to `app` (its own style, or the global
    /// one). Does not touch the config.
    pub fn activate_style_for_app(&self, app: &str) {
        let changed = if let Ok(mut st) = self.state.lock() {
            let style = st.config.style_for_app(Some(app));
            if style != st.style {
                st.style = style;
                true
            } else {
                false
            }
        } else {
            false
        };
        if changed {
            self.post_msg(WM_APP_STYLE);
        }
    }

    /// Changes the font size of the style currently in effect (the active app's own style if
    /// it has one, otherwise the global style), saves it, and returns the new size.
    fn adjust_font_size(&self, delta: i32) -> Option<i32> {
        let mut st = self.state.lock().ok()?;
        let new_size = (st.style.font_size + delta).clamp(14, 64);
        let app_key = st
            .active_app
            .clone()
            .filter(|a| st.config.auto_profiles && st.config.profile_styles.contains_key(a));

        match app_key {
            Some(app) => {
                if let Some(s) = st.config.profile_styles.get_mut(&app) {
                    s.font_size = new_size;
                }
            }
            None => st.config.font_size = new_size,
        }
        st.style.font_size = new_size;
        self.save_config_or_notice(&st.config);
        drop(st);
        self.post_msg(WM_APP_STYLE);
        Some(new_size)
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

    pub fn is_stream_mode(&self) -> bool {
        self.state
            .lock()
            .map(|s| s.config.stream_mode)
            .unwrap_or(false)
    }

    pub fn set_stream_mode(&self, enabled: bool) {
        let save_result = if let Ok(mut st) = self.state.lock() {
            st.config.stream_mode = enabled;
            st.config.save()
        } else {
            Ok(())
        };
        if let Err(e) = save_result {
            self.push_notice(
                NoticeLevel::Error,
                "config",
                format!("Failed to save stream_mode: {e}"),
            );
        }
        self.post_msg(WM_APP_STREAM_MODE);
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
            // Windows shell processes take focus all the time (taskbar, Start, search) and are
            // never a subtitle source, so keep them out of the per-app style list.
            const SHELL_PROCESSES: &[&str] = &[
                "explorer.exe",
                "shellexperiencehost.exe",
                "startmenuexperiencehost.exe",
                "searchhost.exe",
                "searchapp.exe",
                "textinputhost.exe",
                "applicationframehost.exe",
                "lockapp.exe",
            ];
            if let Some(name) = app.as_deref() {
                if !SHELL_PROCESSES.contains(&name) {
                    st.last_app = Some(name.to_string());
                }
            }
            st.active_app = app;
        }
    }

    pub fn last_app(&self) -> Option<String> {
        self.state.lock().ok().and_then(|s| s.last_app.clone())
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
        if let Ok(mut history) = self.history.lock() {
            history.push(entry.timestamp.clone(), entry.dst.clone());
        }
        self.last_latency_ms
            .store(entry.total_ms as u64, Ordering::SeqCst);
        if let Ok(mut logs) = self.logs.lock() {
            while logs.len() >= MAX_LOG_ENTRIES {
                logs.pop_front();
            }
            logs.push_back(entry);
        }
        // Çizim yalnızca Win32 işçisinde yapılır; ağ işçisi pencereyi beklemez.
        self.post_msg(WM_APP_HISTORY_REFRESH);
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
        if let Ok(mut history) = self.history.lock() {
            history.clear();
        }
        self.post_msg(WM_APP_HISTORY_REFRESH);
    }

    /// Arayüz ve global kısayol aynı aç/kapat yolunu kullanır.
    pub fn toggle_history(&self) {
        self.post_msg(WM_APP_HISTORY_TOGGLE);
    }

    pub fn set_history_language(&self, turkish: bool) {
        if self.history_turkish.swap(turkish, Ordering::SeqCst) != turkish {
            self.post_msg(WM_APP_HISTORY_REFRESH);
        }
    }

    /// Records a user-visible problem. Identical consecutive notices are merged into one
    /// entry with a growing `count`, so a failing API cannot flood the list.
    pub fn push_notice(&self, level: NoticeLevel, source: &str, message: impl Into<String>) {
        let mut message: String = message.into().replace(['\r', '\n'], " ");
        if message.chars().count() > MAX_NOTICE_CHARS {
            message = message.chars().take(MAX_NOTICE_CHARS).collect::<String>() + "...";
        }
        let timestamp = crate::current_hms();

        let Ok(mut notices) = self.notices.lock() else {
            return;
        };
        if let Some(last) = notices.back_mut() {
            if last.level == level && last.source == source && last.message == message {
                last.count = last.count.saturating_add(1);
                last.timestamp = timestamp;
                return;
            }
        }
        while notices.len() >= MAX_NOTICES {
            notices.pop_front();
        }
        let id = self.next_notice_id.fetch_add(1, Ordering::SeqCst) + 1;
        notices.push_back(Notice {
            id,
            timestamp,
            level,
            source: source.to_string(),
            message,
            count: 1,
        });
    }

    pub fn recent_notices(&self) -> Vec<Notice> {
        self.notices
            .lock()
            .map(|n| n.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear_notices(&self) {
        if let Ok(mut notices) = self.notices.lock() {
            notices.clear();
        }
    }

    /// Saves `cfg` and reports a failure as a notice instead of dropping it.
    pub fn save_config_or_notice(&self, cfg: &AppConfig) {
        let path = AppConfig::path();
        if let Err(e) = cfg.save_to(&path) {
            self.push_notice(
                NoticeLevel::Error,
                "config",
                format!("Could not save {}: {e}", path.display()),
            );
        } else {
            println!("[Config] Saved {}", path.display());
        }
    }

    pub fn request_open_gui(&self) {
        self.gui_open_requested.store(true, Ordering::SeqCst);
    }

    pub fn request_engine_reload(&self) {
        // Durum kilidi nesil kontrolüyle aynı sınırdadır; eski işçi hazır
        // bilgisini yeni bir yenileme isteğinin üzerine yazamaz.
        if let Ok(mut status) = self.engine_status.lock() {
            self.engine_revision.fetch_add(1, Ordering::SeqCst);
            status.loading = true;
            status.message = None;
            status.last_result = None;
            self.last_latency_ms.store(0, Ordering::SeqCst);
        }
        self.engine_reload_requested.store(true, Ordering::SeqCst);
    }

    /// İşçi yalnızca başladığı nesil hâlâ geçerliyse durumunu yayımlar.
    pub fn publish_engine_status(&self, revision: u64, status: EngineStatus) {
        if let Ok(mut current) = self.engine_status.lock() {
            if self.engine_revision.load(Ordering::SeqCst) == revision {
                *current = status;
            }
        }
    }

    pub fn snapshot_engine_status(&self) -> EngineStatus {
        self.engine_status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Nesil doğrulaması, son sonuç ve log yayını aynı kilit altında yapılır.
    /// Böylece tam sonuç gelirken yapılan yenileme eski satırı geri getiremez.
    pub fn publish_translation(
        &self,
        revision: u64,
        entry: LogEntry,
        bounds: Option<SourceBounds>,
    ) -> bool {
        let Ok(mut status) = self.engine_status.lock() else {
            return false;
        };
        if self.engine_revision.load(Ordering::SeqCst) != revision || status.loading {
            return false;
        }
        status.last_result = Some(entry.report.clone());
        // Geliştirme konsolu da arayüzle aynı gerçek sağlayıcı ve süreleri kullanır.
        println!(
            "[{} ms | OCR {} | Wait {} | Translate {} ms | {} {}{}{}] {} -> {}",
            entry.total_ms,
            entry.ocr_ms,
            entry.wait_ms,
            entry.translation_ms,
            entry.engine,
            entry.report.model,
            if entry.report.cached { " | cache" } else { "" },
            if entry.report.fallback {
                " | fallback"
            } else {
                ""
            },
            entry.src,
            entry.dst
        );
        self.set_subtitle(entry.dst.clone(), bounds);
        self.push_log(entry);
        true
    }

    pub fn trigger_reselect(&self) {
        self.post_msg(WM_APP_RESELECT);
    }

    pub fn trigger_select_output(&self, for_app: Option<String>) {
        if let Ok(mut target) = self.pending_output_target_app.lock() {
            *target = for_app;
        }
        self.post_msg(WM_APP_SELECT_OUTPUT);
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
            st.config.profile_styles.remove(app);
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
            // The outline extends past the glyphs on every side, so the box needs room for it.
            let outline = style.outline_width.clamp(0, MAX_OUTLINE_WIDTH);

            let (mut box_x, mut box_y, box_w, box_h) =
                match (style.output_mode, style.output_region) {
                    (OutputMode::Custom, Some(out)) if out.width >= 60 && out.height >= 32 => {
                        if style.fit_to_text {
                            let max_text_w = (out.width - pad_x * 2 - outline * 2).max(60);
                            let (tw, th) = measure_text_size(text.trim(), &style, max_text_w);
                            let bw = (tw + pad_x * 2 + 10 + outline * 2)
                                .min(out.width)
                                .max(60);
                            let bh = (th + pad_y * 2 + 4 + outline * 2)
                                .min(out.height)
                                .max(32);
                            let bx = out.left + (out.width - bw) / 2;
                            let by = out.top + (out.height - bh) / 2;
                            (bx, by, bw, bh)
                        } else {
                            (out.left, out.top, out.width, out.height)
                        }
                    }
                    _ => {
                        if style.fit_to_text {
                            let max_text_w = (region.width - pad_x * 2 - outline * 2).max(60);
                            let (tw, th) = measure_text_size(text.trim(), &style, max_text_w);

                            // Ensure the overlay box is at least large enough to fully cover the
                            // underlying source subtitle (e.g. 2-line EN under 1-line TR).
                            let (min_cover_w, min_cover_h, anchor_cx, anchor_cy) =
                                match source_bounds {
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

                            let bw = (tw + pad_x * 2 + 10 + outline * 2)
                                .max(min_cover_w)
                                .min(region.width)
                                .max(60);
                            let bh = (th + pad_y * 2 + 4 + outline * 2)
                                .max(min_cover_h)
                                .min(region.height)
                                .max(32);

                            let max_x = (region.left + region.width - bw).max(region.left);
                            let max_y = (region.top + region.height - bh).max(region.top);
                            let bx = (anchor_cx - bw / 2).clamp(region.left, max_x);
                            let by = (anchor_cy - bh / 2).clamp(region.top, max_y);
                            (bx, by, bw, bh)
                        } else {
                            (region.left, region.top, region.width, region.height)
                        }
                    }
                };
            box_x += style.offset_x;
            box_y += style.offset_y;

            // Per-pixel-alpha frame: background opacity applies to the box only, text stays opaque.
            let frame = render_overlay(text.trim(), &style, box_w, box_h);
            if let Err(e) = present(hwnd, box_x, box_y, box_w, box_h, &frame) {
                if let Ok(guard) = OVERLAY_HANDLE.lock() {
                    if let Some(h) = guard.as_ref() {
                        h.push_notice(
                            NoticeLevel::Error,
                            "overlay",
                            format!("Could not draw the subtitle overlay: {e}"),
                        );
                    }
                }
                return;
            }
            // Position and size were already set by UpdateLayeredWindow; this only shows the
            // window topmost without activating it.
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
            );
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
    let guide_region = OVERLAY_HANDLE
        .lock()
        .ok()
        .and_then(|g| {
            let h = g.as_ref()?;
            let st = h.state.lock().ok()?;
            (st.region.width > 0 && st.region.height > 0).then_some(st.region)
        });
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }

    match select_region_with_options(SelectorOptions {
        guide_region,
        ..SelectorOptions::default()
    }) {
        Ok(Some(new_region)) => {
            println!("[Region] New region: {:?}", new_region);
            match OVERLAY_HANDLE.lock() {
                Ok(guard) => match guard.as_ref() {
                    Some(h) => match h.state.lock() {
                        Ok(mut st) => {
                            st.region = new_region;
                            st.config.region = Some(new_region);
                            if let Some(ref app) = detected_app {
                                st.active_app = Some(app.clone());
                                st.config.profiles.insert(app.clone(), new_region);
                                println!("[Profile] Saved region for {}", app);
                            }
                            h.save_config_or_notice(&st.config);
                            h.region_changed.store(true, Ordering::SeqCst);
                        }
                        Err(e) => h.push_notice(
                            NoticeLevel::Error,
                            "region",
                            format!("Could not update region because overlay state is locked: {e}"),
                        ),
                    },
                    None => {
                        eprintln!("[Region ERROR] Overlay handle is unavailable after selection")
                    }
                },
                Err(e) => eprintln!("[Region ERROR] Could not access overlay handle: {e}"),
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
        Ok(None) => println!("[Region] Selection cancelled or too small; config unchanged"),
        Err(e) => {
            eprintln!("[Region ERROR] Selector failed: {e}");
            if let Ok(guard) = OVERLAY_HANDLE.lock() {
                if let Some(h) = guard.as_ref() {
                    h.push_notice(
                        NoticeLevel::Error,
                        "region",
                        format!("Region selector failed: {e}"),
                    );
                }
            }
        }
    }
    apply_visibility_and_redraw(hwnd);
}

fn handle_select_output(hwnd: HWND) {
    let (target_app, guide_region) = OVERLAY_HANDLE
        .lock()
        .ok()
        .and_then(|g| {
            let h = g.as_ref()?;
            let target = h
                .pending_output_target_app
                .lock()
                .ok()
                .and_then(|mut t| t.take());
            let guide = h.state.lock().ok().and_then(|st| {
                let target_output = match target.as_deref() {
                    Some(app) => st
                        .config
                        .profile_styles
                        .get(app)
                        .and_then(|s| s.output_region),
                    None => st.config.output_region,
                };
                target_output
                    .filter(|r| r.width > 0 && r.height > 0)
                    .or_else(|| (st.region.width > 0 && st.region.height > 0).then_some(st.region))
            });
            Some((target, guide))
        })
        .unwrap_or((None, None));

    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }

    match select_region_with_options(SelectorOptions {
        title: "Drag to select output subtitle area   (ESC or Right-Click to cancel)",
        guide_region,
        border_colorref: OUTPUT_BORDER_COLORREF,
    }) {
        Ok(Some(new_region)) => {
            println!(
                "[OutputRegion] New output region ({}): {:?}",
                target_app.as_deref().unwrap_or("global"),
                new_region
            );
            match OVERLAY_HANDLE.lock() {
                Ok(guard) => match guard.as_ref() {
                    Some(h) => match h.state.lock() {
                        Ok(mut st) => {
                            if let Some(ref app) = target_app {
                                let base = OverlayStyle::from_config(&st.config);
                                let entry = st
                                    .config
                                    .profile_styles
                                    .entry(app.clone())
                                    .or_insert(base);
                                entry.output_mode = OutputMode::Custom;
                                entry.output_region = Some(new_region);
                                st.style = entry.clone();
                            } else {
                                st.config.output_mode = OutputMode::Custom;
                                st.config.output_region = Some(new_region);
                                st.style.output_mode = OutputMode::Custom;
                                st.style.output_region = Some(new_region);
                            }
                            h.save_config_or_notice(&st.config);
                        }
                        Err(e) => h.push_notice(
                            NoticeLevel::Error,
                            "region",
                            format!(
                                "Could not update output region because overlay state is locked: {e}"
                            ),
                        ),
                    },
                    None => eprintln!(
                        "[OutputRegion ERROR] Overlay handle is unavailable after selection"
                    ),
                },
                Err(e) => eprintln!("[OutputRegion ERROR] Could not access overlay handle: {e}"),
            }
        }
        Ok(None) => println!("[OutputRegion] Selection cancelled or too small; config unchanged"),
        Err(e) => {
            eprintln!("[OutputRegion ERROR] Selector failed: {e}");
            if let Ok(guard) = OVERLAY_HANDLE.lock() {
                if let Some(h) = guard.as_ref() {
                    h.push_notice(
                        NoticeLevel::Error,
                        "region",
                        format!("Output region selector failed: {e}"),
                    );
                }
            }
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
        WM_APP_SELECT_OUTPUT => {
            handle_select_output(hwnd);
            LRESULT(0)
        }
        WM_APP_TOGGLE_PAUSE => {
            let _ = handle_toggle_pause(hwnd);
            LRESULT(0)
        }
        WM_APP_HISTORY_TOGGLE => {
            crate::history::toggle();
            LRESULT(0)
        }
        WM_APP_HISTORY_REFRESH => {
            crate::history::refresh();
            LRESULT(0)
        }
        WM_APP_STREAM_MODE => {
            let stream_mode = OVERLAY_HANDLE
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|h| h.is_stream_mode()))
                .unwrap_or(false);
            let affinity = if stream_mode {
                WDA_NONE
            } else {
                WDA_EXCLUDEFROMCAPTURE
            };
            let _ = SetWindowDisplayAffinity(hwnd, affinity);
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
                            if let Some(size) = h.adjust_font_size(delta) {
                                println!("[Hotkey] Font size: {size}");
                            }
                        }
                    }
                }
                HK_HISTORY => crate::history::toggle(),
                HK_SETTINGS => {
                    println!("[Hotkey] Ctrl+Shift+A -> Opening Overcap window...");
                    handle_open_gui();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_PAINT => {
            // Content is pushed with UpdateLayeredWindow (see `render::present`); this only
            // validates the update region.
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
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
            let _ = UnregisterHotKey(hwnd, HK_HISTORY);
            crate::history::shutdown();
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
            last_app: None,
            config,
        })),
        region_changed: Arc::new(AtomicBool::new(false)),
        stop_flag: Arc::new(AtomicBool::new(false)),
        gui_open_requested: Arc::new(AtomicBool::new(false)),
        engine_reload_requested: Arc::new(AtomicBool::new(false)),
        engine_revision: Arc::new(AtomicU64::new(0)),
        engine_status: Arc::new(Mutex::new(EngineStatus {
            loading: true,
            ..Default::default()
        })),
        ocr_diagnostics: Arc::new(AtomicBool::new(false)),
        last_latency_ms: Arc::new(AtomicU64::new(0)),
        logs: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES))),
        history: Arc::new(Mutex::new(crate::history::HistoryBuffer::default())),
        history_visible: Arc::new(AtomicBool::new(false)),
        history_turkish: Arc::new(AtomicBool::new(false)),
        pending_output_target_app: Arc::new(Mutex::new(None)),
        notices: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_NOTICES))),
        next_notice_id: Arc::new(AtomicU64::new(0)),
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

        let ex_style =
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;

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

        // Exclude this overlay window from screen capture in normal mode so OCR always sees
        // the underlying original subtitle. In Stream Mode (WGC window capture), WDA_NONE allows
        // Discord/OBS screen share to see the translated overlay.
        let initial_affinity = if handle.is_stream_mode() {
            WDA_NONE
        } else {
            WDA_EXCLUDEFROMCAPTURE
        };
        SetWindowDisplayAffinity(hwnd, initial_affinity)?;
        // Do NOT call SetLayeredWindowAttributes on this window: once it has been used,
        // UpdateLayeredWindow (per-pixel alpha, see render.rs) fails for the window's lifetime.

        let mods = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0);
        let _ = RegisterHotKey(hwnd, HK_QUIT, mods, VK_Q.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_TOGGLE, mods, VK_H.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_RESELECT, mods, VK_S.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_FONT_UP, mods, VK_UP.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_FONT_DOWN, mods, VK_DOWN.0 as u32);
        let _ = RegisterHotKey(hwnd, HK_SETTINGS, mods, VK_A.0 as u32);
        if let Err(error) = RegisterHotKey(hwnd, HK_HISTORY, mods, VK_L.0 as u32) {
            handle.push_notice(
                NoticeLevel::Warning,
                "history",
                format!("Ctrl+Shift+L unavailable: {error}. Open history from Live Log."),
            );
        }
        if let Err(error) = crate::history::initialize(handle.clone()) {
            handle.push_notice(
                NoticeLevel::Error,
                "history",
                format!("History could not start: {error}"),
            );
        }

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

#[cfg(test)]
mod engine_status_tests {
    use super::*;

    /// Gerçek pencere oluşturmadan işçi/arayüz arasındaki yenileme yarışını doğrular.
    #[test]
    fn reload_rejects_old_status_and_old_translation() {
        let handle = create_handle(
            AppConfig::default(),
            Region {
                left: 0,
                top: 0,
                width: 100,
                height: 50,
            },
        );
        handle.publish_engine_status(
            0,
            EngineStatus {
                ready: "groq".into(),
                loading: false,
                ..Default::default()
            },
        );
        handle.request_engine_reload();
        handle.publish_engine_status(
            0,
            EngineStatus {
                ready: "google".into(),
                ..Default::default()
            },
        );
        assert!(handle.snapshot_engine_status().loading);
        assert_eq!(handle.snapshot_engine_status().ready, "groq");
        let entry = LogEntry {
            timestamp: String::new(),
            total_ms: 20,
            ocr_ms: 5,
            wait_ms: 5,
            translation_ms: 10,
            report: Default::default(),
            engine: "groq".into(),
            src: "old".into(),
            dst: "eski".into(),
        };
        assert!(!handle.publish_translation(0, entry.clone(), None));
        assert!(handle.recent_logs().is_empty());
        assert!(handle.snapshot_text().is_empty());
        handle.publish_engine_status(
            1,
            EngineStatus {
                ready: "google".into(),
                ..Default::default()
            },
        );
        assert!(handle.publish_translation(1, entry, None));
        assert_eq!(handle.snapshot_text(), "eski");
        handle.request_engine_reload();
        assert!(handle.snapshot_engine_status().last_result.is_none());
        assert_eq!(handle.last_latency_ms.load(Ordering::SeqCst), 0);
    }
}
