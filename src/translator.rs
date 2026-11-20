use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

pub trait Translator: Send {
    fn name(&self) -> &'static str;
    fn translate(&mut self, text: &str) -> Result<String>;
}

pub struct GoogleTranslator {
    client: Client,
    source_lang: String,
    target_lang: String,
    cache: HashMap<String, String>,
    preferred_endpoint: usize,
}

impl GoogleTranslator {
    pub fn new(source_lang: &str, target_lang: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(4))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .build()?;

        Ok(Self {
            client,
            source_lang: source_lang.to_string(),
            target_lang: target_lang.to_string(),
            cache: HashMap::new(),
            preferred_endpoint: 1, // clients5 JSON is much faster than mobile HTML
        })
    }

    fn try_mobile_web(&self, text: &str) -> Result<String> {
        let resp = self
            .client
            .get("https://translate.google.com/m")
            .query(&[
                ("sl", self.source_lang.as_str()),
                ("tl", self.target_lang.as_str()),
                ("hl", self.target_lang.as_str()),
                ("q", text),
            ])
            .send()?;

        if !resp.status().is_success() {
            return Err(anyhow!("google/m HTTP {}", resp.status()));
        }

        let html = resp.text()?;
        let marker = "class=\"result-container\">";
        let start = html
            .find(marker)
            .ok_or_else(|| anyhow!("google/m translation not found in response"))?
            + marker.len();
        let rest = &html[start..];
        let end = rest
            .find("</div>")
            .ok_or_else(|| anyhow!("google/m response incomplete"))?;

        Ok(decode_html_entities(&rest[..end]).trim().to_string())
    }

    fn try_clients5(&self, text: &str) -> Result<String> {
        let resp = self
            .client
            .get("https://clients5.google.com/translate_a/t")
            .query(&[
                ("client", "dict-chrome-ex"),
                ("sl", self.source_lang.as_str()),
                ("tl", self.target_lang.as_str()),
                ("q", text),
            ])
            .send()?;

        if !resp.status().is_success() {
            return Err(anyhow!("clients5 HTTP {}", resp.status()));
        }

        let json: Value = resp.json()?;
        // Response can be ["translated"] or [["translated", "src"]]
        if let Some(arr) = json.as_array() {
            if let Some(first) = arr.first() {
                if let Some(s) = first.as_str() {
                    return Ok(s.trim().to_string());
                }
                if let Some(inner) = first.as_array().and_then(|a| a.first()).and_then(|v| v.as_str())
                {
                    return Ok(inner.trim().to_string());
                }
            }
        }
        Err(anyhow!("failed to parse clients5 response"))
    }

    fn try_gtx(&self, text: &str) -> Result<String> {
        let resp = self
            .client
            .get("https://translate.googleapis.com/translate_a/single")
            .query(&[
                ("client", "gtx"),
                ("sl", self.source_lang.as_str()),
                ("tl", self.target_lang.as_str()),
                ("dt", "t"),
                ("q", text),
            ])
            .send()?;

        if !resp.status().is_success() {
            return Err(anyhow!("gtx HTTP {}", resp.status()));
        }

        let json: Value = resp.json()?;
        let mut translated = String::new();
        if let Some(segments) = json.get(0).and_then(|v| v.as_array()) {
            for seg in segments {
                if let Some(piece) = seg.get(0).and_then(|v| v.as_str()) {
                    translated.push_str(piece);
                }
            }
        }
        let result = translated.trim().to_string();
        if result.is_empty() {
            return Err(anyhow!("gtx returned empty response"));
        }
        Ok(result)
    }
}

impl Translator for GoogleTranslator {
    fn name(&self) -> &'static str {
        "google"
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            return Ok(cached.clone());
        }

        let order = [
            self.preferred_endpoint,
            (self.preferred_endpoint + 1) % 3,
            (self.preferred_endpoint + 2) % 3,
        ];

        let mut last_err = anyhow!("Translation failed");
        for idx in order {
            let res = match idx {
                0 => self.try_mobile_web(key),
                1 => self.try_clients5(key),
                _ => self.try_gtx(key),
            };
            match res {
                Ok(result) if !result.is_empty() => {
                    self.preferred_endpoint = idx;
                    if self.cache.len() > 500 {
                        self.cache.clear();
                    }
                    self.cache.insert(key.to_string(), result.clone());
                    return Ok(result);
                }
                Ok(_) => {}
                Err(e) => {
                    last_err = e;
                }
            }
        }

        Err(last_err)
    }
}

