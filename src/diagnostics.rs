//! OCR teşhisi isteğe bağlıdır: görüntüler yalnızca yerelde, sınırlı bir
//! halka içinde saklanır. Disk yazımı OCR işçisini bekletmez.

use crate::capture::CapturedFrame;
use crate::config::{AppConfig, Region};
use crate::ocr::OcrOutput;
use crate::overlay::{NoticeLevel, OverlayHandle};
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_SAMPLES: u64 = 60;

/// Kayıt konumu arayüz ve yazıcı tarafından aynı şekilde hesaplanır.
pub fn directory() -> PathBuf {
    AppConfig::path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("ocr-diagnostics")
}

struct Sample {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
    metadata: serde_json::Value,
}

pub struct OcrDiagnostics {
    handle: OverlayHandle,
    sender: SyncSender<Sample>,
    last_sample: Instant,
}

impl OcrDiagnostics {
    pub fn new(handle: OverlayHandle) -> Self {
        // En fazla iki görüntü bekleyebilir; yavaş diskte OCR yerine örnek düşürülür.
        let (sender, receiver) = sync_channel::<Sample>(2);
        let writer_handle = handle.clone();
        std::thread::spawn(move || {
            let mut index = 0;
            while let Ok(sample) = receiver.recv() {
                if !writer_handle.ocr_diagnostics.load(Ordering::SeqCst) {
                    continue;
                }
                let result = write_sample(&directory(), index, &sample);
                index += 1;
                if let Err(error) = result {
                    writer_handle.ocr_diagnostics.store(false, Ordering::SeqCst);
                    writer_handle.push_notice(
                        NoticeLevel::Error,
                        "ocr-diagnostics",
                        format!("OCR diagnostics stopped: {error}"),
                    );
                }
            }
        });
        Self {
            handle,
            sender,
            last_sample: Instant::now() - Duration::from_secs(1),
        }
    }

    /// Gerçek yakalama, OCR sınırları ve tekrar kararı tek örnekte eşleştirilir.
    /// API anahtarları ve tüm masaüstü bu kayda dahil edilmez.
    pub fn record(
        &mut self,
        frame: &CapturedFrame,
        region: Region,
        output: &OcrOutput,
        previous: &str,
        ocr_ms: u128,
        decision: &str,
    ) {
        if !self.handle.ocr_diagnostics.load(Ordering::SeqCst)
            || self.last_sample.elapsed() < Duration::from_millis(250)
        {
            return;
        }
        self.last_sample = Instant::now();
        let bounds = output
            .bounds
            .map(|b| json!({"x": b.x, "y": b.y, "width": b.width, "height": b.height}));
        let similarity = crate::translator::text_similarity(&output.text, previous);
        let reason = if decision != "duplicate" {
            decision
        } else if output.text.eq_ignore_ascii_case(previous) {
            "same_text"
        } else if similarity >= 0.80 {
            "levenshtein"
        } else {
            "word_overlap"
        };
        let metadata = json!({
            "unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "region": {"left": region.left, "top": region.top, "width": region.width, "height": region.height},
            "text": output.text, "previous_text": previous, "source_bounds": bounds,
            "decision": decision, "dedup_reason": reason, "similarity": similarity,
            "capture_ocr_ms": ocr_ms, "overlay_text": self.handle.snapshot_text(),
        });
        let _ = self.sender.try_send(Sample {
            width: frame.width,
            height: frame.height,
            pixels: frame.bgra.clone(),
            metadata,
        });
    }
}

/// Sabit yuvalar yalnızca bu teşhis klasöründeki örneklerin üzerine yazar.
fn write_sample(dir: &std::path::Path, index: u64, sample: &Sample) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let stem = format!("sample-{:02}", index % MAX_SAMPLES);
    let metadata_path = dir.join(format!("{stem}.json"));
    // Eski JSON yeni görüntüyle eşleşmiş görünmesin; tamamlanmış çift JSON ile belirlenir.
    match std::fs::remove_file(&metadata_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut bitmap = std::fs::File::create(dir.join(format!("{stem}.bmp")))?;
    write_bmp(&mut bitmap, sample.width, sample.height, &sample.pixels)?;
    std::fs::write(metadata_path, serde_json::to_vec_pretty(&sample.metadata)?)?;
    Ok(())
}

/// Negatif yükseklik üstten alta BGRA düzenini korur; ek resim kütüphanesi gerekmez.
pub(crate) fn write_bmp(
    writer: &mut impl Write,
    width: i32,
    height: i32,
    pixels: &[u8],
) -> anyhow::Result<()> {
    anyhow::ensure!(width > 0 && height > 0, "Invalid bitmap size");
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("Bitmap size overflow"))?;
    anyhow::ensure!(
        pixels.len() == len && len <= (u32::MAX - 54) as usize,
        "Invalid bitmap buffer"
    );
    writer.write_all(b"BM")?;
    writer.write_all(&(54 + len as u32).to_le_bytes())?;
    writer.write_all(&[0; 4])?;
    writer.write_all(&54u32.to_le_bytes())?;
    writer.write_all(&40u32.to_le_bytes())?;
    writer.write_all(&width.to_le_bytes())?;
    writer.write_all(&(-height).to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&32u16.to_le_bytes())?;
    writer.write_all(&0u32.to_le_bytes())?;
    writer.write_all(&(len as u32).to_le_bytes())?;
    writer.write_all(&[0; 16])?;
    writer.write_all(pixels)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uzun oturumda sınırın aşılmadığı ve JSON'un en yeni pikselle eşleştiği doğrulanır.
    #[test]
    fn recording_ring_keeps_at_most_sixty_pairs() {
        let dir = std::env::temp_dir().join(format!(
            "overcap-ocr-ring-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for index in 0..65 {
            write_sample(
                &dir,
                index,
                &Sample {
                    width: 1,
                    height: 1,
                    pixels: vec![index as u8, 0, 0, 255],
                    metadata: json!({"sample": index}),
                },
            )
            .unwrap();
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 120);
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("sample-04.json")).unwrap()).unwrap();
        assert_eq!(metadata["sample"], 64);
        assert_eq!(std::fs::read(dir.join("sample-04.bmp")).unwrap()[54], 64);
        // Yalnızca bu testin benzersiz adla oluşturduğu geçici klasör temizlenir.
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Ham piksel sırası ve boyutlar bozulursa teşhis yanlış kanıt üretir.
    #[test]
    fn bitmap_preserves_capture_pixels_and_top_down_orientation() {
        let mut bytes = Vec::new();
        write_bmp(&mut bytes, 1, 2, &[1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        assert_eq!(&bytes[..2], b"BM");
        assert_eq!(i32::from_le_bytes(bytes[22..26].try_into().unwrap()), -2);
        assert_eq!(bytes.len(), 62);
        assert_eq!(&bytes[54..], &[1, 2, 3, 255, 4, 5, 6, 255]);
        assert!(write_bmp(&mut Vec::new(), 2, 2, &[0; 4]).is_err());
    }
}
