//! The machine that scans the QR code. It dials the throwaway endpoint the ticket names (so the
//! host is authenticated by the key in the QR), proves it saw the screen, commits to a nonce,
//! shows six digits for the person to type on the host, and learns the host's long-term identity
//! only if the host accepted them.

use crate::code::{self, SasInputs, commit, new_nonce, printable};
use crate::host::{DECISION_TIMEOUT, HELLO_TIMEOUT};
use crate::ticket::{Purpose, Ticket, WIRE_VERSIONS};
use crate::wire::{
    Card, Details, Final, Hello, MAX_NAME_CHARS, Outcome, Reply, Reveal, b32, bind_key_from,
    binding_from, clean_addrs, clean_relays, nonce_from, normalize_card, read_line, send_line,
};
use anyhow::{Context, Result, anyhow, bail};
use iroh::{Endpoint, EndpointAddr, EndpointId, endpoint::ConnectOptions};
use std::time::Duration;

const SHOWN_ERROR_CHARS: usize = 200;
/// Reaching a host that is not there takes the transport this long to give up on its own; this is
/// the joiner's own limit, so a ticket that points nowhere ends in a message, not a wait.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long to wait for the host to read the details and close, so they are not cut off.
const CLOSE_WAIT: Duration = Duration::from_secs(2);

pub struct JoinOptions {
    /// What this device calls itself, shown to the person at the host.
    pub name: String,
    /// How the host can dial this device back. Sent only after the host accepted and its identity
    /// checked out, so a host that never accepts learns none of it.
    pub relays: Vec<String>,
    pub addrs: Vec<String>,
    /// How long to reach the host before giving up.
    pub connect_timeout: Duration,
    pub hello_timeout: Duration,
    pub decision_timeout: Duration,
}

impl JoinOptions {
    pub fn new(name: &str, relays: Vec<String>, addrs: Vec<String>) -> Self {
        Self {
            name: printable(name, MAX_NAME_CHARS),
            relays,
            addrs,
            connect_timeout: CONNECT_TIMEOUT,
            hello_timeout: HELLO_TIMEOUT,
            decision_timeout: DECISION_TIMEOUT,
        }
    }
}

/// A host that accepted the digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joined {
    pub sas: String,
    /// The host's long-term identity, checked against what it promised before the digits.
    pub card: Card,
    /// What the host's registrar sent back (empty when pairing two devices).
    pub info: Option<serde_json::Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinResult {
    Paired(Joined),
    /// The host refused before the digits, and said why.
    Refused(String),
    Declined {
        sas: String,
    },
}

pub(crate) fn target_from(
    id: EndpointId,
    relays: &[String],
    addrs: &[String],
) -> Result<EndpointAddr> {
    let mut target = EndpointAddr::new(id);
    for addr in addrs {
        target = target.with_ip_addr(
            addr.parse()
                .with_context(|| format!("bad address {addr}"))?,
        );
    }
    for relay in relays {
        target = target.with_relay_url(
            relay
                .parse()
                .with_context(|| format!("bad relay {relay}"))?,
        );
    }
    Ok(target)
}

/// The protocol names to offer, newest first: the highest version both sides speak, then the
/// older ones they also speak, so the host's own preference decides.
fn offered_alpns(ticket: &Ticket) -> Result<Vec<Vec<u8>>> {
    let best = ticket.version()?;
    let mut common: Vec<u16> = WIRE_VERSIONS
        .iter()
        .copied()
        .filter(|v| ticket.proto.contains(v) && *v <= best)
        .collect();
    common.sort_unstable_by(|a, b| b.cmp(a));
    Ok(common.into_iter().map(|v| ticket.k.alpn(v)).collect())
}

/// Pairs with the host named in `ticket`. `show_sas` runs as soon as the digits are known, before
/// the person at the host has answered. Nothing is stored here; the caller keeps the host.
pub async fn join(
    endpoint: &Endpoint,
    ticket: &Ticket,
    opts: &JoinOptions,
    show_sas: impl FnOnce(&str),
) -> Result<JoinResult> {
    join_recording(endpoint, ticket, opts, show_sas, async |_: &Card| Ok(())).await
}

