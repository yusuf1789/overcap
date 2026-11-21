# Overcap

[Türkçe dokümantasyon için README.tr.md](README.tr.md)

[Improvement ideas and roadmap](ROADMAP.md)

General-purpose, low-latency live game and video subtitle translator overlay for Windows.

Overcap captures a user-selected screen region via GDI `BitBlt`, extracts text and word bounding coordinates using the built-in `Windows.Media.Ocr` engine, translates it through your chosen backend (`Groq` / `Gemini` / `Google Translate`), and renders the translation **directly over** the original subtitle using a capture-excluded (`WDA_EXCLUDEFROMCAPTURE`) transparent Win32 overlay window.

Works game-independently with PC games (Borderless Windowed), console streaming/capture (Xbox Remote Play / HDMI Capture), and video players.

---

## Core Features

- **Single `.exe` (~4.2 MB, Rust):** No external runtime or external OCR model files required.
- **Direct Subtitle Replacement (`WDA_EXCLUDEFROMCAPTURE`):**
  - The overlay window is visible to the user but invisible to screen capture APIs, allowing OCR to continuously read the original subtitle underneath without self-capture loops.
  - Uses `WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` so mouse clicks pass through to the game, focus is never stolen, and the window stays hidden from Alt+Tab.
  - Automatically hides when the source subtitle disappears.
- **OCR Bounding Box Cover & Auto-Fit:**
  - Computes the actual pixel bounds of the source subtitle from `OcrWord.BoundingRect`.
  - Guarantees full coverage of the underlying text (even when a 2-line source translates to 1 line) while keeping short lines compact without wide black bars.
- **Smart Change Detection & Deduplication:**
  - Bright-pixel change mask (`luma >= 170`) prevents unnecessary OCR triggers from background camera motion.
  - Levenshtein similarity + word-overlap deduplication avoids redundant API calls and overlay flicker on noisy OCR frames.
- **Pluggable Translation Backends (`Translator` trait):**
  - **Groq (`qwen/qwen3.8-27b`, recommended):** `~150-280 ms` end-to-end latency with context-aware, colloquial game/video translation (`reasoning_effort: "none"`).
  - **Google Gemini (`gemini-3.1-flash-lite`):** Context-aware translation with OCR glitch correction.
  - **OpenAI-Compatible / Local LLMs (`openai-compat` / `ollama`):** Connect to Ollama, LM Studio, vLLM, or any `/chat/completions` endpoint (`--custom-base-url`, `--custom-model`, `--custom-key`).
  - **Google Translate:** Keyless free default and automatic fallback on API errors.
- **Automatic Per-App Profiles:**
  - `Ctrl + Shift + S` saves the selected subtitle region to the active `.exe` profile, and Overcap automatically switches regions when you switch games or video players (disable with `--no-auto-profiles` or in Settings UI).
- **Native Desktop Control Center (`egui`) & System Tray:**
  - Consoleless Windows desktop app (`Ctrl + Shift + A` or System Tray icon to open/focus; closing the window minimizes to the System Tray).
  - Configure Font Family, Font Size, Bold, Text Color, Text Shadow, Background Color, Opacity, Border Radius, Padding X/Y, Auto-Fit Container, Engine (with live reload), Languages, **OCR Diagnostics**, **App Profiles**, and **Live Log** with live preview and automatic saving to `config.json`.
- **High-DPI & Multi-Monitor Ready:** Uses `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` for 1:1 coordinate accuracy across `%125` / `%150` Windows display scaling.

---

## Measured Real-World Performance

Tested on Windows in Borderless Windowed game/video playback (Groq `qwen/qwen3.8-27b`):

| Stage | Latency |
|---|---|
| GDI `BitBlt` + bright-pixel diff | `2 - 6 ms` |
| 2x Upscale + `Windows.Media.Ocr` | `20 - 50 ms` |
| Groq (`qwen/qwen3.8-27b`, persistent TLS) | `130 - 250 ms` |
| **Total (capture -> translation result ready)** | **`~160 - 300 ms`** |

*Note: Cache hits on repeated lines complete in `~30 - 55 ms`.*

---

## In-Game Translation History

`Ctrl + Shift + L` toggles a history panel showing the **last 15 translated lines** in the target language. A button in Live Log provides the same action. Entries run from oldest to newest; opening the panel starts at the latest line.

Scroll with the mouse wheel or, while the panel is open, `Ctrl + Shift + PageUp / PageDown`. Incoming translations preserve your reading position when browsing older lines; at the bottom, the panel follows new entries. Close it using the top-right `X`, a right-click, or the toggle hotkey.

The panel does not take game focus and is excluded from OCR capture. Translation and the normal subtitle overlay continue running. Mouse interaction is handled within the panel's own area. History stays in memory for the current session only and is cleared on exit or when Live Log is cleared. Panel labels follow the EN/TR UI selection.

---

## Engine Status and Latency Diagnostics

The Engine tab distinguishes the selected engine, initialized engine, and provider of the last successful result. Missing-key and per-request Google fallbacks stay visible. Saving requests a reload; readiness is reported only after the worker finishes. Key badges also recognize the corresponding environment variable.

