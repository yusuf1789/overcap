# Overcap

[English documentation: README.md](README.md)

Windows için genel amaçlı, düşük gecikmeli canlı oyun ve video altyazı çevirici.

Ekranda seçilen altyazı bölgesini yerleşik `Windows.Media.Ocr` ile okur, seçilen çeviri motoruyla (`Groq` / `Gemini` / `Google Translate`) çevirir ve çevrilmiş metni orijinal altyazının **tam üstüne** yakalamadan hariç tutma bayraklı (`WDA_EXCLUDEFROMCAPTURE`) şeffaf bir Win32 overlay penceresiyle çizer.

Herhangi bir PC oyunu (Borderless Windowed), konsol ekran aktarımı (Xbox Remote Play / HDMI Capture) veya video oynatıcı ile oyun-bağımsız çalışır.

---

## Temel Özellikler

- **Tek `.exe` (~4.2 MB, Rust):** Harici runtime veya harici OCR modeli gerektirmez.
- **Altyazının Tam Üstüne Yazma (`WDA_EXCLUDEFROMCAPTURE`):**
  - Overlay penceresi kullanıcıya görünür, ancak ekran yakalayıcıya karşı görünmezdir; bu sayede OCR alttaki orijinal altyazıyı kesintisiz okumaya devam eder.
  - `WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` ile tıklamaları alttaki oyuna geçirir, odak çalmaz ve Alt+Tab listesinde görünmez.
  - Altyazı kaybolduğunda otomatik gizlenir.
- **OCR Bounding Box Örtme & Auto-Fit:**
  - `OcrWord.BoundingRect` koordinatlarıyla alttaki orijinal altyazının gerçek piksel alanını hesaplar.
  - Orijinal altyazı 2 satır, çeviri 1 satır olsa bile alttaki metni tamamen kapatır; kısa cümlelerde gereksiz siyah şerit bırakmaz.
- **Akıllı Değişim ve Tekrar Filtresi:**
  - Parlak piksel maskesi (`luma >= 170`) ile kamera/arka plan hareketlerinde gereksiz OCR tetiklenmesini engeller.
  - Levenshtein + kelime örtüşmesi (word-overlap) ile gürültülü OCR karelerinde aynı cümleyi tekrar API'ye göndermez ve ekranda titreme yapmaz.
- **Çoklu Çeviri Motoru (`Translator` trait):**
  - **Groq (`qwen/qwen3.8-27b`, önerilen):** `~150-280 ms` toplam gecikme, bağlamlı ve günlük/argo uyumlu oyun çevirisi (`reasoning_effort: "none"`).
  - **Google Gemini (`gemini-3.1-flash-lite`):** Bağlamlı ve OCR hata düzeltmeli çeviri.
  - **OpenAI-Uyumlu / Yerel LLM'ler (`openai-compat` / `ollama`):** Ollama, LM Studio, vLLM veya herhangi bir `/chat/completions` uç noktasına bağlanın (`--custom-base-url`, `--custom-model`, `--custom-key`).
  - **Google Translate:** API anahtarsız ücretsiz seçenek ve diğer motorlar için otomatik yedek (fallback).
- **Otomatik Uygulama Profilleri:**
  - `Ctrl + Shift + S` seçilen altyazı bölgesini aktif `.exe` profiline kaydeder; oyunlar veya video oynatıcılar arasında geçiş yaptığınızda Overcap bölgeyi otomatik değiştirir (`--no-auto-profiles` ile veya Ayar Panelinden kapatılabilir).
- **Yerel Masaüstü Kontrol Merkezi (`egui`) & Sistem Tepsisi:**
  - Konsol penceresi olmayan Windows masaüstü uygulaması (`Ctrl + Shift + A` veya Sistem Tepsisi ikonu ile açılır/odaklanır; pencereyi kapatmak uygulamayı Sistem Tepsisine küçültür).
  - Font Family, Font Size, Bold, Text Color, Text Shadow, Background Color, Opacity, Border Radius, Padding X/Y, Auto-Fit, Engine (canlı yeniden yükleme ile), dil ayarları, **OCR Tanılama**, **Uygulama Profilleri** ve **Canlı Kayıt** sekmesi canlı önizlemeyle yapılır ve otomatik `config.json` dosyasına kaydedilir.
- **High-DPI & Çoklu Monitör:** `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` ile `%125` / `%150` Windows ölçeklemelerinde sıfır koordinat kayması.

---

## Ölçülen Gerçek Performans

PC'de Borderless Windowed oyun/video testlerinde (Groq `qwen/qwen3.8-27b`):

| Aşama | Süre |
|---|---|
| GDI `BitBlt` + parlak piksel farkı | `2 - 6 ms` |
| 2x Upscale + `Windows.Media.Ocr` | `20 - 50 ms` |
| Groq (`qwen/qwen3.8-27b`, kalıcı TLS) | `130 - 250 ms` |
| **Toplam (yakalama -> ekranda çeviri)** | **`~160 - 300 ms`** |

*Not: Aynı metin önbellekten döndüğünde toplam süre `~30 - 55 ms` civarındadır.*

---

## Kurulum ve Çalıştırma

