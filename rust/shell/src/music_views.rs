//! 音樂模組在外殼裡的畫面（移植自 Swift `MusicPreviewView`、`MusicExpandedView`、`CoverView`、
//! `EqualizerBars`、`LyricStripView`，以及外殼的 `NotchContentView`／`ModuleAppBar`／`DefaultShellContent`）。
//!
//! # 結構
//!
//! [`MusicShellContent`] 實作 [`ShellContent`]。它擁有兩個 GPUI entity（`MusicView`，一個畫 Preview、
//! 一個畫 Expanded），外殼的 `preview()`／`expanded()` 只回傳對應 entity 的元素；兩者共用一份 `Shared`
//! （控制器、模組狀態、模組管理器、時鐘、封面快取、跑馬燈與歌詞捲動狀態）。
//!
//! 與 Swift `NotchContentView` 相同：
//! - Preview 只在 active 模組是音樂（= 正在播放）時畫音樂內容，否則什麼都不畫（外殼只剩黑色長條；
//!   暫停時回到空黑條，HANDOFF 第 2 項）。
//! - Expanded 上方是模組切換列；下方依 `ModuleManager::selected_module_id()` 畫音樂頁，沒有選取時畫
//!   `DefaultShellContent` 的說明文字。每次新展開由 [`ShellContent::on_expand`] 把選取重設為 active 模組。
//!
//! # 重繪如何回到主執行緒
//!
//! 控制器、模組狀態與模組管理器的通知可能在**任何**執行緒上抵達。訂閱的回呼只做一件事：
//! 若「待處理」旗標原本是 false 就設為 true，並往一個無界 channel 送一個 `()`（合併連續的通知）。
//! 主執行緒上的 GPUI 工作（`App::spawn`）等這個 channel，收到時清旗標、把快取的狀態快照標為過期、
//! 對兩個 entity `cx.notify()`，並喚醒定時器工作。render 讀的是 `Shared::state()`：過期時才重新
//! `MusicController::state()`（含歌詞的整份複製），所以每次狀態改變只複製一次，每一格不複製；
//! 現值一律重讀，不信任通知的順序。回呼只持有 channel 的送出端，不持有任何 GPUI 物件，所以從不在外部執行緒碰 GPUI。
//!
//! 定時重繪（音柱 12 fps、跑馬燈捲動時 30 fps、歌詞條／進度／歌詞跟隨 4 Hz）由另一個主執行緒工作依
//! [`tick_interval`](crate::music_layout::tick_interval) 決定：需要時以該週期計時，同時也等「喚醒」；
//! 不需要時（面板隱藏、Preview 沒在播、Expanded 沒有隨時間變化的東西）**只等喚醒**，不定時醒來。
//! 喚醒來源：狀態改變（事件工作）、外殼的模式／可見性改變（`on_shell_event`）、跑馬燈開始捲動（paint）。
//! 所以按下播放後音柱立刻開始動。只有歌詞換行的 0.3 秒捲動動畫由 render `request_animation_frame()`，
//! 只在進行中才要求。
//!
//! # 生命週期
//!
//! 訂閱把手與兩個工作都存在 `MusicShellContent` 裡；drop 它就取消訂閱（送出端隨之消失、channel 結束）並取消工作。
//! 工作只持有 entity 的 `WeakEntity` 與 `Shared` 的 `Weak`，沒有 `Arc`／`Rc` 循環。

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_channel::mpsc;
use futures_util::future::{select, Either};
use futures_util::StreamExt;
use gpui::{
    black, canvas, div, img, point, prelude::*, px, white, AnyElement, App, AsyncApp, Bounds,
    Context, Entity, FontWeight, Hsla, ObjectFit, PathBuilder, Pixels, RenderImage, SharedString,
    StyledImage, Task, WeakEntity, Window,
};
use musicat_core::lyrics::{LyricLine, Lyrics};
use musicat_core::modules::{ModuleManager, ModuleState};
use musicat_core::music::controller::{MusicController, MusicState, VersionInfo};
use musicat_core::music::module::{MusicModuleState, LYRIC_STRIP_OPACITY, LYRIC_STRIP_ROW_HEIGHT};
use musicat_core::music::presentation::{
    has_progress, lyrics_status, progress_fraction, strip_line_index, strip_rows, StripRow,
};
use musicat_core::nowplaying::PlaybackCommand;
use musicat_core::runtime::{Clock, Subscription};
use musicat_core::shell_logic::{mode_size, NotchGeometry, ShellMode};
use musicat_core::types::{ArtworkRef, TrackSnapshot};

use crate::artwork::{
    decode_cover, CoverCache, CoverEntry, CoverKey, DecodeQueue, COVER_CACHE_CAPACITY,
    COVER_TEXTURE_PIXELS,
};
use crate::marquee::{marquee, MarqueeState, MarqueeStyle, SYSTEM_FONT};
use crate::music_layout::*;
use crate::{ShellContent, ShellEvent};

/// 喚醒定時器工作（主執行緒內使用；合併連續的喚醒）。
struct Kick {
    tx: mpsc::UnboundedSender<()>,
    pending: Cell<bool>,
}

impl Kick {
    fn kick(&self) {
        if !self.pending.replace(true) {
            // 定時器工作已結束（內容正在 drop）時送出失敗，忽略。
            let _ = self.tx.unbounded_send(());
        }
    }
}

