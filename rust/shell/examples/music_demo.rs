//! 音樂畫面示範（Task 24）：真正的 `ShellWindow` + `MusicShellContent`，配上離線的假資料。
//!
//! 不連網、不用任何私有 API：現正播放來自程式內的腳本來源（實作 `NowPlayingSource`），歌詞來自程式內的
//! 假解析器（約 20 行自寫的中英混合同步歌詞），封面是程式產生的 PNG（480 × 300、非正方形，用來確認置中裁切：
//! 左右兩側的灰色條紋不應出現在畫面上）。控制器、模組狀態、模組管理器、外殼視窗與 hover 機制都是真的。
//!
//! 用法：
//!
//! ```sh
//! cd rust && cargo run -p musicat-shell --example music_demo -- --scene playing
//! ```
//!
//! 場景（`--scene`，預設 `playing`）：
//! - `playing`：180 秒的曲目從 0 秒開始即時前進，有同步歌詞與封面。
//! - `paused`：同一首停在 42 秒。Preview 保留封面與靜止的音柱、沒有歌詞條；Expanded 直接是音樂頁。
//! - `none`：沒有曲目。
//! - `plain`：播放中、只有純文字歌詞（歌詞條只顯示「純文字版」、Expanded 是靜態全文）。
//! - `nolyrics`：播放中、沒有歌詞（歌詞條只顯示「沒有歌詞」、Expanded 標頭垂直置中）。
//! - `longtitle`：播放中、很長的曲名與演出者（跑馬燈），歌詞裡有很長的一行。
//!
//! 環境變數：
//! - `MUSICAT_DEMO_SECS`：幾秒後自動結束（預設 60）。
//! - `MUSICAT_DEMO_NO_CONTROL=1`：沒有播放控制（按鈕顯示停用樣式）。
//! - `MUSICAT_DEMO_STRIP=0`：歌詞條一開始是收起的。
//!
//! Expanded 控制列左側的歌詞來源標籤（有歌詞時顯示 `lrclib.net ↗`）點擊後只印出 `open <網址>`，不會開瀏覽器。
//! Expanded 的按鈕會作用在腳本來源上：播放／暫停會凍結或恢復位置，上一首／下一首在兩首曲目間切換。
//! 只在 macOS 有效；其他平台印一行訊息就結束。

#[cfg(target_os = "macos")]
fn main() {
    demo::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("music_demo: macOS only");
}

#[cfg(target_os = "macos")]
mod demo {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use gpui::{App, Application};
    use image::{ImageEncoder, Rgba, RgbaImage};
    use musicat_core::artwork_cache::ArtworkCache;
    use musicat_core::lyrics::repository::LyricsResolving;
    use musicat_core::lyrics::{LyricLine, Lyrics, LyricsHit, TrackIdentity};
    use musicat_core::modules::{ModuleManager, ModuleState};
    use musicat_core::music::controller::MusicController;
    use musicat_core::music::module::MusicModuleState;
    use musicat_core::nowplaying::{
        NowPlayingMonitor, NowPlayingSource, PlaybackCommand, PlaybackControl, SnapshotCallback,
    };
    use musicat_core::runtime::{BoxFuture, Clock, Notifier, Subscription};
    use musicat_core::shell_logic::{choose_screen, container_frame, geometry_for, VirtualNotch};
    use musicat_core::types::{ArtworkRef, PositionAnchor, TrackSnapshot};
    use musicat_platform_macos::panel;
    use musicat_platform_macos::screens::list_screens;
    use musicat_shell::{GpuiSpawner, MusicShellContent, ShellEvent, ShellWindow};

