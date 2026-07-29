//! Shared application-boundary markers for Python and JavaScript/TypeScript.

use std::collections::BTreeSet;

use crate::fileset::FileSet;

const PROJECT_MARKERS: &[&str] = &[
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "Pipfile",
    "setup.py",
    "manage.py",
];

/// Directories that own source files independently of their parent project.
pub(crate) fn project_directories(fs: &FileSet) -> BTreeSet<String> {
    PROJECT_MARKERS
        .iter()
        .flat_map(|marker| fs.dirs_with(marker))
        .collect()
}
