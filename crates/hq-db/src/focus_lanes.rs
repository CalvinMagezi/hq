//! The few lanes of work the owner has chosen to move forward. Declared by the owner, never
//! changed by HQ: HQ may suggest a swap, nothing more. Everything outside a lane is parked.
//! Stays on this device (`sql/141_focus_lanes.sql`).

use anyhow::{Result, bail};
use rusqlite::params;

use crate::Database;
use crate::owner_prefs;

/// Lanes allowed when the owner has not chosen a number.
pub const DEFAULT_MAX_LANES: usize = 3;
/// The most lanes the owner may ask for.
pub const CEILING_LANES: usize = 5;
const MAX_NAME_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub name: String,
    pub parked: bool,
}

/// How many active lanes are allowed.
pub fn max_lanes(db: &Database) -> usize {
    owner_prefs::get(db, owner_prefs::FOCUS_MAX_LANES)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(DEFAULT_MAX_LANES, |n| n.clamp(1, CEILING_LANES))
}

pub fn list(db: &Database) -> Result<Vec<Lane>> {
    db.with_conn(|c| {
        let rows = c
            .prepare("SELECT lane, parked_at IS NOT NULL FROM focus_lanes ORDER BY added_at, lane")?
            .query_map([], |r| {
                Ok(Lane {
                    name: r.get(0)?,
                    parked: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    })
}

/// Makes `name` an active lane, or brings a parked one back. Refused above the maximum: park or
/// remove a lane first.
pub fn add(db: &Database, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        bail!("a lane name is 1 to {MAX_NAME_CHARS} characters");
    }
    let active = list(db)?
        .iter()
        .filter(|l| !l.parked && l.name != name)
        .count();
    let max = max_lanes(db);
    if active >= max {
        bail!("{max} lanes are active already; park or remove one first (`hq focus park <lane>`)");
    }
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO focus_lanes (lane, added_at) VALUES (?1, datetime('now'))
             ON CONFLICT(lane) DO UPDATE SET parked_at = NULL",
            params![name],
        )?;
        Ok(())
    })
}

/// Sets `name` aside without forgetting it. False when there is no such lane.
pub fn park(db: &Database, name: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE focus_lanes SET parked_at = datetime('now') WHERE lane = ?1 AND parked_at IS NULL",
            params![name.trim()],
        )? == 1)
    })
}

pub fn remove(db: &Database, name: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.execute(
            "DELETE FROM focus_lanes WHERE lane = ?1",
            params![name.trim()],
        )? == 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_lanes_by_default_and_the_fourth_is_refused_until_one_is_parked() {
        let db = Database::open_memory().unwrap();
        for n in ["Alpha", "Beta", "Gamma"] {
            add(&db, n).unwrap();
        }
        assert!(add(&db, "Delta").is_err());
        assert!(park(&db, "Beta").unwrap());
        add(&db, "Delta").unwrap();
        assert!(
            add(&db, "Beta").is_err(),
            "bringing Beta back would be a fourth active lane"
        );
        let lanes = list(&db).unwrap();
        assert!(lanes.iter().any(|l| l.name == "Beta" && l.parked));
    }

    #[test]
    fn the_limit_follows_the_owners_choice_up_to_the_ceiling() {
        let db = Database::open_memory().unwrap();
        owner_prefs::set(&db, owner_prefs::FOCUS_MAX_LANES, "5").unwrap();
        assert_eq!(max_lanes(&db), 5);
        assert!(owner_prefs::set(&db, owner_prefs::FOCUS_MAX_LANES, "6").is_err());
        assert!(owner_prefs::set(&db, owner_prefs::FOCUS_MAX_LANES, "0").is_err());
    }

    #[test]
    fn names_are_checked_and_remove_forgets_a_lane() {
        let db = Database::open_memory().unwrap();
        assert!(add(&db, "  ").is_err());
        assert!(add(&db, &"x".repeat(61)).is_err());
        add(&db, "Alpha").unwrap();
        assert!(remove(&db, "Alpha").unwrap());
        assert!(!remove(&db, "Alpha").unwrap());
        assert!(list(&db).unwrap().is_empty());
    }
}
