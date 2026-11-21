use anyhow::{anyhow, Result};
use eframe::egui::{
    self, Align, Color32, ComboBox, FontFamily, FontId, Frame, Layout, Margin, RichText, Rounding,
    ScrollArea, Sense, Stroke, TextEdit, Vec2, ViewportCommand,
};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::config::{AppConfig, OutputMode, OverlayStyle, MAX_OUTLINE_WIDTH};
use crate::ocr::{
    available_ocr_languages, is_language_installed, launch_ocr_pack_installer, ocr_capability_name,
};
use crate::overlay::{NoticeLevel, OverlayHandle};
use crate::tray;

const FONT_FAMILIES: &[&str] = &[
    "Segoe UI",
    "Bahnschrift",
    "Trebuchet MS",
    "Calibri",
    "Arial",
    "Verdana",
    "Georgia",
    "Consolas",
];

const ENGINES: &[&str] = &["groq", "gemini", "openai-compat", "google"];

const TEXT_SWATCHES: &[&str] = &["#FFFFFF", "#FFE55C", "#E2E8F0", "#D4D4D4"];
const BG_SWATCHES: &[&str] = &["#0A0A0C", "#000000", "#121212", "#242424"];
const OUTLINE_SWATCHES: &[&str] = &["#000000", "#141414", "#FFFFFF"];

const SAMPLES_EN: &[&str] = &[
    "We need to get out of here before they find us. Stay low and follow me!",
    "Seriously, you can't recommend the same movie 24 times!",
    "Nobody notices, because it doesn't happen all at once.",
    "If this goes sideways, run and don't look back.",
];

const SAMPLES_TR: &[&str] = &[
    "Bizi bulmadan buradan cikmamiz lazim, alcaktan git ve beni takip et!",
    "Ama yuh artik, ayni filmi de 24 kez onermezsin!",
    "Kimse fark etmiyor, cunku hepsi birden olmuyor.",
    "Eger bu is ters giderse, arkana bile bakmadan kac.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiLang {
    En,
    Tr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageBg {
    Dark,
    Gray,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Appearance,
    Engine,
    ProfilesOcr,
    LiveLog,
    Setup,
}

#[derive(Clone, Copy)]
struct Palette {
    window_bg: Color32,
    surface_bg: Color32,
    input_bg: Color32,
    hover_bg: Color32,
    active_bg: Color32,
    border: Color32,
    text_primary: Color32,
    text_muted: Color32,
    accent: Color32,
    accent_text: Color32,
    status_green: Color32,
    status_amber: Color32,
    status_red: Color32,
}

impl Palette {
    fn dark() -> Self {
        Self {
            window_bg: Color32::from_rgb(20, 20, 20),       // #141414
            surface_bg: Color32::from_rgb(28, 28, 28),      // #1C1C1C
            input_bg: Color32::from_rgb(17, 17, 17),        // #111111
            hover_bg: Color32::from_rgb(36, 36, 36),        // #242424
            active_bg: Color32::from_rgb(46, 46, 46),       // #2E2E2E
            border: Color32::from_rgb(43, 43, 43),          // #2B2B2B
            text_primary: Color32::from_rgb(235, 235, 235), // #EBEBEB
            text_muted: Color32::from_rgb(142, 142, 142),   // #8E8E8E
            accent: Color32::from_rgb(224, 224, 224),       // #E0E0E0
            accent_text: Color32::from_rgb(20, 20, 20),
            status_green: Color32::from_rgb(63, 185, 80), // #3FB950
            status_amber: Color32::from_rgb(210, 153, 34), // #D29922
            status_red: Color32::from_rgb(248, 81, 73),   // #F85149
        }
    }

    fn light() -> Self {
        Self {
            window_bg: Color32::from_rgb(236, 236, 236),  // #ECECEC
            surface_bg: Color32::from_rgb(247, 247, 247), // #F7F7F7
            input_bg: Color32::from_rgb(255, 255, 255),   // #FFFFFF
            hover_bg: Color32::from_rgb(226, 226, 226),   // #E2E2E2
            active_bg: Color32::from_rgb(212, 212, 212),  // #D4D4D4
            border: Color32::from_rgb(208, 208, 208),     // #D0D0D0
            text_primary: Color32::from_rgb(26, 26, 26),  // #1A1A1A
            text_muted: Color32::from_rgb(98, 98, 98),    // #626262
            accent: Color32::from_rgb(46, 46, 46),        // #2E2E2E
            accent_text: Color32::from_rgb(247, 247, 247),
            status_green: Color32::from_rgb(45, 148, 62),
            status_amber: Color32::from_rgb(176, 124, 20),
            status_red: Color32::from_rgb(208, 52, 44),
        }
    }
}

const LABEL_COL_W: f32 = 120.0;

fn section_header(ui: &mut egui::Ui, pal: Palette, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(FontId::new(11.0, FontFamily::Monospace))
            .color(pal.text_muted),
    );
    ui.separator();
}

/// Fixed-width label so every control in a column lines up.
fn row_label(ui: &mut egui::Ui, text: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(LABEL_COL_W, 20.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(text);
        },
    );
}

/// `label | slider | value`. Returns true when the value changed.
fn slider_row(
    ui: &mut egui::Ui,
    pal: Palette,
    label: &str,
    value: &mut i32,
    range: std::ops::RangeInclusive<i32>,
    format: impl Fn(i32) -> String,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        row_label(ui, label);
        ui.spacing_mut().slider_width = 150.0;
        changed = ui
            .add(egui::Slider::new(value, range).show_value(false))
            .changed();
        ui.add_sized(
            [56.0, 20.0],
            egui::Label::new(
                RichText::new(format(*value))
                    .font(FontId::new(12.0, FontFamily::Monospace))
                    .color(pal.text_muted),
            ),
        );
    });
    changed
}

/// `label | color picker | #hex | swatches`. Returns true when the color changed.
fn color_row(
    ui: &mut egui::Ui,
    pal: Palette,
    label: &str,
    hex: &mut String,
    swatches: &[&str],
    fallback: [u8; 3],
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        row_label(ui, label);
        let mut rgb = parse_hex_rgb(hex, fallback);
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *hex = rgb_to_hex(rgb);
            changed = true;
        }
        ui.label(
            RichText::new(hex.as_str())
                .font(FontId::new(11.5, FontFamily::Monospace))
                .color(pal.text_muted),
        );
        for sw in swatches {
            let c = parse_hex_rgb(sw, fallback);
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
            ui.painter().rect_filled(
                rect,
                Rounding::same(2.0),
                Color32::from_rgb(c[0], c[1], c[2]),
            );
            ui.painter()
                .rect_stroke(rect, Rounding::same(2.0), Stroke::new(1.0_f32, pal.border));
            if resp.clicked() {
                *hex = (*sw).to_string();
                changed = true;
            }
        }
    });
    changed
}

fn notice_color(pal: Palette, level: NoticeLevel) -> Color32 {
    match level {
        NoticeLevel::Warning => pal.status_amber,
        NoticeLevel::Error => pal.status_red,
    }
}

fn tr(lang: UiLang, en: &'static str, tr_text: &'static str) -> &'static str {
    match lang {
        UiLang::En => en,
        UiLang::Tr => tr_text,
    }
}

fn apply_egui_visuals(ctx: &egui::Context, mode: ThemeMode) {
    let pal = match mode {
        ThemeMode::Dark => Palette::dark(),
        ThemeMode::Light => Palette::light(),
    };

    let mut visuals = match mode {
        ThemeMode::Dark => egui::Visuals::dark(),
        ThemeMode::Light => egui::Visuals::light(),
    };

    visuals.panel_fill = pal.window_bg;
    visuals.window_fill = pal.surface_bg;
    visuals.extreme_bg_color = pal.input_bg;
    visuals.faint_bg_color = pal.surface_bg;
    visuals.code_bg_color = pal.input_bg;

    visuals.window_shadow = egui::Shadow::NONE;
    visuals.popup_shadow = egui::Shadow::NONE;
    visuals.window_rounding = Rounding::same(4.0);
    visuals.window_stroke = Stroke::new(1.0_f32, pal.border);

    // Eliminate blue/purple egui defaults completely.
    visuals.selection.bg_fill = pal.active_bg;
    visuals.selection.stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.hyperlink_color = pal.text_primary;

    let r = Rounding::same(3.0);
    visuals.widgets.noninteractive.bg_fill = pal.surface_bg;
    visuals.widgets.noninteractive.weak_bg_fill = pal.surface_bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    visuals.widgets.noninteractive.rounding = r;

    visuals.widgets.inactive.bg_fill = pal.input_bg;
    visuals.widgets.inactive.weak_bg_fill = pal.surface_bg;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    visuals.widgets.inactive.rounding = r;

    visuals.widgets.hovered.bg_fill = pal.hover_bg;
    visuals.widgets.hovered.weak_bg_fill = pal.hover_bg;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.text_muted);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    visuals.widgets.hovered.rounding = r;

    visuals.widgets.active.bg_fill = pal.active_bg;
    visuals.widgets.active.weak_bg_fill = pal.active_bg;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    visuals.widgets.active.rounding = r;

    visuals.widgets.open.bg_fill = pal.hover_bg;
    visuals.widgets.open.weak_bg_fill = pal.hover_bg;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, pal.text_muted);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    visuals.widgets.open.rounding = r;

    ctx.set_visuals(visuals);
}

