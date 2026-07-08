//! Live integration suite — hits real provider APIs. Only compiled with
//! `--features integration`; each provider's tests skip when its credentials
//! aren't present, so a partial `.env` runs a partial suite.
//!
//! Run everything you have creds for:
//!   cargo test -p forgetop-providers --features integration -- --nocapture --test-threads=1
//!
//! Credentials come from the environment (a gitignored `.env` locally, or CI
//! secrets). Never hard-code or commit tokens.

#[macro_use]
mod harness;

mod github;
