//! When a reminder rings, in the owner's own zone. Pure: every function takes `now` or the times
//! it works from, so tests drive it with fixed instants.

use anyhow::{Result, bail};
use chrono::{Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;
use hq_core::config::RemindersConfig;
use hq_db::reminders::{Reminder, STATUS_ACTIVE, STATUS_DONE, STATUS_LAPSED};
use serde::{Deserialize, Serialize};

/// Pings a nudge sends before it gives up: the first at the due time, three follow-ups, and a last
/// one that asks whether to reschedule or drop it.
pub const NUDGE_PINGS: i64 = 5;
/// A ping this long after its time is labelled as late (the daemon was down, or the phone was off).
const LATE_AFTER_SECS: i64 = 300;
const FIRST_FOLLOW_UP_SECS: i64 = 30 * 60;
const SECOND_FOLLOW_UP_SECS: i64 = 2 * 3600;
/// How far ahead a repeat is searched before giving up.
const SCAN_DAYS: i64 = 800;

fn one() -> u32 {
    1
}

/// How a reminder repeats. The time of day is the one the reminder was first set for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Repeat {
    /// Every `every` days.
    Daily {
        #[serde(default = "one")]
        every: u32,
    },
    /// Monday to Friday.
    Weekdays,
    /// On these days, 1 = Monday to 7 = Sunday.
    Weekly { days: Vec<u8> },
    /// On this day of the month; a short month rings on its last day.
    Monthly { day: u8 },
}

impl Repeat {
    pub fn parse(json: &str) -> Result<Self> {
        let rule: Self = serde_json::from_str(json)?;
        rule.validate()?;
        Ok(rule)
    }

    /// The short form a person types: `daily`, `every:3` (days), `weekdays`, `weekly:1,4`
    /// (Monday and Thursday), `monthly:15`.
    pub fn from_spec(spec: &str) -> Result<Self> {
        let spec = spec.trim().to_lowercase();
        let (name, arg) = spec.split_once(':').unwrap_or((spec.as_str(), ""));
        let rule = match name {
            "daily" => Self::Daily { every: 1 },
            "every" => Self::Daily {
                every: arg.parse()?,
            },
            "weekdays" => Self::Weekdays,
            "weekly" => Self::Weekly {
                days: arg
                    .split(',')
                    .map(|d| d.trim().parse::<u8>())
                    .collect::<Result<_, _>>()?,
            },
            "monthly" => Self::Monthly { day: arg.parse()? },
            _ => bail!("repeat is one of daily, every:N, weekdays, weekly:1,4, monthly:15"),
        };
        rule.validate()?;
        Ok(rule)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Daily { every } if !(1..=365).contains(every) => {
                bail!("a daily repeat needs every to be 1 to 365")
            }
            Self::Weekly { days }
                if days.is_empty() || days.iter().any(|d| !(1..=7).contains(d)) =>
            {
                bail!("a weekly repeat needs days between 1 (Monday) and 7 (Sunday)")
            }
            Self::Monthly { day } if !(1..=31).contains(day) => {
                bail!("a monthly repeat needs a day of the month from 1 to 31")
            }
            _ => Ok(()),
        }
    }

    /// The rule in the words the owner would use.
    pub fn describe(&self) -> String {
        match self {
            Self::Daily { every: 1 } => "every day".to_string(),
            Self::Daily { every } => format!("every {every} days"),
            Self::Weekdays => "every weekday".to_string(),
            Self::Weekly { days } => {
                let names: Vec<&str> = days
                    .iter()
                    .map(|d| match d {
                        1 => "Mon",
                        2 => "Tue",
                        3 => "Wed",
                        4 => "Thu",
                        5 => "Fri",
                        6 => "Sat",
                        _ => "Sun",
                    })
                    .collect();
                format!("every {}", names.join(", "))
            }
            Self::Monthly { day } => format!("on day {day} of each month"),
        }
    }

    fn matches(&self, anchor: NaiveDate, date: NaiveDate) -> bool {
        match self {
            Self::Daily { every } => (date - anchor).num_days() % i64::from(*every) == 0,
            Self::Weekdays => date.weekday().number_from_monday() <= 5,
            Self::Weekly { days } => {
                let today = u8::try_from(date.weekday().number_from_monday()).unwrap_or(0);
                days.contains(&today)
            }
            Self::Monthly { day } => u32::from(*day).min(days_in_month(date)) == date.day(),
        }
    }
}