/// 兩個 view 共用的狀態。只在 GPUI 主執行緒使用。
struct Shared {
    controller: Arc<MusicController>,
    module_state: Arc<MusicModuleState>,
    modules: Arc<ModuleManager>,
    clock: Arc<dyn Clock>,
    /// 開啟歌詞來源網址（app 注入平台的 `open_url`；示範只印出來）。
    open_url: Arc<dyn Fn(&str) + Send + Sync>,
    geometry: Cell<NotchGeometry>,
    /// 外殼目前的模式（`on_shell_event` 與 `preview()`／`expanded()` 更新）。
    mode: Cell<ShellMode>,
    /// 面板實際可見（`ShellEvent::Visible`）。
    visible: Cell<bool>,
    covers: RefCell<CoverCache<Arc<RenderImage>>>,
    /// 封面解碼排程：同時最多一個，只解最新要求的（見 `artwork::DecodeQueue`）。
    decodes: RefCell<DecodeQueue<CoverKey, Arc<[u8]>>>,
    /// 控制器狀態的快照；`state_dirty` 時才重新複製（事件工作在每次狀態通知時標為過期）。
    state_cache: RefCell<Option<Rc<MusicState>>>,
    state_dirty: Cell<bool>,
    kick: Kick,
    /// 被快取擠出、等下一次 render（有 `Window`）時從 GPU 圖集釋放的封面。
    released: RefCell<Vec<Arc<RenderImage>>>,
    strip_marquee: MarqueeState,
    /// Preview 左翼的曲名與右翼的演出者。
    wing_title_marquee: MarqueeState,
    wing_artist_marquee: MarqueeState,
    /// 歌詞條的換行捲動。
    strip_scroll: RefCell<StripScroll>,
    title_marquee: MarqueeState,
    artist_marquee: MarqueeState,
    lyric_scroll: RefCell<LyricScroll>,
    /// 音柱動畫的時間原點。
    t0: Instant,
}

impl Shared {
    /// 目前的音樂狀態。只在收到狀態通知後才重新從控制器複製（歌詞可能有上百行，不要每格複製）。
    fn state(&self) -> Rc<MusicState> {
        let mut cache = self.state_cache.borrow_mut();
        if self.state_dirty.replace(false) || cache.is_none() {
            *cache = Some(Rc::new(self.controller.state()));
        }
        cache.clone().unwrap_or_default()
    }

    fn music_is_active(&self) -> bool {
        self.modules.active_module_id().as_deref() == Some(MusicModuleState::ID)
    }

    fn music_is_selected(&self) -> bool {
        self.modules.selected_module_id().as_deref() == Some(MusicModuleState::ID)
    }

    fn tick_inputs(&self) -> TickInputs {
        let state = self.state();
        let playing = state.snapshot.as_ref().is_some_and(|s| s.is_playing);
        let expanded_live = playing
            && self.music_is_selected()
            && state.snapshot.as_ref().is_some_and(|s| {
                has_progress(s)
                    || (s.position.is_some()
                        && state.lyrics.as_ref().is_some_and(|l| l.is_synced()))
            });
        TickInputs {
            visible: self.visible.get(),
            expanded: self.mode.get() == ShellMode::Expanded,
            preview_playing: playing && self.music_is_active(),
            expanded_live,
            marquee_scrolling: if self.mode.get() == ShellMode::Expanded {
                self.title_marquee.is_scrolling() || self.artist_marquee.is_scrolling()
            } else {
                self.strip_marquee.is_scrolling()
                    || self.wing_title_marquee.is_scrolling()
                    || self.wing_artist_marquee.is_scrolling()
            },
        }
    }
}

/// 歌詞跟隨捲動的動畫狀態。
#[derive(Default)]
struct LyricScroll {
    /// 哪一份歌詞（曲名 + 行數）；換歌詞時直接跳到位置、不做動畫（同 Swift 的 `onAppear`）。
    key: Option<(String, usize)>,
    from: f64,
    to: f64,
    started: Option<Instant>,
    /// 下一次直接跳到位置（剛展開，或剛切換歌詞版本）。
    jump: bool,
}

impl LyricScroll {
    fn value(&self, now: Instant) -> f64 {
        match self.started {
            Some(t) => animated_offset(
                self.from,
                self.to,
                now.duration_since(t).as_secs_f64(),
                LYRIC_SCROLL_SECONDS,
            ),
            None => self.to,
        }
    }

    fn animating(&self, now: Instant) -> bool {
        self.started
            .is_some_and(|t| now.duration_since(t).as_secs_f64() < LYRIC_SCROLL_SECONDS)
    }
}

