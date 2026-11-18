//! THROWAWAY SPIKE: can GPUI (gpui 0.2.2 from crates.io) do the NotchCat
//! notch-overlay shell? See ../SPEC.md and REPORT.md. Not production code.
//!
//! Layout:
//!   - geometry (NotchDetector port)            -> `detect_notch`
//!   - window creation via GPUI options          -> `main`
//!   - native NSWindow/NSApp fix-ups (unsafe)    -> `mod native`
//!   - hover state machine driven by polling     -> `Shell::poll`
//!   - drawn shape + 0.28 s animation            -> `impl Render for Shell`

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{
    div, ease_in_out, prelude::*, px, rgb, rgba, size, Animation, AnimationExt as _, App,
    Application, Bounds, Context, Pixels, SharedString, Window, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions,
};
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

const WIDTH_PADDING: f64 = 160.0;
const EXPANDED_H: f64 = 180.0;
const PREVIEW_RADIUS: f32 = 10.0;
const EXPANDED_RADIUS: f32 = 24.0;
const ANIM: Duration = Duration::from_millis(280);
const COLLAPSE_DELAY: Duration = Duration::from_millis(150);
const POLL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug)]
struct Notch {
    width: f64,
    height: f64,
    center_x: f64,
    top_y: f64, // AppKit global coords (bottom-left origin)
}

impl Notch {
    fn container_w(&self) -> f64 {
        self.width + WIDTH_PADDING
    }
    /// Fixed container frame in AppKit global coordinates.
    fn container_frame(&self) -> NSRect {
        NSRect::new(
            NSPoint::new(self.center_x - self.container_w() / 2.0, self.top_y - EXPANDED_H),
            NSSize::new(self.container_w(), EXPANDED_H),
        )
    }
    /// Precise Preview shape in AppKit global coordinates.
    fn preview_frame(&self) -> NSRect {
        let c = self.container_frame();
        NSRect::new(
            NSPoint::new(c.origin.x, self.top_y - self.height),
            NSSize::new(c.size.width, self.height),
        )
    }
}

/// Port of Swift `NotchDetector.detect()`.
fn detect_notch(mtm: MainThreadMarker) -> Option<Notch> {
    let screens = objc2_app_kit::NSScreen::screens(mtm);
    for screen in screens.iter() {
        let frame = screen.frame();
        let left = screen.auxiliaryTopLeftArea();
        let right = screen.auxiliaryTopRightArea();
        // objc2 returns NSRect (zero rect when the screen has no notch).
        if left.size.width <= 0.0 || right.size.width <= 0.0 {
            continue;
        }
        let w = frame.size.width - left.size.width - right.size.width;
        if w <= 0.0 {
            continue;
        }
        return Some(Notch {
            width: w,
            height: left.size.height,
            center_x: frame.origin.x + left.size.width + w / 2.0,
            top_y: frame.origin.y + frame.size.height,
        });
    }
    None
}

fn contains(r: NSRect, p: NSPoint) -> bool {
    // Top edge inclusive: the cursor pinned to the top row reports y == maxY.
    p.x >= r.origin.x
        && p.x < r.origin.x + r.size.width
        && p.y >= r.origin.y
        && p.y <= r.origin.y + r.size.height
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Preview,
    Expanded,
}

fn stamp() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("{}.{:03}", d.as_secs(), d.subsec_millis())
}

struct Shell {
    notch: Notch,
    ns_window: Retained<NSWindow>,
    mode: Mode,
    hover_rect_is_container: bool,
    was_inside: bool,
    pending_collapse: Option<Instant>,
    enters: u32,
    exits: u32,
    transitions: u32,
    // animation bookkeeping (to start an interrupted animation from where it is)
    anim_start: Instant,
    from_h: f32,
    from_r: f32,
    tick: u64,
    debug: bool,
    ignoring_mouse: Option<bool>,
    t0: Instant,
}

