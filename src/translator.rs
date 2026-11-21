use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Her çağrının gerçek kaynağı saklanır; seçilen motor adı tek başına
/// Google yedeği veya önbellekten gelen bir sonucu doğru tanımlamaz.
#[derive(Clone, Debug, Default)]
pub struct TranslationReport {
    /// Başarılı sonuçta metni gerçekten üreten sağlayıcı; hatada son denenen sağlayıcı.
    pub engine: String,
    /// Google'da model seçimi olmadığı için bu alan boş bırakılır.
    pub model: String,
    /// Yedek kullanıldığında Google önbelleği de bu bayrağa dahildir.
    pub cached: bool,
    /// Kimlik bilgisi/sorgu içermeyen sabit Google adres etiketi.
    pub endpoint: Option<String>,
    /// Ana istek ve Google adres denemelerinin toplamı; önbellek HTTP isteği sayılmaz.
    pub attempts: usize,
    pub fallback: bool,
    /// Ana motorun tüm adres denemeleri dahil süresi; yedek ayrı ölçülür.
    pub primary_ms: u128,
    pub fallback_ms: u128,
}

pub trait Translator: Send {
    fn name(&self) -> &'static str;
    fn translate(&mut self, text: &str) -> Result<String>;

    /// Model ve son çağrı raporu, API anahtarı taşımayan durum verileridir.
    fn model(&self) -> &str {
        ""
    }
    fn report(&self) -> TranslationReport;

    /// Returns and clears a note about the last `translate` call, e.g. that the primary
    /// backend failed and the Google fallback was used. The worker surfaces it in the GUI.
    fn take_notice(&mut self) -> Option<String> {
        None
    }
}

