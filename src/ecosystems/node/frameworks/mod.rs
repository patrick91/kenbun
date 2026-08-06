//! Explicit Node framework detector orchestration.
//!
//! Each framework owns its package names and configuration-file convention.
//! Calls remain concrete so their cost and ordering are visible here.

mod astro;
mod nextjs;
mod nuxt;
mod remix;
mod solidstart;
mod sveltekit;
mod tanstack_start;

use super::{add_signal, direct_dependency_evidence, PackageManifest, TechnologySignals};

pub(super) struct Context<'a> {
    manifest: &'a PackageManifest,
    signals: &'a mut TechnologySignals,
}

impl Context<'_> {
    pub(super) fn add_dependency(&mut self, id: &str, package: &str) {
        if let Some(evidence) = direct_dependency_evidence(self.manifest, package) {
            add_signal(self.signals, id, "framework", evidence);
        }
    }
}

pub(super) fn detect(manifest: &PackageManifest, signals: &mut TechnologySignals) {
    let mut context = Context { manifest, signals };

    astro::detect(&mut context);
    nextjs::detect(&mut context);
    nuxt::detect(&mut context);
    remix::detect(&mut context);
    solidstart::detect(&mut context);
    sveltekit::detect(&mut context);
    tanstack_start::detect(&mut context);
}

pub(super) fn is_config_file(name: &str) -> bool {
    astro::is_config_file(name)
        || nextjs::is_config_file(name)
        || nuxt::is_config_file(name)
        || remix::is_config_file(name)
        || sveltekit::is_config_file(name)
}

fn matches_config_prefix(name: &str, prefix: &str) -> bool {
    super::CONFIG_EXTENSIONS
        .iter()
        .any(|extension| name == format!("{prefix}.config.{extension}"))
}
