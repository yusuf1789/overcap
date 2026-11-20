use anyhow::{anyhow, Result};
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;

use crate::capture::CapturedFrame;

/// Bounding box of the detected source subtitle relative to the captured region (in screen pixels).
#[derive(Debug, Clone, Copy)]
pub struct SourceBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone)]
pub struct OcrOutput {
    pub text: String,
    pub bounds: Option<SourceBounds>,
}

pub struct WinOcr {
    engine: OcrEngine,
    pub language_tag: String,
}

impl WinOcr {
    pub fn new(lang_hint: &str) -> Result<Self> {
        let lower = lang_hint.to_lowercase();
        let candidates = match lower.as_str() {
            "en" => vec!["en-US", "en-GB", "en"],
            other => vec![other],
        };

        for tag in candidates {
            if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
                if OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
                    if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                        return Ok(Self {
                            engine,
                            language_tag: tag.to_string(),
                        });
                    }
                }
            }
        }

        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| {
            anyhow!(
                "Failed to initialize Windows OCR engine (ensure the required OCR language pack is installed in Windows Language Settings): {e}"
            )
        })?;

        let tag = engine
            .RecognizerLanguage()
            .and_then(|l| l.LanguageTag())
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "user-profile".to_string());

        Ok(Self {
            engine,
            language_tag: tag,
        })
    }

    pub fn recognize(&self, frame: &CapturedFrame, scale: f32) -> Result<OcrOutput> {
        let buffer = CryptographicBuffer::CreateFromByteArray(&frame.bgra)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            frame.width,
            frame.height,
        )?;

        let result = self.engine.RecognizeAsync(&bitmap)?.get()?;
        let lines = result.Lines()?;
        let line_count = lines.Size()?;

        let mut ordered_lines: Vec<(i32, i32, String)> = Vec::with_capacity(line_count as usize);
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut has_bounds = false;

        for i in 0..line_count {
            let Ok(line) = lines.GetAt(i) else { continue };
            let mut line_min_x = f32::MAX;
            let mut line_min_y = f32::MAX;

            if let Ok(words) = line.Words() {
                let word_count = words.Size().unwrap_or(0);
                for j in 0..word_count {
                    if let Ok(word) = words.GetAt(j) {
                        if let Ok(rect) = word.BoundingRect() {
                            if rect.Width > 0.0 && rect.Height > 0.0 {
                                min_x = min_x.min(rect.X);
                                min_y = min_y.min(rect.Y);
                                max_x = max_x.max(rect.X + rect.Width);
                                max_y = max_y.max(rect.Y + rect.Height);
                                line_min_x = line_min_x.min(rect.X);
                                line_min_y = line_min_y.min(rect.Y);
                                has_bounds = true;
                            }
                        }
                    }
                }
            }

            if let Ok(line_text) = line.Text() {
                let s = line_text.to_string();
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    // Bucket Y by 16px (in OCR space) so words on the same visual line sort left-to-right
                    let y_bucket = if line_min_y < f32::MAX {
                        (line_min_y / 16.0).round() as i32
                    } else {
                        i as i32
                    };
                    let x_pos = if line_min_x < f32::MAX {
                        line_min_x as i32
                    } else {
                        0
                    };
                    ordered_lines.push((y_bucket, x_pos, trimmed.to_string()));
                }
            }
        }

        ordered_lines.sort_by_key(|(y, x, _)| (*y, *x));
        let normalized = ordered_lines
            .into_iter()
            .map(|(_, _, t)| t)
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        let inv = if scale > 0.0 { 1.0 / scale } else { 1.0 };
        let bounds = if has_bounds && max_x > min_x && max_y > min_y {
            Some(SourceBounds {
                x: (min_x * inv).floor().max(0.0) as i32,
                y: (min_y * inv).floor().max(0.0) as i32,
                width: ((max_x - min_x) * inv).ceil().max(1.0) as i32,
                height: ((max_y - min_y) * inv).ceil().max(1.0) as i32,
            })
        } else {
            None
        };

        Ok(OcrOutput {
            text: normalized,
            bounds,
        })
    }
}

/// Returns sorted BCP-47 language tags of all Windows OCR recognizer languages
/// currently installed on the system (e.g. `["en-US", "tr"]`).
pub fn available_ocr_languages() -> Vec<String> {
    let Ok(langs) = OcrEngine::AvailableRecognizerLanguages() else {
        return Vec::new();
    };
    let count = langs.Size().unwrap_or(0);
    let mut tags = Vec::with_capacity(count as usize);
    for i in 0..count {
        if let Ok(lang) = langs.GetAt(i) {
            if let Ok(tag) = lang.LanguageTag() {
                let s = tag.to_string();
                if !s.is_empty() {
                    tags.push(s);
                }
            }
        }
    }
    tags.sort();
    tags.dedup();
    tags
}

/// Returns `true` if any installed Windows OCR language tag matches `lang_hint`
/// (case-insensitive exact match, or primary-subtag prefix match such as `"en"`
/// matching `"en-US"` or `"en-GB"`).
pub fn is_language_installed(lang_hint: &str) -> bool {
    let hint = lang_hint.trim().to_lowercase();
    if hint.is_empty() {
        return false;
    }
    let prefix = format!("{hint}-");
    available_ocr_languages().into_iter().any(|tag| {
        let lower = tag.to_lowercase();
        lower == hint || lower.starts_with(&prefix)
    })
}
