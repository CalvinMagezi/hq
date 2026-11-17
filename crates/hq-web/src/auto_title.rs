//! A short title for a new chat, written after its first reply instead of cutting the first message off.
//!
//! It runs once, when a chat holds exactly one question and one answer and still carries the title that was cut
//! from the question. A chat the owner renamed, a longer chat, or one with the choice turned off is left alone.
//! The model gets the question and the start of the answer and nothing else, with no tools, so the worst a bad
//! reply can do is a bad title, which is cleaned before it is saved.

use std::future::Future;

use anyhow::{Context, Result, anyhow};
use hq_core::types::{ChatMessage, MessageRole};
use hq_llm::{LlmProvider, provider::ChatRequest, router::LlmRouter};
use std::time::Duration;

const MODEL: &str = "fast";
const MAX_REPLY_TOKENS: u32 = 40;
const MODEL_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest title kept, in characters.
const MAX_TITLE_CHARS: usize = 60;
/// How much of each side the model reads.
const READ_CHARS: usize = 600;

pub(crate) const SYSTEM_PROMPT: &str = "You name chats. Reply with a title of two to six words for the chat below, in the language the person wrote in. \
Plain words only: no quotes, no markdown, no trailing full stop, no emoji. Say what the chat is about, not that it is a chat.";

fn first_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// The prompt's user turn: the question and the start of the answer.
pub(crate) fn prompt(question: &str, answer: &str) -> String {
    format!(
        "Question:\n{}\n\nAnswer:\n{}",
        first_chars(question.trim(), READ_CHARS),
        first_chars(answer.trim(), READ_CHARS)
    )
}

/// A model's reply as a title: first line, markup and quotes taken off, cut to size; None when nothing usable is left.
pub(crate) fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches(|c: char| ['#', '*', '_', '`', '>', '-', ' '].contains(&c));
    let line = line
        .trim_end_matches(|c: char| ['*', '_', '`', ' ', '.', ':', ';', ','].contains(&c))
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’'))
        .trim();
    let line = line
        .strip_prefix("Title:")
        .or_else(|| line.strip_prefix("title:"))
        .unwrap_or(line)
        .trim();
    if line.chars().count() < 2 {
        return None;
    }
    if line.chars().count() <= MAX_TITLE_CHARS {
        return Some(line.to_string());
    }
    let cut: String = line.chars().take(MAX_TITLE_CHARS).collect();
    // Back up to a word end so a word is not cut in half.
    Some(
        cut.rsplit_once(' ')
            .map(|(head, _)| head.to_string())
            .unwrap_or(cut),
    )
}

/// The question and answer of a chat that has just had its first reply and has not been renamed, or None.
pub(crate) fn candidate(
    db: &hq_db::Database,
    thread_id: &str,
    cut_title: impl Fn(&str) -> String,
) -> Option<(String, String)> {
    if !hq_db::owner_prefs::auto_title_on(db) {
        return None;
    }
    let (thread, messages) = db
        .with_conn(|c| {
            let thread = hq_db::chat::get_thread(c, thread_id)?;
            let messages = hq_db::chat::get_messages(c, thread_id, 4)?;
            Ok((thread, messages))
        })
        .ok()?;
    let thread = thread?;
    let [first, second] = messages.as_slice() else {
        return None;
    };
    if first.role != "user" || second.role != "assistant" || second.content.trim().is_empty() {
        return None;
    }
    // Only a title still cut from the question is HQ's to replace; a rename by the owner stays.
    (thread.title == cut_title(&first.content))
        .then(|| (first.content.clone(), second.content.clone()))
}

fn message(role: MessageRole, content: String) -> ChatMessage {
    ChatMessage {
        image_parts: Vec::new(),
        role,
        content,
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    }
}

/// One model call with no tools: the naming prompt and the question and answer in, the reply out.
pub(crate) async fn ask_model(user: String) -> Result<String> {
    let router = LlmRouter::from_env();
    let request = ChatRequest {
        model: MODEL.to_string(),
        messages: vec![
            message(MessageRole::System, SYSTEM_PROMPT.to_string()),
            message(MessageRole::User, user),
        ],
        tools: Vec::new(),
        max_tokens: Some(MAX_REPLY_TOKENS),
        ..Default::default()
    };
    let call = hq_llm::with_origin(hq_llm::origin::BACKGROUND, router.chat(&request));
    let reply = tokio::time::timeout(MODEL_TIMEOUT, call)
        .await
        .map_err(|_| anyhow!("the model took too long"))?
        .context("the model call failed")?;
    Ok(reply.message.content)
}

