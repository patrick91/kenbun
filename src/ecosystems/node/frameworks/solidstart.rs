//! SolidStart identity detection.

use super::Context;

pub(super) fn detect(context: &mut Context<'_>) {
    context.add_dependency("solidstart", "@solidjs/start");
}
