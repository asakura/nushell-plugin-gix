//! Test-only helpers that open the shared fixture repositories.

use crate::repository::OpenRepository;

use gix::open::Options;
use nu_plugin_gix_testing_support::{FixtureRepo as _, TaggedTestRepo};

use std::path::Path;

/// Opens the repository at `path` with the isolated open options that the fixtures use.
pub fn open_path(path: &Path) -> Result<OpenRepository, gix::Error> {
    Ok(OpenRepository::from_repository(gix::open_opts(
        path,
        Options::isolated(),
    )?))
}

/// Opens the fixture repository with the isolated open options that the fixtures use.
pub fn open_fixture(fixture: &TaggedTestRepo) -> Result<OpenRepository, gix::Error> {
    open_path(fixture.path())
}