fn build_subtitle_prompt_instructions(source_lang: &str, target_lang: &str) -> String {
    let style_hint = if target_lang.eq_ignore_ascii_case("tr") {
        "\nStyle examples:\n\
         - You know what? => Biliyor musun?\n\
         - I don't care. => Umurumda değil.\n\
         - You have no idea what you're walking into. => Neyin içine girdiğinin farkında bile değilsin.\n\
         - You can't talk me out of this. => Beni bundan vazgeçiremezsin."
    } else {
        "\nStyle: conversational, idiomatic spoken dialogue rather than stiff or textbook phrasing."
    };

    format!(
        "You are a professional film and video game subtitle translator ({source_lang} -> {target_lang}).\n\
         Rules:\n\
         - Translate naturally and colloquially; do NOT translate literally or overly formally.\n\
         - Translate slang and profanity naturally and uncensored.\n\
         - The input is live OCR output: silently fix obvious OCR character glitches (e.g. 1'11 -> I'll, l/1/I swaps) from context.\n\
         - Preserve speaker prefixes if present (e.g. 'Ellie:').\n\
         - Output ONLY the translation with no explanations or extra quotes.{style_hint}"
    )
}

fn format_user_subtitle_prompt(history: &VecDeque<(String, String)>, text: &str) -> String {
    let mut msg = String::new();
    if !history.is_empty() {
        msg.push_str("Context:\n");
        for (src, dst) in history.iter().rev().take(2).rev() {
            msg.push_str(&format!("{}=>{}\n", src, dst));
        }
    }
    msg.push_str(&format!("Text: {}", text));
    msg
}

pub struct GeminiTranslator {
    client: Client,
    api_key: String,
    model: String,
    source_lang: String,
    target_lang: String,
    cache: HashMap<String, String>,
    history: VecDeque<(String, String)>,
    fallback: GoogleTranslator,
}

impl GeminiTranslator {
    pub fn new(
        api_key: String,
        model: String,
        source_lang: &str,
        target_lang: &str,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(2200))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;

        let fallback = GoogleTranslator::new(source_lang, target_lang)?;

        Ok(Self {
            client,
            api_key,
            model,
            source_lang: source_lang.to_string(),
            target_lang: target_lang.to_string(),
            cache: HashMap::new(),
            history: VecDeque::with_capacity(4),
            fallback,
        })
    }

    fn call_gemini(&self, text: &str) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );

        let instructions = build_subtitle_prompt_instructions(&self.source_lang, &self.target_lang);
        let user_text = format_user_subtitle_prompt(&self.history, text);
        let prompt = format!("{instructions}\n\n{user_text}");

        let thinking_cfg = if self.model.contains("gemini-3") {
            json!({ "thinkingLevel": "minimal" })
        } else {
            json!({ "thinkingBudget": 0 })
        };

        let body = json!({
            "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "temperature": 0.15,
                "maxOutputTokens": 90,
                "thinkingConfig": thinking_cfg
            }
        });

        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_txt = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "Gemini HTTP {} ({})",
                status,
                err_txt.chars().take(160).collect::<String>()
            ));
        }

        let json: Value = resp.json()?;
        let out = json
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Gemini response contained no text"))?
            .trim()
            .trim_matches('"')
            .to_string();

        if out.is_empty() {
            return Err(anyhow!("Gemini returned empty translation"));
        }

        Ok(out)
    }
}

