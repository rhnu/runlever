use std::time::Duration;

use crate::config::GroupLimit;

use super::{Admission, Instant, Invocation, Pending, Schedule, Transition};

fn invocation(group: Option<(&str, usize)>, lock: Option<&str>) -> Invocation {
    Invocation {
        action: "action".into(),
        label: "Action".into(),
        target: "action".into(),
        selections: Default::default(),
        program: "tool".into(),
        args: Vec::new(),
        cwd: None,
        lock: lock.map(str::to_owned),
        group_limit: group.map(|(id, limit)| GroupLimit {
            id: id.into(),
            max_concurrent: limit.try_into().unwrap(),
        }),
    }
}

#[test]
fn a_group_of_one_serializes_actions_with_different_locks() {
    let first = invocation(Some(("serial", 1)), Some("project:a"));
    let second = invocation(Some(("serial", 1)), Some("project:b"));
    let mut schedule = Schedule::default();
    schedule.reserve(&first);
    assert_eq!(schedule.admit(&second, 1), Admission::Queued);
    schedule.release(&first);
    assert_eq!(schedule.admit(&second, 1), Admission::Running);
}

#[test]
fn parallel_capacity_and_parameter_locks_both_apply() {
    let first = invocation(Some(("parallel", 2)), Some("project:a"));
    let second = invocation(Some(("parallel", 2)), Some("project:b"));
    let third = invocation(Some(("parallel", 2)), Some("project:c"));
    let outside = invocation(None, Some("project:a"));
    let mut schedule = Schedule::default();
    schedule.reserve(&first);
    assert_eq!(schedule.admit(&second, 1), Admission::Running);
    assert_eq!(schedule.admit(&first, 1), Admission::Queued);
    assert_eq!(schedule.admit(&outside, 1), Admission::Queued);
    schedule.reserve(&second);
    assert_eq!(schedule.admit(&third, 1), Admission::Queued);
    schedule.release(&first);
    assert_eq!(schedule.admit(&first, 1), Admission::Running);
    assert_eq!(schedule.admit(&outside, 1), Admission::Running);
}

#[test]
fn a_full_queue_still_allows_requests_that_can_run_immediately() {
    let running = invocation(None, Some("occupied"));
    let available = invocation(None, None);
    let mut schedule = Schedule::default();
    schedule.reserve(&running);
    assert_eq!(schedule.admit(&running, 0), Admission::Full);
    assert_eq!(schedule.admit(&running, 1), Admission::Queued);
    schedule.pending.push_back(Pending {
        id: 2,
        invocation: invocation(None, Some("occupied")),
        deadline: Instant::now() + Duration::from_secs(3),
    });
    assert_eq!(schedule.admit(&running, 1), Admission::Full);
    assert_eq!(schedule.admit(&available, 1), Admission::Running);
}

#[test]
fn dispatch_chooses_the_oldest_eligible_request_and_expires_at_the_deadline() {
    let running = invocation(None, Some("occupied"));
    let now = Instant::now();
    let deadline = now + Duration::from_secs(3);
    let mut schedule = Schedule::default();
    schedule.reserve(&running);
    schedule.pending.extend([
        Pending {
            id: 2,
            invocation: invocation(None, Some("occupied")),
            deadline,
        },
        Pending {
            id: 3,
            invocation: invocation(None, None),
            deadline,
        },
        Pending {
            id: 4,
            invocation: invocation(None, None),
            deadline,
        },
    ]);
    assert_eq!(schedule.next(now), Some((1, Transition::Start)));
    schedule.release(&running);
    assert_eq!(schedule.next(now), Some((0, Transition::Start)));
    assert_eq!(schedule.next(deadline), Some((0, Transition::Expire)));
}
