//! 封面：位元組格式偵測、解碼成置中裁切的正方形縮圖，以及解碼結果的小型 LRU 快取
//! （對應 Swift `CoverView.swift` 的 `ArtworkImageCache`）。
//!
//! 為什麼自己解碼而不是把位元組交給 `gpui::img(Arc<Image>)`：GPUI 0.2.2 的 `ObjectFit::Cover` 把圖片畫在
//! 比元素大的外框裡，圓角套在那個較大的外框上、再被父元素以矩形裁掉，所以非正方形的封面會失去圓角
//! （`elements/img.rs` 的 `paint`：`corner_radii` 對 `new_bounds` 計算）。先在這裡裁成正方形，圖片外框就等於
//! 元素外框，`rounded` 才會正確。順便縮到 [`COVER_TEXTURE_PIXELS`]，不把 1000 × 1000 的原圖塞進 GPU 圖集。

use std::io::Cursor;

use gpui::{ImageFormat, RenderImage};
use image::imageops::FilterType;
use image::{Frame, ImageReader, Limits};

/// 解碼後縮圖的邊長上限（像素）。Expanded 封面 68 點在 2× 螢幕是 136 像素；留一點餘裕。
pub const COVER_TEXTURE_PIXELS: u32 = 160;
/// 解碼的寬、高上限（像素）：防止惡意或損毀的位元組宣稱超大尺寸而吃光記憶體。
pub const MAX_DECODE_DIMENSION: u32 = 8192;
/// 解碼可配置的記憶體上限（位元組）。
pub const MAX_DECODE_ALLOC: u64 = 256 * 1024 * 1024;
/// LRU 保留的封面數（brief：最近 4 個 id）。
pub const COVER_CACHE_CAPACITY: usize = 4;

/// 以開頭的魔術位元組判斷圖片格式；不認得（含 HEIC、SVG、空的）回 `None`，畫面改畫佔位。
pub fn detect_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        [b'I', b'I', 0x2A, 0x00, ..] | [b'M', b'M', 0x00, 0x2A, ..] => Some(ImageFormat::Tiff),
        [b'B', b'M', ..] => Some(ImageFormat::Bmp),
        _ => None,
    }
}

fn to_image_format(format: ImageFormat) -> Option<image::ImageFormat> {
    match format {
        ImageFormat::Png => Some(image::ImageFormat::Png),
        ImageFormat::Jpeg => Some(image::ImageFormat::Jpeg),
        ImageFormat::Gif => Some(image::ImageFormat::Gif),
        ImageFormat::Webp => Some(image::ImageFormat::WebP),
        ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
        ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
        ImageFormat::Svg => None,
    }
}

/// 解碼封面：偵測格式 → 解碼（有尺寸與記憶體上限）→ 從中央裁成正方形 → 大於 `max_pixels` 時縮小 →
/// 轉成 GPUI 的 BGRA [`RenderImage`]。不是圖片、格式不支援或尺寸為 0 時回 `None`。
/// 會花幾毫秒到幾十毫秒，請在背景執行緒呼叫。
pub fn decode_cover(bytes: &[u8], max_pixels: u32) -> Option<RenderImage> {
    let format = to_image_format(detect_image_format(bytes)?)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIMENSION);
    limits.max_image_height = Some(MAX_DECODE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let (w, h) = (decoded.width(), decoded.height());
    if w == 0 || h == 0 {
        return None;
    }
    let side = w.min(h);
    let square = decoded.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let max_pixels = max_pixels.max(1);
    let square = if side > max_pixels {
        square.resize_exact(max_pixels, max_pixels, FilterType::Triangle)
    } else {
        square
    };
    let mut rgba = square.into_rgba8();
    // GPUI 的圖集是 BGRA（同 `gpui::Image::to_image_data`）。
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(RenderImage::new(vec![Frame::new(rgba)]))
}

/// 快取鍵：封面 id 加位元組數（同 Swift：同一 id 的位元組換了也重新解碼）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CoverKey {
    pub id: String,
    pub len: usize,
}

