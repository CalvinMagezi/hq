//! The one next step for a focus lane, chosen by rules and no model: a decision only the owner
//! can make, then work that is ready, then work to resume, then one hygiene item. A view over a
//! snapshot of the lane's tasks that never writes state, so the same snapshot always gives the
//! same step and every step carries the ids it rests on.

/// One task of a lane, as the rules need it. Built by the caller from the task store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneTask {
    pub id: String,
    pub title: String,
    pub status: LaneStatus,
    /// 0 is most urgent.
    pub priority: u8,
    /// `YYYY-MM-DD`, compared as text.
    pub due: Option<String>,
    pub age_days: i64,
    pub owner_must_decide: bool,
    pub dependencies_done: bool,
    pub has_lease: bool,
    pub has_checkpoint: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneStatus {
    ToDo,
    InProgress,
    Blocked,
    ReadyForReview,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKind {
    OwnerDecision,
    Ready,
    Resume,
    Hygiene,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    pub task_id: String,
    pub title: String,
    /// Ids the choice rests on: the task itself, in order of the rule that picked it.
    pub evidence: Vec<String>,
}

/// The next step for a lane, or `None` when there is nothing to do.
pub fn next_step_for_lane(tasks: &[LaneTask]) -> Option<Step> {
    let open = || tasks.iter().filter(|t| t.status != LaneStatus::Complete);
    let step = |kind, t: &LaneTask| Step {
        kind,
        task_id: t.id.clone(),
        title: t.title.clone(),
        evidence: vec![t.id.clone()],
    };
    // Most overdue first, then the oldest, so a task untouched for days is not buried.
    let urgency = |t: &&LaneTask| {
        (
            t.priority,
            t.due.clone().unwrap_or_else(|| "9999".into()),
            -t.age_days,
            t.id.clone(),
        )
    };

    if let Some(t) = open().filter(|t| t.owner_must_decide).min_by_key(urgency) {
        return Some(step(StepKind::OwnerDecision, t));
    }
    let ready = open()
        .filter(|t| t.status == LaneStatus::ToDo && t.dependencies_done && !t.has_lease)
        .min_by_key(urgency);
    if let Some(t) = ready {
        return Some(step(StepKind::Ready, t));
    }
    let resume = open()
        .filter(|t| t.status == LaneStatus::InProgress && !t.has_lease && t.has_checkpoint)
        .min_by_key(urgency);
    if let Some(t) = resume {
        return Some(step(StepKind::Resume, t));
    }
    open()
        .filter(|t| t.stale && t.status != LaneStatus::ReadyForReview)
        .min_by_key(urgency)
        .map(|t| step(StepKind::Hygiene, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, status: LaneStatus) -> LaneTask {
        LaneTask {
            id: id.into(),
            title: format!("Task {id}"),
            status,
            priority: 2,
            due: None,
            age_days: 1,
            owner_must_decide: false,
            dependencies_done: true,
            has_lease: false,
            has_checkpoint: false,
            stale: false,
        }
    }

    #[test]
    fn alpha_lane_blocked_on_a_review_has_no_ready_work_and_surfaces_hygiene() {
        let mut review = task("a1", LaneStatus::ReadyForReview);
        review.age_days = 6;
        let mut next = task("a2", LaneStatus::ToDo);
        next.dependencies_done = false;
        let mut stale = task("a3", LaneStatus::InProgress);
        stale.stale = true;
        let step = next_step_for_lane(&[review, next, stale]).unwrap();
        assert_eq!(
            (step.kind, step.task_id.as_str()),
            (StepKind::Hygiene, "a3")
        );
    }

    #[test]
    fn beta_lane_waiting_on_the_owner_asks_for_the_decision_before_anything_else() {
        let mut decide = task("b1", LaneStatus::Blocked);
        decide.owner_must_decide = true;
        decide.age_days = 2;
        let ready = task("b2", LaneStatus::ToDo);
        let step = next_step_for_lane(&[ready, decide]).unwrap();
        assert_eq!(
            (step.kind, step.task_id.as_str()),
            (StepKind::OwnerDecision, "b1")
        );
        assert_eq!(step.evidence, vec!["b1".to_string()]);
    }

    #[test]
    fn gamma_lane_picks_the_urgent_old_task_among_ready_work() {
        let mut urgent = task("g1", LaneStatus::ToDo);
        urgent.priority = 0;
        urgent.age_days = 5;
        let routine = task("g2", LaneStatus::ToDo);
        let step = next_step_for_lane(&[routine, urgent]).unwrap();
        assert_eq!((step.kind, step.task_id.as_str()), (StepKind::Ready, "g1"));
    }

    #[test]
    fn an_unleased_in_progress_task_with_a_checkpoint_is_resumed_and_a_leased_one_is_left_alone() {
        let mut resumable = task("r1", LaneStatus::InProgress);
        resumable.has_checkpoint = true;
        let mut leased = task("r2", LaneStatus::InProgress);
        leased.has_checkpoint = true;
        leased.has_lease = true;
        let step = next_step_for_lane(&[leased, resumable]).unwrap();
        assert_eq!((step.kind, step.task_id.as_str()), (StepKind::Resume, "r1"));
    }

    #[test]
    fn nothing_open_means_no_step_and_the_same_snapshot_always_gives_the_same_step() {
        assert_eq!(next_step_for_lane(&[task("d", LaneStatus::Complete)]), None);
        assert_eq!(next_step_for_lane(&[]), None);
        let tasks = vec![task("x", LaneStatus::ToDo), task("y", LaneStatus::ToDo)];
        let first = next_step_for_lane(&tasks);
        for _ in 0..20 {
            assert_eq!(next_step_for_lane(&tasks), first);
        }
        let mut reversed = tasks.clone();
        reversed.reverse();
        assert_eq!(
            next_step_for_lane(&reversed),
            first,
            "input order does not matter"
        );
    }
}
