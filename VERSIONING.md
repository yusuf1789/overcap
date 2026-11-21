# Versioning

Overcap follows [Semantic Versioning 2.0.0](https://semver.org/) (`MAJOR.MINOR.PATCH`).

## Version Policy

- **MAJOR (`X.0.0`)**: Backward-incompatible changes to the `config.json` schema, breaking architectural changes, or breaking CLI flag changes.
- **MINOR (`0.X.0`)**: Backward-compatible new features (new translation backend, new overlay capability, new settings UI controls).
- **PATCH (`0.0.X`)**: Bug fixes, OCR/capture performance improvements, and documentation updates.

**Current practice:** the project stays on `0.4.x` for now. New features and fixes bump `PATCH` (`0.4.1`, `0.4.2`, ...) until the maintainer decides to move to `0.5.0`.

The canonical version number is maintained in `package.version` inside [Cargo.toml](Cargo.toml).

---

## Changelog

### `v0.4.5` — Stream Mode (Windows Graphics Capture)
- Added optional **Stream Mode (Discord / OBS)** in `Profiles & OCR` (`stream_mode` in `config.json`, default `false`).
- Implemented GPU window capture in `src/wgc.rs` using Windows Graphics Capture (`GraphicsCaptureItem::CreateForWindow`, `Direct3D11CaptureFramePool::CreateFreeThreaded`, `IsBorderRequired(false)` on Windows 11) so only the target game/app window is captured.
- When Stream Mode is enabled, the overlay switches its display affinity from `WDA_EXCLUDEFROMCAPTURE` to `WDA_NONE` live without restarting, allowing Discord/OBS screen share (Screen capture) to display translated subtitles without causing OCR self-capture loops.

### `v0.4.4` — In-Game Translation History
- `Ctrl+Shift+L`, son 15 hedef çeviriyi gösteren, oyundan odak almayan yerel geçmiş panelini açar/kapatır. Live Log ekranında da aç/kapat düğmesi bulunur.
- Geçmiş eskiden yeniye sıralanır; fare tekerleği veya panel açıkken `Ctrl+Shift+PageUp/PageDown` ile kaydırılır. Yeni sonuçlar gelirken eski replikleri okuma konumu korunur.
- Ayrı Win32 katmanlı pencere `WDA_EXCLUDEFROMCAPTURE` ile korunur. Çeviri/OCR çalışmaya devam eder; geçmiş diske yazılmaz ve Live Log temizlendiğinde silinir.
- Türkçe/İngilizce arayüz metinleri, monitör çalışma alanına uyum ve DPI ölçeklemesi eklenmiştir. Pencere kapatıldığında sayfa kısayolları serbest bırakılır.

### `v0.4.3` — Actual Engine Status & Latency Diagnostics
- Seçilen motor, başarıyla başlatılan motor ve son çeviriyi üreten sağlayıcı ayrı gösterilir; Google yedeği ve önbellek sonuçları doğru etiketlenir.
- Motor yenileme durumu işçiden bildirilir; önceki nesle ait devam eden çeviriler yeni motorun sonucu olarak yayımlanmaz. Kaydetme başarısızsa girilen API anahtarı korunur.
- Engine ekranında kullanılan ve varsa kullanılmayan ayar dosyalarının yolları gösterilir; anahtar rozetleri ortam değişkenlerini de tanır.
- Live Log yakalama/OCR, bekleme ve çeviri sürelerini; gerçek model, adres denemeleri ve ana/yedek süreleriyle birlikte kaydeder. Toplam ölçümünün sınırları açıklanır.
- Varsayılan olarak kapalı yerel OCR teşhisi, en fazla 60 BMP/JSON çiftini sınırlı bir arka plan kuyruğuyla saklar. Çeviri promptu, bağlamı ve tekrar eşikleri korunur.
- Yerel HTTP testleri ana/yedek motor geçişlerini, önbelleği ve hata raporlamasını; ek testler yenileme nesillerini ve görüntü kaydı sınırını doğrular.

### `v0.4.2` — Config Path & Region Save Diagnostics
- A warning banner now appears when two `config.json` files exist (one next to the exe wins, the one in the working directory is ignored), naming both paths. Launching the exe directly with no config nearby creates a fresh default one, which then shadows a project-root config.
- Region reselection now reports selector failures and logs the exact config path used for saves; config save errors include that path instead of being silently ambiguous.

### `v0.4.1` — Subtitle Look: Background-Only Opacity, Outline & Per-App Styles
- **Behavior change:** `opacity` now applies to the background box only. The overlay is drawn with per-pixel alpha (`UpdateLayeredWindow`) instead of one window-wide alpha, so text is always fully opaque. Existing `opacity` values keep working but the text is now sharper than before at the same setting; the slider range is `0..=255` (was `40..=255`). Text is rendered with grayscale instead of ClearType antialiasing, which is required for transparency. Rounded corners are anti-aliased in the bitmap instead of a window region.
- Added a text outline (`outline_width` 0-6 px, `outline_color`); default width `0` keeps existing looks. The drop shadow now follows the outline shape. `Yellow` and `High Contrast` presets use a thin outline.
- Added per-app subtitle styles (`profile_styles` in `config.json`): the Appearance tab can edit the global style or the style of the last focused app (`Remove App Style` reverts it). The overlay switches styles with the foreground app when `Auto-Switch by App` is on, and the font-size hotkeys change the style in effect. The Profiles tab lists apps with a region and/or a custom style.
- New `src/render.rs` module with unit tests; `OverlayStyle` moved to `config.rs`. New config fields are backward compatible (`#[serde(default)]`).

### `v0.4.0` — In-App Notices (Banner & Events)
- Added a notice channel (`Notice` / `NoticeLevel`, `OverlayHandle::push_notice`) so problems are visible in release builds, which have no console. Identical consecutive notices are merged into one entry with a counter.
- GUI: dismissible banner under the top bar showing the newest notice, plus an `Events` section in the `Live Log` tab. The version label now comes from `Cargo.toml`.
- Reported as notices: reset/unreadable `config.json` (with `.bak` location), failed config saves, missing API key or failed engine start (silent Google fallback), per-line fallback to Google with the primary engine's error, translation/reload errors, capture/OCR errors, and failed OCR language switches.
- Fixed `Quit` (tray / `Ctrl + Shift + Q`) leaving an invisible `overcap.exe` running when the GUI window was hidden in the tray: the process now force-exits 1 s after the overlay loop ends.
- `Translator` trait gained `take_notice()` (default `None`); Groq, Gemini, and OpenAI-compatible translators use it to report why the fallback was used. `build_translator` now returns an optional warning next to the engine.

### `v0.3.1` — Config Safety & Fallback Cache Fix
- `AppConfig::load` no longer silently discards an unreadable `config.json`: the file is copied to `config.json.bak` before defaults are used, so API keys survive a typo. A leading UTF-8 BOM is now tolerated.
- `AppConfig::save` writes to `config.json.tmp` and renames over the target, so a crash mid-write cannot truncate the config.
- Groq, Gemini, and OpenAI-compatible translators no longer cache or record in the context history translations that came from the Google fallback.

### `v0.3.0` — Native Rust `egui` Desktop App, System Tray & Consoleless Mode
- Replaced the local HTTP/browser settings server (`settings_ui.rs`) with a native `eframe` / `egui` desktop control center (`src/gui.rs`) featuring a Neutral Dark Graphite default theme, Light gray toggle, `EN`/`TR` language toggle, live subtitle preview strip, and 4 tabs (`Appearance`, `Engine`, `Profiles & OCR`, `Live Log`).
- Added native Windows System Tray integration (`src/tray.rs`) with a custom geometric icon, right-click menu (`Open Overcap`, `Select Region`, `Pause`/`Resume`, `Quit`), and minimize-to-tray on window close.
- Enabled consoleless release builds (`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`) and added `--minimized` CLI flag to start hidden in the System Tray.
- Added live engine reloading (`Save & Reload Engine`) in `translation_worker` / `ocr_worker` and real-time translation log entries (`LogEntry`) in `OverlayHandle`.

### `v0.2.0` — OpenAI-Compatible Endpoints, Auto App Profiles & OCR Diagnostics
- Added `OpenAiCompatTranslator` (`--engine openai-compat`, also accepts `custom` / `ollama` aliases) for local LLMs (Ollama, LM Studio, vLLM) and custom OpenAI-compatible `/chat/completions` endpoints via `--custom-base-url`, `--custom-model`, and `--custom-key`.
- Added automatic per-app subtitle region profiles (`src/profiles.rs`): `Ctrl + Shift + S` binds the selected region to the active foreground `.exe`, and `ocr_worker` automatically switches regions when switching between apps/games without stealing focus (disable with `--no-auto-profiles` or in Settings UI).
- Added Windows OCR language diagnostics (`available_ocr_languages`, `is_language_installed`) to the Settings UI with a missing-language warning and a direct `Open Windows Language Settings` action (`ms-settings:regionlanguage`).
- Extended `AppConfig` (`config.json`) with backward-compatible defaults (`custom_base_url`, `custom_api_key`, `custom_model`, `auto_profiles`, `profiles`).

### `v0.1.0` — Initial Release
- Single-binary Rust + Win32 architecture (`overcap.exe`).
- `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` and click-through (`WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`) overlay window.
- Per-Monitor DPI V2 and multi-monitor Win32 region selector (`Ctrl + Shift + S`).
- Built-in `Windows.Media.Ocr` integration with 2x bilinear upscale, Y/X line ordering, and `BoundingRect`-based Auto-Fit coverage.
- Bright-pixel change detection mask and Levenshtein + word-overlap duplicate subtitle filter.
- Asynchronous translation pipeline and `Translator` trait:
  - `GroqTranslator` (`qwen/qwen3.8-27b`, `reasoning_effort: "none"`)
  - `GeminiTranslator` (`gemini-3.1-flash-lite`)
  - `GoogleTranslator` (multi-endpoint fallback, keyless default and automatic backup)
- Local live settings and preview UI (`127.0.0.1:8787`, `Ctrl + Shift + A`) with automatic `config.json` persistence.
- Global in-game hotkeys (`Ctrl + Shift + A/S/H/Up/Down/Q`).
