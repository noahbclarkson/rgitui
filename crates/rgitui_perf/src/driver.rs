//! Record real sessions; replay scripted ones.
//!
//! Both modes drive the same code path the user does. Replay resolves command
//! names through gpui's action registry and dispatches keystrokes through the
//! real keymap, so a scenario exercises dispatch, layout, paint and present
//! exactly as a keypress would — there are no test doubles anywhere in it.
//!
//! Recording exists because scripted scenarios only ever measure the paths
//! someone thought to script. A recorded session is ground truth about what
//! the app actually does under a real person, and it converts into a scenario
//! so that a session which felt slow becomes a repeatable benchmark.

use serde::{Deserialize, Serialize};

/// One step of a scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Dispatch a command by its `namespace::Name` action name.
    Action {
        action: String,
        #[serde(default = "one")]
        repeat: usize,
        #[serde(default)]
        settle: Settle,
    },
    /// Dispatch a keystroke through the keymap, e.g. `"ctrl-shift-p"`.
    Key {
        key: String,
        #[serde(default = "one")]
        repeat: usize,
        #[serde(default)]
        settle: Settle,
    },
    /// Synthesise a scroll wheel event at a window position.
    Scroll {
        x: f32,
        y: f32,
        delta_y: f32,
        #[serde(default = "one")]
        repeat: usize,
        #[serde(default)]
        settle: Settle,
    },
    /// Move the mouse, then press and release at a window position.
    Click { x: f32, y: f32 },
    /// Sleep for a fixed duration.
    Wait { ms: u64 },
    /// Block until a condition holds, or fail the run on timeout.
    WaitFor {
        condition: Condition,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u64,
    },
    /// Open a named measurement region.
    Mark { mark: String },
    /// Close the named measurement region opened by [`Step::Mark`].
    EndMark { mark: String },
    /// Force a heap census sample and label it.
    Snapshot { snapshot: String },
    /// Append a line to `count` tracked files in the working tree.
    ///
    /// Without this no scenario can produce a dirty working tree at all: the
    /// corpus generator finishes with a forced checkout, so the tree it hands
    /// over is clean, and `staging-churn` was staging an empty change set while
    /// describing itself as exercising cache invalidation.
    ///
    /// Paths are chosen by walking the repository, so a scenario does not have
    /// to know what the corpus generator named anything.
    Dirty {
        /// How many tracked files to touch.
        #[serde(default = "one")]
        count: usize,
    },
}

/// What to wait for before the next step.
///
/// Not `Eq`: [`Settle::Rate`] carries a rate in hertz, and a fractional rate is
/// the point — 30Hz auto repeat is not an integer number of frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Settle {
    /// Continue immediately.
    #[default]
    None,
    /// Wait for the next presented frame, so per-action latency measures
    /// keypress-to-pixels rather than keypress-to-return.
    Frame,
    /// Deliver at a fixed rate, whether or not the app kept up.
    ///
    /// The only mode that reproduces a held key. An operating system's auto
    /// repeat fires on its own clock — around 30Hz once the initial delay has
    /// passed — and does not wait to see whether the last event was handled, so
    /// events queue up when the app falls behind. That backlog is the entire
    /// phenomenon: [`Settle::Frame`] lock-steps dispatch to frame completion and
    /// therefore measures the one case where it cannot happen.
    Rate {
        /// Events per second.
        hz: f64,
    },
}

/// A condition [`Step::WaitFor`] can block on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// No git operation is in flight. Async work is awaited properly rather
    /// than slept through, so a slow machine does not silently measure a
    /// half-finished operation.
    OperationIdle,
}

fn one() -> usize {
    1
}

fn default_timeout_ms() -> u64 {
    30_000
}

/// A named, ordered list of steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Name used in report filenames and comparisons.
    pub name: String,
    /// What this scenario is trying to measure.
    #[serde(default)]
    pub description: String,
    /// Corpus tier the scenario expects to run against.
    #[serde(default)]
    pub corpus: Option<crate::corpus::Tier>,
    /// State the run starts from.
    #[serde(default)]
    pub start_from: StartState,
    /// Steps, run in order.
    pub steps: Vec<Step>,
}

/// Whether a run starts with rgitui's persistent caches populated.
///
/// rgitui keeps a history cache on disk and hydrates the commit list from it
/// before the first refresh, so the same scenario measures two different things
/// depending on whether that file exists. Left to chance it is whatever the
/// previous run wrote, which is how a scenario silently stops measuring what it
/// was written to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartState {
    /// No caches: what somebody opening a repository for the first time gets.
    #[default]
    Cold,
    /// Caches left in place, as on the second and every later launch.
    Warm,
}

impl Scenario {
    /// Parses a scenario from JSON, naming the file when it does not parse.
    pub fn from_json(source: &str) -> anyhow::Result<Self> {
        serde_json::from_str(source).map_err(|error| anyhow::anyhow!("invalid scenario: {error}"))
    }