/// [`join`], with `record` run once the host accepted and its card checks out, and before this
/// device sends its last message. The host has recorded this device by then and may dial it as
/// soon as the pairing ends, so this side must already list the host: a host it once revoked
/// would otherwise be refused as revoked, and drop the new pairing. A failed `record` ends the
/// pairing with that error before the last message; the host's row then reads "not paired" here,
/// which revokes nothing. A last message that cannot be sent leaves the record, as the pairing
/// still stands: the host accepted, and it saw the address this device came from.
pub async fn join_recording(
    endpoint: &Endpoint,
    ticket: &Ticket,
    opts: &JoinOptions,
    show_sas: impl FnOnce(&str),
    record: impl AsyncFnOnce(&Card) -> Result<()>,
) -> Result<JoinResult> {
    let me = endpoint.id();
    let pairing_endpoint = ticket.endpoint_id()?;
    let code = ticket.code_bytes()?;
    let mut alpns = offered_alpns(ticket)?.into_iter();
    let main = alpns
        .next()
        .ok_or_else(|| anyhow!("no pairing version in common"))?;
    let target = target_from(pairing_endpoint, &ticket.relays, &ticket.addrs)?;
    let options = ConnectOptions::new().with_additional_alpns(alpns.collect());
    let dial = async {
        endpoint
            .connect_with_opts(target, &main, options)
            .await?
            .await
            .map_err(anyhow::Error::from)
    };
    let conn = tokio::time::timeout(opts.connect_timeout, dial)
        .await
        .map_err(|_| anyhow!("reaching the host timed out"))?
        .context("reaching the host")?;
    let alpn = conn.alpn().to_vec();
    let Some((purpose, version)) = Purpose::from_alpn(&alpn) else {
        bail!("the host answered with a protocol this device does not know");
    };
    if purpose != ticket.k {
        bail!("the host answered for a different purpose than the ticket names");
    }

    let (mut send, mut recv) =
        within(opts.hello_timeout, async { Ok(conn.open_bi().await?) }).await?;
    let joiner_nonce = new_nonce();
    within(
        opts.hello_timeout,
        send_line(
            &mut send,
            &Hello {
                v: version,
                code: ticket.code.clone(),
                commit: b32(&commit(&alpn, &joiner_nonce)),
            },
        ),
    )
    .await?;
    let reply: Reply = serde_json::from_str(
        &tokio::time::timeout(opts.hello_timeout, read_line(&mut recv)).await??,
    )?;
    if !reply.ok {
        return Ok(JoinResult::Refused(printable(
            &reply.why,
            SHOWN_ERROR_CHARS,
        )));
    }
    let host_nonce =
        nonce_from(&reply.nonce).ok_or_else(|| anyhow!("the host sent no usable nonce"))?;
    let binding =
        binding_from(&reply.bind).ok_or_else(|| anyhow!("the host sent no identity binding"))?;
    within(
        opts.hello_timeout,
        send_line(
            &mut send,
            &Reveal {
                nonce: b32(&joiner_nonce),
                name: opts.name.clone(),
            },
        ),
    )
    .await?;
    let sas = code::sas(&SasInputs {
        alpn: &alpn,
        pairing_endpoint: &pairing_endpoint,
        host_binding: &binding,
        joiner: &me,
        code: &code,
        joiner_nonce: &joiner_nonce,
        host_nonce: &host_nonce,
    });
    show_sas(&sas);
    let last: Final = serde_json::from_str(
        &tokio::time::timeout(
            opts.decision_timeout.saturating_add(opts.hello_timeout),
            read_line(&mut recv),
        )
        .await??,
    )?;
    if last.result != Outcome::Accepted {
        conn.close(0u32.into(), b"done");
        return Ok(JoinResult::Declined { sas });
    }
    let checked = check_card(last.card, &last.bind_key, &host_nonce, &binding);
    let card = match checked {
        Ok(card) => card,
        Err(e) => {
            conn.close(0u32.into(), b"done");
            return Err(e);
        }
    };
    if let Err(e) = record(&card).await {
        conn.close(0u32.into(), b"done");
        return Err(e);
    }
    // The host is who it promised to be: now it may know how to dial this device back.
    let details = Details {
        relays: clean_relays(opts.relays.clone()),
        addrs: clean_addrs(opts.addrs.clone()),
    };
    let sent = tokio::time::timeout(opts.hello_timeout, send_line(&mut send, &details)).await;
    if matches!(sent, Ok(Ok(()))) && send.finish().is_ok() {
        let _ = tokio::time::timeout(CLOSE_WAIT, conn.closed()).await;
    }
    conn.close(0u32.into(), b"done");
    Ok(JoinResult::Paired(Joined {
        sas,
        card,
        info: last.info,
    }))
}

/// Runs a step under a deadline: a host that never reads or never answers must not hold this
/// device for ever.
async fn within<T>(
    limit: Duration,
    step: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(limit, step).await {
        Ok(result) => result,
        Err(_) => bail!("the host did not answer in time"),
    }
}

/// The card the host sent after the digits, checked against the binding it gave before them, and
/// made safe to store.
fn check_card(
    card: Option<Card>,
    bind_key: &str,
    host_nonce: &code::Nonce,
    binding: &code::Binding,
) -> Result<Card> {
    let card = card.ok_or_else(|| anyhow!("the host accepted but sent no identity"))?;
    let promised: EndpointId = card
        .id
        .parse()
        .map_err(|_| anyhow!("the host's identity is not a key"))?;
    let key = bind_key_from(bind_key).ok_or_else(|| anyhow!("the host sent no binding key"))?;
    if &code::bind_host(&promised, host_nonce, &key) != binding {
        bail!("the host's identity is not the one it promised before the digits");
    }
    normalize_card(card)
}
