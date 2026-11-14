//! Windows' own codecs: PDF page rendering (Windows.Data.Pdf) and WIC image decoding
//! (HEIC/AVIF through the Store codec extensions).

use anyhow::{Context as _, Result, anyhow};
use std::path::Path;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Com::StructuredStorage::PROPBAG2;
use windows::core::HSTRING;

/// COM/WinRT on this thread (multithreaded apartment). Safe to call repeatedly.
pub fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static BUSY: AtomicUsize = AtomicUsize::new(0);
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Counts a Windows codec call in flight. Exiting while one runs (a thumbnail thread
/// rendering a PDF page) crashed the process at a null address while Windows tore down.
struct Busy;
impl Busy {
    fn enter() -> Result<Busy> {
        BUSY.fetch_add(1, Ordering::SeqCst);
        if CLOSING.load(Ordering::SeqCst) {
            BUSY.fetch_sub(1, Ordering::SeqCst);
            return Err(anyhow!("closing"));
        }
        Ok(Busy)
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Call before the process exits: refuses new codec calls and waits (up to `max`) for
/// the ones running to finish.
pub fn shutdown(max: Duration) {
    CLOSING.store(true, Ordering::SeqCst);
    let t = Instant::now();
    while BUSY.load(Ordering::SeqCst) > 0 && t.elapsed() < max {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Render page `index` of a PDF to PNG bytes `width` pixels wide.
pub fn render_pdf_page_png(path: &Path, password: Option<&str>, index: u32, width: u32) -> Result<Vec<u8>> {
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    let _busy = Busy::enter()?;
    com_init();
    let abs = std::path::absolute(path)?;
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(abs.as_os_str()))?.join()?;
    let doc = match password {
        Some(pw) => PdfDocument::LoadFromFileWithPasswordAsync(&file, &HSTRING::from(pw))?.join()?,
        None => PdfDocument::LoadFromFileAsync(&file)?.join()?,
    };
    let page = doc.GetPage(index)?;
    let stream = InMemoryRandomAccessStream::new()?;
    let opts = PdfPageRenderOptions::new()?;
    opts.SetDestinationWidth(width)?;
    page.RenderWithOptionsToStreamAsync(&stream, &opts)?.join()?;
    let size = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    reader.LoadAsync(size)?.join()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf)?;
    Ok(buf)
}

/// Decode any image Windows can read (HEIC, AVIF, RAW...) to RGBA8.
pub fn wic_decode(path: &Path) -> Result<image::RgbaImage> {
    let _busy = Busy::enter()?;
    com_init();
    unsafe {
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let dec = f
            .CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .with_context(|| format!("Windows has no codec for {}", path.display()))?;
        let frame = dec.GetFrame(0)?;
        let conv = f.CreateFormatConverter()?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let (mut w, mut h) = (0, 0);
        conv.GetSize(&mut w, &mut h)?;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut buf)?;
        image::RgbaImage::from_raw(w, h, buf).ok_or_else(|| anyhow!("bad image size"))
    }
}

/// Encode RGBA8 as HEIC through WIC (needs the HEIF + HEVC extensions).
pub fn wic_encode_heic(im: &image::RgbaImage, out: &Path, quality: f32) -> Result<()> {
    let _busy = Busy::enter()?;
    com_init();
    unsafe {
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = f.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(out.as_os_str()), windows::Win32::Foundation::GENERIC_WRITE.0)?;
        let enc = f.CreateEncoder(&GUID_ContainerFormatHeif, std::ptr::null()).context("no HEIF encoder (install HEIF + HEVC Video Extensions)")?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        let mut props = None;
        enc.CreateNewFrame(&mut frame, &mut props)?;
        let frame = frame.ok_or_else(|| anyhow!("no frame"))?;
        if let Some(props) = &props {
            let name = windows::core::w!("ImageQuality");
            let mut bag = [PROPBAG2 { pstrName: windows::core::PWSTR(name.as_ptr() as *mut _), ..Default::default() }];
            let v = windows::Win32::System::Variant::VARIANT::from(quality);
            let _ = props.Write(1, bag.as_mut_ptr(), &v);
        }
        frame.Initialize(props.as_ref())?;
        frame.SetSize(im.width(), im.height())?;
        let mut fmt = GUID_WICPixelFormat32bppRGBA;
        frame.SetPixelFormat(&mut fmt)?;
        let rgba = if fmt == GUID_WICPixelFormat32bppRGBA {
            im.as_raw().clone()
        } else {
            // the encoder asked for BGRA: swap
            let mut v = im.as_raw().clone();
            for p in v.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            v
        };
        frame.WritePixels(im.height(), im.width() * 4, &rgba)?;
        frame.Commit()?;
        enc.Commit()?;
        Ok(())
    }
}

/// Whether WIC can write HEIC here (the HEIF + HEVC Store extensions).
pub fn heif_encoder_available() -> bool {
    // the encoder object exists even without the HEVC extension; a real write tells for sure
    let p = std::env::temp_dir().join(format!("utsuri-heif-check-{}.heic", std::process::id()));
    let ok = wic_encode_heic(&image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 100, 50, 255])), &p, 0.8).is_ok() && std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false);
    let _ = std::fs::remove_file(&p);
    ok
}

/// Where a Windows .lnk shortcut points, if it's an existing file or folder.
pub fn shortcut_target(lnk: &Path) -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::{IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    com_init();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = windows::core::Interface::cast(&link).ok()?;
        pf.Load(&HSTRING::from(lnk.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        let p = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len]));
        p.exists().then_some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heic_round_trip_through_wic() {
        let d = std::env::temp_dir().join(format!("utsuri-wic-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let im = image::RgbaImage::from_fn(64, 48, |x, _| image::Rgba([if x < 32 { 255 } else { 0 }, 0, 255, 255]));
        let out = d.join("t.heic");
        wic_encode_heic(&im, &out, 0.9).unwrap();
        let back = wic_decode(&out).unwrap();
        assert_eq!(back.dimensions(), (64, 48));
        let p = back.get_pixel(8, 8).0;
        assert!(p[0] > 200 && p[2] > 200, "{p:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn renders_a_pdf_page() {
        use lopdf::{Document, Object, Stream, dictionary};
        let d = std::env::temp_dir().join(format!("utsuri-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content = b"0 0 1 rg 100 100 200 300 re f".to_vec();
        let cid = doc.add_object(Stream::new(dictionary! {}, content));
        let page = doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => cid, "MediaBox" => vec![0.into(), 0.into(), 400.into(), 500.into()]});
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1}));
        let cat = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        doc.trailer.set("Root", cat);
        let p = d.join("t.pdf");
        doc.save(&p).unwrap();
        let png = render_pdf_page_png(&p, None, 0, 400).unwrap();
        let im = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(im.width(), 400);
        let blue = im.get_pixel(200, 250).0; // inside the rect (PDF y-up: 100..400 -> image 100..400)
        let white = im.get_pixel(20, 20).0;
        assert!(blue[2] > 200 && blue[0] < 50, "{blue:?}");
        assert!(white[0] > 240, "{white:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
