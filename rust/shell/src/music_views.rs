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
//! 主執行緒上的 GPUI 工作（`App::spawn`）等這個 channel，收到時清旗標、對兩個 entity `cx.notify()`；
//! render 一律重讀 `MusicController::state()` 等現值，不信任通知的順序。回呼只持有 channel 的送出端，
//! 不持有任何 GPUI 物件，所以從不在外部執行緒碰 GPUI。
//!
//! 定時重繪（音柱 12 fps、跑馬燈捲動時 30 fps、歌詞條／進度／歌詞跟隨 4 Hz）由另一個主執行緒工作依
//! [`tick_interval`](crate::music_layout::tick_interval) 決定：面板隱藏、Preview 沒在播、Expanded 沒有隨時間
//! 變化的東西時不重繪，只每 0.5 秒重新判斷一次（不畫任何東西）。只有歌詞換行的 0.3 秒捲動動畫由 render
//! `request_animation_frame()`，只在進行中才要求。
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
use futures_util::StreamExt;
use gpui::{
    black, canvas, div, img, point, prelude::*, px, white, AnyElement, App, AsyncApp, Bounds,
    Context, Entity, FontWeight, Hsla, ObjectFit, PathBuilder, Pixels, RenderImage, SharedString,
    StyledImage, Task, WeakEntity, Window,
};
use musicat_core::lyrics::LyricLine;
use musicat_core::modules::{ModuleManager, ModuleState};
use musicat_core::music::controller::MusicController;
use musicat_core::music::module::{MusicModuleState, LYRIC_STRIP_HEIGHT, LYRIC_STRIP_OPACITY};
use musicat_core::music::presentation::{
    has_progress, progress_fraction, strip_content, StripContent,
};
use musicat_core::nowplaying::PlaybackCommand;
use musicat_core::runtime::{Clock, Subscription};
use musicat_core::shell_logic::{mode_size, NotchGeometry, ShellMode};
use musicat_core::types::{ArtworkRef, TrackSnapshot};

use crate::artwork::{
    decode_cover, CoverCache, CoverEntry, CoverKey, COVER_CACHE_CAPACITY, COVER_TEXTURE_PIXELS,
};
use crate::marquee::{marquee, MarqueeState, MarqueeStyle, SYSTEM_FONT};
use crate::music_layout::*;
use crate::{ShellContent, ShellEvent};

/// 沒有需要定時重繪時，多久重新判斷一次（秒）。只讀狀態、不畫任何東西。
const IDLE_RECHECK_SECONDS: f64 = 0.5;

/// 兩個 view 共用的狀態。只在 GPUI 主執行緒使用。
struct Shared {
    controller: Arc<MusicController>,
    module_state: Arc<MusicModuleState>,
    modules: Arc<ModuleManager>,
    clock: Arc<dyn Clock>,
    geometry: Cell<NotchGeometry>,
    /// 外殼目前的模式（`on_shell_event` 與 `preview()`／`expanded()` 更新）。
    mode: Cell<ShellMode>,
    /// 面板實際可見（`ShellEvent::Visible`）。
    visible: Cell<bool>,
    covers: RefCell<CoverCache<Arc<RenderImage>>>,
    /// 被快取擠出、等下一次 render（有 `Window`）時從 GPU 圖集釋放的封面。
    released: RefCell<Vec<Arc<RenderImage>>>,
    strip_marquee: MarqueeState,
    title_marquee: MarqueeState,
    artist_marquee: MarqueeState,
    lyric_scroll: RefCell<LyricScroll>,
    /// 音柱動畫的時間原點。
    t0: Instant,
}

impl Shared {
    fn music_is_active(&self) -> bool {
        self.modules.active_module_id().as_deref() == Some(MusicModuleState::ID)
    }

    fn music_is_selected(&self) -> bool {
        self.modules.selected_module_id().as_deref() == Some(MusicModuleState::ID)
    }

