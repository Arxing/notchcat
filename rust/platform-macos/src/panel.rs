//! 瀏海面板的 AppKit 視窗設定：MusiCat 中**唯一**使用 objc2 繞過 GPUI 的地方。
//!
//! GPUI 0.2.2 沒有提供「無邊框、不搶焦點、高於選單列、跨所有空間」的面板，
//! 所以 `shell` 用 GPUI 建出視窗後，交給這裡的函式在 AppKit 層補設定（spec 第 7.3 節）。
//! `shell` 與 `app` 只能呼叫本模組的公開函式，自己不碰 Objective-C。
//!
//! 繞路一覽（全部對應 GPUI 0.2.2；升級 GPUI 時逐項重驗，見 `rust/docs/HANDOFF-rust.md`）：
//!
//! 1. [`configure_panel`]：視窗建立後才切成無邊框 + nonactivating panel 並設定層級、空間行為與位置。
//! 2. [`patch_gpui_panel_class`]：把 GPUI 私有類別 `GPUIPanel` 的 `canBecomeKeyWindow`／
//!    `canBecomeMainWindow` 換成回傳 NO。最脆弱的一項。
//! 3. [`set_accessory_activation_policy`]：把 GPUI 啟動時設成 Regular 的啟動模式改為 Accessory（不顯示 Dock 圖示）。
//! 4. （不在本檔）GPUI 的 Cargo feature `runtime_shaders`：見 `rust/shell/Cargo.toml` 與 `rust/README.md`。
//!
//! 座標：所有 [`Rect`]／[`Point`] 都是 MusiCat 全域座標（主螢幕左上原點、y 向下），
//! 定義與換算見 [`crate::screens`]。
//!
//! 執行緒：除了 [`mouse_location`] 與 [`foreground_is_fullscreen`]，其餘函式都必須在主執行緒呼叫
//! （GPUI 的 `App`／`Window` 回呼本來就在主執行緒）。在其他執行緒呼叫時印警告並什麼都不做，不會崩潰。

use std::ffi::c_void;

use musicat_core::shell_logic::{Point, Rect};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::sel;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSColor, NSEvent, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

use crate::screens::{appkit_point_to_global, global_rect_to_appkit, primary_screen_height};

/// 面板的視窗層級：`NSStatusWindowLevel`（25），高於選單列的 `NSMainMenuWindowLevel`（24），
/// 與 Swift 版 `NotchPanel` 的 `.statusBar` 相同。GPUI 的 PopUp 預設是 101（`NSPopUpMenuWindowLevel`），會蓋過系統選單。
pub const PANEL_WINDOW_LEVEL: isize = 25;

/// GPUI 視窗在 AppKit 端的把手：GPUI 經 `raw-window-handle` 的 `AppKitWindowHandle::ns_view`
/// 交出的 **NSView**（GPUI 自己的 `GPUIView`）指標，不是 NSWindow。
/// 面板的 NSWindow 由 `-[NSView window]` 取得。
///
/// 只能用 `unsafe` 的 [`NativeWindow::from_ns_view`] 建立（欄位是私有的），所以接收它的
/// [`configure_panel`]、[`set_ignores_mouse_events`]、[`describe_panel`] 可以是安全函式：
/// 指標的有效性由建立把手的那一處 `unsafe` 負責。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeWindow(*mut c_void);

impl NativeWindow {
    /// 由 GPUI 交出的 NSView 指標（`AppKitWindowHandle::ns_view`）建立把手。
    ///
    /// # Safety
    ///
    /// - `ptr` 必須是 GPUI 視窗的 NSView（`GPUIView`），而且在這個把手（與它的所有複本）被使用的整段期間，
    ///   該視窗都保持開啟、view 保持存活；視窗關閉後不得再把把手傳給本模組的任何函式。
    /// - 把手只能在主執行緒使用（本模組的函式在其他執行緒會拒絕，但指標本身也不得跨執行緒解參照）。
    ///
    /// 空指標是允許的：各函式會拒絕它（印警告、不做事）。
    pub unsafe fn from_ns_view(ptr: *mut c_void) -> Self {
        Self(ptr)
    }

