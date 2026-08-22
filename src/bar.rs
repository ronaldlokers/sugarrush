//! The JSON line a bar's custom module reads.
//!
//! Kept as a thin wrapper over [`crate::status`] so `sugarrush waybar` — which
//! the shipped Waybar examples and anyone's existing config call — keeps
//! working unchanged. The format is `Format::Json`; the subcommand keeps its
//! original name because configs in the wild spell it that way.

use crate::config::Config;
use crate::status::{self, Format};

/// Fetch the last hour and render the JSON line. Always returns valid JSON,
/// even on error, so the bar has something to show.
pub async fn line(cfg: &Config) -> String {
    status::status(cfg).await.render(Format::Json)
}