fn parse_hex_rgb(hex: &str, fallback: [u8; 3]) -> [u8; 3] {
    let s = hex.trim().trim_start_matches('#');
    if s.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        ) {
            return [r, g, b];
        }
    }
    fallback
}

fn rgb_to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

struct OvercapApp {
    handle: OverlayHandle,
    theme: ThemeMode,
    ui_lang: UiLang,
    active_tab: Tab,
    stage_bg: StageBg,
    sample_idx: usize,
    overlay_preview_on: bool,

    // Live style state, mirrored from config.json (global style or the `editing_app` style)
    style: OverlayStyle,
    // `Some(exe)` while the Appearance tab edits that app's own style instead of the global one.
    editing_app: Option<String>,

    // Editable Engine tab state
    engine: String,
    source_lang: String,
    target_lang: String,
    groq_model: String,
    gemini_model: String,
    custom_base_url: String,
    custom_model: String,
    groq_key_input: String,
    gemini_key_input: String,
    custom_key_input: String,
    has_groq_key: bool,
    has_gemini_key: bool,
    has_custom_key: bool,
    engine_status_msg: Option<String>,

    // Profiles & OCR cached state
    auto_profiles: bool,
    installed_ocr_langs: Vec<String>,
    source_ocr_installed: bool,
    last_checked_ocr_lang: String,

    // Notices with an id <= this value were dismissed from the banner.
    dismissed_notice_id: u64,
}

fn load_windows_preview_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mappings: &[(&str, &str)] = &[
        ("Segoe UI", "C:\\Windows\\Fonts\\segoeui.ttf"),
        ("Segoe UI:bold", "C:\\Windows\\Fonts\\segoeuib.ttf"),
        ("Bahnschrift", "C:\\Windows\\Fonts\\bahnschrift.ttf"),
        ("Bahnschrift:bold", "C:\\Windows\\Fonts\\bahnschrift.ttf"),
        ("Trebuchet MS", "C:\\Windows\\Fonts\\trebuc.ttf"),
        ("Trebuchet MS:bold", "C:\\Windows\\Fonts\\trebucbd.ttf"),
        ("Calibri", "C:\\Windows\\Fonts\\calibri.ttf"),
        ("Calibri:bold", "C:\\Windows\\Fonts\\calibrib.ttf"),
        ("Arial", "C:\\Windows\\Fonts\\arial.ttf"),
        ("Arial:bold", "C:\\Windows\\Fonts\\arialbd.ttf"),
        ("Verdana", "C:\\Windows\\Fonts\\verdana.ttf"),
        ("Verdana:bold", "C:\\Windows\\Fonts\\verdanab.ttf"),
        ("Georgia", "C:\\Windows\\Fonts\\georgia.ttf"),
        ("Georgia:bold", "C:\\Windows\\Fonts\\georgiab.ttf"),
        ("Consolas", "C:\\Windows\\Fonts\\consola.ttf"),
        ("Consolas:bold", "C:\\Windows\\Fonts\\consolab.ttf"),
    ];

    for (family_key, path) in mappings {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert((*family_key).to_string(), egui::FontData::from_owned(bytes));
            fonts.families.insert(
                FontFamily::Name(Arc::from(*family_key)),
                vec![(*family_key).to_string()],
            );
        }
    }

    ctx.set_fonts(fonts);
}

fn preview_font_family(ctx: &egui::Context, family: &str, bold: bool) -> FontFamily {
    let key = if bold {
        format!("{}:bold", family)
    } else {
        family.to_string()
    };
    let named = FontFamily::Name(Arc::from(key));
    let exists = ctx.fonts(|f| f.families().into_iter().any(|fam| fam == named));
    if exists {
        named
    } else {
        let regular = FontFamily::Name(Arc::from(family));
        if ctx.fonts(|f| f.families().into_iter().any(|fam| fam == regular)) {
            regular
        } else {
            FontFamily::Proportional
        }
    }
}

impl OvercapApp {
    fn new(cc: &eframe::CreationContext<'_>, handle: OverlayHandle) -> Self {
        load_windows_preview_fonts(&cc.egui_ctx);
        apply_egui_visuals(&cc.egui_ctx, ThemeMode::Dark);
        let cfg = handle.snapshot_config();
        let style = OverlayStyle::from_config(&cfg);
        let installed_ocr_langs = available_ocr_languages();
        let source_ocr_installed = is_language_installed(&cfg.source_lang);

        let initial_tab = if cfg.region.is_none() {
            Tab::Setup
        } else {
            Tab::Appearance
        };

        Self {
            handle,
            theme: ThemeMode::Dark,
            ui_lang: UiLang::En,
            active_tab: initial_tab,
            stage_bg: StageBg::Dark,
            sample_idx: 0,
            overlay_preview_on: false,
            style,
            editing_app: None,
            engine: cfg.engine,
            source_lang: cfg.source_lang.clone(),
            target_lang: cfg.target_lang,
            groq_model: cfg.groq_model,
            gemini_model: cfg.gemini_model,
            custom_base_url: cfg.custom_base_url,
            custom_model: cfg.custom_model,
            groq_key_input: String::new(),
            gemini_key_input: String::new(),
            custom_key_input: String::new(),
            has_groq_key: key_available(cfg.groq_api_key.as_deref(), "GROQ_API_KEY"),
            has_gemini_key: key_available(cfg.gemini_api_key.as_deref(), "GEMINI_API_KEY"),
            has_custom_key: key_available(cfg.custom_api_key.as_deref(), "OPENAI_API_KEY"),
            engine_status_msg: None,
            auto_profiles: cfg.auto_profiles,
            installed_ocr_langs,
            source_ocr_installed,
            last_checked_ocr_lang: cfg.source_lang,
            dismissed_notice_id: 0,
        }
    }

    fn palette(&self) -> Palette {
        match self.theme {
            ThemeMode::Dark => Palette::dark(),
            ThemeMode::Light => Palette::light(),
        }
    }

