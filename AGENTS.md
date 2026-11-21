# AGENTS.md

Architectural guide and rules for contributors and coding agents working on the `overcap` repository.

## Project Summary

Overcap is a low-latency, game-independent live subtitle translator overlay for Windows 10/11 written in Rust.
It captures a user-selected screen rectangle via GDI `BitBlt`, extracts text and word bounding boxes (`BoundingRect`) with `Windows.Media.Ocr`, translates the text via a configured translation backend, and renders the result directly over the original subtitle using a `WDA_EXCLUDEFROMCAPTURE` Win32 overlay window.

## Module Map

- `src/main.rs` — CLI (`clap`), consoleless release subsystem (`windows_subsystem = "windows"`), DPI awareness setup, `ocr_worker` and `translation_worker` (latest-only mailbox with live engine reload) coordination, and `egui` main-thread event loop.
- `src/capture.rs` — GDI `BitBlt` screen capture, bright-pixel change mask (`changed_pixel_ratio`), and 2x bilinear upscale (`upscale_for_ocr`).
- `src/ocr.rs` — `Windows.Media.Ocr` (`WinOcr`) wrapper, Y/X line ordering, and screen-pixel `SourceBounds` calculation.
- `src/translator.rs` — `Translator` trait, `GroqTranslator`, `GeminiTranslator`, `OpenAiCompatTranslator`, `GoogleTranslator`, translation cache, and Levenshtein + word-overlap deduplication (`is_same_subtitle`).
- `src/render.rs` — Software compositing of the overlay bitmap (premultiplied BGRA): background box with its own opacity and anti-aliased rounded corners, shadow, text outline, and opaque text from a GDI grayscale coverage mask; `UpdateLayeredWindow` presenter. Pure parts are unit-tested.
- `src/overlay.rs` — Win32 layered/click-through overlay window, `WDA_EXCLUDEFROMCAPTURE`, per-app style switching, dynamic `Auto-Fit` + `SourceBounds` coverage, global hotkeys, tray message dispatch, and the user-visible `Notice` channel (`push_notice`; release builds have no console, so report errors there instead of only `eprintln!`).
- `src/selector.rs` — Multi-monitor-aware fullscreen Win32 region selector.
- `src/gui.rs` — Native `eframe`/`egui` desktop control center (Appearance, Engine, Profiles & OCR, Live Log), live preview strip, notice banner + `Events` list, `EN`/`TR` toggle, Dark/Light neutral themes, and minimize-to-tray lifecycle.
- `src/tray.rs` — Native Win32 System Tray icon (`Shell_NotifyIconW`), procedural RGBA/HICON generator, and right-click context menu.
- `src/profiles.rs` — Foreground process `.exe` detection for automatic per-app subtitle region profiles.
- `src/config.rs` — `AppConfig` and `OverlayStyle` (global style + per-app `profile_styles`) serialization (`config.json`), backward-compatible `#[serde(default)]` fields, and hex-to-COLORREF parser.

## Architectural Principles & Constraints

1. **Game-Independent Design:** Never add fixed coordinates, game-specific hardcoded rules, or single-game dependencies.
2. **Zero Self-Capture Loop:** `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` must always be preserved on the overlay window; otherwise OCR reads its own translated overlay in an infinite loop.
3. **Zero Focus Stealing & Click-Through:** The overlay must preserve `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, and use `SWP_NOACTIVATE` for visibility/position updates. The window is drawn with per-pixel alpha (`UpdateLayeredWindow`, see `render.rs`); never call `SetLayeredWindowAttributes` on it, because that makes every later `UpdateLayeredWindow` fail.
4. **Minimal Dependencies & Small Binary:** Prefer native Windows APIs and the Rust standard library over heavy GUI frameworks or unnecessary crates.
5. **Adding Translation Engines:** Every backend must implement the `Translator` trait in `src/translator.rs`, fall back to `GoogleTranslator` on failure, and be configurable via `config.rs`, `main.rs`, and `gui.rs`.
6. **Backward-Compatible `config.json`:** Every new field added to `AppConfig` must carry `#[serde(default = "...")]` so existing user `config.json` files continue to load cleanly.
7. **Native GUI Design Rule (`gui.rs`):** Neutral Dark Graphite default (`#141414` bg, `#1C1C1C` surface, `#2B2B2B` stroke, `#EBEBEB` text) plus optional neutral Light theme; zero emojis, zero blue/purple accents, zero drop shadows or gradients, monospace numeric readouts, and crisp technical labels (`Border Radius`, `Opacity`, `Padding X/Y`, `Auto-Fit Container`).
8. **Consoleless & System Tray Lifecycle:** Release builds use `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`. Closing the GUI window (`X`) hides to the System Tray; `Ctrl + Shift + Q` or tray `Quit` terminates the process cleanly.
9. **Security:** `config.json` contains user API keys and must remain in `.gitignore`, together with its `config.json.bak` / `config.json.tmp` copies (`config.json.*`); never commit or expose them.

## Build and Verification

```powershell
cargo build --release
```

Note: On Windows, compiling while `target\release\overcap.exe` is running fails with `os error 5` (file lock). Close any running instance with `Ctrl + Shift + Q` before rebuilding.
