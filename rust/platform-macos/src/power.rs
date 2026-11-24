//! 系統從睡眠喚醒的通知（`NSWorkspaceDidWakeNotification`，公開的 AppKit API）。
//!
//! Swift 版的 `MediaRemoteSource` 自己訂閱這個通知；Rust 版的 `HelperSupervisor` 是純邏輯，由 app 在這裡登記，
//! 收到通知時呼叫 `HelperSupervisor::wake_from_sleep()`。
//!
//! 這是 `panel.rs`、`tray.rs` 之外唯一寫 Objective-C 的檔案：只用 `NSWorkspace.sharedWorkspace.notificationCenter`
//! 的 `addObserverForName:object:queue:usingBlock:`／`removeObserver:`（不是視窗繞路、不碰私有 API）。
//!
//! 執行緒：[`observe_wake`] 必須在主執行緒呼叫（否則印警告並回 `None`）；回呼登記在**主佇列**
//! （`NSOperationQueue.mainQueue`），所以**一律在主執行緒上呼叫**，與 GPUI 的前景工作同一條執行緒，
//! 但不在任何 GPUI 的 `update` 之內：回呼裡不能借用 GPUI 的 `App`。回呼應盡快返回。
//! [`WakeObserver`] 不是 `Send`；drop 它即取消登記（之後不會再被呼叫）。

use std::marker::PhantomData;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue};

/// 一筆喚醒通知的登記。drop 即從通知中心移除。只能在主執行緒持有與釋放（不是 `Send`）。
#[must_use = "dropping the WakeObserver unregisters the wake callback"]
pub struct WakeObserver {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    _main_thread_only: PhantomData<*const ()>,
}

impl std::fmt::Debug for WakeObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WakeObserver")
    }
}

/// 在主執行緒登記「系統喚醒」的回呼；回呼在主執行緒上呼叫（見模組文件）。
/// 不在主執行緒時印警告並回 `None`。
pub fn observe_wake(on_wake: Box<dyn Fn() + Send + Sync>) -> Option<WakeObserver> {
    if MainThreadMarker::new().is_none() {
        eprintln!("musicat: observe_wake() must be called on the main thread; wake events are not observed");
        return None;
    }
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let queue = NSOperationQueue::mainQueue();
    let block = RcBlock::new(move |_note: NonNull<NSNotification>| on_wake());
    // SAFETY:
    // - `NSWorkspaceDidWakeNotification` 是 AppKit 匯出的常數字串（extern static），讀取它是安全的。
    // - `object` 為 nil：接收任何發送者的通知。
    // - block 的簽章（`void (^)(NSNotification *)`）與 API 相符；它捕獲的回呼是 `Send + Sync`，
    //   而且通知中心只會在 `queue`（主佇列）上呼叫它。
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidWakeNotification),
            None,
            Some(&queue),
            &block,
        )
    };
    Some(WakeObserver {
        center,
        token,
        _main_thread_only: PhantomData,
    })
}

impl Drop for WakeObserver {
    fn drop(&mut self) {
        // SAFETY: `token` 是同一個通知中心的 `addObserverForName:…` 回傳的觀察者物件。
        unsafe { self.center.removeObserver(self.token.as_ref()) };
    }
}

/// 在 NSWorkspace 的通知中心發出一則 `NSWorkspaceDidWakeNotification`（測試與煙霧範例用；
/// **不會**讓機器睡眠或喚醒）。已登記的回呼在主佇列上被呼叫。
///
/// 可以在任何執行緒呼叫，但**從主執行緒以外呼叫時會阻塞到主佇列上的回呼跑完**（NSNotificationCenter 的
/// 同步投遞語意），所以主執行緒不能同時等待那個呼叫端（例如 `join`），否則互相等待。
#[doc(hidden)]
pub fn post_wake_for_test() {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: 讀取 AppKit 匯出的常數字串；object 為 nil。
    unsafe { center.postNotificationName_object(NSWorkspaceDidWakeNotification, None) };
}
