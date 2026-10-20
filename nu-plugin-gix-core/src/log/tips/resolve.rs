//! Resolves where a commit walk starts, from `HEAD`, revisions and references, into the commits it includes and hides.

use super::lookup::{commit_id, commit_or_none, parent_ids};
use crate::GixError;
use crate::log::options::LogStart;

use gix::head::Kind as HeadKind;
use gix::revision::plumbing::Spec;

/// The commits a walk starts from and the commits whose ancestry it leaves out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::log) struct Tips {
    /// Commits the walk starts from, sorted and without duplicates.
    pub include: Vec<gix::ObjectId>,
    /// Commits that are not listed, nor are their ancestors; sorted and without duplicates.
    pub hide: Vec<gix::ObjectId>,
}

impl Tips {
    /// Resolves where a walk over `repo` starts: from `HEAD` when `start` names no revision and is
    /// not `all`, from its revisions, and from every reference and `HEAD` when it is `all`.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::UnbornHead`] when `HEAD` names a branch without commits and
    /// [`GixError::Head`] when `HEAD` cannot be read (both only when `HEAD` is the start);
    /// [`GixError::Revision`] when a revision does not resolve and [`GixError::NotACommit`] when it
    /// names a tree or blob; and [`GixError::RefRead`] when the reference store cannot be read for
    /// `all`. Single references that cannot be read or peeled are skipped.
    pub(in crate::log) fn resolve(
        repo: &gix::Repository,
        start: &LogStart,
    ) -> Result<Self, GixError> {
        let mut tips = Self::default();

        if start.revisions.is_empty() && !start.all {
            tips.add_head(repo)?;
        }

        if start.all {
            tips.add_references(repo)?;
        }

        for expression in &start.revisions {
            tips.add_revision(repo, expression)?;
        }

        tips.include.sort_unstable();
        tips.include.dedup();
        tips.hide.sort_unstable();
        tips.hide.dedup();

        Ok(tips)
    }

    /// Adds the commit `HEAD` points at.
    fn add_head(&mut self, repo: &gix::Repository) -> Result<(), GixError> {
        let head = repo.head().map_err(GixError::Head)?;

        if let HeadKind::Unborn(name) = &head.kind {
            return Err(GixError::UnbornHead {
                name: name.to_string(),
            });
        }

        self.include
            .push(head.into_peeled_id().map_err(GixError::Head)?.detach());

        Ok(())
    }

    /// Adds the commits a revision expression includes and hides.
    fn add_revision(&mut self, repo: &gix::Repository, expression: &str) -> Result<(), GixError> {
        let unresolved = |source| GixError::Revision {
            expression: expression.to_owned(),
            source,
        };

        let commit = |id| commit_id(repo, id, expression);

        match repo.rev_parse(expression).map_err(unresolved)?.detach() {
            Spec::Include(id) => self.include.push(commit(id)?),
            Spec::Exclude(id) => self.hide.push(commit(id)?),
            Spec::Range { from, to } => {
                self.hide.push(commit(from)?);
                self.include.push(commit(to)?);
            }
            Spec::Merge { theirs, ours } => {
                let theirs = commit(theirs)?;
                let ours = commit(ours)?;
                let bases = repo.merge_bases_many(theirs, &[ours]).map_err(unresolved)?;

                self.hide.extend(bases.into_iter().map(gix::Id::detach));
                self.include.extend([theirs, ours]);
            }
            Spec::IncludeOnlyParents(id) => {
                self.include
                    .extend(parent_ids(repo, commit(id)?).map_err(unresolved)?);
            }
            Spec::ExcludeParents(id) => {
                let id = commit(id)?;

                self.include.push(id);
                self.hide.extend(parent_ids(repo, id).map_err(unresolved)?);
            }
        }
        Ok(())
    }