    fn tick_inputs(&self) -> TickInputs {
        let state = self.controller.state();
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
    /// 下一次直接跳到位置（剛展開）。
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
    pub fn new(
        cx: &mut App,
        geometry: NotchGeometry,
        controller: Arc<MusicController>,
        module_state: Arc<MusicModuleState>,
        modules: Arc<ModuleManager>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let shared = Rc::new(Shared {
            controller,
            module_state,
            modules,
            clock,
            geometry: Cell::new(geometry),
            mode: Cell::new(ShellMode::Preview),
            visible: Cell::new(true),
            covers: RefCell::new(CoverCache::new(COVER_CACHE_CAPACITY)),
            released: RefCell::new(Vec::new()),
            strip_marquee: MarqueeState::new(),
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

        let (wp, we) = (preview.downgrade(), expanded.downgrade());
        let events = cx.spawn(async move |cx: &mut AsyncApp| {
            while rx.next().await.is_some() {
                pending.store(false, Ordering::SeqCst);
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
            cx.background_executor()
                .timer(Duration::from_secs_f64(
                    interval.unwrap_or(IDLE_RECHECK_SECONDS),
                ))
                .await;
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

    fn preview_extra_height(&self) -> f64 {
        self.shared.module_state.preview_extra_height()
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
            }
            ShellEvent::Visible(v) => s.visible.set(v),
        }
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
            ShellMode::Preview => self.shared.strip_marquee.clear_scrolling(),
            ShellMode::Expanded => {
                self.shared.title_marquee.clear_scrolling();
                self.shared.artist_marquee.clear_scrolling();
            }
        }
        match self.kind {
            ShellMode::Preview => self.render_preview(cx),
            ShellMode::Expanded => self.render_expanded(window, cx),
        }
    }
}

impl MusicView {
    // ------------------------------------------------------------------ Preview

    fn render_preview(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = self.shared.clone();
        let empty = || div().size_full().into_any_element();
        // 同 Swift：active 模組不是音樂（沒在播放）時 Preview 不畫任何內容（DefaultShellContent 是 EmptyView）。
        if !s.music_is_active() {
            return empty();
        }
        let state = s.controller.state();
        let Some(snapshot) = state.snapshot.as_ref() else {
            return empty();
        };
        let g = s.geometry.get();
        let notch_w = finite_non_negative(g.notch.w);
        let row_h = finite_non_negative(g.notch.h);
        let extra = s.module_state.preview_extra_height();
        let size = mode_size(ShellMode::Preview, &g, extra);
        let wing = wing_width(size.w, notch_w);
        let side = preview_cover_side(wing, row_h);

        let left = div()
            .w(px(wing as f32))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .pl(px(PREVIEW_COVER_INSET as f32))
            .child(self.cover(snapshot.artwork.as_ref(), side, cx));
        // 真瀏海後面：不放任何東西。
        let middle = div().w(px(notch_w as f32)).h_full().flex_none();
        let right = div()
            .w(px(wing as f32))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .pr(px(PREVIEW_BARS_INSET as f32))
            .child(equalizer(s.t0.elapsed().as_secs_f64(), snapshot.is_playing));
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
        if extra > 0.0 {
            let content = strip_content(Some(snapshot), state.lyrics.as_ref(), s.clock.now());
            let (text, opacity) = match content {
                StripContent::Line(t) => (t, STRIP_LINE_OPACITY),
                StripContent::Title(t) => (t, STRIP_TITLE_OPACITY),
                StripContent::None => (String::new(), STRIP_TITLE_OPACITY),
            };
            // 歌詞條：外殼在這一段不塗黑，由內容畫 0.6 不透明的黑底與下方圓角（Swift LyricStripView）。
            root = root.child(
                div()
                    .w_full()
                    .h(px(LYRIC_STRIP_HEIGHT as f32))
                    .flex_none()
                    .bg(black().opacity(LYRIC_STRIP_OPACITY as f32))
                    .rounded_b(px(STRIP_CORNER_RADIUS as f32))
                    .px(px(STRIP_HORIZONTAL_PADDING as f32))
                    .flex()
                    .items_center()
                    .child(marquee(
                        &s.strip_marquee,
                        text,
                        MarqueeStyle {
                            font_size: STRIP_FONT_SIZE as f32,
                            weight: FontWeight::MEDIUM,
                            color: white_a(opacity),
                            line_height: (STRIP_FONT_SIZE + 4.0) as f32,
                        },
                    )),
            );
        }
        root.into_any_element()
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
        let mut covers = self.shared.covers.borrow_mut();
        match covers.get(&key) {
            Some(CoverEntry::Ready(image)) => Some(image),
            Some(CoverEntry::Pending | CoverEntry::Failed) => None,
            None => {
                let evicted = covers.insert(key.clone(), CoverEntry::Pending);
                drop(covers);
                self.shared.released.borrow_mut().extend(evicted);
                let bytes = data.clone();
                cx.spawn(async move |this, cx| {
                    let decoded = cx
                        .background_executor()
                        .spawn(async move { decode_cover(&bytes, COVER_TEXTURE_PIXELS) })
                        .await;
                    let entry = match decoded {
                        Some(image) => CoverEntry::Ready(Arc::new(image)),
                        None => CoverEntry::Failed,
                    };
                    let _ = this.update(cx, |view, cx| {
                        let orphan = view.shared.covers.borrow_mut().finish(&key, entry);
                        view.shared.released.borrow_mut().extend(orphan);
                        cx.notify();
                    });
                })
                .detach();
                None
            }
        }
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
        let state = s.controller.state();
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

    /// 上一首／播放暫停／下一首（置中），右側是歌詞條折疊開關，左側等寬空白讓按鈕維持置中。
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

        let square = || div().size(px(CONTROL_ROW_HEIGHT as f32)).flex_none();
        div()
            .w_full()
            .h(px(CONTROL_ROW_HEIGHT as f32))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .child(square())
            .child(div().flex_1())
            .child(buttons)
            .child(div().flex_1())
            .child(fold)
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
