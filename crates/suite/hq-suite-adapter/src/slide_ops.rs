//! Card edits for a slides note: add, replace, move and delete a card, set its speaker notes and
//! its look (transition, colours, layout).
//!
//! A card is the run of blocks that starts at a `## ` heading and ends before the next one
//! (`docs/suite/SLIDES.md`), so every action is a set of block-level hqmd operations and the rest
//! of the deck comes back byte for byte. A card is addressed by its number and its `hash`, which
//! covers all its blocks, so a card that changed since it was read is refused. Notes and look use
//! the comment lines the pptx writer reads (`<!-- notes: ... -->`, `<!-- transition: fade -->`),
//! and their values are checked with the writer's own parsers.

use anyhow::{Context, Result, bail};
use hq_suite_md::hash::block_hash;
use hq_suite_md::{Document, NodeKind, Op};
use hq_suite_slides::deck::{MAX_CARDS, TRANSITION_NAMES, Transition, parse_color};
use hq_suite_slides::{Deck, Kind, Layout, Speed};
use serde_json::{Value, json};

use crate::edit_plan::{Planned, addr, opt_str, patch, req_str, req_u64};

/// Longest speaker note, and longest card title.
const MAX_NOTES_CHARS: usize = 20_000;
const MAX_TITLE_CHARS: usize = 200;

/// One card: its heading's block and the blocks that follow it, up to the next card.
#[derive(Debug, Clone)]
pub struct Span {
    pub title: String,
    pub head: usize,
    /// One past the card's last block.
    pub end: usize,
}

fn deck_of(doc: &Document) -> Deck {
    let text = doc.render();
    Deck::from_markdown("", &text[doc.frontmatter().len()..])
}

/// The cards of a note, in order, found the way the pptx writer finds them.
pub fn spans(doc: &Document) -> Result<Vec<Span>> {
    let nodes = doc.nodes();
    let heads: Vec<usize> = (0..nodes.len())
        .filter(|&i| {
            matches!(nodes[i].kind(), NodeKind::Heading { level: 2 })
                && nodes[i].src().starts_with("## ")
        })
        .collect();
    let out: Vec<Span> = heads
        .iter()
        .enumerate()
        .map(|(k, &head)| Span {
            title: nodes[head].src()[3..].trim().to_string(),
            head,
            end: heads.get(k + 1).copied().unwrap_or(nodes.len()),
        })
        .collect();
    if deck_of(doc).cards.len() != out.len() {
        bail!("this deck has `## ` lines the editor cannot follow as cards; edit it with blocks");
    }
    Ok(out)
}

/// What identifies a card's content: all its blocks, with the blank space between them.
fn card_hash(doc: &Document, s: &Span) -> String {
    let nodes = doc.nodes();
    let mut text = nodes[s.head].src().to_string();
    for n in &nodes[s.head + 1..s.end] {
        text.push_str(n.lead());
        text.push_str(n.src());
    }
    block_hash(&text)
}

fn layout_name(l: Layout) -> &'static str {
    match l {
        Layout::Content => "content",
        Layout::TwoColumn => "two-column",
    }
}

fn transition_text(t: Transition) -> String {
    let kind = match t.kind {
        Kind::None => "none",
        Kind::Fade => "fade",
        Kind::Push => "push",
        Kind::Wipe => "wipe",
        Kind::Split => "split",
        Kind::Cover => "cover",
        Kind::Dissolve => "dissolve",
    };
    match t.speed {
        Speed::Medium => kind.to_string(),
        Speed::Slow => format!("{kind} slow"),
        Speed::Fast => format!("{kind} fast"),
    }
}

fn card_json(doc: &Document, deck: &Deck, i: usize, s: &Span) -> Value {
    let c = &deck.cards[i];
    json!({
        "index": i,
        "title": s.title,
        "hash": card_hash(doc, s),
        "blocks": s.end - s.head,
        "layout": layout_name(c.layout),
        "transition": c.look.transition.map(transition_text),
        "background": c.look.background.as_ref().map(|h| format!("#{h}")),
        "color": c.look.color.as_ref().map(|h| format!("#{h}")),
        "notes": c.notes,
    })
}

