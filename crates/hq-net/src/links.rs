//! The hub's list of device pairs it may forward between, and what the applier hands to `nft`.
//!
//! The firewall rules themselves are not the applier's to write. The owner installs a fixed table
//! once (step 3 of `hq net init`): it drops everything that crosses the tunnel interface except a
//! packet whose source and destination are a pair in one named set. The applier only replaces the
//! contents of that set, with addresses it took from a validated file, so a broken or missing
//! applier leaves the set as it was and an empty set forwards nothing.
//!
//! The set holds directed pairs (source and destination). A link is one pair of devices, so it
//! becomes two elements, one for each direction.

use crate::grammar::Link;
use crate::names::valid_interface;
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;

/// The nftables family and table the owner's rules live in.
pub const TABLE_FAMILY: &str = "inet";
pub const TABLE: &str = "hq_net";
const SET_PREFIX: &str = "links_";
const ARROW: &str = "<->";

/// A directed pair: traffic may go from the first address to the second.
pub type Pair = (Ipv4Addr, Ipv4Addr);

/// The set that holds the pairs for `interface`, such as `links_hq0`. `None` unless the name is
/// one the applier accepts as an interface, so it can never carry anything but lowercase letters,
/// digits and `_` into an nft command.
pub fn set_name(interface: &str) -> Option<String> {
    valid_interface(interface).then(|| format!("{SET_PREFIX}{interface}"))
}

/// Every directed pair the links allow, in order.
pub fn directed(links: &[Link]) -> BTreeSet<Pair> {
    links
        .iter()
        .flat_map(|l| [(l.a, l.b), (l.b, l.a)])
        .collect()
}

/// The script for `nft -f -`: empty the set, then add every pair. One file is one transaction, so
/// the set is never seen half filled, and if any line fails nothing changes.
pub fn script(interface: &str, links: &[Link]) -> Option<String> {
    let set = set_name(interface)?;
    let mut out = format!("flush set {TABLE_FAMILY} {TABLE} {set}\n");
    let pairs = directed(links);
    if !pairs.is_empty() {
        let elements: Vec<String> = pairs.iter().map(|(s, d)| format!("{s} . {d}")).collect();
        out.push_str(&format!(
            "add element {TABLE_FAMILY} {TABLE} {set} {{ {} }}\n",
            elements.join(", ")
        ));
    }
    Some(out)
}

/// The arguments for `nft` that print the set.
pub fn list_args(interface: &str) -> Option<[String; 5]> {
    let set = set_name(interface)?;
    Some([
        "list".into(),
        "set".into(),
        TABLE_FAMILY.into(),
        TABLE.into(),
        set,
    ])
}

fn pair_of(element: &str) -> Option<Pair> {
    // The separator is " . " with spaces: a dot inside an address must not split it.
    let (source, destination) = element.split_once(" . ")?;
    Some((
        crate::subnet::parse_canonical_ipv4(source.trim())?,
        crate::subnet::parse_canonical_ipv4(destination.trim())?,
    ))
}

/// The pairs in the output of `nft list set`, or `None` if the output is not in a shape that can
/// be read with certainty (anything but plain `a . b` elements). `None` only costs a fuller
/// journal entry: the applier does not decide anything from this.
pub fn current_pairs(listing: &str) -> Option<BTreeSet<Pair>> {
    let flat: String = listing.lines().map(str::trim).collect::<Vec<_>>().join(" ");
    let Some(at) = flat.find("elements = {") else {
        return flat.contains("set ").then(BTreeSet::new);
    };
    let rest = &flat[at + "elements = {".len()..];
    let close = rest.find('}')?;
    let body = rest[..close].trim();
    if body.is_empty() {
        return Some(BTreeSet::new());
    }
    body.split(',')
        .map(|element| pair_of(element.trim()))
        .collect()
}

/// How much of a pair is in the set: both directions, or only one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    Both,
    OneWay,
}

/// The pairs in `set`, each as its lower and higher address, with how much of it is there.
fn presence(set: &BTreeSet<Pair>) -> BTreeMap<Pair, Presence> {
    let mut out = BTreeMap::new();
    for &(from, to) in set {
        let key = (from.min(to), from.max(to));
        let both = set.contains(&(to, from));
        out.insert(
            key,
            if both {
                Presence::Both
            } else {
                Presence::OneWay
            },
        );
    }
    out
}

