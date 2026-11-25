//! R1 示範：用真實網路與真實時鐘，經 `LyricsRepository`（LRCLIB 然後 NetEase）查歌詞。
//!
//! 用法：`cargo run -p musicat-app --example lyrics_demo -- [TITLE ARTIST [DURATION_SECS]]`
//! 不帶參數時跑內建的三首公開歌曲。除了提供者本來就會送的歌名／演出者／專輯／曲長查詢外，不送任何資料。

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_executor::block_on;
use musicat_app::http_ureq::UreqHttp;
use musicat_app::real_runtime::RealClock;
use musicat_core::lyrics::lrclib::LrclibProvider;
use musicat_core::lyrics::netease::NetEaseProvider;
use musicat_core::lyrics::provider::LyricsProvider;
use musicat_core::lyrics::repository::{LyricsRepository, LyricsResolving, RepositoryConfig};
use musicat_core::lyrics::{identity_from, Lyrics};
use musicat_core::music::presentation::{strip_content, StripContent};
use musicat_core::runtime::Clock;
use musicat_core::{PositionAnchor, TrackSnapshot};

fn snapshot(title: &str, artist: &str, duration: Option<f64>) -> TrackSnapshot {
    TrackSnapshot {
        title: title.to_string(),
        artist: Some(artist.to_string()),
        album: None,
        duration,
        is_playing: true,
        position: None,
        artwork: None,
        source_id: "demo".to_string(),
    }
}

fn mmss(t: f64) -> String {
    let total = t.max(0.0);
    format!("{:02}:{:05.2}", (total / 60.0) as u64, total % 60.0)
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn describe(lyrics: &Option<Lyrics>) -> String {
    match lyrics {
        Some(Lyrics::Synced(lines)) => {
            let mut out = format!("Synced, {} lines\n", lines.len());
            for l in lines.iter().take(3) {
                out.push_str(&format!("    [{}] {}\n", mmss(l.time), l.text));
            }
            if let Some(last) = lines.last() {
                out.push_str(&format!("    last line at {}", mmss(last.time)));
            }
            out
        }
        Some(Lyrics::Plain(text)) => {
            let head: String = text.chars().take(80).collect();
            format!(
                "Plain (fallback only, never shown live): {}",
                head.replace('\n', " / ")
            )
        }
        None => "none (not found, or offline / provider failed)".to_string(),
    }
}

fn run_song(
    repo: &LyricsRepository,
    clock: &RealClock,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) {
    println!(
        "=== {title} / {artist}{}",
        duration.map_or(String::new(), |d| format!(" ({d:.0} s)"))
    );
    let snap = snapshot(title, artist, duration);
    let Some(identity) = identity_from(&snap) else {
        println!("  no usable identity (empty title or artist)\n");
        return;
    };
    println!(
        "  identity: title={:?} artist={:?} album={:?} duration={:?}",
        identity.title, identity.artist, identity.album, identity.duration
    );

    let start = Instant::now();
    let hit = block_on(repo.lyrics(identity.clone()));
    println!("  1st lookup: {}", ms(start.elapsed()));
    let lyrics = hit.as_ref().map(|h| h.lyrics.clone());
    println!("  result: {}", describe(&lyrics));
    if let Some(h) = &hit {
        println!("  url: {}", h.url);
    }
    if let Some(l) = &lyrics {
        println!("  is_plausible({duration:?}): {}", l.is_plausible(duration));
    }

    let start = Instant::now();
    let again = block_on(repo.lyrics(identity));
    println!(
        "  2nd lookup (cache): {}  same result: {}",
        ms(start.elapsed()),
        again == hit
    );

    // 與控制器一致：不可信的歌詞不進歌詞條。
    let shown = lyrics.filter(|l| l.is_plausible(duration));
    for t in [5.0, 30.0, 60.0] {
        let now = clock.now();
        let mut playing = snap.clone();
        playing.position = Some(PositionAnchor {
            elapsed: t,
            timestamp: now,
            rate: 1.0,
        });
        let strip = match strip_content(Some(&playing), shown.as_ref(), now) {
            StripContent::Line(l) => format!("line   {l:?}"),
            StripContent::Title(s) => format!("title  {s:?} (no current lyric line)"),
            StripContent::None => "none".to_string(),
        };
        println!("  strip at {t:>3.0} s: {strip}");
    }
    println!();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let songs: Vec<(String, String, Option<f64>)> = match args.as_slice() {
        [] => vec![
            (
                "蘇裔非 - 抱我!  ft. 莊岱諺".into(),
                "蘇裔非".into(),
                Some(170.0),
            ),
            ("Bohemian Rhapsody".into(), "Queen".into(), Some(354.0)),
            ("晴天".into(), "周杰倫".into(), Some(269.0)),
        ],
        [title, artist] => vec![(title.clone(), artist.clone(), None)],
        [title, artist, secs] => match secs.parse::<f64>() {
            Ok(d) => vec![(title.clone(), artist.clone(), Some(d))],
            Err(_) => {
                eprintln!("DURATION_SECS must be a number, got {secs:?}");
                std::process::exit(2);
            }
        },
        _ => {
            eprintln!("usage: lyrics_demo [TITLE ARTIST [DURATION_SECS]]");
            std::process::exit(2);
        }
    };

    let clock = Arc::new(RealClock);
    let http = Arc::new(UreqHttp::new());
    let providers: Vec<Arc<dyn LyricsProvider>> = vec![
        Arc::new(LrclibProvider::new(http.clone())),
        Arc::new(NetEaseProvider::new(http)),
    ];
    let repo = LyricsRepository::new(providers, clock.clone(), RepositoryConfig::default());

    for (title, artist, duration) in &songs {
        run_song(&repo, &clock, title, artist, *duration);
    }
}
