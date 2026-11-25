//! 模組狀態與管理器（移植自 Swift `ModuleKit/ModuleManager.swift` 與 `NotchModule.swift`）。
//!
//! [`ModuleState`] 只含狀態；圖示、Preview／Expanded 的畫面在 `shell`，所以 core 不依賴 GPUI。
//! [`ModuleManager`] 持有所有已安裝模組，算出目前的 active 模組與 Preview 額外高度，
//! 並且擁有「模組切換列目前選取」的狀態（外殼在每次新展開時重設，見 spec 5.3）。
//!
//! 並行：管理器的狀態由單一 mutex 保護；通知訂閱者一律在**鎖外**。為了讓「重新計算」與「存入結果」
//! 不被別的執行緒交錯，重新計算在鎖內呼叫各模組的 `is_active`／`preview_extra_height`
//! （模組的這兩個方法只讀自己的狀態，且模組一律在自己的鎖外通知，所以不會死結）。
//! 訂閱者收到通知後應重新讀取管理器的狀態，不要依賴通知的順序。
//! 管理器對模組的訂閱回呼只持有管理器內部狀態的 `Weak`；drop 管理器即取消所有訂閱。

use std::sync::{Arc, Mutex, Weak};

use crate::runtime::{lock, Notifier, Subscription};

/// Preview 額外高度的上限（Swift `NotchMode.maxPreviewExtraHeight` 是 28；這裡放寬到 56，
/// 放得下三行的歌詞條，見 `music::module::lyric_strip_height`）。
pub const MAX_PREVIEW_EXTRA_HEIGHT: f64 = 56.0;

/// 夾到 `0..=MAX_PREVIEW_EXTRA_HEIGHT`；NaN 視為 0。
pub fn clamped_preview_extra(height: f64) -> f64 {
    if height.is_nan() {
        0.0
    } else {
        height.clamp(0.0, MAX_PREVIEW_EXTRA_HEIGHT)
    }
}

/// 可安裝進外殼的功能單元的狀態面（Swift `NotchModule` 去掉畫面的部分）。
pub trait ModuleState: Send + Sync {
    /// 唯一識別，例如 `"com.musicat.music"`。
    fn id(&self) -> &str;

    /// 多個模組同時 active 時，priority 最高者成為 active 模組；同分時先註冊者勝。
    fn priority(&self) -> i32;

    /// 模組目前是否有「不必 hover 就想顯示」的環境內容（例如音樂正在播）。
    fn is_active(&self) -> bool;

    /// 模組作為 active 模組時，希望 Preview 比瀏海高多少。管理器會夾到 `0..=MAX_PREVIEW_EXTRA_HEIGHT`。預設恆為 0。
    fn preview_extra_height(&self) -> f64 {
        0.0
    }

    /// `is_active` 或 `preview_extra_height` 改變時通知（只在值真的改變時）。回呼可能在任何執行緒上被呼叫，
    /// 且不持有模組的鎖；不會在訂閱當下重播。drop 回傳的把手即取消。
    fn subscribe(&self, f: Box<dyn Fn() + Send + Sync>) -> Subscription;

    /// 註冊後立刻呼叫一次（Swift `onInstall`）。
    fn on_install(&self) {}
    /// 緊接 `on_install`；v1 沒有啟用／停用介面，安裝即啟動（Swift `onStart`）。
    fn on_start(&self) {}
    /// v1 沒有任何地方呼叫（尚無停用介面）。
    fn on_stop(&self) {}
    /// v1 沒有任何地方呼叫（尚無解除安裝介面）。
    fn on_remove(&self) {}
}

struct State {
    modules: Vec<Arc<dyn ModuleState>>,
    /// 對各模組的訂閱；管理器 drop 時一併取消。
    subscriptions: Vec<Subscription>,
    active: Option<String>,
    preview_extra_height: f64,
    selected: Option<String>,
}

struct Inner {
    state: Mutex<State>,
    notifier: Notifier<()>,
}

impl Inner {
    /// 依各模組目前的狀態重算 active 模組與額外高度；有變才通知（鎖外）。
    fn recompute(&self) {
        let changed = {
            let mut st = lock(&self.state);
            // 手動找最大值：`max_by_key` 同分時取最後一個，Swift 的 `max(by:)` 取第一個。
            let mut best: Option<&Arc<dyn ModuleState>> = None;
            for module in &st.modules {
                if module.is_active() && best.is_none_or(|b| module.priority() > b.priority()) {
                    best = Some(module);
                }
            }
            let (active, height) = match best {
                Some(m) => (
                    Some(m.id().to_string()),
                    clamped_preview_extra(m.preview_extra_height()),
                ),
                None => (None, 0.0),
            };
            let changed = st.active != active || st.preview_extra_height != height;
            st.active = active;
            st.preview_extra_height = height;
            changed
        };
        if changed {
            self.notifier.notify(&());
        }
    }
}