/// One journal line per pair that was added, completed or removed, with both directions of a link
/// on one line. `previous` is `None` when the set could not be read, and then every pair is listed
/// as set.
pub fn changes(previous: Option<&BTreeSet<Pair>>, links: &[Link]) -> Vec<String> {
    let line = |verb: &str, (a, b): Pair| format!("{verb} {a} {ARROW} {b}");
    let Some(previous) = previous else {
        if links.is_empty() {
            return vec!["cleared the list (what it held before could not be read)".to_string()];
        }
        return links.iter().map(|l| line("set", (l.a, l.b))).collect();
    };
    let before = presence(previous);
    let wanted: BTreeSet<Pair> = links.iter().map(|l| (l.a, l.b)).collect();
    let mut out = Vec::new();
    for pair in &wanted {
        match before.get(pair) {
            Some(Presence::Both) => {}
            Some(Presence::OneWay) => out.push(line("allowed the missing direction of", *pair)),
            None => out.push(line("allowed", *pair)),
        }
    }
    for (pair, state) in &before {
        if wanted.contains(pair) {
            continue;
        }
        let verb = match state {
            Presence::Both => "removed",
            Presence::OneWay => "removed one direction of",
        };
        out.push(line(verb, *pair));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(host: u8) -> Ipv4Addr {
        Ipv4Addr::new(10, 77, 0, host)
    }

    fn link(a: u8, b: u8) -> Link {
        Link::new(ip(a), ip(b)).unwrap()
    }

    #[test]
    fn the_set_name_is_derived_only_from_a_valid_interface() {
        assert_eq!(set_name("hq0").as_deref(), Some("links_hq0"));
        for bad in [
            "",
            "HQ0",
            "hq0; flush ruleset",
            "hq 0",
            "0hq",
            "hq0\n",
            "a b",
            "-x",
        ] {
            assert_eq!(set_name(bad), None, "{bad:?}");
            assert!(script(bad, &[link(2, 3)]).is_none());
            assert!(list_args(bad).is_none());
        }
    }

    #[test]
    fn the_script_flushes_then_adds_both_directions_of_every_link_in_one_file() {
        let text = script("hq0", &[link(5, 2), link(2, 3)]).unwrap();
        assert_eq!(
            text,
            "flush set inet hq_net links_hq0\n\
             add element inet hq_net links_hq0 { 10.77.0.2 . 10.77.0.3, 10.77.0.2 . 10.77.0.5, \
             10.77.0.3 . 10.77.0.2, 10.77.0.5 . 10.77.0.2 }\n"
        );
    }

    #[test]
    fn no_links_flushes_the_set_and_adds_nothing() {
        assert_eq!(
            script("hq0", &[]).unwrap(),
            "flush set inet hq_net links_hq0\n"
        );
    }

    #[test]
    fn the_script_holds_nothing_but_addresses_names_and_fixed_words() {
        let text = script("hq0", &[link(2, 3), link(3, 4)]).unwrap();
        let allowed = |c: char| c.is_ascii_alphanumeric() || " .,{}_\n".contains(c);
        assert!(text.chars().all(allowed), "{text:?}");
        assert_eq!(text.lines().count(), 2);
    }

    const LISTING: &str = "table inet hq_net {\n\tset links_hq0 {\n\t\ttype ipv4_addr . ipv4_addr\n\t\telements = { 10.77.0.2 . 10.77.0.5,\n\t\t\t     10.77.0.5 . 10.77.0.2 }\n\t}\n}\n";

    #[test]
    fn the_listing_nft_prints_is_read_back_including_a_wrapped_line() {
        let pairs = current_pairs(LISTING).unwrap();
        assert_eq!(pairs, directed(&[link(2, 5)]));
    }

    #[test]
    fn an_empty_set_reads_as_empty_and_an_unreadable_listing_as_unknown() {
        let empty =
            "table inet hq_net {\n\tset links_hq0 {\n\t\ttype ipv4_addr . ipv4_addr\n\t}\n}\n";
        assert_eq!(current_pairs(empty), Some(BTreeSet::new()));
        assert_eq!(current_pairs(""), None);
        assert_eq!(current_pairs("Error: No such file or directory"), None);
        let decorated =
            "set links_hq0 {\n elements = { 10.77.0.2 . 10.77.0.5 counter packets 1 bytes 2 }\n}";
        assert_eq!(current_pairs(decorated), None);
        let wrong_shape = "set links_hq0 {\n elements = { 10.77.0.2 }\n}";
        assert_eq!(current_pairs(wrong_shape), None);
        let not_canonical = "set links_hq0 {\n elements = { 10.077.0.2 . 10.77.0.5 }\n}";
        assert_eq!(current_pairs(not_canonical), None);
    }

    #[test]
    fn changes_name_what_was_allowed_and_what_was_removed() {
        let before = directed(&[link(2, 5), link(2, 3)]);
        let lines = changes(Some(&before), &[link(2, 5), link(3, 4)]);
        assert_eq!(
            lines,
            vec![
                "allowed 10.77.0.3 <-> 10.77.0.4".to_string(),
                "removed 10.77.0.2 <-> 10.77.0.3".to_string(),
            ]
        );
        assert!(changes(Some(&before), &[link(2, 3), link(2, 5)]).is_empty());
    }

    #[test]
    fn clearing_a_set_that_could_not_be_read_is_still_journaled() {
        assert_eq!(
            changes(None, &[]),
            vec!["cleared the list (what it held before could not be read)".to_string()]
        );
    }

    #[test]
    fn an_unknown_previous_state_lists_every_pair_and_a_one_way_leftover_is_reported() {
        assert_eq!(
            changes(None, &[link(2, 5)]),
            vec!["set 10.77.0.2 <-> 10.77.0.5".to_string()]
        );
        let one_way: BTreeSet<Pair> = [(ip(5), ip(2))].into_iter().collect();
        let lines = changes(Some(&one_way), &[link(2, 5)]);
        assert_eq!(
            lines,
            vec!["allowed the missing direction of 10.77.0.2 <-> 10.77.0.5".to_string()]
        );
        let extra: BTreeSet<Pair> = [(ip(2), ip(5)), (ip(5), ip(2)), (ip(6), ip(3))]
            .into_iter()
            .collect();
        let lines = changes(Some(&extra), &[link(2, 5)]);
        assert_eq!(
            lines,
            vec!["removed one direction of 10.77.0.3 <-> 10.77.0.6".to_string()]
        );
    }
}
