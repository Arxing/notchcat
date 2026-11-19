//! 封面：位元組格式偵測、解碼成置中裁切的正方形縮圖，以及解碼結果的小型 LRU 快取
//! （對應 Swift `CoverView.swift` 的 `ArtworkImageCache`）。
//!
//! 為什麼自己解碼而不是把位元組交給 `gpui::img(Arc<Image>)`：GPUI 0.2.2 的 `ObjectFit::Cover` 把圖片畫在
//! 比元素大的外框裡，圓角套在那個較大的外框上、再被父元素以矩形裁掉，所以非正方形的封面會失去圓角
//! （`elements/img.rs` 的 `paint`：`corner_radii` 對 `new_bounds` 計算）。先在這裡裁成正方形，圖片外框就等於
//! 元素外框，`rounded` 才會正確。順便縮到 [`COVER_TEXTURE_PIXELS`]，不把 1000 × 1000 的原圖塞進 GPU 圖集。
//!
//! # 記憶體上限（封面位元組不可信：瀏覽器的 Media Session 封面會進 Now Playing）
//!
//! 1. 輸入超過 [`MAX_COVER_BYTES`] 直接拒絕，不碰解碼器。
//! 2. 先只讀檔頭的寬高（[`cover_dimensions`]，不配置像素緩衝區）；任一邊超過 [`MAX_DECODE_DIMENSION`] 就拒絕。
//! 3. 解碼時再套 `image` 的 `Limits`（寬高同上、`max_alloc` = [`MAX_DECODE_ALLOC`]）：`image` 0.25.10 的
//!    `ImageReader::decode` 先以 `Limits::reserve(decoder.total_bytes())` 檢查整個輸出緩衝區，再 `set_limits`
//!    檢查寬高，所以 16 位元或其他會超過 64 MiB 的輸出也會被拒絕。
//! 4. 裁切用 `imageops::crop_imm` 的**檢視**（不複製），直接縮成 ≤ 160 × 160 的縮圖；
//!    所以一次解碼的峰值 ≈ 解碼後的原圖（≤ 64 MiB）加上很小的縮圖。
//! 5. 同時最多一個解碼（[`DecodeQueue`]）：只排隊最新要求的封面，被取代的要求直接丟掉，
//!    已不需要的結果在完成時丟棄。

use std::io::Cursor;

use gpui::{ImageFormat, RenderImage};
use image::{imageops, Frame, ImageReader, Limits};

/// 解碼後縮圖的邊長上限（像素）。Expanded 封面 68 點在 2× 螢幕是 136 像素；留一點餘裕。
pub const COVER_TEXTURE_PIXELS: u32 = 160;
/// 封面位元組的上限：超過就不解碼（真實封面通常是數十到數百 KB）。
pub const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;
/// 解碼的寬、高上限（像素）：防止惡意或損毀的位元組宣稱超大尺寸而吃光記憶體。
pub const MAX_DECODE_DIMENSION: u32 = 4096;
/// 解碼可配置的記憶體上限（位元組）：4096 × 4096 × 4（RGBA8）剛好 64 MiB。
pub const MAX_DECODE_ALLOC: u64 = 64 * 1024 * 1024;
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

/// 解碼前的檢查：位元組數、格式與檔頭宣告的寬高都在上限內時回傳 `(格式, 寬, 高)`。
/// 只讀檔頭（`ImageReader::into_dimensions`），不配置像素緩衝區。
pub fn cover_dimensions(bytes: &[u8]) -> Option<(image::ImageFormat, u32, u32)> {
    if bytes.len() > MAX_COVER_BYTES {
        return None;
    }
    let format = to_image_format(detect_image_format(bytes)?)?;
    let (w, h) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    let ok = |d: u32| d > 0 && d <= MAX_DECODE_DIMENSION;
    (ok(w) && ok(h)).then_some((format, w, h))
}