/// 持有所有已安裝模組，決定目前 active 的是哪一個，並保存模組切換列的選取。
/// 外殼只透過這個型別與模組互動。
pub struct ModuleManager {
    inner: Arc<Inner>,
}

impl ModuleManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    modules: Vec::new(),
                    subscriptions: Vec::new(),
                    active: None,
                    preview_extra_height: 0.0,
                    selected: None,
                }),
                notifier: Notifier::new(),
            }),
        }
    }

    /// 安裝模組：加入清單，呼叫 `on_install` 再 `on_start`（v1 沒有啟用／停用介面，註冊即啟動），
    /// 之後追蹤它的 active／額外高度。id 已存在的模組會被忽略（不安裝、不啟動）。
    pub fn register(&self, module: Arc<dyn ModuleState>) {
        {
            let mut st = lock(&self.inner.state);
            if st.modules.iter().any(|m| m.id() == module.id()) {
                return;
            }
            st.modules.push(module.clone());
        }
        module.on_install();
        module.on_start();
        // 訂閱在鎖外：模組可能在 subscribe 內就同步通知。回呼只持弱參照，drop 管理器即解除。
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let subscription = module.subscribe(Box::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.recompute();
            }
        }));
        lock(&self.inner.state).subscriptions.push(subscription);
        self.inner.recompute();
    }

    /// 已安裝模組的 id，依註冊順序（模組切換列用）。
    pub fn installed_module_ids(&self) -> Vec<String> {
        lock(&self.inner.state)
            .modules
            .iter()
            .map(|m| m.id().to_string())
            .collect()
    }

    /// 目前 active 的模組（active 者中 priority 最高、同分先註冊者勝）；沒有則 `None`。
    pub fn active_module_id(&self) -> Option<String> {
        lock(&self.inner.state).active.clone()
    }

    /// active 模組要求的 Preview 額外高度，夾到 `0..=MAX_PREVIEW_EXTRA_HEIGHT`；沒有 active 模組為 0。
    pub fn preview_extra_height(&self) -> f64 {
        lock(&self.inner.state).preview_extra_height
    }

    /// 模組切換列目前選取的模組；尚未選取為 `None`。
    pub fn selected_module_id(&self) -> Option<String> {
        lock(&self.inner.state).selected.clone()
    }

    /// 選取模組（顯示它的 Expanded 內容）。未安裝的 id 被忽略。
    pub fn select(&self, id: &str) {
        let changed = {
            let mut st = lock(&self.inner.state);
            if !st.modules.iter().any(|m| m.id() == id) {
                return;
            }
            let changed = st.selected.as_deref() != Some(id);
            st.selected = Some(id.to_string());
            changed
        };
        if changed {
            self.inner.notifier.notify(&());
        }
    }

    /// 每次新展開時由外殼呼叫：選取回到目前 active 的模組（沒有 active 模組則清空，同 Swift
    /// `selectedModule = activeModule`）。
    pub fn reset_selection_to_active(&self) {
        let changed = {
            let mut st = lock(&self.inner.state);
            let changed = st.selected != st.active;
            st.selected = st.active.clone();
            changed
        };
        if changed {
            self.inner.notifier.notify(&());
        }
    }

    /// active 模組、額外高度或選取改變時通知。回呼在鎖外、呼叫端執行緒上執行
    /// （可能是任何執行緒）；回呼內可以讀管理器或再註冊／選取。收到通知後請重新讀取狀態。
    pub fn subscribe(&self, f: Box<dyn Fn() + Send + Sync>) -> Subscription {
        self.inner.notifier.subscribe(Box::new(move |_| f()))
    }
}

impl Default for ModuleManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod extra_tests;
#[cfg(test)]
pub(crate) mod fakes {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 對應 Swift 的 `FakePreviewModule`：狀態由測試直接設定，每次設定都通知（同 `CurrentValueSubject.send`）。
    pub(crate) struct FakeModule {
        id: String,
        priority: i32,
        state: Mutex<(bool, f64)>,
        notifier: Notifier<()>,
        pub(crate) installed: AtomicUsize,
        pub(crate) started: AtomicUsize,
    }