    /// Loads a scenario from disk.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("cannot read scenario {}: {error}", path.display()))?;
        Self::from_json(&source)
    }

    /// Every action name the scenario references.
    ///
    /// Checked against gpui's registry before a run starts, so a scenario that
    /// names a command that no longer exists fails immediately with a clear
    /// message instead of silently measuring fewer steps than it claims.
    pub fn referenced_actions(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter_map(|step| match step {
                Step::Action { action, .. } => Some(action.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Checks that every `mark` is closed by a matching `end_mark`.
    pub fn validate_marks(&self) -> anyhow::Result<()> {
        let mut open: Vec<&str> = Vec::new();
        for step in &self.steps {
            match step {
                Step::Mark { mark } => open.push(mark),
                Step::EndMark { mark } => {
                    let found = open.iter().rposition(|candidate| candidate == mark);
                    match found {
                        Some(index) => {
                            open.remove(index);
                        }
                        None => anyhow::bail!("end_mark {mark:?} has no matching mark"),
                    }
                }
                _ => {}
            }
        }
        anyhow::ensure!(
            open.is_empty(),
            "mark(s) never closed with end_mark: {}",
            open.join(", ")
        );
        Ok(())
    }
}

/// One recorded event from a live session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Nanoseconds since the run started.
    pub t_ns: u128,
    /// The `namespace::Name` of the dispatched command.
    pub action: String,
    /// Index of the tab it was dispatched against.
    pub tab: Option<usize>,
}

/// Converts a recorded session into a replayable scenario.
///
/// Gaps between events are preserved as [`Step::Wait`] so that replay keeps the
/// rhythm of the original session; a burst of keypresses and a slow browse
/// stress the app differently and collapsing them would lose that.
pub fn trace_to_scenario(name: &str, events: &[TraceEvent], max_gap_ms: u64) -> Scenario {
    let mut steps = Vec::new();
    let mut previous_ns: Option<u128> = None;

    for event in events {
        if let Some(previous) = previous_ns {
            let gap_ms = ((event.t_ns.saturating_sub(previous)) / 1_000_000) as u64;
            if gap_ms > 0 {
                steps.push(Step::Wait {
                    ms: gap_ms.min(max_gap_ms),
                });
            }
        }
        steps.push(Step::Action {
            action: event.action.clone(),
            repeat: 1,
            // No settling. The gap emitted above is dispatch-to-dispatch time
            // from the real session, and it already contains however long the
            // app took to draw. Waiting for a frame *and then* sleeping the gap
            // adds a draw to every step, which serializes exactly the bursts —
            // held keys, fast scrolling — that are worth replaying, and makes
            // the generated benchmark measure something the user never did.
            settle: Settle::None,
        });
        previous_ns = Some(event.t_ns);
    }

    // Tab switches are made with the mouse as often as not, and there is no
    // command for "activate tab N" to convert them into — `NextTab` and
    // `PrevTab` are all there is, and they depend on where you started. A
    // scenario built from such a trace drives whichever tab replay happens to
    // open, so it says so rather than presenting itself as a faithful replay.
    let tabs: std::collections::BTreeSet<usize> = events.iter().filter_map(|e| e.tab).collect();
    let description = match tabs.len() {
        0 | 1 => format!("Recorded session, {} action(s)", events.len()),
        count => {
            log::warn!(
                "the recorded session spans {count} repository tabs; the generated scenario                  drives one, because there is no command that selects a tab by index"
            );
            format!(
                "Recorded session, {} action(s), originally across {count} tabs — replayed                  against one",
                events.len()
            )
        }
    };

    Scenario {
        name: name.to_string(),
        description,
        corpus: None,
        start_from: StartState::Cold,
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_parses_minimal_json_with_defaults() {
        let scenario = Scenario::from_json(
            r#"{"name":"s","steps":[{"action":{"action":"graph::SelectNext"}}]}"#,
        )
        .unwrap();
        assert_eq!(scenario.steps.len(), 1);
        assert_eq!(
            scenario.steps[0],
            Step::Action {
                action: "graph::SelectNext".to_string(),
                repeat: 1,
                settle: Settle::None,
            }
        );
    }

    #[test]
    fn a_scenario_that_does_not_say_starts_cold() {
        // The dangerous default is the other one: a warm start reports the
        // second-launch cost of a repository under a scenario name that reads
        // like a first launch, and nothing in the output says which it was.
        let scenario = Scenario::from_json(r#"{"name":"s","steps":[]}"#).unwrap();
        assert_eq!(scenario.start_from, StartState::Cold);
    }

    #[test]
    fn a_scenario_can_ask_for_the_populated_caches_of_a_second_launch() {
        let scenario =
            Scenario::from_json(r#"{"name":"s","start_from":"warm","steps":[]}"#).unwrap();
        assert_eq!(scenario.start_from, StartState::Warm);
    }

    #[test]
    fn referenced_actions_lists_only_action_steps() {
        let scenario = Scenario {
            name: "s".into(),
            description: String::new(),
            corpus: None,
            start_from: StartState::Cold,
            steps: vec![
                Step::Action {
                    action: "a::B".into(),
                    repeat: 1,
                    settle: Settle::None,
                },
                Step::Wait { ms: 5 },
                Step::Key {
                    key: "escape".into(),
                    repeat: 1,
                    settle: Settle::None,
                },
            ],
        };
        assert_eq!(scenario.referenced_actions(), vec!["a::B"]);
    }

    #[test]
    fn validate_marks_accepts_balanced_and_nested_marks() {
        let scenario = Scenario {
            name: "s".into(),
            description: String::new(),
            corpus: None,
            start_from: StartState::Cold,
            steps: vec![
                Step::Mark {
                    mark: "outer".into(),
                },
                Step::Mark {
                    mark: "inner".into(),
                },
                Step::EndMark {
                    mark: "inner".into(),
                },
                Step::EndMark {
                    mark: "outer".into(),
                },
            ],
        };
        scenario.validate_marks().unwrap();
    }

    #[test]
    fn validate_marks_rejects_an_unclosed_mark() {
        let scenario = Scenario {
            name: "s".into(),
            description: String::new(),
            corpus: None,
            start_from: StartState::Cold,
            steps: vec![Step::Mark {
                mark: "open".into(),
            }],
        };
        let error = scenario.validate_marks().unwrap_err().to_string();
        assert!(error.contains("never closed"), "error: {error}");
    }

    #[test]
    fn validate_marks_rejects_an_unmatched_end_mark() {
        let scenario = Scenario {
            name: "s".into(),
            description: String::new(),
            corpus: None,
            start_from: StartState::Cold,
            steps: vec![Step::EndMark {
                mark: "stray".into(),
            }],
        };
        let error = scenario.validate_marks().unwrap_err().to_string();
        assert!(error.contains("no matching mark"), "error: {error}");
    }

    #[test]
    fn a_trace_across_tabs_admits_it_cannot_reproduce_them() {
        let events = [
            TraceEvent {
                t_ns: 0,
                action: "a::B".into(),
                tab: Some(0),
            },
            TraceEvent {
                t_ns: 1_000_000,
                action: "a::C".into(),
                tab: Some(1),
            },
        ];

        let scenario = trace_to_scenario("recorded", &events, 5_000);

        assert!(
            scenario.description.contains("2 tabs"),
            "description: {}",
            scenario.description
        );
    }

    #[test]
    fn trace_converts_to_a_scenario_that_keeps_the_original_rhythm() {
        let events = [
            TraceEvent {
                t_ns: 0,
                action: "a::B".into(),
                tab: Some(0),
            },
            TraceEvent {
                t_ns: 250_000_000,
                action: "a::C".into(),
                tab: Some(0),
            },
        ];

        let scenario = trace_to_scenario("recorded", &events, 5_000);
        // No settling on the actions: the 250ms gap is dispatch-to-dispatch
        // time from the real session and already contains whatever the app
        // spent drawing. Waiting for a frame as well would add one to every
        // step and stretch the rhythm this conversion exists to preserve.
        assert_eq!(
            scenario.steps,
            vec![
                Step::Action {
                    action: "a::B".into(),
                    repeat: 1,
                    settle: Settle::None
                },
                Step::Wait { ms: 250 },
                Step::Action {
                    action: "a::C".into(),
                    repeat: 1,
                    settle: Settle::None
                },
            ]
        );
    }

    #[test]
    fn trace_conversion_caps_long_idle_gaps() {
        let events = [
            TraceEvent {
                t_ns: 0,
                action: "a::B".into(),
                tab: None,
            },
            TraceEvent {
                t_ns: 60_000_000_000,
                action: "a::C".into(),
                tab: None,
            },
        ];

        let scenario = trace_to_scenario("recorded", &events, 2_000);
        assert!(
            scenario.steps.contains(&Step::Wait { ms: 2_000 }),
            "a minute of the user reading should not become a minute of benchmark: {:?}",
            scenario.steps
        );
    }
}

#[cfg(test)]
mod shipped_scenario_tests {
    use super::Scenario;

    /// Every scenario in `scenarios/` parses and has balanced marks.
    ///
    /// These files are only exercised by a real run, which needs a built app and
    /// a generated corpus — so without this a typo in a `settle` block, or a
    /// `mark` left open, is discovered minutes into a measurement rather than in
    /// a second here.
    #[test]
    fn the_shipped_scenarios_parse_and_balance_their_marks() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
        let mut checked = 0;

        for entry in std::fs::read_dir(&directory).expect("the scenarios directory exists") {
            let path = entry.expect("a readable directory entry").path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }

            let scenario = Scenario::load(&path)
                .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
            scenario
                .validate_marks()
                .unwrap_or_else(|error| panic!("{} has unbalanced marks: {error}", path.display()));
            assert!(
                !scenario.steps.is_empty(),
                "{} has no steps",
                path.display()
            );
            checked += 1;
        }

        assert!(checked > 0, "no scenarios were found to check");
    }
}
