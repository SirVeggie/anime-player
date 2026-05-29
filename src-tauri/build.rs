use std::env;
use std::path::{Path, PathBuf};

fn stage_runtime_file(src: &Path, file_name: &str) {
    if let Ok(out_dir) = env::var("OUT_DIR") {
        let mut binary_dir = PathBuf::from(out_dir);
        for _ in 0..3 {
            if !binary_dir.pop() {
                break;
            }
        }
        if binary_dir.exists() {
            let target = binary_dir.join(file_name);
            if let Err(err) = std::fs::copy(src, &target) {
                println!(
                    "cargo:warning=Failed to stage {} at {}: {}",
                    file_name,
                    target.display(),
                    err
                );
            }
        }
    }
}

fn main() {
    // libmpv linkage on Windows. The generated libmpv-2.dll + mpv.lib live in
    // src-tauri/libs/mpv/ (install with `npm run setup:mpv`). MSVC link.exe
    // consumes the MinGW-built .dll.a archive when renamed to mpv.lib, which
    // is what the downloader script does.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let mpv_dir = PathBuf::from(&manifest_dir).join("libs").join("mpv");
        let lib = mpv_dir.join("mpv.lib");
        let dll = mpv_dir.join("libmpv-2.dll");
        if !lib.exists() || !dll.exists() {
            panic!(
                "\n[!] Cannot find libmpv runtime under {}.\n    Run: npm run setup:mpv\n",
                mpv_dir.display()
            );
        }

        println!("cargo:rustc-link-search=native={}", mpv_dir.display());
        println!("cargo:rustc-link-lib=dylib=mpv");
        println!("cargo:rerun-if-changed=libs/mpv/mpv.lib");
        println!("cargo:rerun-if-changed=libs/mpv/libmpv-2.dll");
        println!("cargo:rerun-if-changed=icons/icon.ico");
        println!("cargo:rerun-if-changed=icons/icon.png");

        // Make libmpv-2.dll discoverable for `cargo run` / `tauri dev`. The
        // Rust binary lives in target/<profile>/, so we copy the DLL beside
        // it. For installer builds the bundle.resources entry handles it.
        stage_runtime_file(&dll, "libmpv-2.dll");

        let ffmpeg_dir = PathBuf::from(&manifest_dir).join("libs").join("ffmpeg");
        for name in ["ffmpeg.exe", "ffprobe.exe"] {
            let tool = ffmpeg_dir.join(name);
            if tool.exists() {
                println!("cargo:rerun-if-changed=libs/ffmpeg/{name}");
                stage_runtime_file(&tool, name);
            } else {
                println!(
                    "cargo:warning=Missing {} — run: npm run setup:ffmpeg",
                    tool.display()
                );
            }
        }
    }

    tauri_build::build()
}
