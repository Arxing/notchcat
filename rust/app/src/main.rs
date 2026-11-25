//! MusiCat 可執行檔：把設定、現正播放來源、歌詞、音樂模組、瀏海面板與選單列圖示組起來（Task 25b）。
//!
//! 只支援 macOS；其他平台印一行訊息並以非零結束（函式庫 `musicat_app` 本身仍可在任何平台編譯與測試）。
//! 能寫成純函式的部分都在函式庫：設定（[`musicat_app::config`]）、選單（[`musicat_app::menu`]）、
//! 面板位置與螢幕變化（[`musicat_app::composition`]）、perl 宿主開關（[`musicat_app::gated_source`]）。
//! 這個檔案只是 GPUI／AppKit 的膠水。
//!
//! # 啟動順序
//!
//! 1. 讀設定（`MUSICAT_CONFIG` 或平台預設路徑），把警告印到 stderr（只有鍵名，不印內容）。
//! 2. `Application::new()`、`panel::patch_gpui_panel_class()`；在 `run` 的回呼一開始
//!    `panel::set_accessory_activation_policy()`（必須在 GPUI 啟動之後，否則會被蓋回 Regular；見 `panel.rs`）。
//! 3. `GpuiClock`／`GpuiSpawner`（GPUI 背景執行器）、`UreqHttp`、`SwitchableResolver`（依 `[sources]` 建的 repository）。
//! 4. 現正播放來源，依優先序：`[perl 宿主（HelperSupervisor，包在 GatedSource 裡）, WindowTitleSource]`。
//!    - helper 動態庫不在可執行檔旁 → 整個 perl 宿主不存在，印一次「system now-playing unavailable」。
//!    - `perl_host = false` → GatedSource 關著，**不建立**監管者；之後從選單打開時才建立並啟動。
//! 5. `NowPlayingMonitor`、`MediaRemoteControl::load()`、`MusicController`、`MusicModuleState`、
//!    `ModuleManager::register`（主執行緒；這一步啟動來源）。
//! 6. 選單列圖示、喚醒通知、螢幕：有螢幕就開面板；一個螢幕都沒有就等輪詢。
//!
//! # 擁有權與生命週期
//!
//! 所有東西放在 [`macos::AppState`]，以 `Rc<RefCell<…>>` 由主執行緒上的前景工作（選單事件、螢幕輪詢）共用；
//! `run` 的回呼返回後只剩這些工作持有它，它們與行程同壽。特別是 `Arc<NowPlayingMonitor>`：
//! 控制器只對它持弱參考，`AppState` 不持有它的話快照更新會悄悄停止。`ShellWindow`、`MenuBar`、`WakeObserver`
//! 也都在 `AppState` 裡（drop 即關閉面板／移除圖示／取消登記）。`RefCell` 的借用一律不跨過 `await`，
//! 也不會在 AppKit 回呼裡發生（選單與喚醒的回呼只送 channel 或呼叫 `Send + Sync` 的來源）。
//!
//! # 執行緒
//!
//! - GPUI 的前景工作、AppKit 的選單回呼與喚醒回呼都在主執行緒。`ShellWindow`、`MenuBar`、`list_screens`、
//!   來源的啟停只在主執行緒呼叫。
//! - 選單事件**不在** `MenuBar` 的回呼裡處理：回呼只把事件送進無界 channel，由前景工作取出處理
//!   （回呼裡不得借用 GPUI 的 `App`，也不呼叫 `MenuBar` 自己的方法）。
//! - `GpuiClock`／`GpuiSpawner` 在 GPUI 的背景執行緒跑 future（歌詞查詢、視窗標題輪詢、helper 重試計時）。
//!   其他執行緒上的狀態變化（來源、控制器）由 `MusicShellContent` 經 channel 帶回主執行緒；
//!   這裡沒有任何從外部執行緒呼叫 GPUI 的路徑。
//!
//! # 結束
//!
//! 選單「結束」（或 `MUSICAT_AUTO_QUIT_SECS`）→ 停止所有來源（監管者終止並回收 perl helper）→ `cx.quit()`。
//!
//! 其他結束路徑：
//!
//! - **經由 AppKit 的 terminate**（登出、關機、`NSRunningApplication.terminate()`；MusiCat 是沒有 Dock 圖示與主選單的
//!   accessory app、不會成為前景，所以沒有 Dock 結束與全域 ⌘Q，選單項目上的「⌘Q」就是選單路徑本身）：GPUI 在 `applicationWillTerminate`
//!   呼叫 `cx.on_app_quit` 的觀察者，這裡在觀察者裡也停止所有來源（`stop_sources` 只做一次，與選單路徑不會重複）。
//!   已驗證：對執行中的 App 呼叫 `NSRunningApplication.terminate()`（送出與登出相同的結束 Apple Event）→
//!   日誌出現 `app quit notification`、行程與 helper 都消失。
//! - **不經過 terminate**：`kill -9`（Activity Monitor 的強制結束）、`SIGINT`／`SIGTERM`（Ctrl-C、`kill`）
//!   直接殺行程，GPUI 與這裡都沒有機會清理，**`stop_sources` 不會被呼叫**。helper 在 stdin 的 pipe 關閉
//!   （父行程消失）時自行結束，所以不會留下孤兒（`kill -9` 與 `SIGTERM` 都已驗證 0.5 秒內 `pgrep -f musicat_helper_main` 為空）。
//!
//! # 環境變數（開發與測試用）
//!
//! - `MUSICAT_LOG=1`：在 stderr 印曲目（**含曲名與演出者**）、歌詞狀態（行數，不含內容）與外殼狀態轉移。
//!   沒設時不印任何與曲目有關的內容。
//! - `MUSICAT_AUTO_QUIT_SECS=<n>`：n 秒後走與選單「結束」完全相同的路徑結束（自動化驗證用）。
//! - `MUSICAT_CONFIG=<path>`：改用這個設定檔（測試損毀設定時用，避免碰到真正的設定檔）。
//!
//! 日誌一律是 stderr 的 `eprintln!`，不印設定檔內容。

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("MusiCat currently supports macOS only.");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicU8};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};
    use std::time::Instant;

    use futures_channel::mpsc;
    use futures_util::StreamExt;
    use gpui::{App, Application, AsyncApp};
    use musicat_app::composition::{
        auto_quit_after, config_path_with_override, describe_lyrics, describe_snapshot,
        log_enabled, placement, should_log_no_screens, target_geometry, Placement, ScreenTracker,
        SCREEN_POLL_INTERVAL,
    };
    use musicat_app::config::{config_path, ConfigStore, LoadSource};
    use musicat_app::gated_source::GatedSource;
    use musicat_app::http_ureq::UreqHttp;
    use musicat_app::menu::{build_menu, Effect, MenuPlatform};
    use musicat_app::single_instance::{self, Acquire};
    use musicat_app::switchable::{build_repository, SwitchableResolver};
    use musicat_core::artwork_cache::ArtworkCache;
    use musicat_core::http::HttpClient;
    use musicat_core::menu::MenuEvent;
    use musicat_core::modules::{ModuleManager, ModuleState};
    use musicat_core::music::controller::MusicController;
    use musicat_core::music::module::MusicModuleState;
    use musicat_core::nowplaying::{NowPlayingMonitor, NowPlayingSource, PlaybackControl};
    use musicat_core::runtime::{Clock, Spawner, Subscription};
    use musicat_core::shell_logic::{container_frame, NotchGeometry};
    use musicat_platform_macos::mediaremote_control::MediaRemoteControl;
    use musicat_platform_macos::perl_launcher::PerlLauncher;
    use musicat_platform_macos::power::{observe_wake, WakeObserver};
    use musicat_platform_macos::screens::list_screens;
    use musicat_platform_macos::supervisor::{HelperSupervisor, SupervisorConfig};
    use musicat_platform_macos::tray::MenuBar;
    use musicat_platform_macos::window_title_source::{CgWindowLister, WindowTitleSource};
    use musicat_platform_macos::{panel, DYLIB_NAME};
    use musicat_shell::{GpuiClock, GpuiSpawner, MusicShellContent, ShellContent, ShellWindow};

    // ------------------------------------------------------------------ 日誌

    fn t0() -> Instant {
        static T0: OnceLock<Instant> = OnceLock::new();
        *T0.get_or_init(Instant::now)
    }

    /// stderr 的一行日誌，前綴為啟動後經過的秒數。
    macro_rules! log {
        ($($arg:tt)*) => {
            eprintln!("musicat [+{:>8.3}s] {}", t0().elapsed().as_secs_f64(), format_args!($($arg)*))
        };
    }

    // ------------------------------------------------------------------ 狀態

    type Helper = GatedSource<HelperSupervisor>;

    /// 整個 App 的狀態（主執行緒）。見 crate 文件的「擁有權與生命週期」。
    pub(super) struct AppState {
        store: ConfigStore,
        detailed_log: bool,
        http: Arc<dyn HttpClient>,
        clock: Arc<dyn Clock>,
        resolver: Arc<SwitchableResolver>,
        /// perl 宿主；helper 動態庫不在時為 `None`。
        helper: Option<Arc<Helper>>,
        /// 依優先序；結束時逐一停止。
        sources: Vec<Arc<dyn NowPlayingSource>>,
        /// **必須持有**：控制器只對它持弱參考。
        _monitor: Arc<NowPlayingMonitor>,
        controller: Arc<MusicController>,
        module_state: Arc<MusicModuleState>,
        modules: Arc<ModuleManager>,
        /// 第一次開面板時建立；之後換螢幕時 `set_geometry`。
        content: Option<Arc<MusicShellContent>>,
        shell: Option<ShellWindow>,
        /// 面板目前的幾何（`shell` 開著時才有）。
        geometry: Option<NotchGeometry>,
        screens: ScreenTracker,
        open_failed_logged: bool,
        /// 「沒有螢幕」的日誌只在狀況開始時寫一次（見 `should_log_no_screens`）。
        no_screens_logged: bool,
        /// `stop_sources` 已跑過（選單「結束」與 AppKit 的結束通知兩條路徑都會呼叫，只做一次）。
        sources_stopped: Cell<bool>,
        menu: Option<MenuBar>,
        _wake: Option<WakeObserver>,
        _log_subscriptions: Vec<Subscription>,
    }

    pub fn run() {
        let _ = t0();
        let detailed_log = log_enabled(std::env::var("MUSICAT_LOG").ok().as_deref());
        let auto_quit = auto_quit_after(std::env::var("MUSICAT_AUTO_QUIT_SECS").ok().as_deref());

        let Some(path) =
            config_path_with_override(std::env::var("MUSICAT_CONFIG").ok(), config_path())
        else {
            eprintln!(
                "MusiCat: cannot find the home directory ($HOME is unset or relative); exiting."
            );
            std::process::exit(1);
        };

        // 單一執行個體：在建立任何東西（設定檔、視窗、選單列圖示、helper、GPUI）之前先取鎖。
        // 鎖檔在設定檔旁，所以用另一個 `MUSICAT_CONFIG` 的測試執行個體不會與真正的那個互相衝突。
        // 這個守衛必須活到行程結束：放在 `run` 的區域變數（比 `Application::run` 長命）；
        // GPUI 的結束路徑不跑解構函式也沒關係，行程結束時作業系統會釋放鎖。
        let _instance_lock = match single_instance::acquire(&single_instance::lock_path_for(&path))
        {
            Acquire::Acquired(lock) => Some(lock),
            Acquire::AlreadyRunning => {
                eprintln!("MusiCat 已在執行中，結束。／MusiCat is already running; exiting.");
                std::process::exit(1);
            }
            Acquire::Skipped(reason) => {
                eprintln!(
                    "MusiCat: single-instance guard unavailable ({reason}); starting anyway."
                );
                None
            }
        };

        let (store, warnings) = ConfigStore::open(&path);
        log!(
            "config {} ({})",
            path.display(),
            match store.load_source() {
                LoadSource::Missing => "missing: using defaults",
                LoadSource::Loaded => "loaded",
                LoadSource::Broken => "unreadable: using defaults",
            }
        );
        for w in &warnings {
            log!("config warning: {w}");
        }

        let app = Application::new();
        panel::patch_gpui_panel_class();
        app.run(move |cx: &mut App| {
            panel::set_accessory_activation_policy();
            let state = Rc::new(RefCell::new(build(store, detailed_log, cx)));

            // 選單：回呼只送 channel；前景工作處理。
            let (tx, rx) = mpsc::unbounded::<MenuEvent>();
            {
                let st = state.borrow();
                if let Some(menu) = &st.menu {
                    let tx = tx.clone();
                    menu.on_event(Box::new(move |event| {
                        // 接收端只在行程結束時消失。
                        let _ = tx.unbounded_send(event);
                    }));
                }
            }
            place_panel(&mut state.borrow_mut(), cx, true);
            state.borrow().refresh_menu();
            if detailed_log {
                if let Some(menu) = &state.borrow().menu {
                    log!("menu bar: {}", menu.describe());
                }
            }

            // 選單「結束」以外、經由 AppKit 的 terminate 結束的路徑（登出、關機、`NSRunningApplication.terminate()`；沒有 Dock 結束與全域 ⌘Q）：
            // GPUI 在 applicationWillTerminate 呼叫這個觀察者，在這裡也停掉來源，不必靠 helper 的 stdin 關閉。
            // 借用失敗（不該發生：結束通知在主執行緒的事件迴圈裡，不在任何借用之內）時略過，helper 仍會自行結束。
            cx.on_app_quit({
                let state = state.clone();
                move |_cx| {
                    match state.try_borrow() {
                        // 選單「結束」已經停過了就不再印。
                        Ok(st) if st.sources_stopped.get() => {}
                        Ok(st) => {
                            log!("app quit notification: stopping now-playing sources");
                            st.stop_sources();
                        }
                        Err(_) => {
                            log!("app quit notification while the state is borrowed; skipping")
                        }
                    }
                    std::future::ready(())
                }
            })
            .detach();
            cx.spawn({
                let state = state.clone();
                async move |cx: &mut AsyncApp| menu_loop(state, rx, cx).await
            })
            .detach();
            cx.spawn({
                let state = state.clone();
                async move |cx: &mut AsyncApp| screen_loop(state, cx).await
            })
            .detach();
            if let Some(after) = auto_quit {
                log!(
                    "MUSICAT_AUTO_QUIT_SECS: quitting after {:.1}s",
                    after.as_secs_f64()
                );
                cx.spawn(async move |cx: &mut AsyncApp| {
                    cx.background_executor().timer(after).await;
                    log!("auto-quit timer fired");
                    let _ = tx.unbounded_send(MenuEvent::Quit);
                })
                .detach();
            }
        });
    }

    /// 建出所有長壽的物件（主執行緒，`run` 的回呼裡）。
    fn build(store: ConfigStore, detailed_log: bool, cx: &mut App) -> AppState {
        let executor = cx.background_executor().clone();
        let clock: Arc<dyn Clock> = Arc::new(GpuiClock::new(executor.clone()));
        let spawner: Arc<dyn Spawner> = Arc::new(GpuiSpawner::new(executor));
        let http: Arc<dyn HttpClient> = Arc::new(UreqHttp::new());
        let cfg = store.config().clone();

        let resolver = Arc::new(SwitchableResolver::new(build_repository(
            &cfg.sources,
            http.clone(),
            clock.clone(),
        )));

        // 現正播放來源（優先序：perl 宿主、視窗標題備援）。
        let helper = if helper_dylib_present() {
            let (clock, spawner) = (clock.clone(), spawner.clone());
            let launcher = Arc::new(PerlLauncher::default());
            if !cfg.macos.perl_host {
                log!("perl host disabled in the config; using the window-title fallback (can be enabled from the menu)");
            }
            Some(GatedSource::new(
                musicat_platform_macos::helper_wire::SOURCE_ID,
                cfg.macos.perl_host,
                move || {
                    log!("starting the system now-playing helper (perl host)");
                    HelperSupervisor::new(
                        launcher.clone(),
                        clock.clone(),
                        spawner.clone(),
                        SupervisorConfig::default(),
                    )
                },
            ))
        } else {
            log!("system now-playing unavailable ({DYLIB_NAME} is not next to the executable); using window-title fallback");
            None
        };
        let window_title =
            WindowTitleSource::new(Arc::new(CgWindowLister), clock.clone(), spawner.clone());
        let mut sources: Vec<Arc<dyn NowPlayingSource>> = Vec::new();
        if let Some(helper) = &helper {
            sources.push(helper.clone());
        }
        sources.push(window_title);

        let monitor = Arc::new(NowPlayingMonitor::new(
            sources.clone(),
            ArtworkCache::default(),
        ));
        let control: Option<Arc<dyn PlaybackControl>> =
            MediaRemoteControl::load().map(|c| Arc::new(c) as Arc<dyn PlaybackControl>);
        if control.is_none() {
            log!("playback control unavailable (MediaRemote could not be loaded); buttons are disabled");
        }
        let controller =
            MusicController::new(monitor.clone(), resolver.clone(), control, spawner.clone());
        let module_state = Arc::new(MusicModuleState::with_strip_lines(
            controller.clone(),
            Arc::new(AtomicBool::new(cfg.lyric_strip_visible)),
            Arc::new(AtomicU8::new(cfg.lyric_strip_lines)),
            sources.clone(),
        ));
        let modules = Arc::new(ModuleManager::new());

        let mut log_subscriptions = Vec::new();
        if detailed_log {
            log_subscriptions.push(monitor.subscribe(Box::new(|snapshot| {
                log!(
                    "now playing: {}",
                    describe_snapshot(snapshot.as_ref(), true)
                );
            })));
            let last = Mutex::new(String::new());
            let weak = Arc::downgrade(&controller);
            log_subscriptions.push(controller.subscribe(Box::new(move |_| {
                let Some(controller) = weak.upgrade() else {
                    return;
                };
                // 通知只是「有變化」的訊號：重讀狀態（見 MusicController 的文件）。
                let line = describe_lyrics(&controller.state());
                let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
                if *last != line {
                    log!("lyrics: {line}");
                    *last = line;
                }
            })));
        }

        // 註冊即啟動來源（主執行緒）。
        modules.register(module_state.clone() as Arc<dyn ModuleState>);

        let menu = MenuBar::new(None);
        if menu.is_none() {
            log!("could not create the menu bar item");
        }
        let wake = helper.as_ref().and_then(|helper| {
            let helper = helper.clone();
            observe_wake(Box::new(move || {
                log!("system woke from sleep");
                if let Some(supervisor) = helper.inner() {
                    supervisor.wake_from_sleep();
                }
            }))
        });

        AppState {
            store,
            detailed_log,
            http,
            clock,
            resolver,
            helper,
            sources,
            _monitor: monitor,
            controller,
            module_state,
            modules,
            content: None,
            shell: None,
            geometry: None,
            screens: ScreenTracker::default(),
            open_failed_logged: false,
            no_screens_logged: false,
            sources_stopped: Cell::new(false),
            menu,
            _wake: wake,
            _log_subscriptions: log_subscriptions,
        }
    }

    fn helper_dylib_present() -> bool {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(DYLIB_NAME)))
            .is_some_and(|p| p.is_file())
    }

    impl AppState {
        fn refresh_menu(&self) {
            if let Some(menu) = &self.menu {
                menu.set_model(build_menu(
                    self.store.config(),
                    self.screens.screens(),
                    MenuPlatform::MacOs,
                ));
            }
        }

        /// 歌詞條的折疊也能在 Expanded 的箭頭上切換（直接改共用旗標）：把它同步回設定檔。
        fn sync_strip_preference(&mut self) {
            let visible = self.module_state.strip_visible();
            if self.store.config().lyric_strip_visible == visible {
                return;
            }
            if let Err(e) = self.store.update(|c| c.lyric_strip_visible = visible) {
                log!("could not save the config ({:?})", e.kind());
            }
            self.refresh_menu();
        }

        /// 停止所有來源（perl helper 被終止並回收）。結束前呼叫。
        fn stop_sources(&self) {
            if self.sources_stopped.replace(true) {
                return;
            }
            for source in &self.sources {
                source.stop();
            }
        }
    }

    /// 依目前的螢幕與設定開啟或移動面板。`relist` 為真時重新列舉螢幕。
    /// 一個螢幕都沒有時什麼都不做（面板保留在原處、程式不結束），等下一次輪詢。
    fn place_panel(st: &mut AppState, cx: &mut App, relist: bool) {
        if relist {
            let screens = list_screens();
            st.screens.observe(&screens);
        }
        let target = target_geometry(st.screens.screens(), st.store.config());
        let current = if st.shell.is_some() {
            st.geometry
        } else {
            None
        };
        if should_log_no_screens(
            &mut st.no_screens_logged,
            target.is_some(),
            st.shell.is_some(),
        ) {
            log!("no screens yet; waiting");
        }
        match placement(current, target.as_ref().map(|(_, g)| *g)) {
            Placement::Stay => {}
            Placement::Move(geometry) => {
                if let Some(shell) = &st.shell {
                    shell.reposition(geometry);
                }
                if let Some(content) = &st.content {
                    content.set_geometry(geometry);
                }
                st.geometry = Some(geometry);
                log!(
                    "panel moved to screen {} ({:?})",
                    target.as_ref().map_or("?", |(id, _)| id.as_str()),
                    container_frame(&geometry)
                );
            }
            Placement::Open(geometry) => open_panel(st, cx, geometry, target.map(|(id, _)| id)),
        }
    }

    fn open_panel(
        st: &mut AppState,
        cx: &mut App,
        geometry: NotchGeometry,
        screen_id: Option<String>,
    ) {
        let content = match &st.content {
            Some(content) => {
                content.set_geometry(geometry);
                content.clone()
            }
            None => {
                let mut content = MusicShellContent::new(
                    cx,
                    geometry,
                    st.controller.clone(),
                    st.module_state.clone(),
                    st.modules.clone(),
                    st.clock.clone(),
                    // 歌詞來源標籤：只開 https 網址（`open_url` 自己再驗證一次），在主執行緒呼叫。
                    Arc::new(|url: &str| {
                        musicat_platform_macos::links::open_url(url);
                    }),
                );
                if st.detailed_log {
                    content = content.with_event_hook(|event| log!("shell: {event:?}"));
                }
                // 內容含 GPUI entity，不是 Send/Sync；只在主執行緒使用（ShellContent 的約定）。
                // 這裡要同時保有具體型別（換螢幕時 set_geometry）與 `Arc<dyn ShellContent>`，所以自己包 Arc。
                #[allow(clippy::arc_with_non_send_sync)]
                let content = Arc::new(content);
                st.content = Some(content.clone());
                content
            }
        };
        let dyn_content: Arc<dyn ShellContent> = content;
        match ShellWindow::open(cx, geometry, dyn_content) {
            Ok(shell) => {
                st.shell = Some(shell);
                st.geometry = Some(geometry);
                st.open_failed_logged = false;
                log!(
                    "panel open on screen {} at {:?}",
                    screen_id.as_deref().unwrap_or("?"),
                    container_frame(&geometry)
                );
            }
            Err(e) => {
                if !st.open_failed_logged {
                    log!("could not open the panel ({e}); retrying on the next screen poll");
                    st.open_failed_logged = true;
                }
            }
        }
    }

    /// 每 2 秒列舉螢幕；有變化（或面板還沒開）時重新決定面板位置並更新選單。也順便同步歌詞條偏好。
    async fn screen_loop(state: Rc<RefCell<AppState>>, cx: &mut AsyncApp) {
        loop {
            cx.background_executor().timer(SCREEN_POLL_INTERVAL).await;
            let alive = cx.update(|cx| {
                let mut st = state.borrow_mut();
                let screens = list_screens();
                let changed = st.screens.observe(&screens);
                if changed {
                    log!("screens changed: {} connected", screens.len());
                }
                if changed || st.shell.is_none() {
                    place_panel(&mut st, cx, false);
                }
                if changed {
                    st.refresh_menu();
                }
                st.sync_strip_preference();
            });
            if alive.is_err() {
                break;
            }
        }
    }

    /// 選單事件的前景工作：套用到設定（只在有變化時存檔）、執行副作用、重建選單。
    async fn menu_loop(
        state: Rc<RefCell<AppState>>,
        mut rx: mpsc::UnboundedReceiver<MenuEvent>,
        cx: &mut AsyncApp,
    ) {
        while let Some(event) = rx.next().await {
            let quit = cx
                .update(|cx| handle_menu_event(&mut state.borrow_mut(), cx, event))
                .unwrap_or(true);
            if quit {
                let _ = cx.update(|cx| cx.quit());
                break;
            }
        }
    }

    /// 回傳是否要結束。
    fn handle_menu_event(st: &mut AppState, cx: &mut App, event: MenuEvent) -> bool {
        // 先把 Expanded 箭頭造成的歌詞條變更同步進設定，事件才以正確的現值套用。
        st.sync_strip_preference();
        let (effects, saved) = st.store.apply_menu_event(event);
        if let Err(e) = saved {
            log!(
                "could not save the config ({:?}); the change applies to this run only",
                e.kind()
            );
        }
        let mut quit = false;
        for effect in effects {
            match effect {
                Effect::RebuildLyricsProviders => {
                    let sources = st.store.config().sources;
                    log!("lyrics providers: lrclib={} netease={}", sources.lrclib, sources.netease);
                    st.resolver
                        .swap(build_repository(&sources, st.http.clone(), st.clock.clone()));
                }
                Effect::SetPerlHost(on) => match &st.helper {
                    Some(helper) => {
                        log!("perl host {}", if on { "enabled" } else { "disabled" });
                        helper.set_enabled(on);
                    }
                    None => log!(
                        "perl host toggled but {DYLIB_NAME} is not available; saved for the next launch"
                    ),
                },
                Effect::SetLyricStripVisible(visible) => {
                    st.module_state.set_strip_visible(visible);
                    if let Some(shell) = &st.shell {
                        shell.refresh();
                    }
                }
                Effect::SetLyricStripLines(lines) => {
                    log!("lyric strip lines: {lines}");
                    st.module_state.set_strip_lines(lines);
                    if let Some(shell) = &st.shell {
                        shell.refresh();
                    }
                }
                Effect::MoveToScreen(id) => {
                    log!("screen choice: {}", id.as_deref().unwrap_or("automatic"));
                    place_panel(st, cx, true);
                }
                Effect::Quit => quit = true,
            }
        }
        if quit {
            log!("quitting: stopping now-playing sources");
            st.stop_sources();
            // 立刻隱藏面板與圖示（cx.quit 之後行程直接結束，不會跑 drop）。
            st.shell = None;
            st.menu = None;
            return true;
        }
        st.refresh_menu();
        false
    }
}
