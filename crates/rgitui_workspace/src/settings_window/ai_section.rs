//! The AI settings page.
//!
//! Three sections replacing six flat cards, plus a status strip in the header
//! block that never scrolls away:
//!
//! 1. **Connection** — one expandable row per provider, each owning its own
//!    key field, its own connection status, and its own model pin. A single
//!    shared key field is what made "connected" an assertion the app could not
//!    back up.
//! 2. **Model** — folded into the expanded provider row, so it is always shown
//!    against the credentials it will actually be used with.
//! 3. **Behaviour** — commit style with a live example, and the two toggles
//!    that cost money, each stating what it costs.

use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, ElementId, FontWeight, SharedString};
use rgitui_ai::catalog::{
    self, classify_pinned, filter_models, CatalogSource, ModelFilter, ModelInfo, PinnedModelStatus,
};
use rgitui_ai::CommitStyle;
use rgitui_settings::{AiProvider, SettingsState};
use rgitui_theme::{ActiveTheme, Color};
use rgitui_ui::{
    Button, ButtonSize, ButtonStyle, CheckState, Checkbox, ConnectionState, Disclosure, Icon,
    IconButton, IconName, IconSize, Label, LabelSize, PickerChip, PickerRow, StatusPill,
};

use super::view::{
    credential_store_name, masked_tail, relative_age, MaskedField, SettingsSection, SettingsView,
    SETTINGS_TAB_INDEX_BASE,
};

/// How long a connection test may take before it is treated as a failure.
const CONNECTION_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// How long to wait after the last keystroke before writing a secret.
const SECRET_SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(400);

impl SettingsView {
    // ── Behaviour ────────────────────────────────────────────────────

    /// Show the AI page. Everything that reports an AI misconfiguration routes
    /// here so the complaint and its fix are one click apart.
    pub fn show_ai_section(&mut self, cx: &mut Context<Self>) {
        self.active_section = SettingsSection::Ai;
        let provider = self.ai_provider;
        self.expanded_ai_provider = Some(provider);
        self.load_ai_catalog(provider, cx);
        cx.notify();
    }

