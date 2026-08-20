//! `milim-storage` — SQLite persistence + at-rest encryption.
//!
//! Phase 2 foundation for the harness. Provides a [`Database`] wrapper over
//! bundled SQLite with an ordered [`Migration`] runner, and an
//! [`EncryptedStore`] (AES-256-GCM) used by [`SecretKv`] to keep API keys,
//! OAuth tokens, and agent secrets encrypted at rest.

mod crypto;
mod db;
mod private_file;

pub use crypto::EncryptedStore;
pub use db::{
    ControlApprovalRecord, ControlBackupState, ControlCommandReceiptRecord, ControlHostRecord,
    ControlInboxRecord, ControlMailboxRecord, ControlQueuedTurnRecord, ControlRunArtifactRecord,
    ControlRunEventRecord, ControlRunRecord, ControlThreadLinkRecord, ControlThreadRecord,
    ControlTimelinePage, ControlTimelineRecord, Database, DatabaseOptions, JournalMode, Migration,
    SecretKv, SessionsDelta, UserDataStore, SECRETS_MIGRATIONS,
};
pub use private_file::create_private_file;
