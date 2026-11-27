//! A session started after the chat read outside text is started narrowed: the strict sandbox
//! (no credentials, a bounded network), a brief built from the stored task and no free text the
//! model wrote. Only the governance layer around a tool call can ask for it, and a launch can only
//! be narrowed, never widened by it.

use std::future::Future;

tokio::task_local! {
    static NARROWED: ();
}

/// Runs `fut`, and every session it launches, narrowed.
pub async fn scoped<F: Future>(fut: F) -> F::Output {
    NARROWED.scope((), fut).await
}

/// Whether the launch being made is narrowed.
pub fn active() -> bool {
    NARROWED.try_with(|_| ()).is_ok()
}

/// `fut` narrowed if the caller is. The mark belongs to the current task, so anything handed to
/// `tokio::spawn` has to be wrapped in this or it runs wide. Read when called, not when polled.
pub fn carry<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    let narrowed = active();
    async move {
        if narrowed {
            scoped(fut).await
        } else {
            fut.await
        }
    }
}

/// The prompt a narrowed session starts with: the stored task and nothing the model typed.
pub(super) fn brief(task_id: &str, goal: &str) -> String {
    format!(
        "You were started from HQ task {task_id}. Work only on this task:\n\n{goal}\n\n\
         The request that started you was withheld because the chat had read outside text. \
         You run under a strict sandbox: no credentials, and only the network hosts it allows."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn narrowing_holds_inside_the_scope_only() {
        assert!(!active());
        scoped(async { assert!(active()) }).await;
        assert!(!active());
    }

    #[tokio::test]
    async fn narrowing_follows_a_spawned_task_only_when_carried() {
        scoped(async {
            assert!(!tokio::spawn(async { active() }).await.unwrap());
            assert!(tokio::spawn(carry(async { active() })).await.unwrap());
        })
        .await;
        assert!(!tokio::spawn(carry(async { active() })).await.unwrap());
    }

    #[test]
    fn the_brief_carries_the_task_and_says_the_request_was_withheld() {
        let b = brief("tk-1", "Fix the login page");
        assert!(b.contains("tk-1") && b.contains("Fix the login page") && b.contains("withheld"));
    }
}
