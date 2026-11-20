use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::thread;

use crate::overlay::{OverlayHandle, OverlayStyle};

#[derive(Serialize)]
struct PublicConfig {
    font_family: String,
    font_size: i32,
    font_bold: bool,
    text_color: String,
    bg_color: String,
    opacity: u8,
    corner_radius: i32,
    text_shadow: bool,
    fit_to_text: bool,
    padding_x: i32,
    padding_y: i32,
    source_lang: String,
    target_lang: String,
    engine: String,
    gemini_model: String,
    groq_model: String,
    has_gemini_key: bool,
    has_groq_key: bool,
}

#[derive(Deserialize)]
struct StyleUpdatePayload {
    font_family: String,
    font_size: i32,
    font_bold: bool,
    text_color: String,
    bg_color: String,
    opacity: u8,
    corner_radius: i32,
    text_shadow: bool,
    #[serde(default = "default_true")]
    fit_to_text: bool,
    #[serde(default = "default_pad_x")]
    padding_x: i32,
    #[serde(default = "default_pad_y")]
    padding_y: i32,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    source_lang: Option<String>,
    #[serde(default)]
    target_lang: Option<String>,
    #[serde(default)]
    gemini_model: Option<String>,
    #[serde(default)]
    groq_model: Option<String>,
    #[serde(default)]
    gemini_api_key: Option<String>,
    #[serde(default)]
    groq_api_key: Option<String>,
}

fn default_true() -> bool {
    true
}
fn default_pad_x() -> i32 {
    18
}
fn default_pad_y() -> i32 {
    10
}

#[derive(Deserialize)]
struct TestOverlayPayload {
    show: bool,
    #[serde(default)]
    text: Option<String>,
}

pub fn start_settings_server(handle: OverlayHandle, port: u16) {
    thread::spawn(move || {
        let addr = format!("127.0.0.1:{}", port);
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[Settings UI] Failed to bind {}: {}", addr, e);
                return;
            }
        };

        for stream in listener.incoming() {
            if handle.stop_flag.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(stream) = stream {
                let h = handle.clone();
                thread::spawn(move || handle_client(stream, h));
            }
        }
    });
}

