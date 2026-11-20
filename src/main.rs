mod capture;
mod config;
mod ocr;
mod overlay;
mod selector;
mod settings_ui;
mod translator;

use anyhow::Result;
use clap::Parser;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

use crate::capture::{capture_region, changed_pixel_ratio, upscale_for_ocr};
use crate::config::AppConfig;
use crate::ocr::{SourceBounds, WinOcr};
use crate::overlay::{create_handle, open_settings_in_browser, run_overlay_loop, OverlayHandle};
use crate::selector::select_region;
use crate::settings_ui::start_settings_server;
use crate::translator::{is_same_subtitle, GeminiTranslator, GoogleTranslator, GroqTranslator, Translator};

#[derive(Parser, Debug)]
#[command(
    name = "overcap",
    about = "General-purpose live game/video subtitle translator overlay (OCR -> Overlay)"
)]
struct Args {
    /// Re-select the subtitle capture region
    #[arg(long)]
    select: bool,

    /// Translation engine: google, gemini, or groq
    #[arg(long)]
    engine: Option<String>,

    /// Gemini API key (or GEMINI_API_KEY env var / config.json)
    #[arg(long)]
    gemini_key: Option<String>,

    /// Gemini model (default: gemini-3.1-flash-lite)
    #[arg(long)]
    gemini_model: Option<String>,

    /// Groq API key (or GROQ_API_KEY env var / config.json)
    #[arg(long)]
    groq_key: Option<String>,

    /// Groq model (default: qwen/qwen3.8-27b)
    #[arg(long)]
    groq_model: Option<String>,

    /// Source language code (e.g. en, de, fr, ja)
    #[arg(long)]
    from: Option<String>,

    /// Target language code (e.g. tr, es, de)
    #[arg(long)]
    to: Option<String>,

    /// Capture scan interval in milliseconds
    #[arg(long, default_value_t = 100)]
    interval_ms: u64,

    /// Overlay font size in pixels
    #[arg(long)]
    font_size: Option<i32>,

    /// Overlay background opacity (0-255, recommended: 225-245)
    #[arg(long)]
    opacity: Option<u8>,

    /// Open the appearance and settings UI in the browser on startup
    #[arg(long)]
    settings: bool,

    /// Local port for the settings UI server (default: 8787)
    #[arg(long, default_value_t = 8787)]
    settings_port: u16,
}

struct TranslationJob {
    seq: u64,
    text: String,
    source_bounds: Option<SourceBounds>,
    captured_at: Instant,
    ocr_ms: u128,
}

/// Latest-only mailbox so OCR never blocks on network translation and stale lines are skipped.
struct TranslationSlot {
    job: Mutex<Option<TranslationJob>>,
    cv: Condvar,
}

impl TranslationSlot {
    fn new() -> Self {
        Self {
            job: Mutex::new(None),
            cv: Condvar::new(),
        }
    }

    fn push(&self, job: TranslationJob) {
        if let Ok(mut guard) = self.job.lock() {
            *guard = Some(job);
            self.cv.notify_one();
        }
    }

    fn take_blocking(&self, stop_flag: &std::sync::atomic::AtomicBool) -> Option<TranslationJob> {
        let mut guard = self.job.lock().ok()?;
        while guard.is_none() {
            if stop_flag.load(Ordering::SeqCst) {
                return None;
            }
            let (g, _) = self
                .cv
                .wait_timeout(guard, Duration::from_millis(150))
                .ok()?;
            guard = g;
        }
        guard.take()
    }
}

fn has_meaningful_letters(s: &str) -> bool {
    let alpha_count = s.chars().filter(|c| c.is_alphabetic()).count();
    alpha_count >= 2
}

