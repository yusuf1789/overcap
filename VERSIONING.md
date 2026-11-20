# Versioning

Overcap follows [Semantic Versioning 2.0.0](https://semver.org/) (`MAJOR.MINOR.PATCH`).

## Version Policy

- **MAJOR (`X.0.0`)**: Backward-incompatible changes to the `config.json` schema, breaking architectural changes, or breaking CLI flag changes.
- **MINOR (`0.X.0`)**: Backward-compatible new features (new translation backend, new overlay capability, new settings UI controls).
- **PATCH (`0.0.X`)**: Bug fixes, OCR/capture performance improvements, and documentation updates.

The canonical version number is maintained in `package.version` inside [Cargo.toml](Cargo.toml).

---

## Changelog

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
