fn main() {
    raise_windows_main_thread_stack();

    #[cfg(target_os = "windows")]
    {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let icon_path = std::path::PathBuf::from(&manifest_dir)
            .join("..")
            .join("..")
            .join("assets")
            .join("icons")
            .join("app-icon.ico");
        let icon_str = icon_path.to_string_lossy().to_string();
        if icon_path.exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(&icon_str);
            res.set("ProductName", "rgitui");
            res.set("FileDescription", "GPU-accelerated Git client");
            res.set("LegalCopyright", "Copyright 2026 rgitui contributors");
            if let Err(e) = res.compile() {
                eprintln!("cargo:warning=Failed to compile Windows resources: {}", e);
            }
        } else {
            println!("cargo:warning=No app icon found at {icon_str}, skipping Windows resource embedding");
        }
    }
}

/// Reserves an 8 MB main-thread stack in Windows MSVC builds.
///
/// Windows gives a program's main thread 1 MB of stack, where Linux and macOS
/// give 8 MB. An unoptimised build spends far more stack per call, and drawing
/// the workspace overflows 1 MB shortly after a repository loads. Zed raises its
/// own binary to the same 8 MB for the same reason. The linker only reserves the
/// address space and pages are committed as the stack grows, so an optimised
/// build pays nothing for it.
///
/// A build script runs on the build host, so the target is read from Cargo's
/// environment: `cfg!` would describe the host instead.
fn raise_windows_main_thread_stack() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-arg-bins=/STACK:{}", 8 * 1024 * 1024);
    }
}
