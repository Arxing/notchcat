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
//!    **讀檔頭本身也要設上限**（[`HEADER_CHECK_ALLOC`]）：`image` 的 PNG 解碼器讀檔頭時會讀到第一個 IDAT 為止，
//!    途中把 iCCP（ICC 色彩描述檔）等壓縮的附屬區塊**立刻解壓**（png 0.18.1 `decoder/stream.rs` 的 `parse_iccp`：
//!    `fdeflate::decompress_to_vec_bounded(buf, limits.bytes)`）。不設限時用的是 `image` 預設的 512 MiB，
//!    一個 489 KB、1 × 1 的 PNG 就能讓「檔頭檢查」吃到約 376 MB（審查者實測）。設了上限後，解壓輸出到上限就停下；
//!    png 0.18.1 對 iCCP 是 `let _ = self.parse_iccp_raw();`，也就是**超過上限不算錯誤**：描述檔被丟掉
//!    （`icc_profile()` 為 `None`），檔頭照常讀完。所以 png 自己不會拒絕炸彈；檔頭檢查最多只花 4 MiB，
//!    而且本版再補一條：PNG **有** iCCP 區塊（[`png_has_iccp`] 只掃區塊表）但描述檔在 4 MiB 內沒有留下來時，
//!    整張封面視為不可信、畫佔位，不進第 3 步（否則解碼會以 64 MiB 的預算把同一顆炸彈再解一次）。
//!    代價：描述檔損毀（不是過大）的真實封面也會變成佔位，這種封面很少見。
//!    其他壓縮的附屬區塊（zTXt／iTXt）在讀檔頭時只保留壓縮後的位元組（並從同一預算扣），`image` 從不要求解壓，
//!    壓縮後的大小又受第 1 步的 8 MiB 所限。
//! 3. 解碼時再套 `image` 的 `Limits`（寬高同上、`max_alloc` = [`MAX_DECODE_ALLOC`]）：`image` 0.25.10 的
//!    `ImageReader::decode` 先以 `Limits::reserve(decoder.total_bytes())` 檢查整個輸出緩衝區，再 `set_limits`
//!    檢查寬高，所以 16 位元或其他會超過上限的輸出也會被拒絕。解碼會**重新**讀一次檔頭（`image` 0.25.10 的
//!    `PngDecoder::set_limits` 不會改 png 內部的上限，見它的 TODO，所以不能沿用第 2 步那個 4 MiB 的解碼器），
//!    這時 iCCP 的解壓以 64 MiB 為上限：超過就停下並丟掉；沒超過的描述檔會從同一份 64 MiB 預算扣掉，
//!    所以 PNG 的「描述檔 + 像素」合計仍 ≤ 64 MiB（審查者實測同一顆炸彈走解碼路徑峰值約 69 MB）。
//! 4. 裁切用 `imageops::crop_imm` 的**檢視**（不複製），直接縮成 ≤ 160 × 160 的縮圖。
//!
//! **實際峰值（誠實的上界）。** 解碼後的原圖最大 3072 × 3072 × 4 = 36 MiB（在 `Limits` 之內），但有些解碼器的
//! 暫存緩衝區**不在** `Limits` 的計算裡：審查者在 4096² 實測漸進式 4:4:4 JPEG 約 150 MiB、無 alpha 的
//! 無損 WebP 約 112 MiB 峰值。依面積換算到 3072²（× 0.5625）約為 85 MiB 與 63 MiB，所以一次解碼的實際峰值
//! 約 **≤ 90 MiB**（漸進式 JPEG 最差；本版沒有在 3072² 另外實測）。這就是把邊長上限從 4096 降到 3072 的原因：
//! 真實的封面遠小於此（常見 600–1400 像素）。
//! 5. 同時最多一個解碼（[`DecodeQueue`]）：只排隊最新要求的封面，被取代的要求直接丟掉，
//!    已不需要的結果在完成時丟棄。