fn read_http_request(stream: &mut TcpStream) -> Option<(String, String, String)> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let mut header_end = None;

    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));

    while header_end.is_none() {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = Some(pos + 4);
        }
        if buf.len() > 65536 {
            return None;
        }
    }

    let h_end = header_end?;
    let header_str = String::from_utf8_lossy(&buf[..h_end]).to_string();
    let mut lines = header_str.lines();
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_len: usize = 0;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse().unwrap_or(0);
            }
        }
    }

    while buf.len() < h_end + content_len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let body_bytes = &buf[h_end..(h_end + content_len).min(buf.len())];
    let body = String::from_utf8_lossy(body_bytes).to_string();
    Some((method, path, body))
}

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn handle_client(mut stream: TcpStream, handle: OverlayHandle) {
    let Some((method, path, body)) = read_http_request(&mut stream) else {
        return;
    };

    match (method.as_str(), path.as_str()) {
        ("GET", "/") => {
            write_response(&mut stream, "200 OK", "text/html; charset=utf-8", SETTINGS_HTML);
        }
        ("GET", "/api/config") => {
            let cfg = handle.snapshot_config();
            let pub_cfg = PublicConfig {
                font_family: cfg.font_family,
                font_size: cfg.font_size,
                font_bold: cfg.font_bold,
                text_color: cfg.text_color,
                bg_color: cfg.bg_color,
                opacity: cfg.opacity,
                corner_radius: cfg.corner_radius,
                text_shadow: cfg.text_shadow,
                fit_to_text: cfg.fit_to_text,
                padding_x: cfg.padding_x,
                padding_y: cfg.padding_y,
                source_lang: cfg.source_lang,
                target_lang: cfg.target_lang,
                engine: cfg.engine,
                gemini_model: cfg.gemini_model,
                groq_model: cfg.groq_model,
                has_gemini_key: cfg
                    .gemini_api_key
                    .as_deref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false),
                has_groq_key: cfg
                    .groq_api_key
                    .as_deref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false),
            };
            let json = serde_json::to_string(&pub_cfg).unwrap_or_else(|_| "{}".to_string());
            write_response(&mut stream, "200 OK", "application/json; charset=utf-8", &json);
        }
        ("POST", "/api/preview") => {
            if let Ok(p) = serde_json::from_str::<StyleUpdatePayload>(&body) {
                let style = OverlayStyle {
                    font_family: p.font_family,
                    font_size: p.font_size.clamp(14, 64),
                    font_bold: p.font_bold,
                    text_color: p.text_color,
                    bg_color: p.bg_color,
                    opacity: p.opacity.clamp(35, 255),
                    corner_radius: p.corner_radius.clamp(0, 40),
                    text_shadow: p.text_shadow,
                    fit_to_text: p.fit_to_text,
                    padding_x: p.padding_x.clamp(4, 60),
                    padding_y: p.padding_y.clamp(2, 40),
                };
                handle.apply_style(style, true);
                write_response(&mut stream, "200 OK", "application/json", "{\"ok\":true}");
            } else {
                write_response(&mut stream, "400 Bad Request", "application/json", "{\"ok\":false}");
            }
        }
        ("POST", "/api/save") => {
            if let Ok(p) = serde_json::from_str::<StyleUpdatePayload>(&body) {
                let mut cfg = handle.snapshot_config();
                cfg.font_family = p.font_family;
                cfg.font_size = p.font_size.clamp(14, 64);
                cfg.font_bold = p.font_bold;
                cfg.text_color = p.text_color;
                cfg.bg_color = p.bg_color;
                cfg.opacity = p.opacity.clamp(35, 255);
                cfg.corner_radius = p.corner_radius.clamp(0, 40);
                cfg.text_shadow = p.text_shadow;
                cfg.fit_to_text = p.fit_to_text;
                cfg.padding_x = p.padding_x.clamp(4, 60);
                cfg.padding_y = p.padding_y.clamp(2, 40);

                if let Some(eng) = p.engine.filter(|s| !s.trim().is_empty()) {
                    cfg.engine = eng;
                }
                if let Some(sl) = p.source_lang.filter(|s| !s.trim().is_empty()) {
                    cfg.source_lang = sl;
                }
                if let Some(tl) = p.target_lang.filter(|s| !s.trim().is_empty()) {
                    cfg.target_lang = tl;
                }
                if let Some(gm) = p.gemini_model.filter(|s| !s.trim().is_empty()) {
                    cfg.gemini_model = gm;
                }
                if let Some(qm) = p.groq_model.filter(|s| !s.trim().is_empty()) {
                    cfg.groq_model = qm;
                }
                if let Some(gk) = p.gemini_api_key.filter(|s| !s.trim().is_empty()) {
                    cfg.gemini_api_key = Some(gk.trim().to_string());
                }
                if let Some(qk) = p.groq_api_key.filter(|s| !s.trim().is_empty()) {
                    cfg.groq_api_key = Some(qk.trim().to_string());
                }

                match handle.update_config_and_save(cfg) {
                    Ok(_) => write_response(&mut stream, "200 OK", "application/json", "{\"ok\":true}"),
                    Err(e) => write_response(
                        &mut stream,
                        "500 Internal Server Error",
                        "application/json",
                        &format!("{{\"ok\":false,\"error\":\"{}\"}}", e),
                    ),
                }
            } else {
                write_response(&mut stream, "400 Bad Request", "application/json", "{\"ok\":false}");
            }
        }
        ("POST", "/api/test-overlay") => {
            if let Ok(p) = serde_json::from_str::<TestOverlayPayload>(&body) {
                if p.show {
                    let sample = p.text.unwrap_or_else(|| {
                        "We need to get out of here before they find us. Stay low and follow me!"
                            .to_string()
                    });
                    handle.set_text(sample);
                } else {
                    handle.set_text(String::new());
                }
                write_response(&mut stream, "200 OK", "application/json", "{\"ok\":true}");
            } else {
                write_response(&mut stream, "400 Bad Request", "application/json", "{\"ok\":false}");
            }
        }
        _ => {
            write_response(&mut stream, "404 Not Found", "text/plain", "Not Found");
        }
    }
}