/// 音樂內容。見模組文件。
///
/// `geometry` 只用來排 Preview 的兩翼（中央 = 瀏海寬，不放重要內容）；換螢幕時與
/// `ShellWindow::reposition` 一起呼叫 [`MusicShellContent::set_geometry`]。
pub struct MusicShellContent {
    shared: Rc<Shared>,
    preview: Entity<MusicView>,
    expanded: Entity<MusicView>,
    event_hook: Option<Box<dyn Fn(ShellEvent)>>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl MusicShellContent {
    /// 建立內容並訂閱 `controller`（曲目／歌詞）、`module_state`（歌詞條額外高度）與 `modules`（active／選取）。
    /// 必須在 GPUI 主執行緒（`App` 回呼）呼叫。
    ///
    /// - `module_state` 應已註冊進 `modules`（外殼以 `modules.active_module_id()` 判斷音樂是否 active）。
    /// - `clock` 給播放位置推算用（Unix 秒），應與控制器的快照同一時間基準。
    /// - `open_url` 在使用者點 Expanded 控制列左側的歌詞來源標籤時呼叫，參數是點擊當下狀態裡的
    ///   `lyrics_url`（一律 `https://`）。在主執行緒呼叫；外殼不另做驗證，由注入的實作負責。
    pub fn new(
        cx: &mut App,
        geometry: NotchGeometry,
        controller: Arc<MusicController>,
        module_state: Arc<MusicModuleState>,
        modules: Arc<ModuleManager>,
        clock: Arc<dyn Clock>,
        open_url: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        // 定時器的喚醒 channel：送出端在 Shared 裡（主執行緒），Shared drop 時定時器工作隨之結束。
        let (kick_tx, mut kick_rx) = mpsc::unbounded::<()>();
        let shared = Rc::new(Shared {
            controller,
            module_state,
            modules,
            clock,
            open_url,
            geometry: Cell::new(geometry),
            mode: Cell::new(ShellMode::Preview),
            visible: Cell::new(true),
            covers: RefCell::new(CoverCache::new(COVER_CACHE_CAPACITY)),
            decodes: RefCell::new(DecodeQueue::new()),
            state_cache: RefCell::new(None),
            state_dirty: Cell::new(true),
            kick: Kick {
                tx: kick_tx,
                pending: Cell::new(false),
            },
            released: RefCell::new(Vec::new()),
            strip_marquee: MarqueeState::new(),
            wing_title_marquee: MarqueeState::new(),
            wing_artist_marquee: MarqueeState::new(),
            strip_scroll: RefCell::new(StripScroll::default()),
            title_marquee: MarqueeState::new(),
            artist_marquee: MarqueeState::new(),
            lyric_scroll: RefCell::new(LyricScroll::default()),
            t0: Instant::now(),
        });
        let preview = cx.new(|_| MusicView {
            shared: shared.clone(),
            kind: ShellMode::Preview,
        });
        let expanded = cx.new(|_| MusicView {
            shared: shared.clone(),
            kind: ShellMode::Expanded,
        });

        // 任意執行緒 → 主執行緒：合併通知的旗標 + 無界 channel。
        let (tx, mut rx) = mpsc::unbounded::<()>();
        let pending = Arc::new(AtomicBool::new(false));
        let wake = || {
            let tx = tx.clone();
            let pending = pending.clone();
            move || {
                if !pending.swap(true, Ordering::SeqCst) {
                    // 接收端已結束（內容正在 drop）時送出失敗，忽略。
                    let _ = tx.unbounded_send(());
                }
            }
        };
        let subscriptions = vec![
            shared.controller.subscribe(Box::new({
                let wake = wake();
                move |_state| wake()
            })),
            shared.module_state.subscribe(Box::new(wake())),
            shared.modules.subscribe(Box::new(wake())),
        ];
        // 只留回呼裡的送出端：訂閱取消時 channel 隨之結束，事件工作自然停止。
        drop(tx);

        // 跑馬燈開始捲動時喚醒定時器（它可能正閒置等待）。
        for marquee in [
            &shared.wing_title_marquee,
            &shared.wing_artist_marquee,
            &shared.strip_marquee,
            &shared.title_marquee,
            &shared.artist_marquee,
        ] {
            let weak = Rc::downgrade(&shared);
            marquee.set_waker(move || {
                if let Some(shared) = weak.upgrade() {
                    shared.kick.kick();
                }
            });
        }

        let (wp, we) = (preview.downgrade(), expanded.downgrade());
        let events_shared: Weak<Shared> = Rc::downgrade(&shared);
        let events = cx.spawn(async move |cx: &mut AsyncApp| {
            while rx.next().await.is_some() {
                pending.store(false, Ordering::SeqCst);
                if let Some(shared) = events_shared.upgrade() {
                    shared.state_dirty.set(true);
                    shared.kick.kick();
                }
                let a = wp.update(cx, |_, cx| cx.notify());
                let b = we.update(cx, |_, cx| cx.notify());
                if a.is_err() && b.is_err() {
                    break;
                }
            }
        });

        let weak_shared: Weak<Shared> = Rc::downgrade(&shared);
        let (wp, we) = (preview.downgrade(), expanded.downgrade());
        let ticker = cx.spawn(async move |cx: &mut AsyncApp| loop {
            let Some(shared) = weak_shared.upgrade() else {
                break;
            };
            // 這一輪會重新判斷，之前累積的喚醒都算處理了。
            shared.kick.pending.set(false);
            let interval = tick_interval(shared.tick_inputs());
            let target: &WeakEntity<MusicView> = if shared.mode.get() == ShellMode::Expanded {
                &we
            } else {
                &wp
            };
            drop(shared);
            if interval.is_some() && target.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
            let woken = match interval {
                Some(seconds) => {
                    let timer = cx
                        .background_executor()
                        .timer(Duration::from_secs_f64(seconds));
                    match select(Box::pin(timer), kick_rx.next()).await {
                        Either::Left(_) => true,
                        Either::Right((kick, _)) => kick.is_some(),
                    }
                }
                // 閒置：只等喚醒，不定時醒來。
                None => kick_rx.next().await.is_some(),
            };
            if !woken {
                // 送出端（Shared）已不在：內容已 drop。
                break;
            }
        });

        Self {
            shared,
            preview,
            expanded,
            event_hook: None,
            _subscriptions: subscriptions,
            _tasks: vec![events, ticker],
        }
    }

    /// 外殼狀態轉移時另外呼叫 `hook`（示範程式印日誌用）。
    pub fn with_event_hook(mut self, hook: impl Fn(ShellEvent) + 'static) -> Self {
        self.event_hook = Some(Box::new(hook));
        self
    }

    /// 包成外殼要的 `Arc<dyn ShellContent>`（`ShellWindow::open` 的參數）。
    ///
    /// 內容含 GPUI entity 與 `Rc`，不是 `Send`／`Sync`；這沒問題，因為外殼只在 GPUI 主執行緒使用它
    /// （[`ShellContent`] 的文件），`Arc` 只是外殼介面選的共享把手。clippy 的 `arc_with_non_send_sync`
    /// 因此在這裡被允許一次，呼叫端不必各自處理。
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn into_shell_content(self) -> Arc<dyn ShellContent> {
        Arc::new(self)
    }

    /// 換螢幕（瀏海大小改變）時更新 Preview 兩翼的排法；與 `ShellWindow::reposition` 一起呼叫。
    pub fn set_geometry(&self, geometry: NotchGeometry) {
        self.shared.geometry.set(geometry);
    }
}

impl ShellContent for MusicShellContent {
    fn preview(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
        self.shared.mode.set(ShellMode::Preview);
        self.preview.clone().into_any_element()
    }

    fn expanded(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
        self.shared.mode.set(ShellMode::Expanded);
        self.expanded.clone().into_any_element()
    }

    /// 外殼每拍（16–40 ms）讀一次：用模組管理器快取的值（已夾到 0…28），不每拍重算、不複製狀態。
    /// 本輪只有音樂一個模組，所以它就是 `MusicModuleState::preview_extra_height()`（播放中且歌詞條展開時 24）。
    fn preview_extra_height(&self) -> f64 {
        self.shared.modules.preview_extra_height()
    }

    /// spec 5.3：每次新展開把模組切換列的選取重設為 active 模組。只碰 core（不碰 GPUI），
    /// 所以在外殼 entity 被借用中呼叫也安全；選取改變的通知經 channel 回到主執行緒。
    fn on_expand(&self) {
        self.shared.modules.reset_selection_to_active();
    }

