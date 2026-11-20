//! PP-OCRv6 Small (DESIGN 5.6 step 3) through oar-ocr. The engine loads in the
//! background as soon as a screenshot starts, overlapping the user's drag, and
//! is dropped after [`IDLE_RELEASE`] without use to keep the app's memory low.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oar_ocr::domain::tasks::TextDetectionConfig;
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};

/// Loaded the engine costs ~270 MB; reloading takes ~0.35 s and happens while
/// the user drags, so it is released soon after use.
const IDLE_RELEASE: Duration = Duration::from_secs(60);
const DET: &str = "pp-ocrv6_small_det.onnx";
const REC: &str = "pp-ocrv6_small_rec.onnx";
const DICT: &str = "ppocrv6_dict.txt";

/// One recognised text line, axis-aligned, in image pixels.
#[derive(Debug, Clone)]
pub struct Line {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub text: String,
    pub confidence: f32,
}

impl Line {
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
}

pub struct Ocr {
    dir: PathBuf,
    engine: Mutex<Option<Arc<OAROCR>>>,
    /// Held while loading so a preload and a recognise never load twice.
    loading: Mutex<()>,
    last_used: Mutex<Instant>,
}

impl Ocr {
    pub fn new(dir: PathBuf) -> Arc<Self> {
        let ocr =
            Arc::new(Self { dir, engine: Mutex::new(None), loading: Mutex::new(()), last_used: Mutex::new(Instant::now()) });
        let weak = Arc::downgrade(&ocr);
        std::thread::Builder::new()
            .name("ocr-idle".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(15));
                    let Some(ocr) = weak.upgrade() else { return };
                    let idle = ocr.last_used.lock().unwrap().elapsed() >= IDLE_RELEASE;
                    if idle && ocr.engine.lock().unwrap().take().is_some() {
                        tracing::info!("OCR engine released after idle");
                    }
                }
            })
            .expect("spawning the OCR idle thread");
        ocr
    }

    fn engine(&self) -> Result<Arc<OAROCR>, String> {
        *self.last_used.lock().unwrap() = Instant::now();
        if let Some(e) = self.engine.lock().unwrap().clone() {
            return Ok(e);
        }
        let _loading = self.loading.lock().unwrap();
        if let Some(e) = self.engine.lock().unwrap().clone() {
            return Ok(e);
        }
        let t0 = Instant::now();
        // The official PP-OCRv6 detection defaults (what `pp_ocrv6()` presets),
        // with the model files from our resource directory.
        let det_cfg = TextDetectionConfig {
            score_threshold: 0.2,
            box_threshold: 0.45,
            unclip_ratio: 1.4,
            max_candidates: 3000,
            ..TextDetectionConfig::default()
        };
        let engine = OAROCRBuilder::new(self.dir.join(DET), self.dir.join(REC), self.dir.join(DICT))
            .text_detection_config(det_cfg)
            .build()
            .map_err(|e| {
                tracing::error!("loading OCR models from {} failed: {e}", self.dir.display());
                format!("加载文字识别模型失败：{e}")
            })?;
        let engine = Arc::new(engine);
        tracing::info!("OCR engine loaded in {} ms", t0.elapsed().as_millis());
        *self.engine.lock().unwrap() = Some(engine.clone());
        Ok(engine)
    }

    /// Start loading in the background if not loaded yet.
    pub fn preload(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            let _ = me.engine();
        });
    }

    /// Recognise text lines in a top-down BGRA image (blocking).
    pub fn recognize(&self, bgra: &[u8], width: u32, height: u32) -> Result<Vec<Line>, String> {
        let engine = self.engine()?;
        let t0 = Instant::now();
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for px in bgra.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        let image = image::RgbImage::from_raw(width, height, rgb).ok_or("图像尺寸无效")?;
        let pages = engine.predict(vec![image]).map_err(|e| {
            tracing::error!("OCR failed: {e}");
            format!("文字识别失败：{e}")
        })?;
        *self.last_used.lock().unwrap() = Instant::now();
        let mut lines = Vec::new();
        for region in pages.into_iter().flat_map(|p| p.text_regions) {
            let Some((text, confidence)) = region.text_with_confidence() else { continue };
            let text = text.trim().to_string();
            let pts = &region.bounding_box.points;
            if text.is_empty() || pts.is_empty() {
                continue;
            }
            // Window buttons and icons come out as single low-confidence symbols.
            if confidence < 0.8 && text.chars().count() <= 1 {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for p in pts {
                x0 = x0.min(p.x);
                y0 = y0.min(p.y);
                x1 = x1.max(p.x);
                y1 = y1.max(p.y);
            }
            lines.push(Line {
                x0: x0.max(0.0),
                y0: y0.max(0.0),
                x1: x1.min(width as f32),
                y1: y1.min(height as f32),
                text,
                confidence,
            });
        }
        tracing::info!("OCR {}x{}: {} lines in {} ms", width, height, lines.len(), t0.elapsed().as_millis());
        Ok(lines)
    }
}
