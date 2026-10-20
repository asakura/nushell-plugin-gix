//! Which commits followed edges reach while a simplified walk proceeds.
//!
//! The invariant: under simplification a commit is considered only after some followed edge has
//! reached it. Normally the child is walked first and has already followed the edge; the exception is
//! clock skew, where the parent is walked first and must wait.

use std::collections::{HashMap, HashSet};

/// Which commits followed edges reach as a simplified walk proceeds. A commit is listed only when a followed edge
/// reaches it; usually a child walked earlier has followed that edge. When commit times run backwards along a parent
/// edge, the walk can reach the commit first, so it waits, with its parents, until a child follows it. Commits that no
/// followed edge reaches stay in memory, with their parents, until the walk ends: a child walked later may still follow
/// them under clock skew, and the walk cannot know when none will.
#[derive(Debug, Default)]
pub(super) struct Reach {
    followed: HashSet<gix::ObjectId>,
    waiting: HashMap<gix::ObjectId, Vec<gix::ObjectId>>,
}

impl Reach {
    pub(super) fn new<I>(tips: I) -> Self
    where
        I: IntoIterator<Item = gix::ObjectId>,
    {
        Self {
            followed: tips.into_iter().collect(),
            waiting: HashMap::new(),
        }
    }

    /// Whether a followed edge has reached `id`; otherwise `id` waits with `parents`.
    pub(super) fn admit(&mut self, id: gix::ObjectId, parents: &[gix::ObjectId]) -> bool {
        if self.followed.contains(&id) {
            return true;
        }

        self.waiting.insert(id, parents.to_vec());

        false
    }

    /// Records a followed edge to `parent`; returns it with its parents when the walk reached it earlier and it waited.
    pub(super) fn follow(
        &mut self,
        parent: gix::ObjectId,
    ) -> Option<(gix::ObjectId, Vec<gix::ObjectId>)> {
        let waiting = self.waiting.remove(&parent);

        if waiting.is_none() {
            self.followed.insert(parent);
        }

        waiting.map(|parents| (parent, parents))
    }
}