/// The deck's cards for `office_edit`'s `cards` action: each card's number, `hash` and look.
pub fn list(doc: &Document) -> Result<Value> {
    let spans = spans(doc)?;
    let deck = deck_of(doc);
    let cards: Vec<Value> = spans
        .iter()
        .enumerate()
        .map(|(i, s)| card_json(doc, &deck, i, s))
        .collect();
    Ok(json!({"revision": doc.revision(), "title": deck.title, "cards": cards}))
}

fn card_at<'a>(
    doc: &Document,
    spans: &'a [Span],
    args: &Value,
    key: &str,
) -> Result<(usize, &'a Span)> {
    let i = req_u64(args, key)? as usize;
    let s = spans
        .get(i)
        .with_context(|| format!("the deck has {} cards; card {i} is not one", spans.len()))?;
    let hash = req_str(args, "hash")?;
    if card_hash(doc, s) != hash {
        bail!("card {i} changed since you read it; read the cards again and retry");
    }
    Ok((i, s))
}

fn clean_title(title: &str) -> Result<String> {
    let t = title.trim();
    if t.is_empty() || t.contains(['\n', '\r']) || t.chars().count() > MAX_TITLE_CHARS {
        bail!("a card title is one line of 1 to {MAX_TITLE_CHARS} characters");
    }
    Ok(t.to_string())
}

fn card_text(title: &str, body: Option<&str>) -> String {
    match body.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => format!("## {title}\n\n{b}"),
        None => format!("## {title}"),
    }
}

/// After a change, the deck must read back as the cards the action meant to leave.
fn verify(new: &Document, titles: &[String]) -> Result<()> {
    let got: Vec<String> = spans(new)
        .map_err(|e| anyhow::anyhow!("that change would break the deck's cards: {e}"))?
        .into_iter()
        .map(|s| s.title)
        .collect();
    if got != titles {
        bail!(
            "that text would change the deck's cards in another way than asked (for example by carrying its own `## ` heading); make one card per call"
        );
    }
    Ok(())
}

fn titles_of(spans: &[Span]) -> Vec<String> {
    spans.iter().map(|s| s.title.clone()).collect()
}

fn with_card(doc: &Document, i: usize) -> Value {
    let (Ok(spans), deck) = (spans(doc), deck_of(doc)) else {
        return json!({});
    };
    match spans.get(i) {
        Some(s) => json!({"card": card_json(doc, &deck, i, s)}),
        None => json!({}),
    }
}

// ---------------------------------------------------------------- add, replace, move, delete

/// Adds a card after card `after_card` (which must still hash to `hash`), or at the end.
pub fn add_card(doc: &Document, args: &Value) -> Result<Planned> {
    let spans = spans(doc)?;
    if spans.len() >= MAX_CARDS {
        bail!("a deck holds at most {MAX_CARDS} cards");
    }
    let title = clean_title(req_str(args, "title")?)?;
    let (place, index) = match args.get("after_card").filter(|v| !v.is_null()) {
        Some(_) => {
            let (i, s) = card_at(doc, &spans, args, "after_card")?;
            (doc.addr(s.end - 1), i + 1)
        }
        None => (
            doc.nodes().len().checked_sub(1).and_then(|i| doc.addr(i)),
            spans.len(),
        ),
    };
    let new = patch(
        doc,
        vec![Op::InsertAfter {
            at: place,
            markdown: card_text(&title, opt_str(args, "markdown")),
        }],
    )?;
    let mut want = titles_of(&spans);
    want.insert(index, title.clone());
    verify(&new, &want)?;
    let extra = with_card(&new, index);
    Ok(Planned::new(new, format!("Added card {} `{title}`", index + 1)).with(extra))
}

