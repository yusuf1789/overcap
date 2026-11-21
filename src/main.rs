#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod config;
mod diagnostics;
mod gui;
mod history;
mod ocr;
mod overlay;
mod profiles;
mod render;
mod selector;
mod translator;
mod tray;

use anyhow::Result;
use clap::Parser;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

use crate::capture::{capture_region, changed_pixel_ratio, upscale_for_ocr};
use crate::config::AppConfig;
use crate::ocr::{SourceBounds, WinOcr};
use crate::overlay::{
    create_handle, run_overlay_loop, EngineStatus, LogEntry, NoticeLevel, OverlayHandle,
};
use crate::selector::select_region;
use crate::translator::{
    is_same_subtitle, GeminiTranslator, GoogleTranslator, GroqTranslator, OpenAiCompatTranslator,
    Translator,
};

#[derive(Parser, Debug)]
#[command(
    name = "overcap",
    about = "General-purpose live game/video subtitle translator overlay (OCR -> Overlay)"
)]
struct Args {
    /// Re-select the subtitle capture region
    #[arg(long)]
    select: bool,

    /// Translation engine: google, gemini, groq, or openai-compat
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

    /// OpenAI-compatible base URL (e.g. http://127.0.0.1:11434/v1)
    #[arg(long)]
    custom_base_url: Option<String>,

    /// OpenAI-compatible API key (optional for local servers like Ollama)
    #[arg(long)]
    custom_key: Option<String>,

    /// OpenAI-compatible model (default: qwen2.5:7b)
    #[arg(long)]
    custom_model: Option<String>,

    /// Disable automatic per-app subtitle region switching
    #[arg(long)]
    no_auto_profiles: bool,

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

    /// Start minimized in the System Tray without showing the main window
    #[arg(long)]
    minimized: bool,
}

struct TranslationJob {
    seq: u64,
    text: String,
    source_bounds: Option<SourceBounds>,
    captured_at: Instant,
    ocr_ms: u128,
    /// OCR dili ve motor nesli, yenileme sırasında eski işlerin karışmasını önler.
    source_lang: String,
    revision: u64,
    queued_at: Instant,
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

    fn take_blocking(
        &self,
        stop_flag: &std::sync::atomic::AtomicBool,
        reload_flag: &std::sync::atomic::AtomicBool,
    ) -> Option<TranslationJob> {
        let mut guard = self.job.lock().ok()?;
        while guard.is_none() {
            if stop_flag.load(Ordering::SeqCst) || reload_flag.load(Ordering::SeqCst) {
                return None;
            }
            let (g, _) = self
                .cv
                .wait_timeout(guard, Duration::from_millis(150))
                .ok()?;
            guard = g;
        }
        // Dolu kutu da yenilemeden önce tüketilmemelidir.
        if stop_flag.load(Ordering::SeqCst) || reload_flag.load(Ordering::SeqCst) {
            return None;
        }
        guard.take()
    }
}

fn has_meaningful_letters(s: &str) -> bool {
    let alpha_count = s.chars().filter(|c| c.is_alphabetic()).count();
    alpha_count >= 2
}

