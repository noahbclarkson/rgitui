#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Allocation-site heap profiling, enabled by `--features perf-dhat`. It has to
// be declared in the binary because a global allocator cannot come from a
// library. Runs built this way are around 100x slower, so their timings
// describe dhat rather than rgitui; the report says as much rather than
// leaving it to memory.
#[cfg(feature = "perf-dhat")]
#[global_allocator]
static ALLOC: rgitui_perf::heap_profile::Alloc = rgitui_perf::heap_profile::Alloc;

use gpui::*;
use rust_embed::RustEmbed;
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// AppRoot – transitions from splash screen to workspace
// ---------------------------------------------------------------------------

struct WorkspaceInit {
    repos_to_open: Vec<PathBuf>,
    startup_workspace: Option<rgitui_settings::StoredWorkspace>,
    has_cli_path: bool,
    was_clean_exit: bool,
}

struct AppRoot {
    splash: Option<Entity<rgitui_workspace::SplashScreen>>,
    workspace: Option<Entity<rgitui_workspace::Workspace>>,
    transitioning: bool,
    needs_maximize: bool,
    focus: FocusHandle,
}

impl AppRoot {
    fn init_workspace(
        init: WorkspaceInit,
        cx: &mut Context<Self>,
    ) -> Entity<rgitui_workspace::Workspace> {
        let workspace = cx.new(|cx| {
            let mut ws = rgitui_workspace::Workspace::new(cx);
            ws.set_crash_recovery_available(
                !init.was_clean_exit && init.startup_workspace.is_some(),
            );

            if !init.has_cli_path {
                if let Some(snapshot) = init.startup_workspace {
                    if let Err(error) = ws.restore_workspace_snapshot(snapshot, cx) {
                        log::error!("Failed to restore saved workspace: {}", error);
                    }
                } else {
                    for repo_path in init.repos_to_open {
                        if let Err(error) = ws.open_repo(repo_path, cx) {
                            log::error!("Failed to open repo: {}", error);
                        }
                    }
                    ws.refresh_all_tabs_prioritized(cx);
                }
            } else {
                for repo_path in init.repos_to_open {
                    if let Err(error) = ws.open_repo(repo_path, cx) {
                        log::error!("Failed to open repo: {}", error);
                    }
                }
                ws.refresh_all_tabs_prioritized(cx);
            }
            ws
        });

        workspace.update(cx, |ws, cx| {
            ws.start_background_tasks(cx);
        });

        workspace
    }

    fn show_workspace(&mut self, cx: &mut Context<Self>) {
        if self.transitioning {
            return;
        }
        self.transitioning = true;

        if let Some(workspace) = &self.workspace {
            workspace.update(cx, |ws, cx| {
                ws.show_crash_recovery_toast(cx);
                // Startup keymap problems are reported here rather than during
                // `keymap::init`, which runs before any window exists.
                ws.show_keymap_problems(cx);
            });
        }

        self.splash = None;
        self.needs_maximize = true;
        cx.notify();
    }

    fn skip_splash(&mut self, cx: &mut Context<Self>) {
        self.show_workspace(cx);
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Each branch returns a different concrete type due to GPUI's builder pattern.
        // Type-erase with into_any_element() to avoid a massive enum that overflows
        // the stack frame.
        if let Some(splash) = &self.splash {
            if !self.focus.is_focused(window) {
                self.focus.focus(window, cx);
            }
            div()
                .id("app-root-splash")
                .size_full()
                .track_focus(&self.focus)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event, _window, cx| {
                        this.skip_splash(cx);
                    }),
                )
                .on_key_down(cx.listener(|this, _event: &KeyDownEvent, _window, cx| {
                    this.skip_splash(cx);
                }))
                .child(splash.clone())
                .into_any_element()
        } else if let Some(workspace) = &self.workspace {
            if self.needs_maximize {
                self.needs_maximize = false;
                window.set_window_title("rgitui");
                window.zoom_window();
            }
            div()
                .id("app-root-workspace")
                .size_full()
                .child(workspace.clone())
                .into_any_element()
        } else {
            div().id("app-root-empty").size_full().into_any_element()
        }
    }
}

#[derive(RustEmbed)]
#[folder = "../../assets"]
#[include = "icons/**/*"]
#[include = "fonts/**/*"]
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

fn load_embedded_fonts(cx: &App) {
    let asset_source = cx.asset_source();
    let font_paths = match asset_source.list("fonts") {
        Ok(paths) => paths,
        Err(e) => {
            log::error!("failed to list embedded fonts directory: {e}");
            return;
        }
    };

    if font_paths.is_empty() {
        log::warn!("no embedded fonts found — text may not render on some platforms");
        return;
    }

    log::info!("loading {} embedded font files", font_paths.len());
    let mut fonts = Vec::new();
    for font_path in &font_paths {
        if !font_path.ends_with(".ttf") {
            continue;
        }
        match asset_source.load(font_path) {
            Ok(Some(font_bytes)) => {
                log::debug!("loaded font: {font_path} ({} bytes)", font_bytes.len());
                fonts.push(font_bytes);
            }
            Ok(None) => {
                log::warn!("font path listed but load returned None: {font_path}");
            }
            Err(e) => {
                log::error!("failed to load font {}: {e}", font_path);
            }
        }
    }

    if fonts.is_empty() {
        log::error!("no embedded fonts could be loaded — text will not render");
        return;
    }

    if let Err(e) = cx.text_system().add_fonts(fonts) {
        log::error!("text_system.add_fonts failed: {e}");
    } else {
        log::info!("embedded fonts registered successfully");
    }
}