    fn on_shell_event(&self, event: ShellEvent) {
        let s = &self.shared;
        match event {
            ShellEvent::Expanded => {
                s.mode.set(ShellMode::Expanded);
                // 畫面重新出現：跑馬燈從頭、歌詞直接跳到目前行（同 Swift 的 onAppear）。
                s.title_marquee.reset();
                s.artist_marquee.reset();
                s.lyric_scroll.borrow_mut().jump = true;
            }
            ShellEvent::Collapsed => {
                s.mode.set(ShellMode::Preview);
                s.strip_marquee.reset();
                s.wing_title_marquee.reset();
                s.wing_artist_marquee.reset();
                // 歌詞條重新出現：直接顯示目前行，不從展開前的舊行捲過來。
                s.strip_scroll.borrow_mut().reset();
            }
            ShellEvent::Visible(v) => s.visible.set(v),
        }
        // 模式或可見性改變：定時器重新判斷（例如再顯示時音柱立刻恢復）。
        s.kick.kick();
        if let Some(hook) = &self.event_hook {
            hook(event);
        }
    }
}

/// 一個畫面（Preview 或 Expanded）。
struct MusicView {
    shared: Rc<Shared>,
    kind: ShellMode,
}

fn white_a(opacity: f64) -> Hsla {
    white().opacity(opacity as f32)
}

impl Render for MusicView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 釋放被快取擠出的封面在 GPU 圖集裡的空間。
        let released: Vec<_> = self.shared.released.borrow_mut().drain(..).collect();
        for image in released {
            let _ = window.drop_image(image);
        }
        // 只有這一格畫到的跑馬燈會把旗標設回來（見 MarqueeState::clear_scrolling）。
        match self.kind {
            ShellMode::Preview => {
                self.shared.strip_marquee.clear_scrolling();
                self.shared.wing_title_marquee.clear_scrolling();
                self.shared.wing_artist_marquee.clear_scrolling();
            }
            ShellMode::Expanded => {
                self.shared.title_marquee.clear_scrolling();
                self.shared.artist_marquee.clear_scrolling();
            }
        }
        match self.kind {
            ShellMode::Preview => self.render_preview(window, cx),
            ShellMode::Expanded => self.render_expanded(window, cx),
        }
    }
}

impl MusicView {
    // ------------------------------------------------------------------ Preview

    fn render_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.shared.clone();
        let empty = || div().size_full().into_any_element();
        // active 模組不是音樂（沒有曲目）時 Preview 不畫任何內容（Swift DefaultShellContent 是 EmptyView）。
        if !s.music_is_active() {
            return empty();
        }
        let state = s.state();
        let Some(snapshot) = state.snapshot.as_ref() else {
            return empty();
        };
        let g = s.geometry.get();
        let notch_w = finite_non_negative(g.notch.w);
        let row_h = finite_non_negative(g.notch.h);
        let extra = s.modules.preview_extra_height();
        let size = mode_size(ShellMode::Preview, &g, extra);
        let wing = wing_width(size.w, notch_w);
        let side = preview_cover_side(wing, row_h);

        let labels = preview_labels(snapshot);
        if labels.is_none() {
            // 暫停：下次出現時從頭捲。
            s.wing_title_marquee.reset();
            s.wing_artist_marquee.reset();
        }
        // 兩翼的文字佔封面／音柱與瀏海之間剩下的寬度，放不下就跑馬燈。
        let label = |state: &MarqueeState, text: &str, opacity: f64| {
            div().flex_1().min_w_0().child(marquee(
                state,
                text.to_string(),
                MarqueeStyle {
                    font_size: PREVIEW_TEXT_FONT_SIZE as f32,
                    weight: FontWeight::MEDIUM,
                    color: white_a(opacity),
                    line_height: PREVIEW_TEXT_LINE_HEIGHT as f32,
                },
            ))
        };