/// 一個封面的解碼狀態。
#[derive(Clone, Debug)]
pub enum CoverEntry<T> {
    /// 背景解碼中（先畫佔位）。
    Pending,
    /// 解碼完成。
    Ready(T),
    /// 不是可解碼的圖片；記住結果，不重試（同 Swift：失敗的解碼也被快取）。
    Failed,
}

/// 最近使用的 `capacity` 個封面（最新的在前）。泛型只是為了讓 LRU 規則不需要 GPUI 就能測試。
#[derive(Debug)]
pub struct CoverCache<T> {
    capacity: usize,
    entries: Vec<(CoverKey, CoverEntry<T>)>,
}

impl<T: Clone> CoverCache<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Vec::new(),
        }
    }

    /// 查詢並標為最近使用。
    pub fn get(&mut self, key: &CoverKey) -> Option<CoverEntry<T>> {
        let pos = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(pos);
        let value = entry.1.clone();
        self.entries.insert(0, entry);
        Some(value)
    }

    /// 放入（或取代）一筆並標為最近使用；回傳被擠出去的已解碼值（呼叫端要釋放它在 GPU 圖集的空間）。
    #[must_use = "evicted images must be released (Window::drop_image)"]
    pub fn insert(&mut self, key: CoverKey, entry: CoverEntry<T>) -> Vec<T> {
        let mut evicted = Vec::new();
        if let Some(pos) = self.entries.iter().position(|(k, _)| *k == key) {
            if let (_, CoverEntry::Ready(old)) = self.entries.remove(pos) {
                evicted.push(old);
            }
        }
        self.entries.insert(0, (key, entry));
        while self.entries.len() > self.capacity {
            if let Some((_, CoverEntry::Ready(old))) = self.entries.pop() {
                evicted.push(old);
            }
        }
        evicted
    }

    /// 只在這一筆仍在快取中（且仍是 `Pending`）時寫入解碼結果；已被擠出時回傳 `Some(value)` 讓呼叫端釋放。
    pub fn finish(&mut self, key: &CoverKey, entry: CoverEntry<T>) -> Option<T> {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some((_, slot @ CoverEntry::Pending)) => {
                *slot = entry;
                None
            }
            _ => match entry {
                CoverEntry::Ready(v) => Some(v),
                _ => None,
            },
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, Rgba, RgbaImage};

    #[test]
    fn detects_formats_by_magic_bytes() {
        let f = detect_image_format;
        assert_eq!(f(b"\x89PNG\r\n\x1a\n...."), Some(ImageFormat::Png));
        assert_eq!(f(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some(ImageFormat::Jpeg));
        assert_eq!(f(b"GIF89a...."), Some(ImageFormat::Gif));
        assert_eq!(f(b"GIF87a...."), Some(ImageFormat::Gif));
        assert_eq!(f(b"RIFF\x10\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(f(b"II*\0...."), Some(ImageFormat::Tiff));
        assert_eq!(f(b"MM\0*...."), Some(ImageFormat::Tiff));
        assert_eq!(f(b"BM......"), Some(ImageFormat::Bmp));
    }

    #[test]
    fn unknown_short_or_empty_bytes_are_not_images() {
        let f = detect_image_format;
        assert_eq!(f(b""), None);
        assert_eq!(f(&[0x89, b'P']), None);
        assert_eq!(f(&[0xFF, 0xD8]), None);
        // RIFF 但不是 WEBP（例如 WAV）。
        assert_eq!(f(b"RIFF\x10\0\0\0WAVEfmt "), None);
        // HEIC（ISO BMFF `ftypheic`）：GPUI／image 不支援，畫佔位。
        assert_eq!(f(b"\0\0\0\x18ftypheic"), None);
        assert_eq!(f(b"<svg xmlns="), None);
        assert_eq!(f(b"hello world"), None);
    }

    fn png(w: u32, h: u32, color: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |x, y| Rgba(color(x, y)));
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn decode_crops_the_centre_square_and_converts_to_bgra() {
        // 30 × 10：左右各 10 像素是藍色，中央 10 × 10 是紅色。
        let bytes = png(30, 10, |x, _| {
            if (10..20).contains(&x) {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let image = decode_cover(&bytes, 160).expect("decodes");
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (10, 10));
        let px = image.as_bytes(0).unwrap();
        // BGRA：紅色 = (0, 0, 255, 255)；整張都是中央的紅色，沒有藍邊。
        assert!(px.chunks_exact(4).all(|p| p == [0, 0, 255, 255]));
    }

    #[test]
    fn decode_downscales_large_covers() {
        let bytes = png(400, 300, |_, _| [10, 20, 30, 255]);
        let image = decode_cover(&bytes, 160).expect("decodes");
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (160, 160));
    }

    #[test]
    fn decode_rejects_garbage_and_truncated_images() {
        assert!(decode_cover(b"", 160).is_none());
        assert!(decode_cover(b"not an image at all", 160).is_none());
        let bytes = png(20, 20, |_, _| [1, 2, 3, 255]);
        assert!(decode_cover(&bytes[..bytes.len() / 2], 160).is_none());
        // 宣稱是 JPEG 的垃圾。
        assert!(decode_cover(&[0xFF, 0xD8, 0xFF, 0, 1, 2, 3], 160).is_none());
    }

    fn key(id: &str) -> CoverKey {
        CoverKey {
            id: id.into(),
            len: 1,
        }
    }

    #[test]
    fn cache_keeps_the_most_recent_four_and_returns_evicted_images() {
        let mut c: CoverCache<u32> = CoverCache::new(COVER_CACHE_CAPACITY);
        for i in 0..4 {
            assert!(c
                .insert(key(&i.to_string()), CoverEntry::Ready(i))
                .is_empty());
        }
        // 使用 "0"，讓 "1" 變成最久沒用的。
        assert!(matches!(c.get(&key("0")), Some(CoverEntry::Ready(0))));
        assert_eq!(c.insert(key("4"), CoverEntry::Ready(4)), vec![1]);
        assert!(c.get(&key("1")).is_none());
        assert_eq!(c.len(), 4);
        // 擠出 Pending／Failed 不回傳東西。
        let mut c: CoverCache<u32> = CoverCache::new(1);
        assert!(c.insert(key("a"), CoverEntry::Pending).is_empty());
        assert!(c.insert(key("b"), CoverEntry::Failed).is_empty());
        assert_eq!(c.insert(key("c"), CoverEntry::Ready(7)), Vec::<u32>::new());
        // 同鍵取代：舊值被回傳。
        assert_eq!(c.insert(key("c"), CoverEntry::Ready(8)), vec![7]);
    }

    #[test]
    fn same_id_with_different_bytes_is_a_different_key() {
        let mut c: CoverCache<u32> = CoverCache::new(4);
        let _ = c.insert(key("a"), CoverEntry::Ready(1));
        let other = CoverKey {
            id: "a".into(),
            len: 2,
        };
        assert!(c.get(&other).is_none());
    }

    #[test]
    fn finish_fills_pending_entries_and_hands_back_orphans() {
        let mut c: CoverCache<u32> = CoverCache::new(1);
        let _ = c.insert(key("a"), CoverEntry::Pending);
        assert_eq!(c.finish(&key("a"), CoverEntry::Ready(1)), None);
        assert!(matches!(c.get(&key("a")), Some(CoverEntry::Ready(1))));
        // "a" 已被擠出時完成：值交還呼叫端釋放。
        let _ = c.insert(key("b"), CoverEntry::Pending);
        assert_eq!(c.finish(&key("a"), CoverEntry::Ready(2)), Some(2));
        assert_eq!(c.finish(&key("zz"), CoverEntry::Failed), None);
        assert_eq!(c.finish(&key("b"), CoverEntry::Failed), None);
        assert!(matches!(c.get(&key("b")), Some(CoverEntry::Failed)));
    }
}