    /// 取得 NSView 所在的 NSWindow。空指標或 view 還沒放進視窗時回 `None`。
    fn ns_window(self, _mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
        if self.0.is_null() {
            return None;
        }
        // SAFETY: 把手只能經 `unsafe fn from_ns_view` 建立，其呼叫端保證指標是仍存活的 GPUI NSView；
        // 我們在主執行緒（持有 MainThreadMarker）。只借用、不轉移所有權。
        let view: &NSView = unsafe { &*self.0.cast::<NSView>() };
        view.window()
    }
}

fn main_thread(what: &str) -> Option<MainThreadMarker> {
    let mtm = MainThreadMarker::new();
    if mtm.is_none() {
        eprintln!("musicat: {what} must run on the main thread; ignored");
    }
    mtm
}

/// 繞路 3：把 App 的啟動模式設成 Accessory（不顯示 Dock 圖示、不出現在 Cmd-Tab）。
///
/// 為什麼需要：GPUI 0.2.2 在 `applicationDidFinishLaunching:` 裡無條件呼叫
/// `setActivationPolicy:NSApplicationActivationPolicyRegular`，沒有選項可關。
/// 瀏海面板是常駐的系統層 UI，不該佔 Dock 與 App 切換器。
///
/// 呼叫時機（對應 GPUI 0.2.2）：必須在 GPUI 完成啟動**之後**、建立任何視窗**之前**，
/// 也就是 `Application::run` 的回呼一開始。在 `run` 之前呼叫會被 GPUI 的啟動流程蓋回 Regular。
pub fn set_accessory_activation_policy() {
    let Some(mtm) = main_thread("set_accessory_activation_policy()") else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
        eprintln!("musicat: setActivationPolicy(Accessory) was refused by AppKit");
    }
}

/// 取代 `canBecomeKeyWindow`／`canBecomeMainWindow` 的實作：永遠回 NO。
extern "C-unwind" fn answer_no(_this: *mut AnyObject, _sel: Sel) -> Bool {
    Bool::NO
}

/// 繞路 2：把 GPUI 私有類別 `GPUIPanel` 的 `canBecomeKeyWindow`、`canBecomeMainWindow` 換成回傳 NO。
///
/// 為什麼需要：GPUI 0.2.2 為 `WindowKind::PopUp` 建立的 `GPUIPanel`（NSPanel 子類別）兩個方法都回 YES，
/// 點一下面板它就會變成 key window、搶走前景 App 的鍵盤焦點。Swift 版的 `NotchPanel` 是覆寫這兩個屬性回 false；
/// GPUI 不讓我們提供自己的視窗類別，只好在執行時替換類別的方法。
///
/// 影響範圍：整個行程裡所有 GPUI PopUp 視窗（MusiCat 只有這一個面板用 PopUp）。
/// 類別名稱與方法是 GPUI 0.2.2 的內部實作（`gpui/src/platform/mac/window.rs` 的 `build_window_class`），
/// GPUI 升級後可能改名或消失：找不到類別時只印警告並繼續執行（面板仍能顯示，只是點擊時可能搶焦點），不會崩潰。
///
/// 呼叫時機：GPUI 0.2.2 在行程載入時（`#[ctor]`）就註冊類別，所以任何時候呼叫都可以；
/// 慣例上在 `Application::new()` 之後、建立視窗之前呼叫一次。重複呼叫無害。
pub fn patch_gpui_panel_class() {
    let Some(cls) = AnyClass::get(c"GPUIPanel") else {
        eprintln!(
            "musicat: WARNING: GPUI class `GPUIPanel` not found (GPUI version changed?); \
             the panel may take keyboard focus when clicked"
        );
        return;
    };
    // SAFETY: `answer_no` 的簽章與 `- (BOOL)canBecome…Window` 相符（self、_cmd，回傳 BOOL；
    // objc2 的 `Bool` 在每個架構上都與 Objective-C 的 BOOL 同 ABI：arm64 為 bool，x86_64 為 signed char）。
    // 型別編碼 "B@:" 只在 arm64 上與 BOOL 的實際編碼相同（x86_64 是 "c@:"），但兩個方法在 GPUIPanel 上都已存在，
    // class_replaceMethod 替換既有方法時會忽略這個參數、沿用原本的編碼，所以不影響正確性。
    // class_replaceMethod 是執行緒安全的 runtime 呼叫；類別由 GPUI 擁有且終身存在。
    unsafe {
        let imp: Imp = std::mem::transmute::<extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool, Imp>(
            answer_no,
        );
        for sel in [sel!(canBecomeKeyWindow), sel!(canBecomeMainWindow)] {
            objc2::ffi::class_replaceMethod(
                (cls as *const AnyClass).cast_mut(),
                sel,
                imp,
                c"B@:".as_ptr(),
            );
        }
    }
}

