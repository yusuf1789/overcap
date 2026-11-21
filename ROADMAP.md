# Overcap — Improvement Ideas

[Türkçe sürüm: ROADMAP.tr.md](ROADMAP.tr.md) · [English README](README.md)

This document records proposed improvements to everyday use and the gaming experience. Implemented items are marked explicitly; the others are proposals whose order and details will be refined during development.

The priority is to preserve natural translation quality while reducing setup effort and showing the right translation at the right time. The design must remain independent of specific games and applications.

## 1. Attach the Subtitle Region to a Window

**Suggested priority: First phase.**

The selected region currently uses screen coordinates. Moving or resizing the source window can leave the region behind. Users should be able to attach their selection to a Remote Play window, video player, or another source window.

Proposed behavior:

- Keep the existing fixed screen region; window tracking is optional.
- Let users explicitly choose the source window. Changing the foreground application must not silently switch the source.
- Store the selection as position and size ratios within the window's client area, excluding its title bar and borders. Recalculate screen coordinates when the window moves and scale the region proportionally when it resizes.
- Update the capture region and translation overlay together. A coordinate change alone should not require another API request for the same subtitle.
- Pause OCR and hide the old overlay when the source is minimized, closed, or has no usable capture area. Do not keep reading the previous screen location when other applications occupy it.
- Resume tracking when the source becomes usable again. Revalidate window identity after an application restarts; do not silently choose an ambiguous match when the application has multiple windows.

Technical scope: source selection and identity checks in `selector.rs` and `profiles.rs`; backward-compatible optional attachment settings in `config.rs`; runtime coordinate tracking in `main.rs`; source bounds and visibility updates in `overlay.rs`; and tracking controls/source status in `gui.rs`. Frequent position changes must not trigger a settings-file write every time.

**Limitations:** Proportional window resizing does not guarantee proportional video placement. Letterboxing, aspect-ratio changes, and player UI changes may require reselection. The first version should not promise perfect automatic video-area detection. Retaining the existing GDI screen capture requires visible source content; window tracking does not provide capture of minimized or obscured windows.

**Acceptance criteria:** Verify moving, resizing, crossing monitors with different DPI settings, minimizing/restoring, and closing/reopening. Preserve fixed screen regions, no-focus-stealing behavior, click-through behavior, and `WDA_EXCLUDEFROMCAPTURE`. Test fullscreen transitions separately because they can change window identity or content layout.

## 2. Synchronize Translations with Fast Dialogue

**Suggested priority: First phase.**

An old translation request can finish after a new subtitle has appeared. The goal is to display the translation matching the current source subtitle rather than simply the most recently completed request.

- Associate results with source subtitle identity and freshness.
- Prevent late results from displaying an old subtitle or reverting to a previous line.
- Improve hiding behavior when the source disappears while avoiding flicker during brief OCR gaps.
- Preserve translation context from previous lines and tolerance for short OCR glitches.

**Acceptance criteria:** Avoid displaying the wrong line during fast dialogue and abrupt subtitle changes without unnecessarily hiding long subtitles. Evaluate time spent showing the correct line as well as total milliseconds.

## 3. Expand Application Profiles

Extend existing region and appearance profiles with source language, target language, and translation engine/model preferences. Remote Play and a video player could automatically use different preferences.

**Acceptance criteria:** Apply the correct settings consistently, prevent old results from appearing under the new profile, and load existing settings files successfully. Avoid unnecessarily duplicating API keys across profiles.

## 4. Optional Terminology and Name Glossary

Support user-defined translations or “keep unchanged” preferences for character names, locations, and special terms. A glossary could be global or tied to an application profile.

**Acceptance criteria:** Improve naming consistency over long sessions while retaining natural dialogue. Do not implement a blind text replacement that alters parts of words or unrelated meanings. Measure added latency and differences in backend support.

## 5. Short In-Game Translation History

**Status: Implemented in v0.4.4.** A non-activating panel toggled by `Ctrl+Shift+L` displays the last 15 target-language translations. Scroll using the mouse wheel or `Ctrl+Shift+PageUp/PageDown` while open. Session history is not saved to disk; panel labels follow the EN/TR UI choice. See the [README](README.md#in-game-translation-history) for usage.

Use a hotkey to open a temporary view of the last few subtitles. Users should be able to read a missed line without opening the main control window.

**Acceptance criteria:** Open and close without stealing game focus, retain a bounded history, and avoid blocking the current subtitle stream. Exclude the history view from OCR capture too.

## 6. Long-Session Resilience

Recover with understandable status reporting after network interruptions, sleep/resume, resolution/DPI changes, and Remote Play restarts.

**Acceptance criteria:** Keep retries bounded and controlled without accumulating stale subtitles. Report source and engine states accurately and recover from temporary failures without requiring an application restart. Validate separately in actual Remote Play sessions.

## 7. Speaker Separation and Highlighting in the Same Translation Request

**Status: Idea; not implemented yet.**

The translation model could return the speaker label and translated dialogue in separate fields within the same request. No second model call is needed. Multiple speakers use ordered segments, each containing an optional `speaker` and translated `text`.

- Separate only speakers explicitly identified in the source; never infer a name or identity. Use an empty/null `speaker` when no label exists. An address such as “Michael, come here!” is not a speaker label; do not force ambiguous text such as mission headings into speaker names.
- Render extracted names in a distinct color and weight while retaining the existing dialogue appearance. Offer optional speaker highlighting and a color setting in Appearance. A single accent color for all speakers is sufficient initially.
- Display ordinary, single-color translations when using Google fallback or a backend without structured-response support. Preserve usable translation text for malformed/incomplete structured responses; never display raw JSON, silently lose a line, or make another network request just to repair formatting.
- Preserve speaker information in the cache and history too. Verify support for each backend/model rather than assuming that API compatibility guarantees support.
- Additional output fields are not free. Keep the existing model and two-line context while comparing translation quality, speaker-label accuracy, and latency on the same subtitles.

**Acceptance criteria:** Test single/multiple speakers, unlabelled dialogue, names used as an address, OCR-damaged labels, mission headings, and malformed responses. Do not duplicate labels or change dialogue order/content. Do not assume a speed or accuracy improvement before measuring quality and latency.

## 8. Optional In-Scene Non-Subtitle Text Filtering (HUD & Signs)

**Status: Idea; not implemented yet.**

When subtitles overlap with world signs, floating HUD prompts, or background labels inside the capture strip, OCR can accidentally pick up non-subtitle text. An optional filter can pre-screen OCR lines using brightness and glyph-height heuristics before sending text to translation.

- Filter candidate OCR lines by minimum bright-pixel ratio/luminance and expected line height relative to the dominant subtitle text, keeping the filter strictly optional and off by default.
- Preserve game-independent heuristics without fixed coordinates or title-specific rules.

**Acceptance criteria:** Reject dim/tiny background signs and off-scale HUD tags without dropping legitimate dialogue lines, single-word replies, or italic/dimmed speaker cues.

## Suggested Order and Shared Rules

Translation history is complete. For the remaining items, start with window tracking, then subtitle synchronization. Prioritize the remaining ideas according to actual usage.

- Compare translation quality and latency together whenever a change to the existing model, prompt, or context is necessary.
- Do not introduce game-specific fixed coordinates or rules.
- Keep new settings backward compatible with defaults.
- Prefer native Windows APIs and existing dependencies.
- Validate each improvement with relevant functional tests and a short desktop check.