fn days_in_month(date: NaiveDate) -> u32 {
    let (y, m) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .and_then(|first_of_next| first_of_next.pred_opt())
        .map_or(30, |last| last.day())
}

/// The instant of a wall-clock time in `tz`. A time a clock change skipped lands just after the
/// gap; one that happens twice takes the first.
pub fn resolve_wall_clock(tz: Tz, naive: NaiveDateTime) -> i64 {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(t) => t.timestamp(),
        LocalResult::Ambiguous(first, _) => first.timestamp(),
        LocalResult::None => {
            let mut later = naive;
            for _ in 0..4 {
                later += Duration::minutes(30);
                if let Some(t) = tz.from_local_datetime(&later).earliest() {
                    return t.timestamp();
                }
            }
            tz.from_utc_datetime(&naive).timestamp()
        }
    }
}

fn local_date(tz: Tz, epoch: i64) -> NaiveDate {
    tz.timestamp_opt(epoch, 0)
        .single()
        .map_or_else(|| chrono::Utc::now().date_naive(), |t| t.date_naive())
}

/// The next time a repeat rings after `after` (and after `occurrence_at`), at the time of day of
/// `occurrence_at`. None only when no day within about two years matches.
pub fn next_occurrence(rule: &Repeat, after: i64, occurrence_at: i64, tz: Tz) -> Option<i64> {
    let first = tz.timestamp_opt(occurrence_at, 0).single()?;
    let anchor = first.date_naive();
    let time = first.time();
    let from = anchor.max(local_date(tz, after) - Duration::days(1));
    (0..SCAN_DAYS).find_map(|offset| {
        let date = from + Duration::days(offset);
        if !rule.matches(anchor, date) {
            return None;
        }
        let at = resolve_wall_clock(tz, date.and_time(time));
        (at > after && at > occurrence_at).then_some(at)
    })
}

/// The next `hour`:00 in `tz` strictly after `after`.
pub fn next_local_hour(tz: Tz, after: i64, hour: u32) -> i64 {
    let time = NaiveTime::from_hms_opt(hour.min(23), 0, 0).unwrap_or(NaiveTime::MIN);
    let today = local_date(tz, after);
    let candidate = resolve_wall_clock(tz, today.and_time(time));
    if candidate > after {
        candidate
    } else {
        resolve_wall_clock(tz, (today + Duration::days(1)).and_time(time))
    }
}

/// Hours in which follow-up pings wait for the morning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietHours {
    start: NaiveTime,
    end: NaiveTime,
}

impl QuietHours {
    /// `21:00-07:00`. Empty or malformed means no quiet hours.
    pub fn parse(spec: &str) -> Option<Self> {
        let (a, b) = spec.trim().split_once('-')?;
        let start = NaiveTime::parse_from_str(a.trim(), "%H:%M").ok()?;
        let end = NaiveTime::parse_from_str(b.trim(), "%H:%M").ok()?;
        (start != end).then_some(Self { start, end })
    }

    fn wraps(self) -> bool {
        self.start > self.end
    }

    /// `at` itself, or the end of the quiet window it falls in.
    pub fn hold(self, tz: Tz, at: i64) -> i64 {
        let Some(local) = tz.timestamp_opt(at, 0).single() else {
            return at;
        };
        let time = local.time();
        let inside = if self.wraps() {
            time >= self.start || time < self.end
        } else {
            time >= self.start && time < self.end
        };
        if !inside {
            return at;
        }
        let date = local.date_naive();
        let end_date = if self.wraps() && time >= self.start {
            date + Duration::days(1)
        } else {
            date
        };
        resolve_wall_clock(tz, end_date.and_time(self.end))
    }
}

/// When the follow-up after the `ping_no`-th ping rings: 30 minutes, 2 hours, then the next two
/// mornings. Held through quiet hours. None once the ladder is used up.
pub fn follow_up(ping_no: i64, now: i64, tz: Tz, cfg: &RemindersConfig) -> Option<i64> {
    let at = match ping_no {
        1 => now + FIRST_FOLLOW_UP_SECS,
        2 => now + SECOND_FOLLOW_UP_SECS,
        3 | 4 => next_local_hour(tz, now, u32::from(cfg.follow_up_hour)),
        _ => return None,
    };
    Some(QuietHours::parse(&cfg.quiet_hours).map_or(at, |q| q.hold(tz, at)))
}