fn current_hms() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Builds the configured engine. If it cannot be created (missing key, client error) the
/// Google translator is returned instead, together with a warning describing why.
fn build_translator(cfg: &AppConfig) -> Result<(Box<dyn Translator>, Option<String>)> {
    let engine_name = cfg.engine.as_str();
    let source_lang = cfg.source_lang.as_str();
    let target_lang = cfg.target_lang.as_str();
    let mut warning: Option<String> = None;

    if engine_name.eq_ignore_ascii_case("groq") {
        let key = cfg
            .groq_api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                std::env::var("GROQ_API_KEY")
                    .ok()
                    .filter(|k| !k.trim().is_empty())
            });

        if let Some(api_key) = key {
            match GroqTranslator::new(api_key, cfg.groq_model.clone(), source_lang, target_lang) {
                Ok(t) => {
                    println!("[Ready] Groq model: {}", cfg.groq_model);
                    return Ok((Box::new(t), None));
                }
                Err(e) => {
                    eprintln!("[Groq ERROR] {e} -> falling back to Google");
                    warning = Some(format!("Groq could not start ({e}); using Google."));
                }
            }
        } else {
            println!("[Warning] GROQ_API_KEY not found (--groq-key or config.json), falling back to Google.");
            warning = Some("Groq is selected but no API key is set; using Google.".to_string());
        }
    } else if engine_name.eq_ignore_ascii_case("gemini") {
        let key = cfg
            .gemini_api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                std::env::var("GEMINI_API_KEY")
                    .ok()
                    .filter(|k| !k.trim().is_empty())
            });

        if let Some(api_key) = key {
            match GeminiTranslator::new(api_key, cfg.gemini_model.clone(), source_lang, target_lang)
            {
                Ok(t) => {
                    println!("[Ready] Gemini model: {}", cfg.gemini_model);
                    return Ok((Box::new(t), None));
                }
                Err(e) => {
                    eprintln!("[Gemini ERROR] {e} -> falling back to Google");
                    warning = Some(format!("Gemini could not start ({e}); using Google."));
                }
            }
        } else {
            println!("[Warning] GEMINI_API_KEY not found (--gemini-key or config.json), falling back to Google.");
            warning = Some("Gemini is selected but no API key is set; using Google.".to_string());
        }
    } else if engine_name.eq_ignore_ascii_case("openai-compat")
        || engine_name.eq_ignore_ascii_case("custom")
        || engine_name.eq_ignore_ascii_case("ollama")
    {
        let api_key = cfg
            .custom_api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                std::env::var("OPENAI_API_KEY")
                    .ok()
                    .filter(|k| !k.trim().is_empty())
            });

        match OpenAiCompatTranslator::new(
            cfg.custom_base_url.clone(),
            api_key,
            cfg.custom_model.clone(),
            source_lang,
            target_lang,
        ) {
            Ok(t) => {
                println!(
                    "[Ready] OpenAI-compat endpoint: {} (model: {})",
                    cfg.custom_base_url, cfg.custom_model
                );
                return Ok((Box::new(t), None));
            }
            Err(e) => {
                eprintln!("[OpenAI-compat ERROR] {e} -> falling back to Google");
                warning = Some(format!(
                    "OpenAI-compatible endpoint could not start ({e}); using Google."
                ));
            }
        }
    }

    Ok((
        Box::new(GoogleTranslator::new(source_lang, target_lang)?),
        warning,
    ))
}