pub struct GoogleTranslator {
    client: Client,
    source_lang: String,
    target_lang: String,
    cache: HashMap<String, String>,
    preferred_endpoint: usize,
    report: TranslationReport,
    /// Testler gerçek Google servisine ulaşmadan adres geçişlerini doğrular.
    #[cfg(test)]
    test_base_url: Option<String>,
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
            report: TranslationReport::default(),
            #[cfg(test)]
            test_base_url: None,
            preferred_endpoint: 1, // clients5 JSON is much faster than mobile HTML
        })
    }

    /// Üretimde adresler değişmez; yalnızca test derlemesi yerel sunucu kullanabilir.
    fn endpoint(&self, production: &str) -> String {
        #[cfg(test)]
        if let Some(base) = &self.test_base_url {
            let suffix = production
                .split_once(".com/")
                .map(|(_, p)| p)
                .unwrap_or("translate_a/single");
            return format!("{base}/{suffix}");
        }
        production.to_string()
    }

    fn try_mobile_web(&self, text: &str) -> Result<String> {
        let resp = self
            .client
            .get(self.endpoint("https://translate.google.com/m"))
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
            .get(self.endpoint("https://clients5.google.com/translate_a/t"))
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
                if let Some(inner) = first
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
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
            .get(self.endpoint("https://translate.googleapis.com/translate_a/single"))
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

    /// Rapor kopyası işçi tarafından log ve arayüze güvenle taşınır.
    fn report(&self) -> TranslationReport {
        self.report.clone()
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        // Önceki çağrının yedek/önbellek bilgisi yeni satıra sızmamalıdır.
        self.report = TranslationReport {
            engine: self.name().to_string(),
            model: self.model().to_string(),
            ..Default::default()
        };
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            self.report.cached = true;
            return Ok(cached.clone());
        }

        let order = [
            self.preferred_endpoint,
            (self.preferred_endpoint + 1) % 3,
            (self.preferred_endpoint + 2) % 3,
        ];

        let mut last_err = anyhow!("Translation failed");
        let started = Instant::now();
        for idx in order {
            // Yalnızca adresin sabit adı tutulur; sorgu ve kullanıcı metni URL olarak loglanmaz.
            self.report.attempts += 1;
            self.report.endpoint = Some(
                match idx {
                    0 => "mobile",
                    1 => "clients5",
                    _ => "gtx",
                }
                .into(),
            );
            let res = match idx {
                0 => self.try_mobile_web(key),
                1 => self.try_clients5(key),
                _ => self.try_gtx(key),
            };
            match res {
                Ok(result) if !result.is_empty() => {
                    self.report.primary_ms = started.elapsed().as_millis();
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

        self.report.primary_ms = started.elapsed().as_millis();
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
    fallback_notice: Option<String>,
    report: TranslationReport,
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
            fallback_notice: None,
            report: TranslationReport::default(),
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

    fn take_notice(&mut self) -> Option<String> {
        self.fallback_notice.take()
    }

    /// Rapor kopyası işçi tarafından log ve arayüze güvenle taşınır.
    fn report(&self) -> TranslationReport {
        self.report.clone()
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        // Önceki çağrının yedek/önbellek bilgisi yeni satıra sızmamalıdır.
        self.report = TranslationReport {
            engine: self.name().to_string(),
            model: self.model().to_string(),
            ..Default::default()
        };
        self.fallback_notice = None;
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            self.report.cached = true;
            return Ok(cached.clone());
        }

        let started = Instant::now();
        self.report.attempts = 1;
        let (translated, from_primary) = match self.call_gemini(key) {
            Ok(res) => {
                self.report.primary_ms = started.elapsed().as_millis();
                (res, true)
            }
            Err(e) => {
                eprintln!("[Gemini ERROR -> falling back to Google] {e}");
                let primary_ms = started.elapsed().as_millis();
                let fallback_started = Instant::now();
                let result = self.fallback.translate(key);
                self.report = self.fallback.report();
                self.report.fallback = true;
                self.report.attempts += 1;
                self.report.primary_ms = primary_ms;
                self.report.fallback_ms = fallback_started.elapsed().as_millis();
                // Yedek başarısızsa kullanıcıya çeviri yapılmış gibi bilgi verilmez.
                self.fallback_notice = Some(if result.is_ok() {
                    format!("{e} | Google fallback succeeded")
                } else {
                    format!("{e} | Google fallback also failed")
                });
                (result?, false)
            }
        };

        // Fallback results are shown but never cached or fed into the context history,
        // so a temporary API error cannot leave a lower-quality line stuck for the session.
        if from_primary && !translated.is_empty() {
            if self.cache.len() > 500 {
                self.cache.clear();
            }
            self.cache.insert(key.to_string(), translated.clone());
            if self.history.len() >= 3 {
                self.history.pop_front();
            }
            self.history
                .push_back((key.to_string(), translated.clone()));
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
    fallback_notice: Option<String>,
    report: TranslationReport,
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
            fallback_notice: None,
            report: TranslationReport::default(),
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

    fn take_notice(&mut self) -> Option<String> {
        self.fallback_notice.take()
    }

    /// Rapor kopyası işçi tarafından log ve arayüze güvenle taşınır.
    fn report(&self) -> TranslationReport {
        self.report.clone()
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        // Önceki çağrının yedek/önbellek bilgisi yeni satıra sızmamalıdır.
        self.report = TranslationReport {
            engine: self.name().to_string(),
            model: self.model().to_string(),
            ..Default::default()
        };
        self.fallback_notice = None;
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            self.report.cached = true;
            return Ok(cached.clone());
        }

        let started = Instant::now();
        self.report.attempts = 1;
        let (translated, from_primary) = match self.call_groq(key) {
            Ok(res) => {
                self.report.primary_ms = started.elapsed().as_millis();
                (res, true)
            }
            Err(e) => {
                eprintln!("[Groq ERROR -> falling back to Google] {e}");
                let primary_ms = started.elapsed().as_millis();
                let fallback_started = Instant::now();
                let result = self.fallback.translate(key);
                self.report = self.fallback.report();
                self.report.fallback = true;
                self.report.attempts += 1;
                self.report.primary_ms = primary_ms;
                self.report.fallback_ms = fallback_started.elapsed().as_millis();
                // Yedek başarısızsa kullanıcıya çeviri yapılmış gibi bilgi verilmez.
                self.fallback_notice = Some(if result.is_ok() {
                    format!("{e} | Google fallback succeeded")
                } else {
                    format!("{e} | Google fallback also failed")
                });
                (result?, false)
            }
        };

        // Fallback results are shown but never cached or fed into the context history,
        // so a temporary API error cannot leave a lower-quality line stuck for the session.
        if from_primary && !translated.is_empty() {
            if self.cache.len() > 500 {
                self.cache.clear();
            }
            self.cache.insert(key.to_string(), translated.clone());
            if self.history.len() >= 3 {
                self.history.pop_front();
            }
            self.history
                .push_back((key.to_string(), translated.clone()));
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
    fallback_notice: Option<String>,
    report: TranslationReport,
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
            fallback_notice: None,
            report: TranslationReport::default(),
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

    fn take_notice(&mut self) -> Option<String> {
        self.fallback_notice.take()
    }

    /// Rapor kopyası işçi tarafından log ve arayüze güvenle taşınır.
    fn report(&self) -> TranslationReport {
        self.report.clone()
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn translate(&mut self, text: &str) -> Result<String> {
        // Önceki çağrının yedek/önbellek bilgisi yeni satıra sızmamalıdır.
        self.report = TranslationReport {
            engine: self.name().to_string(),
            model: self.model().to_string(),
            ..Default::default()
        };
        self.fallback_notice = None;
        let key = text.trim();
        if key.is_empty() {
            return Ok(String::new());
        }
        if let Some(cached) = self.cache.get(key) {
            self.report.cached = true;
            return Ok(cached.clone());
        }

        let started = Instant::now();
        self.report.attempts = 1;
        let (translated, from_primary) = match self.call_endpoint(key) {
            Ok(res) => {
                self.report.primary_ms = started.elapsed().as_millis();
                (res, true)
            }
            Err(e) => {
                eprintln!("[OpenAI-Compat ERROR -> falling back to Google] {e}");
                let primary_ms = started.elapsed().as_millis();
                let fallback_started = Instant::now();
                let result = self.fallback.translate(key);
                self.report = self.fallback.report();
                self.report.fallback = true;
                self.report.attempts += 1;
                self.report.primary_ms = primary_ms;
                self.report.fallback_ms = fallback_started.elapsed().as_millis();
                // Yedek başarısızsa kullanıcıya çeviri yapılmış gibi bilgi verilmez.
                self.fallback_notice = Some(if result.is_ok() {
                    format!("{e} | Google fallback succeeded")
                } else {
                    format!("{e} | Google fallback also failed")
                });
                (result?, false)
            }
        };

        // Fallback results are shown but never cached or fed into the context history,
        // so a temporary API error cannot leave a lower-quality line stuck for the session.
        if from_primary && !translated.is_empty() {
            if self.cache.len() > 500 {
                self.cache.clear();
            }
            self.cache.insert(key.to_string(), translated.clone());
            if self.history.len() >= 3 {
                self.history.pop_front();
            }
            self.history
                .push_back((key.to_string(), translated.clone()));
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
            let parsed = if let Some(hex) = code_str
                .strip_prefix('x')
                .or_else(|| code_str.strip_prefix('X'))
            {
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
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
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
            if w_prev.iter().any(|wp| {
                wp == wc || (wc.len() >= 4 && wp.len() >= 4 && text_similarity(wc, wp) >= 0.78)
            }) {
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

#[cfg(test)]
mod report_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Yerel HTTP yanıtları gerçek çeviri/yeniden deneme yollarını dış ağa çıkmadan çalıştırır.
    fn server(responses: Vec<(u16, &'static str)>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            for (status, body) in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Test request did not arrive");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                // Windows kabul edilen sokette dinleyicinin kilitlemeyen kipini
                // devralabilir; gövdenin tamamı gelmeden okuma hata vermemelidir.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let size = stream.read(&mut buffer).unwrap();
                    if size == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..size]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (address, worker)
    }

    fn google_at(base: String) -> GoogleTranslator {
        let mut translator = GoogleTranslator::new("en", "tr").unwrap();
        translator.client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        translator.test_base_url = Some(base);
        translator
    }

    /// Başarılı alternatif adresin seçimi ile önbelleğin sıfır denemesi birlikte doğrulanır.
    #[test]
    fn google_records_endpoint_attempts_and_cache_without_stale_metadata() {
        let (base, worker) = server(vec![
            (500, "{}"),
            (200, r#"[[["Merhaba"]]]"#),
            (200, r#"[[["Dünya"]]]"#),
        ]);
        let mut translator = google_at(base);
        assert_eq!(translator.translate("Hello").unwrap(), "Merhaba");
        assert_eq!(translator.report().endpoint.as_deref(), Some("gtx"));
        assert_eq!(translator.report().attempts, 2);
        assert_eq!(translator.translate("World").unwrap(), "Dünya");
        assert_eq!(translator.report().attempts, 1);
        assert_eq!(translator.translate("Hello").unwrap(), "Merhaba");
        assert!(translator.report().cached);
        assert_eq!(translator.report().attempts, 0);
        assert!(translator.report().endpoint.is_none());
        worker.join().unwrap();
    }

    /// Ana motor hatasından sonra Google sonucu ana motor önbelleğine/bağlamına giremez.
    #[test]
    fn fallback_reports_real_provider_and_does_not_pollute_primary_history() {
        let (base, worker) = server(vec![
            (500, "{}"),
            (200, r#"["Merhaba"]"#),
            (200, r#"{"choices":[{"message":{"content":"Selam"}}]}"#),
        ]);
        let mut translator =
            OpenAiCompatTranslator::new(base.clone(), None, "same-model".into(), "en", "tr")
                .unwrap();
        translator.client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        translator.fallback = google_at(base);
        assert_eq!(translator.translate("Hello").unwrap(), "Merhaba");
        let report = translator.report();
        assert_eq!(report.engine, "google");
        assert!(report.fallback);
        assert_eq!(report.attempts, 2);
        assert!(translator.cache.is_empty());
        assert!(translator.history.is_empty());
        assert!(translator.take_notice().unwrap().contains("succeeded"));
        assert_eq!(translator.translate("Hello").unwrap(), "Selam");
        assert_eq!(translator.report().engine, "openai-compat");
        assert_eq!(translator.report().model, "same-model");
        assert!(!translator.report().fallback);
        assert_eq!(translator.history.len(), 1);
        assert_eq!(translator.translate("Hello").unwrap(), "Selam");
        assert!(translator.report().cached);
        assert_eq!(translator.report().attempts, 0);
        worker.join().unwrap();
    }

    /// Her iki taraf da hatalıysa başarı bildirimi ve önbellek kaydı üretilmez.
    #[test]
    fn failed_fallback_is_reported_as_failure() {
        let (base, worker) = server(vec![(500, "{}"), (500, "{}"), (500, "{}"), (500, "{}")]);
        let mut translator =
            OpenAiCompatTranslator::new(base.clone(), None, "model".into(), "en", "tr").unwrap();
        translator.client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        translator.fallback = google_at(base);
        assert!(translator.translate("Hello").is_err());
        assert!(translator.take_notice().unwrap().contains("also failed"));
        assert_eq!(translator.report().attempts, 4);
        assert!(translator.history.is_empty());
        assert!(translator.cache.is_empty());
        worker.join().unwrap();
    }

    /// Üç LLM motorunun da Google önbelleği ve sonrasında kendi önbelleği doğru etiketlenir.
    #[test]
    fn all_llm_backends_clear_fallback_metadata_on_the_next_call() {
        let mut gemini =
            GeminiTranslator::new("invalid\nkey".into(), "gemini-model".into(), "en", "tr")
                .unwrap();
        let mut groq =
            GroqTranslator::new("invalid\nkey".into(), "qwen-model".into(), "en", "tr").unwrap();
        let mut custom = OpenAiCompatTranslator::new(
            "http://127.0.0.1:1".into(),
            Some("invalid\nkey".into()),
            "custom-model".into(),
            "en",
            "tr",
        )
        .unwrap();
        gemini
            .fallback
            .cache
            .insert("Hello".into(), "Merhaba".into());
        groq.fallback.cache.insert("Hello".into(), "Merhaba".into());
        custom
            .fallback
            .cache
            .insert("Hello".into(), "Merhaba".into());
        gemini.cache.insert("Goodbye".into(), "Hoşça kal".into());
        groq.cache.insert("Goodbye".into(), "Hoşça kal".into());
        custom.cache.insert("Goodbye".into(), "Hoşça kal".into());
        let mut engines: Vec<Box<dyn Translator>> =
            vec![Box::new(gemini), Box::new(groq), Box::new(custom)];
        for engine in &mut engines {
            assert_eq!(engine.translate("Hello").unwrap(), "Merhaba");
            assert!(engine.report().fallback);
            assert!(engine.report().cached);
            assert_eq!(engine.report().engine, "google");
            assert_eq!(engine.report().attempts, 1);
            assert_eq!(engine.translate("Goodbye").unwrap(), "Hoşça kal");
            assert_eq!(engine.report().engine, engine.name());
            assert!(!engine.report().fallback);
            assert!(engine.report().cached);
            assert_eq!(engine.report().attempts, 0);
            assert!(engine.take_notice().is_none());
        }
    }

    /// İki satırlık mevcut bağlam davranışı kalitenin korunması için sabitlenir.
    #[test]
    fn prompt_keeps_the_last_two_context_lines() {
        let history = VecDeque::from([
            ("one".into(), "bir".into()),
            ("two".into(), "iki".into()),
            ("three".into(), "üç".into()),
        ]);
        assert_eq!(
            format_user_subtitle_prompt(&history, "four"),
            "Context:\ntwo=>iki\nthree=>üç\nText: four"
        );
    }
}
