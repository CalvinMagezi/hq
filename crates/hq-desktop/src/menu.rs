//! The menu bar of the HQ window, as data: which items it has, their keys, and what each does.
//!
//! Inside a desktop window there are no browser tabs, so Cmd 1, 2 and 3 are free here. A menu item
//! with a key takes the key before the page sees it, so every item does what the page's own key
//! would: it opens a place, or sends the page the same event the page's key sends. Building the real
//! menu from this model is the shell's job (`apps/desktop`); what each item may do is decided here
//! and tested.

use crate::words::word;

/// What choosing an item does in the HQ window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Opens a place of the web app. Only an address `main_path_allowed` accepts.
    Go(&'static str),
    /// Sends the page a window event, the one its own key sends. Only names in `WEB_EVENTS`.
    Web(&'static str),
}

/// The events the page listens for on behalf of the menu.
pub const WEB_EVENTS: [&str; 4] = [
    "hq:open-search",
    "hq:open-quick-note",
    "hq:new-chat",
    "hq:toggle-pane",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub id: &'static str,
    /// The catalogue key of the item's name.
    pub label: &'static str,
    /// In Tauri's accelerator spelling, so one string serves the menu.
    pub accelerator: &'static str,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The catalogue key of the menu's name.
    pub title: &'static str,
    pub items: Vec<Item>,
}

/// The menus after the app's own, in order. Every item is also in the palette.
pub fn sections() -> Vec<Section> {
    vec![
        Section {
            title: "hub_menu_file",
            items: vec![
                Item {
                    id: "new-note",
                    label: "hub_menu_new_note",
                    accelerator: "Alt+CmdOrCtrl+N",
                    action: Action::Web("hq:open-quick-note"),
                },
                Item {
                    id: "new-chat",
                    label: "hub_menu_new_chat",
                    accelerator: "Shift+CmdOrCtrl+O",
                    action: Action::Web("hq:new-chat"),
                },
            ],
        },
        Section {
            title: "hub_menu_go",
            items: vec![
                Item {
                    id: "go-vault",
                    label: "hub_menu_vault",
                    accelerator: "CmdOrCtrl+1",
                    action: Action::Go("/notes"),
                },
                Item {
                    id: "go-chat",
                    label: "hub_menu_chat",
                    accelerator: "CmdOrCtrl+2",
                    action: Action::Go("/chat"),
                },
                Item {
                    id: "go-inbox",
                    label: "hub_menu_inbox",
                    accelerator: "CmdOrCtrl+3",
                    action: Action::Go("/inbox"),
                },
                Item {
                    id: "palette",
                    label: "hub_menu_palette",
                    accelerator: "CmdOrCtrl+K",
                    action: Action::Web("hq:open-search"),
                },
            ],
        },
        Section {
            title: "hub_menu_view",
            items: vec![Item {
                id: "pane",
                label: "hub_menu_pane",
                accelerator: "CmdOrCtrl+.",
                action: Action::Web("hq:toggle-pane"),
            }],
        },
    ]
}

/// The name of an item or menu, from the shared catalogue.
pub fn name(key: &str) -> String {
    word(key)
}

/// The item with this id, for the shell's menu handler.
pub fn find(id: &str) -> Option<Item> {
    sections()
        .into_iter()
        .flat_map(|s| s.items)
        .find(|i| i.id == id)
}

/// The script that sends the page one of the menu's events. `None` for any other name, so a menu item
/// can never run anything else in the window.
pub fn web_event_script(event: &str) -> Option<String> {
    WEB_EVENTS
        .contains(&event)
        .then(|| format!("window.dispatchEvent(new Event('{event}'))"))
}

/// The Dock badge: the number of decisions waiting for the owner, and nothing when none wait. The
/// system draws it, and updates never count.
pub fn dock_badge(waiting: usize) -> Option<String> {
    match waiting {
        0 => None,
        1..=99 => Some(waiting.to_string()),
        _ => Some("99+".to_string()),
    }
}

/// Past this many waiting items the system notice stops listing them one by one.
pub const NOTICE_LIST_MAX: usize = 5;

/// The single line that stands for a pile, once more than `NOTICE_LIST_MAX` are waiting.
pub fn pile_notice(waiting: usize) -> Option<String> {
    (waiting > NOTICE_LIST_MAX)
        .then(|| word("hub_notice_more").replace("{n}", &waiting.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::main_path_allowed;
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn every_place_an_item_opens_is_one_the_window_allows() {
        for item in sections().into_iter().flat_map(|s| s.items) {
            if let Action::Go(path) = item.action {
                assert!(
                    main_path_allowed(path).is_some(),
                    "{} opens {path}",
                    item.id
                );
            }
        }
    }

    #[test]
    fn ids_and_keys_are_each_used_once() {
        let items: Vec<Item> = sections().into_iter().flat_map(|s| s.items).collect();
        assert_eq!(
            items.iter().map(|i| i.id).collect::<HashSet<_>>().len(),
            items.len()
        );
        assert_eq!(
            items
                .iter()
                .map(|i| i.accelerator)
                .collect::<HashSet<_>>()
                .len(),
            items.len()
        );
    }

    #[test]
    fn the_three_places_take_command_1_2_and_3() {
        let key = |id: &str| find(id).unwrap().accelerator;
        assert_eq!(key("go-vault"), "CmdOrCtrl+1");
        assert_eq!(key("go-chat"), "CmdOrCtrl+2");
        assert_eq!(key("go-inbox"), "CmdOrCtrl+3");
    }

    #[test]
    fn every_name_is_in_the_catalogue() {
        for section in sections() {
            assert!(!name(section.title).is_empty());
            for item in section.items {
                assert!(!name(item.label).is_empty(), "{}", item.id);
            }
        }
    }

    #[test]
    fn a_web_item_sends_only_an_event_the_page_listens_for() {
        for item in sections().into_iter().flat_map(|s| s.items) {
            if let Action::Web(event) = item.action {
                assert!(web_event_script(event).is_some(), "{event}");
            }
        }
        assert!(web_event_script("hq:anything-else").is_none());
        assert!(web_event_script("x');alert(1);//").is_none());
    }

    #[test]
    fn the_page_really_listens_for_each_event() {
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/hq-web/src");
        let mut all = String::new();
        for rel in [
            "components/SearchOverlay.tsx",
            "components/QuickNoteOverlay.tsx",
            "components/chat/ChatView.tsx",
        ] {
            all.push_str(&std::fs::read_to_string(web.join(rel)).unwrap_or_default());
        }
        for event in WEB_EVENTS {
            assert!(
                all.contains(&format!("'{event}'")),
                "no listener for {event}"
            );
        }
    }

    #[test]
    fn the_badge_counts_decisions_and_vanishes_at_zero() {
        assert_eq!(dock_badge(0), None);
        assert_eq!(dock_badge(4).as_deref(), Some("4"));
        assert_eq!(dock_badge(99).as_deref(), Some("99"));
        assert_eq!(dock_badge(100).as_deref(), Some("99+"));
    }

    #[test]
    fn a_pile_gets_one_line_past_five() {
        assert_eq!(pile_notice(5), None);
        assert_eq!(pile_notice(6).as_deref(), Some("6 more waiting for you"));
    }
}
