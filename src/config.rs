use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Region {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

/// Largest supported text outline width in pixels.
pub const MAX_OUTLINE_WIDTH: i32 = 6;

/// Where the translated subtitle overlay is placed relative to the captured subtitle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    #[default]
    Cover,
    Custom,
}

/// Everything that controls how the translated subtitle looks. Used live by the overlay,
/// and stored per app in `AppConfig::profile_styles`. Missing fields fall back to defaults,
/// so hand-edited or older entries keep loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayStyle {
    pub font_family: String,
    pub font_size: i32,
    pub font_bold: bool,
    pub text_color: String,
    pub bg_color: String,
    /// Opacity of the background box only (0 = invisible, 255 = solid). Text is always opaque.
    pub opacity: u8,
    pub corner_radius: i32,
    pub text_shadow: bool,
    pub fit_to_text: bool,
    pub padding_x: i32,
    pub padding_y: i32,
    pub outline_width: i32,
    pub outline_color: String,
    #[serde(default)]
    pub output_mode: OutputMode,
    #[serde(default)]
    pub output_region: Option<Region>,
    #[serde(default)]
    pub offset_x: i32,
    #[serde(default)]
    pub offset_y: i32,
}

impl Default for OverlayStyle {
    fn default() -> Self {
        Self::from_config(&AppConfig::default())
    }
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
            outline_width: cfg.outline_width,
            outline_color: cfg.outline_color.clone(),
            output_mode: cfg.output_mode,
            output_region: cfg.output_region,
            offset_x: cfg.offset_x,
            offset_y: cfg.offset_y,
        }
    }

    /// Writes this style into the global (non per-app) fields of `cfg`.
    pub fn write_to_config(&self, cfg: &mut AppConfig) {
        cfg.font_family = self.font_family.clone();
        cfg.font_size = self.font_size;
        cfg.font_bold = self.font_bold;
        cfg.text_color = self.text_color.clone();
        cfg.bg_color = self.bg_color.clone();
        cfg.opacity = self.opacity;
        cfg.corner_radius = self.corner_radius;
        cfg.text_shadow = self.text_shadow;
        cfg.fit_to_text = self.fit_to_text;
        cfg.padding_x = self.padding_x;
        cfg.padding_y = self.padding_y;
        cfg.outline_width = self.outline_width;
        cfg.outline_color = self.outline_color.clone();
        cfg.output_mode = self.output_mode;
        cfg.output_region = self.output_region;
        cfg.offset_x = self.offset_x;
        cfg.offset_y = self.offset_y;
    }
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
    #[serde(default = "default_outline_width")]
    pub outline_width: i32,
    #[serde(default = "default_outline_color")]
    pub outline_color: String,
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
    #[serde(default = "default_custom_base_url")]
    pub custom_base_url: String,
    #[serde(default)]
    pub custom_api_key: Option<String>,
    #[serde(default = "default_custom_model")]
    pub custom_model: String,
    #[serde(default = "default_auto_profiles")]
    pub auto_profiles: bool,
    #[serde(default)]
    pub profiles: HashMap<String, Region>,
    /// Per-app subtitle look, keyed like `profiles` (lowercase `.exe` name).
    #[serde(default)]
    pub profile_styles: HashMap<String, OverlayStyle>,
    /// Capture the target window via WGC and allow the overlay to appear in Discord/OBS screen share.
    #[serde(default)]
    pub stream_mode: bool,
    #[serde(default)]
    pub output_mode: OutputMode,
    #[serde(default)]
    pub output_region: Option<Region>,
    #[serde(default)]
    pub offset_x: i32,
    #[serde(default)]
    pub offset_y: i32,
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

fn default_outline_width() -> i32 {
    0
}

fn default_outline_color() -> String {
    "#000000".to_string()
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

fn default_custom_base_url() -> String {
    "http://127.0.0.1:11434/v1".to_string()
}

fn default_custom_model() -> String {
    "qwen2.5:7b".to_string()
}

fn default_auto_profiles() -> bool {
    true
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
            outline_width: default_outline_width(),
            outline_color: default_outline_color(),
            source_lang: default_source_lang(),
            target_lang: default_target_lang(),
            engine: default_engine(),
            gemini_api_key: None,
            gemini_model: default_gemini_model(),
            groq_api_key: None,
            groq_model: default_groq_model(),
            custom_base_url: default_custom_base_url(),
            custom_api_key: None,
            custom_model: default_custom_model(),
            auto_profiles: default_auto_profiles(),
            profiles: HashMap::new(),
            profile_styles: HashMap::new(),
            stream_mode: false,
            output_mode: OutputMode::default(),
            output_region: None,
            offset_x: 0,
            offset_y: 0,
        }
    }
}

