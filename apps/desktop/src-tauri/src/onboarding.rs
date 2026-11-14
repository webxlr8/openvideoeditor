//! Onboarding state: whether the welcome was finished or skipped, the assistant chosen there, which steps of the
//! guided first edit are done, and whether its checklist is closed.
//!
//! The native side keeps it in the app's configuration folder (`onboarding.json`, next to `window-state.json`), not
//! in the web profile: clearing web data does not bring the welcome back, and every window sees the same state.
//! A missing or unreadable file means "first run"; nothing here is ever an error shown to the person.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::error::AppResult;
use crate::worker::{self, relock};

/// File name inside the app's configuration folder.
pub const FILE_NAME: &str = "onboarding.json";
/// Format of the file. Raise it when a field changes meaning; a file from a newer app is read as far as it parses.
pub const VERSION: u32 = 1;

/// Where the welcome stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Welcome {
    /// Never finished or skipped: shown at launch.
    #[default]
    Pending,
    Finished,
    Skipped,
}

/// A step of the guided first edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ChecklistStep {
    Import,
    Cut,
    Title,
    Captions,
    Export,
}

/// What `onboarding.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct OnboardingState {
    pub version: u32,
    pub welcome: Welcome,
    /// The assistant picked in the welcome: an agent's id, `"api-key"`, or `None` for "I'll edit by hand".
    pub assistant: Option<String>,
    /// Steps of the guided first edit the person has done, in checklist order.
    pub steps_done: Vec<ChecklistStep>,
    /// The checklist is out of the way until Help → Getting started brings it back.
    pub checklist_closed: bool,
}

impl Default for OnboardingState {
    fn default() -> Self {
        OnboardingState {
            version: VERSION,
            welcome: Welcome::Pending,
            assistant: None,
            steps_done: Vec::new(),
            checklist_closed: false,
        }
    }
}

/// One change to the state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OnboardingChange {
    /// The welcome was walked to its end with this assistant chosen (`None`: edit by hand).
    FinishWelcome { assistant: Option<String> },
    /// Skip was pressed on a welcome screen.
    SkipWelcome,
    /// The assistant was chosen somewhere else (Settings, the chat panel).
    SetAssistant { assistant: Option<String> },
    /// Checklist steps the project or the app showed as done.
    CompleteSteps { steps: Vec<ChecklistStep> },
    /// The checklist was closed, or reopened from Help.
    SetChecklistClosed { closed: bool },
}

impl OnboardingState {
    /// Applies a change; true when the state is different afterwards.
    pub fn apply(&mut self, change: OnboardingChange) -> bool {
        let before = self.clone();
        match change {
            OnboardingChange::FinishWelcome { assistant } => {
                self.welcome = Welcome::Finished;
                self.assistant = assistant.filter(|a| !a.trim().is_empty());
            }
            // Skipping the first run also keeps the checklist out of the way: the person asked for no guidance.
            // Skipping a welcome reopened from Help changes nothing.
            OnboardingChange::SkipWelcome => {
                if self.welcome == Welcome::Pending {
                    self.welcome = Welcome::Skipped;
                    self.checklist_closed = true;
                }
            }
            OnboardingChange::SetAssistant { assistant } => {
                self.assistant = assistant.filter(|a| !a.trim().is_empty());
            }
            OnboardingChange::CompleteSteps { steps } => self.steps_done.extend(steps),
            OnboardingChange::SetChecklistClosed { closed } => self.checklist_closed = closed,
        }
        self.normalise();
        *self != before
    }

    /// Each step once, in checklist order; the version this app writes.
    fn normalise(&mut self) {
        self.steps_done.sort();
        self.steps_done.dedup();
        self.version = VERSION;
    }
}

/// Reads the state; a missing, unreadable or malformed file is a first run.
pub fn load(path: &Path) -> OnboardingState {
    let Ok(text) = std::fs::read_to_string(path) else { return OnboardingState::default() };
    match serde_json::from_str::<OnboardingState>(&text) {
        Ok(mut state) => {
            state.normalise();
            state
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "onboarding state is unreadable; starting as a first run");
            OnboardingState::default()
        }
    }
}