    /// Adds every reference that peels to a commit, and `HEAD` when it does; a reference that cannot
    /// be read, peeled or looked up is skipped, like an unborn `HEAD`.
    fn add_references(&mut self, repo: &gix::Repository) -> Result<(), GixError> {
        let platform = repo.references().map_err(GixError::RefRead)?;
        let references = platform.all().map_err(GixError::RefRead)?;

        self.include.extend(references.filter_map(|reference| {
            let id = reference.ok()?.peel_to_id().ok()?.detach();

            commit_or_none(repo, id)
        }));

        let head = repo.head().ok().and_then(|head| head.into_peeled_id().ok());

        self.include
            .extend(head.and_then(|id| commit_or_none(repo, id.detach())));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::GixError;
    use crate::fixtures::{open_fixture, open_path};

    use gix::refs::transaction::PreviousValue;
    use nu_plugin_gix_testing_support::{
        ANNOTATED_TAG, DivergedTestRepo, FixtureError, FixtureRepo as _, ForkedTestRepo, GIT_DIR,
        MAIN_BRANCH, TaggedTestRepo, TestRepo, UnbornTestRepo, diverged_repo, forked_repo,
        tagged_repo, test_repo, unborn_repo,
    };
    use rstest::rstest;

    use std::{assert_matches, fs, io, path::Path};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// Reference that [`NonCommit::Tree`] is written to, so it can be peeled and skipped by `All`.
    const TREE_TAG_REF: &str = "refs/tags/tree";
    /// Reflog message for the reference written by the tests.
    const REFLOG_MESSAGE: &str = "test";
    const TREE_KIND: &str = "tree";
    const BLOB_KIND: &str = "blob";
    const DANGLING_CONTENT: &str = "ref: refs/heads/missing\n";
    const MISSING_OBJECT_CONTENT: &str = "0123456789abcdef0123456789abcdef01234567\n";
    const UNPARSABLE_CONTENT: &str = "not a reference\n";
    /// Reference written beside `main` and `other`, so that it sorts between them; the tests check
    /// that iteration continues past it.
    const BROKEN_REF: &str = "refs/heads/n-broken";

    /// Resolves `start` in the repository at `path`.
    fn resolve_at(path: &Path, start: &LogStart) -> Result<Tips, TestError> {
        Ok(Tips::resolve(&open_path(path)?.repo(), start)?)
    }

    /// The error that resolving `start` in the repository at `path` reports.
    fn resolve_error(path: &Path, start: &LogStart) -> Result<GixError, TestError> {
        Tips::resolve(&open_path(path)?.repo(), start)
            .err()
            .ok_or_else(|| TestError::Failure("expected an error".to_owned()))
    }

    fn revision(expression: &str) -> LogStart {
        LogStart {
            revisions: vec![expression.to_owned()],
            all: false,
        }
    }

    const fn all() -> LogStart {
        LogStart {
            revisions: Vec::new(),
            all: true,
        }
    }

    fn sorted_hex(ids: &[gix::ObjectId]) -> Vec<String> {
        let mut hex: Vec<String> = ids.iter().map(ToString::to_string).collect();

        hex.sort();
        hex
    }

    fn sorted_strings(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }

    /// A commit of [`ForkedTestRepo`], named by its role in the fork.
    #[derive(Clone, Copy, Debug)]
    enum Fork {
        Merge,
        FirstParent,
        SecondParent,
        Root,
    }

    impl Fork {
        fn hex(self, repo: &ForkedTestRepo) -> String {
            match self {
                Self::Merge => repo.head.clone(),
                Self::FirstParent => repo.first_parent.clone(),
                Self::SecondParent => repo.second_parent.clone(),
                Self::Root => repo.root.clone(),
            }
        }
    }

    fn sorted(forks: &[Fork], repo: &ForkedTestRepo) -> Vec<String> {
        sorted_strings(forks.iter().map(|fork| fork.hex(repo)).collect())
    }

    /// An object of [`TaggedTestRepo`] that is not a commit.
    #[derive(Clone, Copy, Debug)]
    enum NonCommit {
        Tree,
        Blob,
    }

    impl NonCommit {
        fn id(self, fixture: &TaggedTestRepo) -> gix::ObjectId {
            match self {
                Self::Tree => fixture.tree,
                Self::Blob => fixture.blob,
            }
        }

        fn kind(self) -> &'static str {
            match self {
                Self::Tree => TREE_KIND,
                Self::Blob => BLOB_KIND,
            }
        }
    }

    /// A reference that `All` must skip because it cannot be read or peeled to a commit.
    #[derive(Clone, Copy, Debug)]
    enum BrokenRef {
        DanglingSymbolic,
        MissingObject,
        Unparsable,
    }