/// 繞路 1：GPUI 視窗建立之後，把它改成瀏海面板並放到 `frame`（MusiCat 全域座標）。
///
/// 為什麼需要（對應 GPUI 0.2.2）：GPUI 的 `WindowOptions` 不論 `titlebar: None` 與否都建出
/// `Titled | FullSizeContentView` 的視窗（PopUp 再加 NonactivatingPanel）。有 Titled 的視窗會被 AppKit
/// 限制在選單列下方、帶視窗外框與陰影；層級（PopUp 為 101）與空間行為也不是我們要的。
/// GPUI 沒有對應的選項，只能在建立後直接改 NSWindow。設定內容與 Swift 版 `NotchPanel` 相同：
///
/// - style mask：Borderless | NonactivatingPanel（點擊不啟動 App）
/// - 不透明 = NO、背景透明、無陰影、不可拖動、App 失去前景時不隱藏
/// - 層級 [`PANEL_WINDOW_LEVEL`]（25，高於選單列的 24）
/// - 空間行為：CanJoinAllSpaces | Stationary | FullScreenAuxiliary | IgnoresCycle
/// - `setFrame:` 到 `frame`（由 MusiCat 全域座標換成 AppKit 的左下原點座標），再 `orderFrontRegardless`
///   （不經過 App 啟動就顯示到最前面，不搶焦點）
///
/// 視窗外框終身不變：之後換螢幕時再呼叫一次即可（重設上述屬性無害）。
pub fn configure_panel(window: NativeWindow, frame: Rect) {
    let Some(mtm) = main_thread("configure_panel()") else {
        return;
    };
    let Some(win) = window.ns_window(mtm) else {
        eprintln!("musicat: configure_panel(): no NSWindow behind the given view; ignored");
        return;
    };
    let f = global_rect_to_appkit(frame, primary_screen_height());
    let appkit_frame = NSRect::new(NSPoint::new(f.x, f.y), NSSize::new(f.w, f.h));

    win.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    win.setOpaque(false);
    win.setBackgroundColor(Some(&NSColor::clearColor()));
    win.setHasShadow(false);
    win.setMovable(false);
    win.setHidesOnDeactivate(false);
    win.setLevel(PANEL_WINDOW_LEVEL);
    win.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    win.setFrame_display(appkit_frame, true);
    win.orderFrontRegardless();
}

/// 讓面板整個忽略（`true`）或接收（`false`）滑鼠事件，用來做點擊穿透：
/// 游標不在畫出來的外形上時打開，點擊就落到下層視窗。
///
/// 為什麼需要（對應 GPUI 0.2.2）：GPUI 的透明視窗仍會吃下透明區域的點擊（Metal layer 不做逐像素命中測試），
/// 而 GPUI 沒有 `ignoresMouseEvents` 的選項。spike 以每 16 ms 輪詢游標位置切換這個屬性。
pub fn set_ignores_mouse_events(window: NativeWindow, ignore: bool) {
    let Some(mtm) = main_thread("set_ignores_mouse_events()") else {
        return;
    };
    if let Some(win) = window.ns_window(mtm) {
        win.setIgnoresMouseEvents(ignore);
    }
}

/// 目前的全域滑鼠位置，MusiCat 全域座標（主螢幕左上原點、y 向下；與 `ScreenInfo.frame` 同一空間）。
///
/// 由 `+[NSEvent mouseLocation]`（AppKit 全域、左下原點）以主螢幕高度翻轉而得。任何執行緒都可呼叫。
/// 游標貼在螢幕最上緣時 AppKit 回報 `y == 螢幕頂端`，換算後是 `y == 0`（落在左上邊界含入的 `Rect::contains` 內）。
pub fn mouse_location() -> Point {
    let p = NSEvent::mouseLocation();
    appkit_point_to_global(Point { x: p.x, y: p.y }, primary_screen_height())
}

