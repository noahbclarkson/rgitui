fn main() {
    #[cfg(target_os = "windows")]
    {
        let icon_path = "../../assets/icons/app-icon.ico";
        if std::path::Path::new(icon_path).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon_path);
            res.set("ProductName", "rgitui");
            res.set("FileDescription", "GPU-accelerated Git client");
            res.set("LegalCopyright", "Copyright 2026 rgitui contributors");
            if let Err(e) = res.compile() {
                eprintln!("cargo:warning=Failed to compile Windows resources: {}", e);
            }
        } else {
            println!("cargo:warning=No app icon found at {icon_path}, skipping Windows resource embedding");
        }
    }
}
