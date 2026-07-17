//! Launchpad view adapter.
//!
//! The triage engine itself lives in [`forgetop_core::launchpad`] so the terminal UI and the web
//! dashboard share one set of rules. This module just re-exports it and maps the TUI's row types
//! onto the core inputs.

pub use forgetop_core::launchpad::{Bucket, Entry, EntryItem, EntryKind, Launchpad, Overflow, PrRole};

use crate::app::{PipeRow, PrRow, WiRow};

/// Builds the Launchpad rows from the TUI's aggregated feeds (see [`forgetop_core::launchpad::build`]).
pub fn build(prs_review: &[PrRow], prs_mine: &[PrRow], wis: &[WiRow], pipes: &[PipeRow]) -> Launchpad {
    use forgetop_core::launchpad::{PipeInput, PrInput, WiInput};

    let pr_input = |r: &PrRow| PrInput {
        connection_id: r.connection_id.clone(),
        connection: r.connection.clone(),
        provider: r.provider,
        pr: r.pr.clone(),
    };
    let review: Vec<PrInput> = prs_review.iter().map(pr_input).collect();
    let mine: Vec<PrInput> = prs_mine.iter().map(pr_input).collect();
    let wis: Vec<WiInput> = wis
        .iter()
        .map(|r| WiInput {
            connection_id: r.connection_id.clone(),
            connection: r.connection.clone(),
            provider: r.provider,
            wi: r.wi.clone(),
        })
        .collect();
    let pipes: Vec<PipeInput> = pipes
        .iter()
        .map(|r| PipeInput {
            connection_id: r.connection_id.clone(),
            connection: r.connection.clone(),
            provider: r.provider,
            run: r.run.clone(),
            definition_name: r.definition_name.clone(),
            awaiting_approval: r.awaiting_approval,
        })
        .collect();

    forgetop_core::launchpad::build(&review, &mine, &wis, &pipes)
}
