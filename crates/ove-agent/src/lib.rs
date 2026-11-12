//! The bring-your-own-key assistant: an Anthropic Messages API client (streaming, retries, prompt caching), the tools
//! that read and edit the open project through its timeline projection, the turn loop with budgets and
//! cancellation, chat logs in the project bundle, and API key storage in the OS credential store.
// No unsafe code in the library; one unit test changes the environment, which needs `unsafe`.
#![cfg_attr(not(test), forbid(unsafe_code))]
#![cfg_attr(test, deny(unsafe_code))]

pub mod agent;
pub mod api;
pub mod chat;
pub mod keys;
pub mod mock;
pub mod prompt;
pub mod tools;

pub use agent::{AgentEvent, ProjectCap, TurnConfig, TurnSummary, run_turn};
pub use api::{ApiError, Client, MODELS, ModelInfo, Usage};
pub use chat::{ChatStore, ChatSummary, Record, Spend};
pub use keys::{KeySource, KeyStore, MemoryKeyStore, OsKeyStore};
pub use tools::{ApplyRequest, ToolHost};