use std::io::Cursor;

use gpui::{ImageFormat, RenderImage};
use image::{imageops, Frame, ImageReader, Limits};

/// 解碼後縮圖的邊長上限（像素）。Expanded 封面 68 點在 2× 螢幕是 136 像素；留一點餘裕。
pub const COVER_TEXTURE_PIXELS: u32 = 160;
/// 封面位元組的上限：超過就不解碼（真實封面通常是數十到數百 KB）。
pub const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;
/// 解碼的寬、高上限（像素）：防止惡意或損毀的位元組宣稱超大尺寸而吃光記憶體。3072 仍遠大於任何真實封面，
/// 而且讓不在 `Limits` 計算內的解碼器暫存區（漸進式 JPEG、無損 WebP）的峰值也維持在約 90 MiB 以下（見模組文件）。
pub const MAX_DECODE_DIMENSION: u32 = 3072;
/// 解碼可配置的記憶體上限（位元組）：涵蓋 3072 × 3072 × 4（RGBA8，36 MiB）的輸出與 PNG 附屬區塊。
pub const MAX_DECODE_ALLOC: u64 = 64 * 1024 * 1024;
/// 檔頭檢查（[`cover_dimensions`]）可配置的記憶體上限：只夠讀檔頭與正常大小的附屬區塊
/// （真實的 ICC 描述檔通常是數 KB 到數百 KB），用來擋下解壓炸彈。
pub const HEADER_CHECK_ALLOC: u64 = 4 * 1024 * 1024;
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

/// 檔頭檢查用的 `Limits`：只限制配置量（[`HEADER_CHECK_ALLOC`]）；寬高由 [`cover_dimensions`] 自己比較。
fn header_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(HEADER_CHECK_ALLOC);
    limits
}

/// 解碼用的 `Limits`：寬高 [`MAX_DECODE_DIMENSION`]、配置量 [`MAX_DECODE_ALLOC`]。
fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIMENSION);
    limits.max_image_height = Some(MAX_DECODE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    limits
}

/// 解碼前的檢查：位元組數、格式與檔頭宣告的寬高都在上限內時回傳 `(格式, 寬, 高)`。
/// 只讀檔頭（`ImageReader::into_dimensions`），不配置像素緩衝區；讀檔頭途中的配置（例如 PNG 的 iCCP 解壓）
/// 以 [`HEADER_CHECK_ALLOC`] 為上限，超過即拒絕。
pub fn cover_dimensions(bytes: &[u8]) -> Option<(image::ImageFormat, u32, u32)> {
    if bytes.len() > MAX_COVER_BYTES {
        return None;
    }
    let format = to_image_format(detect_image_format(bytes)?)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(header_limits());
    let mut decoder = reader.into_decoder().ok()?;
    let (w, h) = image::ImageDecoder::dimensions(&decoder);
    if format == image::ImageFormat::Png
        && png_has_iccp(bytes)
        && image::ImageDecoder::icc_profile(&mut decoder)
            .ok()
            .flatten()
            .is_none()
    {
        // 描述檔在 4 MiB 內沒有留下來（太大被丟掉，或損毀）：不再以更大的預算解碼它。
        return None;
    }
    drop(decoder);
    let ok = |d: u32| d > 0 && d <= MAX_DECODE_DIMENSION;
    (ok(w) && ok(h)).then_some((format, w, h))
}

