use std::env;
use std::path::PathBuf;

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
        println!("cargo:rerun-if-changed=icons/app-icon-source.svg");

        // Make libmpv-2.dll discoverable for `cargo run` / `tauri dev`. The
        // Rust binary lives in target/<profile>/, so we copy the DLL beside
        // it. For installer builds the bundle.resources entry handles it.
        if let Ok(out_dir) = env::var("OUT_DIR") {
            // OUT_DIR looks like target/<profile>/build/<crate>-<hash>/out;
            // the binary's directory is target/<profile>/.
            let mut binary_dir = PathBuf::from(out_dir);
            for _ in 0..3 {
                if !binary_dir.pop() {
                    break;
                }
            }
            if binary_dir.exists() {
                let target = binary_dir.join("libmpv-2.dll");
                if let Err(err) = std::fs::copy(&dll, &target) {
                    println!(
                        "cargo:warning=Failed to stage libmpv-2.dll at {}: {}",
                        target.display(),
                        err
                    );
                }
            }
        }
    }

    tauri_build::build()
}
