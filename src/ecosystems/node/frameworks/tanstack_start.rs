//! TanStack Start identity detection.

use super::Context;

pub(super) fn detect(context: &mut Context<'_>) {
    context.add_dependency("tanstack-start", "@tanstack/react-start");
    context.add_dependency("tanstack-start", "@tanstack/solid-start");
    context.add_dependency("tanstack-start", "@tanstack/start");
}