const SETTINGS_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Overcap / Settings</title>
<style>
  :root {
    --bg: #ececec;
    --surface: #f8f8f8;
    --input-bg: #ffffff;
    --border: #d0d0d0;
    --text: #1a1a1a;
    --muted: #5a5a5a;
    --btn-dark: #222222;
    --btn-dark-hover: #383838;
    --ok: #1f6f43;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    background: var(--bg);
    color: var(--text);
    font-family: "Segoe UI", system-ui, -apple-system, sans-serif;
    font-size: 13px;
    padding: 20px 18px 44px;
    line-height: 1.45;
  }
  .container { max-width: 860px; margin: 0 auto; }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 14px;
    margin-bottom: 16px;
    border-bottom: 1px solid var(--border);
    gap: 12px;
    flex-wrap: wrap;
  }
  h1 { font-size: 17px; font-weight: 600; color: var(--text); }
  .subtitle { color: var(--muted); font-size: 12px; margin-top: 2px; }
  .preview-stage {
    position: relative;
    height: 190px;
    border-radius: 6px;
    overflow: hidden;
    border: 1px solid var(--border);
    display: flex;
    align-items: flex-end;
    justify-content: center;
    padding: 20px;
    margin-bottom: 12px;
  }
  .stage-dark { background: #1e1e1e; }
  .stage-mid { background: #5a5a5a; }
  .stage-bright { background: #d8d8d8; }
  .stage-controls {
    position: absolute;
    top: 10px;
    left: 10px;
    right: 10px;
    display: flex;
    justify-content: space-between;
    gap: 6px;
    flex-wrap: wrap;
  }
  .chip-group { display: flex; gap: 4px; }
  .chip {
    background: rgba(255, 255, 255, 0.88);
    color: #222;
    border: 1px solid #bdbdbd;
    border-radius: 4px;
    padding: 4px 9px;
    font-size: 11px;
    cursor: pointer;
  }
  .chip.active {
    background: #222;
    color: #fff;
    border-color: #222;
  }
  #previewBox {
    max-width: 92%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    text-align: center;
  }
  .presets {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 16px;
    align-items: center;
  }
  .presets-label {
    font-size: 12px;
    color: var(--muted);
    margin-right: 4px;
  }
  .preset-btn {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 5px 11px;
    font-size: 12px;
    cursor: pointer;
  }
  .preset-btn:hover { background: #e2e2e2; }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: 12px;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 14px 16px;
  }
  .card h2 {
    font-size: 13px;
    font-weight: 600;
    margin-bottom: 12px;
    color: var(--text);
    border-bottom: 1px solid var(--border);
    padding-bottom: 6px;
  }
  .field { margin-bottom: 12px; }
  .field:last-child { margin-bottom: 0; }
  label {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
    color: var(--muted);
    margin-bottom: 5px;
  }
  label span.val { color: var(--text); font-weight: 600; }
  input[type="range"] { width: 100%; accent-color: #333333; cursor: pointer; }
  select, input[type="text"], input[type="password"] {
    width: 100%;
    background: var(--input-bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 6px 8px;
    font-size: 12px;
  }
  .row { display: flex; gap: 8px; align-items: center; }
  .color-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
  }
  input[type="color"] {
    width: 34px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--input-bg);
    cursor: pointer;
    padding: 2px;
  }
  .swatch {
    width: 22px;
    height: 22px;
    border-radius: 3px;
    border: 1px solid #999;
    cursor: pointer;
  }
  .toggle-row {
    display: flex;
    gap: 16px;
    margin-top: 6px;
  }
  .toggle-label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
  }
  .actions {
    margin-top: 16px;
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    align-items: center;
  }
  .btn {
    border-radius: 4px;
    padding: 7px 14px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-secondary {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
  }
  .btn-secondary:hover { background: #e2e2e2; }
  .btn-primary {
    background: var(--btn-dark);
    color: #ffffff;
    border: 1px solid var(--btn-dark);
  }
  .btn-primary:hover { background: var(--btn-dark-hover); }
  #toast {
    font-size: 12px;
    color: var(--ok);
    font-weight: 600;
    opacity: 0;
    transition: opacity 0.2s;
    margin-right: auto;
  }
  .hint { font-size: 11px; color: var(--muted); margin-top: 4px; }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .lang-chip {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 5px 10px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
  }
  .lang-chip.active {
    background: var(--btn-dark);
    color: #ffffff;
    border-color: var(--btn-dark);
  }
</style>
</head>
<body>
<div class="container">
  <header>
    <div>
      <h1>Overcap / Settings</h1>
      <div class="subtitle" id="ui_subtitle">Live overlay sync (127.0.0.1)</div>
    </div>
    <div class="header-actions">
      <div class="chip-group" role="group" aria-label="UI Language">
        <button class="lang-chip active" id="langEn" onclick="setUiLang('en')">EN</button>
        <button class="lang-chip" id="langTr" onclick="setUiLang('tr')">TR</button>
      </div>
      <button class="btn btn-secondary" id="btnTestOverlay" onclick="toggleScreenTest()">Overlay Preview: OFF</button>
    </div>
  </header>

  <div class="preview-stage stage-dark" id="stage">
    <div class="stage-controls">
      <div class="chip-group">
        <button class="chip active" id="ui_stage_dark" onclick="setStage('stage-dark', this)">Dark</button>
        <button class="chip" id="ui_stage_mid" onclick="setStage('stage-mid', this)">Gray</button>
        <button class="chip" id="ui_stage_bright" onclick="setStage('stage-bright', this)">Light</button>
      </div>
      <div class="chip-group">
        <button class="chip" id="ui_next_sample" onclick="cycleSample()">Next Sample</button>
      </div>
    </div>
    <div id="previewBox">We need to get out of here before they find us. Stay low and follow me!</div>
  </div>

  <div class="presets">
    <span class="presets-label" id="ui_presets_label">Presets:</span>
    <button class="preset-btn" id="ui_preset_cinema" onclick="applyPreset('cinema')">Cinema</button>
    <button class="preset-btn" id="ui_preset_yellow" onclick="applyPreset('yellow')">Yellow</button>
    <button class="preset-btn" id="ui_preset_minimal" onclick="applyPreset('minimal')">Minimal</button>
    <button class="preset-btn" id="ui_preset_contrast" onclick="applyPreset('contrast')">High Contrast</button>
  </div>

  <div class="grid">
    <div class="card">
      <h2 id="ui_card_typo">Typography</h2>

      <div class="field">
        <label>Font Family</label>
        <select id="font_family" onchange="onStyleChange()">
          <option value="Segoe UI">Segoe UI</option>
          <option value="Bahnschrift">Bahnschrift</option>
          <option value="Trebuchet MS">Trebuchet MS</option>
          <option value="Calibri">Calibri</option>
          <option value="Arial">Arial</option>
          <option value="Verdana">Verdana</option>
          <option value="Georgia">Georgia</option>
          <option value="Consolas">Consolas</option>
        </select>
      </div>

      <div class="field">
        <label>Font Size <span class="val" id="font_size_val">28 px</span></label>
        <input type="range" id="font_size" min="14" max="64" value="28" oninput="onStyleChange()">
      </div>

      <div class="field">
        <label>Text Color</label>
        <div class="row">
          <div class="color-wrap">
            <input type="color" id="text_color" value="#FFFFFF" oninput="onStyleChange()">
            <span class="swatch" style="background:#FFFFFF" onclick="setTextColor('#FFFFFF')" title="#FFFFFF"></span>
            <span class="swatch" style="background:#FFE55C" onclick="setTextColor('#FFE55C')" title="#FFE55C"></span>
            <span class="swatch" style="background:#E2E8F0" onclick="setTextColor('#E2E8F0')" title="#E2E8F0"></span>
            <span class="swatch" style="background:#D4D4D4" onclick="setTextColor('#D4D4D4')" title="#D4D4D4"></span>
          </div>
        </div>
      </div>

      <div class="toggle-row">
        <label class="toggle-label"><input type="checkbox" id="font_bold" checked onchange="onStyleChange()"> Bold</label>
        <label class="toggle-label"><input type="checkbox" id="text_shadow" checked onchange="onStyleChange()"> Text Shadow</label>
      </div>
    </div>

    <div class="card">
      <h2 id="ui_card_layout">Container & Layout</h2>

      <div class="field">
        <label>Opacity <span class="val" id="opacity_val">90%</span></label>
        <input type="range" id="opacity" min="40" max="255" value="230" oninput="onStyleChange()">
      </div>

      <div class="field">
        <label>Border Radius <span class="val" id="corner_radius_val">14 px</span></label>
        <input type="range" id="corner_radius" min="0" max="32" value="14" oninput="onStyleChange()">
      </div>

      <div class="field">
        <label>Padding X <span class="val" id="padding_x_val">18 px</span></label>
        <input type="range" id="padding_x" min="4" max="60" value="18" oninput="onStyleChange()">
      </div>

      <div class="field">
        <label>Padding Y <span class="val" id="padding_y_val">10 px</span></label>
        <input type="range" id="padding_y" min="2" max="40" value="10" oninput="onStyleChange()">
      </div>

      <div class="field">
        <label>Background Color</label>
        <div class="row">
          <div class="color-wrap">
            <input type="color" id="bg_color" value="#0A0A0C" oninput="onStyleChange()">
            <span class="swatch" style="background:#000000" onclick="setBgColor('#000000')" title="#000000"></span>
            <span class="swatch" style="background:#121212" onclick="setBgColor('#121212')" title="#121212"></span>
            <span class="swatch" style="background:#242424" onclick="setBgColor('#242424')" title="#242424"></span>
          </div>
        </div>
      </div>

      <div class="toggle-row">
        <label class="toggle-label"><input type="checkbox" id="fit_to_text" checked onchange="onStyleChange()"> Auto-Fit Container</label>
      </div>
    </div>

    <div class="card" style="grid-column: 1 / -1;">
      <h2 id="ui_card_engine">Translation Engine (config.json)</h2>
      <div class="grid" style="grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));">
        <div class="field">
          <label id="ui_engine_lbl">Engine</label>
          <select id="engine">
            <option value="groq">groq</option>
            <option value="gemini">gemini</option>
            <option value="google">google</option>
          </select>
        </div>
        <div class="field">
          <label>Groq Model</label>
          <input type="text" id="groq_model" value="qwen/qwen3.8-27b">
        </div>
        <div class="field">
          <label>Gemini Model</label>
          <input type="text" id="gemini_model" value="gemini-3.1-flash-lite">
        </div>
        <div class="field">
          <label id="ui_lang_lbl">Lang (src / dst)</label>
          <div class="row">
            <input type="text" id="source_lang" value="en" style="width: 48%">
            <input type="text" id="target_lang" value="tr" style="width: 48%">
          </div>
        </div>
      </div>
      <div class="grid" style="margin-top: 10px; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));">
        <div class="field">
          <label>Groq API Key <span class="val" id="groq_key_status"></span></label>
          <input type="password" id="groq_api_key" placeholder="Leave blank to keep existing">
        </div>
        <div class="field">
          <label>Gemini API Key <span class="val" id="gemini_key_status"></span></label>
          <input type="password" id="gemini_api_key" placeholder="Leave blank to keep existing">
        </div>
      </div>
      <div class="hint" id="ui_engine_hint">Style updates live. Engine/model/lang changes apply on restart.</div>
    </div>
  </div>

  <div class="actions">
    <span id="toast">Saved</span>
    <button class="btn btn-primary" id="ui_save_btn" onclick="saveConfig()">Save config.json</button>
  </div>
</div>

<script>
  const i18n = {
    en: {
      subtitle: 'Live overlay sync (127.0.0.1)',
      previewOff: 'Overlay Preview: OFF',
      previewOn: 'Overlay Preview: ON',
      stageDark: 'Dark',
      stageMid: 'Gray',
      stageBright: 'Light',
      nextSample: 'Next Sample',
      presetsLabel: 'Presets:',
      presetCinema: 'Cinema',
      presetYellow: 'Yellow',
      presetMinimal: 'Minimal',
      presetContrast: 'High Contrast',
      cardTypo: 'Typography',
      cardLayout: 'Container & Layout',
      cardEngine: 'Translation Engine (config.json)',
      engineLbl: 'Engine',
      langLbl: 'Lang (src / dst)',
      keyPlaceholder: 'Leave blank to keep existing',
      keySet: 'SET',
      keyEmpty: 'EMPTY',
      engineHint: 'Style updates live. Engine/model/lang changes apply on restart.',
      saveBtn: 'Save config.json',
      toastAutoSaved: 'Auto-saved (config.json)',
      toastSaved: 'Saved to config.json',
      toastDefault: 'Saved',
      samples: [
        "We need to get out of here before they find us. Stay low and follow me!",
        "Seriously, you can't recommend the same movie 24 times!",
        "Nobody notices, because it doesn't happen all at once.",
        "If this goes sideways, run and don't look back."
      ]
    },
    tr: {
      subtitle: 'Canlı overlay senkronizasyonu (127.0.0.1)',
      previewOff: 'Ekran Önizleme: KAPALI',
      previewOn: 'Ekran Önizleme: AÇIK',
      stageDark: 'Koyu',
      stageMid: 'Gri',
      stageBright: 'Açık',
      nextSample: 'Sonraki Örnek',
      presetsLabel: 'Hazır Stiller:',
      presetCinema: 'Sinema',
      presetYellow: 'Sarı',
      presetMinimal: 'Minimal',
      presetContrast: 'Yüksek Kontrast',
      cardTypo: 'Tipografi',
      cardLayout: 'Kutu ve Yerleşim',
      cardEngine: 'Çeviri Motoru (config.json)',
      engineLbl: 'Motor',
      langLbl: 'Dil (kaynak / hedef)',
      keyPlaceholder: 'Mevcut anahtarı korumak için boş bırakın',
      keySet: 'KAYITLI',
      keyEmpty: 'BOŞ',
      engineHint: 'Stil ayarları anlık güncellenir. Motor/model/dil değişiklikleri yeniden başlatmada geçerli olur.',
      saveBtn: 'config.json Kaydet',
      toastAutoSaved: 'Otomatik kaydedildi (config.json)',
      toastSaved: 'config.json kaydedildi',
      toastDefault: 'Kaydedildi',
      samples: [
        "Bizi bulmadan buradan çıkmamız lazım, alçaktan git ve beni takip et!",
        "Ama yuh artık, aynı filmi de 24 kez önermezsin!",
        "Kimse fark etmiyor, çünkü hepsi birden olmuyor.",
        "Eğer bu iş ters giderse, arkana bile bakmadan kaç."
      ]
    }
  };

  let uiLang = localStorage.getItem('overcap_ui_lang') === 'tr' ? 'tr' : 'en';
  let sampleIdx = 0;
  let screenTestOn = false;
  let previewTimer = null;
  let hasGroqKey = false;
  let hasGeminiKey = false;

  function currentSamples() {
    return i18n[uiLang].samples;
  }

  function applyUiLang() {
    const t = i18n[uiLang];
    document.documentElement.lang = uiLang;
    document.getElementById('langEn').classList.toggle('active', uiLang === 'en');
    document.getElementById('langTr').classList.toggle('active', uiLang === 'tr');

    document.getElementById('ui_subtitle').textContent = t.subtitle;
    document.getElementById('btnTestOverlay').textContent = screenTestOn ? t.previewOn : t.previewOff;
    document.getElementById('ui_stage_dark').textContent = t.stageDark;
    document.getElementById('ui_stage_mid').textContent = t.stageMid;
    document.getElementById('ui_stage_bright').textContent = t.stageBright;
    document.getElementById('ui_next_sample').textContent = t.nextSample;
    document.getElementById('ui_presets_label').textContent = t.presetsLabel;
    document.getElementById('ui_preset_cinema').textContent = t.presetCinema;
    document.getElementById('ui_preset_yellow').textContent = t.presetYellow;
    document.getElementById('ui_preset_minimal').textContent = t.presetMinimal;
    document.getElementById('ui_preset_contrast').textContent = t.presetContrast;
    document.getElementById('ui_card_typo').textContent = t.cardTypo;
    document.getElementById('ui_card_layout').textContent = t.cardLayout;
    document.getElementById('ui_card_engine').textContent = t.cardEngine;
    document.getElementById('ui_engine_lbl').textContent = t.engineLbl;
    document.getElementById('ui_lang_lbl').textContent = t.langLbl;
    document.getElementById('groq_api_key').placeholder = t.keyPlaceholder;
    document.getElementById('gemini_api_key').placeholder = t.keyPlaceholder;
    document.getElementById('groq_key_status').textContent = hasGroqKey ? t.keySet : t.keyEmpty;
    document.getElementById('gemini_key_status').textContent = hasGeminiKey ? t.keySet : t.keyEmpty;
    document.getElementById('ui_engine_hint').textContent = t.engineHint;
    document.getElementById('ui_save_btn').textContent = t.saveBtn;
    document.getElementById('toast').textContent = t.toastDefault;

    const sample = currentSamples()[sampleIdx % currentSamples().length];
    document.getElementById('previewBox').textContent = sample;
    if (screenTestOn) {
      fetch('/api/test-overlay', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({ show: true, text: sample })
      });
    }
  }

  function setUiLang(lang) {
    uiLang = lang === 'tr' ? 'tr' : 'en';
    localStorage.setItem('overcap_ui_lang', uiLang);
    applyUiLang();
  }

  applyUiLang();

  function setStage(cls, btn) {
    const st = document.getElementById('stage');
    st.className = 'preview-stage ' + cls;
    document.querySelectorAll('.stage-controls .chip').forEach(c => c.classList.remove('active'));
    btn.classList.add('active');
  }

  function cycleSample() {
    const samples = currentSamples();
    sampleIdx = (sampleIdx + 1) % samples.length;
    document.getElementById('previewBox').textContent = samples[sampleIdx];
    if (screenTestOn) {
      fetch('/api/test-overlay', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({ show: true, text: samples[sampleIdx] })
      });
    }
  }

  function setTextColor(hex) {
    document.getElementById('text_color').value = hex;
    onStyleChange();
  }

  function setBgColor(hex) {
    document.getElementById('bg_color').value = hex;
    onStyleChange();
  }

  function applyPreset(name) {
    const presets = {
      cinema:   { font_family: 'Bahnschrift', font_size: 30, font_bold: true,  text_color: '#FFFFFF', bg_color: '#0A0A0C', opacity: 225, corner_radius: 10, text_shadow: true,  fit_to_text: true,  padding_x: 18, padding_y: 8 },
      yellow:   { font_family: 'Trebuchet MS', font_size: 30, font_bold: true, text_color: '#FFE55C', bg_color: '#08080A', opacity: 220, corner_radius: 8,  text_shadow: true,  fit_to_text: true,  padding_x: 16, padding_y: 8 },
      minimal:  { font_family: 'Segoe UI', font_size: 28, font_bold: false,    text_color: '#F5F5F5', bg_color: '#121212', opacity: 185, corner_radius: 12, text_shadow: true,  fit_to_text: true,  padding_x: 14, padding_y: 6 },
      contrast: { font_family: 'Segoe UI', font_size: 32, font_bold: true,     text_color: '#FFFFFF', bg_color: '#000000', opacity: 250, corner_radius: 4,  text_shadow: false, fit_to_text: false, padding_x: 20, padding_y: 10 }
    };
    const p = presets[name];
    if (!p) return;
    document.getElementById('font_family').value = p.font_family;
    document.getElementById('font_size').value = p.font_size;
    document.getElementById('font_bold').checked = p.font_bold;
    document.getElementById('text_color').value = p.text_color;
    document.getElementById('bg_color').value = p.bg_color;
    document.getElementById('opacity').value = p.opacity;
    document.getElementById('corner_radius').value = p.corner_radius;
    document.getElementById('text_shadow').checked = p.text_shadow;
    document.getElementById('fit_to_text').checked = p.fit_to_text;
    document.getElementById('padding_x').value = p.padding_x;
    document.getElementById('padding_y').value = p.padding_y;
    onStyleChange();
  }

  function hexToRgba(hex, alpha255) {
    const c = hex.replace('#', '');
    const r = parseInt(c.substring(0,2), 16) || 10;
    const g = parseInt(c.substring(2,4), 16) || 10;
    const b = parseInt(c.substring(4,6), 16) || 12;
    return `rgba(${r}, ${g}, ${b}, ${(alpha255 / 255).toFixed(2)})`;
  }

  function collectStyle() {
    return {
      font_family: document.getElementById('font_family').value,
      font_size: parseInt(document.getElementById('font_size').value, 10),
      font_bold: document.getElementById('font_bold').checked,
      text_color: document.getElementById('text_color').value,
      bg_color: document.getElementById('bg_color').value,
      opacity: parseInt(document.getElementById('opacity').value, 10),
      corner_radius: parseInt(document.getElementById('corner_radius').value, 10),
      text_shadow: document.getElementById('text_shadow').checked,
      fit_to_text: document.getElementById('fit_to_text').checked,
      padding_x: parseInt(document.getElementById('padding_x').value, 10),
      padding_y: parseInt(document.getElementById('padding_y').value, 10)
    };
  }

  function renderLocalPreview(s) {
    document.getElementById('font_size_val').textContent = s.font_size + ' px';
    document.getElementById('opacity_val').textContent = Math.round((s.opacity / 255) * 100) + '%';
    document.getElementById('corner_radius_val').textContent = s.corner_radius + ' px';
    document.getElementById('padding_x_val').textContent = s.padding_x + ' px';
    document.getElementById('padding_y_val').textContent = s.padding_y + ' px';

    const box = document.getElementById('previewBox');
    box.style.fontFamily = `"${s.font_family}", sans-serif`;
    box.style.fontSize = s.font_size + 'px';
    box.style.fontWeight = s.font_bold ? '700' : '400';
    box.style.color = s.text_color;
    box.style.background = hexToRgba(s.bg_color, s.opacity);
    box.style.borderRadius = s.corner_radius + 'px';
    box.style.padding = `${s.padding_y}px ${s.padding_x}px`;
    box.style.width = s.fit_to_text ? 'auto' : '92%';
    box.style.textShadow = s.text_shadow ? '2px 2px 3px rgba(0,0,0,0.95)' : 'none';
  }

  function onStyleChange() {
    const s = collectStyle();
    renderLocalPreview(s);
    clearTimeout(previewTimer);
    previewTimer = setTimeout(() => {
      fetch('/api/preview', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify(s)
      }).then((r) => {
        if (r.ok) {
          const toast = document.getElementById('toast');
          toast.textContent = i18n[uiLang].toastAutoSaved;
          toast.style.opacity = '1';
          clearTimeout(window._toastTimer);
          window._toastTimer = setTimeout(() => { toast.style.opacity = '0'; }, 1400);
        }
      }).catch(() => {});
    }, 80);
  }

  function toggleScreenTest() {
    screenTestOn = !screenTestOn;
    const btn = document.getElementById('btnTestOverlay');
    const t = i18n[uiLang];
    btn.textContent = screenTestOn ? t.previewOn : t.previewOff;
    fetch('/api/test-overlay', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({ show: screenTestOn, text: currentSamples()[sampleIdx] })
    });
  }

  async function saveConfig() {
    const payload = {
      ...collectStyle(),
      engine: document.getElementById('engine').value,
      groq_model: document.getElementById('groq_model').value,
      gemini_model: document.getElementById('gemini_model').value,
      source_lang: document.getElementById('source_lang').value,
      target_lang: document.getElementById('target_lang').value,
      groq_api_key: document.getElementById('groq_api_key').value || null,
      gemini_api_key: document.getElementById('gemini_api_key').value || null
    };
    const r = await fetch('/api/save', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify(payload)
    });
    const toast = document.getElementById('toast');
    if (r.ok) {
      toast.textContent = i18n[uiLang].toastSaved;
      toast.style.opacity = '1';
      document.getElementById('groq_api_key').value = '';
      document.getElementById('gemini_api_key').value = '';
      setTimeout(() => { toast.style.opacity = '0'; }, 2500);
      loadConfig();
    }
  }

  async function loadConfig() {
    const r = await fetch('/api/config');
    const c = await r.json();
    document.getElementById('font_family').value = c.font_family || 'Segoe UI';
    document.getElementById('font_size').value = c.font_size || 28;
    document.getElementById('font_bold').checked = c.font_bold !== false;
    document.getElementById('text_color').value = c.text_color || '#FFFFFF';
    document.getElementById('bg_color').value = c.bg_color || '#0A0A0C';
    document.getElementById('opacity').value = c.opacity || 230;
    document.getElementById('corner_radius').value = c.corner_radius ?? 14;
    document.getElementById('text_shadow').checked = c.text_shadow !== false;
    document.getElementById('fit_to_text').checked = c.fit_to_text !== false;
    document.getElementById('padding_x').value = c.padding_x ?? 18;
    document.getElementById('padding_y').value = c.padding_y ?? 10;
    document.getElementById('engine').value = c.engine || 'google';
    document.getElementById('groq_model').value = c.groq_model || 'qwen/qwen3.8-27b';
    document.getElementById('gemini_model').value = c.gemini_model || 'gemini-3.1-flash-lite';
    document.getElementById('source_lang').value = c.source_lang || 'en';
    document.getElementById('target_lang').value = c.target_lang || 'tr';
    hasGroqKey = Boolean(c.has_groq_key);
    hasGeminiKey = Boolean(c.has_gemini_key);
    const t = i18n[uiLang];
    document.getElementById('groq_key_status').textContent = hasGroqKey ? t.keySet : t.keyEmpty;
    document.getElementById('gemini_key_status').textContent = hasGeminiKey ? t.keySet : t.keyEmpty;
    renderLocalPreview(collectStyle());
  }

  loadConfig();
</script>
</body>
</html>
"##;
