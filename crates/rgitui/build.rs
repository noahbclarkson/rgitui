fn main() {
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
