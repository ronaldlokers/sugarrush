//! Waybar custom-module output.
//!
//! Kept as a thin wrapper over [`crate::status`] so `sugarrush waybar` — which
//! the shipped Waybar examples and anyone's existing config call — keeps
//! working unchanged. New formats live in `status`.

use crate::config::Config;
use crate::status::{self, Format};

/// Fetch the last hour and render the Waybar JSON line. Always returns valid
/// JSON, even on error, so Waybar has something to show.
pub async fn line(cfg: &Config) -> String {
    status::status(cfg).await.render(Format::Waybar)
}
