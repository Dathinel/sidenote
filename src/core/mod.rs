//! Meeting data and background work, with no Rune or other UI dependencies.
pub mod agent_instructions;
pub mod annotations;
pub mod area_ops;
pub mod area_version;
pub mod areas;
pub mod ask;
pub mod ask_all;
pub mod ask_options;
pub mod ask_session;
pub mod asr;
pub mod check_state;
pub mod config;
pub mod export;
pub mod meeting_router;
pub mod meeting_watch;
pub mod recording_guard;
pub use config::{Config, Feature, Features};
pub mod check_v2;
pub mod daily_check;
pub mod diary;
pub mod memory;
pub mod notes;
mod paths;
pub mod project_merge;
pub mod settle;
pub use paths::{configure_paths, is_proof_path};
pub mod agent_tools;
mod device;
pub mod file_reads;
pub mod live_edit;
pub mod mcp_connect;
pub use sync_state::{EditView, mutation_generation};
mod memory_migration;
pub(crate) mod store_io;
mod sync_state;
pub use daily_check::{apply_action, company_memory, latest_check, list_checks, load_check};
pub use device::DeviceSettings;
mod grouping;
pub mod live;
mod meetings;
mod persistence;
mod runner_process;
pub mod runners;
mod search;
mod tasks;
mod ui_prefs;
pub use ask::{ask, ask_with_progress};
pub use grouping::*;
pub use meetings::*;
pub use persistence::*;
pub use runner_process::run_runner;
pub use runners::{AskProgress, Runner, last_used_runner, list_runners};
pub use search::*;
pub use tasks::*;
pub use ui_prefs::UiPrefs;

pub mod model_catalog;
mod model_catalog_data;

pub fn validation_message(error: &std::io::Error) -> Option<&str> {
    store_io::message(error)
}

pub mod companies;
pub mod people;
pub mod tasks_v2;
pub use tasks_v2::*;

pub mod task_titles;