fn translation_worker(
    handle: OverlayHandle,
    slot: Arc<TranslationSlot>,
    latest_seq: Arc<std::sync::atomic::AtomicU64>,
    cleared_seq: Arc<std::sync::atomic::AtomicU64>,
    engine_name: String,
    gemini_key: Option<String>,
    gemini_model: String,
    groq_key: Option<String>,
    groq_model: String,
    source_lang: String,
    target_lang: String,
) {
    let mut translator: Box<dyn Translator> = if engine_name.eq_ignore_ascii_case("groq") {
        let key = groq_key
            .filter(|k| !k.trim().is_empty())
            .or_else(|| std::env::var("GROQ_API_KEY").ok().filter(|k| !k.trim().is_empty()));

        if let Some(api_key) = key {
            match GroqTranslator::new(api_key, groq_model.clone(), &source_lang, &target_lang) {
                Ok(t) => {
                    println!("[Ready] Groq model: {}", groq_model);
                    Box::new(t)
                }
                Err(e) => {
                    eprintln!("[Groq ERROR] {e} -> falling back to Google");
                    match GoogleTranslator::new(&source_lang, &target_lang) {
                        Ok(g) => Box::new(g),
                        Err(ge) => {
                            eprintln!("[Translation ERROR] {ge}");
                            handle.stop_flag.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            }
        } else {
            println!("[Warning] GROQ_API_KEY not found (--groq-key or config.json), falling back to Google.");
            match GoogleTranslator::new(&source_lang, &target_lang) {
                Ok(g) => Box::new(g),
                Err(e) => {
                    eprintln!("[Translation ERROR] {e}");
                    handle.stop_flag.store(true, Ordering::SeqCst);
                    return;
                }
            }
        }
    } else if engine_name.eq_ignore_ascii_case("gemini") {
        let key = gemini_key
            .filter(|k| !k.trim().is_empty())
            .or_else(|| std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.trim().is_empty()));

        if let Some(api_key) = key {
            match GeminiTranslator::new(api_key, gemini_model.clone(), &source_lang, &target_lang) {
                Ok(t) => {
                    println!("[Ready] Gemini model: {}", gemini_model);
                    Box::new(t)
                }
                Err(e) => {
                    eprintln!("[Gemini ERROR] {e} -> falling back to Google");
                    match GoogleTranslator::new(&source_lang, &target_lang) {
                        Ok(g) => Box::new(g),
                        Err(ge) => {
                            eprintln!("[Translation ERROR] {ge}");
                            handle.stop_flag.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            }
        } else {
            println!("[Warning] GEMINI_API_KEY not found (--gemini-key or config.json), falling back to Google.");
            match GoogleTranslator::new(&source_lang, &target_lang) {
                Ok(g) => Box::new(g),
                Err(e) => {
                    eprintln!("[Translation ERROR] {e}");
                    handle.stop_flag.store(true, Ordering::SeqCst);
                    return;
                }
            }
        }
    } else {
        match GoogleTranslator::new(&source_lang, &target_lang) {
            Ok(t) => Box::new(t),
            Err(e) => {
                eprintln!("[Translation ERROR] {e}");
                handle.stop_flag.store(true, Ordering::SeqCst);
                return;
            }
        }
    };

    println!(
        "[Ready] Translation engine: {} ({} -> {})",
        translator.name(),
        source_lang,
        target_lang
    );

    while !handle.stop_flag.load(Ordering::SeqCst) {
        let Some(job) = slot.take_blocking(&handle.stop_flag) else {
            continue;
        };

        // Drop job only if it was already superseded BEFORE we even started translating
        if job.seq < latest_seq.load(Ordering::SeqCst) {
            continue;
        }

        match translator.translate(&job.text) {
            Ok(translated) if !translated.is_empty() => {
                // Show translation unless subtitles were explicitly cleared/reset after this capture
                if job.seq > cleared_seq.load(Ordering::SeqCst) {
                    let total_ms = job.captured_at.elapsed().as_millis();
                    println!(
                        "[{:>4} ms | OCR {:>2} ms] {}: {}\n                     {}: {}\n",
                        total_ms,
                        job.ocr_ms,
                        source_lang.to_uppercase(),
                        job.text,
                        target_lang.to_uppercase(),
                        translated
                    );
                    handle.set_subtitle(translated, job.source_bounds);
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[Translation ERROR] {e}");
            }
        }
    }
}

fn ocr_worker(
    handle: OverlayHandle,
    slot: Arc<TranslationSlot>,
    latest_seq: Arc<std::sync::atomic::AtomicU64>,
    cleared_seq: Arc<std::sync::atomic::AtomicU64>,
    source_lang: String,
    interval_ms: u64,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let ocr = match WinOcr::new(&source_lang) {
        Ok(engine) => {
            println!(
                "[OCR] Windows built-in OCR ready (lang: {}, interval: {} ms)",
                engine.language_tag, interval_ms
            );
            if source_lang.eq_ignore_ascii_case("en")
                && !engine.language_tag.to_lowercase().starts_with("en")
            {
                println!(
                    "[WARNING] English OCR language pack is not installed in Windows (currently using '{}'). \
                     For much cleaner recognition: Windows Settings -> Time & Language -> Language & Region -> Add 'English (United States)'.",
                    engine.language_tag
                );
            }
            engine
        }
        Err(e) => {
            eprintln!("[OCR ERROR] {e}");
            handle.stop_flag.store(true, Ordering::SeqCst);
            return;
        }
    };

    let sleep_dur = Duration::from_millis(interval_ms);
    let mut prev_bgra: Option<Vec<u8>> = None;
    let mut last_src = String::new();
    let mut empty_streak: u32 = 0;

    while !handle.stop_flag.load(Ordering::SeqCst) {
        if handle.is_paused() {
            thread::sleep(Duration::from_millis(150));
            continue;
        }

        if handle.region_changed.swap(false, Ordering::SeqCst) {
            prev_bgra = None;
            last_src.clear();
            empty_streak = 0;
            let seq = latest_seq.fetch_add(1, Ordering::SeqCst) + 1;
            cleared_seq.store(seq, Ordering::SeqCst);
            handle.set_text(String::new());
        }

        let region = handle.current_region();
        let t0 = Instant::now();

        let frame = match capture_region(region) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[Capture ERROR] {e}");
                thread::sleep(Duration::from_millis(300));
                continue;
            }
        };

        // Sensitive changed-pixel ratio check so even small subtitle changes trigger immediately
        if let Some(prev) = &prev_bgra {
            let ratio = changed_pixel_ratio(prev, &frame.bgra);
            if ratio < 0.0025 && empty_streak == 0 {
                thread::sleep(sleep_dur);
                continue;
            }
        }
        prev_bgra = Some(frame.bgra.clone());

        let ocr_frame = upscale_for_ocr(&frame);
        let scale = (ocr_frame.width as f32) / (frame.width.max(1) as f32);
        let ocr_out = match ocr.recognize(&ocr_frame, scale) {
            Ok(out) => out,
            Err(e) => {
                eprintln!("[OCR ERROR] {e}");
                thread::sleep(sleep_dur);
                continue;
            }
        };

        let clean = ocr_out.text.trim();
        if !has_meaningful_letters(clean) {
            empty_streak += 1;
            if empty_streak >= 8 && !last_src.is_empty() {
                last_src.clear();
                let seq = latest_seq.fetch_add(1, Ordering::SeqCst) + 1;
                cleared_seq.store(seq, Ordering::SeqCst);
                handle.set_text(String::new());
            }
            thread::sleep(sleep_dur);
            continue;
        }

        empty_streak = 0;

        if !last_src.is_empty() && is_same_subtitle(clean, &last_src) {
            thread::sleep(sleep_dur);
            continue;
        }

        last_src = clean.to_string();
        let ocr_ms = t0.elapsed().as_millis();
        let seq = latest_seq.fetch_add(1, Ordering::SeqCst) + 1;

        slot.push(TranslationJob {
            seq,
            text: clean.to_string(),
            source_bounds: ocr_out.bounds,
            captured_at: t0,
            ocr_ms,
        });

        thread::sleep(sleep_dur);
    }
}

fn main() -> Result<()> {
    // Enable Per-Monitor DPI awareness so selector & capture coordinates match 1:1
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let args = Args::parse();
    let mut cfg = AppConfig::load();

    let mut cfg_dirty = false;
    if let Some(engine) = args.engine {
        cfg.engine = engine;
        cfg_dirty = true;
    }
    if let Some(key) = args.gemini_key {
        cfg.gemini_api_key = Some(key);
        cfg_dirty = true;
    }
    if let Some(model) = args.gemini_model {
        cfg.gemini_model = model;
        cfg_dirty = true;
    }
    if let Some(key) = args.groq_key {
        cfg.groq_api_key = Some(key);
        cfg_dirty = true;
    }
    if let Some(model) = args.groq_model {
        cfg.groq_model = model;
        cfg_dirty = true;
    }
    if let Some(from) = args.from {
        cfg.source_lang = from;
        cfg_dirty = true;
    }
    if let Some(to) = args.to {
        cfg.target_lang = to;
        cfg_dirty = true;
    }
    if let Some(fs) = args.font_size {
        cfg.font_size = fs.clamp(14, 64);
        cfg_dirty = true;
    }
    if let Some(op) = args.opacity {
        cfg.opacity = op;
        cfg_dirty = true;
    }
    if cfg_dirty && cfg.region.is_some() {
        let _ = cfg.save();
    }

    let region = if args.select || cfg.region.is_none() {
        println!("Drag on the screen to select the subtitle region (Cancel: ESC)...");
        match select_region()? {
            Some(r) => {
                cfg.region = Some(r);
                cfg.save()?;
                println!("[Region] Saved: {:?}", r);
                r
            }
            None => {
                println!("No region selected, exiting.");
                return Ok(());
            }
        }
    } else {
        let r = cfg.region.unwrap();
        println!("[Region] Using saved region: {:?}", r);
        r
    };

    println!();
    println!("=== Hotkeys (Work In-Game) ===");
    println!("  Ctrl + Shift + A        : Open Appearance & Settings UI (http://127.0.0.1:{})", args.settings_port);
    println!("  Ctrl + Shift + S        : Re-select subtitle region");
    println!("  Ctrl + Shift + H        : Toggle overlay visibility / pause");
    println!("  Ctrl + Shift + Up       : Increase font size");
    println!("  Ctrl + Shift + Down     : Decrease font size");
    println!("  Ctrl + Shift + Q        : Quit");
    println!("==============================\n");

    let source_lang = cfg.source_lang.clone();
    let target_lang = cfg.target_lang.clone();
    let engine_name = cfg.engine.clone();
    let gemini_key = cfg.gemini_api_key.clone();
    let gemini_model = cfg.gemini_model.clone();
    let groq_key = cfg.groq_api_key.clone();
    let groq_model = cfg.groq_model.clone();
    let handle = create_handle(cfg, region, args.settings_port);
    let slot = Arc::new(TranslationSlot::new());
    let latest_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let cleared_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));

    start_settings_server(handle.clone(), args.settings_port);
    if args.settings {
        open_settings_in_browser(args.settings_port);
    }

    let tx_handle = handle.clone();
    let tx_slot = slot.clone();
    let tx_seq = latest_seq.clone();
    let tx_cleared = cleared_seq.clone();
    let tx_src = source_lang.clone();
    let tx_dst = target_lang.clone();
    thread::spawn(move || {
        translation_worker(
            tx_handle,
            tx_slot,
            tx_seq,
            tx_cleared,
            engine_name,
            gemini_key,
            gemini_model,
            groq_key,
            groq_model,
            tx_src,
            tx_dst,
        );
    });

    let ocr_handle = handle.clone();
    let ocr_slot = slot.clone();
    let ocr_seq = latest_seq.clone();
    let ocr_cleared = cleared_seq.clone();
    let interval_ms = args.interval_ms;
    thread::spawn(move || {
        ocr_worker(
            ocr_handle,
            ocr_slot,
            ocr_seq,
            ocr_cleared,
            source_lang,
            interval_ms,
        );
    });

    run_overlay_loop(handle)?;
    Ok(())
}
