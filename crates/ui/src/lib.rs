//! Reading surface and application chrome. Provider, SQL, and process code stay out of this crate.
mod app;
mod commands;
pub mod native_text;
mod panels;
pub mod probe;
mod reader;
pub mod theme;
pub mod viewport;
pub use app::{LaunchOptions, launch};

mod chrome;
pub mod icons;
mod keyboard;

mod review_panels;
pub(crate) mod rich_view;