    /// Queue a keychain write for `SECRET_SAVE_DEBOUNCE` from now.
    ///
    /// Replaces the old per-keystroke-then-never model, where typing a key and
    /// closing the window discarded it while pasting the identical key saved it
    /// immediately: same field, same value, three different outcomes.
    pub(super) fn schedule_secret_save(&mut self, cx: &mut Context<Self>) {
        self.pending_secret_save = Some(cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(SECRET_SAVE_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                this.pending_secret_save = None;
                this.save_settings(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    /// Write any pending secret immediately, cancelling the debounce.
    pub(super) fn flush_secret_save(&mut self, cx: &mut Context<Self>) {
        self.pending_secret_save = None;
        self.save_settings(cx);
    }

    /// Make `provider` the active one, remembering the model the previous
    /// provider was using so switching back restores it.
    pub(super) fn use_ai_provider(&mut self, provider: AiProvider, cx: &mut Context<Self>) {
        if self.ai_provider == provider {
            return;
        }
        // Remember the outgoing provider's model before overwriting the pin.
        let previous = self.ai_provider;
        let previous_model = self.ai_model.clone();
        cx.update_global::<SettingsState, _>(|state, _cx| {
            let ai = &mut state.settings_mut().ai;
            if !previous_model.trim().is_empty() {
                ai.models_by_provider
                    .insert(previous.id().to_string(), previous_model);
            }
            ai.set_active_provider(provider);
        });
        self.ai_provider = provider;
        self.ai_model = cx
            .read_global::<SettingsState, _>(|state, _cx| state.settings().ai.model_for(provider));
        self.expanded_ai_provider = Some(provider);
        self.save_settings(cx);
        self.load_ai_catalog(provider, cx);
        cx.notify();
    }

    /// Expand one provider row, collapsing whichever was open.
    pub(super) fn toggle_ai_provider_row(&mut self, provider: AiProvider, cx: &mut Context<Self>) {
        if self.expanded_ai_provider == Some(provider) {
            self.expanded_ai_provider = None;
        } else {
            self.expanded_ai_provider = Some(provider);
            self.ai_model_picker_open = false;
            self.load_ai_catalog(provider, cx);
        }
        cx.notify();
    }

    pub(super) fn select_ai_model(&mut self, model: String, cx: &mut Context<Self>) {
        self.ai_model = model.clone();
        self.ai_model_picker_open = false;
        cx.update_global::<SettingsState, _>(|state, _cx| {
            state.settings_mut().ai.set_active_model(model);
        });
        self.save_settings(cx);
        cx.notify();
    }

    pub(super) fn commit_base_url_override(&mut self, cx: &mut Context<Self>) {
        let value = self.ai_base_url_editor.read(cx).text().trim().to_string();
        if let Err(error) = rgitui_ai::validate_base_url(&value) {
            self.set_feedback(error.message(), true, cx);
            return;
        }
        self.ai_base_url_override = value;
        self.save_settings(cx);
    }

    /// The one honest answer the settings page can give about a key.
    ///
    /// `has_api_key` was only `!trim().is_empty()`, so a typo was
    /// indistinguishable from a working key until the user staged, clicked,
    /// waited and got a red toast.
    pub(super) fn test_ai_connection(&mut self, provider: AiProvider, cx: &mut Context<Self>) {
        let key = self
            .ai_key_editors
            .get(&provider)
            .map(|editor| editor.read(cx).text().trim().to_string())
            .unwrap_or_default();
        if key.is_empty() {
            self.ai_connection
                .insert(provider, ConnectionState::Unconfigured);
            cx.notify();
            return;
        }

        self.ai_connection
            .insert(provider, ConnectionState::Testing);
        self.ai_connection_error.remove(&provider);
        cx.notify();

        let client = cx.http_client();
        self.ai_test_task = Some(cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // The round-trip and the parse both run off the UI thread.
            let executor = cx.background_executor().clone();
            let probe = executor
                .spawn(async move { catalog::fetch_models(provider, &client, Some(&key)).await });
            let timeout = cx.background_executor().timer(CONNECTION_TEST_TIMEOUT);
            let result = futures::future::select(Box::pin(probe), Box::pin(timeout)).await;

            let outcome = match result {
                futures::future::Either::Left((outcome, _)) => outcome,
                futures::future::Either::Right(((), _)) => Err(anyhow::anyhow!(
                    "{} did not respond within {}s.",
                    provider.display_name(),
                    CONNECTION_TEST_TIMEOUT.as_secs()
                )),
            };

            this.update(cx, |this, cx| match outcome {
                Ok(models) => {
                    this.ai_connection
                        .insert(provider, ConnectionState::Connected);
                    this.ai_connection_error.remove(&provider);
                    this.ai_verified_at
                        .insert(provider, std::time::Instant::now());
                    // The test already fetched the catalogue; keep it rather
                    // than making a second identical request.
                    this.apply_ai_catalog(provider, models, CatalogSource::Live, cx);
                    cx.notify();
                }
                Err(error) => {
                    this.ai_connection.insert(provider, ConnectionState::Failed);
                    this.ai_connection_error
                        .insert(provider, connection_error_message(provider, &error));
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Read `provider`'s cached catalogue and render it at once, revalidating
    /// in the background only when it is not fresh.
    pub(super) fn load_ai_catalog(&mut self, provider: AiProvider, cx: &mut Context<Self>) {
        if let std::collections::btree_map::Entry::Vacant(e) = self.ai_catalog.entry(provider) {
            let cached = catalog::read_cached(provider);
            let fresh = cached.as_ref().map(|cached| {
                catalog::freshness(cached.fetched_at, catalog::now_unix())
                    == catalog::CatalogFreshness::Fresh
            });
            let (models, source) = catalog::resolve_catalog(provider, cached);
            e.insert(models);
            self.ai_catalog_source.insert(provider, source);
            self.sync_model_picker(cx);
            if fresh == Some(true) {
                return;
            }
        } else if matches!(
            self.ai_catalog_source.get(&provider),
            Some(CatalogSource::Live)
        ) {
            return;
        }

        self.refresh_ai_catalog(provider, false, cx);
    }

    /// Fetch `provider`'s catalogue in the background.
    ///
    /// A failure keeps the cached list on screen and reports itself inline.
    /// Blanking the picker because a refresh failed would be strictly worse
    /// than showing a slightly stale list.
    pub(super) fn refresh_ai_catalog(
        &mut self,
        provider: AiProvider,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let key = cx.read_global::<SettingsState, _>(|state, _cx| state.ai_api_key_for(provider));
        if catalog::catalog_needs_key(provider) && key.is_none() {
            // Not an error: the user simply has not connected this provider
            // yet, and the bundled list is already showing.
            return;
        }
        if !force && self.ai_catalog_loading {
            return;
        }

        let client = cx.http_client();
        let generation = self.ai_catalog_generation.wrapping_add(1);
        self.ai_catalog_generation = generation;
        self.ai_catalog_loading = true;
        cx.notify();

        self.ai_catalog_task = Some(cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // Send, parse and cache-write, all off the UI thread: the
            // unfiltered OpenRouter payload is around 700 KB.
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let models = catalog::fetch_models(provider, &client, key.as_deref()).await?;
                    let envelope = catalog::CachedCatalog {
                        schema: catalog::CATALOG_SCHEMA,
                        fetched_at: catalog::now_unix(),
                        models: models.clone(),
                    };
                    if let Err(error) = catalog::write_cached(provider, &envelope) {
                        log::warn!("Failed to cache the {} model list: {}", provider, error);
                    }
                    anyhow::Ok(models)
                })
                .await;

            this.update(cx, |this, cx| {
                // Drop a superseded result, the same guard `apply_refresh_data`
                // uses in `rgitui_git`.
                if this.ai_catalog_generation != generation {
                    return;
                }
                this.ai_catalog_loading = false;
                match fetched {
                    Ok(models) => {
                        this.ai_catalog_error = None;
                        this.apply_ai_catalog(provider, models, CatalogSource::Live, cx);
                    }
                    Err(error) => {
                        this.ai_catalog_error = Some(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn apply_ai_catalog(
        &mut self,
        provider: AiProvider,
        models: Vec<ModelInfo>,
        source: CatalogSource,
        cx: &mut Context<Self>,
    ) {
        self.ai_catalog.insert(provider, models);
        self.ai_catalog_source.insert(provider, source);
        self.sync_model_picker(cx);
    }

    /// Feed the picker the current provider's catalogue.
    pub(super) fn sync_model_picker(&mut self, cx: &mut Context<Self>) {
        let provider = self.ai_provider;
        let models = self.ai_catalog.get(&provider).cloned().unwrap_or_default();
        let filter = ModelFilter {
            tools_only: self.ai_use_tools,
            ..ModelFilter::default()
        };
        let rows: Vec<PickerRow> = filter_models(&models, filter)
            .into_iter()
            .map(|model| model_row(provider, model))
            .collect();

        let footer = self
            .ai_catalog_source
            .get(&provider)
            .map(|source| catalog_source_label(*source));
        let status = self.ai_catalog_error.clone().map(|error| {
            format!(
                "Couldn't refresh the model list — showing the last known {} models. {error}",
                models.len()
            )
        });
        let selected = self.ai_model.clone();

        self.ai_model_picker.update(cx, |picker, cx| {
            picker.set_rows(rows, cx);
            picker.set_chips(
                vec![
                    PickerChip::new("", "All"),
                    PickerChip::new("cheap", "Cheap"),
                    PickerChip::new("tools", "Tools"),
                    PickerChip::new("free", "Free"),
                ],
                cx,
            );
            picker.set_selected(Some(selected.into()), cx);
            picker.set_footer_note(footer.map(SharedString::from), cx);
            picker.set_status_note(status.map(SharedString::from), cx);
        });
    }

    // ── Rendering ────────────────────────────────────────────────────

    /// The sticky status strip. Lives in the header block, outside the scroll
    /// child, so it never scrolls away and never shifts the layout.
    pub(super) fn render_ai_status_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let provider = self.ai_provider;
        let state = self.connection_state(provider);
        let enabled = self.ai_enabled;

        let detail = if !enabled {
            "AI is turned off. Nothing will be sent to any provider.".to_string()
        } else {
            match state {
                ConnectionState::Connected => {
                    let verified = self
                        .ai_verified_at
                        .get(&provider)
                        .map(|at| format!(" · verified {}", relative_age(at.elapsed())))
                        .unwrap_or_default();
                    format!(
                        "{} · {}{}",
                        provider.display_name(),
                        self.ai_model,
                        verified
                    )
                }
                ConnectionState::Testing => format!("Testing {}…", provider.display_name()),
                ConnectionState::Failed => self
                    .ai_connection_error
                    .get(&provider)
                    .cloned()
                    .unwrap_or_else(|| format!("{} rejected this key.", provider.display_name())),
                ConnectionState::Unconfigured => {
                    format!("Add a {} API key to get started.", provider.display_name())
                }
            }
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .w_full()
            .p(px(10.))
            .rounded(px(8.))
            .bg(colors.element_background)
            .child(
                div().flex_1().min_w_0().child(
                    StatusPill::new(
                        "ai-status",
                        if enabled {
                            state
                        } else {
                            ConnectionState::Unconfigured
                        },
                        "AI Commit Messages",
                    )
                    .detail(detail),
                ),
            )
            .child(
                div()
                    .id("ai-enabled-toggle")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .tab_index(SETTINGS_TAB_INDEX_BASE)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ai_enabled = !this.ai_enabled;
                        this.save_settings(cx);
                    }))
                    .child(
                        Label::new("Enabled")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(Checkbox::new(
                        "ai-enabled-cb",
                        if enabled {
                            CheckState::Checked
                        } else {
                            CheckState::Unchecked
                        },
                    )),
            )
    }

    /// The AI page body.
    pub(super) fn render_ai_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut section = div().flex().flex_col().w_full().min_w_0().gap(px(16.));
        section = section.child(Self::section_label_row("CONNECTION"));
        section = section.child(self.render_provider_accordion(cx));
        section = section.child(Self::section_label_row("BEHAVIOUR"));
        section = section.child(self.render_behaviour_card(cx));
        section
    }

    fn section_label_row(text: &'static str) -> impl IntoElement {
        div().w_full().child(
            Label::new(text)
                .size(LabelSize::XSmall)
                .weight(FontWeight::SEMIBOLD)
                .color(Color::Muted),
        )
    }

    fn connection_state(&self, provider: AiProvider) -> ConnectionState {
        if let Some(state) = self.ai_connection.get(&provider) {
            return *state;
        }
        if self.provider_has_key(provider) {
            // A stored key is "configured", never "verified" — the difference
            // is the whole point of the Test button.
            ConnectionState::Connected
        } else {
            ConnectionState::Unconfigured
        }
    }

    fn provider_has_key(&self, provider: AiProvider) -> bool {
        self.ai_keys_loaded
            .get(&provider)
            .is_some_and(|key| !key.trim().is_empty())
    }

    fn render_provider_accordion(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors().clone();
        let mut card = Self::setting_card(cx).gap(px(2.));

        for (index, provider) in AiProvider::ALL.iter().copied().enumerate() {
            let is_expanded = self.expanded_ai_provider == Some(provider);
            let is_active = self.ai_provider == provider;
            let state = self.connection_state(provider);
            // The header alone answers "which providers am I set up on, and
            // which am I using?" without expanding anything.
            let pinned_model = cx.read_global::<SettingsState, _>(|settings, _cx| {
                settings.settings().ai.model_for(provider)
            });

            let header = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .w_full()
                // A comfortable hit target; the chevron and the row body share
                // one click handler.
                .min_h(px(32.))
                .child(
                    div().flex_shrink_0().child(
                        Disclosure::new(
                            ElementId::Name(format!("ai-provider-{}", provider.id()).into()),
                            provider.display_name(),
                            is_expanded,
                        )
                        .tab_index(SETTINGS_TAB_INDEX_BASE + 1 + index as isize)
                        .on_toggle(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.toggle_ai_provider_row(provider, cx);
                            },
                        )),
                    ),
                )
                .child(
                    // The active provider is a radio, marked in the header
                    // rather than chosen from a separate pill row.
                    div().flex_shrink_0().child(
                        Icon::new(if is_active {
                            IconName::Sparkle
                        } else {
                            IconName::DotOutline
                        })
                        .size(IconSize::XSmall)
                        .color(if is_active {
                            Color::Accent
                        } else {
                            Color::Muted
                        }),
                    ),
                )
                .child(div().flex_1().min_w_0())
                .child(div().flex_shrink_0().child(StatusPill::for_state(
                    ElementId::Name(format!("ai-state-{}", provider.id()).into()),
                    state,
                )))
                .child(
                    div().flex_shrink_0().min_w(px(140.)).child(
                        Label::new(if state == ConnectionState::Unconfigured {
                            SharedString::default()
                        } else {
                            SharedString::from(pinned_model)
                        })
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .truncate(),
                    ),
                );

            let mut row = div()
                .id(ElementId::Name(
                    format!("ai-provider-row-{}", provider.id()).into(),
                ))
                .flex()
                .flex_col()
                .w_full()
                .min_w_0()
                .px(px(6.))
                .py(px(4.))
                .rounded(px(6.))
                .when(is_expanded, |el| el.bg(colors.element_background))
                .child(header);

            if is_expanded {
                row = row.child(self.render_provider_body(provider, index, cx));
            }

            card = card.child(row);
        }

        card
    }

    /// The expanded provider body — key field, model, and Advanced.
    fn render_provider_body(
        &mut self,
        provider: AiProvider,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors().clone();
        let state = self.connection_state(provider);
        let has_key = self.provider_has_key(provider);
        let tab_base = SETTINGS_TAB_INDEX_BASE + 20 + (index as isize * 10);

        // `div().flex().flex_col()`, never `v_flex()`/`h_flex()` here: the
        // forced vertical centring in the shared helpers is a recurring cause
        // of broken scroll containers and misaligned children.
        let mut body = div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .gap(px(10.))
            .pt(px(10.))
            .pl(px(20.))
            .pr(px(6.))
            .pb(px(6.));

        if !has_key {
            // A provider with no key opens onto onboarding, not an empty text
            // box. This is the first thing a new user sees.
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .w_full()
                    .child(
                        Icon::new(IconName::Sparkle)
                            .size(IconSize::Medium)
                            .color(Color::Accent),
                    )
                    .child(
                        Label::new("Connect an AI provider")
                            .size(LabelSize::Default)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new("rgitui writes commit messages from your staged diff.")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );
        }

        body = body.child(self.render_key_field(provider, tab_base, cx));

        // Status line under the field: what the app actually knows.
        let status_line: Option<SharedString> = match state {
            ConnectionState::Connected if has_key => Some(
                match self.ai_verified_at.get(&provider) {
                    Some(at) => format!(
                        "Verified {} · stored in {}",
                        relative_age(at.elapsed()),
                        credential_store_name()
                    ),
                    None => format!(
                        "Key stored in {}. Test it to confirm it works.",
                        credential_store_name()
                    ),
                }
                .into(),
            ),
            ConnectionState::Testing => {
                Some(format!("Testing {}…", self.ai_model).to_string().into())
            }
            ConnectionState::Failed => self
                .ai_connection_error
                .get(&provider)
                .cloned()
                .map(SharedString::from),
            _ => Some(
                format!(
                    "Keys are stored in {}, never in settings.json, and are only read when a \
                     request is sent.",
                    credential_store_name()
                )
                .into(),
            ),
        };
        if let Some(line) = status_line {
            body = body.child(Label::new(line).size(LabelSize::XSmall).color(match state {
                ConnectionState::Failed => Color::Error,
                ConnectionState::Connected => Color::Success,
                _ => Color::Muted,
            }));
        }

        if has_key {
            body = body.child(self.render_model_row(provider, tab_base, cx));
        }

        body = body.child(self.render_provider_actions(provider, has_key, tab_base, cx));

        if provider.is_openai_compatible() {
            body = body.child(self.render_advanced(provider, tab_base, cx));
        }

        body.border_t_1()
            .border_color(colors.border_variant)
            .into_any_element()
    }

    fn render_key_field(
        &self,
        provider: AiProvider,
        tab_base: isize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(editor) = self.ai_key_editors.get(&provider).cloned() else {
            return div().into_any_element();
        };
        let unmasked = self.is_field_unmasked(MaskedField::AiApiKey(provider));
        let tail = self
            .ai_keys_loaded
            .get(&provider)
            .map(|key| masked_tail(key))
            .unwrap_or_default();

        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Label::new("API key")
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .when(!tail.is_empty() && !unmasked, |el| {
                        // The last four characters answer "is this the right
                        // key?" without unmasking the whole thing.
                        el.child(
                            Label::new(tail.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new(
                            ElementId::Name(format!("ai-key-url-{}", provider.id()).into()),
                            "Get a key",
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .icon(IconName::ExternalLink)
                        .color(Color::Accent)
                        .tab_index(tab_base + 3)
                        .on_click(move |_event, _window, cx| {
                            cx.open_url(provider.key_url());
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .w_full()
                    .child(
                        // A lock, not a decorative eye two inches from a live
                        // Show button using the same glyph.
                        Icon::new(IconName::Lock)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1().min_w_0().child(editor))
                    .child(
                        IconButton::new(
                            ElementId::Name(format!("ai-key-paste-{}", provider.id()).into()),
                            IconName::File,
                        )
                        .size(ButtonSize::Compact)
                        .color(Color::Muted)
                        .tooltip("Paste from clipboard")
                        .tab_index(tab_base + 1)
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.import_from_clipboard(MaskedField::AiApiKey(provider), cx);
                            },
                        )),
                    )
                    .child(
                        IconButton::new(
                            ElementId::Name(format!("ai-key-mask-{}", provider.id()).into()),
                            if unmasked {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            },
                        )
                        .size(ButtonSize::Compact)
                        .color(Color::Muted)
                        .tooltip(if unmasked { "Hide" } else { "Show" })
                        .tab_index(tab_base + 2)
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.toggle_mask_visibility(MaskedField::AiApiKey(provider), cx);
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    fn render_model_row(
        &mut self,
        provider: AiProvider,
        tab_base: isize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.colors().clone();
        let models = self.ai_catalog.get(&provider).cloned().unwrap_or_default();
        let source = self
            .ai_catalog_source
            .get(&provider)
            .copied()
            .unwrap_or(CatalogSource::Bundled);
        let pinned = if provider == self.ai_provider {
            self.ai_model.clone()
        } else {
            cx.read_global::<SettingsState, _>(|state, _cx| state.settings().ai.model_for(provider))
        };
        let status = classify_pinned(&pinned, &models, source, self.ai_use_tools);
        let is_active_provider = provider == self.ai_provider;

        let summary: SharedString = match &status {
            PinnedModelStatus::Known(model) => model.summary_line().into(),
            _ => SharedString::default(),
        };

        let mut column = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(4.))
            .child(
                Label::new("Model")
                    .size(LabelSize::Small)
                    .weight(FontWeight::SEMIBOLD),
            )
            .child(
                div()
                    .id(ElementId::Name(
                        format!("ai-model-field-{}", provider.id()).into(),
                    ))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .w_full()
                    .min_h(px(32.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.editor_background)
                    .cursor_pointer()
                    .tab_index(tab_base + 4)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        if !is_active_provider {
                            this.use_ai_provider(provider, cx);
                        }
                        this.ai_model_picker_open = !this.ai_model_picker_open;
                        this.sync_model_picker(cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                Label::new(SharedString::from(pinned.clone()))
                                    .size(LabelSize::Small),
                            )
                            .when(!summary.is_empty(), |el| {
                                el.child(
                                    Label::new(summary.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                            }),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    ),
            );

        // Inline warnings, never a mutation: `settings.ai.model` is user
        // intent, and silently retargeting it is how someone ends up billed
        // for a model they did not choose.
        match &status {
            PinnedModelStatus::Missing { suggestion } => {
                let mut warning = div().flex().flex_row().items_center().gap(px(6.)).child(
                    Label::new(format!(
                        "`{pinned}` is not in {}'s current model list. It may have been retired.",
                        provider.display_name()
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Warning),
                );
                if let Some(suggestion) = suggestion.clone() {
                    warning = warning.child(
                        Button::new(
                            ElementId::Name(
                                format!("ai-model-suggestion-{}", provider.id()).into(),
                            ),
                            format!("Use {suggestion}"),
                        )
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .color(Color::Accent)
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.select_ai_model(suggestion.clone(), cx);
                            },
                        )),
                    );
                }
                column = column.child(warning);
            }
            PinnedModelStatus::Incompatible { reason } => {
                // The highest-value check: an opaque runtime 404 becomes a
                // settings-time warning.
                column = column.child(
                    Label::new(reason.clone())
                        .size(LabelSize::XSmall)
                        .color(Color::Warning),
                );
            }
            // `Unverified` deliberately warns about nothing: a bundled or
            // absent catalogue is not evidence a model is gone, and crying
            // wolf offline is worse than staying quiet.
            PinnedModelStatus::Unverified | PinnedModelStatus::Known(_) => {}
        }

        column = column.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(
                    Label::new(format!(
                        "{} models · {}",
                        models.len(),
                        catalog_source_label(source)
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .child(div().flex_1())
                .child(
                    Button::new(
                        ElementId::Name(format!("ai-model-refresh-{}", provider.id()).into()),
                        if self.ai_catalog_loading {
                            "Refreshing…"
                        } else {
                            "Refresh"
                        },
                    )
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Compact)
                    .icon(IconName::Refresh)
                    .color(Color::Muted)
                    .disabled(self.ai_catalog_loading)
                    .tab_index(tab_base + 5)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.refresh_ai_catalog(provider, true, cx);
                        },
                    )),
                ),
        );

        if self.ai_model_picker_open && is_active_provider {
            column = column.child(self.ai_model_picker.clone());
        }

        column.into_any_element()
    }

    fn render_provider_actions(
        &self,
        provider: AiProvider,
        has_key: bool,
        tab_base: isize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_active = self.ai_provider == provider;
        let testing = self.connection_state(provider) == ConnectionState::Testing;

        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .w_full()
            .pt(px(4.))
            .child(div().flex_1());

        if !has_key {
            // "Connect" is the user's goal; "Save" never was.
            return row
                .child(
                    Button::new(
                        ElementId::Name(format!("ai-connect-{}", provider.id()).into()),
                        "Connect",
                    )
                    .style(ButtonStyle::Filled)
                    .size(ButtonSize::Compact)
                    .color(Color::Accent)
                    .tab_index(tab_base + 6)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            // Save and test in one action.
                            this.flush_secret_save(cx);
                            this.use_ai_provider(provider, cx);
                            this.test_ai_connection(provider, cx);
                        },
                    )),
                )
                .into_any_element();
        }

        if !is_active {
            row = row.child(
                Button::new(
                    ElementId::Name(format!("ai-use-{}", provider.id()).into()),
                    "Use this provider",
                )
                .style(ButtonStyle::Filled)
                .size(ButtonSize::Compact)
                .color(Color::Accent)
                .tab_index(tab_base + 6)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.use_ai_provider(provider, cx);
                })),
            );
        }

        row.child(
            Button::new(
                ElementId::Name(format!("ai-test-{}", provider.id()).into()),
                if testing { "Testing…" } else { "Test" },
            )
            .style(ButtonStyle::Outlined)
            .size(ButtonSize::Compact)
            .disabled(testing)
            .tab_index(tab_base + 7)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.flush_secret_save(cx);
                this.test_ai_connection(provider, cx);
            })),
        )
        .child(
            Button::new(
                ElementId::Name(format!("ai-remove-{}", provider.id()).into()),
                "Remove key",
            )
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Compact)
            .color(Color::Error)
            .tab_index(tab_base + 8)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.remove_ai_key(provider, cx);
            })),
        )
        .into_any_element()
    }

    pub(super) fn remove_ai_key(&mut self, provider: AiProvider, cx: &mut Context<Self>) {
        if let Some(editor) = self.ai_key_editors.get(&provider).cloned() {
            editor.update(cx, |e, cx| e.clear(cx));
        }
        self.ai_connection
            .insert(provider, ConnectionState::Unconfigured);
        self.ai_connection_error.remove(&provider);
        self.ai_verified_at.remove(&provider);
        self.flush_secret_save(cx);
        cx.notify();
    }

    /// The `base_url_override` field, behind an Advanced disclosure so it does
    /// not clutter the common path.
    fn render_advanced(
        &self,
        provider: AiProvider,
        tab_base: isize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = self.ai_advanced_open;
        let mut column = div().flex().flex_col().w_full().gap(px(6.)).child(
            Disclosure::new(
                ElementId::Name(format!("ai-advanced-{}", provider.id()).into()),
                "Advanced",
                open,
            )
            .tab_index(tab_base + 9)
            .on_toggle(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.ai_advanced_open = !this.ai_advanced_open;
                cx.notify();
            })),
        );

        if !open {
            return column.into_any_element();
        }

        let host = rgitui_ai::effective_host(provider, &self.ai_base_url_override);
        let overridden = !self.ai_base_url_override.trim().is_empty();

        column = column
            .child(
                Label::new("Base URL")
                    .size(LabelSize::Small)
                    .weight(FontWeight::SEMIBOLD),
            )
            .child(div().w_full().child(self.ai_base_url_editor.clone()))
            .child(
                Label::new(if overridden {
                    // Say plainly where the key goes. A user pointing this at
                    // a third-party gateway should see that stated.
                    format!(
                        "Requests go to {host} instead of {}. Your API key is sent to that host.",
                        provider.default_host()
                    )
                } else {
                    format!(
                        "Empty means use {}. Only OpenAI-compatible providers honour an override.",
                        provider.default_host()
                    )
                })
                .size(LabelSize::XSmall)
                .color(if overridden {
                    Color::Warning
                } else {
                    Color::Muted
                }),
            );

        if provider == AiProvider::OpenRouter {
            let attribution = self.ai_openrouter_attribution;
            column = column.child(
                div()
                    .id("ai-openrouter-attribution")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .tab_index(tab_base + 10)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ai_openrouter_attribution = !this.ai_openrouter_attribution;
                        this.save_settings(cx);
                    }))
                    .child(Checkbox::new(
                        "ai-openrouter-attribution-cb",
                        if attribution {
                            CheckState::Checked
                        } else {
                            CheckState::Unchecked
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(Label::new("Send attribution headers").size(LabelSize::Small))
                            .child(
                                Label::new(
                                    "Adds HTTP-Referer and X-Title so rgitui appears on \
                                     OpenRouter's public leaderboard. Never functional.",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    ),
            );
        }

        column.into_any_element()
    }

    fn render_behaviour_card(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let style = CommitStyle::from_id(&self.ai_commit_style).unwrap_or_default();
        let ids: Vec<&str> = CommitStyle::ALL.iter().map(|style| style.id()).collect();

        Self::setting_card(cx)
            .child(Self::setting_label(
                "Commit style",
                "How the AI should format commit messages.",
            ))
            .child(self.pill_group(
                "commit-style",
                &ids,
                &self.ai_commit_style,
                |this, value, cx| {
                    this.ai_commit_style = value;
                    this.save_settings(cx);
                },
                cx,
            ))
            // A live example: the three labels are guesses until you see
            // output, which makes this the highest-value line on the page.
            .child(
                Label::new(style.example())
                    .size(LabelSize::XSmall)
                    .color(Color::Accent),
            )
            .child(Self::section_divider(cx))
            .child(self.render_behaviour_toggle(
                "ai-inject-ctx",
                "Include project context",
                "Adds README.md, CLAUDE.md and AGENTS.md to the prompt. ~4k extra tokens per request.",
                self.ai_inject_project_context,
                SETTINGS_TAB_INDEX_BASE + 80,
                |this, cx| {
                    this.ai_inject_project_context = !this.ai_inject_project_context;
                    this.save_settings(cx);
                },
                cx,
            ))
            .child(self.render_behaviour_toggle(
                "ai-use-tools",
                "Let the model read files",
                "The model may request file contents and commit history. Slower and more \
                 expensive; usually a better message.",
                self.ai_use_tools,
                SETTINGS_TAB_INDEX_BASE + 81,
                |this, cx| {
                    this.ai_use_tools = !this.ai_use_tools;
                    this.sync_model_picker(cx);
                    this.save_settings(cx);
                },
                cx,
            ))
    }

    /// A behaviour toggle that states what it costs.
    ///
    /// `use_tools` defaults to on, which means the out-of-box configuration is
    /// the expensive multi-round-trip one; presenting that as an unremarkable
    /// checkbox hid the tradeoff entirely.
    #[allow(clippy::too_many_arguments)]
    fn render_behaviour_toggle(
        &self,
        id: &'static str,
        title: &'static str,
        detail: &'static str,
        checked: bool,
        tab_index: isize,
        on_toggle: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .flex_row()
            .items_start()
            .gap(px(10.))
            .w_full()
            .min_w_0()
            .cursor_pointer()
            .tab_index(tab_index)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                on_toggle(this, cx);
            }))
            .child(div().flex_shrink_0().pt(px(1.)).child(Checkbox::new(
                ElementId::Name(format!("{id}-cb").into()),
                if checked {
                    CheckState::Checked
                } else {
                    CheckState::Unchecked
                },
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        Label::new(title)
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(detail)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
    }
}

/// A picker row for one model, with the facets its filter chips need.
fn model_row(provider: AiProvider, model: &ModelInfo) -> PickerRow {
    let mut row = PickerRow::new(model.id.clone(), model.display_name.clone())
        .secondary(provider.display_name())
        .trailing(model.trailing_label());

    if let Some(badge) = model.tool_support.badge() {
        row = row.badge(badge);
        if model.tool_support == rgitui_ai::catalog::ToolSupport::Supported {
            row = row.facet("tools");
        }
    } else {
        // `Unknown` renders with no badge rather than a false one, but still
        // belongs in the Tools chip: dropping it would empty the Gemini and
        // OpenAI lists entirely.
        row = row.facet("tools");
    }

    if model.is_free() {
        row = row.badge("free").facet("free");
    }
    // "Cheap" means under a dollar per million prompt tokens; a provider that
    // reports no pricing is not claimed to be cheap.
    if model.prompt_price_per_mtok.is_some_and(|price| price < 1.0) {
        row = row.facet("cheap");
    }
    row
}

/// How the catalogue on screen was obtained, so "three weeks old" is
/// distinguishable from "shipped with the app".
fn catalog_source_label(source: CatalogSource) -> String {
    match source {
        CatalogSource::Live => "updated just now".to_string(),
        CatalogSource::Cache { fetched_at } => {
            let age = catalog::now_unix().saturating_sub(fetched_at).max(0) as u64;
            format!(
                "updated {}",
                relative_age(std::time::Duration::from_secs(age))
            )
        }
        CatalogSource::Bundled => "shipped with rgitui".to_string(),
    }
}

/// Map a failed connection test onto the sentence that says what to do.
fn connection_error_message(provider: AiProvider, error: &anyhow::Error) -> String {
    let text = error.to_string();
    let name = provider.display_name();

    // Order matters: a rate limit is transient and a bad key is not, so the
    // 429 check must come before the auth check rather than being swallowed by
    // it.
    if text.contains("429") || text.contains("Rate limited") {
        return format!("Rate limited by {name}. This usually clears in a minute.");
    }
    if text.contains("401") || text.contains("403") || text.contains("did not accept") {
        let hint = match provider {
            AiProvider::Gemini => " Keys usually start with \"AIza\".",
            AiProvider::OpenAi => " Keys usually start with \"sk-\".",
            AiProvider::Anthropic => " Keys usually start with \"sk-ant-\".",
            AiProvider::OpenRouter => " Keys usually start with \"sk-or-\".",
            // No hint rather than an invented one.
            AiProvider::DeepSeek => "",
        };
        return format!("{name} did not accept this key.{hint}");
    }
    if text.contains("Couldn't reach") || text.contains("did not respond") {
        return text;
    }
    format!("{name} could not be reached: {text}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgitui_ai::catalog::ToolSupport;

    fn model(id: &str) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            display_name: id.to_string(),
            context_length: Some(128_000),
            max_output_tokens: Some(4096),
            prompt_price_per_mtok: None,
            completion_price_per_mtok: None,
            tool_support: ToolSupport::Supported,
            emits_text: true,
            is_variant: false,
            created: None,
        }
    }

    #[test]
    fn a_tool_capable_model_lands_in_the_tools_chip() {
        let row = model_row(AiProvider::OpenAi, &model("gpt-5.6-luna"));
        assert!(row.facets.iter().any(|facet| facet == "tools"));
        assert!(row.badges.iter().any(|badge| badge == "Tools"));
    }

    /// Dropping `Unknown` from the Tools chip would empty the Gemini and
    /// OpenAI lists entirely, since neither advertises tool support.
    #[test]
    fn an_unknown_tool_capability_stays_in_the_chip_but_gets_no_badge() {
        let mut info = model("gemini-3.1-flash-lite");
        info.tool_support = ToolSupport::Unknown;
        let row = model_row(AiProvider::Gemini, &info);
        assert!(row.facets.iter().any(|facet| facet == "tools"));
        assert!(row.badges.is_empty());
    }

    #[test]
    fn a_model_with_no_reported_price_is_not_claimed_to_be_cheap() {
        let row = model_row(AiProvider::OpenAi, &model("gpt-5.6-luna"));
        assert!(!row.facets.iter().any(|facet| facet == "cheap"));
    }

    #[test]
    fn cheap_and_free_are_derived_from_reported_pricing() {
        let mut cheap = model("vendor/cheap");
        cheap.prompt_price_per_mtok = Some(0.25);
        cheap.completion_price_per_mtok = Some(1.5);
        let row = model_row(AiProvider::OpenRouter, &cheap);
        assert!(row.facets.iter().any(|facet| facet == "cheap"));
        assert!(!row.facets.iter().any(|facet| facet == "free"));

        let mut free = model("vendor/free");
        free.prompt_price_per_mtok = Some(0.0);
        free.completion_price_per_mtok = Some(0.0);
        let row = model_row(AiProvider::OpenRouter, &free);
        assert!(row.facets.iter().any(|facet| facet == "free"));
        assert!(row.badges.iter().any(|badge| badge == "free"));
    }

    #[test]
    fn an_expensive_model_is_not_in_the_cheap_chip() {
        let mut expensive = model("vendor/opus");
        expensive.prompt_price_per_mtok = Some(15.0);
        let row = model_row(AiProvider::OpenRouter, &expensive);
        assert!(!row.facets.iter().any(|facet| facet == "cheap"));
    }

    #[test]
    fn the_catalogue_source_is_named_so_stale_is_distinguishable_from_bundled() {
        assert_eq!(
            catalog_source_label(CatalogSource::Live),
            "updated just now"
        );
        assert_eq!(
            catalog_source_label(CatalogSource::Bundled),
            "shipped with rgitui"
        );
        let hours_ago = catalog::now_unix() - 3 * 60 * 60;
        assert!(catalog_source_label(CatalogSource::Cache {
            fetched_at: hours_ago
        })
        .contains("h ago"));
    }

    #[test]
    fn an_auth_failure_names_the_key_prefix_the_provider_uses() {
        let error = anyhow::anyhow!("Google Gemini rejected the request (401)");
        let message = connection_error_message(AiProvider::Gemini, &error);
        assert!(message.contains("did not accept this key"));
        assert!(message.contains("AIza"));
    }

    #[test]
    fn a_rate_limit_says_it_will_clear_rather_than_looking_permanent() {
        let error = anyhow::anyhow!("OpenAI rejected the request (429)");
        let message = connection_error_message(AiProvider::OpenAi, &error);
        assert!(message.contains("clears in a minute"));
        // A transient limit must not be reported as a bad key.
        assert!(!message.contains("did not accept"));
    }

    #[test]
    fn a_network_failure_is_passed_through_unchanged() {
        let error = anyhow::anyhow!("Couldn't reach api.openai.com. Check your connection.");
        assert_eq!(
            connection_error_message(AiProvider::OpenAi, &error),
            "Couldn't reach api.openai.com. Check your connection."
        );
    }

    #[test]
    fn deepseek_gets_no_fabricated_key_prefix_hint() {
        let error = anyhow::anyhow!("DeepSeek rejected the request (401)");
        let message = connection_error_message(AiProvider::DeepSeek, &error);
        assert!(message.contains("did not accept this key"));
        assert!(!message.contains("start with"));
    }
}