/// 前景 App 是否為全螢幕。**刻意尚未實作，固定回 `false`。**
///
/// spec 第 5.3 節：若 macOS 本來就會在其他 App 的全螢幕空間裡隱藏我們的跨空間視窗，就以實測為準、
/// 不要硬加偵測。這需要在作者的 Mac 上實測（`rust/docs/HANDOFF-rust.md` 的「GPUI 視窗驗證（Task 21）」）。
/// 注意面板設了 `FullScreenAuxiliary`，它**可能**會出現在全螢幕空間裡，所以結果不能預設。
///
/// 若實測發現面板會留在全螢幕 App 之上，計畫改為：以 `CGWindowListCopyWindowInfo`
/// （`OnScreenOnly | ExcludeDesktopElements`）取前景 App（`NSWorkspace.frontmostApplication` 的 pid）
/// 在第 0 層、且 bounds 完全覆蓋面板所在螢幕 `CGDisplayBounds` 的視窗；有就回 `true`。
/// 由 app 以低頻率（例如每秒）輪詢，結果交給 core 決定隱藏面板。
pub fn foreground_is_fullscreen() -> bool {
    false
}

/// 給診斷用的一行視窗狀態（層級、AppKit 外框、style mask、key/main 等），格式仿 spike 的 `describe`。
pub fn describe_panel(window: NativeWindow) -> String {
    let Some(mtm) = MainThreadMarker::new() else {
        return "describe_panel(): not on the main thread".to_string();
    };
    let Some(win) = window.ns_window(mtm) else {
        return "describe_panel(): no NSWindow behind the given view".to_string();
    };
    let f = win.frame();
    let class_name = win.class().name().to_string_lossy().into_owned();
    format!(
        "class={} windowNumber={} level={} appkitFrame=({}, {}, {}x{}) styleMask={:?} opaque={} \
         shadow={} key={} main={} canBecomeKey={} canBecomeMain={} ignoresMouse={} behavior={:?}",
        class_name,
        win.windowNumber(),
        win.level(),
        f.origin.x,
        f.origin.y,
        f.size.width,
        f.size.height,
        win.styleMask(),
        win.isOpaque(),
        win.hasShadow(),
        win.isKeyWindow(),
        win.isMainWindow(),
        win.canBecomeKeyWindow(),
        win.canBecomeMainWindow(),
        win.ignoresMouseEvents(),
        win.collectionBehavior(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mouse_location()` 與 CoreGraphics 的全域顯示座標（左上原點，`CGEvent` 的 location）一致：
    /// 驗證「MusiCat 全域座標 = CG 全域顯示座標」的定義在真機上成立。游標在兩次取樣間移動時重試。
    /// 需要視窗伺服器的工作階段（依工作區慣例標 `#[ignore]`，不進 CI）：
    /// `cargo test -p musicat-platform-macos -- --ignored mouse_location`。
    #[test]
    #[ignore = "needs a window-server session"]
    fn mouse_location_matches_core_graphics_global_space() {
        use core_graphics::event::CGEvent;
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

        let mut last = None;
        for _ in 0..20 {
            let ours = mouse_location();
            let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
                eprintln!("no CGEventSource (headless?); skipping");
                return;
            };
            let Ok(event) = CGEvent::new(source) else {
                eprintln!("no CGEvent (headless?); skipping");
                return;
            };
            let cg = event.location();
            if (ours.x - cg.x).abs() <= 1.0 && (ours.y - cg.y).abs() <= 1.0 {
                return;
            }
            last = Some((ours, (cg.x, cg.y)));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("mouse_location() and CGEvent location disagree: {last:?}");
    }

    #[test]
    fn calls_off_the_main_thread_do_not_crash() {
        // cargo test 的測試通常在非主執行緒跑：這些呼叫都必須安全地什麼都不做。
        if MainThreadMarker::new().is_some() {
            eprintln!("test runs on the main thread; skipping");
            return;
        }
        // SAFETY: 空指標由各函式拒絕，不會被解參照。
        let bogus = unsafe { NativeWindow::from_ns_view(std::ptr::null_mut()) };
        set_accessory_activation_policy();
        configure_panel(bogus, Rect::default());
        set_ignores_mouse_events(bogus, true);
        assert!(describe_panel(bogus).contains("not on the main thread"));
        assert!(!foreground_is_fullscreen());
    }

    #[test]
    fn patching_without_gpui_loaded_only_warns() {
        // 這個測試行程沒有連結 GPUI，類別不存在：必須回來而不是崩潰。
        patch_gpui_panel_class();
    }

    #[test]
    fn panel_level_is_above_the_menu_bar() {
        assert_eq!(PANEL_WINDOW_LEVEL, 25);
    }
}