    fn current_sample(&self) -> &'static str {
        let list = match self.ui_lang {
            UiLang::En => SAMPLES_EN,
            UiLang::Tr => SAMPLES_TR,
        };
        list[self.sample_idx % list.len()]
    }

    fn t(&self, en: &'static str, tr: &'static str) -> &'static str {
        match self.ui_lang {
            UiLang::En => en,
            UiLang::Tr => tr,
        }
    }

    /// config.json (via the handle) is the source of truth for what the Appearance tab shows,
    /// so changes made elsewhere (font-size hotkeys, removing an app style) show up here too.
    fn sync_from_handle_if_needed(&mut self) {
        let cfg = self.handle.snapshot_config();
        self.auto_profiles = cfg.auto_profiles;

        if let Some(app) = &self.editing_app {
            if !cfg.profile_styles.contains_key(app) {
                self.editing_app = None;
            }
        }
        self.style = match self
            .editing_app
            .as_ref()
            .and_then(|app| cfg.profile_styles.get(app))
        {
            Some(style) => style.clone(),
            None => OverlayStyle::from_config(&cfg),
        };
    }

    /// Applies `self.style` live and stores it in the scope being edited.
    fn commit_style(&mut self) {
        match self.editing_app.clone() {
            Some(app) => self.handle.apply_app_style(&app, self.style.clone()),
            None => self.handle.apply_style(self.style.clone(), true),
        }
    }

    /// Switches what the Appearance tab edits. Selecting an app that has no style yet creates
    /// one from the current global style.
    fn set_style_scope(&mut self, app: Option<String>) {
        let cfg = self.handle.snapshot_config();
        match &app {
            Some(name) => match cfg.profile_styles.get(name) {
                Some(existing) => self.handle.apply_style(existing.clone(), false),
                None => self
                    .handle
                    .apply_app_style(name, OverlayStyle::from_config(&cfg)),
            },
            None => self
                .handle
                .apply_style(OverlayStyle::from_config(&cfg), false),
        }
        self.editing_app = app;
    }

    fn refresh_ocr_diagnostics(&mut self) {
        self.installed_ocr_langs = available_ocr_languages();
        self.source_ocr_installed = is_language_installed(&self.source_lang);
        self.last_checked_ocr_lang = self.source_lang.clone();
    }

    fn apply_preset(&mut self, name: &str) {
        let (family, size, bold, text, bg, opacity, radius, shadow, fit, px, py, outline) =
            match name {
                "cinema" => (
                    "Bahnschrift",
                    30,
                    true,
                    "#FFFFFF",
                    "#0A0A0C",
                    225,
                    10,
                    true,
                    true,
                    18,
                    8,
                    0,
                ),
                "yellow" => (
                    "Trebuchet MS",
                    30,
                    true,
                    "#FFE55C",
                    "#08080A",
                    220,
                    8,
                    true,
                    true,
                    16,
                    8,
                    1,
                ),
                "minimal" => (
                    "Segoe UI", 28, false, "#F5F5F5", "#121212", 185, 12, true, true, 14, 6, 0,
                ),
                _ => (
                    "Segoe UI", 32, true, "#FFFFFF", "#000000", 250, 4, false, false, 20, 10, 2,
                ),
            };

        self.style = OverlayStyle {
            font_family: family.to_string(),
            font_size: size,
            font_bold: bold,
            text_color: text.to_string(),
            bg_color: bg.to_string(),
            opacity,
            corner_radius: radius,
            text_shadow: shadow,
            fit_to_text: fit,
            padding_x: px,
            padding_y: py,
            outline_width: outline,
            outline_color: "#000000".to_string(),
            output_mode: self.style.output_mode,
            output_region: self.style.output_region,
            offset_x: self.style.offset_x,
            offset_y: self.style.offset_y,
        };
        self.commit_style();
    }

    fn save_and_reload_engine(&mut self) {
        let mut cfg: AppConfig = self.handle.snapshot_config();
        cfg.engine = self.engine.clone();
        cfg.source_lang = self.source_lang.trim().to_string();
        cfg.target_lang = self.target_lang.trim().to_string();
        cfg.groq_model = self.groq_model.trim().to_string();
        cfg.gemini_model = self.gemini_model.trim().to_string();
        cfg.custom_base_url = self.custom_base_url.trim().to_string();
        cfg.custom_model = self.custom_model.trim().to_string();

        if !self.groq_key_input.trim().is_empty() {
            cfg.groq_api_key = Some(self.groq_key_input.trim().to_string());
        }
        if !self.gemini_key_input.trim().is_empty() {
            cfg.gemini_api_key = Some(self.gemini_key_input.trim().to_string());
        }
        if !self.custom_key_input.trim().is_empty() {
            cfg.custom_api_key = Some(self.custom_key_input.trim().to_string());
        }

        match self.handle.update_config_and_save(cfg) {
            Ok(()) => {
                // Kaydetme başarısızsa girilen anahtar korunur ve SET rozeti verilmez.
                self.groq_key_input.clear();
                self.gemini_key_input.clear();
                self.custom_key_input.clear();
                let saved = self.handle.snapshot_config();
                self.has_groq_key = key_available(saved.groq_api_key.as_deref(), "GROQ_API_KEY");
                self.has_gemini_key =
                    key_available(saved.gemini_api_key.as_deref(), "GEMINI_API_KEY");
                self.has_custom_key =
                    key_available(saved.custom_api_key.as_deref(), "OPENAI_API_KEY");
                self.handle.request_engine_reload();
                if self.source_lang != self.last_checked_ocr_lang {
                    self.refresh_ocr_diagnostics();
                }
                self.engine_status_msg = Some(
                    self.t(
                        "Settings saved; engine reload requested.",
                        "Ayarlar kaydedildi; motor yenileme istendi.",
                    )
                    .to_string(),
                );
            }
            Err(e) => {
                self.engine_status_msg = Some(format!("Save failed: {e}"));
            }
        }
    }

    fn render_top_bar(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let is_paused = self.handle.is_paused();
        let latency = self.handle.last_latency_ms.load(Ordering::SeqCst);
        // Kullanıcı tercihi yerine gerçekten hazırlanmış işçi gösterilir.
        let runtime = self.handle.snapshot_engine_status();
        let fallback = runtime.last_result.as_ref().is_some_and(|r| r.fallback);
        let active_engine = if fallback {
            format!(
                "{} (fallback)",
                runtime.last_result.as_ref().unwrap().engine
            )
        } else if runtime.ready.is_empty() {
            "—".into()
        } else {
            runtime.ready.clone()
        };
        let running_color = if fallback || runtime.message.is_some() {
            pal.status_amber
        } else {
            pal.status_green
        };

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            ui.label(
                RichText::new("OVERCAP")
                    .strong()
                    .size(15.0)
                    .color(pal.text_primary),
            );
            ui.label(
                RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                    .font(FontId::new(11.5, FontFamily::Monospace))
                    .color(pal.text_muted),
            );

            ui.add_space(6.0);

            let (dot_color, status_text) = if !self.source_ocr_installed {
                (
                    pal.status_red,
                    self.t("Missing OCR", "OCR Eksik").to_string(),
                )
            } else if runtime.loading {
                (
                    pal.status_amber,
                    self.t("Preparing engine", "Motor hazirlaniyor").to_string(),
                )
            } else if is_paused {
                (
                    pal.status_amber,
                    format!("{} · {}", self.t("Paused", "Duraklatildi"), active_engine),
                )
            } else if latency > 0 {
                (
                    running_color,
                    format!(
                        "{} · {} · {} ms",
                        self.t("Running", "Calisiyor"),
                        active_engine,
                        latency
                    ),
                )
            } else {
                (
                    running_color,
                    format!("{} · {}", self.t("Running", "Calisiyor"), active_engine),
                )
            };

            let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
            ui.painter()
                .circle_filled(dot_rect.center(), 4.0, dot_color);

            ui.label(
                RichText::new(status_text)
                    .font(FontId::new(12.0, FontFamily::Monospace))
                    .color(pal.text_muted),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                let theme_lbl = match self.theme {
                    ThemeMode::Dark => "Dark | Light",
                    ThemeMode::Light => "Light | Dark",
                };
                if ui.button(theme_lbl).clicked() {
                    self.theme = match self.theme {
                        ThemeMode::Dark => ThemeMode::Light,
                        ThemeMode::Light => ThemeMode::Dark,
                    };
                    apply_egui_visuals(ui.ctx(), self.theme);
                }

                let lang_lbl = match self.ui_lang {
                    UiLang::En => "EN | TR",
                    UiLang::Tr => "TR | EN",
                };
                if ui.button(lang_lbl).clicked() {
                    self.ui_lang = match self.ui_lang {
                        UiLang::En => UiLang::Tr,
                        UiLang::Tr => UiLang::En,
                    };
                }

                let pause_lbl = if is_paused {
                    self.t("Resume", "Devam Et")
                } else {
                    self.t("Pause", "Duraklat")
                };
                if ui.button(pause_lbl).clicked() {
                    self.handle.toggle_pause();
                }

                if ui.button(self.t("Select Region", "Bolge Sec")).clicked() {
                    self.handle.trigger_reselect();
                }
            });
        });
    }

    fn render_preview_strip(&mut self, ui: &mut egui::Ui, pal: Palette) {
        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        RichText::new(self.t("Preview", "Onizleme"))
                            .size(12.0)
                            .color(pal.text_muted),
                    );

                    for (bg, label) in [
                        (StageBg::Dark, self.t("Dark", "Koyu")),
                        (StageBg::Gray, self.t("Gray", "Gri")),
                        (StageBg::Light, self.t("Light", "Acik")),
                    ] {
                        let selected = self.stage_bg == bg;
                        let btn =
                            egui::Button::new(RichText::new(label).size(11.5).color(if selected {
                                pal.accent_text
                            } else {
                                pal.text_primary
                            }))
                            .fill(if selected {
                                pal.accent
                            } else {
                                pal.input_bg
                            });
                        if ui.add(btn).clicked() {
                            self.stage_bg = bg;
                        }
                    }

                    if ui.button(self.t("Next Sample", "Sonraki Ornek")).clicked() {
                        self.sample_idx = (self.sample_idx + 1) % SAMPLES_EN.len();
                        if self.overlay_preview_on {
                            self.handle
                                .set_test_preview(true, Some(self.current_sample().to_string()));
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let preview_lbl = if self.overlay_preview_on {
                            self.t("Overlay Preview: ON", "Ekran Onizleme: ACIK")
                        } else {
                            self.t("Overlay Preview: OFF", "Ekran Onizleme: KAPALI")
                        };
                        let btn = egui::Button::new(RichText::new(preview_lbl).size(11.5).color(
                            if self.overlay_preview_on {
                                pal.accent_text
                            } else {
                                pal.text_primary
                            },
                        ))
                        .fill(if self.overlay_preview_on {
                            pal.accent
                        } else {
                            pal.input_bg
                        });
                        if ui.add(btn).clicked() {
                            self.overlay_preview_on = !self.overlay_preview_on;
                            if self.overlay_preview_on {
                                self.handle.set_test_preview(
                                    true,
                                    Some(self.current_sample().to_string()),
                                );
                            } else {
                                self.handle.set_test_preview(false, None);
                            }
                        }
                    });
                });

                ui.add_space(6.0);

                let stage_fill = match self.stage_bg {
                    StageBg::Dark => Color32::from_rgb(15, 15, 16),
                    StageBg::Gray => Color32::from_rgb(74, 74, 77),
                    StageBg::Light => Color32::from_rgb(216, 216, 220),
                };

                let text_rgb = parse_hex_rgb(&self.style.text_color, [255, 255, 255]);
                let bg_rgb = parse_hex_rgb(&self.style.bg_color, [10, 10, 12]);
                let sub_bg = Color32::from_rgba_unmultiplied(
                    bg_rgb[0],
                    bg_rgb[1],
                    bg_rgb[2],
                    self.style.opacity,
                );
                let sub_fg = Color32::from_rgb(text_rgb[0], text_rgb[1], text_rgb[2]);
                let preview_font_px = (self.style.font_size as f32 * 0.82).clamp(13.0, 44.0);
                let pad_x = (self.style.padding_x as f32).clamp(4.0, 60.0);
                let pad_y = (self.style.padding_y as f32).clamp(2.0, 40.0);
                let rounding = Rounding::same((self.style.corner_radius as f32).min(32.0));

                let stage_h = 136.0;
                let (stage_rect, _) = ui
                    .allocate_exact_size(egui::vec2(ui.available_width(), stage_h), Sense::hover());
                let painter = ui.painter_at(stage_rect);
                painter.rect(
                    stage_rect,
                    Rounding::same(3.0),
                    stage_fill,
                    Stroke::new(1.0_f32, pal.border),
                );

                let font_fam =
                    preview_font_family(ui.ctx(), &self.style.font_family, self.style.font_bold);
                let font_id = FontId::new(preview_font_px, font_fam);
                let sample = self.current_sample();
                let max_wrap_w = (stage_rect.width() - 36.0 - pad_x * 2.0).max(120.0);

                let text_galley =
                    painter.layout(sample.to_string(), font_id.clone(), sub_fg, max_wrap_w);
                let text_size = text_galley.size();

                let box_w = if self.style.fit_to_text {
                    (text_size.x + pad_x * 2.0).min(stage_rect.width() - 24.0)
                } else {
                    (stage_rect.width() - 24.0).max(120.0)
                };
                let box_h = (text_size.y + pad_y * 2.0)
                    .min(stage_rect.height() - 14.0)
                    .max(28.0);

                let box_rect =
                    egui::Rect::from_center_size(stage_rect.center(), egui::vec2(box_w, box_h));
                painter.rect_filled(box_rect, rounding, sub_bg);

                let text_pos = egui::pos2(
                    box_rect.center().x - text_size.x * 0.5,
                    box_rect.center().y - text_size.y * 0.5,
                );
                // Same layer order as the real overlay: shadow, outline, text.
                let outline_rgb = parse_hex_rgb(&self.style.outline_color, [0, 0, 0]);
                let outline_col = Color32::from_rgb(outline_rgb[0], outline_rgb[1], outline_rgb[2]);
                let outline_px = if self.style.outline_width > 0 {
                    (self.style.outline_width as f32 * 0.82).max(1.0)
                } else {
                    0.0
                };
                const RING: [(f32, f32); 8] = [
                    (1.0, 0.0),
                    (0.707, 0.707),
                    (0.0, 1.0),
                    (-0.707, 0.707),
                    (-1.0, 0.0),
                    (-0.707, -0.707),
                    (0.0, -1.0),
                    (0.707, -0.707),
                ];

                if self.style.text_shadow {
                    let shadow_galley = painter.layout(
                        sample.to_string(),
                        font_id.clone(),
                        Color32::from_rgba_unmultiplied(0, 0, 0, 220),
                        max_wrap_w,
                    );
                    let shadow_pos = text_pos + egui::vec2(2.0, 2.0);
                    if outline_px > 0.0 {
                        for (dx, dy) in RING {
                            painter.galley(
                                shadow_pos + egui::vec2(dx, dy) * outline_px,
                                shadow_galley.clone(),
                                Color32::BLACK,
                            );
                        }
                    }
                    painter.galley(shadow_pos, shadow_galley, Color32::BLACK);
                }
                if outline_px > 0.0 {
                    let outline_galley =
                        painter.layout(sample.to_string(), font_id, outline_col, max_wrap_w);
                    for (dx, dy) in RING {
                        painter.galley(
                            text_pos + egui::vec2(dx, dy) * outline_px,
                            outline_galley.clone(),
                            outline_col,
                        );
                    }
                }
                painter.galley(text_pos, text_galley, sub_fg);
            });
    }

    fn render_tabs(&mut self, ui: &mut egui::Ui, pal: Palette) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let tabs = [
                (Tab::Appearance, self.t("Appearance", "Gorunum")),
                (Tab::Engine, self.t("Engine", "Motor")),
                (
                    Tab::ProfilesOcr,
                    self.t("Profiles & OCR", "Profiller & OCR"),
                ),
                (Tab::LiveLog, self.t("Live Log", "Canli Kayit")),
                (Tab::Setup, self.t("Setup", "Kurulum")),
            ];

            let region = self.handle.current_region();
            let setup_needs_attention =
                !self.source_ocr_installed || region.width < 60 || region.height < 32;

            for (tab, label) in tabs {
                let active = self.active_tab == tab;
                let text = RichText::new(label).size(13.0).color(if active {
                    pal.text_primary
                } else {
                    pal.text_muted
                });
                let text = if active { text.strong() } else { text };
                let resp = ui.add(egui::Label::new(text).sense(Sense::click()));
                if resp.clicked() {
                    self.active_tab = tab;
                    if tab == Tab::Setup {
                        self.refresh_ocr_diagnostics();
                    }
                }
                if tab == Tab::Setup && setup_needs_attention {
                    let dot_center = egui::pos2(resp.rect.right() + 5.0, resp.rect.center().y);
                    ui.painter().circle_filled(dot_center, 3.0, pal.status_amber);
                }
                if active {
                    let y = resp.rect.bottom() + 4.0;
                    ui.painter().line_segment(
                        [
                            egui::pos2(resp.rect.left(), y),
                            egui::pos2(resp.rect.right(), y),
                        ],
                        Stroke::new(2.0_f32, pal.accent),
                    );
                }
            }
        });
        ui.add_space(4.0);
        let rect = ui.available_rect_before_wrap();
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.top()),
                egui::pos2(rect.right(), rect.top()),
            ],
            Stroke::new(1.0_f32, pal.border),
        );
        ui.add_space(6.0);
    }

    /// "Style for: [Global] [game.exe] [Remove]" selector above the presets.
    fn render_style_scope(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let mut apps: Vec<String> = Vec::new();
        for app in [self.editing_app.clone(), self.handle.last_app()]
            .into_iter()
            .flatten()
        {
            if !apps.contains(&app) {
                apps.push(app);
            }
        }

        let mut next_scope: Option<Option<String>> = None;
        let mut remove_app: Option<String> = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                RichText::new(self.t("Style for", "Stil"))
                    .size(12.5)
                    .color(pal.text_muted),
            );

            let scope_button = |ui: &mut egui::Ui, label: &str, selected: bool| -> bool {
                let btn = egui::Button::new(RichText::new(label).size(11.5).color(if selected {
                    pal.accent_text
                } else {
                    pal.text_primary
                }))
                .fill(if selected { pal.accent } else { pal.input_bg });
                ui.add(btn).clicked()
            };

            if scope_button(ui, self.t("Global", "Genel"), self.editing_app.is_none())
                && self.editing_app.is_some()
            {
                next_scope = Some(None);
            }
            for app in &apps {
                let selected = self.editing_app.as_deref() == Some(app.as_str());
                if scope_button(ui, app, selected) && !selected {
                    next_scope = Some(Some(app.clone()));
                }
            }

            if let Some(app) = &self.editing_app {
                if ui
                    .button(self.t("Remove App Style", "Uygulama Stilini Sil"))
                    .clicked()
                {
                    remove_app = Some(app.clone());
                }
            }
        });

        if self.editing_app.is_none() && apps.is_empty() {
            ui.label(
                RichText::new(self.t(
                    "Focus a game or video player once to give it its own subtitle style.",
                    "Bir oyun veya oynatici odaklandiginda ona ozel altyazi stili verebilirsin.",
                ))
                .size(11.5)
                .color(pal.text_muted),
            );
        } else if !self.auto_profiles {
            ui.label(
                RichText::new(self.t(
                    "Per-app styles are ignored while Auto-Switch by App is off.",
                    "Uygulama stilleri, Auto-Switch by App kapaliyken kullanilmaz.",
                ))
                .size(11.5)
                .color(pal.status_amber),
            );
        }

        if let Some(app) = remove_app {
            self.handle.remove_app_style(&app);
            self.editing_app = None;
        } else if let Some(scope) = next_scope {
            self.set_style_scope(scope);
        }
    }

    fn render_appearance_tab(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let lang = self.ui_lang;
        let mut changed = false;

        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                self.render_style_scope(ui, pal);
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.t("Presets", "Hazir Stiller"))
                            .size(12.5)
                            .color(pal.text_muted),
                    );
                    ui.add_space(6.0);
                    for (id, label) in [
                        ("cinema", self.t("Cinema", "Sinema")),
                        ("yellow", self.t("Yellow", "Sari")),
                        ("minimal", self.t("Minimal", "Minimal")),
                        ("contrast", self.t("High Contrast", "Yuksek Kontrast")),
                    ] {
                        if ui.button(label).clicked() {
                            self.apply_preset(id);
                        }
                    }
                });

                ui.add_space(12.0);

                ui.columns(2, |cols| {
                    // Left column: Text, then Effects.
                    {
                        let ui = &mut cols[0];
                        ui.spacing_mut().item_spacing.y = 8.0;

                        section_header(ui, pal, tr(lang, "Text", "Yazi"));

                        ui.horizontal(|ui| {
                            row_label(ui, tr(lang, "Font Family", "Font Family"));
                            ComboBox::from_id_salt("font_family_combo")
                                .selected_text(&self.style.font_family)
                                .width(150.0)
                                .show_ui(ui, |ui| {
                                    for fam in FONT_FAMILIES {
                                        if ui
                                            .selectable_value(
                                                &mut self.style.font_family,
                                                (*fam).to_string(),
                                                *fam,
                                            )
                                            .changed()
                                        {
                                            changed = true;
                                        }
                                    }
                                });
                        });
                        changed |= slider_row(
                            ui,
                            pal,
                            tr(lang, "Font Size", "Font Size"),
                            &mut self.style.font_size,
                            14..=64,
                            |v| format!("{v} px"),
                        );
                        changed |= color_row(
                            ui,
                            pal,
                            tr(lang, "Text Color", "Text Color"),
                            &mut self.style.text_color,
                            TEXT_SWATCHES,
                            [255, 255, 255],
                        );
                        ui.horizontal(|ui| {
                            row_label(ui, "");
                            changed |= ui
                                .checkbox(&mut self.style.font_bold, tr(lang, "Bold", "Bold"))
                                .changed();
                        });

                        ui.add_space(10.0);
                        section_header(ui, pal, tr(lang, "Effects", "Efektler"));

                        ui.horizontal(|ui| {
                            row_label(ui, "");
                            changed |= ui
                                .checkbox(
                                    &mut self.style.text_shadow,
                                    tr(lang, "Text Shadow", "Text Shadow"),
                                )
                                .changed();
                        });
                        changed |= slider_row(
                            ui,
                            pal,
                            tr(lang, "Outline Width", "Outline Width"),
                            &mut self.style.outline_width,
                            0..=MAX_OUTLINE_WIDTH,
                            |v| format!("{v} px"),
                        );
                        changed |= color_row(
                            ui,
                            pal,
                            tr(lang, "Outline Color", "Outline Color"),
                            &mut self.style.outline_color,
                            OUTLINE_SWATCHES,
                            [0, 0, 0],
                        );
                    }

                    // Right column: the background box.
                    {
                        let ui = &mut cols[1];
                        ui.spacing_mut().item_spacing.y = 8.0;

                        section_header(ui, pal, tr(lang, "Box", "Kutu"));

                        changed |= color_row(
                            ui,
                            pal,
                            tr(lang, "Background Color", "Background Color"),
                            &mut self.style.bg_color,
                            BG_SWATCHES,
                            [10, 10, 12],
                        );
                        let mut opacity = self.style.opacity as i32;
                        if slider_row(
                            ui,
                            pal,
                            tr(lang, "Background Opacity", "Arkaplan Opakligi"),
                            &mut opacity,
                            0..=255,
                            |v| format!("{}%", ((v as f32 / 255.0) * 100.0).round() as i32),
                        ) {
                            self.style.opacity = opacity.clamp(0, 255) as u8;
                            changed = true;
                        }
                        changed |= slider_row(
                            ui,
                            pal,
                            tr(lang, "Border Radius", "Border Radius"),
                            &mut self.style.corner_radius,
                            0..=32,
                            |v| format!("{v} px"),
                        );
                        changed |= slider_row(
                            ui,
                            pal,
                            tr(lang, "Padding X", "Padding X"),
                            &mut self.style.padding_x,
                            4..=60,
                            |v| format!("{v} px"),
                        );
                        changed |= slider_row(
                            ui,
                            pal,
                            tr(lang, "Padding Y", "Padding Y"),
                            &mut self.style.padding_y,
                            2..=40,
                            |v| format!("{v} px"),
                        );
                        ui.horizontal(|ui| {
                            row_label(ui, "");
                            changed |= ui
                                .checkbox(
                                    &mut self.style.fit_to_text,
                                    tr(lang, "Auto-Fit Container", "Auto-Fit Container"),
                                )
                                .changed();
                        });
                    }
                });

                ui.add_space(10.0);
                section_header(ui, pal, tr(lang, "Placement", "Konumlandirma"));

                ui.horizontal(|ui| {
                    row_label(ui, tr(lang, "Output Mode", "Cikti Modu"));
                    for (mode, label) in [
                        (
                            OutputMode::Cover,
                            tr(lang, "Cover Original", "Orijinali Kapat"),
                        ),
                        (OutputMode::Custom, tr(lang, "Custom Area", "Ozel Alan")),
                    ] {
                        let selected = self.style.output_mode == mode;
                        let btn =
                            egui::Button::new(RichText::new(label).size(11.5).color(if selected {
                                pal.accent_text
                            } else {
                                pal.text_primary
                            }))
                            .fill(if selected { pal.accent } else { pal.input_bg });
                        if ui.add(btn).clicked() && self.style.output_mode != mode {
                            self.style.output_mode = mode;
                            changed = true;
                        }
                    }
                });

                if self.style.output_mode == OutputMode::Custom {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        row_label(ui, tr(lang, "Output Area", "Cikti Alani"));
                        let area_text = match self.style.output_region {
                            Some(r) => format!("{}x{} @ {},{}", r.width, r.height, r.left, r.top),
                            None => tr(lang, "Not set", "Secilmedi").to_string(),
                        };
                        ui.label(
                            RichText::new(area_text)
                                .font(FontId::new(12.0, FontFamily::Monospace))
                                .color(if self.style.output_region.is_some() {
                                    pal.text_primary
                                } else {
                                    pal.status_amber
                                }),
                        );
                        ui.add_space(6.0);
                        if ui
                            .button(tr(lang, "Select Output Area", "Cikti Alani Sec"))
                            .clicked()
                        {
                            self.handle.trigger_select_output(self.editing_app.clone());
                        }
                        if ui
                            .add_enabled(
                                self.style.output_region.is_some(),
                                egui::Button::new(tr(lang, "Clear", "Temizle")),
                            )
                            .clicked()
                        {
                            self.style.output_region = None;
                            changed = true;
                        }
                    });
                }

                ui.add_space(4.0);
                ui.columns(2, |cols| {
                    changed |= slider_row(
                        &mut cols[0],
                        pal,
                        tr(lang, "Offset X", "Offset X"),
                        &mut self.style.offset_x,
                        -300..=300,
                        |v| format!("{v} px"),
                    );
                    cols[1].horizontal(|ui| {
                        row_label(ui, tr(lang, "Offset Y", "Offset Y"));
                        ui.spacing_mut().slider_width = 150.0;
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut self.style.offset_y, -300..=300)
                                    .show_value(false),
                            )
                            .changed();
                        ui.add_sized(
                            [56.0, 20.0],
                            egui::Label::new(
                                RichText::new(format!("{} px", self.style.offset_y))
                                    .font(FontId::new(12.0, FontFamily::Monospace))
                                    .color(pal.text_muted),
                            ),
                        );
                        if ui
                            .add_enabled(
                                self.style.offset_x != 0 || self.style.offset_y != 0,
                                egui::Button::new(tr(lang, "Reset", "Sifirla")),
                            )
                            .clicked()
                        {
                            self.style.offset_x = 0;
                            self.style.offset_y = 0;
                            changed = true;
                        }
                    });
                });
            });

        if changed {
            self.commit_style();
        }
    }

    fn render_engine_tab(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let lang = self.ui_lang;
        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                egui::Grid::new("engine_grid")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .min_col_width(150.0)
                    .show(ui, |ui| {
                        ui.label(self.t("Engine", "Engine"));
                        ComboBox::from_id_salt("engine_combo")
                            .selected_text(&self.engine)
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                for eng in ENGINES {
                                    ui.selectable_value(&mut self.engine, (*eng).to_string(), *eng);
                                }
                            });
                        ui.end_row();

                        ui.label(self.t("Source / Target Lang", "Kaynak / Hedef Dil"));
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.source_lang)
                                    .desired_width(64.0)
                                    .font(FontId::new(12.5, FontFamily::Monospace)),
                            );
                            ui.label("->");
                            ui.add(
                                TextEdit::singleline(&mut self.target_lang)
                                    .desired_width(64.0)
                                    .font(FontId::new(12.5, FontFamily::Monospace)),
                            );
                        });
                        ui.end_row();

                        ui.label("Groq Model");
                        ui.add(
                            TextEdit::singleline(&mut self.groq_model)
                                .desired_width(300.0)
                                .font(FontId::new(12.0, FontFamily::Monospace)),
                        );
                        ui.end_row();

                        ui.label("Groq API Key");
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.groq_key_input)
                                    .password(true)
                                    .desired_width(240.0)
                                    .hint_text(tr(
                                        lang,
                                        "Leave blank to keep existing",
                                        "Mevcut anahtari korumak icin bos birakin",
                                    )),
                            );
                            let badge = if self.has_groq_key { "SET" } else { "EMPTY" };
                            let col = if self.has_groq_key {
                                pal.status_green
                            } else {
                                pal.text_muted
                            };
                            ui.label(
                                RichText::new(badge)
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .color(col),
                            );
                        });
                        ui.end_row();

                        ui.label("Gemini Model");
                        ui.add(
                            TextEdit::singleline(&mut self.gemini_model)
                                .desired_width(300.0)
                                .font(FontId::new(12.0, FontFamily::Monospace)),
                        );
                        ui.end_row();

                        ui.label("Gemini API Key");
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.gemini_key_input)
                                    .password(true)
                                    .desired_width(240.0)
                                    .hint_text(tr(
                                        lang,
                                        "Leave blank to keep existing",
                                        "Mevcut anahtari korumak icin bos birakin",
                                    )),
                            );
                            let badge = if self.has_gemini_key { "SET" } else { "EMPTY" };
                            let col = if self.has_gemini_key {
                                pal.status_green
                            } else {
                                pal.text_muted
                            };
                            ui.label(
                                RichText::new(badge)
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .color(col),
                            );
                        });
                        ui.end_row();

                        ui.label("Custom Base URL");
                        ui.add(
                            TextEdit::singleline(&mut self.custom_base_url)
                                .desired_width(300.0)
                                .font(FontId::new(12.0, FontFamily::Monospace)),
                        );
                        ui.end_row();

                        ui.label("Custom Model");
                        ui.add(
                            TextEdit::singleline(&mut self.custom_model)
                                .desired_width(300.0)
                                .font(FontId::new(12.0, FontFamily::Monospace)),
                        );
                        ui.end_row();

                        ui.label("Custom API Key");
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut self.custom_key_input)
                                    .password(true)
                                    .desired_width(240.0)
                                    .hint_text(tr(
                                        lang,
                                        "Leave blank to keep existing",
                                        "Mevcut anahtari korumak icin bos birakin",
                                    )),
                            );
                            let badge = if self.has_custom_key { "SET" } else { "EMPTY" };
                            let col = if self.has_custom_key {
                                pal.status_green
                            } else {
                                pal.text_muted
                            };
                            ui.label(
                                RichText::new(badge)
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .color(col),
                            );
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);
                // Ayar kaydetme ile işçinin hazır olması farklı aşamalardır.
                let runtime = self.handle.snapshot_engine_status();
                ui.label(format!(
                    "{}: {} | {}: {} {}",
                    self.t("Selected", "Secilen"),
                    self.engine,
                    self.t("Active", "Aktif"),
                    if runtime.ready.is_empty() {
                        "—"
                    } else {
                        &runtime.ready
                    },
                    runtime.model
                ));
                if runtime.loading {
                    ui.label(self.t("Preparing engine...", "Motor hazirlaniyor..."));
                }
                if let Some(message) = &runtime.message {
                    ui.colored_label(pal.status_amber, message);
                }
                if let Some(last) = &runtime.last_result {
                    ui.label(format!(
                        "{}: {} {}{}{}",
                        self.t("Last result", "Son sonuc"),
                        last.engine,
                        last.model,
                        if last.cached { " · cache" } else { "" },
                        if last.fallback { " · fallback" } else { "" }
                    ));
                }
                ui.label(self.t("Settings file:", "Ayar dosyasi:"));
                ui.label(RichText::new(AppConfig::path().display().to_string()).monospace());
                if let Some(ignored) = AppConfig::ignored_path() {
                    ui.colored_label(
                        pal.status_amber,
                        format!(
                            "{}: {}",
                            self.t("Ignored settings file", "Kullanilmayan ayar dosyasi"),
                            ignored.display()
                        ),
                    );
                }
                ui.horizontal(|ui| {
                    let btn = egui::Button::new(
                        RichText::new(self.t("Save & Reload Engine", "Kaydet ve Motoru Yenile"))
                            .color(pal.accent_text)
                            .strong(),
                    )
                    .fill(pal.accent);
                    if ui.add(btn).clicked() {
                        self.save_and_reload_engine();
                    }

                    if let Some(msg) = &self.engine_status_msg {
                        ui.add_space(8.0);
                        ui.label(RichText::new(msg).size(12.0).color(pal.text_muted));
                    }
                });
            });
    }

    fn render_profiles_ocr_tab(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let lang = self.ui_lang;
        let active_app = self.handle.active_app();
        let cfg = self.handle.snapshot_config();

        // Yerel görüntü kaydı her açılışta kapalı başlar ve açıkça etkinleştirilir.
        let mut diagnostics = self.handle.ocr_diagnostics.load(Ordering::SeqCst);
        if ui
            .checkbox(
                &mut diagnostics,
                self.t(
                    "Record OCR diagnostics locally (max 60 samples)",
                    "OCR teshisini yerel kaydet (en fazla 60 ornek)",
                ),
            )
            .changed()
        {
            self.handle
                .ocr_diagnostics
                .store(diagnostics, Ordering::SeqCst);
        }
        ui.label(self.t(
            "Contains captured subtitle images and text. Recording starts when resumed.",
            "Yakalanan altyazi goruntulerini ve metni icerir. Kayit devam ettirince baslar.",
        ));
        ui.label(RichText::new(crate::diagnostics::directory().display().to_string()).monospace());
        ui.add_space(8.0);

        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Stream Mode (Discord / OBS)")
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                );
                ui.add_space(6.0);
                let mut stream_mode = cfg.stream_mode;
                if ui
                    .checkbox(
                        &mut stream_mode,
                        self.t(
                            "Enable Stream Mode (WGC Window Capture)",
                            "Stream Mode Acik (WGC Pencere Yakalama)",
                        ),
                    )
                    .changed()
                {
                    self.handle.set_stream_mode(stream_mode);
                }
                ui.add_space(3.0);
                ui.label(
                    RichText::new(self.t(
                        "Captures the target window directly via GPU (WGC) and makes the overlay visible in Discord/OBS screen share (share Screen, not Application).",
                        "Hedef pencereyi dogrudan GPU'dan (WGC) yakalar ve altyaziyi Discord/OBS ekran paylasiminda gorunur yapar (Uygulama degil Ekran paylasin).",
                    ))
                    .size(11.5)
                    .color(pal.text_muted),
                );
            });

        ui.add_space(10.0);

        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(self.t("App Profiles", "App Profiles"))
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                );
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label(self.t("Active App:", "Aktif Uygulama:"));
                    ui.label(
                        RichText::new(
                            active_app
                                .as_deref()
                                .unwrap_or_else(|| self.t("None", "Yok")),
                        )
                        .font(FontId::new(12.0, FontFamily::Monospace))
                        .color(pal.text_primary),
                    );
                    ui.add_space(18.0);
                    if ui
                        .checkbox(
                            &mut self.auto_profiles,
                            tr(lang, "Auto-Switch by App", "Auto-Switch by App"),
                        )
                        .changed()
                    {
                        let mut updated = cfg.clone();
                        updated.auto_profiles = self.auto_profiles;
                        let _ = self.handle.update_config_and_save(updated);
                    }
                });

                ui.add_space(8.0);

                if cfg.profiles.is_empty() && cfg.profile_styles.is_empty() {
                    ui.label(
                        RichText::new(self.t(
                            "No app profiles saved yet. Use Ctrl+Shift+S while focused on an app.",
                            "Henuz kayitli profil yok. Bir uygulama odakliyken Ctrl+Shift+S kullanin.",
                        ))
                        .size(12.0)
                        .color(pal.text_muted),
                    );
                } else {
                    let mut apps: Vec<String> = cfg
                        .profiles
                        .keys()
                        .chain(cfg.profile_styles.keys())
                        .cloned()
                        .collect();
                    apps.sort();
                    apps.dedup();

                    egui::Grid::new("profiles_table")
                        .num_columns(4)
                        .spacing([20.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(self.t("Application", "Uygulama"))
                                    .size(11.5)
                                    .color(pal.text_muted),
                            );
                            ui.label(
                                RichText::new("Region")
                                    .size(11.5)
                                    .color(pal.text_muted),
                            );
                            ui.label(
                                RichText::new(self.t("Style", "Stil"))
                                    .size(11.5)
                                    .color(pal.text_muted),
                            );
                            ui.label("");
                            ui.end_row();

                            for app in apps {
                                ui.label(
                                    RichText::new(&app)
                                        .font(FontId::new(12.0, FontFamily::Monospace)),
                                );
                                let region_text = match cfg.profiles.get(&app) {
                                    Some(reg) => format!(
                                        "{}x{} @ {},{}",
                                        reg.width, reg.height, reg.left, reg.top
                                    ),
                                    None => "-".to_string(),
                                };
                                ui.label(
                                    RichText::new(region_text)
                                        .font(FontId::new(12.0, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                                let style_text = if cfg.profile_styles.contains_key(&app) {
                                    self.t("custom", "ozel")
                                } else {
                                    "-"
                                };
                                ui.label(
                                    RichText::new(style_text)
                                        .font(FontId::new(12.0, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                                if ui.button(self.t("Remove", "Kaldir")).clicked() {
                                    let _ = self.handle.delete_profile(&app);
                                }
                                ui.end_row();
                            }
                        });
                }
            });

        ui.add_space(10.0);

        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("OCR Diagnostics")
                            .strong()
                            .size(13.0)
                            .color(pal.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(self.t("Refresh", "Yenile")).clicked() {
                            self.refresh_ocr_diagnostics();
                        }
                    });
                });
                ui.add_space(6.0);

                let langs_text = if self.installed_ocr_langs.is_empty() {
                    self.t("None detected", "Bulunamadi").to_string()
                } else {
                    self.installed_ocr_langs.join(", ")
                };

                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(self.t(
                            "Installed Windows OCR Languages:",
                            "Kurulu Windows OCR Dilleri:",
                        ))
                        .color(pal.text_muted),
                    );
                    ui.label(
                        RichText::new(langs_text)
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .color(pal.text_primary),
                    );
                });

                if !self.source_ocr_installed {
                    ui.add_space(8.0);
                    let resp = Frame::none()
                        .fill(pal.input_bg)
                        .stroke(Stroke::new(1.0_f32, pal.border))
                        .rounding(Rounding::same(3.0))
                        .inner_margin(Margin::symmetric(10.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} ({})",
                                        self.t(
                                            "Source OCR language pack is not installed in Windows.",
                                            "Kaynak OCR dil paketi Windows uzerinde kurulu degil."
                                        ),
                                        self.source_lang
                                    ))
                                    .color(pal.status_red),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .button(self.t(
                                            "Open Windows Language Settings",
                                            "Windows Dil Ayarlarini Ac",
                                        ))
                                        .clicked()
                                    {
                                        let _ = Command::new("cmd")
                                            .args(["/C", "start", "", "ms-settings:regionlanguage"])
                                            .spawn();
                                    }
                                });
                            });
                        });
                    // Muted red left-border indicator
                    let r = resp.response.rect;
                    ui.painter().line_segment(
                        [
                            egui::pos2(r.left() + 1.5, r.top() + 2.0),
                            egui::pos2(r.left() + 1.5, r.bottom() - 2.0),
                        ],
                        Stroke::new(3.0_f32, pal.status_red),
                    );
                }
            });
    }

    /// Shows the newest undismissed notice (config reset, engine fallback, ...) under the top bar.
    fn render_notice_banner(&mut self, ctx: &egui::Context, pal: Palette) {
        let Some(notice) = self
            .handle
            .recent_notices()
            .into_iter()
            .last()
            .filter(|n| n.id > self.dismissed_notice_id)
        else {
            return;
        };

        let color = notice_color(pal, notice.level);
        let level_label = match notice.level {
            NoticeLevel::Warning => self.t("Warning", "Uyari"),
            NoticeLevel::Error => self.t("Error", "Hata"),
        };
        let dismiss_label = self.t("Dismiss", "Kapat");
        let mut head = format!(
            "{} | {} | {}",
            level_label,
            notice.source.to_uppercase(),
            notice.timestamp
        );
        if notice.count > 1 {
            head.push_str(&format!(" | x{}", notice.count));
        }

        let mut dismissed = false;
        egui::TopBottomPanel::top("notice_banner")
            .frame(
                Frame::none()
                    .fill(pal.window_bg)
                    .inner_margin(Margin::symmetric(14.0, 0.0)),
            )
            .show(ctx, |ui| {
                Frame::none()
                    .fill(pal.surface_bg)
                    .stroke(Stroke::new(1.0_f32, color))
                    .rounding(Rounding::same(4.0))
                    .inner_margin(Margin::symmetric(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (dot_rect, _) =
                                ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter().circle_filled(dot_rect.center(), 4.0, color);
                            ui.label(
                                RichText::new(head)
                                    .font(FontId::new(11.5, FontFamily::Monospace))
                                    .color(pal.text_muted),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button(dismiss_label).clicked() {
                                    dismissed = true;
                                }
                            });
                        });
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(&notice.message)
                                .font(FontId::new(12.0, FontFamily::Monospace))
                                .color(pal.text_primary),
                        );
                    });
                ui.add_space(6.0);
            });

        if dismissed {
            self.dismissed_notice_id = notice.id;
        }
    }

    fn render_events_section(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let notices = self.handle.recent_notices();
        if notices.is_empty() {
            return;
        }

        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} ({})",
                            self.t("Events", "Olaylar"),
                            notices.len()
                        ))
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(self.t("Clear", "Temizle")).clicked() {
                            self.handle.clear_notices();
                        }
                    });
                });

                ui.add_space(6.0);

                ScrollArea::vertical()
                    .max_height(120.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for n in notices.iter().rev() {
                            let level = match n.level {
                                NoticeLevel::Warning => "WARN",
                                NoticeLevel::Error => "ERROR",
                            };
                            let times = if n.count > 1 {
                                format!(" (x{})", n.count)
                            } else {
                                String::new()
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{} [{} | {}] {}{}",
                                    n.timestamp, level, n.source, n.message, times
                                ))
                                .font(FontId::new(11.5, FontFamily::Monospace))
                                .color(notice_color(pal, n.level)),
                            );
                        }
                    });
            });

        ui.add_space(8.0);
    }

    fn render_live_log_tab(&mut self, ui: &mut egui::Ui, pal: Palette) {
        ui.horizontal(|ui| {
            let visible = self.handle.history_visible.load(Ordering::SeqCst);
            let label = if visible {
                self.t("Hide History", "Gecmisi Gizle")
            } else {
                self.t("Show Last 15 Translations", "Son 15 Ceviriyi Goster")
            };
            if ui.button(label).clicked() {
                self.handle.toggle_history();
            }
            ui.label(
                RichText::new("Ctrl+Shift+L")
                    .monospace()
                    .color(pal.text_muted),
            );
        });
        ui.add_space(6.0);
        ui.label(self.t(
            "Total: capture start to result ready; excludes Remote Play and display time.",
            "Toplam: yakalama baslangicindan sonuc hazir olana kadar; Remote Play ve cizim haric.",
        ));
        self.render_events_section(ui, pal);
        let logs = self.handle.recent_logs();

        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} ({})",
                            self.t("Translation Log", "Ceviri Kayitlari"),
                            logs.len()
                        ))
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(self.t("Clear", "Temizle")).clicked() {
                            self.handle.clear_logs();
                        }
                    });
                });

                ui.add_space(6.0);

                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if logs.is_empty() {
                            ui.label(
                                RichText::new(
                                    self.t(
                                        "No translations recorded yet.",
                                        "Henuz ceviri kaydi yok.",
                                    ),
                                )
                                .font(FontId::new(12.0, FontFamily::Monospace))
                                .color(pal.text_muted),
                            );
                        } else {
                            for entry in logs.iter().rev() {
                                ui.label(
                                    RichText::new(format!(
                                        "{} [{} ms | OCR {} | Wait {} | Translate {} ms | {} {}{}{} | {} attempts{} | Primary {} / Fallback {} ms] {} -> {}",
                                        entry.timestamp, entry.total_ms, entry.ocr_ms, entry.wait_ms,
                                        entry.translation_ms, entry.engine, entry.report.model,
                                        if entry.report.cached { " | cache" } else { "" },
                                        if entry.report.fallback { " | fallback" } else { "" },
                                        entry.report.attempts,
                                        entry.report.endpoint.as_ref().map(|s| format!(" | {s}")).unwrap_or_default(),
                                        entry.report.primary_ms, entry.report.fallback_ms,
                                        entry.src, entry.dst
                                    ))
                                    .font(FontId::new(11.5, FontFamily::Monospace))
                                    .color(pal.text_primary),
                                );
                            }
                        }
                    });
            });
    }

    fn render_setup_tab(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let cfg = self.handle.snapshot_config();
        let region = self.handle.current_region();
        let region_ok = region.width >= 60 && region.height >= 32;
        let cap_opt = ocr_capability_name(&cfg.source_lang);

        let engine_label = cfg.engine.to_lowercase();
        let engine_has_key = match engine_label.as_str() {
            "groq" => key_available(cfg.groq_api_key.as_deref(), "GROQ_API_KEY"),
            "gemini" => key_available(cfg.gemini_api_key.as_deref(), "GEMINI_API_KEY"),
            "openai-compat" | "custom" | "ollama" => true,
            _ => true, // Google keyless
        };
        let is_google_only = engine_label == "google";

        // Step 1: Windows OCR Language Pack
        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (badge, badge_col) = if self.source_ocr_installed {
                        ("[OK]", pal.status_green)
                    } else {
                        ("[MISSING]", pal.status_red)
                    };
                    ui.label(
                        RichText::new(badge)
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .strong()
                            .color(badge_col),
                    );
                    ui.label(
                        RichText::new(format!(
                            "1. {} ({})",
                            self.t("Windows OCR Language Pack", "Windows OCR Dil Paketi"),
                            cfg.source_lang
                        ))
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(self.t("Recheck", "Tekrar Kontrol")).clicked() {
                            self.refresh_ocr_diagnostics();
                        }
                    });
                });
                ui.add_space(5.0);
                ui.label(
                    RichText::new(self.t(
                        "Required for accurate subtitle reading. Does not change your Windows display language (1 MB OCR capability only).",
                        "Altyazilarin hatasiz okunmasi icin gereklidir. Windows goruntuleme dilini degistirmez (sadece 1 MB OCR ozelligi).",
                    ))
                    .size(11.5)
                    .color(pal.text_muted),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if let Some(cap) = cap_opt {
                        let install_btn = egui::Button::new(
                            RichText::new(self.t(
                                "Install OCR Pack (Admin)",
                                "OCR Paketini Kur (Yonetici)",
                            ))
                            .color(if !self.source_ocr_installed {
                                pal.accent_text
                            } else {
                                pal.text_primary
                            }),
                        )
                        .fill(if !self.source_ocr_installed {
                            pal.accent
                        } else {
                            pal.input_bg
                        });
                        if ui.add(install_btn).clicked() {
                            if let Err(e) = launch_ocr_pack_installer(cap) {
                                self.handle.push_notice(
                                    NoticeLevel::Error,
                                    "ocr",
                                    format!("Could not launch OCR installer: {e}"),
                                );
                            }
                        }
                    }
                    if ui
                        .button(self.t("Open Windows Settings", "Windows Ayarlarini Ac"))
                        .clicked()
                    {
                        let _ = Command::new("cmd")
                            .args(["/C", "start", "", "ms-settings:regionlanguage"])
                            .spawn();
                    }
                    let installed = if self.installed_ocr_langs.is_empty() {
                        "-".to_string()
                    } else {
                        self.installed_ocr_langs.join(", ")
                    };
                    ui.label(
                        RichText::new(format!("Installed: {installed}"))
                            .font(FontId::new(11.5, FontFamily::Monospace))
                            .color(pal.text_muted),
                    );
                });
            });

        ui.add_space(8.0);

        // Step 2: Subtitle Capture Region
        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (badge, badge_col) = if region_ok {
                        ("[OK]", pal.status_green)
                    } else {
                        ("[MISSING]", pal.status_red)
                    };
                    ui.label(
                        RichText::new(badge)
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .strong()
                            .color(badge_col),
                    );
                    ui.label(
                        RichText::new(self.t(
                            "2. Subtitle Capture Region",
                            "2. Altyazi Yakalama Bolgesi",
                        ))
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                    );
                });
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    let sel_btn = egui::Button::new(
                        RichText::new(self.t(
                            "Select Region (Ctrl+Shift+S)",
                            "Bolge Sec (Ctrl+Shift+S)",
                        ))
                        .color(if !region_ok {
                            pal.accent_text
                        } else {
                            pal.text_primary
                        }),
                    )
                    .fill(if !region_ok {
                        pal.accent
                    } else {
                        pal.input_bg
                    });
                    if ui.add(sel_btn).clicked() {
                        self.handle.trigger_reselect();
                    }
                    let reg_txt = if region_ok {
                        format!(
                            "{}x{} @ {},{}",
                            region.width, region.height, region.left, region.top
                        )
                    } else {
                        self.t("Not selected yet", "Henuz secilmedi").to_string()
                    };
                    ui.label(
                        RichText::new(reg_txt)
                            .font(FontId::new(11.5, FontFamily::Monospace))
                            .color(pal.text_muted),
                    );
                });
            });

        ui.add_space(8.0);

        // Step 3: Translation Engine (Optional Speed Upgrade)
        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (badge, badge_col) = if !engine_has_key {
                        ("[KEY MISSING]", pal.status_amber)
                    } else if is_google_only {
                        ("[READY]", pal.status_green)
                    } else {
                        ("[OK]", pal.status_green)
                    };
                    ui.label(
                        RichText::new(badge)
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .strong()
                            .color(badge_col),
                    );
                    ui.label(
                        RichText::new(format!(
                            "3. {} ({})",
                            self.t("Translation Engine", "Ceviri Motoru"),
                            cfg.engine
                        ))
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                    );
                });
                ui.add_space(5.0);
                ui.label(
                    RichText::new(self.t(
                        "Google Translate works immediately with no key (~300-600 ms). For ~180 ms natural game/slang translation, get a free Groq API key and paste it in the Engine tab.",
                        "Google Translate anahtarsiz aninda calisir (~300-600 ms). ~180 ms hiz ve dogal oyun/argo cevirisi icin ucretsiz Groq anahtari alip Motor sekmesine yapistirin.",
                    ))
                    .size(11.5)
                    .color(pal.text_muted),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(self.t("Get Free Groq Key", "Ucretsiz Groq Anahtari Al"))
                        .clicked()
                    {
                        let _ = Command::new("cmd")
                            .args(["/C", "start", "", "https://console.groq.com/keys"])
                            .spawn();
                    }
                    if ui
                        .button(self.t("Get Gemini Key", "Gemini Anahtari Al"))
                        .clicked()
                    {
                        let _ = Command::new("cmd")
                            .args(["/C", "start", "", "https://aistudio.google.com/apikey"])
                            .spawn();
                    }
                    if ui
                        .button(self.t("Open Engine Tab", "Motor Sekmesini Ac"))
                        .clicked()
                    {
                        self.active_tab = Tab::Engine;
                    }
                });
            });

        ui.add_space(8.0);

        // Step 4: Game & Stream Checklist
        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("[INFO]")
                            .font(FontId::new(12.0, FontFamily::Monospace))
                            .strong()
                            .color(pal.text_muted),
                    );
                    ui.label(
                        RichText::new(self.t(
                            "4. Game & Discord Stream Checklist",
                            "4. Oyun ve Discord Yayin Kontrol Listesi",
                        ))
                        .strong()
                        .size(13.0)
                        .color(pal.text_primary),
                    );
                });
                ui.add_space(5.0);
                for (en, tr_txt) in [
                    (
                        "- Display mode: set games to Borderless Windowed (Exclusive Fullscreen hides desktop overlays).",
                        "- Goruntu modu: oyunlari Borderless Windowed (Cercevesiz Tam Ekran) yapin.",
                    ),
                    (
                        "- In-game subtitles: enable Subtitle Background (Dark) if available for higher OCR accuracy.",
                        "- Oyun ici altyazi: mumkunse koyu altyazi arka planini (Subtitle Background) acin.",
                    ),
                    (
                        "- Discord / OBS stream: enable Stream Mode in Profiles & OCR, then share Screen (not Application).",
                        "- Discord / OBS yayini: Profiller & OCR altindan Stream Mode acin ve Uygulama degil Ekran paylasin.",
                    ),
                ] {
                    ui.label(
                        RichText::new(self.t(en, tr_txt))
                            .size(11.5)
                            .color(pal.text_muted),
                    );
                }
                ui.add_space(6.0);
                if ui
                    .button(self.t("Open Profiles & Stream", "Profiller ve Stream Ac"))
                    .clicked()
                {
                    self.active_tab = Tab::ProfilesOcr;
                }
            });
    }
}