impl Shell {
    fn log(&self, what: &str) {
        println!(
            "[{} +{:>7}ms] {:<16} mode={:?} enters={} exits={} transitions={}",
            stamp(),
            self.t0.elapsed().as_millis(),
            what,
            self.mode,
            self.enters,
            self.exits,
            self.transitions
        );
    }

    fn target(mode: Mode, notch: &Notch) -> (f32, f32) {
        match mode {
            Mode::Preview => (notch.height as f32, PREVIEW_RADIUS),
            Mode::Expanded => (EXPANDED_H as f32, EXPANDED_RADIUS),
        }
    }

    /// Current (possibly mid-animation) height/radius, same easing as render.
    fn current(&self) -> (f32, f32) {
        let (th, tr) = Self::target(self.mode, &self.notch);
        let t = (self.anim_start.elapsed().as_secs_f32() / ANIM.as_secs_f32()).min(1.0);
        let e = ease_in_out(t);
        (self.from_h + (th - self.from_h) * e, self.from_r + (tr - self.from_r) * e)
    }

    fn transition(&mut self, to: Mode) {
        if self.mode == to {
            return;
        }
        let (h, r) = self.current();
        self.from_h = h;
        self.from_r = r;
        self.anim_start = Instant::now();
        self.mode = to;
        self.transitions += 1;
        self.log("TRANSITION");
    }

    fn hover_rect(&self) -> NSRect {
        if self.hover_rect_is_container {
            self.notch.container_frame()
        } else {
            self.notch.preview_frame()
        }
    }

    /// Polled at ~60 Hz. Mirrors NotchShellController's enter/exit/debounce,
    /// plus click-through toggling (ignoresMouseEvents outside the drawn shape).
    fn poll(&mut self, cx: &mut Context<Self>) {
        let p = objc2_app_kit::NSEvent::mouseLocation();
        let inside = contains(self.hover_rect(), p);
        let mut changed = false;

        if inside && !self.was_inside {
            self.enters += 1;
            self.log("hover-enter");
            self.pending_collapse = None;
            self.transition(Mode::Expanded);
            self.hover_rect_is_container = true;
            changed = true;
        } else if !inside && self.was_inside {
            self.exits += 1;
            self.log("hover-exit");
            self.pending_collapse = Some(Instant::now() + COLLAPSE_DELAY);
            changed = true;
        }
        self.was_inside = inside;

        if let Some(deadline) = self.pending_collapse {
            if Instant::now() >= deadline {
                self.pending_collapse = None;
                self.transition(Mode::Preview);
                self.hover_rect_is_container = false;
                self.was_inside = contains(self.hover_rect(), p);
                changed = true;
            }
        }

        // Click-through: accept mouse only over the drawn shape. In steady
        // state the drawn shape == the hover rect (Preview pill, or the whole
        // container once expanded).
        let over_shape = contains(self.hover_rect(), p);
        let ignore = !over_shape;
        if self.ignoring_mouse != Some(ignore) {
            self.ns_window.setIgnoresMouseEvents(ignore);
            self.ignoring_mouse = Some(ignore);
            self.log(if ignore { "click-through=ON" } else { "click-through=OFF" });
        }

        if changed {
            cx.notify();
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (to_h, to_r) = Self::target(self.mode, &self.notch);
        let (from_h, from_r) = (self.from_h, self.from_r);
        let cw = self.notch.container_w() as f32;
        let anim_log = std::env::var("SPIKE_ANIMLOG").as_deref() == Ok("1");
        let wing = ((cw as f64 - self.notch.width) / 2.0) as f32;
        let mode_txt = match self.mode {
            Mode::Preview => "Preview",
            Mode::Expanded => "Expanded",
        };
        let label: SharedString = format!(
            "{mode_txt}  in:{} out:{}  tick:{}",
            self.enters, self.exits, self.tick
        )
        .into();

        // Wing text (left of the hardware notch) so it is visible in Preview.
        let left_wing = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(wing))
            .h(px(self.notch.height as f32))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .text_color(rgb(0xffffff))
            .child(format!("{}", self.tick));
        let right_wing = div()
            .absolute()
            .top_0()
            .right_0()
            .w(px(wing))
            .h(px(self.notch.height as f32))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .text_color(rgb(0xffffff))
            .child(mode_txt);

        let body = div()
            .absolute()
            .top(px(self.notch.height as f32 + 8.))
            .left_0()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .text_color(rgb(0xffffff))
            .text_size(px(14.))
            .child(label)
            .child("夜空中最亮的星 能否聽清 — 中文歌詞測試")
            .child("繁體／简体／日本語かな");

        let shape = div()
            .id("shape")
            .relative()
            .overflow_hidden()
            .w(px(cw))
            .bg(rgb(0x000000))
            .child(left_wing)
            .child(right_wing)
            .child(body)
            .with_animation(
                // New id per transition => GPUI restarts the animation.
                SharedString::from(format!("shape-{}", self.transitions)),
                Animation::new(ANIM).with_easing(ease_in_out),
                move |el, t| {
                    let h = from_h + (to_h - from_h) * t;
                    let r = from_r + (to_r - from_r) * t;
                    if anim_log {
                        println!("[{}] anim-frame eased={:.3} h={:.1} r={:.1}", stamp(), t, h, r);
                    }
                    el.h(px(h)).rounded_bl(px(r)).rounded_br(px(r))
                },
            );

        let mut root = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .child(shape);

        if self.debug {
            let c = self.notch.container_frame();
            let r = self.hover_rect();
            let top = (c.origin.y + c.size.height) - (r.origin.y + r.size.height);
            root = root.child(
                div()
                    .absolute()
                    .left(px((r.origin.x - c.origin.x) as f32))
                    .top(px(top as f32))
                    .w(px(r.size.width as f32))
                    .h(px(r.size.height as f32))
                    .border_2()
                    .border_color(rgba(0xff0000ff)),
            );
        }
        root
    }
}

