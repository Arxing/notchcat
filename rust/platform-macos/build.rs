//! 把 `helper/helper.c` 編成 `libmusicat_mediaremote_helper.dylib`。
//!
//! 動態庫只會被 Apple 簽章的 `/usr/bin/perl` 以絕對路徑經 DynaLoader 載入，
//! 所以不需要 rpath；產物先放 `OUT_DIR`，再複製到 Cargo 的 profile 目錄
//! （`target/<profile>/`，與最終可執行檔同目錄）。
//! arm64 的連結器輸出自帶 ad-hoc 簽章（與 Swift 版相同），這裡不另做 codesign。
//!
//! 非 macOS 目標：什麼都不做（不需要 C 編譯器）。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DYLIB_NAME: &str = "libmusicat_mediaremote_helper.dylib";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=helper/");

    // build.rs 本身以主機平台編譯，必須看 CARGO_CFG_TARGET_OS 而非 cfg!。
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let built = out_dir.join(DYLIB_NAME);

    // cc 負責選編譯器並依 TARGET 加上 -arch / 部署版本旗標（主機 arch 或 --target 指定的 arch）。
    // 注意：get_compiler() 不會帶入 .file()，來源檔要自己加在命令列上。
    let mut cmd: Command = cc::Build::new()
        .opt_level(2)
        .warnings(true)
        .get_compiler()
        .to_command();
    // 部署目標與 Swift 版 Package.swift 相同（macOS 14）；cc 預設會用本機 SDK 版本，
    // 在較新的機器上編出的庫就無法在較舊的 macOS 載入。環境變數有設時尊重它。
    if env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() {
        cmd.arg("-mmacosx-version-min=14.0");
    }
    // 不嵌入建置目錄路徑：用固定的 @rpath 名稱（perl 以絕對路徑載入，與此無關）。
    cmd.arg("-I")
        .arg("helper")
        .arg("helper/helper.c")
        .arg("-dynamiclib")
        .arg("-install_name")
        .arg(format!("@rpath/{DYLIB_NAME}"))
        .arg("-framework")
        .arg("CoreFoundation")
        .arg("-o")
        .arg(&built);
    // 不要在訊息中印 `{cmd:?}`：Command 的 Debug 輸出會連同整份環境變數一起印出。
    let program = cmd.get_program().to_owned();
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to run {program:?}: {e}"));
    assert!(
        status.success(),
        "compiling helper/helper.c with {program:?} failed ({status})"
    );

    let profile_dir = profile_dir(&out_dir);
    let dest = profile_dir.join(DYLIB_NAME);
    // 先刪再複製：避免覆寫一個已被簽章／載入的檔案時沿用舊的 inode 與簽章快取。
    let _ = fs::remove_file(&dest);
    fs::copy(&built, &dest).unwrap_or_else(|e| panic!("copy {built:?} -> {dest:?}: {e}"));
}

/// `OUT_DIR` 形如 `<target>[/<triple>]/<profile>/build/<pkg>-<hash>/out`；
/// 回傳含有 `build` 的那一層的上一層，即 `<target>[/<triple>]/<profile>`。
fn profile_dir(out_dir: &Path) -> PathBuf {
    out_dir
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build"))
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("cannot derive the profile directory from OUT_DIR {out_dir:?}"))
        .to_path_buf()
}
