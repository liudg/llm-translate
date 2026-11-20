//! M0 spike: PP-OCRv6 via oar-ocr (ONNX Runtime). Measures load + predict time.
//! Usage: ocr-spike <tiny|small> <image>...
use oar_ocr::prelude::*;
use std::path::Path;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let size = match args.next().as_deref() {
        Some("tiny") => PpOcrV6Size::Tiny,
        _ => PpOcrV6Size::Small,
    };
    let t = Instant::now();
    let ocr = OAROCRBuilder::pp_ocrv6(size).build()?;
    println!("load: {} ms ({size:?})", t.elapsed().as_millis());
    for img_path in args {
        let image = load_image(Path::new(&img_path))?;
        // warm-up + timed runs
        let _ = ocr.predict(vec![image.clone()])?;
        let t = Instant::now();
        let runs = 3;
        let mut res = None;
        for _ in 0..runs {
            res = Some(ocr.predict(vec![image.clone()])?);
        }
        let ms = t.elapsed().as_millis() / runs;
        let res = res.unwrap();
        println!("\n== {img_path}  {}x{}  predict: {ms} ms", image.width(), image.height());
        for region in &res[0].text_regions {
            if let Some((text, conf)) = region.text_with_confidence() {
                println!("  [{conf:.2}] {text}   bbox={:?}", region.bounding_box);
            }
        }
    }
    Ok(())
}