fn translation_worker(
    handle: OverlayHandle,
    slot: Arc<TranslationSlot>,
    latest_seq: Arc<std::sync::atomic::AtomicU64>,
    cleared_seq: Arc<std::sync::atomic::AtomicU64>,
) {
    let mut revision = handle.engine_revision.load(Ordering::SeqCst);
    let init_cfg = handle.snapshot_config();
    let mut source_lang = init_cfg.source_lang.clone();

    let mut translator = match build_translator(&init_cfg) {
        Ok((t, warning)) => {
            handle.publish_engine_status(
                revision,
                EngineStatus {
                    ready: t.name().into(),
                    model: t.model().into(),
                    loading: false,
                    message: warning.clone(),
                    last_result: None,
                },
            );
            if let Some(w) = warning {
                handle.push_notice(NoticeLevel::Warning, "engine", w);
            }
            t
        }
        Err(e) => {
            eprintln!("[Translation ERROR] {e}");
            handle.push_notice(
                NoticeLevel::Error,
                "engine",
                format!("Translation engine failed to start: {e}"),
            );
            handle.stop_flag.store(true, Ordering::SeqCst);
            return;
        }
    };

    println!(
        "[Ready] Translation engine: {} ({} -> {})",
        translator.name(),
        source_lang,
        init_cfg.target_lang
    );

    while !handle.stop_flag.load(Ordering::SeqCst) {
        if handle.engine_reload_requested.swap(false, Ordering::SeqCst) {
            revision = handle.engine_revision.load(Ordering::SeqCst);
            let cfg = handle.snapshot_config();
            match build_translator(&cfg) {
                Ok((t, warning)) => {
                    // Etiketler ancak yeni motor kurulunca değiştirilir.
                    source_lang = cfg.source_lang.clone();
                    handle.publish_engine_status(
                        revision,
                        EngineStatus {
                            ready: t.name().into(),
                            model: t.model().into(),
                            loading: false,
                            message: warning.clone(),
                            last_result: None,
                        },
                    );
                    if let Some(w) = warning {
                        handle.push_notice(NoticeLevel::Warning, "engine", w);
                    }
                    translator = t;
                    println!(
                        "[Reloaded] Translation engine: {} ({} -> {})",
                        translator.name(),
                        source_lang,
                        cfg.target_lang
                    );
                }
                Err(e) => {
                    handle.publish_engine_status(
                        revision,
                        EngineStatus {
                            ready: translator.name().into(),
                            model: translator.model().into(),
                            loading: false,
                            message: Some(format!("Reload failed; previous engine retained: {e}")),
                            last_result: None,
                        },
                    );
                    eprintln!("[Reload ERROR] {e}");
                    handle.push_notice(
                        NoticeLevel::Error,
                        "engine",
                        format!("Engine reload failed, keeping the previous engine: {e}"),
                    );
                }
            }
        }

        let Some(job) = slot.take_blocking(&handle.stop_flag, &handle.engine_reload_requested)
        else {
            continue;
        };

        // Drop job only if it was already superseded BEFORE we even started translating
        if job.seq < latest_seq.load(Ordering::SeqCst)
            || job.revision != revision
            || !job.source_lang.eq_ignore_ascii_case(&source_lang)
            || handle.engine_reload_requested.load(Ordering::SeqCst)
        {
            continue;
        }

        let wait_ms = job.queued_at.elapsed().as_millis();
        let translation_started = Instant::now();
        let result = translator.translate(&job.text);
        let translation_ms = translation_started.elapsed().as_millis();
        let total_ms = job.captured_at.elapsed().as_millis();
        let report = translator.report();
        // Devam eden eski istek iptal edilemese bile yeni motorun sonucuymuş gibi sunulmaz.
        if revision != handle.engine_revision.load(Ordering::SeqCst)
            || handle.engine_reload_requested.load(Ordering::SeqCst)
        {
            continue;
        }
        if let Some(note) = translator.take_notice() {
            handle.push_notice(NoticeLevel::Warning, translator.name(), note);
        }

        match result {
            Ok(translated) if !translated.is_empty() => {
                // Show translation unless subtitles were explicitly cleared/reset after this capture
                if job.seq > cleared_seq.load(Ordering::SeqCst) {
                    handle.publish_translation(
                        revision,
                        LogEntry {
                            timestamp: current_hms(),
                            total_ms,
                            ocr_ms: job.ocr_ms,
                            wait_ms,
                            translation_ms,
                            engine: report.engine.clone(),
                            report: report.clone(),
                            src: job.text,
                            dst: translated.clone(),
                        },
                        job.source_bounds,
                    );
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[Translation ERROR] {e}");
                handle.push_notice(
                    NoticeLevel::Error,
                    translator.name(),
                    format!("Translation failed: {e}"),
                );
            }
        }
    }
}

fn ocr_worker(
    handle: OverlayHandle,
    slot: Arc<TranslationSlot>,
    latest_seq: Arc<std::sync::atomic::AtomicU64>,
    cleared_seq: Arc<std::sync::atomic::AtomicU64>,
    initial_source_lang: String,
    interval_ms: u64,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let mut active_source_lang = initial_source_lang;
    let mut ocr = match WinOcr::new(&active_source_lang) {
        Ok(engine) => {
            println!(
                "[OCR] Windows built-in OCR ready (lang: {}, interval: {} ms)",
                engine.language_tag, interval_ms
            );
            if active_source_lang.eq_ignore_ascii_case("en")
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
            handle.push_notice(
                NoticeLevel::Error,
                "ocr",
                format!("OCR engine failed to start: {e}"),
            );
            handle.stop_flag.store(true, Ordering::SeqCst);
            return;
        }
    };

    let sleep_dur = Duration::from_millis(interval_ms);
    let mut prev_bgra: Option<Vec<u8>> = None;
    let mut last_src = String::new();
    let mut empty_streak: u32 = 0;
    let mut last_profile_check = Instant::now() - Duration::from_secs(1);
    let mut last_fg_app: Option<String> = None;
    let mut last_revision = handle.engine_revision.load(Ordering::SeqCst);
    let mut diagnostics = diagnostics::OcrDiagnostics::new(handle.clone());

    while !handle.stop_flag.load(Ordering::SeqCst) {
        if last_profile_check.elapsed() >= Duration::from_millis(500) {
            last_profile_check = Instant::now();
            let cfg_snap = handle.snapshot_config();

            if !cfg_snap
                .source_lang
                .eq_ignore_ascii_case(&active_source_lang)
            {
                match WinOcr::new(&cfg_snap.source_lang) {
                    Ok(new_ocr) => {
                        active_source_lang = cfg_snap.source_lang.clone();
                        ocr = new_ocr;
                        prev_bgra = None;
                        last_src.clear();
                    }
                    Err(e) => {
                        handle.push_notice(
                            NoticeLevel::Error,
                            "ocr",
                            format!(
                                "Cannot switch OCR to '{}': {e}. Still reading '{}'.",
                                cfg_snap.source_lang, active_source_lang
                            ),
                        );
                    }
                }
            }

            let fg_app = profiles::foreground_process_name();
            if fg_app != last_fg_app {
                last_fg_app = fg_app.clone();
                handle.set_active_app(fg_app.clone());
                if let Some(ref app) = fg_app {
                    // Per-app subtitle look (falls back to the global style when the app has
                    // none or per-app switching is off).
                    handle.activate_style_for_app(app);
                    if cfg_snap.auto_profiles {
                        if let Some(&prof_region) = cfg_snap.profiles.get(app) {
                            if handle.current_region() != prof_region {
                                println!(
                                    "[Profile] Switched region for {} -> {:?}",
                                    app, prof_region
                                );
                                handle.set_region_from_profile(prof_region);
                            }
                        }
                    }
                }
            }
        }

        let revision = handle.engine_revision.load(Ordering::SeqCst);
        if revision != last_revision {
            // Aynı altyazı yeni motora da gönderilebilsin; eski tekrar durumu sıfırlanır.
            last_revision = revision;
            prev_bgra = None;
            last_src.clear();
        }
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
        if region.width <= 0 || region.height <= 0 {
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        let t0 = Instant::now();

        let frame = match capture_region(region) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[Capture ERROR] {e}");
                handle.push_notice(
                    NoticeLevel::Error,
                    "capture",
                    format!("Screen capture failed: {e}"),
                );
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
                handle.push_notice(NoticeLevel::Error, "ocr", format!("OCR failed: {e}"));
                thread::sleep(sleep_dur);
                continue;
            }
        };

        let clean = ocr_out.text.trim();
        let ocr_completed = Instant::now();
        let ocr_ms = ocr_completed.duration_since(t0).as_millis();
        let meaningful = has_meaningful_letters(clean);
        let duplicate = !last_src.is_empty() && is_same_subtitle(clean, &last_src);
        diagnostics.record(
            &frame,
            region,
            &ocr_out,
            &last_src,
            ocr_ms,
            if !meaningful {
                "empty"
            } else if duplicate {
                "duplicate"
            } else {
                "translate"
            },
        );
        if !meaningful {
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

        if duplicate {
            thread::sleep(sleep_dur);
            continue;
        }

        last_src = clean.to_string();
        let seq = latest_seq.fetch_add(1, Ordering::SeqCst) + 1;

        slot.push(TranslationJob {
            seq,
            text: clean.to_string(),
            source_bounds: ocr_out.bounds,
            captured_at: t0,
            ocr_ms,
            source_lang: active_source_lang.clone(),
            revision,
            queued_at: ocr_completed,
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
    let (mut cfg, config_notice) = AppConfig::load_with_notice();

    // The overlay handle does not exist yet, so notices are queued and pushed once it does.
    let mut startup_notices: Vec<(NoticeLevel, &str, String)> = Vec::new();
    if let Some(msg) = config_notice {
        startup_notices.push((NoticeLevel::Warning, "config", msg));
    }
    if let Some(ignored) = AppConfig::ignored_path() {
        startup_notices.push((
            NoticeLevel::Warning,
            "config",
            format!(
                "Two config.json files exist. Using {} and ignoring {}. Keys and settings in the \
                 ignored file have no effect; merge them or delete one.",
                AppConfig::path().display(),
                ignored.display()
            ),
        ));
    }

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
    if let Some(url) = args.custom_base_url {
        cfg.custom_base_url = url;
        cfg_dirty = true;
    }
    if let Some(key) = args.custom_key {
        cfg.custom_api_key = Some(key);
        cfg_dirty = true;
    }
    if let Some(model) = args.custom_model {
        cfg.custom_model = model;
        cfg_dirty = true;
    }
    if args.no_auto_profiles {
        cfg.auto_profiles = false;
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
        if let Err(e) = cfg.save() {
            startup_notices.push((
                NoticeLevel::Error,
                "config",
                format!("Could not save config.json: {e}"),
            ));
        }
    }

    let region = if args.select {
        match select_region()? {
            Some(r) => {
                cfg.region = Some(r);
                cfg.save()?;
                r
            }
            None => cfg.region.unwrap_or(crate::config::Region {
                left: 0,
                top: 0,
                width: 0,
                height: 0,
            }),
        }
    } else {
        cfg.region.unwrap_or(crate::config::Region {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
        })
    };

    let source_lang = cfg.source_lang.clone();
    let handle = create_handle(cfg, region);
    for (level, source, msg) in startup_notices {
        handle.push_notice(level, source, msg);
    }
    let slot = Arc::new(TranslationSlot::new());
    let latest_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let cleared_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let tx_handle = handle.clone();
    let tx_slot = slot.clone();
    let tx_seq = latest_seq.clone();
    let tx_cleared = cleared_seq.clone();
    thread::spawn(move || {
        translation_worker(tx_handle, tx_slot, tx_seq, tx_cleared);
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

    let overlay_handle = handle.clone();
    thread::spawn(move || {
        let mut exit_code = 0;
        if let Err(e) = run_overlay_loop(overlay_handle.clone()) {
            eprintln!("[Overlay ERROR] {e}");
            overlay_handle.push_notice(
                NoticeLevel::Error,
                "overlay",
                format!("Overlay window failed: {e}"),
            );
            overlay_handle.stop_flag.store(true, Ordering::SeqCst);
            exit_code = 1;
        }

        // The overlay loop only ends on Quit (or a fatal error). The GUI normally notices
        // `stop_flag` in `update()` and closes itself, but a window hidden in the tray may
        // never be repainted, which would leave an invisible process running. Give it a
        // moment to shut down cleanly, then force the exit. Settings are saved on every
        // change, so nothing is lost.
        overlay_handle.stop_flag.store(true, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(1000));
        std::process::exit(exit_code);
    });

    gui::run_gui(handle, args.minimized)?;
    Ok(())
}

#[cfg(test)]
mod mailbox_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Dolu kutu yenileme sırasında tüketilirse iş yanlış motorla çevrilebilir.
    #[test]
    fn reload_preserves_latest_mailbox_job_until_worker_is_ready() {
        let slot = TranslationSlot::new();
        for seq in 1..=2 {
            slot.push(TranslationJob {
                seq,
                text: seq.to_string(),
                source_bounds: None,
                captured_at: Instant::now(),
                ocr_ms: 0,
                source_lang: "en".into(),
                revision: 0,
                queued_at: Instant::now(),
            });
        }
        let stop = AtomicBool::new(false);
        let reload = AtomicBool::new(true);
        assert!(slot.take_blocking(&stop, &reload).is_none());
        reload.store(false, Ordering::SeqCst);
        assert_eq!(slot.take_blocking(&stop, &reload).unwrap().seq, 2);
    }
}
