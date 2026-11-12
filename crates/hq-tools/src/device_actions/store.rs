//! The open device actions, in memory. An action is a request HQ sent to the phone and has not
//! heard the end of. The id is the only handle the phone holds, so it is random, single use, and
//! tied here to the chat that made it. Nothing is written to disk: a restart drops open actions,
//! and the tool's own timeouts have long since answered the model.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{Value, json};
use tokio::sync::watch;

pub const DEVICE_ACTION_ACK_SECS: u64 = 5;
pub const DEVICE_ACTION_RESULT_SECS: u64 = 20;
/// Actions one chat may have waiting at once.
pub const MAX_PENDING_PER_SESSION: usize = 4;
/// How long after the result window a late result is still taken, so a phone that inserted
/// the event and lost its connection can still settle the action.
pub const LATE_RESULT_GRACE_SECS: i64 = 300;
/// How long a finished action is remembered, so the same request id gets the same answer.
pub const SETTLED_KEEP_SECS: i64 = 600;
pub const WIRE_VERSION: u32 = 1;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// How long the phone has to answer each step. Constants in production, short in tests.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub ack_secs: u64,
    pub result_secs: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            ack_secs: DEVICE_ACTION_ACK_SECS,
            result_secs: DEVICE_ACTION_RESULT_SECS,
        }
    }
}

/// What a waiter watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Open,
    Acked,
    Settled,
    Abandoned,
}

/// Why the store refused a phone's call. Maps to a status code in the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// No such action, or it belongs to another chat.
    Unknown,
    /// Past its window.
    Expired,
    /// Already acknowledged or settled; ids are single use.
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    TooManyPending,
}

struct Record {
    name: &'static str,
    thread_id: String,
    request_id: String,
    phase: Phase,
    /// Dropped once settled: titles and places do not outlive the action.
    payload: Option<Value>,
    ack_by: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    accept_until: DateTime<Utc>,
    outcome: Option<Value>,
    /// The tool gave up waiting; a result that still arrives is stored, not discarded.
    given_up: bool,
    tx: watch::Sender<Phase>,
}

#[derive(Default)]
struct Inner {
    actions: HashMap<String, Record>,
    by_request: HashMap<(String, String), String>,
}

/// What `open` found.
pub enum Opened {
    New {
        action_id: String,
        frame: Value,
        phase: watch::Receiver<Phase>,
    },
    /// This request id was seen before: the answer so far, `None` while still open.
    Known { outcome: Option<Value> },
}

pub struct DeviceActionStore {
    clock: Arc<dyn Clock>,
    inner: Mutex<Inner>,
}

impl Default for DeviceActionStore {
    fn default() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }
}

/// A fresh unguessable id: 128 random bits, hex.
pub fn new_action_id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

