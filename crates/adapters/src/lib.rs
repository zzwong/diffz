//! Side-effect boundaries. Native widgets never receive credentials, SQL or process handles.
pub mod error;
pub mod export;
pub mod fixtures;
pub mod github;
pub mod local_git;
pub mod outbox;
pub mod process;
pub mod provider;
pub mod service;
pub mod store;
pub use error::{AdapterError, Result};

pub mod gitlab;