impl AppConfig {
    /// Resolves `config.json` for portable usage:
    /// 1. Prefer `config.json` next to `overcap.exe` if it exists.
    /// 2. Fall back to `config.json` in the current working directory if it exists (dev/launcher compatibility).
    /// 3. Otherwise default to the executable's directory so portable installs stay self-contained.
    pub fn path() -> PathBuf {
        Self::resolve_paths().0
    }

    /// A second `config.json` that exists but is NOT being used (it lives in the working
    /// directory while the one next to the exe wins). Settings and API keys in it are silently
    /// ignored, so the caller should tell the user.
    pub fn ignored_path() -> Option<PathBuf> {
        Self::resolve_paths().1
    }

    /// Returns `(used, ignored)`, see `path` and `ignored_path`.
    fn resolve_paths() -> (PathBuf, Option<PathBuf>) {
        let exe_cfg = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("config.json")));
        let cwd_cfg = std::env::current_dir()
            .map(|d| d.join("config.json"))
            .unwrap_or_else(|_| PathBuf::from("config.json"));
        pick_config(exe_cfg, cwd_cfg)
    }

    /// The style to use while `app` is in the foreground: its own style if per-app switching
    /// is enabled and one exists, otherwise the global style.
    pub fn style_for_app(&self, app: Option<&str>) -> OverlayStyle {
        if self.auto_profiles {
            if let Some(style) = app.and_then(|a| self.profile_styles.get(a)) {
                return style.clone();
            }
        }
        OverlayStyle::from_config(self)
    }

    /// Loads the default config path and returns a user-facing warning if the file had to be
    /// discarded (see `load_checked`). The caller is expected to show the warning in the GUI.
    pub fn load_with_notice() -> (Self, Option<String>) {
        Self::load_checked(&Self::path())
    }

    #[cfg(test)]
    pub fn load_from(path: &Path) -> Self {
        Self::load_checked(path).0
    }

    /// Loads the config from `path`.
    /// A missing file silently yields defaults (first run). A file that exists but cannot be
    /// read or parsed is copied to `<path>.bak` first, so a later `save()` of the default
    /// config can never destroy the user's API keys. In that case a warning is returned.
    fn load_checked(path: &Path) -> (Self, Option<String>) {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                if path.exists() {
                    eprintln!("[Config ERROR] Cannot read {}: {e}", path.display());
                    let notice = Self::discard_notice(path, &e.to_string());
                    return (Self::default(), Some(notice));
                }
                return (Self::default(), None);
            }
        };

        // Windows editors may prepend a UTF-8 BOM, which serde_json rejects.
        match serde_json::from_str::<AppConfig>(content.trim_start_matches('\u{feff}')) {
            Ok(cfg) => (cfg, None),
            Err(e) => {
                eprintln!("[Config ERROR] Cannot parse {}: {e}", path.display());
                let notice = Self::discard_notice(path, &e.to_string());
                (Self::default(), Some(notice))
            }
        }
    }

    /// Backs the unreadable file up and describes what happened.
    fn discard_notice(path: &Path, reason: &str) -> String {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.json".to_string());
        let bak = sibling_with_suffix(path, ".bak");
        match fs::copy(path, &bak) {
            Ok(_) => {
                eprintln!(
                    "[Config] Backed up the unreadable config to {}.",
                    bak.display()
                );
                format!(
                    "{name} could not be loaded ({reason}). Settings were reset to defaults; \
                     the original file, including API keys, was saved as {name}.bak."
                )
            }
            Err(e) => {
                eprintln!("[Config ERROR] Could not back up to {}: {e}", bak.display());
                format!(
                    "{name} could not be loaded ({reason}) and the backup to {name}.bak failed \
                     ({e}). Settings were reset to defaults."
                )
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    /// Writes to `<path>.tmp` first and then renames over `path`, so a crash mid-write
    /// leaves the previous config intact instead of a truncated file.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let tmp = sibling_with_suffix(path, ".tmp");
        fs::write(&tmp, json)?;
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Chooses the config file: the one next to the exe if it exists, else the one in the working
/// directory if it exists, else a new one next to the exe. The second value is a config file
/// that exists but loses to the first.
fn pick_config(exe_cfg: Option<PathBuf>, cwd_cfg: PathBuf) -> (PathBuf, Option<PathBuf>) {
    if let Some(used) = exe_cfg.as_ref().filter(|p| p.exists()) {
        let ignored = (cwd_cfg.exists() && !same_file(used, &cwd_cfg)).then_some(cwd_cfg);
        return (used.clone(), ignored);
    }
    if cwd_cfg.exists() {
        return (cwd_cfg, None);
    }
    (exe_cfg.unwrap_or(cwd_cfg), None)
}

/// `config.json` + `.bak` -> `config.json.bak` (appends instead of replacing the extension).
fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("overcap-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_yields_defaults_without_backup() {
        let dir = temp_dir("missing");
        let path = dir.join("config.json");
        let cfg = AppConfig::load_from(&path);
        assert_eq!(cfg.engine, default_engine());
        assert!(!dir.join("config.json.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_backed_up_before_defaults() {
        let dir = temp_dir("corrupt");
        let path = dir.join("config.json");
        let broken = r#"{ "groq_api_key": "gsk_secret" "engine": "groq" }"#;
        fs::write(&path, broken).unwrap();

        let cfg = AppConfig::load_from(&path);
        assert!(cfg.groq_api_key.is_none());
        assert_eq!(
            fs::read_to_string(dir.join("config.json.bak")).unwrap(),
            broken
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_roundtrip_leaves_no_tmp_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("config.json");
        let mut cfg = AppConfig::default();
        cfg.groq_api_key = Some("gsk_test".to_string());
        cfg.save_to(&path).unwrap();
        // Overwriting an existing file must also work (rename replaces the target).
        cfg.font_size = 40;
        cfg.save_to(&path).unwrap();

        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.groq_api_key.as_deref(), Some("gsk_test"));
        assert_eq!(loaded.font_size, 40);
        assert!(!dir.join("config.json.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn notice_is_returned_only_when_a_file_was_discarded() {
        let dir = temp_dir("notice");
        let path = dir.join("config.json");

        let (_, notice) = AppConfig::load_checked(&path);
        assert!(notice.is_none(), "missing file is a normal first run");

        fs::write(&path, "{ not json").unwrap();
        let (_, notice) = AppConfig::load_checked(&path);
        let notice = notice.expect("corrupt file must produce a notice");
        assert!(notice.contains("config.json.bak"), "{notice}");

        fs::write(&path, "{\"font_size\": 30}").unwrap();
        let (cfg, notice) = AppConfig::load_checked(&path);
        assert_eq!(cfg.font_size, 30);
        assert!(notice.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_config_without_new_style_fields_still_loads() {
        let dir = temp_dir("oldcfg");
        let path = dir.join("config.json");
        // A v0.4.0 era file: no outline_*, no profile_styles.
        fs::write(
            &path,
            r##"{"font_size": 31, "opacity": 200, "profiles": {}}"##,
        )
        .unwrap();
        let (cfg, notice) = AppConfig::load_checked(&path);
        assert!(notice.is_none());
        assert_eq!(cfg.font_size, 31);
        assert_eq!(cfg.outline_width, 0);
        assert_eq!(cfg.outline_color, "#000000");
        assert!(cfg.profile_styles.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_app_style_entry_fills_in_defaults() {
        let json = r##"{"profile_styles": {"game.exe": {"font_size": 40, "outline_width": 3}}}"##;
        let cfg: AppConfig = serde_json::from_str(json).unwrap();
        let style = &cfg.profile_styles["game.exe"];
        assert_eq!(style.font_size, 40);
        assert_eq!(style.outline_width, 3);
        assert_eq!(style.text_color, default_text_color());
        assert_eq!(style.opacity, default_opacity());
    }

    #[test]
    fn style_for_app_prefers_app_style_only_when_auto_profiles_is_on() {
        let mut cfg = AppConfig::default();
        let custom = OverlayStyle {
            font_size: 44,
            ..OverlayStyle::default()
        };
        cfg.profile_styles
            .insert("game.exe".to_string(), custom.clone());

        assert_eq!(cfg.style_for_app(Some("game.exe")), custom);
        assert_eq!(
            cfg.style_for_app(Some("other.exe")).font_size,
            cfg.font_size
        );
        assert_eq!(cfg.style_for_app(None).font_size, cfg.font_size);

        cfg.auto_profiles = false;
        assert_eq!(cfg.style_for_app(Some("game.exe")).font_size, cfg.font_size);
    }

    #[test]
    fn app_styles_survive_a_save_load_roundtrip() {
        let dir = temp_dir("appstyle");
        let path = dir.join("config.json");
        let mut cfg = AppConfig::default();
        cfg.profile_styles.insert(
            "game.exe".to_string(),
            OverlayStyle {
                outline_width: 4,
                outline_color: "#FF0000".to_string(),
                ..OverlayStyle::default()
            },
        );
        cfg.save_to(&path).unwrap();
        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.profile_styles["game.exe"].outline_width, 4);
        assert_eq!(loaded.profile_styles["game.exe"].outline_color, "#FF0000");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_to_config_updates_only_global_style_fields() {
        let mut cfg = AppConfig::default();
        let style = OverlayStyle {
            font_size: 50,
            outline_width: 2,
            ..OverlayStyle::default()
        };
        style.write_to_config(&mut cfg);
        assert_eq!(cfg.font_size, 50);
        assert_eq!(cfg.outline_width, 2);
        assert_eq!(OverlayStyle::from_config(&cfg), style);
    }

    #[test]
    fn pick_config_reports_the_config_it_ignores() {
        let dir = temp_dir("pick");
        let exe_dir = dir.join("exe");
        let cwd_dir = dir.join("cwd");
        fs::create_dir_all(&exe_dir).unwrap();
        fs::create_dir_all(&cwd_dir).unwrap();
        let exe_cfg = exe_dir.join("config.json");
        let cwd_cfg = cwd_dir.join("config.json");

        // Neither exists: a new file next to the exe, nothing ignored.
        assert_eq!(
            pick_config(Some(exe_cfg.clone()), cwd_cfg.clone()),
            (exe_cfg.clone(), None)
        );

        // Only the working-directory one exists: use it.
        fs::write(&cwd_cfg, "{}").unwrap();
        assert_eq!(
            pick_config(Some(exe_cfg.clone()), cwd_cfg.clone()),
            (cwd_cfg.clone(), None)
        );

        // Both exist: the exe one wins and the other is reported.
        fs::write(&exe_cfg, "{}").unwrap();
        assert_eq!(
            pick_config(Some(exe_cfg.clone()), cwd_cfg.clone()),
            (exe_cfg.clone(), Some(cwd_cfg.clone()))
        );

        // Same file reached twice (cwd == exe dir): not a duplicate.
        assert_eq!(
            pick_config(Some(exe_cfg.clone()), exe_cfg.clone()),
            (exe_cfg, None)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn utf8_bom_is_tolerated() {
        let dir = temp_dir("bom");
        let path = dir.join("config.json");
        fs::write(&path, "\u{feff}{\"font_size\": 33}").unwrap();
        let cfg = AppConfig::load_from(&path);
        assert_eq!(cfg.font_size, 33);
        assert!(!dir.join("config.json.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn placement_fields_are_backward_compatible_and_roundtrip() {
        let dir = temp_dir("placement");
        let path = dir.join("config.json");

        // Old config without output_mode, output_region, offset_x, offset_y.
        fs::write(
            &path,
            r##"{"font_size": 28, "profile_styles": {"game.exe": {"font_size": 34}}}"##,
        )
        .unwrap();
        let (old_cfg, notice) = AppConfig::load_checked(&path);
        assert!(notice.is_none());
        assert_eq!(old_cfg.output_mode, OutputMode::Cover);
        assert!(old_cfg.output_region.is_none());
        assert_eq!(old_cfg.offset_x, 0);
        assert_eq!(old_cfg.offset_y, 0);
        let old_app_style = &old_cfg.profile_styles["game.exe"];
        assert_eq!(old_app_style.output_mode, OutputMode::Cover);
        assert!(old_app_style.output_region.is_none());
        assert_eq!(old_app_style.offset_x, 0);
        assert_eq!(old_app_style.offset_y, 0);

        // Roundtrip with Custom output_region and offsets (global + per-app).
        let custom_region = Region {
            left: 480,
            top: 820,
            width: 960,
            height: 110,
        };
        let app_region = Region {
            left: 120,
            top: 700,
            width: 800,
            height: 90,
        };
        let mut cfg = AppConfig::default();
        cfg.output_mode = OutputMode::Custom;
        cfg.output_region = Some(custom_region);
        cfg.offset_x = -24;
        cfg.offset_y = 36;
        cfg.profile_styles.insert(
            "player.exe".to_string(),
            OverlayStyle {
                output_mode: OutputMode::Custom,
                output_region: Some(app_region),
                offset_x: 15,
                offset_y: -40,
                ..OverlayStyle::default()
            },
        );
        cfg.save_to(&path).unwrap();

        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.output_mode, OutputMode::Custom);
        assert_eq!(loaded.output_region, Some(custom_region));
        assert_eq!(loaded.offset_x, -24);
        assert_eq!(loaded.offset_y, 36);
        let loaded_app = &loaded.profile_styles["player.exe"];
        assert_eq!(loaded_app.output_mode, OutputMode::Custom);
        assert_eq!(loaded_app.output_region, Some(app_region));
        assert_eq!(loaded_app.offset_x, 15);
        assert_eq!(loaded_app.offset_y, -40);
        let _ = fs::remove_dir_all(&dir);
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
