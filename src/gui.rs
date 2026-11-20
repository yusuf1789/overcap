use anyhow::{anyhow, Result};
use eframe::egui::{
    self, Align, Color32, ComboBox, FontFamily, FontId, Frame, Layout, Margin, RichText, Rounding,
    ScrollArea, Sense, Stroke, TextEdit, Vec2, ViewportCommand,
};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppConfig;
use crate::ocr::{available_ocr_languages, is_language_installed};
use crate::overlay::{OverlayHandle, OverlayStyle};
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
            status_green: Color32::from_rgb(63, 185, 80),   // #3FB950
            status_amber: Color32::from_rgb(210, 153, 34),  // #D29922
            status_red: Color32::from_rgb(248, 81, 73),     // #F85149
        }
    }

    fn light() -> Self {
        Self {
            window_bg: Color32::from_rgb(236, 236, 236),    // #ECECEC
            surface_bg: Color32::from_rgb(247, 247, 247),   // #F7F7F7
            input_bg: Color32::from_rgb(255, 255, 255),     // #FFFFFF
            hover_bg: Color32::from_rgb(226, 226, 226),     // #E2E2E2
            active_bg: Color32::from_rgb(212, 212, 212),    // #D4D4D4
            border: Color32::from_rgb(208, 208, 208),       // #D0D0D0
            text_primary: Color32::from_rgb(26, 26, 26),    // #1A1A1A
            text_muted: Color32::from_rgb(98, 98, 98),      // #626262
            accent: Color32::from_rgb(46, 46, 46),          // #2E2E2E
            accent_text: Color32::from_rgb(247, 247, 247),
            status_green: Color32::from_rgb(45, 148, 62),
            status_amber: Color32::from_rgb(176, 124, 20),
            status_red: Color32::from_rgb(208, 52, 44),
        }
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

    // Live style state (synced with OverlayHandle)
    style: OverlayStyle,

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

        Self {
            handle,
            theme: ThemeMode::Dark,
            ui_lang: UiLang::En,
            active_tab: Tab::Appearance,
            stage_bg: StageBg::Dark,
            sample_idx: 0,
            overlay_preview_on: false,
            style,
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
            has_groq_key: cfg
                .groq_api_key
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            has_gemini_key: cfg
                .gemini_api_key
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            has_custom_key: cfg
                .custom_api_key
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            engine_status_msg: None,
            auto_profiles: cfg.auto_profiles,
            installed_ocr_langs,
            source_ocr_installed,
            last_checked_ocr_lang: cfg.source_lang,
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

    fn sync_from_handle_if_needed(&mut self) {
        // Keep font_size / auto_profiles in sync if changed by global hotkeys.
        if let Ok(st) = self.handle.state.lock() {
            if st.style.font_size != self.style.font_size {
                self.style.font_size = st.style.font_size;
            }
            self.auto_profiles = st.config.auto_profiles;
        }
    }

    fn refresh_ocr_diagnostics(&mut self) {
        self.installed_ocr_langs = available_ocr_languages();
        self.source_ocr_installed = is_language_installed(&self.source_lang);
        self.last_checked_ocr_lang = self.source_lang.clone();
    }

    fn apply_preset(&mut self, name: &str) {
        let (family, size, bold, text, bg, opacity, radius, shadow, fit, px, py) = match name {
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
            ),
            "minimal" => (
                "Segoe UI",
                28,
                false,
                "#F5F5F5",
                "#121212",
                185,
                12,
                true,
                true,
                14,
                6,
            ),
            _ => (
                "Segoe UI",
                32,
                true,
                "#FFFFFF",
                "#000000",
                250,
                4,
                false,
                false,
                20,
                10,
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
        };
        self.handle.apply_style(self.style.clone(), true);
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
            self.groq_key_input.clear();
            self.has_groq_key = true;
        }
        if !self.gemini_key_input.trim().is_empty() {
            cfg.gemini_api_key = Some(self.gemini_key_input.trim().to_string());
            self.gemini_key_input.clear();
            self.has_gemini_key = true;
        }
        if !self.custom_key_input.trim().is_empty() {
            cfg.custom_api_key = Some(self.custom_key_input.trim().to_string());
            self.custom_key_input.clear();
            self.has_custom_key = true;
        }

        match self.handle.update_config_and_save(cfg) {
            Ok(()) => {
                self.handle.request_engine_reload();
                if self.source_lang != self.last_checked_ocr_lang {
                    self.refresh_ocr_diagnostics();
                }
                self.engine_status_msg = Some(
                    self.t(
                        "Saved config.json and reloaded engine.",
                        "config.json kaydedildi ve motor yenilendi.",
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
        let active_engine = self.handle.snapshot_config().engine;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            ui.label(
                RichText::new("OVERCAP")
                    .strong()
                    .size(15.0)
                    .color(pal.text_primary),
            );
            ui.label(
                RichText::new("v0.3.0")
                    .font(FontId::new(11.5, FontFamily::Monospace))
                    .color(pal.text_muted),
            );

            ui.add_space(6.0);

            let (dot_color, status_text) = if !self.source_ocr_installed {
                (
                    pal.status_red,
                    self.t("Missing OCR", "OCR Eksik").to_string(),
                )
            } else if is_paused {
                (pal.status_amber, self.t("Paused", "Duraklatildi").to_string())
            } else if latency > 0 {
                (
                    pal.status_green,
                    format!(
                        "{} · {} · {} ms",
                        self.t("Running", "Calisiyor"),
                        active_engine,
                        latency
                    ),
                )
            } else {
                (
                    pal.status_green,
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

                if ui
                    .button(self.t("Select Region", "Bolge Sec"))
                    .clicked()
                {
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
                        let btn = egui::Button::new(
                            RichText::new(label).size(11.5).color(if selected {
                                pal.accent_text
                            } else {
                                pal.text_primary
                            }),
                        )
                        .fill(if selected { pal.accent } else { pal.input_bg });
                        if ui.add(btn).clicked() {
                            self.stage_bg = bg;
                        }
                    }

                    if ui
                        .button(self.t("Next Sample", "Sonraki Ornek"))
                        .clicked()
                    {
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
                        let btn = egui::Button::new(
                            RichText::new(preview_lbl).size(11.5).color(
                                if self.overlay_preview_on {
                                    pal.accent_text
                                } else {
                                    pal.text_primary
                                },
                            ),
                        )
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
                let (stage_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), stage_h),
                    Sense::hover(),
                );
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

                let text_galley = painter.layout(
                    sample.to_string(),
                    font_id.clone(),
                    sub_fg,
                    max_wrap_w,
                );
                let text_size = text_galley.size();

                let box_w = if self.style.fit_to_text {
                    (text_size.x + pad_x * 2.0).min(stage_rect.width() - 24.0)
                } else {
                    (stage_rect.width() - 24.0).max(120.0)
                };
                let box_h = (text_size.y + pad_y * 2.0)
                    .min(stage_rect.height() - 14.0)
                    .max(28.0);

                let box_rect = egui::Rect::from_center_size(
                    stage_rect.center(),
                    egui::vec2(box_w, box_h),
                );
                painter.rect_filled(box_rect, rounding, sub_bg);

                let text_pos = egui::pos2(
                    box_rect.center().x - text_size.x * 0.5,
                    box_rect.center().y - text_size.y * 0.5,
                );
                if self.style.text_shadow {
                    let shadow_galley = painter.layout(
                        sample.to_string(),
                        font_id,
                        Color32::from_rgba_unmultiplied(0, 0, 0, 220),
                        max_wrap_w,
                    );
                    painter.galley(
                        text_pos + egui::vec2(2.0, 2.0),
                        shadow_galley,
                        Color32::BLACK,
                    );
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
            ];

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

    fn render_appearance_tab(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let lang = self.ui_lang;
        let mut changed = false;

        Frame::none()
            .fill(pal.surface_bg)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(4.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
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

                ui.add_space(10.0);

                egui::Grid::new("appearance_grid")
                    .num_columns(2)
                    .spacing([24.0, 10.0])
                    .min_col_width(280.0)
                    .show(ui, |ui| {
                        // Row 1: Font Family | Font Size
                        ui.horizontal(|ui| {
                            ui.label(self.t("Font Family", "Font Family"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ComboBox::from_id_salt("font_family_combo")
                                    .selected_text(&self.style.font_family)
                                    .width(160.0)
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
                        });

                        ui.horizontal(|ui| {
                            ui.label(self.t("Font Size", "Font Size"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{} px", self.style.font_size))
                                        .font(FontId::new(12.0, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.style.font_size, 14..=64)
                                            .show_value(false),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                        });
                        ui.end_row();

                        // Row 2: Bold | Text Shadow
                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(&mut self.style.font_bold, tr(lang, "Bold", "Bold"))
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(
                                    &mut self.style.text_shadow,
                                    tr(lang, "Text Shadow", "Text Shadow"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.end_row();

                        // Row 3: Text Color | Background Color
                        ui.horizontal(|ui| {
                            ui.label(self.t("Text Color", "Text Color"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                for hex in TEXT_SWATCHES.iter().rev() {
                                    let rgb = parse_hex_rgb(hex, [255, 255, 255]);
                                    let c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                                    let (rect, resp) =
                                        ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                                    ui.painter().rect_filled(rect, Rounding::same(2.0), c);
                                    ui.painter().rect_stroke(
                                        rect,
                                        Rounding::same(2.0),
                                        Stroke::new(1.0_f32, pal.border),
                                    );
                                    if resp.clicked() {
                                        self.style.text_color = (*hex).to_string();
                                        changed = true;
                                    }
                                }
                                let mut rgb =
                                    parse_hex_rgb(&self.style.text_color, [255, 255, 255]);
                                if ui.color_edit_button_srgb(&mut rgb).changed() {
                                    self.style.text_color = rgb_to_hex(rgb);
                                    changed = true;
                                }
                                ui.label(
                                    RichText::new(&self.style.text_color)
                                        .font(FontId::new(11.5, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label(self.t("Background Color", "Background Color"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                for hex in BG_SWATCHES.iter().rev() {
                                    let rgb = parse_hex_rgb(hex, [10, 10, 12]);
                                    let c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                                    let (rect, resp) =
                                        ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                                    ui.painter().rect_filled(rect, Rounding::same(2.0), c);
                                    ui.painter().rect_stroke(
                                        rect,
                                        Rounding::same(2.0),
                                        Stroke::new(1.0_f32, pal.border),
                                    );
                                    if resp.clicked() {
                                        self.style.bg_color = (*hex).to_string();
                                        changed = true;
                                    }
                                }
                                let mut rgb = parse_hex_rgb(&self.style.bg_color, [10, 10, 12]);
                                if ui.color_edit_button_srgb(&mut rgb).changed() {
                                    self.style.bg_color = rgb_to_hex(rgb);
                                    changed = true;
                                }
                                ui.label(
                                    RichText::new(&self.style.bg_color)
                                        .font(FontId::new(11.5, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                            });
                        });
                        ui.end_row();

                        // Row 4: Opacity | Border Radius
                        ui.horizontal(|ui| {
                            ui.label(self.t("Opacity", "Opacity"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let pct = ((self.style.opacity as f32 / 255.0) * 100.0).round()
                                    as i32;
                                ui.label(
                                    RichText::new(format!("{}%", pct))
                                        .font(FontId::new(12.0, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.style.opacity, 40..=255)
                                            .show_value(false),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label(self.t("Border Radius", "Border Radius"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{} px", self.style.corner_radius))
                                        .font(FontId::new(12.0, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.style.corner_radius, 0..=32)
                                            .show_value(false),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                        });
                        ui.end_row();

                        // Row 5: Padding X | Padding Y
                        ui.horizontal(|ui| {
                            ui.label(self.t("Padding X", "Padding X"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{} px", self.style.padding_x))
                                        .font(FontId::new(12.0, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.style.padding_x, 4..=60)
                                            .show_value(false),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label(self.t("Padding Y", "Padding Y"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{} px", self.style.padding_y))
                                        .font(FontId::new(12.0, FontFamily::Monospace))
                                        .color(pal.text_muted),
                                );
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.style.padding_y, 2..=40)
                                            .show_value(false),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                        });
                        ui.end_row();

                        // Row 6: Auto-Fit Container
                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(
                                    &mut self.style.fit_to_text,
                                    tr(lang, "Auto-Fit Container", "Auto-Fit Container"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        ui.label("");
                        ui.end_row();
                    });
            });

        if changed {
            self.handle.apply_style(self.style.clone(), true);
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
                ui.horizontal(|ui| {
                    let btn = egui::Button::new(
                        RichText::new(self.t(
                            "Save & Reload Engine",
                            "Kaydet ve Motoru Yenile",
                        ))
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

                if cfg.profiles.is_empty() {
                    ui.label(
                        RichText::new(self.t(
                            "No app profiles saved yet. Use Ctrl+Shift+S while focused on an app.",
                            "Henuz kayitli profil yok. Bir uygulama odakliyken Ctrl+Shift+S kullanin.",
                        ))
                        .size(12.0)
                        .color(pal.text_muted),
                    );
                } else {
                    let mut entries: Vec<(String, crate::config::Region)> =
                        cfg.profiles.into_iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(&b.0));

                    egui::Grid::new("profiles_table")
                        .num_columns(3)
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
                            ui.label("");
                            ui.end_row();

                            for (app, reg) in entries {
                                ui.label(
                                    RichText::new(&app)
                                        .font(FontId::new(12.0, FontFamily::Monospace)),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{}x{} @ {},{}",
                                        reg.width, reg.height, reg.left, reg.top
                                    ))
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
                        [egui::pos2(r.left() + 1.5, r.top() + 2.0), egui::pos2(r.left() + 1.5, r.bottom() - 2.0)],
                        Stroke::new(3.0_f32, pal.status_red),
                    );
                }
            });
    }

    fn render_live_log_tab(&mut self, ui: &mut egui::Ui, pal: Palette) {
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
                                RichText::new(self.t(
                                    "No translations recorded yet.",
                                    "Henuz ceviri kaydi yok.",
                                ))
                                .font(FontId::new(12.0, FontFamily::Monospace))
                                .color(pal.text_muted),
                            );
                        } else {
                            for entry in logs.iter().rev() {
                                ui.label(
                                    RichText::new(format!(
                                        "{} [{} ms | OCR {} ms | {}] {} -> {}",
                                        entry.timestamp,
                                        entry.total_ms,
                                        entry.ocr_ms,
                                        entry.engine,
                                        entry.src,
                                        entry.dst
                                    ))
                                    .font(FontId::new(11.5, FontFamily::Monospace))
                                    .color(pal.text_primary),
                                );
                            }
                        }
                    });
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

        if self
            .handle
            .gui_open_requested
            .swap(false, Ordering::SeqCst)
        {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        self.sync_from_handle_if_needed();

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
                        "Ctrl+Shift+  S: Region  |  H: Pause  |  Up/Down: Size  |  A: Window  |  Q: Quit",
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
                    });
            });

        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

pub fn run_gui(handle: OverlayHandle, start_hidden: bool) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Overcap")
            .with_inner_size([760.0, 620.0])
            .with_min_inner_size([680.0, 540.0])
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
