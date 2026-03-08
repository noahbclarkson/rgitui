use gpui::*;
use rust_embed::RustEmbed;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "../../assets"]
#[include = "icons/**/*"]
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Self::get(path)
            .map(|f| Some(f.data))
            .ok_or_else(|| anyhow::anyhow!("asset not found: {path}"))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| {
                if p.starts_with(path) {
                    Some(SharedString::from(p.into_owned()))
                } else {
                    None
                }
            })
            .collect())
    }
}

fn main() {
    env_logger::init();

    let http_client = Arc::new(reqwest_client::ReqwestClient::new());
    let app = Application::with_platform(gpui_platform::current_platform(false))
        .with_http_client(http_client)
        .with_assets(Assets);

    app.run(move |cx| {
        // Initialize subsystems
        rgitui_theme::init(cx);
        rgitui_settings::init(cx);
        cx.set_global(rgitui_ui::AvatarCache::new());

        // Determine which repos to open
        let cli_path = std::env::args().nth(1).map(PathBuf::from);

        let repos_to_open: Vec<PathBuf> = if let Some(raw_path) = cli_path {
            // CLI arg given — open that specific repo
            let repo_path = std::fs::canonicalize(&raw_path).unwrap_or(raw_path.clone());
            let repo_path = match git2::Repository::discover(&repo_path) {
                Ok(repo) => repo
                    .workdir()
                    .unwrap_or_else(|| repo.path())
                    .to_path_buf(),
                Err(_) => repo_path,
            };
            vec![repo_path]
        } else {
            // No CLI arg — try to restore last workspace
            let last_workspace = cx
                .try_global::<rgitui_settings::SettingsState>()
                .map(|s| s.settings().last_workspace.clone())
                .unwrap_or_default();

            if last_workspace.is_empty() {
                // Fall back to current directory if it's a git repo, otherwise show welcome
                let raw_path =
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let repo_path = std::fs::canonicalize(&raw_path).unwrap_or(raw_path);
                match git2::Repository::discover(&repo_path) {
                    Ok(repo) => {
                        let workdir = repo
                            .workdir()
                            .unwrap_or_else(|| repo.path())
                            .to_path_buf();
                        vec![workdir]
                    }
                    Err(_) => {
                        // Not a git repo — show empty welcome screen
                        Vec::new()
                    }
                }
            } else {
                // Filter to repos that still exist on disk
                last_workspace
                    .into_iter()
                    .filter(|p| p.exists())
                    .collect()
            }
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
                for repo_path in repos_to_open {
                    if let Err(e) = ws.open_repo(repo_path, cx) {
                        log::error!("Failed to open repo: {}", e);
                    }
                }
                ws
            });

            workspace
        })
        .expect("Failed to open window");
    });
}