impl eframe::App for OvercapApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Minimize-to-tray on window close unless quit was requested.
        if ctx.input(|i| i.viewport().close_requested())
            && !self.handle.stop_flag.load(Ordering::SeqCst)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        if self.handle.stop_flag.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        if self.handle.gui_open_requested.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        self.sync_from_handle_if_needed();
        // Panel dili GUI seçimiyle birlikte değişir; hedef çeviri dili bundan bağımsızdır.
        self.handle.set_history_language(self.ui_lang == UiLang::Tr);

        let pal = self.palette();

        egui::TopBottomPanel::top("top_bar")
            .frame(
                Frame::none()
                    .fill(pal.window_bg)
                    .inner_margin(Margin::symmetric(14.0, 10.0)),
            )
            .show(ctx, |ui| {
                self.render_top_bar(ui, pal);
            });

        self.render_notice_banner(ctx, pal);

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(24.0)
            .frame(
                Frame::none()
                    .fill(pal.surface_bg)
                    .stroke(Stroke::new(1.0_f32, pal.border))
                    .inner_margin(Margin::symmetric(14.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Ctrl+Shift+  L: History  |  S: Region  |  H: Pause  |  Up/Down: Size  |  A: Window  |  Q: Quit",
                    )
                    .font(FontId::new(11.0, FontFamily::Monospace))
                    .color(pal.text_muted),
                );
            });

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(pal.window_bg)
                    .inner_margin(Margin::symmetric(14.0, 6.0)),
            )
            .show(ctx, |ui| {
                self.render_preview_strip(ui, pal);
                ui.add_space(10.0);
                self.render_tabs(ui, pal);

                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.active_tab {
                        Tab::Appearance => self.render_appearance_tab(ui, pal),
                        Tab::Engine => self.render_engine_tab(ui, pal),
                        Tab::ProfilesOcr => self.render_profiles_ocr_tab(ui, pal),
                        Tab::LiveLog => self.render_live_log_tab(ui, pal),
                        Tab::Setup => self.render_setup_tab(ui, pal),
                    });
            });

        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

pub fn run_gui(handle: OverlayHandle, start_hidden: bool) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Overcap")
            // Fixed size: the layout is designed for exactly this window.
            .with_inner_size([900.0, 720.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_visible(!start_hidden)
            .with_icon(Arc::new(egui::IconData {
                rgba: tray::generate_icon_rgba(32),
                width: 32,
                height: 32,
            })),
        ..Default::default()
    };

    eframe::run_native(
        "Overcap",
        options,
        Box::new(move |cc| Ok(Box::new(OvercapApp::new(cc, handle)))),
    )
    .map_err(|e| anyhow!("eframe error: {e}"))
}

/// Anahtarın değeri taşınmadan, işçiyle aynı dosya/ortam önceliği kontrol edilir.
fn key_available(configured: Option<&str>, variable: &str) -> bool {
    configured.is_some_and(|s| !s.trim().is_empty())
        || std::env::var(variable)
            .ok()
            .is_some_and(|s| !s.trim().is_empty())
}
