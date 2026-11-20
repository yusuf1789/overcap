use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Region {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub region: Option<Region>,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: i32,
    #[serde(default = "default_font_bold")]
    pub font_bold: bool,
    #[serde(default = "default_text_color")]
    pub text_color: String,
    #[serde(default = "default_bg_color")]
    pub bg_color: String,
    #[serde(default = "default_opacity")]
    pub opacity: u8,
    #[serde(default = "default_corner_radius")]
    pub corner_radius: i32,
    #[serde(default = "default_text_shadow")]
    pub text_shadow: bool,
    #[serde(default = "default_fit_to_text")]
    pub fit_to_text: bool,
    #[serde(default = "default_padding_x")]
    pub padding_x: i32,
    #[serde(default = "default_padding_y")]
    pub padding_y: i32,
    #[serde(default = "default_source_lang")]
    pub source_lang: String,
    #[serde(default = "default_target_lang")]
    pub target_lang: String,
    #[serde(default = "default_engine")]
    pub engine: String,
    #[serde(default)]
    pub gemini_api_key: Option<String>,
    #[serde(default = "default_gemini_model")]
    pub gemini_model: String,
    #[serde(default)]
    pub groq_api_key: Option<String>,
    #[serde(default = "default_groq_model")]
    pub groq_model: String,
}

fn default_font_family() -> String {
    "Segoe UI".to_string()
}

fn default_font_size() -> i32 {
    28
}

fn default_font_bold() -> bool {
    true
}

fn default_text_color() -> String {
    "#FFFFFF".to_string()
}

fn default_bg_color() -> String {
    "#0A0A0C".to_string()
}

fn default_opacity() -> u8 {
    230
}

fn default_corner_radius() -> i32 {
    14
}

fn default_text_shadow() -> bool {
    true
}

fn default_fit_to_text() -> bool {
    true
}

fn default_padding_x() -> i32 {
    18
}

fn default_padding_y() -> i32 {
    10
}

fn default_source_lang() -> String {
    "en".to_string()
}

fn default_target_lang() -> String {
    "tr".to_string()
}

fn default_engine() -> String {
    "google".to_string()
}

fn default_gemini_model() -> String {
    "gemini-3.1-flash-lite".to_string()
}

fn default_groq_model() -> String {
    "qwen/qwen3.8-27b".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            region: None,
            font_family: default_font_family(),
            font_size: default_font_size(),
            font_bold: default_font_bold(),
            text_color: default_text_color(),
            bg_color: default_bg_color(),
            opacity: default_opacity(),
            corner_radius: default_corner_radius(),
            text_shadow: default_text_shadow(),
            fit_to_text: default_fit_to_text(),
            padding_x: default_padding_x(),
            padding_y: default_padding_y(),
            source_lang: default_source_lang(),
            target_lang: default_target_lang(),
            engine: default_engine(),
            gemini_api_key: None,
            gemini_model: default_gemini_model(),
            groq_api_key: None,
            groq_model: default_groq_model(),
        }
    }
}

impl AppConfig {
    pub fn path() -> PathBuf {
        PathBuf::from("config.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<AppConfig>(&content) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(Self::path(), json)?;
        Ok(())
    }
}

/// Parses a `#RRGGBB` hex string into a Win32 COLORREF (`0x00BBGGRR`).
pub fn parse_hex_colorref(hex: &str, fallback: u32) -> u32 {
    let cleaned = hex.trim().trim_start_matches('#');
    if cleaned.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&cleaned[0..2], 16),
            u8::from_str_radix(&cleaned[2..4], 16),
            u8::from_str_radix(&cleaned[4..6], 16),
        ) {
            return (r as u32) | ((g as u32) << 8) | ((b as u32) << 16);
        }
    }
    fallback
}
