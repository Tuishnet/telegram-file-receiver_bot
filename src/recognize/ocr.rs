//! Распознавание текста на картинках и в PDF системным движком macOS Vision (вызов из Rust).

use std::path::Path;

use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2_app_kit::NSImage;
use objc2_foundation::{NSArray, NSDictionary, NSSize, NSString, NSURL};
use objc2_pdf_kit::{PDFDisplayBox, PDFDocument};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedTextObservation, VNRequest,
    VNRequestTextRecognitionLevel,
};

/// Сколько страниц сканированного PDF распознавать
const MAX_PDF_PAGES: usize = 30;

fn new_request() -> Retained<VNRecognizeTextRequest> {
    let req = unsafe { VNRecognizeTextRequest::init(VNRecognizeTextRequest::alloc()) };
    req.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    req.setUsesLanguageCorrection(true);
    let langs = NSArray::from_retained_slice(&[
        NSString::from_str("ru-RU"),
        NSString::from_str("uk-UA"),
        NSString::from_str("en-US"),
    ]);
    req.setRecognitionLanguages(&langs);
    req
}

fn collect(req: &VNRecognizeTextRequest) -> String {
    let mut lines = Vec::new();
    if let Some(results) = req.results() {
        for obs in results.iter() {
            if let Ok(obs) = obs.downcast::<VNRecognizedTextObservation>() {
                if let Some(best) = obs.topCandidates(1).firstObject() {
                    lines.push(best.string().to_string());
                }
            }
        }
    }
    lines.join("\n")
}

fn perform(handler: &VNImageRequestHandler) -> Result<String, String> {
    let req = new_request();
    let as_request: &VNRequest = &req;
    let requests = NSArray::from_slice(&[as_request]);
    handler.performRequests_error(&requests).map_err(|e| e.localizedDescription().to_string())?;
    Ok(collect(&req))
}

/// Текст с картинки. Блокирующая функция.
pub fn image_text(path: &Path) -> Result<String, String> {
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let handler = unsafe {
            VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &NSDictionary::new())
        };
        perform(&handler)
    })
}

/// Текст из PDF: сначала текстовый слой, а если его нет (скан) — распознавание страниц.
pub fn pdf_text(path: &Path) -> Result<String, String> {
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let doc = unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url) }.ok_or("не удалось открыть PDF")?;
        if let Some(s) = unsafe { doc.string() } {
            let s = s.to_string();
            if s.trim().chars().count() > 20 {
                return Ok(s);
            }
        }
        // Сканированный документ: рендерим страницы и распознаём
        let pages = unsafe { doc.pageCount() }.min(MAX_PDF_PAGES);
        let mut out = Vec::new();
        for i in 0..pages {
            let Some(page) = (unsafe { doc.pageAtIndex(i) }) else { continue };
            let image: Retained<NSImage> =
                unsafe { page.thumbnailOfSize_forBox(NSSize::new(2000.0, 2000.0), PDFDisplayBox::MediaBox) };
            let Some(cg) = (unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) })
            else {
                continue;
            };
            let handler = unsafe {
                VNImageRequestHandler::initWithCGImage_options(VNImageRequestHandler::alloc(), &cg, &NSDictionary::new())
            };
            if let Ok(t) = perform(&handler) {
                if !t.trim().is_empty() {
                    out.push(t);
                }
            }
        }
        Ok(out.join("\n\n"))
    })
}

/// Прогрев: первое обращение к Vision в процессе загружает модели и может занять десятки секунд.
/// Делаем это при старте бота на маленькой встроенной картинке, а не на первом присланном фото.
pub fn warm_up() {
    static IMG: &[u8] = include_bytes!("warmup.png");
    let path = std::env::temp_dir().join(format!("file_receiver_warmup_{}.png", std::process::id()));
    if std::fs::write(&path, IMG).is_err() {
        return;
    }
    let started = std::time::Instant::now();
    let result = image_text(&path);
    let _ = std::fs::remove_file(&path);
    match result {
        Ok(_) => log::info!("Распознавание текста готово (прогрев {:.1} с)", started.elapsed().as_secs_f32()),
        Err(e) => log::warn!("Прогрев распознавания текста не удался: {e}"),
    }
}
