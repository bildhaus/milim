//! `milim-core` — foundational types shared across milim.
//!
//! Ported from the Swift `MilimCore` package: configuration, on-disk paths,
//! the error type, and the OpenAI/Ollama API DTOs that define the public wire
//! contract. Kept dependency-light (no async runtime) so every other crate can
//! depend on it.

pub mod api;
pub mod config;
pub mod error;
pub mod paths;
pub mod proc;
pub mod provider_error;

pub use error::{Error, Result};