/// Shortest time the splash stays up: the whole opening animation.
///
/// Taken from the splash rather than chosen here, so the animation is never cut
/// off partway by a number that drifted away from it.
const SPLASH_MIN: std::time::Duration = rgitui_workspace::SplashScreen::ANIMATION;

/// Longest the splash will wait for a repository to become ready.
///
/// Only reached when loading outlasts the animation. Past this an empty
/// workspace that is visibly filling in beats a splash that has finished
/// animating and is now just a delay.
const SPLASH_MAX: std::time::Duration = std::time::Duration::from_millis(2500);

/// How often readiness is checked while the splash is up.
const SPLASH_POLL: std::time::Duration = std::time::Duration::from_millis(25);

/// Baselines the clock startup measurements are taken against.
#[cfg(feature = "perf")]
fn start_perf_clock() {
    rgitui_perf::process_start();
}

/// A build without the harness has no clock to start.
#[cfg(not(feature = "perf"))]
fn start_perf_clock() {}

/// Keeps an instrumented run's settings and caches out of the user's
/// directories, so a measured run neither inherits state from the last one nor
/// leaves its corpus in somebody's recent-repository list.
#[cfg(feature = "perf")]
fn isolate_perf_state() {
    let root = match rgitui_perf::prepare_state_root() {
        Ok(Some(root)) => root,
        Ok(None) => return,
        // Refusing to touch a directory the harness does not own is the point;
        // continuing would measure against the user's real state, which is the
        // thing this exists to prevent.
        Err(error) => {
            eprintln!("perf run: {error}");
            std::process::exit(78);
        }
    };
    if rgitui_settings::redirect_state_to(&root) {
        log::info!(
            "perf run: settings and caches redirected to {}",
            root.display()
        );
    } else {
        log::warn!("perf run: state was already resolved, so this run shares the user's");
    }
}

/// A build without the harness has nothing to redirect.
#[cfg(not(feature = "perf"))]
fn isolate_perf_state() {}