/// Replaces a whole card (its title, text, notes and look) with `markdown`.
pub fn replace_card(doc: &Document, args: &Value) -> Result<Planned> {
    let spans = spans(doc)?;
    let (i, s) = card_at(doc, &spans, args, "card")?;
    let title = match opt_str(args, "title") {
        Some(t) => clean_title(t)?,
        None => s.title.clone(),
    };
    let body = req_str(args, "markdown")?;
    let nodes = doc.nodes();
    let mut ops = vec![Op::Replace {
        at: addr(doc, s.head, None)?,
        markdown: format!("## {title}"),
    }];
    for k in s.head + 1..s.end {
        ops.push(Op::Delete {
            at: addr(doc, k, None)?,
        });
    }
    if !body.trim().is_empty() {
        ops.push(Op::InsertAfter {
            at: Some(addr(doc, s.head, None)?),
            markdown: body.trim().to_string(),
        });
    }
    let new = patch(doc, ops)?;
    let mut want = titles_of(&spans);
    want[i] = title.clone();
    verify(&new, &want)?;
    let extra = with_card(&new, i);
    Ok(Planned::new(
        new,
        format!(
            "Replaced card {} `{}` ({} blocks) with `{title}`; its notes and look go with it unless the new text carries them",
            i + 1,
            s.title,
            nodes[s.head..s.end].len()
        ),
    )
    .removing()
    .with(extra))
}

/// Moves a card so it ends up as card number `to` (from 0).
pub fn move_card(doc: &Document, args: &Value) -> Result<Planned> {
    let spans = spans(doc)?;
    let (i, s) = card_at(doc, &spans, args, "card")?;
    let to = req_u64(args, "to")? as usize;
    if to >= spans.len() {
        bail!("`to` must be from 0 to {}", spans.len() - 1);
    }
    if to == i {
        bail!("card {i} is already at position {to}");
    }
    let nodes = doc.nodes();
    let mut fragment = nodes[s.head].src().to_string();
    for n in &nodes[s.head + 1..s.end] {
        fragment.push_str(n.lead());
        fragment.push_str(n.src());
    }
    let rest: Vec<&Span> = spans
        .iter()
        .enumerate()
        .filter(|(k, _)| *k != i)
        .map(|(_, s)| s)
        .collect();
    // The new card goes before the card now at `to`, or at the end; it follows the last block
    // that stays and comes before that place.
    let slot = rest.get(to).map_or(nodes.len(), |s| s.head);
    let anchor = (0..slot).rev().find(|k| !(s.head..s.end).contains(k));
    let mut ops: Vec<Op> = (s.head..s.end)
        .map(|k| {
            Ok(Op::Delete {
                at: addr(doc, k, None)?,
            })
        })
        .collect::<Result<_>>()?;
    ops.push(Op::InsertAfter {
        at: anchor.and_then(|k| doc.addr(k)),
        markdown: fragment,
    });
    let new = patch(doc, ops)?;
    let mut want = titles_of(&spans);
    let moved = want.remove(i);
    want.insert(to, moved);
    verify(&new, &want)?;
    let extra = with_card(&new, to);
    Ok(Planned::new(
        new,
        format!("Moved card {} `{}` to position {}", i + 1, s.title, to + 1),
    )
    .with(extra))
}

