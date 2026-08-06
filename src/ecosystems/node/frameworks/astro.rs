//! Astro identity and configuration-file conventions.

use super::{matches_config_prefix, Context};

pub(super) fn detect(context: &mut Context<'_>) {
    context.add_dependency("astro", "astro");
}

pub(super) fn is_config_file(name: &str) -> bool {
    matches_config_prefix(name, "astro")
}