/// What one ping does to its reminder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub status: &'static str,
    pub next_due_at: i64,
    pub occurrence_at: i64,
    pub nudge_step: i64,
    /// Which ping this is for the current occurrence, from 1.
    pub ping_no: i64,
    /// The last ping of a nudge: it asks whether to reschedule or drop the reminder.
    pub last_ping: bool,
    /// It rings well after its time.
    pub late: bool,
}

pub fn zone_of(reminder: &Reminder) -> Tz {
    reminder.tz.parse().unwrap_or(Tz::UTC)
}

/// Decides what ringing `reminder` at `now` changes. A repeat skips any occurrence that has
/// already passed, so a daemon that was down does not ring a week of them at once.
pub fn plan_fire(reminder: &Reminder, now: i64, cfg: &RemindersConfig) -> Plan {
    let tz = zone_of(reminder);
    let late = now - reminder.due_at > LATE_AFTER_SECS;
    let next_occ = reminder
        .repeat_rule
        .as_deref()
        .and_then(|json| Repeat::parse(json).ok())
        .and_then(|rule| next_occurrence(&rule, now, reminder.occurrence_at, tz));
    let ping_no = reminder.nudge_step + 1;
    let roll = |next: i64, ping_no: i64, last_ping: bool| Plan {
        status: STATUS_ACTIVE,
        next_due_at: next,
        occurrence_at: next,
        nudge_step: 0,
        ping_no,
        last_ping,
        late,
    };
    if !reminder.nudge {
        return match next_occ {
            Some(next) => roll(next, 1, false),
            None => Plan {
                status: STATUS_DONE,
                next_due_at: reminder.due_at,
                occurrence_at: reminder.occurrence_at,
                nudge_step: 0,
                ping_no: 1,
                last_ping: false,
                late,
            },
        };
    }
    let last_ping = ping_no >= NUDGE_PINGS;
    let follow = if last_ping {
        None
    } else {
        follow_up(ping_no, now, tz, cfg)
    };
    match (follow, next_occ) {
        // The next occurrence comes before the follow-up would: the ladder starts over there.
        (Some(f), Some(n)) if n <= f => roll(n, ping_no, last_ping),
        (Some(f), _) => Plan {
            status: STATUS_ACTIVE,
            next_due_at: f,
            occurrence_at: reminder.occurrence_at,
            nudge_step: ping_no,
            ping_no,
            last_ping,
            late,
        },
        (None, Some(n)) => roll(n, ping_no, last_ping),
        (None, None) => Plan {
            status: STATUS_LAPSED,
            next_due_at: reminder.due_at,
            occurrence_at: reminder.occurrence_at,
            nudge_step: ping_no,
            ping_no,
            last_ping,
            late,
        },
    }
}