fn main() {
    // First statement in the process: everything timed from here, so a startup
    // measurement includes the platform and window setup a user is also waiting for.
    start_perf_clock();

    env_logger::init();

    // Bound for the whole of `main` because dhat writes `dhat-heap.json` when
    // this drops, and anything allocated after that point goes unrecorded.
    #[cfg(feature = "perf-dhat")]
    let _heap_profiler = rgitui_perf::heap_profile::HeapProfiler::start();

    // Before any subsystem resolves a path.
    isolate_perf_state();

    std::panic::set_hook(Box::new(|panic_info| {
        log::error!("panic: {}", panic_info);
    }));

    let http_client = Arc::new(reqwest_client::ReqwestClient::new());
    let app = Application::with_platform(gpui_platform::current_platform(false))
        .with_http_client(http_client)
        .with_assets(Assets);

    app.run(move |cx| {
        log::info!("starting rgitui");
        load_embedded_fonts(cx);

        // Initialize subsystems
        rgitui_theme::init(cx);
        rgitui_settings::init(cx);
        // Applies the default keybindings plus the user's keymap.json, and
        // watches that file so saving it reloads the keymap. Must come after
        // settings init, which creates the config directory.
        rgitui_workspace::keymap::init(cx);

        // Initialize empty avatar cache immediately, load disk data in background
        cx.set_global(rgitui_ui::AvatarCache::new());
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let loaded = cx
                .background_executor()
                .spawn(async { rgitui_ui::AvatarCache::load_from_disk() })
                .await;
            cx.update(|cx| {
                cx.update_global::<rgitui_ui::AvatarCache, _>(|cache, _| {
                    for (email, url) in loaded {
                        cache.set_resolved(email, url);
                    }
                });
            });
        })
        .detach();

        // Apply saved theme from settings
        let saved_theme = cx
            .try_global::<rgitui_settings::SettingsState>()
            .map(|s| s.settings().theme.clone())
            .unwrap_or_default();
        if !saved_theme.is_empty() {
            rgitui_theme::set_theme(&saved_theme, cx);
        }

        // Determine which repos to open
        let cli_path = std::env::args().nth(1).map(PathBuf::from);
        let has_cli_path = cli_path.is_some();

        // Check if last session ended cleanly (crash recovery)
        let was_clean_exit =
            cx.update_global::<rgitui_settings::SettingsState, _>(|settings, _| {
                settings.mark_startup()
            });

        let startup_workspace = cx
            .try_global::<rgitui_settings::SettingsState>()
            .and_then(|settings| settings.active_workspace().cloned());

        let repos_to_open: Vec<PathBuf> = if let Some(raw_path) = cli_path.clone() {
            // CLI arg given — open that specific repo
            let repo_path = std::fs::canonicalize(&raw_path).unwrap_or(raw_path.clone());
            let repo_path = match git2::Repository::discover(&repo_path) {
                Ok(repo) => repo.workdir().unwrap_or_else(|| repo.path()).to_path_buf(),
                Err(_) => repo_path,
            };
            vec![repo_path]
        } else {
            // No CLI arg — try to restore the active saved workspace
            if let Some(workspace) = startup_workspace.as_ref() {
                workspace
                    .repos
                    .iter()
                    .filter(|path| path.exists())
                    .cloned()
                    .collect()
            } else {
                // Fall back to current directory if it's a git repo, otherwise show welcome
                let raw_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let repo_path = std::fs::canonicalize(&raw_path).unwrap_or(raw_path);
                match git2::Repository::discover(&repo_path) {
                    Ok(repo) => {
                        let workdir = repo.workdir().unwrap_or_else(|| repo.path()).to_path_buf();
                        vec![workdir]
                    }
                    Err(_) => {
                        // Not a git repo — show empty welcome screen
                        Vec::new()
                    }
                }
            }
        };

        log::info!(
            "startup resolved {} repositories (clean_exit={})",
            repos_to_open.len(),
            was_clean_exit
        );

        // Small centered window for the splash, maximizes on workspace transition.
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("rgitui".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            window_min_size: Some(Size {
                width: px(320.0),
                height: px(420.0),
            }),
            window_decorations: Some(gpui::WindowDecorations::Server),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                Size {
                    width: px(320.0),
                    height: px(420.0),
                },
                cx,
            ))),
            focus: true,
            show: true,
            app_id: Some("rgitui".to_string()),
            ..Default::default()
        };

        let window = cx
            .open_window(options, |_window, cx| {
                cx.new(|cx| {
                    let splash = cx.new(rgitui_workspace::SplashScreen::new);

                    let init = WorkspaceInit {
                        repos_to_open,
                        startup_workspace,
                        has_cli_path,
                        was_clean_exit,
                    };

                    // Start loading repos immediately so refresh + diff prewarm
                    // run in parallel with the splash animation.
                    let workspace = AppRoot::init_workspace(init, cx);

                    // Play the whole opening animation, then hand over as soon
                    // as there is something to show.
                    //
                    // This used to be a flat 1.5s wait with no regard for the
                    // repository: it handed over to an empty workspace if
                    // loading was slow, and sat on a finished one if it was
                    // fast. Waiting on the animation and then on the data fixes
                    // the first case without shortening the second.
                    let workspace_for_splash = workspace.downgrade();
                    cx.spawn(
                        async move |this: gpui::WeakEntity<AppRoot>, cx: &mut gpui::AsyncApp| {
                            let started = std::time::Instant::now();
                            loop {
                                cx.background_executor().timer(SPLASH_POLL).await;
                                let elapsed = started.elapsed();
                                if elapsed >= SPLASH_MAX {
                                    break;
                                }
                                if elapsed < SPLASH_MIN {
                                    continue;
                                }
                                let ready = workspace_for_splash
                                    .read_with(cx, |workspace, cx| {
                                        workspace.has_visible_content(cx)
                                    })
                                    // The workspace is gone, so waiting for it
                                    // to fill is pointless.
                                    .unwrap_or(true);
                                if ready {
                                    break;
                                }
                            }
                            this.update(cx, |this: &mut AppRoot, cx| {
                                this.show_workspace(cx);
                            })
                            .ok();
                        },
                    )
                    .detach();

                    AppRoot {
                        splash: Some(splash),
                        workspace: Some(workspace),
                        transitioning: false,
                        needs_maximize: false,
                        focus: cx.focus_handle(),
                    }
                })
            })
            .expect("Failed to open window");

        start_perf_harness(window, cx);
    });
}

/// Starts the performance harness if `RGITUI_PERF` asked for one.
///
/// Installed here rather than after the splash so that startup itself — repo
/// load, first refresh, first paint — is inside the measurement. That window is
/// where a cold-start regression would hide.
fn start_perf_harness(window: WindowHandle<AppRoot>, cx: &mut App) {
    let installed = window.update(cx, |root, window, cx| {
        let Some(workspace) = root.workspace.as_ref() else {
            return false;
        };
        rgitui_workspace::perf::install(workspace.downgrade(), window, cx);
        rgitui_workspace::perf::is_active(cx)
    });

    if !matches!(installed, Ok(true)) {
        return;
    }

    // A recorded session has no final step to finish on, so closing the window
    // is what ends it. Without this the run would produce no report at all,
    // which is the one outcome worse than a partial one.
    //
    // The observer is global, so it has to check which window closed: finishing
    // on any of them means dismissing the settings window writes the report and
    // ends the run while the workspace is still open, silently discarding the
    // rest of the session.
    let instrumented = window.window_id();
    cx.on_window_closed(move |cx, closed_id| {
        if closed_id == instrumented {
            rgitui_workspace::perf::finish(cx);
        }
    })
    .detach();
}
