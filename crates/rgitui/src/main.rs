use gpui::*;
use std::path::PathBuf;

fn main() {
    env_logger::init();

    let app = Application::with_platform(gpui_platform::current_platform(false));

    app.run(move |cx| {
        // Initialize subsystems
        rgitui_theme::init(cx);
        rgitui_settings::init(cx);

        // Determine which repo to open — resolve to actual git root
        let raw_path = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        // Canonicalize relative paths
        let repo_path = std::fs::canonicalize(&raw_path).unwrap_or(raw_path.clone());

        // Try to discover git repo (walks up to find .git)
        let repo_path = match git2::Repository::discover(&repo_path) {
            Ok(repo) => repo
                .workdir()
                .unwrap_or_else(|| repo.path())
                .to_path_buf(),
            Err(_) => repo_path,
        };

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("rgitui".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            window_min_size: Some(Size {
                width: px(800.0),
                height: px(600.0),
            }),
            focus: true,
            show: true,
            app_id: Some("rgitui".to_string()),
            ..Default::default()
        };

        cx.open_window(options, |_window, cx| {
            let workspace = cx.new(|cx| {
                let mut ws = rgitui_workspace::Workspace::new(cx);
                if let Err(e) = ws.open_repo(repo_path, cx) {
                    log::error!("Failed to open repo: {}", e);
                }
                ws
            });

            workspace
        })
        .expect("Failed to open window");
    });
}