impl Translator for GeminiTranslator {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            return Ok(cached.clone());
        }

        let translated = match self.call_gemini(key) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("[Gemini ERROR -> falling back to Google] {e}");
                self.fallback.translate(key)?
            }
        };

        if !translated.is_empty() {
            if self.cache.len() > 500 {
                self.cache.clear();
            }
            self.cache.insert(key.to_string(), translated.clone());
            if self.history.len() >= 3 {
                self.history.pop_front();
            }
            self.history.push_back((key.to_string(), translated.clone()));
        }

        Ok(translated)
    }
}

pub struct GroqTranslator {
    client: Client,
    api_key: String,
    model: String,
    source_lang: String,
    target_lang: String,
    cache: HashMap<String, String>,
    history: VecDeque<(String, String)>,
    fallback: GoogleTranslator,
}

impl GroqTranslator {
    pub fn new(
        api_key: String,
        model: String,
        source_lang: &str,
        target_lang: &str,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(2500))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;

        let fallback = GoogleTranslator::new(source_lang, target_lang)?;

        Ok(Self {
            client,
            api_key,
            model,
            source_lang: source_lang.to_string(),
            target_lang: target_lang.to_string(),
            cache: HashMap::new(),
            history: VecDeque::with_capacity(4),
            fallback,
        })
    }

    fn call_groq(&self, text: &str) -> Result<String> {
        let sys = build_subtitle_prompt_instructions(&self.source_lang, &self.target_lang);
        let user_msg = format_user_subtitle_prompt(&self.history, text);

        let mut body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": sys },
                { "role": "user", "content": user_msg }
            ],
            "temperature": 0.30,
            "max_tokens": 100
        });

        // Disable thinking on Qwen3 models for ultra-low latency
        if self.model.to_lowercase().contains("qwen") {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("reasoning_effort".to_string(), json!("none"));
                obj.insert("reasoning_format".to_string(), json!("hidden"));
            }
        }

        let resp = self
            .client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_txt = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "Groq HTTP {} ({})",
                status,
                err_txt.chars().take(160).collect::<String>()
            ));
        }

        let json: Value = resp.json()?;
        let raw = json
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Groq response contained no text"))?;

        let cleaned = strip_think_tags(raw).trim().trim_matches('"').to_string();
        if cleaned.is_empty() {
            return Err(anyhow!("Groq returned empty translation"));
        }

        Ok(cleaned)
    }
}

impl Translator for GroqTranslator {
    fn name(&self) -> &'static str {
        "groq"
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            return Ok(cached.clone());
        }

        let translated = match self.call_groq(key) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("[Groq ERROR -> falling back to Google] {e}");
                self.fallback.translate(key)?
            }
        };

        if !translated.is_empty() {
            if self.cache.len() > 500 {
                self.cache.clear();
            }
            self.cache.insert(key.to_string(), translated.clone());
            if self.history.len() >= 3 {
                self.history.pop_front();
            }
            self.history.push_back((key.to_string(), translated.clone()));
        }

        Ok(translated)
    }
}

pub struct OpenAiCompatTranslator {
    client: Client,
    endpoint_url: String,
    api_key: Option<String>,
    model: String,
    source_lang: String,
    target_lang: String,
    cache: HashMap<String, String>,
    history: VecDeque<(String, String)>,
    fallback: GoogleTranslator,
}

impl OpenAiCompatTranslator {
    /// Creates a translator targeting any OpenAI-compatible `/chat/completions` endpoint.
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        model: String,
        source_lang: &str,
        target_lang: &str,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(6))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;

        let trimmed = base_url.trim().trim_end_matches('/');
        let endpoint_url = if trimmed.ends_with("/chat/completions") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/chat/completions")
        };

        let api_key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());

        let fallback = GoogleTranslator::new(source_lang, target_lang)?;

        Ok(Self {
            client,
            endpoint_url,
            api_key,
            model,
            source_lang: source_lang.to_string(),
            target_lang: target_lang.to_string(),
            cache: HashMap::new(),
            history: VecDeque::with_capacity(4),
            fallback,
        })
    }

    fn call_endpoint(&self, text: &str) -> Result<String> {
        let sys = build_subtitle_prompt_instructions(&self.source_lang, &self.target_lang);
        let user_msg = format_user_subtitle_prompt(&self.history, text);

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": sys },
                { "role": "user", "content": user_msg }
            ],
            "temperature": 0.30,
            "max_tokens": 100
        });

        let mut req = self.client.post(&self.endpoint_url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send()?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_txt = resp.text().unwrap_or_default();
            return Err(anyhow!(
                "OpenAI-compatible HTTP {} ({})",
                status,
                err_txt.chars().take(160).collect::<String>()
            ));
        }

        let json: Value = resp.json()?;
        let raw = json
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("OpenAI-compatible response contained no text"))?;

        let cleaned = strip_think_tags(raw).trim().trim_matches('"').to_string();
        if cleaned.is_empty() {
            return Err(anyhow!(
                "OpenAI-compatible endpoint returned empty translation"
            ));
        }

        Ok(cleaned)
    }
}