        let mut left = div()
            .w(px(wing as f32))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(PREVIEW_TEXT_SPACING as f32))
            .pl(px(PREVIEW_COVER_INSET as f32))
            .pr(px(PREVIEW_TEXT_NOTCH_INSET as f32))
            .overflow_hidden()
            .child(self.cover(snapshot.artwork.as_ref(), side, cx));
        if let Some((title, _)) = labels {
            left = left.child(label(&s.wing_title_marquee, title, PREVIEW_TITLE_OPACITY));
        }
        // 真瀏海後面：不放任何東西。
        let middle = div().w(px(notch_w as f32)).h_full().flex_none();
        let mut right = div()
            .w(px(wing as f32))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(PREVIEW_TEXT_SPACING as f32))
            .pl(px(PREVIEW_TEXT_NOTCH_INSET as f32))
            .pr(px(PREVIEW_BARS_INSET as f32))
            .overflow_hidden();
        if let Some((_, Some(artist))) = labels {
            right = right.child(label(
                &s.wing_artist_marquee,
                artist,
                PREVIEW_ARTIST_OPACITY,
            ));
        }
        let right = right.child(equalizer(s.t0.elapsed().as_secs_f64(), snapshot.is_playing));
        let wings = div()
            .w_full()
            .h(px(row_h as f32))
            .flex_none()
            .flex()
            .flex_row()
            .child(left)
            .child(middle)
            .child(right);

        let mut root = div().size_full().flex().flex_col().child(wings);
        if extra <= 0.0 {
            // 歌詞條收起（暫停或使用者折疊）：下次出現時從頭顯示目前行，不接著舊的捲動位置。
            s.strip_marquee.reset();
            s.strip_scroll.borrow_mut().reset();
        } else {
            let rows = self.strip_content_element(snapshot, &state, window);
            // 歌詞條：外殼在這一段不塗黑，由內容畫 0.6 不透明的黑底與下方圓角（Swift LyricStripView）。
            // 高度就是模組回報的額外高度（隨行數變），列在其中垂直置中。
            root = root.child(
                div()
                    .w_full()
                    .h(px(extra as f32))
                    .flex_none()
                    .bg(black().opacity(LYRIC_STRIP_OPACITY as f32))
                    .rounded_b(px(STRIP_CORNER_RADIUS as f32))
                    .px(px(STRIP_HORIZONTAL_PADDING as f32))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .child(rows),
            );
        }
        root.into_any_element()
    }

    /// 歌詞條黑底裡的內容：同步歌詞時是會換行捲動的歌詞列，否則是單列的歌詞命中狀態。
    fn strip_content_element(
        &self,
        snapshot: &TrackSnapshot,
        state: &MusicState,
        window: &mut Window,
    ) -> AnyElement {
        let s = &self.shared;
        let now = s.clock.now();
        let lines = s.module_state.strip_lines();
        let style = |opacity: f64| MarqueeStyle {
            font_size: STRIP_FONT_SIZE as f32,
            weight: FontWeight::MEDIUM,
            color: white_a(opacity),
            line_height: LYRIC_STRIP_ROW_HEIGHT as f32,
        };
        // 每一列固定高度；目前行用跑馬燈（太長會捲動），前後文只截斷（不捲動，避免多行同時動）。
        let row_box = || {
            div()
                .w_full()
                .h(px(LYRIC_STRIP_ROW_HEIGHT as f32))
                .flex_none()
                .overflow_hidden()
        };

        let lyrics = state.lyrics.as_ref();
        let current = strip_line_index(Some(snapshot), lyrics, now);
        let (Some(current), Some(Lyrics::Synced(all))) = (current, lyrics) else {
            s.strip_scroll.borrow_mut().reset();
            // 沒有可顯示的歌詞行：只顯示命中狀態；曲名已經在左翼，不在這裡重複。
            return row_box()
                .whitespace_nowrap()
                .text_center()
                .text_size(px(STRIP_FONT_SIZE as f32))
                .line_height(px(LYRIC_STRIP_ROW_HEIGHT as f32))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white_a(STRIP_STATUS_OPACITY))
                .child(lyrics_status_label(lyrics_status(lyrics, state.searching)))
                .into_any_element();
        };

        let position = {
            let mut scroll = s.strip_scroll.borrow_mut();
            let t = s.t0.elapsed().as_secs_f64();
            let position = scroll.follow((snapshot.title.clone(), all.len(), lines), current, t);
            if scroll.is_animating(t) {
                window.request_animation_frame();
            }
            position
        };

        // 固定高度的視窗，列依捲動位置絕對定位；滑出視窗的部分被裁掉。
        div()
            .relative()
            .w_full()
            .h(px((f64::from(lines) * LYRIC_STRIP_ROW_HEIGHT) as f32))
            .flex_none()
            .overflow_hidden()
            .children(strip_rows(all, position, lines).into_iter().map(
                |StripRow {
                     index,
                     text,
                     slot,
                     emphasis,
                 }| {
                    let opacity = strip_row_opacity(emphasis);
                    let row = row_box()
                        .absolute()
                        .left_0()
                        .top(px((slot * LYRIC_STRIP_ROW_HEIGHT) as f32));
                    if index == current {
                        row.child(marquee(&s.strip_marquee, text, style(opacity)))
                    } else {
                        row.whitespace_nowrap()
                            .text_ellipsis()
                            .text_center()
                            .text_size(px(STRIP_FONT_SIZE as f32))
                            .line_height(px(LYRIC_STRIP_ROW_HEIGHT as f32))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white_a(opacity))
                            .child(text)
                    }
                },
            ))
            .into_any_element()
    }

    // ------------------------------------------------------------------ 封面

    /// 封面：已解碼就畫置中裁切好的正方形（圓角 6），否則畫佔位（Swift CoverView）。
    fn cover(&self, artwork: Option<&ArtworkRef>, side: f64, cx: &mut Context<Self>) -> AnyElement {
        let side_px = px(side as f32);
        match artwork.and_then(|a| self.cover_image(a, cx)) {
            Some(image) => img(image)
                .size(side_px)
                .flex_none()
                .rounded(px(COVER_CORNER_RADIUS as f32))
                .object_fit(ObjectFit::Fill)
                .into_any_element(),
            None => div()
                .size(side_px)
                .flex_none()
                .rounded(px(COVER_CORNER_RADIUS as f32))
                .bg(white_a(COVER_PLACEHOLDER_OPACITY))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px((side * COVER_PLACEHOLDER_GLYPH_SCALE) as f32))
                .line_height(px(side as f32))
                .text_color(white_a(COVER_PLACEHOLDER_GLYPH_OPACITY))
                .child(MUSIC_NOTE_GLYPH)
                .into_any_element(),
        }
    }

    /// 查快取；第一次見到的封面在背景解碼，完成後重繪這個 view。
    fn cover_image(
        &self,
        artwork: &ArtworkRef,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let data = artwork.data.as_ref().filter(|d| !d.is_empty())?;
        let key = CoverKey {
            id: artwork.id.clone(),
            len: data.len(),
        };
        let cached = self.shared.covers.borrow_mut().get(&key);
        match cached {
            Some(CoverEntry::Ready(image)) => Some(image),
            Some(CoverEntry::Failed) => None,
            None => {
                // 排進解碼佇列（最新的要求取代排隊中的舊要求），先畫佔位。
                self.shared.decodes.borrow_mut().request(key, data.clone());
                self.pump_decodes(cx);
                None
            }
        }
    }

    /// 沒有解碼在進行時，在背景開始解排隊中的封面；完成後只在結果仍需要時放進快取，接著解下一個。
    fn pump_decodes(&self, cx: &mut Context<Self>) {
        let Some((key, bytes)) = self.shared.decodes.borrow_mut().start_next() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move { decode_cover(&bytes, COVER_TEXTURE_PIXELS) })
                .await;
            // view 已被釋放（內容 drop 中）時 update 失敗、佇列的 in_flight 不會清掉：無害，佇列隨 Shared 一起消失。
            let _ = this.update(cx, |view, cx| {
                let wanted = view.shared.decodes.borrow_mut().finish(&key);
                if wanted {
                    let entry = match decoded {
                        Some(image) => CoverEntry::Ready(Arc::new(image)),
                        // 失敗也記住，不重試（畫佔位）。
                        None => CoverEntry::Failed,
                    };
                    let evicted = view.shared.covers.borrow_mut().insert(key, entry);
                    view.shared.released.borrow_mut().extend(evicted);
                }
                // 不需要的結果直接丟棄：它從沒畫過，不在 GPU 圖集裡。
                view.pump_decodes(cx);
                cx.notify();
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------ Expanded

    fn render_expanded(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.shared.clone();
        let page = if s.music_is_selected() {
            self.music_page(window, cx)
        } else {
            // DefaultShellContent：緊接在切換列下方、水平置中。
            div()
                .w_full()
                .flex()
                .justify_center()
                .text_size(px(DEFAULT_EXPANDED_FONT_SIZE as f32))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white_a(DEFAULT_EXPANDED_OPACITY))
                .child(DEFAULT_EXPANDED_TEXT)
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(APP_BAR_SPACING as f32))
            .font_family(SYSTEM_FONT)
            .child(self.app_bar())
            .child(page)
            .into_any_element()
    }

    /// 模組切換列（Swift ModuleAppBar）：每個已安裝模組一個圖示，選取者不透明度 1、其餘 0.4；點擊即選取。
    fn app_bar(&self) -> AnyElement {
        let modules = &self.shared.modules;
        let selected = modules.selected_module_id();
        let icons = modules.installed_module_ids().into_iter().map(|id| {
            let is_selected = selected.as_deref() == Some(id.as_str());
            let glyph = if id == MusicModuleState::ID {
                MUSIC_NOTE_GLYPH
            } else {
                "\u{2022}"
            };
            let modules = modules.clone();
            let click_id = id.clone();
            div()
                .id(SharedString::from(format!("module-tab:{id}")))
                .h_full()
                .flex()
                .items_center()
                .text_size(px(APP_BAR_ICON_SIZE as f32))
                .line_height(px(APP_BAR_HEIGHT as f32))
                .text_color(white_a(if is_selected {
                    1.0
                } else {
                    APP_BAR_UNSELECTED_OPACITY
                }))
                .child(glyph)
                .on_click(move |_, _, _| modules.select(&click_id))
        });
        div()
            .w_full()
            .h(px(APP_BAR_HEIGHT as f32))
            .flex_none()
            .flex()
            .flex_row()
            .justify_center()
            .items_center()
            .gap(px(APP_BAR_ICON_SPACING as f32))
            .children(icons)
            .into_any_element()
    }

    /// 音樂頁（Swift MusicExpandedView）。
    fn music_page(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let s = self.shared.clone();
        let page = div()
            .w_full()
            .h(px(MUSIC_PAGE_HEIGHT as f32))
            .flex_none()
            .px(px(PAGE_HORIZONTAL_PADDING as f32))
            .pb(px(PAGE_BOTTOM_PADDING as f32))
            .flex()
            .flex_col()
            .overflow_hidden();
        let state = s.state();
        let Some(snapshot) = state.snapshot.as_ref() else {
            return page
                .items_center()
                .justify_center()
                .text_size(px(NO_TRACK_FONT_SIZE as f32))
                .text_color(white_a(NO_TRACK_OPACITY))
                .child(NO_TRACK_TEXT)
                .into_any_element();
        };
        let now = s.clock.now();
        let display = lyrics_display(snapshot, state.lyrics.as_ref(), now);

        let header = div()
            .w_full()
            .h(px(EXPANDED_COVER_SIDE as f32))
            .flex_none()
            .flex()
            .flex_row()
            .gap(px(HEADER_SPACING as f32))
            .child(self.cover(snapshot.artwork.as_ref(), EXPANDED_COVER_SIDE, cx))
            .child(self.info(snapshot, now, cx));

        let mut column = page.gap(px(SECTION_SPACING as f32)).child(header);
        match display {
            // 沒有歌詞：標頭在空間中垂直置中。
            LyricsDisplay::None => column = column.justify_center(),
            other => column = column.child(self.lyrics(snapshot, other, window)),
        }
        column.into_any_element()
    }

    /// 曲名、演出者、進度條、控制列（資訊欄，高 68）。
    fn info(&self, snapshot: &TrackSnapshot, now: f64, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.shared;
        let mut col = div()
            .flex_1()
            .min_w_0()
            .h(px(EXPANDED_COVER_SIDE as f32))
            .flex()
            .flex_col()
            .child(marquee(
                &s.title_marquee,
                snapshot.title.clone(),
                MarqueeStyle {
                    font_size: TITLE_FONT_SIZE as f32,
                    weight: FontWeight::SEMIBOLD,
                    color: white(),
                    line_height: TITLE_LINE_HEIGHT as f32,
                },
            ));
        if let Some(artist) = &snapshot.artist {
            col = col.child(marquee(
                &s.artist_marquee,
                artist.clone(),
                MarqueeStyle {
                    font_size: ARTIST_FONT_SIZE as f32,
                    weight: FontWeight::NORMAL,
                    color: white_a(ARTIST_OPACITY),
                    line_height: ARTIST_LINE_HEIGHT as f32,
                },
            ));
        }
        col = col.child(div().flex_1().min_h(px(INFO_MIN_SPACER as f32)));
        if has_progress(snapshot) {
            // 資訊欄寬 = 外形寬 − 左右內距 − 封面 − 間距。
            let g = s.geometry.get();
            let width = mode_size(ShellMode::Expanded, &g, 0.0).w
                - 2.0 * PAGE_HORIZONTAL_PADDING
                - EXPANDED_COVER_SIDE
                - HEADER_SPACING;
            let fill = progress_fill_width(progress_fraction(snapshot, now), width);
            col = col.child(
                div()
                    .w_full()
                    .h(px(PROGRESS_HEIGHT as f32))
                    .flex_none()
                    .mb(px(PROGRESS_BOTTOM_PADDING as f32))
                    .relative()
                    .rounded_full()
                    .bg(white_a(PROGRESS_TRACK_OPACITY))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h_full()
                            .w(px(fill as f32))
                            .rounded_full()
                            .bg(white_a(PROGRESS_FILL_OPACITY)),
                    ),
            );
        }
        col.child(self.controls(snapshot, cx)).into_any_element()
    }

    /// 上一首／播放暫停／下一首（置中），右側是歌詞條折疊開關，兩者之間放時間偏移控制；左側等寬的區域放
    /// 歌詞命中狀態（有多個版本時點它換版本）與來源標籤（沒有網址時空白）。兩側寬度與內容無關，按鈕維持置中。
    fn controls(&self, snapshot: &TrackSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.shared;
        let enabled = s.controller.has_control();
        let button = |id: &'static str, glyph: Glyph, size: f64, cmd: PlaybackCommand| {
            let color = white_a(if enabled {
                CONTROL_ENABLED_OPACITY
            } else {
                CONTROL_DISABLED_OPACITY
            });
            let el = div()
                .id(id)
                .w(px(CONTROL_BUTTON_WIDTH as f32))
                .h(px(CONTROL_ROW_HEIGHT as f32))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(glyph_element(glyph, size, color));
            if enabled {
                let controller = s.controller.clone();
                el.on_click(move |_, _, _| controller.send(cmd))
                    .into_any_element()
            } else {
                el.into_any_element()
            }
        };
        let play_glyph = if snapshot.is_playing {
            Glyph::Pause
        } else {
            Glyph::Play
        };
        let buttons = div()
            .flex()
            .flex_row()
            .flex_none()
            .gap(px(CONTROL_BUTTON_SPACING as f32))
            .child(button(
                "music-previous",
                Glyph::Previous,
                SKIP_GLYPH_SIZE,
                PlaybackCommand::Previous,
            ))
            .child(button(
                "music-play-pause",
                play_glyph,
                PLAY_GLYPH_SIZE,
                PlaybackCommand::TogglePlayPause,
            ))
            .child(button(
                "music-next",
                Glyph::Next,
                SKIP_GLYPH_SIZE,
                PlaybackCommand::Next,
            ));

        let strip_visible = s.module_state.strip_visible();
        let fold = div()
            .id("music-strip-fold")
            .size(px(CONTROL_ROW_HEIGHT as f32))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(glyph_element(
                if strip_visible {
                    Glyph::ChevronUp
                } else {
                    Glyph::ChevronDown
                },
                FOLD_GLYPH_SIZE,
                white_a(FOLD_OPACITY),
            ))
            .on_click(cx.listener(|view, _, _, cx| {
                let module = &view.shared.module_state;
                module.set_strip_visible(!module.strip_visible());
                // 暫停中折疊不改變額外高度，模組不會通知：自己重繪開關的樣子。
                cx.notify();
            }));

        // 左側區域取代原本的「24 方塊 + flex_1」：flex_1 但基準寬 24，所以寬度仍等於右側的
        // 「flex_1 + 24 折疊開關」（見 `control_side_widths`），按鈕群位置不變。`min_w_0` + 裁切讓標籤
        // 再長也只會被截斷，不會撐寬這一側去推擠按鈕。
        // 上行是歌詞命中狀態，下行是來源連結（沒有網址時只剩狀態，垂直置中）。
        let state = s.state();
        // `version` 只在顯示時間軸歌詞時是 `Some`：查詢中、沒有歌詞、純文字版都沒有版本切換與偏移控制。
        let status = match state.version {
            Some(v) => version_label(v.index, v.count),
            None => lyrics_status_label(lyrics_status(state.lyrics.as_ref(), state.searching))
                .to_string(),
        };
        let status = div()
            .max_w_full()
            .truncate()
            .text_size(px(SOURCE_LABEL_FONT_SIZE as f32))
            .line_height(px(SOURCE_LABEL_LINE_HEIGHT as f32))
            .text_color(white_a(STATUS_LABEL_OPACITY))
            .child(status);
        let status = if state.version.is_some_and(|v| v.count >= 2) {
            status
                .id("music-version-next")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                    let s = &view.shared;
                    // 換版本是整份歌詞換掉：直接跳到目前行，不從舊版本的位置捲過去。
                    s.lyric_scroll.borrow_mut().jump = true;
                    s.controller.next_version();
                    // 事件工作稍後才把快照標為過期；這裡先標，跳躍才不會被舊歌詞的一格用掉。
                    s.state_dirty.set(true);
                    cx.notify();
                }))
                .into_any_element()
        } else {
            status.into_any_element()
        };
        let mut source = div()
            .flex_1()
            .flex_basis(px(CONTROL_ROW_HEIGHT as f32))
            .min_w_0()
            .h(px(CONTROL_ROW_HEIGHT as f32))
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_start()
            .justify_center()
            .child(status);
        if let Some(label) = state.lyrics_url.as_deref().and_then(source_label) {
            let host = label
                .strip_suffix(SOURCE_LABEL_SUFFIX)
                .unwrap_or(&label)
                .to_string();
            source = source.child(
                div()
                    .id("music-source-link")
                    .min_w_0()
                    .max_w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(SOURCE_LABEL_FONT_SIZE as f32))
                    .line_height(px(SOURCE_LABEL_LINE_HEIGHT as f32))
                    .text_color(white_a(SOURCE_LABEL_OPACITY))
                    // 主機名稱可被省略號截斷；箭頭固定保留。
                    .child(div().min_w_0().truncate().child(host))
                    .child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .child(SOURCE_LABEL_SUFFIX),
                    )
                    .on_click(cx.listener(|view, _, _, _| {
                        // 讀點擊當下的狀態，不用 render 時的舊值（換曲後網址可能已變或清空）。
                        let url = view.shared.state().lyrics_url.clone();
                        if let Some(url) = url {
                            (view.shared.open_url)(&url);
                        }
                    })),
            );
        }
        div()
            .w_full()
            .h(px(CONTROL_ROW_HEIGHT as f32))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .child(source)
            .child(buttons)
            .child(self.offset_controls(state.version))
            .child(fold)
            .into_any_element()
    }

    /// 按鈕群與折疊開關之間的彈性區：顯示時間軸歌詞時放 `−`、目前偏移（點一下歸零）、`＋`，否則空白。
    /// ＋ 讓歌詞提早出現；按住 Option 點走大步（見 [`offset_step`]）。`min_w_0` 加裁切讓這一區的寬度
    /// 只由 flex 分配，內容不會推擠按鈕群。
    fn offset_controls(&self, version: Option<VersionInfo>) -> AnyElement {
        let area = div()
            .flex_1()
            .min_w_0()
            .h(px(CONTROL_ROW_HEIGHT as f32))
            .overflow_hidden()
            .flex()
            .flex_row()
            .items_center()
            .justify_center();
        let Some(version) = version else {
            return area.into_any_element();
        };
        let cell = |id: &'static str, width: f64, text: SharedString| {
            div()
                .id(id)
                .w(px(width as f32))
                .h(px(CONTROL_ROW_HEIGHT as f32))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .whitespace_nowrap()
                .cursor_pointer()
                .text_size(px(SOURCE_LABEL_FONT_SIZE as f32))
                .line_height(px(SOURCE_LABEL_LINE_HEIGHT as f32))
                .text_color(white_a(STATUS_LABEL_OPACITY))
                .child(text)
        };
        // 控制器的三個版本動作要在主執行緒呼叫；點擊回呼就在主執行緒。
        let step = |id: &'static str, glyph: &'static str, sign: f64| {
            let controller = self.shared.controller.clone();
            cell(id, OFFSET_BUTTON_WIDTH, glyph.into()).on_click(move |event, _, _| {
                controller.nudge_offset(sign * offset_step(event.modifiers().alt))
            })
        };
        let controller = self.shared.controller.clone();
        area.child(step("music-offset-minus", OFFSET_MINUS_GLYPH, -1.0))
            .child(
                cell(
                    "music-offset-reset",
                    OFFSET_VALUE_WIDTH,
                    offset_label(version.offset).into(),
                )
                .on_click(move |_, _, _| controller.reset_offset()),
            )
            .child(step("music-offset-plus", OFFSET_PLUS_GLYPH, 1.0))
            .into_any_element()
    }

    /// 歌詞區（高 67）：純文字與位置未知的同步歌詞是靜態全文（可用滾輪捲動）；位置已知時跟隨並標示目前行。
    fn lyrics(
        &self,
        snapshot: &TrackSnapshot,
        display: LyricsDisplay<'_>,
        window: &mut Window,
    ) -> AnyElement {
        let area = || {
            div()
                .w_full()
                .h(px(LYRICS_VIEWPORT_HEIGHT as f32))
                .flex_none()
                .text_size(px(LYRIC_FONT_SIZE as f32))
                .line_height(px(LYRIC_LINE_HEIGHT as f32))
        };
        let static_text = |lines: Vec<SharedString>, opacity: f64| {
            area()
                .id("music-lyrics-static")
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(LYRIC_LINE_SPACING as f32))
                .text_color(white_a(opacity))
                .children(lines.into_iter().map(|line| {
                    div()
                        .w_full()
                        .flex_none()
                        .min_h(px(LYRIC_LINE_HEIGHT as f32))
                        .text_center()
                        .child(line)
                }))
                .into_any_element()
        };
        match display {
            LyricsDisplay::None => div().into_any_element(),
            LyricsDisplay::Plain(text) => static_text(
                text.lines()
                    .map(|l| SharedString::from(l.to_string()))
                    .collect(),
                LYRIC_PLAIN_OPACITY,
            ),
            LyricsDisplay::Static(lines) => static_text(
                lines
                    .iter()
                    .map(|l| SharedString::from(l.text.clone()))
                    .collect(),
                LYRIC_OTHER_OPACITY,
            ),
            LyricsDisplay::Following { lines, current } => {
                let target = lyric_scroll_offset(current, lines.len(), LYRICS_VIEWPORT_HEIGHT);
                let offset =
                    self.scroll_offset((snapshot.title.clone(), lines.len()), target, window);
                area()
                    .relative()
                    .overflow_hidden()
                    .children(visible_lines(lines, offset).map(|(i, line)| {
                        let is_current = current == Some(i);
                        div()
                            .absolute()
                            .left_0()
                            .top(px((offset + i as f64 * LYRIC_PITCH) as f32))
                            .w_full()
                            .h(px(LYRIC_LINE_HEIGHT as f32))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_center()
                            .font_weight(if is_current {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(white_a(if is_current {
                                LYRIC_CURRENT_OPACITY
                            } else {
                                LYRIC_OTHER_OPACITY
                            }))
                            .child(line.text.clone())
                    }))
                    .into_any_element()
            }
        }
    }

    /// 目前的捲動位移：目標改變時以 0.3 秒 ease-in-out 過去（動畫中每格要求重繪）；換歌詞或剛展開時直接跳。
    fn scroll_offset(&self, key: (String, usize), target: f64, window: &mut Window) -> f64 {
        let mut sc = self.shared.lyric_scroll.borrow_mut();
        let now = Instant::now();
        if sc.jump || sc.key.as_ref() != Some(&key) {
            *sc = LyricScroll {
                key: Some(key),
                from: target,
                to: target,
                started: None,
                jump: false,
            };
            return target;
        }
        if target != sc.to {
            let from = sc.value(now);
            sc.from = from;
            sc.to = target;
            sc.started = Some(now);
        }
        if sc.animating(now) {
            window.request_animation_frame();
        }
        sc.value(now)
    }
}