/// Deletes a card and everything in it.
pub fn delete_card(doc: &Document, args: &Value) -> Result<Planned> {
    let spans = spans(doc)?;
    let (i, s) = card_at(doc, &spans, args, "card")?;
    let ops = (s.head..s.end)
        .map(|k| {
            Ok(Op::Delete {
                at: addr(doc, k, None)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let new = patch(doc, ops)?;
    let mut want = titles_of(&spans);
    want.remove(i);
    verify(&new, &want)?;
    Ok(Planned::new(
        new,
        format!(
            "Deleted card {} `{}` ({} blocks)",
            i + 1,
            s.title,
            s.end - s.head
        ),
    )
    .removing())
}

// ---------------------------------------------------------------- notes and look

/// The comment a block is, as `(key, value)`, when it is one single-line `<!-- key: value -->`.
fn setting_of(src: &str) -> Option<(String, String)> {
    let t = src.trim();
    let inner = t.strip_prefix("<!--")?.strip_suffix("-->")?;
    if inner.contains("<!--") || inner.contains('\n') {
        return None;
    }
    let (k, v) = inner.split_once(':')?;
    Some((k.trim().to_string(), v.trim().to_string()))
}

fn is_comment(src: &str) -> bool {
    let t = src.trim();
    t.starts_with("<!--") && t.ends_with("-->") && t.matches("<!--").count() == 1
}

/// The block after which a new comment goes: the last of the comments right under the heading.
fn comment_anchor(doc: &Document, s: &Span) -> usize {
    let nodes = doc.nodes();
    let mut at = s.head;
    while at + 1 < s.end
        && matches!(nodes[at + 1].kind(), NodeKind::Html)
        && is_comment(nodes[at + 1].src())
    {
        at += 1;
    }
    at
}

fn notes_comment(src: &str) -> bool {
    let t = src.trim();
    t.starts_with("<!--")
        && t.ends_with("-->")
        && t[4..].trim_start().starts_with("notes:")
        && t.matches("<!--").count() == 1
}

/// What the writer reads from notes text: paragraphs split at blank lines, lines joined by a space.
fn paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for l in text.lines().map(str::trim) {
        if l.is_empty() {
            if !cur.is_empty() {
                out.push(cur.join(" "));
                cur.clear();
            }
        } else {
            cur.push(l);
        }
    }
    if !cur.is_empty() {
        out.push(cur.join(" "));
    }
    out
}

/// Sets a card's speaker notes (an empty text clears them).
pub fn set_notes(doc: &Document, args: &Value) -> Result<Planned> {
    let spans = spans(doc)?;
    let (i, s) = card_at(doc, &spans, args, "card")?;
    let text = req_str(args, "text")?.replace("\r\n", "\n");
    let text = text.trim();
    if text.chars().count() > MAX_NOTES_CHARS {
        bail!("speaker notes hold at most {MAX_NOTES_CHARS} characters");
    }
    if text.contains("-->") || text.contains("<!--") {
        bail!("speaker notes cannot contain `-->` or `<!--`");
    }
    let nodes = doc.nodes();
    let existing: Vec<usize> = (s.head + 1..s.end)
        .filter(|&k| matches!(nodes[k].kind(), NodeKind::Html) && notes_comment(nodes[k].src()))
        .collect();
    let comment = if text.contains('\n') {
        format!("<!-- notes:\n{text}\n-->")
    } else {
        format!("<!-- notes: {text} -->")
    };
    let mut ops = Vec::new();
    for (n, &k) in existing.iter().enumerate() {
        let at = addr(doc, k, None)?;
        ops.push(if n == 0 && !text.is_empty() {
            Op::Replace {
                at,
                markdown: comment.clone(),
            }
        } else {
            Op::Delete { at }
        });
    }
    if existing.is_empty() && !text.is_empty() {
        ops.push(Op::InsertAfter {
            at: Some(addr(doc, comment_anchor(doc, s), None)?),
            markdown: comment,
        });
    }
    let had = !existing.is_empty();
    let new = patch(doc, ops)?;
    verify(&new, &titles_of(&spans))?;
    let got = deck_of(&new)
        .cards
        .get(i)
        .map(|c| c.notes.clone())
        .unwrap_or_default();
    if got != paragraphs(text) {
        bail!("those notes would not read back as written; avoid leading `-` or `#` lines in them");
    }
    let summary = match (text.is_empty(), had) {
        (true, true) => format!("Cleared the speaker notes of card {} `{}`", i + 1, s.title),
        (true, false) => format!("Card {} `{}` has no speaker notes to clear", i + 1, s.title),
        _ => format!("Set the speaker notes of card {} `{}`", i + 1, s.title),
    };
    let extra = with_card(&new, i);
    let mut plan = Planned::new(new, summary).with(extra);
    plan.destructive = text.is_empty() && had;
    Ok(plan)
}

/// A look setting as the text its comment holds, checked the way the pptx writer checks it.
fn style_value(key: &str, value: &str) -> Result<String> {
    match key {
        "transition" => Transition::parse(value)
            .map(|_| value.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase())
            .with_context(|| {
                format!("`{value}` is not a transition. Use one of {TRANSITION_NAMES}, then slow, medium or fast if you like")
            }),
        "background" | "color" => parse_color(value)
            .map(|h| format!("#{h}"))
            .with_context(|| format!("`{value}` is not a colour for {key}. Write it as #RRGGBB, for example #1F3A5F")),
        _ => Layout::named(value)
            .map(|l| layout_name(l).to_string())
            .with_context(|| format!("`{value}` is not a layout. Use content or two-column")),
    }
}

/// Sets a card's transition, background, text colour and layout. A key given as `null` is cleared.
pub fn set_card_style(doc: &Document, args: &Value) -> Result<Planned> {
    let spans = spans(doc)?;
    let (i, s) = card_at(doc, &spans, args, "card")?;
    let mut changes: Vec<(&str, Option<String>)> = Vec::new();
    for key in ["transition", "background", "color", "layout"] {
        match args.get(key) {
            None => {}
            Some(Value::Null) => changes.push((key, None)),
            Some(Value::String(v)) => changes.push((key, Some(style_value(key, v)?))),
            Some(_) => bail!("`{key}` is text, or null to clear it"),
        }
    }
    if changes.is_empty() {
        bail!("give at least one of transition, background, color or layout");
    }
    let nodes = doc.nodes();
    let mut ops = Vec::new();
    let mut fresh: Vec<String> = Vec::new();
    for (key, value) in &changes {
        let found = (s.head + 1..s.end).find(|&k| {
            matches!(nodes[k].kind(), NodeKind::Html)
                && setting_of(nodes[k].src()).is_some_and(|(name, _)| name == *key)
        });
        match (found, value) {
            (Some(k), Some(v)) => ops.push(Op::Replace {
                at: addr(doc, k, None)?,
                markdown: format!("<!-- {key}: {v} -->"),
            }),
            (Some(k), None) => ops.push(Op::Delete {
                at: addr(doc, k, None)?,
            }),
            (None, Some(v)) => fresh.push(format!("<!-- {key}: {v} -->")),
            (None, None) => {}
        }
    }
    if !fresh.is_empty() {
        ops.push(Op::InsertAfter {
            at: Some(addr(doc, comment_anchor(doc, s), None)?),
            markdown: fresh.join("\n"),
        });
    }
    let new = patch(doc, ops)?;
    verify(&new, &titles_of(&spans))?;
    let deck = deck_of(&new);
    let card = &deck.cards[i];
    for (key, value) in changes
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| (*k, v)))
    {
        let ok = match key {
            "transition" => card.look.transition == Transition::parse(value),
            "background" => card.look.background == parse_color(value),
            "color" => card.look.color == parse_color(value),
            _ => Some(card.layout) == Layout::named(value),
        };
        if !ok {
            bail!(
                "the card's {key} would not read back as `{value}`; another comment in the card sets it first"
            );
        }
    }
    let place = format!("Card \"{}\"", s.title);
    let warnings: Vec<&String> = deck
        .look_problems
        .iter()
        .filter(|p| p.starts_with(&place))
        .collect();
    let said: Vec<String> = changes
        .iter()
        .map(|(k, v)| match v {
            Some(v) => format!("{k} `{v}`"),
            None => format!("{k} cleared"),
        })
        .collect();
    let mut extra = with_card(&new, i);
    extra["warnings"] = json!(warnings);
    Ok(Planned::new(
        new,
        format!("Card {} `{}`: {}", i + 1, s.title, said.join(", ")),
    )
    .with(extra))
}

#[cfg(test)]
#[path = "slide_ops_tests.rs"]
mod tests;