    fn unix_now() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }

    fn stamp(t0: Instant) -> String {
        format!("[+{:>6}ms]", t0.elapsed().as_millis())
    }

    // ------------------------------------------------------------------ 執行環境

    /// 牆上時鐘。`sleep` 只是為了滿足介面（示範中沒有人用），每次起一條睡眠執行緒。
    struct SystemClock;

    impl Clock for SystemClock {
        fn now(&self) -> f64 {
            unix_now()
        }
        fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
            let (tx, rx) = futures_channel::oneshot::channel::<()>();
            std::thread::spawn(move || {
                std::thread::sleep(d);
                let _ = tx.send(());
            });
            Box::pin(async move {
                let _ = rx.await;
            })
        }
    }

    // ------------------------------------------------------------------ 場景

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Scene {
        Playing,
        Paused,
        None,
        Plain,
        NoLyrics,
        LongTitle,
    }

    impl Scene {
        fn parse(args: &[String]) -> Result<Scene, String> {
            let mut scene = Scene::Playing;
            let mut it = args.iter();
            while let Some(a) = it.next() {
                let value = match a.as_str() {
                    "--scene" => it.next().cloned().unwrap_or_default(),
                    s if s.starts_with("--scene=") => s["--scene=".len()..].to_string(),
                    other => return Err(format!("unknown argument {other:?}")),
                };
                scene = match value.as_str() {
                    "playing" => Scene::Playing,
                    "paused" => Scene::Paused,
                    "none" => Scene::None,
                    "plain" => Scene::Plain,
                    "nolyrics" => Scene::NoLyrics,
                    "longtitle" => Scene::LongTitle,
                    other => return Err(format!("unknown scene {other:?}")),
                };
            }
            Ok(scene)
        }
    }

    struct Track {
        title: &'static str,
        artist: &'static str,
        duration: f64,
        cover: ArtworkRef,
    }

    /// 480 × 300 的 PNG：中央 300 × 300 是 `color` 加一個白色圓，左右各 90 像素是灰色條紋（應被裁掉）。
    fn cover_png(color: [u8; 3]) -> Vec<u8> {
        let (w, h) = (480u32, 300u32);
        let img = RgbaImage::from_fn(w, h, |x, y| {
            if !(90..390).contains(&x) {
                let v = if (x / 6) % 2 == 0 { 90 } else { 160 };
                return Rgba([v, v, v, 255]);
            }
            let (dx, dy) = (x as f64 - 240.0, y as f64 - 150.0);
            let d = (dx * dx + dy * dy).sqrt();
            if (60.0..78.0).contains(&d) {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([color[0], color[1], color[2], 255])
            }
        });
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .expect("encoding a PNG into memory cannot fail");
        out
    }

    fn tracks(scene: Scene) -> Vec<Track> {
        let cover = |id: &str, color| ArtworkRef {
            id: id.into(),
            data: Some(Arc::from(cover_png(color))),
        };
        let (title, artist) = if scene == Scene::LongTitle {
            (
                "這是一個非常非常長的曲名，用來測試跑馬燈會不會捲動 — An Extremely Long Track Title",
                "MusiCat 示範樂團 feat. A Very Long List Of Guest Artists And Friends",
            )
        } else {
            ("夜航 Night Ferry", "MusiCat 示範樂團")
        };
        vec![
            Track {
                title,
                artist,
                duration: 180.0,
                cover: cover("demo-cover-1", [232, 93, 74]),
            },
            Track {
                title: "晨光 Morning Light",
                artist: "MusiCat 示範樂團",
                duration: 150.0,
                cover: cover("demo-cover-2", [52, 120, 246]),
            },
        ]
    }

    /// 自寫的同步歌詞（中英混合），4 秒開始每 8 秒一行，共 20 行（最後一行 156 秒 < 180）。
    fn synced_lyrics(long_line: bool) -> Lyrics {
        let texts = [
            "港口的燈一盞一盞亮起",
            "The ferry hums a quiet tune",
            "我把車票摺成一隻紙船",
            "Drifting slow beneath the moon",
            "海風把昨天吹得好遠",
            "Every wave a softer word",
            "你說夜航不需要地圖",
            "Only stars and what we heard",
            "甲板上的影子慢慢變長",
            "Lanterns sway from left to right",
            "如果天亮以前到不了岸",
            "We can sing into the night",
            "霧裡的燈塔眨了眨眼",
            "Telling us we're not alone",
            "把心事交給起伏的浪",
            "Every tide will bring us home",
            "遠方傳來第一聲汽笛",
            "Morning paints the water gold",
            "夜航結束，故事還在",
            "Keep the ticket, keep it close",
        ];
        Lyrics::Synced(
            texts
                .iter()
                .enumerate()
                .map(|(i, t)| LyricLine {
                    time: 4.0 + 8.0 * i as f64,
                    text: if long_line && i == 1 {
                        "這一行歌詞特別特別長，長到歌詞條放不下，所以應該要以每秒三十點的速度往左捲動 — and it keeps going on and on"
                            .to_string()
                    } else {
                        (*t).to_string()
                    },
                })
                .collect(),
        )
    }

    fn plain_lyrics() -> Lyrics {
        Lyrics::Plain(
            "港口的燈一盞一盞亮起\nThe ferry hums a quiet tune\n\n我把車票摺成一隻紙船\nDrifting slow beneath the moon\n海風把昨天吹得好遠\nEvery wave a softer word\n你說夜航不需要地圖\nOnly stars and what we heard"
                .to_string(),
        )
    }

    // ------------------------------------------------------------------ 假來源與控制

    struct Playback {
        index: usize,
        has_track: bool,
        playing: bool,
        elapsed: f64,
        timestamp: f64,
    }

    /// 腳本來源：目前曲目與播放錨點；`publish` 送出快照（訂閱時重播目前值）。
    struct DemoSource {
        tracks: Vec<Track>,
        state: Mutex<Playback>,
        notifier: Notifier<Option<TrackSnapshot>>,
    }

    impl DemoSource {
        fn snapshot(&self) -> Option<TrackSnapshot> {
            let p = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if !p.has_track {
                return None;
            }
            let t = &self.tracks[p.index];
            Some(TrackSnapshot {
                title: t.title.to_string(),
                artist: Some(t.artist.to_string()),
                album: None,
                duration: Some(t.duration),
                is_playing: p.playing,
                position: Some(PositionAnchor {
                    elapsed: p.elapsed,
                    timestamp: p.timestamp,
                    rate: if p.playing { 1.0 } else { 0.0 },
                }),
                artwork: Some(t.cover.clone()),
                source_id: "demo".to_string(),
            })
        }

        fn publish(&self) {
            self.notifier.notify(&self.snapshot());
        }

        fn apply(&self, cmd: PlaybackCommand) {
            {
                let mut p = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                let now = unix_now();
                let position = if p.playing {
                    p.elapsed + (now - p.timestamp).max(0.0)
                } else {
                    p.elapsed
                };
                let play = match cmd {
                    PlaybackCommand::Play => true,
                    PlaybackCommand::Pause => false,
                    PlaybackCommand::TogglePlayPause => !p.playing,
                    PlaybackCommand::Next | PlaybackCommand::Previous => {
                        let n = self.tracks.len();
                        p.index = if cmd == PlaybackCommand::Next {
                            (p.index + 1) % n
                        } else {
                            (p.index + n - 1) % n
                        };
                        p.elapsed = 0.0;
                        p.timestamp = now;
                        p.playing
                    }
                };
                if !matches!(cmd, PlaybackCommand::Next | PlaybackCommand::Previous) {
                    p.elapsed = position;
                    p.timestamp = now;
                }
                p.playing = play;
            }
            self.publish();
        }
    }

    impl NowPlayingSource for DemoSource {
        fn id(&self) -> &str {
            "demo"
        }
        fn start(&self) {}
        fn stop(&self) {}
        fn subscribe(&self, f: SnapshotCallback) -> Subscription {
            let f: SharedCallback = Arc::from(f);
            let f2 = f.clone();
            let sub = self.notifier.subscribe(Box::new(move |v| f2(v)));
            f(&self.snapshot());
            sub
        }
    }

    type SharedCallback = Arc<dyn Fn(&Option<TrackSnapshot>) + Send + Sync>;

    struct DemoControl {
        source: Arc<DemoSource>,
        t0: Instant,
    }

    impl PlaybackControl for DemoControl {
        fn send(&self, cmd: PlaybackCommand) {
            println!("{} control {cmd:?}", stamp(self.t0));
            self.source.apply(cmd);
        }
    }

    /// 假歌詞解析器：依場景與曲名回答，立即完成。
    struct DemoLyrics {
        scene: Scene,
    }

    impl LyricsResolving for DemoLyrics {
        fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Option<LyricsHit>> {
            let lyrics = match self.scene {
                Scene::Plain => Some(plain_lyrics()),
                Scene::NoLyrics => None,
                // 第二首（晨光）沒有歌詞：切歌時歌詞條改顯示「沒有歌詞」。
                _ if track.title.contains("Morning") || track.title.contains("晨光") => None,
                scene => Some(synced_lyrics(scene == Scene::LongTitle)),
            };
            let hit = lyrics.map(|l| LyricsHit::new(l, "https://lrclib.net/api/get/1"));
            Box::pin(std::future::ready(hit))
        }
    }

    // ------------------------------------------------------------------ 主程式

    pub fn run() {
        let t0 = Instant::now();
        let args: Vec<String> = std::env::args().skip(1).collect();
        let scene = match Scene::parse(&args) {
            Ok(scene) => scene,
            Err(e) => {
                eprintln!(
                    "music_demo: {e}; usage: --scene playing|paused|none|plain|nolyrics|longtitle"
                );
                std::process::exit(2);
            }
        };
        let secs = std::env::var("MUSICAT_DEMO_SECS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(60.0)
            .max(0.0);
        let no_control = std::env::var("MUSICAT_DEMO_NO_CONTROL").as_deref() == Ok("1");
        let strip_initially = std::env::var("MUSICAT_DEMO_STRIP").as_deref() != Ok("0");
        println!(
            "music_demo scene={scene:?} secs={secs} control={}",
            !no_control
        );
        println!("hover the top-centre of the built-in screen");

        let app = Application::new();
        panel::patch_gpui_panel_class();

        app.run(move |cx: &mut App| {
            panel::set_accessory_activation_policy();

            let screens = list_screens();
            let Some(screen) = choose_screen(&screens, None) else {
                println!("{} no screens; quitting", stamp(t0));
                cx.quit();
                return;
            };
            let geometry = geometry_for(screen, VirtualNotch::default());
            println!(
                "{} screen={:?} hardware_notch={:?} container={:?} pid={}",
                stamp(t0),
                screen.name,
                screen.hardware_notch,
                container_frame(&geometry),
                std::process::id()
            );

            let now = unix_now();
            let source = Arc::new(DemoSource {
                tracks: tracks(scene),
                state: Mutex::new(Playback {
                    index: 0,
                    has_track: scene != Scene::None,
                    playing: scene != Scene::Paused,
                    elapsed: if scene == Scene::Paused { 42.0 } else { 0.0 },
                    timestamp: now,
                }),
                notifier: Notifier::new(),
            });
            let control: Option<Arc<dyn PlaybackControl>> = if no_control {
                None
            } else {
                Some(Arc::new(DemoControl {
                    source: source.clone(),
                    t0,
                }))
            };
            let monitor = Arc::new(NowPlayingMonitor::new(
                vec![source.clone() as Arc<dyn NowPlayingSource>],
                ArtworkCache::default(),
            ));
            let controller = MusicController::new(
                monitor.clone(),
                Arc::new(DemoLyrics { scene }),
                control,
                Arc::new(GpuiSpawner::new(cx.background_executor().clone())),
            );
            let module_state = Arc::new(MusicModuleState::new(
                controller.clone(),
                Arc::new(AtomicBool::new(strip_initially)),
                vec![source.clone() as Arc<dyn NowPlayingSource>],
            ));
            let modules = Arc::new(ModuleManager::new());
            modules.register(module_state.clone() as Arc<dyn ModuleState>);

            let content = MusicShellContent::new(
                cx,
                geometry,
                controller.clone(),
                module_state.clone(),
                modules.clone(),
                Arc::new(SystemClock),
                // 示範不開瀏覽器：只印出要開的網址，用來確認點擊來源標籤。
                Arc::new(move |url: &str| println!("{} open {url}", stamp(t0))),
            )
            .with_event_hook(move |event| match event {
                ShellEvent::Expanded => println!("{} expand", stamp(t0)),
                ShellEvent::Collapsed => println!("{} collapse", stamp(t0)),
                ShellEvent::Visible(v) => println!("{} visible={v}", stamp(t0)),
            });
            let content = content.into_shell_content();
            let shell = match ShellWindow::open(cx, geometry, content) {
                Ok(shell) => shell,
                Err(e) => {
                    println!("{} open failed: {e:?}; quitting", stamp(t0));
                    cx.quit();
                    return;
                }
            };

            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs_f64(secs))
                    .await;
                println!("{} {secs}s elapsed; quitting", stamp(t0));
                drop(shell);
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                // 監視器要活到最後（控制器只對它持弱參照）。
                drop((monitor, controller, module_state, modules));
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}