    impl FakeModule {
        pub(crate) fn new(id: &str, priority: i32, is_active: bool, extra: f64) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                priority,
                state: Mutex::new((is_active, extra)),
                notifier: Notifier::new(),
                installed: AtomicUsize::new(0),
                started: AtomicUsize::new(0),
            })
        }

        pub(crate) fn set_active(&self, active: bool) {
            lock(&self.state).0 = active;
            self.notifier.notify(&());
        }

        pub(crate) fn set_extra_height(&self, height: f64) {
            lock(&self.state).1 = height;
            self.notifier.notify(&());
        }
    }

    impl ModuleState for FakeModule {
        fn id(&self) -> &str {
            &self.id
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        fn is_active(&self) -> bool {
            lock(&self.state).0
        }
        fn preview_extra_height(&self) -> f64 {
            lock(&self.state).1
        }
        fn subscribe(&self, f: Box<dyn Fn() + Send + Sync>) -> Subscription {
            self.notifier.subscribe(Box::new(move |_| f()))
        }
        fn on_install(&self) {
            self.installed.fetch_add(1, Ordering::SeqCst);
        }
        fn on_start(&self) {
            self.started.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::FakeModule;
    use super::*;

    /// 不覆寫 `preview_extra_height` 的模組（Swift `PlainModule`）。
    struct PlainModule;

    impl ModuleState for PlainModule {
        fn id(&self) -> &str {
            "test.plain"
        }
        fn priority(&self) -> i32 {
            0
        }
        fn is_active(&self) -> bool {
            true
        }
        fn subscribe(&self, _f: Box<dyn Fn() + Send + Sync>) -> Subscription {
            Subscription::new(|| {})
        }
    }

    #[test]
    fn defaults_to_zero_without_active_module() {
        let manager = ModuleManager::new();
        assert_eq!(manager.preview_extra_height(), 0.0);
        assert_eq!(manager.active_module_id(), None);
    }

    #[test]
    fn follows_active_module_value() {
        let manager = ModuleManager::new();
        let module = FakeModule::new("a", 0, true, 24.0);
        manager.register(module.clone());
        assert_eq!(manager.preview_extra_height(), 24.0);

        module.set_extra_height(10.0);
        assert_eq!(manager.preview_extra_height(), 10.0);
    }

    #[test]
    fn clamps_to_maximum() {
        let manager = ModuleManager::new();
        let module = FakeModule::new("a", 0, true, 500.0);
        manager.register(module.clone());
        assert_eq!(manager.preview_extra_height(), MAX_PREVIEW_EXTRA_HEIGHT);

        module.set_extra_height(-5.0);
        assert_eq!(manager.preview_extra_height(), 0.0);

        module.set_extra_height(99.0);
        assert_eq!(manager.preview_extra_height(), 56.0);

        module.set_extra_height(f64::NAN);
        assert_eq!(manager.preview_extra_height(), 0.0);
    }

    #[test]
    fn inactive_module_does_not_contribute() {
        let manager = ModuleManager::new();
        let module = FakeModule::new("a", 0, false, 24.0);
        manager.register(module.clone());
        assert_eq!(manager.preview_extra_height(), 0.0);

        // 變成 active 才有貢獻；停用後回到 0。
        module.set_active(true);
        assert_eq!(manager.preview_extra_height(), 24.0);

        module.set_active(false);
        assert_eq!(manager.preview_extra_height(), 0.0);
    }

    #[test]
    fn switching_active_module_switches_source() {
        let manager = ModuleManager::new();
        let low = FakeModule::new("low", 1, true, 12.0);
        let high = FakeModule::new("high", 2, true, 20.0);
        manager.register(low.clone());
        assert_eq!(manager.preview_extra_height(), 12.0);

        manager.register(high.clone());
        assert_eq!(manager.preview_extra_height(), 20.0);
        assert_eq!(manager.active_module_id().as_deref(), Some("high"));

        // 只有 active 模組的值算數。
        low.set_extra_height(5.0);
        assert_eq!(manager.preview_extra_height(), 20.0);

        high.set_active(false);
        assert_eq!(manager.preview_extra_height(), 5.0);
        assert_eq!(manager.active_module_id().as_deref(), Some("low"));

        low.set_active(false);
        assert_eq!(manager.preview_extra_height(), 0.0);
        assert_eq!(manager.active_module_id(), None);
    }

    /// Swift `testClampedPreviewExtra`（Swift 的 `NotchMode.clampedPreviewExtra`；NaN 另見 extra_tests）。
    #[test]
    fn clamped_preview_extra_clamps_to_zero_through_maximum() {
        assert_eq!(MAX_PREVIEW_EXTRA_HEIGHT, 56.0);
        assert_eq!(clamped_preview_extra(-5.0), 0.0);
        assert_eq!(clamped_preview_extra(0.0), 0.0);
        assert_eq!(clamped_preview_extra(24.0), 24.0);
        assert_eq!(clamped_preview_extra(56.0), 56.0);
        assert_eq!(clamped_preview_extra(500.0), 56.0);
    }

    /// Swift `testNotchModuleDefaultIsZero`：沒覆寫額外高度的模組，預設就是 0（trait 的預設方法）。
    #[test]
    fn notch_module_default_is_zero() {
        assert_eq!(PlainModule.preview_extra_height(), 0.0);

        // 交給管理器也一樣：active 但沒要求額外高度。
        let manager = ModuleManager::new();
        manager.register(Arc::new(PlainModule));
        assert_eq!(manager.active_module_id().as_deref(), Some("test.plain"));
        assert_eq!(manager.preview_extra_height(), 0.0);
    }
}
