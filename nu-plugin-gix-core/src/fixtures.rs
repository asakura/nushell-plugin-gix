//! Test-only helpers that open the shared fixture repositories.

use crate::repository::OpenRepository;

use gix::open::Options;
use nu_plugin_gix_testing_support::TaggedTestRepo;

/// Opens the fixture repository with the isolated open options that the fixtures use.
pub fn open_fixture(fixture: &TaggedTestRepo) -> Result<OpenRepository, gix::Error> {
    let repo = gix::open_opts(fixture.path(), Options::isolated())?;

    Ok(OpenRepository::from_repository(repo))
}