impl DeviceActionStore {
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        self.prune(&mut inner);
        inner
    }

    fn prune(&self, inner: &mut Inner) {
        let cutoff = self.clock.now() - TimeDelta::seconds(SETTLED_KEEP_SECS);
        let dead: Vec<String> = inner
            .actions
            .iter()
            .filter(|(_, r)| r.accept_until < cutoff)
            .map(|(id, _)| id.clone())
            .collect();
        for id in dead {
            if let Some(r) = inner.actions.remove(&id) {
                inner.by_request.remove(&(r.thread_id, r.request_id));
            }
        }
    }

    /// Starts an action called `name` for `thread_id`. The frame is what goes to the phone.
    pub fn open_named(
        &self,
        name: &'static str,
        thread_id: &str,
        tool_call_id: &str,
        request_id: &str,
        payload: Value,
        timeouts: Timeouts,
    ) -> Result<Opened, OpenError> {
        let mut inner = self.lock();
        let key = (thread_id.to_string(), request_id.to_string());
        if let Some(id) = inner.by_request.get(&key) {
            let outcome = inner.actions.get(id).and_then(|r| r.outcome.clone());
            return Ok(Opened::Known { outcome });
        }
        let waiting = inner
            .actions
            .values()
            .filter(|r| r.thread_id == thread_id && !r.given_up)
            .filter(|r| matches!(r.phase, Phase::Open | Phase::Acked))
            .count();
        if waiting >= MAX_PENDING_PER_SESSION {
            return Err(OpenError::TooManyPending);
        }
        let now = self.clock.now();
        let ack_by = now + TimeDelta::seconds(timeouts.ack_secs as i64);
        let expires_at = ack_by + TimeDelta::seconds(timeouts.result_secs as i64);
        let accept_until = expires_at + TimeDelta::seconds(LATE_RESULT_GRACE_SECS);
        let action_id = new_action_id();
        let frame = json!({
            "type": "device_action",
            "action_id": action_id,
            "thread_id": thread_id,
            "tool_call_id": tool_call_id,
            "name": name,
            "v": WIRE_VERSION,
            "payload": payload,
            "ack_by": ack_by.to_rfc3339(),
            "expires_at": expires_at.to_rfc3339(),
        });
        let (tx, phase) = watch::channel(Phase::Open);
        inner.by_request.insert(key, action_id.clone());
        inner.actions.insert(
            action_id.clone(),
            Record {
                name,
                thread_id: thread_id.to_string(),
                request_id: request_id.to_string(),
                phase: Phase::Open,
                payload: Some(payload),
                ack_by,
                expires_at,
                accept_until,
                outcome: None,
                given_up: false,
                tx,
            },
        );
        Ok(Opened::New {
            action_id,
            frame,
            phase,
        })
    }

    /// What kind of action `action_id` is, so a result is judged against the right contract.
    pub fn action_name(&self, action_id: &str) -> Option<&'static str> {
        self.lock().actions.get(action_id).map(|r| r.name)
    }

    /// The phone has the action and will act on it.
    pub fn ack(&self, action_id: &str) -> Result<(), Reject> {
        let mut inner = self.lock();
        let now = self.clock.now();
        let record = inner.actions.get_mut(action_id).ok_or(Reject::Unknown)?;
        match record.phase {
            Phase::Open if now <= record.ack_by => {
                record.phase = Phase::Acked;
                let _ = record.tx.send(Phase::Acked);
                Ok(())
            }
            Phase::Open => {
                record.phase = Phase::Abandoned;
                let _ = record.tx.send(Phase::Abandoned);
                Err(Reject::Expired)
            }
            Phase::Abandoned => Err(Reject::Expired),
            Phase::Acked | Phase::Settled => Err(Reject::Resolved),
        }
    }

    /// Takes the phone's result. `finalize` turns it into the outcome the model sees, and runs
    /// while the payload is still there to compare against.
    pub fn settle(
        &self,
        action_id: &str,
        body: &Value,
        finalize: impl FnOnce(&Value, &Value) -> Value,
    ) -> Result<(), Reject> {
        let mut inner = self.lock();
        let now = self.clock.now();
        let record = inner.actions.get_mut(action_id).ok_or(Reject::Unknown)?;
        match record.phase {
            Phase::Settled => return Err(Reject::Resolved),
            Phase::Abandoned => return Err(Reject::Expired),
            Phase::Open if now > record.ack_by => return Err(Reject::Expired),
            Phase::Acked if now > record.accept_until => return Err(Reject::Expired),
            Phase::Open | Phase::Acked => {}
        }
        let payload = record.payload.take().unwrap_or(Value::Null);
        record.outcome = Some(finalize(body, &payload));
        record.phase = Phase::Settled;
        let _ = record.tx.send(Phase::Settled);
        Ok(())
    }

    /// The ack window passed. True when the action was still unacknowledged, which is now
    /// final: the phone's late ack is refused, so it never touches the provider.
    pub fn abandon_if_unacked(&self, action_id: &str) -> bool {
        let mut inner = self.lock();
        let Some(record) = inner.actions.get_mut(action_id) else {
            return false;
        };
        if record.phase != Phase::Open {
            return false;
        }
        record.phase = Phase::Abandoned;
        record.payload = None;
        let _ = record.tx.send(Phase::Abandoned);
        true
    }

    /// The result window passed with no result. A later one is still stored.
    pub fn give_up(&self, action_id: &str) {
        if let Some(record) = self.lock().actions.get_mut(action_id) {
            record.given_up = true;
        }
    }

    pub fn outcome(&self, action_id: &str) -> Option<Value> {
        self.lock()
            .actions
            .get(action_id)
            .and_then(|r| r.outcome.clone())
    }

    /// Actions the phone may still settle, for `thread_id` or for every chat. A dropped socket
    /// after an insert leaves one of these.
    pub fn pending(&self, thread_id: Option<&str>) -> Vec<Value> {
        let inner = self.lock();
        let now = self.clock.now();
        let mut open: Vec<(&String, &Record)> = inner
            .actions
            .iter()
            .filter(|(_, r)| thread_id.is_none_or(|t| r.thread_id == t))
            .filter(|(_, r)| match r.phase {
                Phase::Open => now <= r.ack_by,
                Phase::Acked => now <= r.accept_until,
                Phase::Settled | Phase::Abandoned => false,
            })
            .collect();
        open.sort_by_key(|(_, r)| r.ack_by);
        open.into_iter()
            .map(|(id, r)| {
                json!({
                    "action_id": id,
                    "thread_id": r.thread_id,
                    "name": r.name,
                    "v": WIRE_VERSION,
                    "payload": r.payload,
                    "acked": r.phase == Phase::Acked,
                    "ack_by": r.ack_by.to_rfc3339(),
                    "expires_at": r.expires_at.to_rfc3339(),
                })
            })
            .collect()
    }
}