The active `config.json` path is shown, along with any ignored duplicate. The file beside the executable takes precedence over the working-directory file. Files are never merged automatically.

Live Log separates capture/OCR, post-OCR wait, translation, and total milliseconds. It records the actual provider/model, cache hits, fallback use, and request attempts. Translation time includes a failed primary request and any Google fallback attempts. Total time excludes Remote Play, polling before capture, and final display presentation.

Profiles & OCR offers optional local OCR recording, off on every launch. Up to four OCR samples per second are written asynchronously beside the active settings file in `ocr-diagnostics`, retaining at most 60 BMP/JSON pairs. Metadata includes recognized/previous text, source bounds, duplicate-filter decision, and current overlay text. Samples are dropped when the writer is busy. Files stay local and are ignored by Git; disabling recording does not delete existing files. Compare the same video with the overlay on/off to investigate repeated OCR without changing translation prompts or duplicate thresholds.

---

## Setup and Running

### 1. Prerequisite (Recommended)
For the highest OCR accuracy on English subtitles, install the Windows English OCR capability:
`Windows Settings -> Time & Language -> Language & Region -> Add a language -> English (United States)` (you do not need to change your Windows display language; only the Optical Character Recognition feature is required).

### 2. Build from Source
With the Rust `msvc` toolchain installed:

```powershell
cargo build --release
```

Output binary: `target\release\overcap.exe` (single portable executable with statically linked CRT; `config.json` is stored next to the executable).

> Note on unsigned binaries: If you download a prebuilt `overcap.exe` from Releases, Windows SmartScreen may show a warning on first run ("More info" -> "Run anyway"). You can always inspect the source and build locally with `cargo build --release`.

### 3. Run

```powershell
.\start.bat
```

or directly:

```powershell
.\target\release\overcap.exe
```

On first launch, the screen dims so you can drag-select the subtitle strip. Coordinates are saved automatically to `config.json`.

### CLI Arguments

```powershell
# Re-select the subtitle region on startup
.\start.bat --select

# Start minimized in the System Tray
.\target\release\overcap.exe --minimized

# Run with Groq (key and engine are saved to config.json)
.\start.bat --engine groq --groq-key gsk_...

# Run with Gemini
.\start.bat --engine gemini --gemini-key AIza...

# Custom language pair (default: en -> tr)
.\start.bat --from en --to tr

# Custom scan interval (ms)
.\start.bat --interval-ms 100
```

See [config.example.json](config.example.json) for a configuration template. `config.json` is listed in `.gitignore` so API keys are never committed.

---

## Global In-Game Hotkeys

| Hotkey | Action |
|---|---|
| `Ctrl + Shift + L` | Show / hide the last 15 translations |
| `Ctrl + Shift + PageUp / PageDown` | Scroll to older / newer history entries while open |
| `Ctrl + Shift + A` | Open / focus Overcap control center window |
| `Ctrl + Shift + S` | Re-select subtitle capture region |
| `Ctrl + Shift + H` | Toggle overlay visibility / pause |
| `Ctrl + Shift + Up` | Increase font size (+2 px) |
| `Ctrl + Shift + Down` | Decrease font size (-2 px) |
| `Ctrl + Shift + Q` | Quit application |

---

## In-Game Tips

- Run games in **Borderless Windowed** mode (in Exclusive Fullscreen, Windows overlay windows are rendered behind the game).
- If the game offers a **Subtitle Background** (dark/semi-transparent backdrop) option, enabling it significantly improves OCR accuracy.

---

## Project Structure

- `Cargo.toml` — Rust package manifest and Windows API dependencies.
- `src/main.rs` — CLI (`clap`), DPI initialization, and asynchronous OCR / translation worker coordination.
- `src/capture.rs` — GDI `BitBlt` region capture, bright-pixel change mask, and 2x bilinear upscale.
- `src/ocr.rs` — `Windows.Media.Ocr` wrapper, line ordering, and `SourceBounds` calculation.
- `src/translator.rs` — `Translator` trait (`GroqTranslator`, `GeminiTranslator`, `GoogleTranslator`), cache, and smart duplicate filter.
- `src/render.rs` — Per-pixel-alpha overlay compositing: translucent background box, text outline and shadow, always-opaque text.
- `src/history.rs` — Last-15 translation buffer, non-activating Win32 history panel, and scrolling.
- `src/overlay.rs` — `WDA_EXCLUDEFROMCAPTURE` + click-through Win32 overlay, dynamic Auto-Fit, per-app styles, tray dispatch, and global hotkeys.
- `src/selector.rs` — High-DPI, multi-monitor Win32 region selector.
- `src/gui.rs` — Native `eframe`/`egui` desktop control center and live preview UI.
- `src/tray.rs` — Native Win32 System Tray icon and context menu.
- `src/profiles.rs` — Foreground `.exe` detection for per-app subtitle region profiles.
- `src/config.rs` — `config.json` serialization and hex color parser.
- `start.bat` — One-click Windows launcher.
- `config.example.json` — Example configuration template.
- `VERSIONING.md` — Semantic Versioning policy and changelog.
- `AGENTS.md` — Architectural guide and rules for contributors and coding agents.

---

## License

Licensed under the [MIT License](LICENSE).