### 1. Ön Gereksinim (Önerilen)
İngilizce oyunlarda en yüksek OCR doğruluğu için Windows'ta İngilizce OCR dil paketinin kurulu olması önerilir:
`Windows Ayarlar -> Zaman ve Dil -> Dil ve Bölge -> Dil ekle -> English (United States)` (Windows görüntüleme dilini değiştirmenize gerek yoktur; sadece "Optik karakter tanıma" özelliği yeterlidir).

### 2. Derleme (Kaynak Koddan)
Rust (`msvc` toolchain) kuruluysa:

```powershell
cargo build --release
```

Çıktı: `target\release\overcap.exe` (statik CRT ile derlenmiş tek taşınabilir `.exe`; `config.json` çalıştırılabilir dosyanın yanında saklanır).

> İmzasız dosya notu: Releases üzerinden hazır `overcap.exe` indirildiğinde Windows SmartScreen ilk açılışta uyarı gösterebilir ("Ek bilgi" -> "Yine de çalıştır"). Dilerseniz kodu inceleyip doğrudan `cargo build --release` ile kendiniz derleyebilirsiniz.

### 3. Çalıştırma

```powershell
.\start.bat
```

veya doğrudan:

```powershell
.\target\release\overcap.exe
```

İlk çalıştırmada ekran kararır; fareyle altyazının çıktığı şeridi kutu içine alın. Koordinatlar `config.json` dosyasına kaydedilir.

### CLI Parametreleri

```powershell
# Altyazı bölgesini yeniden seçerek başlat
.\start.bat --select

# Sistem Tepsisinde gizli başlat
.\target\release\overcap.exe --minimized

# Groq ile başlat (anahtar ve motor config.json'a kaydedilir)
.\start.bat --engine groq --groq-key gsk_...

# Gemini ile başlat
.\start.bat --engine gemini --gemini-key AIza...

# Farklı dil çifti (varsayılan: en -> tr)
.\start.bat --from en --to tr

# Özel tarama aralığı (ms)
.\start.bat --interval-ms 100
```

Örnek yapılandırma dosyası için [config.example.json](config.example.json) dosyasına bakabilirsiniz. `config.json` `.gitignore` içerisindedir ve API anahtarlarınız repoya dahil edilmez.

---

## Oyun İçi Global Kısayollar

| Kısayol | İşlev |
|---|---|
| `Ctrl + Shift + A` | Overcap kontrol merkezi penceresini aç / odakla |
| `Ctrl + Shift + S` | Altyazı bölgesini anında yeniden seç |
| `Ctrl + Shift + H` | Overlay'i geçici gizle / duraklat |
| `Ctrl + Shift + Yukarı` | Yazı boyutunu büyüt (+2 px) |
| `Ctrl + Shift + Aşağı` | Yazı boyutunu küçült (-2 px) |
| `Ctrl + Shift + Q` | Uygulamayı kapat |

---

## Oyun İçi İpuçları

- Oyunu **Borderless Windowed (Çerçevesiz Tam Ekran)** modunda çalıştırın (Exclusive Fullscreen'de Windows overlay pencereleri oyunun arkasında kalır).
- Oyunun kendi altyazı ayarlarında **Subtitle Background (Koyu/Yarı Şeffaf Arka Plan)** seçeneği varsa açmanız OCR doğruluğunu belirgin şekilde artırır.

---

## Proje Yapısı

- `Cargo.toml` — Rust proje ve Windows API bağımlılıkları.
- `src/main.rs` — CLI, asenkron OCR ve çeviri worker koordinasyonu.
- `src/capture.rs` — GDI `BitBlt` bölge yakalama, parlak piksel değişim maskesi ve 2x upscale.
- `src/ocr.rs` — `Windows.Media.Ocr` sarmalayıcısı, satır sıralama ve `BoundingRect` hesabı.
- `src/translator.rs` — `Translator` trait'i (`GroqTranslator`, `GeminiTranslator`, `GoogleTranslator`), önbellek ve akıllı tekrar filtresi.
- `src/overlay.rs` — `WDA_EXCLUDEFROMCAPTURE` + click-through Win32 overlay, dinamik Auto-Fit, yuvarlatılmış köşe, sistem tepsisi yönlendirmesi ve global kısayollar.
- `src/selector.rs` — High-DPI, çoklu monitör uyumlu Win32 bölge seçici.
- `src/gui.rs` — Yerel `eframe`/`egui` masaüstü kontrol merkezi ve canlı önizleme arayüzü.
- `src/tray.rs` — Yerel Win32 Sistem Tepsisi ikonu ve sağ tık menüsü.
- `src/profiles.rs` — Uygulama bazlı altyazı bölgesi profilleri için ön plan `.exe` algılama.
- `src/config.rs` — `config.json` okuma/yazma ve hex renk ayrıştırıcı.
- `start.bat` — Tek tıkla başlatıcı.
- `config.example.json` — Örnek yapılandırma şablonu.
- `VERSIONING.md` — Sürümleme kuralları ve sürüm geçmişi.
- `AGENTS.md` — Geliştirici ve kodlama ajanı mimari rehberi.

---

## Lisans

[MIT Lisansı](LICENSE) ile lisanslanmıştır.