/// Writes the state atomically (temporary file, then rename), so a crash never leaves half a file.
pub fn save(path: &Path, state: &OnboardingState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

/// The state and its file. Managed by Tauri; read from disk on first use.
pub struct Onboarding {
    /// `None` when the platform gave no configuration folder: the state then lasts until the app closes.
    path: Option<PathBuf>,
    state: Mutex<Option<OnboardingState>>,
}

impl Onboarding {
    pub fn new(config_dir: Option<PathBuf>) -> Self {
        Onboarding { path: config_dir.map(|d| d.join(FILE_NAME)), state: Mutex::new(None) }
    }

    pub fn get(&self) -> OnboardingState {
        let mut g = relock(&self.state);
        g.get_or_insert_with(|| self.path.as_deref().map(load).unwrap_or_default()).clone()
    }

    /// Applies a change and saves it. A failed save is logged and the change still holds for this run.
    pub fn update(&self, change: OnboardingChange) -> OnboardingState {
        let mut g = relock(&self.state);
        let state = g.get_or_insert_with(|| self.path.as_deref().map(load).unwrap_or_default());
        if state.apply(change)
            && let Some(path) = &self.path
            && let Err(e) = save(path, state)
        {
            tracing::warn!(path = %path.display(), error = %e, "could not save onboarding state");
        }
        state.clone()
    }
}

/// The onboarding state (a first run when there is none yet).
#[tauri::command]
#[specta::specta]
pub async fn onboarding_state(app: AppHandle) -> AppResult<OnboardingState> {
    worker::blocking("onboarding_state", move || Ok(app.try_state::<Onboarding>().map(|o| o.get()).unwrap_or_default()))
        .await
}

/// Changes the onboarding state and returns it as it is now.
#[tauri::command]
#[specta::specta]
pub async fn onboarding_update(app: AppHandle, change: OnboardingChange) -> AppResult<OnboardingState> {
    worker::blocking("onboarding_update", move || {
        Ok(app.try_state::<Onboarding>().map(|o| o.update(change)).unwrap_or_default())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_a_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let state = load(&dir.path().join(FILE_NAME));
        assert_eq!(state, OnboardingState::default());
        assert_eq!(state.welcome, Welcome::Pending);
        assert!(!state.checklist_closed);
        // Reading does not create the file.
        assert!(!dir.path().join(FILE_NAME).exists());
    }

    #[test]
    fn a_corrupt_file_is_a_first_run_and_is_replaced_by_the_next_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        for junk in ["", "not json", "[1, 2]", r#"{"welcome": 7}"#, r#"{"stepsDone": ["dance"]}"#] {
            std::fs::write(&path, junk).unwrap();
            assert_eq!(load(&path), OnboardingState::default(), "{junk:?}");
        }
        let store = Onboarding::new(Some(dir.path().to_path_buf()));
        assert_eq!(store.get().welcome, Welcome::Pending);
        store.update(OnboardingChange::SkipWelcome);
        assert_eq!(load(&path).welcome, Welcome::Skipped);
    }

    #[test]
    fn state_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        // A configuration folder that does not exist yet is created by the first save.
        let config = dir.path().join("config").join("app");
        let store = Onboarding::new(Some(config.clone()));
        store.update(OnboardingChange::FinishWelcome { assistant: Some("claude".into()) });
        store.update(OnboardingChange::CompleteSteps { steps: vec![ChecklistStep::Title, ChecklistStep::Import] });
        store.update(OnboardingChange::CompleteSteps { steps: vec![ChecklistStep::Import, ChecklistStep::Cut] });
        let state = store.update(OnboardingChange::SetChecklistClosed { closed: true });
        let expected = OnboardingState {
            version: VERSION,
            welcome: Welcome::Finished,
            assistant: Some("claude".into()),
            steps_done: vec![ChecklistStep::Import, ChecklistStep::Cut, ChecklistStep::Title],
            checklist_closed: true,
        };
        assert_eq!(state, expected);
        // A new launch reads the same state.
        assert_eq!(Onboarding::new(Some(config.clone())).get(), expected);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config.join(FILE_NAME)).unwrap()).unwrap();
        assert_eq!(json["version"], VERSION);
        assert_eq!(json["welcome"], "finished");
        assert_eq!(json["stepsDone"], serde_json::json!(["import", "cut", "title"]));
        assert!(!config.join("onboarding.json.tmp").exists());
    }

    #[test]
    fn a_file_with_fields_missing_keeps_what_it_has() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, r#"{"welcome": "skipped", "somethingNewer": true}"#).unwrap();
        let state = load(&path);
        assert_eq!(state.welcome, Welcome::Skipped);
        assert_eq!(state.assistant, None);
        assert!(state.steps_done.is_empty());
    }

    #[test]
    fn skip_closes_the_checklist_on_a_first_run_only() {
        let mut first = OnboardingState::default();
        assert!(first.apply(OnboardingChange::SkipWelcome));
        assert_eq!(first.welcome, Welcome::Skipped);
        assert!(first.checklist_closed);

        // The welcome reopened from Help and skipped: nothing changes.
        let mut done = OnboardingState::default();
        done.apply(OnboardingChange::FinishWelcome { assistant: Some("api-key".into()) });
        assert!(!done.checklist_closed);
        assert!(!done.apply(OnboardingChange::SkipWelcome));
        assert_eq!(done.welcome, Welcome::Finished);
        assert_eq!(done.assistant.as_deref(), Some("api-key"));

        // "I'll edit by hand", and an empty choice, are no assistant.
        done.apply(OnboardingChange::SetAssistant { assistant: Some("  ".into()) });
        assert_eq!(done.assistant, None);
    }

    #[test]
    fn without_a_configuration_folder_the_state_lasts_for_the_run() {
        let store = Onboarding::new(None);
        assert_eq!(store.get().welcome, Welcome::Pending);
        store.update(OnboardingChange::FinishWelcome { assistant: None });
        assert_eq!(store.get().welcome, Welcome::Finished);
    }
}