/// 位移為 `offset` 時落在歌詞區內（含上下各一行餘裕）的行。
fn visible_lines(lines: &[LyricLine], offset: f64) -> impl Iterator<Item = (usize, &LyricLine)> {
    let first = ((-offset) / LYRIC_PITCH).floor().max(0.0) as usize;
    let count = (LYRICS_VIEWPORT_HEIGHT / LYRIC_PITCH).ceil() as usize + 2;
    lines
        .iter()
        .enumerate()
        .skip(first.saturating_sub(1))
        .take(count + 1)
}

fn finite_non_negative(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        0.0
    }
}

/// 四根音柱，底部對齊（Swift EqualizerBars）。
fn equalizer(t: f64, playing: bool) -> impl IntoElement {
    div()
        .w(px(equalizer_width() as f32))
        .h(px(EQ_HEIGHT as f32))
        .flex_none()
        .flex()
        .flex_row()
        .items_end()
        .gap(px(EQ_BAR_SPACING as f32))
        .children((0..EQ_DURATIONS.len()).map(move |i| {
            div()
                .w(px(EQ_BAR_WIDTH as f32))
                .h(px((EQ_HEIGHT * equalizer_scale(i, t, playing)) as f32))
                .flex_none()
                .rounded_full()
                .bg(white_a(EQ_BAR_OPACITY))
        }))
}

/// 以路徑畫的字形（取代 SF Symbols）。
fn glyph_element(glyph: Glyph, size: f64, color: Hsla) -> impl IntoElement {
    let (w, h) = glyph_box(glyph, size);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _cx: &mut App| {
            for shape in glyph_shapes(glyph, size) {
                let mut builder = if shape.filled {
                    PathBuilder::fill()
                } else {
                    PathBuilder::stroke(px(CHEVRON_STROKE as f32))
                };
                let points: Vec<_> = shape
                    .points
                    .iter()
                    .map(|&(x, y)| {
                        point(
                            bounds.origin.x + px(x as f32),
                            bounds.origin.y + px(y as f32),
                        )
                    })
                    .collect();
                builder.add_polygon(&points, shape.filled);
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .w(px(w as f32))
    .h(px(h as f32))
    .flex_none()
}