/// 解碼封面：[`cover_dimensions`] 檢查 → 解碼（有尺寸與記憶體上限）→ 以檢視從中央裁成正方形 →
/// 大於 `max_pixels` 時直接縮成縮圖 → 轉成 GPUI 的 BGRA [`RenderImage`]。
/// 不是圖片、格式不支援、太大或尺寸為 0 時回 `None`。會花幾毫秒到幾十毫秒，請在背景執行緒呼叫。
pub fn decode_cover(bytes: &[u8], max_pixels: u32) -> Option<RenderImage> {
    let (format, _, _) = cover_dimensions(bytes)?;
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
    // 檢視，不複製原圖。
    let square = imageops::crop_imm(&decoded, (w - side) / 2, (h - side) / 2, side, side);
    let max_pixels = max_pixels.max(1);
    let mut rgba = if side > max_pixels {
        imageops::thumbnail(&*square, max_pixels, max_pixels)
    } else {
        square.to_image()
    };
    drop(decoded);
    // GPUI 的圖集是 BGRA（同 `gpui::Image::to_image_data`）。
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(RenderImage::new(vec![Frame::new(rgba)]))
}

/// 同時最多一個封面解碼的排程（純狀態機）。
///
/// - [`request`](Self::request)：畫面需要某個還沒解碼的封面時呼叫。最新的要求取代排隊中的舊要求（舊的直接丟掉）；
///   已在解碼中的同一個鍵不重複排隊。
/// - [`start_next`](Self::start_next)：沒有解碼在進行時，取出排隊中的要求開始解碼。
/// - [`finish`](Self::finish)：解碼完成；回傳這個結果是否仍然需要（= 它是最後一次被要求的鍵），
///   不需要的結果由呼叫端丟棄。之後再呼叫 `start_next` 開始下一個。
///
/// 所以任何時刻最多一個解碼在跑、一個在排隊，記憶體上限與封面 id 換得多快無關。
#[derive(Debug)]
pub struct DecodeQueue<K, V> {
    in_flight: Option<K>,
    queued: Option<(K, V)>,
    latest: Option<K>,
}

impl<K: Clone + PartialEq, V> Default for DecodeQueue<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + PartialEq, V> DecodeQueue<K, V> {
    pub fn new() -> Self {
        Self {
            in_flight: None,
            queued: None,
            latest: None,
        }
    }

    /// 要求解碼 `key`（附帶要解碼的資料）。
    pub fn request(&mut self, key: K, payload: V) {
        self.latest = Some(key.clone());
        if self.in_flight.as_ref() == Some(&key) {
            // 已在解碼中：不必再排；排隊中的舊要求也不再需要。
            self.queued = None;
            return;
        }
        self.queued = Some((key, payload));
    }

    /// 沒有解碼在進行時，取出排隊中的要求並標為進行中。
    pub fn start_next(&mut self) -> Option<(K, V)> {
        if self.in_flight.is_some() {
            return None;
        }
        let (key, payload) = self.queued.take()?;
        self.in_flight = Some(key.clone());
        Some((key, payload))
    }

    /// `key` 的解碼完成；回傳結果是否仍然需要。
    pub fn finish(&mut self, key: &K) -> bool {
        if self.in_flight.as_ref() == Some(key) {
            self.in_flight = None;
        }
        self.latest.as_ref() == Some(key)
    }

    /// 是否有解碼在進行。
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }
}

/// 快取鍵：封面 id 加位元組數（同 Swift：同一 id 的位元組換了也重新解碼）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CoverKey {
    pub id: String,
    pub len: usize,
}

/// 一個封面的解碼結果（解碼中的封面不在快取裡，由 [`DecodeQueue`] 管）。
#[derive(Clone, Debug)]
pub enum CoverEntry<T> {
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
        // 擠出 Failed 不回傳東西。
        let mut c: CoverCache<u32> = CoverCache::new(1);
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

