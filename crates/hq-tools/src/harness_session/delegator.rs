//! The hook through which a session that spawns from inside itself is turned into a delegation.
//!
//! The delegation itself lives in `a2a`, which sits above this module because it starts sessions.
//! `a2a` installs it once (`a2a::install_delegation`, called when its tools are made and where the
//! chat agent's harness tools are made); a spawn from a launched session then calls through here
//! instead of naming `a2a`, so this module no longer reaches up into it.

use anyhow::{Result, bail};
use hq_db::Database;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

/// What `a2a` installs: the spawn's database, the calling session and the spawn's arguments in,
/// the delegation's report out.
pub type Delegator =
    fn(Arc<Database>, String, Value) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>;

static DELEGATOR: OnceLock<Delegator> = OnceLock::new();

/// Installs the delegator. The first call wins; a later one is ignored.
pub fn install_delegator(delegator: Delegator) {
    let _ = DELEGATOR.set(delegator);
}

/// Delegates for session `from` as `harness_session_spawn` called from inside it.
pub(crate) async fn delegate_from_spawn(
    db: Arc<Database>,
    from: String,
    args: &Value,
) -> Result<Value> {
    match DELEGATOR.get() {
        Some(delegate) => delegate(db, from, args.clone()).await,
        None => bail!("a session cannot delegate here: agent delegation is not installed"),
    }
}
