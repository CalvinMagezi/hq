//! A driver turn must not paste a session's brief back into it. The session has
//! had its brief since launch; typed again mid-task, it can contradict the
//! instructions given since and stop the agent on a question about which applies.
//! Prompt text alone did not stop this, so the check is here, in code.

use super::*;
use std::collections::HashSet;

/// Words per shingle: long enough that ordinary phrasing does not collide.
const SHINGLE_WORDS: usize = 6;
/// Shared shingles below this never count: quoting one acceptance line to say it
/// is still unmet is a fair instruction.
const MIN_SHARED: usize = 12;
/// Share of a brief's shingles an instruction must repeat to count as a re-send.
const SHARE_NUMERATOR: usize = 2;
const SHARE_DENOMINATOR: usize = 5;

fn shingles(text: &str) -> HashSet<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    words.windows(SHINGLE_WORDS).map(|w| w.join(" ")).collect()
}

/// `text` repeats most of `brief`: at least `MIN_SHARED` of the brief's
/// shingles, and at least two fifths of them.
pub(super) fn repeats(text: &str, brief: &str) -> bool {
    let brief = shingles(brief);
    if brief.len() < MIN_SHARED {
        return false;
    }
    let shared = shingles(text).intersection(&brief).count();
    shared >= MIN_SHARED && shared * SHARE_DENOMINATOR >= brief.len() * SHARE_NUMERATOR
}

/// Refuses a driver instruction that repeats the session's goal, its definition
/// of done or its task's description.
pub(super) fn refuse_repeated_brief(
    db: &Arc<Database>,
    session_id: &str,
    text: &str,
) -> Result<()> {
    let row = get_row(db, session_id)?;
    let task = row
        .mission_id
        .as_deref()
        .and_then(|id| db.with_conn(|c| Ok(task_goal(c, id))).ok().flatten());
    let sources = [
        row.goal.as_deref(),
        row.done_criteria.as_deref(),
        task.as_deref(),
    ];
    if sources
        .into_iter()
        .flatten()
        .any(|brief| repeats(text, brief))
    {
        bail!(
            "this instruction repeats most of session {session_id}'s brief, which it has had since it started. \
             Sending it again mid-task can contradict what it was told since. Send only the next step, in a sentence or two, \
             or name the task to continue."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRIEF: &str = "Build plan task 3 of the booking redesign on the feature branch, per the brief file, \
        and commit it without pushing. Booking records gain venue, party size, channel, createdAt and confirmedAt; \
        legacy note fields are optional and no longer written; the cancellation window is enforced at thirty minutes \
        and never in the past; a migration exists with dry run on by default and tests; generated code is \
        regenerated; the targeted tests, typecheck and lint pass.";

    #[test]
    fn the_brief_pasted_back_whole_or_mostly_is_a_repeat() {
        assert!(repeats(BRIEF, BRIEF));
        let mostly: String = BRIEF.split(';').take(4).collect::<Vec<_>>().join(";");
        assert!(repeats(&format!("Reminder of your task: {mostly}"), BRIEF));
    }

    #[test]
    fn a_next_step_or_one_quoted_line_is_not() {
        assert!(!repeats(
            "Continue with task 4. Its acceptance is on the task.",
            BRIEF
        ));
        assert!(!repeats(
            "The cancellation window is enforced at thirty minutes and never in the past: \
             the test for the past case still fails, fix that next.",
            BRIEF
        ));
    }

    #[test]
    fn a_short_goal_is_never_matched() {
        let goal = "Add rate limiting to the login endpoint";
        assert!(!repeats(goal, goal));
    }

    #[test]
    fn case_and_punctuation_do_not_hide_a_repeat() {
        assert!(repeats(&BRIEF.to_uppercase().replace(',', " "), BRIEF));
    }
}
