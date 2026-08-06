//! Flask identity detection.

use super::{dependency_evidence, Context, DetectorResult};

const DEPENDENCIES: &[&str] = &["flask"];

pub(super) fn matches_dependency(name: &str) -> bool {
    DEPENDENCIES.contains(&name)
}

pub(super) fn detect(context: &Context<'_>) -> DetectorResult {
    let dependencies = context.dependencies.named("flask");
    let mut result = DetectorResult {
        framework: (!dependencies.is_empty()).then_some("flask"),
        evidence: dependencies.iter().map(dependency_evidence).collect(),
        ..DetectorResult::default()
    };
    result.evidence.extend(context.setup_evidence(DEPENDENCIES));
    result
}
