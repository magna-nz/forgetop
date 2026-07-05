//! forgetop core: provider-neutral domain model, capability-scoped provider traits,
//! and (from Wave 2) config/secrets/services.

pub mod domain;
pub mod error;
pub mod provider;

pub use error::{Error, Result};