/// The time to carry a repeating reminder to once the owner is done with this occurrence.
pub fn after_done(reminder: &Reminder, now: i64) -> Option<i64> {
    let rule = Repeat::parse(reminder.repeat_rule.as_deref()?).ok()?;
    next_occurrence(&rule, now, reminder.occurrence_at, zone_of(reminder))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::{Africa::Kampala, Europe::London};

    fn at(tz: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp()
    }

    fn cfg() -> RemindersConfig {
        RemindersConfig::default()
    }

    fn reminder(due: i64, nudge: bool, repeat: Option<Repeat>) -> Reminder {
        Reminder {
            id: "r1".into(),
            title: "Call Moses".into(),
            note: None,
            status: STATUS_ACTIVE.into(),
            due_at: due,
            occurrence_at: due,
            tz: "Africa/Kampala".into(),
            repeat_rule: repeat.map(|r| r.to_json()),
            nudge,
            nudge_step: 0,
            fire_seq: 0,
            last_item_id: None,
            task_id: None,
            thread_id: None,
            tainted: false,
            source: "chat".into(),
            created_at: 0,
            updated_at: 0,
            last_fired_at: None,
            acked_at: None,
        }
    }

    #[test]
    fn a_kampala_time_is_three_hours_ahead_of_utc() {
        let naive = NaiveDate::from_ymd_opt(2026, 11, 26)
            .unwrap()
            .and_hms_opt(16, 0, 0)
            .unwrap();
        let utc = chrono::Utc
            .with_ymd_and_hms(2026, 11, 26, 13, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(resolve_wall_clock(Kampala, naive), utc);
    }

    #[test]
    fn a_time_the_clock_skipped_rings_after_the_gap_and_a_repeated_one_rings_first() {
        // London springs forward at 01:00 on 29 March 2026: 01:30 does not exist.
        let skipped = NaiveDate::from_ymd_opt(2026, 3, 29)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        let after_gap = chrono::Utc
            .with_ymd_and_hms(2026, 3, 29, 1, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(resolve_wall_clock(London, skipped), after_gap);
        // It falls back at 02:00 on 25 October: 01:30 happens twice, and the first is BST.
        let twice = NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        let first = chrono::Utc
            .with_ymd_and_hms(2026, 10, 25, 0, 30, 0)
            .unwrap()
            .timestamp();
        assert_eq!(resolve_wall_clock(London, twice), first);
    }

    #[test]
    fn a_daily_repeat_keeps_its_wall_clock_time_across_a_clock_change() {
        let occ = at(London, 2026, 3, 28, 9, 0);
        let next = next_occurrence(&Repeat::Daily { every: 1 }, occ, occ, London).unwrap();
        assert_eq!(next, at(London, 2026, 3, 29, 9, 0));
        assert_eq!(
            next - occ,
            23 * 3600,
            "the day the clocks went forward is 23 hours"
        );
    }

    #[test]
    fn weekdays_skip_the_weekend_and_weekly_picks_its_days() {
        // 27 November 2026 is a Friday.
        let fri = at(Kampala, 2026, 11, 27, 16, 0);
        assert_eq!(
            next_occurrence(&Repeat::Weekdays, fri, fri, Kampala),
            Some(at(Kampala, 2026, 11, 30, 16, 0))
        );
        let weekly = Repeat::Weekly { days: vec![1, 4] };
        let thu = at(Kampala, 2026, 11, 26, 9, 0);
        assert_eq!(
            next_occurrence(&weekly, thu, thu, Kampala),
            Some(at(Kampala, 2026, 11, 30, 9, 0))
        );
    }

    #[test]
    fn monthly_clamps_to_the_last_day_of_a_short_month() {
        let jan31 = at(Kampala, 2027, 1, 31, 8, 0);
        assert_eq!(
            next_occurrence(&Repeat::Monthly { day: 31 }, jan31, jan31, Kampala),
            Some(at(Kampala, 2027, 2, 28, 8, 0))
        );
    }

    #[test]
    fn a_repeat_skips_occurrences_that_passed_while_nothing_ran() {
        let occ = at(Kampala, 2026, 11, 1, 9, 0);
        let now = at(Kampala, 2026, 11, 20, 12, 0);
        assert_eq!(
            next_occurrence(&Repeat::Daily { every: 1 }, now, occ, Kampala),
            Some(at(Kampala, 2026, 11, 21, 9, 0))
        );
        assert_eq!(
            next_occurrence(&Repeat::Daily { every: 7 }, now, occ, Kampala),
            Some(at(Kampala, 2026, 11, 22, 9, 0))
        );
    }

    #[test]
    fn the_short_form_a_person_types_becomes_a_rule() {
        assert_eq!(
            Repeat::from_spec("daily").unwrap(),
            Repeat::Daily { every: 1 }
        );
        assert_eq!(
            Repeat::from_spec("every:3").unwrap(),
            Repeat::Daily { every: 3 }
        );
        assert_eq!(Repeat::from_spec(" Weekdays ").unwrap(), Repeat::Weekdays);
        assert_eq!(
            Repeat::from_spec("weekly:1, 4").unwrap(),
            Repeat::Weekly { days: vec![1, 4] }
        );
        assert_eq!(
            Repeat::from_spec("monthly:15").unwrap(),
            Repeat::Monthly { day: 15 }
        );
        for bad in ["", "hourly", "weekly:", "weekly:9", "monthly:40", "every:0"] {
            assert!(Repeat::from_spec(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repeat_rules_are_validated_and_round_trip() {
        assert!(Repeat::parse(r#"{"kind":"weekly","days":[]}"#).is_err());
        assert!(Repeat::parse(r#"{"kind":"weekly","days":[8]}"#).is_err());
        assert!(Repeat::parse(r#"{"kind":"monthly","day":0}"#).is_err());
        assert!(Repeat::parse(r#"{"kind":"daily","every":0}"#).is_err());
        let rule = Repeat::Weekly { days: vec![1, 3] };
        assert_eq!(Repeat::parse(&rule.to_json()).unwrap(), rule);
        assert_eq!(rule.describe(), "every Mon, Wed");
        assert_eq!(
            Repeat::parse(r#"{"kind":"daily"}"#).unwrap().describe(),
            "every day"
        );
    }

    #[test]
    fn quiet_hours_hold_a_ping_to_the_end_of_the_window() {
        let q = QuietHours::parse("21:00-07:00").unwrap();
        let late_evening = at(Kampala, 2026, 11, 26, 21, 20);
        assert_eq!(
            q.hold(Kampala, late_evening),
            at(Kampala, 2026, 11, 27, 7, 0)
        );
        let small_hours = at(Kampala, 2026, 11, 27, 3, 0);
        assert_eq!(
            q.hold(Kampala, small_hours),
            at(Kampala, 2026, 11, 27, 7, 0)
        );
        let noon = at(Kampala, 2026, 11, 26, 12, 0);
        assert_eq!(q.hold(Kampala, noon), noon);
        assert!(QuietHours::parse("").is_none());
        assert!(QuietHours::parse("nonsense").is_none());
        assert!(QuietHours::parse("08:00-08:00").is_none());
    }

    #[test]
    fn a_one_shot_is_done_after_it_rings_and_says_so_when_late() {
        let due = at(Kampala, 2026, 11, 26, 16, 0);
        let on_time = plan_fire(&reminder(due, false, None), due + 20, &cfg());
        assert_eq!(on_time.status, STATUS_DONE);
        assert!(!on_time.late);
        let late = plan_fire(&reminder(due, false, None), due + 3600, &cfg());
        assert!(late.late);
    }

    #[test]
    fn a_repeating_reminder_rolls_to_its_next_occurrence() {
        let due = at(Kampala, 2026, 11, 26, 16, 0);
        let rule = Repeat::Daily { every: 1 };
        let plan = plan_fire(&reminder(due, false, Some(rule)), due, &cfg());
        assert_eq!(plan.status, STATUS_ACTIVE);
        assert_eq!(plan.next_due_at, at(Kampala, 2026, 11, 27, 16, 0));
        assert_eq!(plan.occurrence_at, plan.next_due_at);
    }

    #[test]
    fn a_nudge_climbs_its_ladder_then_lapses() {
        let due = at(Kampala, 2026, 11, 26, 16, 0);
        let mut r = reminder(due, true, None);
        let mut now = due;
        let mut dues = Vec::new();
        for expected_ping in 1..=NUDGE_PINGS {
            let plan = plan_fire(&r, now, &cfg());
            assert_eq!(plan.ping_no, expected_ping);
            assert_eq!(plan.last_ping, expected_ping == NUDGE_PINGS);
            if plan.status == STATUS_LAPSED {
                assert_eq!(expected_ping, NUDGE_PINGS);
                break;
            }
            dues.push(plan.next_due_at);
            now = plan.next_due_at;
            r.nudge_step = plan.nudge_step;
            r.due_at = plan.next_due_at;
        }
        // +30 min, +2 h, then 09:00 on each of the next two mornings.
        assert_eq!(dues[0], due + 1800);
        assert_eq!(dues[1], due + 1800 + 7200);
        assert_eq!(dues[2], at(Kampala, 2026, 11, 27, 9, 0));
        assert_eq!(dues[3], at(Kampala, 2026, 11, 28, 9, 0));
    }

    #[test]
    fn a_follow_up_that_would_ring_at_night_waits_for_the_morning() {
        let late = at(Kampala, 2026, 11, 26, 20, 50);
        let held = follow_up(1, late, Kampala, &cfg()).unwrap();
        assert_eq!(held, at(Kampala, 2026, 11, 27, 7, 0));
        let mut open = cfg();
        open.quiet_hours.clear();
        assert_eq!(follow_up(1, late, Kampala, &open).unwrap(), late + 1800);
        assert_eq!(follow_up(5, late, Kampala, &cfg()), None);
    }

    #[test]
    fn done_on_a_repeat_carries_it_to_the_next_occurrence() {
        let due = at(Kampala, 2026, 11, 26, 16, 0);
        let r = reminder(due, true, Some(Repeat::Weekdays));
        assert_eq!(
            after_done(&r, due + 60),
            Some(at(Kampala, 2026, 11, 27, 16, 0))
        );
        assert_eq!(after_done(&reminder(due, true, None), due), None);
    }
}