    /// 只有 PNG 簽章、IHDR（宣告寬高）與一個空 IDAT 的檔，沒有像素資料。
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &b in data {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&w.to_be_bytes());
        chunk.extend_from_slice(&h.to_be_bytes());
        chunk.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 位元 RGBA
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
        // 一個空的 IDAT：PNG 解碼器讀檔頭時會讀到第一個 IDAT 為止，所以要有它（但沒有任何像素資料）。
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(b"IDAT");
        out.extend_from_slice(&crc32(b"IDAT").to_be_bytes());
        out
    }

    #[test]
    fn header_dimensions_are_read_without_decoding() {
        // 只有檔頭、沒有像素：讀得到寬高（證明沒有解碼）。
        let header = png_header(300, 200);
        assert_eq!(
            cover_dimensions(&header),
            Some((image::ImageFormat::Png, 300, 200))
        );
        assert!(decode_cover(&header, 160).is_none(), "no pixel data");
    }

    #[test]
    fn oversize_dimensions_are_rejected_from_the_header() {
        // 宣告 100000 × 100000（RGBA 約 37 GB）的 PNG：只看檔頭就拒絕，不配置任何像素緩衝區。
        let huge = png_header(100_000, 100_000);
        assert_eq!(cover_dimensions(&huge), None);
        assert!(decode_cover(&huge, 160).is_none());
        // 一邊超過上限也拒絕；剛好等於上限通過檔頭檢查。
        assert_eq!(
            cover_dimensions(&png_header(MAX_DECODE_DIMENSION + 1, 10)),
            None
        );
        assert_eq!(
            cover_dimensions(&png_header(10, MAX_DECODE_DIMENSION + 1)),
            None
        );
        assert!(
            cover_dimensions(&png_header(MAX_DECODE_DIMENSION, MAX_DECODE_DIMENSION)).is_some()
        );
        assert_eq!(cover_dimensions(&png_header(0, 10)), None);
        // 上限本身：4096² × 4 = 64 MiB。
        assert_eq!(u64::from(MAX_DECODE_DIMENSION).pow(2) * 4, MAX_DECODE_ALLOC);
    }

    #[test]
    fn input_over_the_byte_cap_is_rejected_before_decoding() {
        let mut big = png(4, 4, |_, _| [1, 2, 3, 255]);
        assert!(decode_cover(&big, 160).is_some());
        // 合法的 PNG 後面塞滿垃圾到超過上限：解碼器根本不會看到它。
        big.resize(MAX_COVER_BYTES + 1, 0);
        assert_eq!(cover_dimensions(&big), None);
        assert!(decode_cover(&big, 160).is_none());
    }

    #[test]
    fn decode_crops_portrait_covers_from_the_centre() {
        // 10 × 30：上下各 10 像素是藍色，中央 10 × 10 是綠色。
        let bytes = png(10, 30, |_, y| {
            if (10..20).contains(&y) {
                [0, 255, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let image = decode_cover(&bytes, 160).expect("decodes");
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (10, 10));
        assert!(image
            .as_bytes(0)
            .unwrap()
            .chunks_exact(4)
            .all(|p| p == [0, 255, 0, 255]));
    }

    #[test]
    fn decode_queue_runs_one_at_a_time_and_newest_wins() {
        let mut q: DecodeQueue<&str, u8> = DecodeQueue::new();
        assert!(q.start_next().is_none());
        q.request("a", 1);
        assert_eq!(q.start_next(), Some(("a", 1)));
        assert!(q.is_busy());
        // 解碼 a 的期間又要求 b、c：只留最新的 c，而且在 a 完成前不開始。
        q.request("b", 2);
        q.request("c", 3);
        assert_eq!(q.start_next(), None);
        // a 已不是最後要求的：結果丟棄。
        assert!(!q.finish(&"a"));
        assert!(!q.is_busy());
        assert_eq!(q.start_next(), Some(("c", 3)));
        assert_eq!(q.start_next(), None, "b was dropped");
        assert!(q.finish(&"c"));
        assert_eq!(q.start_next(), None);
    }

    #[test]
    fn decode_queue_does_not_requeue_the_key_in_flight() {
        let mut q: DecodeQueue<&str, u8> = DecodeQueue::new();
        q.request("a", 1);
        assert_eq!(q.start_next(), Some(("a", 1)));
        // 每一格重畫都會再要求一次 a：不重複排隊。
        q.request("a", 1);
        q.request("b", 2);
        q.request("a", 1);
        assert!(q.finish(&"a"), "a is the latest request again");
        assert_eq!(q.start_next(), None, "b was superseded by a");
        // 完成一個不在進行中的鍵不會清掉進行中的那個。
        q.request("x", 9);
        assert_eq!(q.start_next(), Some(("x", 9)));
        assert!(!q.finish(&"zz"));
        assert!(q.is_busy());
    }
}
