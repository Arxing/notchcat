//! 驗證 build.rs 產出的 MediaRemote helper 動態庫（僅 macOS）。
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Command;

use musicat_platform_macos::{DYLIB_NAME, HELPER_SYMBOL};

/// 測試執行檔位於 `target/<profile>/deps/`，動態庫在其上一層。
/// 以 current_exe 推導，與 CARGO_TARGET_DIR、--target 是否指定無關。
fn dylib_path() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let deps = exe.parent().expect("exe dir");
    assert_eq!(
        deps.file_name().and_then(|n| n.to_str()),
        Some("deps"),
        "unexpected exe location: {exe:?}"
    );
    deps.parent().expect("profile dir").join(DYLIB_NAME)
}

fn tool_available(name: &str, arg: &str) -> bool {
    // 只在執行檔不存在時才算「缺少」；以非零結束碼結束仍表示存在。
    !matches!(Command::new(name).arg(arg).output(), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

#[test]
fn dylib_exists_next_to_the_profile_directory() {
    let path = dylib_path();
    let meta = std::fs::metadata(&path).unwrap_or_else(|e| panic!("{path:?} missing: {e}"));
    assert!(meta.is_file() && meta.len() > 0, "{path:?} is empty");
}

#[test]
fn dylib_exports_the_helper_entry_symbol() {
    if !tool_available("nm", "--version") {
        eprintln!("SKIP: `nm` not found on PATH");
        return;
    }
    let out = Command::new("nm")
        .arg("-gU")
        .arg(dylib_path())
        .output()
        .expect("run nm");
    assert!(
        out.status.success(),
        "nm failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let expected = format!("_{HELPER_SYMBOL}");
    assert!(
        text.lines()
            .any(|l| l.split_whitespace().last() == Some(expected.as_str())),
        "{expected} not exported; nm output:\n{text}"
    );
    assert!(
        !text.contains("notchcat_helper_main"),
        "old symbol still exported:\n{text}"
    );
}

#[test]
fn dylib_is_ad_hoc_signed() {
    if !tool_available("codesign", "--version") {
        eprintln!("SKIP: `codesign` not found on PATH");
        return;
    }
    let out = Command::new("codesign")
        .arg("-dv")
        .arg(dylib_path())
        .output()
        .expect("run codesign");
    // codesign -dv 把資訊寫到 stderr。
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "codesign -dv failed:\n{text}");
    assert!(
        text.to_lowercase().contains("adhoc"),
        "not ad-hoc signed:\n{text}"
    );
}

#[test]
fn constants_match_the_expected_names() {
    assert_eq!(DYLIB_NAME, "libmusicat_mediaremote_helper.dylib");
    assert_eq!(HELPER_SYMBOL, "musicat_helper_main");
}