/// PNG 在第一個 IDAT 之前有沒有 iCCP 區塊：只讀區塊表（長度與型別），不解壓。格式不完整時回 false。
pub fn png_has_iccp(bytes: &[u8]) -> bool {
    let mut pos = 8usize; // 簽章
    while let Some(header) = bytes.get(pos..pos + 8) {
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        match &header[4..8] {
            b"iCCP" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        // 長度 + 型別 + 資料 + CRC。
        pos = match pos.checked_add(12).and_then(|p| p.checked_add(len)) {
            Some(next) => next,
            None => return false,
        };
    }
    false
}

/// 解碼封面：[`cover_dimensions`] 檢查 → 解碼（有尺寸與記憶體上限）→ 以檢視從中央裁成正方形 →
/// 大於 `max_pixels` 時直接縮成縮圖 → 轉成 GPUI 的 BGRA [`RenderImage`]。
/// 不是圖片、格式不支援、太大或尺寸為 0 時回 `None`。會花幾毫秒到幾十毫秒，請在背景執行緒呼叫。
pub fn decode_cover(bytes: &[u8], max_pixels: u32) -> Option<RenderImage> {
    let (format, _, _) = cover_dimensions(bytes)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(decode_limits());
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

    fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }

    /// PNG 簽章、IHDR（宣告寬高）、`ancillary` 附屬區塊、一個空 IDAT：沒有像素資料。
    /// PNG 解碼器讀檔頭時會讀到第一個 IDAT 為止，所以要有它。
    fn png_with_chunks(w: u32, h: u32, ancillary: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 位元 RGBA
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        push_chunk(&mut out, b"IHDR", &ihdr);
        for (kind, data) in ancillary {
            push_chunk(&mut out, kind, data);
        }
        push_chunk(&mut out, b"IDAT", &[]);
        out
    }

    /// 只有 PNG 簽章、IHDR（宣告寬高）與一個空 IDAT 的檔，沒有像素資料。
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        png_with_chunks(w, h, &[])
    }

    /// 解壓後是 `1 + 258 × refs` 個 0 位元組的 zlib 串流（固定 Huffman：一個字面 0，之後每個
    /// 「長度 258、距離 1」的回溯 13 位元），所以壓縮比約 1:160。只寫出壓縮後的位元組，
    /// 測試本身不配置解壓後的大小。回傳（串流, 解壓後長度）。
    fn zlib_zeros(refs: usize) -> (Vec<u8>, u64) {
        struct Bits {
            out: Vec<u8>,
            acc: u64,
            n: u32,
        }
        impl Bits {
            /// 依 deflate 的規則從最低位元開始寫。
            fn put(&mut self, value: u32, len: u32) {
                self.acc |= u64::from(value) << self.n;
                self.n += len;
                while self.n >= 8 {
                    self.out.push(self.acc as u8);
                    self.acc >>= 8;
                    self.n -= 8;
                }
            }
            /// Huffman 碼以最高位元先寫：反轉後再 `put`。
            fn huff(&mut self, code: u32, len: u32) {
                self.put(code.reverse_bits() >> (32 - len), len);
            }
        }
        let mut bits = Bits {
            out: vec![0x78, 0x9C],
            acc: 0,
            n: 0,
        };
        bits.put(1, 1); // BFINAL
        bits.put(1, 2); // BTYPE = 01（固定 Huffman）
        bits.huff(0x30, 8); // 字面 0
        for _ in 0..refs {
            bits.huff(0xC5, 8); // 長度碼 285 = 258
            bits.huff(0, 5); // 距離碼 0 = 1
        }
        bits.huff(0, 7); // 區塊結尾 256
        if bits.n > 0 {
            bits.out.push(bits.acc as u8);
        }
        let len = 1 + 258 * refs as u64;
        // 全是 0 的 Adler-32：a 一直是 1，b = 長度 mod 65521。
        let adler = (((len % 65521) as u32) << 16) | 1;
        bits.out.extend_from_slice(&adler.to_be_bytes());
        (bits.out, len)
    }

    /// 1 × 1 的 PNG，帶一個解壓後有 `1 + 258 × refs` 位元組的 iCCP 區塊。
    fn png_with_iccp(refs: usize) -> (Vec<u8>, u64) {
        let (stream, len) = zlib_zeros(refs);
        let mut iccp = b"bomb\0\0".to_vec(); // 名稱、NUL、壓縮方法 0
        iccp.extend_from_slice(&stream);
        (png_with_chunks(1, 1, &[(b"iCCP", iccp)]), len)
    }

    #[test]
    fn small_iccp_profiles_still_pass_the_header_check() {
        // 64 KiB 的描述檔：正常大小，檔頭檢查照常讀出寬高（證明 iCCP 建構正確、會被解壓）。
        let (png, len) = png_with_iccp(254);
        assert!(len < HEADER_CHECK_ALLOC);
        assert_eq!(
            cover_dimensions(&png),
            Some((image::ImageFormat::Png, 1, 1))
        );
        assert_eq!(icc_len(&png, header_limits()), Some(len as usize));
    }

    /// 用 `limits` 讀檔頭後拿到的 ICC 描述檔長度（被丟掉時為 `None`）。
    fn icc_len(png: &[u8], limits: Limits) -> Option<usize> {
        use image::ImageDecoder;
        let mut reader = ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().expect("the header itself is valid");
        decoder
            .icc_profile()
            .expect("png reports Ok")
            .map(|p| p.len())
    }

    #[test]
    fn iccp_bomb_is_cut_off_by_the_bounded_header_check() {
        // 約 17 MiB 的描述檔（壓縮後約 110 KB）：遠大於檔頭檢查的 4 MiB 上限。
        let (bomb, len) = png_with_iccp(70_000);
        assert!(bomb.len() < 1024 * 1024, "{}", bomb.len());
        assert!(len > 4 * HEADER_CHECK_ALLOC, "{len}");
        // 串流是真的：上限夠大時描述檔完整解出來（這裡配置約 17 MiB，只是對照組）。
        let mut generous = Limits::default();
        generous.max_alloc = Some(64 * 1024 * 1024);
        assert_eq!(icc_len(&bomb, generous), Some(len as usize));
        // 檔頭檢查的上限：解壓到 4 MiB 就停下、描述檔被丟掉（png 0.18.1 不把它當錯誤）。
        assert_eq!(icc_len(&bomb, header_limits()), None);
        // 有 iCCP 但描述檔沒留下來：整張封面被拒絕，不會再以 64 MiB 的預算解碼。
        assert!(png_has_iccp(&bomb));
        assert_eq!(cover_dimensions(&bomb), None);
        assert!(decode_cover(&bomb, 160).is_none());
    }

    #[test]
    fn png_has_iccp_walks_the_chunk_table_only() {
        let (with, _) = png_with_iccp(1);
        assert!(png_has_iccp(&with));
        assert!(!png_has_iccp(&png_header(10, 10)));
        // 截斷或長度溢位的區塊表不會 panic。
        assert!(!png_has_iccp(&with[..12]));
        let mut bad = png_header(10, 10);
        bad[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(!png_has_iccp(&bad));
        assert!(!png_has_iccp(b""));
    }

    #[test]
    fn decode_path_also_bounds_ancillary_decompression() {
        // 約 80 MiB 的描述檔（壓縮後約 520 KB）走解碼的上限：解壓到 64 MiB 就停下並丟掉
        // （測試期間最多配置 64 MiB）。正常大小的描述檔在同樣的上限下照常保留。
        let (bomb, len) = png_with_iccp(325_000);
        assert!(bomb.len() < 1024 * 1024, "{}", bomb.len());
        assert!(len > MAX_DECODE_ALLOC, "{len}");
        assert_eq!(icc_len(&bomb, decode_limits()), None);
        let (small, small_len) = png_with_iccp(254);
        assert_eq!(icc_len(&small, decode_limits()), Some(small_len as usize));
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
        // 上限本身：3072² × 4（36 MiB）放得進解碼預算。
        assert_eq!(MAX_DECODE_DIMENSION, 3072);
        assert!(u64::from(MAX_DECODE_DIMENSION).pow(2) * 4 <= MAX_DECODE_ALLOC);
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
