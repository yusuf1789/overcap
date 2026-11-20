# AGENTS.md

Architectural guide and rules for contributors and coding agents working on the `overcap` repository.

## Project Summary

Overcap is a low-latency, game-independent live subtitle translator overlay for Windows 10/11 written in Rust.
It captures a user-selected screen rectangle via GDI `BitBlt`, extracts text and word bounding boxes (`BoundingRect`) with `Windows.Media.Ocr`, translates the text via a configured translation backend, and renders the result directly over the original subtitle using a `WDA_EXCLUDEFROMCAPTURE` Win32 overlay window.

## Module Map

- `src/main.rs` — CLI (`clap`), DPI awareness setup, `ocr_worker` and `translation_worker` (latest-only mailbox) coordination.
- `src/capture.rs` — GDI `BitBlt` screen capture, bright-pixel change mask (`changed_pixel_ratio`), and 2x bilinear upscale (`upscale_for_ocr`).
- `src/ocr.rs` — `Windows.Media.Ocr` (`WinOcr`) wrapper, Y/X line ordering, and screen-pixel `SourceBounds` calculation.
- `src/translator.rs` — `Translator` trait, `GroqTranslator`, `GeminiTranslator`, `GoogleTranslator`, translation cache, and Levenshtein + word-overlap deduplication (`is_same_subtitle`).
- `src/overlay.rs` — Win32 layered/click-through overlay window, `WDA_EXCLUDEFROMCAPTURE`, `CreateRoundRectRgn`, dynamic `Auto-Fit` + `SourceBounds` coverage, and global hotkeys.
- `src/selector.rs` — Multi-monitor-aware fullscreen Win32 region selector.
- `src/settings_ui.rs` — Local HTTP settings and live-preview server (`std::net::TcpListener` on `127.0.0.1:8787`) with embedded HTML/CSS/JS UI.
- `src/config.rs` — `AppConfig` serialization (`config.json`), backward-compatible `#[serde(default)]` fields, and hex-to-COLORREF parser.

## Architectural Principles & Constraints

1. **Game-Independent Design:** Never add fixed coordinates, game-specific hardcoded rules, or single-game dependencies.
2. **Zero Self-Capture Loop:** `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` must always be preserved on the overlay window; otherwise OCR reads its own translated overlay in an infinite loop.
3. **Zero Focus Stealing & Click-Through:** The overlay must preserve `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, and use `SWP_NOACTIVATE` for visibility/position updates.
4. **Minimal Dependencies & Small Binary:** Prefer native Windows APIs and the Rust standard library over heavy GUI frameworks or unnecessary crates.
5. **Adding Translation Engines:** Every backend must implement the `Translator` trait in `src/translator.rs`, fall back to `GoogleTranslator` on failure, and be configurable via `config.rs`, `main.rs`, and `settings_ui.rs`.
6. **Backward-Compatible `config.json`:** Every new field added to `AppConfig` must carry `#[serde(default = "...")]` so existing user `config.json` files continue to load cleanly.
7. **Settings UI Design Rule (`settings_ui.rs`):** Neutral-gray palette, clean desktop utility layout, zero emojis, and clear technical labels (`Border Radius`, `Opacity`, `Padding X/Y`, `Auto-Fit Container`).
8. **Security:** `config.json` contains user API keys and must remain in `.gitignore`; never commit or expose it. The settings server must bind strictly to `127.0.0.1`.

## Build and Verification

```powershell
cargo build --release
```

Note: On Windows, compiling while `target\release\overcap.exe` is running fails with `os error 5` (file lock). Close any running instance with `Ctrl + Shift + Q` before rebuilding.