    impl BrokenRef {
        fn content(self) -> &'static str {
            match self {
                Self::DanglingSymbolic => DANGLING_CONTENT,
                Self::MissingObject => MISSING_OBJECT_CONTENT,
                Self::Unparsable => UNPARSABLE_CONTENT,
            }
        }
    }

    mod resolve {
        use super::*;

        #[rstest]
        fn starts_from_head_by_default(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let tips = resolve_at(repo.path(), &LogStart::default())?;

            assert_eq!(sorted_hex(&tips.include), vec![repo.head]);
            assert_eq!(tips.hide, Vec::<gix::ObjectId>::new());

            Ok(())
        }

        #[rstest]
        fn reports_an_unborn_head(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = unborn_repo?;
            let err = resolve_error(repo.path(), &LogStart::default())?;

            assert_matches!(
                &err,
                GixError::UnbornHead { name } if name == MAIN_BRANCH,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        #[case::include("HEAD", &[Fork::Merge], &[])]
        #[case::exclude("^HEAD", &[], &[Fork::Merge])]
        #[case::range("HEAD^1..HEAD", &[Fork::Merge], &[Fork::FirstParent])]
        #[case::symmetric_difference("HEAD^1...HEAD^2", &[Fork::FirstParent, Fork::SecondParent], &[Fork::Root])]
        #[case::only_parents("HEAD^@", &[Fork::FirstParent, Fork::SecondParent], &[])]
        #[case::without_parents("HEAD^!", &[Fork::Merge], &[Fork::FirstParent, Fork::SecondParent])]
        fn resolves_each_kind_of_revision(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
            #[case] expression: &str,
            #[case] include: &[Fork],
            #[case] hide: &[Fork],
        ) -> Result<(), TestError> {
            let repo = forked_repo?;
            let tips = resolve_at(repo.path(), &revision(expression))?;

            assert_eq!(sorted_hex(&tips.include), sorted(include, &repo));
            assert_eq!(sorted_hex(&tips.hide), sorted(hide, &repo));

            Ok(())
        }

        #[rstest]
        fn peels_an_annotated_tag_to_its_commit(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let tips = resolve_at(fixture.path(), &revision(ANNOTATED_TAG))?;

            assert_eq!(tips.include, vec![fixture.commit]);

            Ok(())
        }

        #[rstest]
        fn rejects_a_revision_that_does_not_resolve(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let err = resolve_error(fixture.path(), &revision("not-a-revision"))?;

            assert_matches!(&err, GixError::Revision { .. }, "got {err:?}");

            Ok(())
        }

        #[rstest]
        #[case::tree(NonCommit::Tree)]
        #[case::blob(NonCommit::Blob)]
        fn rejects_a_revision_that_names_a_tree_or_blob(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
            #[case] non_commit: NonCommit,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let expression = non_commit.id(&fixture).to_string();
            let err = resolve_error(fixture.path(), &revision(&expression))?;

            assert_matches!(
                &err,
                GixError::NotACommit { kind, .. } if kind == non_commit.kind(),
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn all_starts_from_every_reference_and_head(
            diverged_repo: Result<DivergedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = diverged_repo?;
            let tips = resolve_at(repo.path(), &all())?;

            assert_eq!(
                sorted_hex(&tips.include),
                sorted_strings(vec![repo.head, repo.other])
            );

            Ok(())
        }

        #[rstest]
        fn all_combines_with_a_hidden_revision(
            diverged_repo: Result<DivergedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = diverged_repo?;

            let start = LogStart {
                revisions: vec!["^main".to_owned()],
                all: true,
            };

            let tips = resolve_at(repo.path(), &start)?;

            assert_eq!(
                sorted_hex(&tips.include),
                sorted_strings(vec![repo.head.clone(), repo.other])
            );

            assert_eq!(sorted_hex(&tips.hide), vec![repo.head]);

            Ok(())
        }

        #[rstest]
        fn unites_what_several_revisions_include_and_hide(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;

            let start = LogStart {
                revisions: vec![
                    "HEAD^1".to_owned(),
                    "HEAD^2".to_owned(),
                    "^HEAD^1^1".to_owned(),
                ],
                all: false,
            };

            let tips = resolve_at(repo.path(), &start)?;

            assert_eq!(
                sorted_hex(&tips.include),
                sorted(&[Fork::FirstParent, Fork::SecondParent], &repo)
            );

            assert_eq!(sorted_hex(&tips.hide), sorted(&[Fork::Root], &repo));

            Ok(())
        }

        #[rstest]
        fn all_skips_references_to_non_commits(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let tips = resolve_at(fixture.path(), &all())?;

            open_fixture(&fixture)?.repo().reference(
                TREE_TAG_REF,
                fixture.tree,
                PreviousValue::MustNotExist,
                REFLOG_MESSAGE,
            )?;

            assert_eq!(tips.include, vec![fixture.commit]);

            Ok(())
        }

        #[rstest]
        #[case::dangling_symbolic(BrokenRef::DanglingSymbolic)]
        #[case::missing_object(BrokenRef::MissingObject)]
        #[case::unparsable(BrokenRef::Unparsable)]
        fn all_skips_references_that_cannot_be_read_or_peeled(
            diverged_repo: Result<DivergedTestRepo, FixtureError>,
            #[case] broken: BrokenRef,
        ) -> Result<(), TestError> {
            let repo = diverged_repo?;

            fs::write(repo.path().join(GIT_DIR).join(BROKEN_REF), broken.content())?;

            let tips = resolve_at(repo.path(), &all())?;

            assert_eq!(
                sorted_hex(&tips.include),
                sorted_strings(vec![repo.head, repo.other])
            );

            Ok(())
        }

        #[rstest]
        fn all_in_an_unborn_repository_starts_nowhere(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = unborn_repo?;
            let tips = resolve_at(repo.path(), &all())?;

            assert_eq!(tips.include, Vec::<gix::ObjectId>::new());
            assert_eq!(tips.hide, Vec::<gix::ObjectId>::new());

            Ok(())
        }
    }
}