impl Translator for OpenAiCompatTranslator {
    fn name(&self) -> &'static str {
        "openai-compat"
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            return Ok(cached.clone());
        }

        let translated = match self.call_endpoint(key) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("[OpenAI-Compat ERROR -> falling back to Google] {e}");
                self.fallback.translate(key)?
            }
        };

        if !translated.is_empty() {
            if self.cache.len() > 500 {
                self.cache.clear();
            }
            self.cache.insert(key.to_string(), translated.clone());
            if self.history.len() >= 3 {
                self.history.pop_front();
            }
            self.history.push_back((key.to_string(), translated.clone()));
        }

        Ok(translated)
    }
}

fn strip_think_tags(s: &str) -> String {
    if let Some(end) = s.find("</think>") {
        return s[end + "</think>".len()..].to_string();
    }
    s.to_string()
}

fn decode_html_entities(s: &str) -> String {
    let mut out = s
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");

    // Decode numeric decimal entities like &#231; or &#305;
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(pos) = rest.find("&#") {
        result.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        if let Some(semi) = after.find(';') {
            let code_str = &after[..semi];
            let parsed = if let Some(hex) = code_str.strip_prefix('x').or_else(|| code_str.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                code_str.parse::<u32>().ok()
            };
            if let Some(ch) = parsed.and_then(char::from_u32) {
                result.push(ch);
                rest = &after[semi + 1..];
                continue;
            }
        }
        result.push_str("&#");
        rest = after;
    }
    result.push_str(rest);
    out = result;
    out
}

/// Normalized Levenshtein similarity in [0.0, 1.0].
pub fn text_similarity(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.to_lowercase().chars().collect();
    let b_chars: Vec<char> = b.to_lowercase().chars().collect();

    if a_chars.is_empty() && b_chars.is_empty() {
        return 1.0;
    }
    let max_len = a_chars.len().max(b_chars.len());
    if max_len == 0 {
        return 1.0;
    }

    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0usize; b_chars.len() + 1];

    for (i, &ca) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1)
                .min(curr[j] + 1)
                .min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    let dist = prev[b_chars.len()];
    1.0 - (dist as f64 / max_len as f64)
}

fn extract_clean_words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|w| w.to_string())
        .collect()
}

/// Smart subtitle duplicate detector: combines Levenshtein similarity with word-overlap
/// so noisy middle-word OCR glitches on a static subtitle don't re-trigger translation.
pub fn is_same_subtitle(curr: &str, prev: &str) -> bool {
    if curr.eq_ignore_ascii_case(prev) {
        return true;
    }
    if text_similarity(curr, prev) >= 0.80 {
        return true;
    }

    let w_curr = extract_clean_words(curr);
    let w_prev = extract_clean_words(prev);
    let min_words = w_curr.len().min(w_prev.len());
    let max_words = w_curr.len().max(w_prev.len());

    if min_words >= 3 && max_words <= min_words + 3 {
        let mut matches = 0usize;
        for wc in &w_curr {
            if w_prev
                .iter()
                .any(|wp| wp == wc || (wc.len() >= 4 && wp.len() >= 4 && text_similarity(wc, wp) >= 0.78))
            {
                matches += 1;
            }
        }
        let overlap = matches as f64 / min_words as f64;
        if overlap >= 0.65 {
            return true;
        }
    }

    false
}