/// Writes the title and returns it. `ask` is the model call: system prompt and user turn in, its reply out.
pub(crate) async fn name_chat<F, Fut>(
    db: &hq_db::Database,
    thread_id: &str,
    question: &str,
    answer: &str,
    ask: F,
) -> Option<String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let reply = ask(prompt(question, answer)).await.ok()?;
    let title = clean_title(&reply)?;
    // The chat may have been renamed while the model thought; only replace the title that was there.
    db.with_conn(|c| hq_db::chat::set_thread_title(c, thread_id, &title))
        .ok()?;
    Some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_becomes_a_plain_short_title() {
        assert_eq!(
            clean_title("\"Samsung battery settings\"").as_deref(),
            Some("Samsung battery settings")
        );
        assert_eq!(
            clean_title("## Title: Stock count for week 46.\nMore words").as_deref(),
            Some("Stock count for week 46")
        );
        assert_eq!(clean_title("**Trip notes**").as_deref(), Some("Trip notes"));
        assert_eq!(clean_title("   \n  "), None);
        assert_eq!(clean_title("x"), None);
    }

    fn fresh(db: &hq_db::Database, question: &str, answer: &str, rename: Option<&str>) -> String {
        db.with_conn(|c| {
            let t =
                hq_db::chat::create_thread(c, &crate::ws::thread_title(question), "user", "web")?;
            hq_db::chat::add_message(c, &t.thread_id, "user", question)?;
            hq_db::chat::add_message(c, &t.thread_id, "assistant", answer)?;
            if let Some(name) = rename {
                hq_db::chat::set_thread_title(c, &t.thread_id, name)?;
            }
            Ok(t.thread_id)
        })
        .unwrap()
    }

    #[test]
    fn a_long_reply_is_cut_at_a_word() {
        let long = "word ".repeat(30);
        let title = clean_title(&long).unwrap();
        assert!(title.chars().count() <= MAX_TITLE_CHARS);
        assert!(title.ends_with("word"), "{title}");
    }

    #[test]
    fn a_new_chat_is_named_once_and_a_renamed_or_longer_one_is_left_alone() {
        let db = hq_db::Database::open_memory().unwrap();
        let fresh_chat = fresh(
            &db,
            "How do I stop Samsung killing the app in the background?",
            "Open Settings, then Battery.",
            None,
        );
        assert!(candidate(&db, &fresh_chat, crate::ws::thread_title).is_some());

        let renamed = fresh(&db, "Question", "Answer", Some("My own name"));
        assert!(
            candidate(&db, &renamed, crate::ws::thread_title).is_none(),
            "a rename by the owner stays"
        );

        let longer = fresh(&db, "Another", "Reply", None);
        db.with_conn(|c| hq_db::chat::add_message(c, &longer, "user", "And then?").map(|_| ()))
            .unwrap();
        assert!(
            candidate(&db, &longer, crate::ws::thread_title).is_none(),
            "only the first exchange is named"
        );

        hq_db::owner_prefs::set(&db, hq_db::owner_prefs::CHAT_AUTO_TITLE, "false").unwrap();
        let other = fresh(&db, "Third", "Reply", None);
        assert!(
            candidate(&db, &other, crate::ws::thread_title).is_none(),
            "turned off in Settings"
        );
    }

    #[tokio::test]
    async fn the_title_the_model_gives_is_cleaned_and_saved() {
        let db = hq_db::Database::open_memory().unwrap();
        let id = fresh(&db, "Why is my shop's stock count off?", "Because...", None);
        let named = name_chat(&db, &id, "q", "a", |_| async {
            Ok("\"Stock count mismatch\"\n".to_string())
        })
        .await;
        assert_eq!(named.as_deref(), Some("Stock count mismatch"));
        let saved = db
            .with_conn(|c| hq_db::chat::get_thread(c, &id))
            .unwrap()
            .unwrap()
            .title;
        assert_eq!(saved, "Stock count mismatch");
        // A model that fails, or says nothing usable, leaves the title alone.
        let other = fresh(&db, "Another one", "Reply", None);
        assert!(
            name_chat(&db, &other, "q", "a", |_| async { Err(anyhow!("down")) })
                .await
                .is_none()
        );
        assert!(
            name_chat(&db, &other, "q", "a", |_| async { Ok("  ".to_string()) })
                .await
                .is_none()
        );
    }

    #[test]
    fn the_model_reads_only_the_start_of_each_side() {
        let p = prompt(&"q".repeat(5000), &"a".repeat(5000));
        assert!(p.chars().count() < 1300);
    }
}
