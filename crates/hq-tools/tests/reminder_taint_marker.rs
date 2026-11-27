//! The reminder tools read the same "this session read outside text" marker the task tools do.
//! They repeat the string because `reminders` sits in a lower layer than `tasks` and may not
//! import it; this test keeps the two from drifting apart.

#[test]
fn reminders_read_the_marker_the_runtime_stamps_on_task_writes() {
    assert_eq!(
        hq_tools::reminders::tools::TAINTED_SESSION_ARG,
        hq_tools::tasks::TAINTED_SESSION_ARG
    );
}