mod native {
    //! All AppKit/objc code lives here. Everything GPUI doesn't expose.
    use super::*;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
    use objc2::{msg_send, sel};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSView, NSWindowCollectionBehavior,
        NSWindowStyleMask,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    extern "C" fn no(_this: *mut AnyObject, _sel: Sel) -> Bool {
        Bool::NO
    }

    /// GPUI's private `GPUIPanel` class answers YES to canBecomeKey/Main.
    /// Patch the class at runtime (process-wide, affects every GPUI PopUp).
    pub fn patch_gpui_panel_class() {
        let Some(cls) = AnyClass::get(c"GPUIPanel") else {
            eprintln!("WARN: GPUIPanel class not found; cannot patch canBecomeKey");
            return;
        };
        unsafe {
            let imp: objc2::runtime::Imp = std::mem::transmute(
                no as extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
            let types = c"B@:";
            for s in [sel!(canBecomeKeyWindow), sel!(canBecomeMainWindow)] {
                objc2::ffi::class_replaceMethod(
                    cls as *const AnyClass as *mut AnyClass,
                    s,
                    imp,
                    types.as_ptr(),
                );
            }
        }
    }

    pub fn set_accessory(mtm: MainThreadMarker) {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }

    /// Reach the NSWindow behind a GPUI window through raw-window-handle
    /// (gpui hands out the GPUIView NSView; -[NSView window] gives the panel).
    pub fn ns_window(window: &Window) -> Retained<NSWindow> {
        let h = HasWindowHandle::window_handle(window).expect("window handle");
        let RawWindowHandle::AppKit(h) = h.as_raw() else {
            panic!("not AppKit")
        };
        let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        view.window().expect("view has no window")
    }

    pub fn configure(win: &NSWindow, frame: NSRect) {
        unsafe {
            // GPUI always creates Titled|FullSizeContentView windows (even with
            // titlebar: None). Titled windows get constrained below the menu
            // bar and carry chrome; switch to borderless + nonactivating panel.
            win.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
            win.setHasShadow(false);
            win.setOpaque(false);
            win.setMovable(false);
            win.setHidesOnDeactivate(false);
            // NSStatusWindowLevel (25) like the Swift version; GPUI PopUp used 101.
            win.setLevel(25);
            win.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            win.setFrame_display(frame, true);
            let _: () = msg_send![win, orderFrontRegardless];
        }
    }

    pub fn describe(win: &NSWindow) -> String {
        let f = win.frame();
        let key: bool = win.isKeyWindow();
        let main: bool = win.isMainWindow();
        let can_key: bool = win.canBecomeKeyWindow();
        format!(
            "windowNumber={} level={} frame=({}, {}, {}x{}) styleMask={:?} opaque={} shadow={} key={} main={} canBecomeKey={} behavior={:?}",
            win.windowNumber(),
            win.level(),
            f.origin.x,
            f.origin.y,
            f.size.width,
            f.size.height,
            win.styleMask(),
            win.isOpaque(),
            win.hasShadow(),
            key,
            main,
            can_key,
            win.collectionBehavior()
        )
    }
}

fn main() {
    let app = Application::new();
    native::patch_gpui_panel_class();

    app.run(|cx: &mut App| {
        let mtm = MainThreadMarker::new().expect("main thread");
        native::set_accessory(mtm);

        let Some(notch) = detect_notch(mtm) else {
            println!("No notched screen found (auxiliaryTopLeftArea/RightArea absent). Exiting.");
            cx.quit();
            return;
        };
        println!(
            "[{}] notch: width={} height={} centerX={} topY={}",
            stamp(),
            notch.width,
            notch.height,
            notch.center_x,
            notch.top_y
        );
        let debug = std::env::var("SPIKE_DEBUG").as_deref() == Ok("1");
        let tick_ms: u64 = std::env::var("SPIKE_TICK_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(250);

        // Initial bounds are approximate (GPUI uses display-relative,
        // top-left coords); the authoritative frame is set via AppKit below.
        let cw = notch.container_w() as f32;
        let bounds = Bounds::centered(None, size(px(cw), px(EXPANDED_H as f32)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: gpui::point(bounds.origin.x, px(0.)),
                size: bounds.size,
            })),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let ns_window = native::ns_window(window);
            native::configure(&ns_window, notch.container_frame());
            println!("[{}] window: {}", stamp(), native::describe(&ns_window));

            cx.new(|cx: &mut Context<Shell>| {
                let shell = Shell {
                    notch,
                    ns_window,
                    mode: Mode::Preview,
                    hover_rect_is_container: false,
                    was_inside: false,
                    pending_collapse: None,
                    enters: 0,
                    exits: 0,
                    transitions: 0,
                    anim_start: Instant::now() - ANIM,
                    from_h: notch.height as f32,
                    from_r: PREVIEW_RADIUS,
                    tick: 0,
                    debug,
                    ignoring_mouse: None,
                    t0: Instant::now(),
                };
                shell.log("start");

                // Hover/click-through poll loop.
                cx.spawn(async move |this, cx| loop {
                    cx.background_executor().timer(POLL).await;
                    if this.update(cx, |s, cx| s.poll(cx)).is_err() {
                        break;
                    }
                })
                .detach();

                // 250 ms redraw-pressure ticker (SPIKE_TICK_MS=0 disables).
                if tick_ms > 0 {
                    cx.spawn(async move |this, cx| loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(tick_ms))
                            .await;
                        if this
                            .update(cx, |s, cx| {
                                s.tick += 1;
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    })
                    .detach();
                }

                // Periodic window-state dump for evidence (every 5 s).
                cx.spawn(async move |this, cx| loop {
                    cx.background_executor().timer(Duration::from_secs(5)).await;
                    if this
                        .update(cx, |s, _| {
                            println!("[{}] window: {}", stamp(), native::describe(&s.ns_window))
                        })
                        .is_err()
                    {
                        break;
                    }
                })
                .detach();

                shell
            })
        })
        .expect("open_window");
    });
}

#[allow(dead_code)]
fn _unused(_: Pixels) {}
