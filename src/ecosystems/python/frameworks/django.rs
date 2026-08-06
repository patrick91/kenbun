//! Django identity detection.

use super::{dependency_evidence, join, Context, DetectorResult};
use crate::model::Evidence;

const DEPENDENCIES: &[&str] = &["django"];

pub(super) fn matches_dependency(name: &str) -> bool {
    DEPENDENCIES.contains(&name)
}

pub(super) fn detect(context: &Context<'_>) -> DetectorResult {
    let dependencies = context.dependencies.named("django");
    let mut result = DetectorResult {
        framework: (!dependencies.is_empty()).then_some("django"),
        evidence: dependencies.iter().map(dependency_evidence).collect(),
        ..DetectorResult::default()
    };

    if dependencies.is_empty() {
        let manage_py = join(context.dir, "manage.py");
        if let Some(source) = context.fs.read_str(&manage_py) {
            if source.contains("DJANGO_SETTINGS_MODULE") {
                result.framework = Some("django");
                result.evidence.push(Evidence {
                    kind: "marker-file".to_string(),
                    path: manage_py,
                    span: None,
                    detail: "manage.py sets DJANGO_SETTINGS_MODULE".to_string(),
                });
            }
        }
    }

    result.evidence.extend(context.setup_evidence(DEPENDENCIES));
    result
}
