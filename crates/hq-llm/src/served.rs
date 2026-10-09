//! Which configured backend actually answered a call. The backend chain is one provider to the
//! router, so without this every row in the ledger would read "backends".

use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::cost::ProviderClass;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    pub backend: String,
    pub class: ProviderClass,
}

type Slot = Arc<Mutex<Option<Served>>>;

tokio::task_local! {
    static SERVED: Slot;
}

/// Run `fut` with a fresh slot a provider can report into.
pub(crate) async fn with_served_slot<F: Future>(fut: F) -> F::Output {
    SERVED.scope(Slot::default(), fut).await
}

/// A provider that picked among several backends says which one it used.
pub fn note_served(backend: &str, class: ProviderClass) {
    let _ = SERVED.try_with(|slot| {
        *slot.lock().unwrap() = Some(Served {
            backend: backend.to_string(),
            class,
        });
    });
}

/// The backend the current call last reported, if a provider reported one.
pub(crate) fn served_now() -> Option<Served> {
    SERVED
        .try_with(|slot| slot.lock().unwrap().clone())
        .ok()
        .flatten()
}
