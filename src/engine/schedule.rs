use std::collections::{HashMap, HashSet, VecDeque};

use tokio::time::Instant;

use crate::config::Invocation;

/// Tracks execution slots and accepted requests in submission order.
#[derive(Default)]
pub(super) struct Schedule {
    groups: HashMap<String, usize>,
    locks: HashSet<String>,
    pub(super) pending: VecDeque<Pending>,
}

pub(super) struct Pending {
    pub(super) id: i64,
    pub(super) invocation: Invocation,
    pub(super) deadline: Instant,
}

#[derive(Debug, PartialEq)]
pub(super) enum Admission {
    Running,
    Queued,
    Full,
}

#[derive(Debug, PartialEq)]
pub(super) enum Transition {
    Start,
    Expire,
}

impl Schedule {
    pub(super) fn admit(&self, invocation: &Invocation, max_pending: usize) -> Admission {
        if self.available(invocation) {
            Admission::Running
        } else if self.pending.len() < max_pending {
            Admission::Queued
        } else {
            Admission::Full
        }
    }

    /// Choose the oldest request that has expired or can acquire all its slots.
    pub(super) fn next(&self, now: Instant) -> Option<(usize, Transition)> {
        self.pending.iter().enumerate().find_map(|(index, run)| {
            if run.deadline <= now {
                Some((index, Transition::Expire))
            } else if self.available(&run.invocation) {
                Some((index, Transition::Start))
            } else {
                None
            }
        })
    }

    fn available(&self, invocation: &Invocation) -> bool {
        invocation.group_limit.as_ref().is_none_or(|group| {
            self.groups.get(&group.id).copied().unwrap_or(0) < group.max_concurrent.get()
        }) && invocation
            .lock
            .as_ref()
            .is_none_or(|key| !self.locks.contains(key))
    }

    pub(super) fn reserve(&mut self, invocation: &Invocation) {
        if let Some(group) = &invocation.group_limit {
            *self.groups.entry(group.id.clone()).or_default() += 1;
        }
        if let Some(key) = &invocation.lock {
            self.locks.insert(key.clone());
        }
    }

    pub(super) fn release(&mut self, invocation: &Invocation) {
        if let Some(group) = &invocation.group_limit {
            let count = self.groups.get_mut(&group.id).expect("reserved group slot");
            *count -= 1;
            if *count == 0 {
                self.groups.remove(&group.id);
            }
        }
        if let Some(key) = &invocation.lock {
            self.locks.remove(key);
        }
    }
}

#[cfg(test)]
mod tests;
