//! forgetop core: provider-neutral domain model, capability-scoped provider traits,
//! config + bindings, secret store, and the runtime services.

pub mod config;
pub mod domain;
pub mod error;
pub mod filter;
pub mod provider;
pub mod secret;
pub mod service;

pub use error::{Error, Result};
