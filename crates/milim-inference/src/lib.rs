//! `milim-inference` - the backend-neutral inference layer.
//!
//! Defines [`ModelService`], the trait every backend implements, plus the
//! backend-neutral request/stream types the server translates wire formats
//! into. Ships runtime backends plus an opt-in deterministic test backend:
//!   - `test_backend::TestBackend` - deterministic, no native deps (feature
//!     `test-backend`, for tests).
//!   - [`remote::RemoteBackend`] - OpenAI-compatible upstream passthrough.
//!   - [`unavailable::UnavailableBackend`] - explicit runtime fallback when no
//!     real model service is configured.
//!
//! Local model runtimes are integrated through provider APIs outside this crate.

pub mod anthropic;
pub mod gemini;
mod http_error;
mod image_input;
pub mod remote;
pub mod service;
#[cfg(any(test, feature = "test-backend"))]
pub mod test_backend;
pub mod unavailable;

pub use service::{
    CompletionOutput, CompletionRequest, DeltaEvent, EventStream, ModelService, SamplingParams,
    SharedService, StreamEvent, ToolCallAccumulator,
};
