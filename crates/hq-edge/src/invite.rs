//! Registering a device with the edge by QR code. The edge shows a ticket; the device scans it,
//! the person at the edge types the six digits the device shows, and the device is written to
//! the allowlist. It is the pairing protocol of `hq-bridge` under the name `hq/register/*`, so
//! there is one exchange to review; this module only says what the edge does with a device the
//! person accepted. How the exchange works: `docs/security/PAIRING.md`.

use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Result, anyhow};
use hq_bridge::{
    EndpointId, Joiner, OpenTicket, PairingSpec, Purpose, Registrar, RelayMode, Tuning, wire::Card,
};
use iroh::RelayUrl;

use crate::{
    config::{EdgeConfig, normalize_hostname},
    registry::{ListError, Registered, allow_device, device_label, set_device_relay},
};

/// What the edge does with a device the person accepted: write it to the allowlist (the running
/// edge reads the list again every second) and answer with the name the device is reached at.
pub struct EdgeRegistrar {
    list: PathBuf,
    domain: String,
    /// The relay devices use as their home when they follow the edge's advice.
    own_relay: Option<RelayUrl>,
    /// Other relays a device may be dialed through (`edge.device_relays`), and no others.
    allowed_relays: Vec<RelayUrl>,
}

impl EdgeRegistrar {
    pub fn new(config: &EdgeConfig) -> Result<Self> {
        Ok(Self {
            list: config.allowlist_path(),
            domain: normalize_hostname(&config.domain)?,
            own_relay: own_relay(config)?,
            allowed_relays: config
                .device_relays
                .iter()
                .map(|r| r.parse())
                .collect::<Result<_, _>>()?,
        })
    }

    fn hostname(&self, id: &EndpointId) -> String {
        format!("{}.{}", device_label(id), self.domain)
    }
}

impl Registrar for EdgeRegistrar {
    fn accept(&self, joiner: &Joiner) -> Result<Option<serde_json::Value>> {
        match allow_device(
            &self.list,
            Registered {
                id: joiner.id,
                relay: None,
            },
        ) {
            // Registering again is how a reinstalled device with the same key gets its name back.
            Ok(()) | Err(ListError::Duplicate(_)) => {}
            Err(e) => return Err(anyhow!(e)),
        }
        Ok(Some(serde_json::json!({
            "hostname": self.hostname(&joiner.id),
            "relay": self.own_relay.as_ref().map(ToString::to_string),
        })))
    }

    /// A device that names a home relay other than the edge's own is dialed through it only when
    /// the operator listed that relay (`edge.device_relays`). The device chose the URL and the
    /// edge connects to whatever is recorded, and a name or address cannot be vetted from the
    /// outside (a name can resolve anywhere), so an unlisted relay is ignored and nothing on the
    /// device's line changes.
    fn details(&self, joiner: &EndpointId, relays: &[String], _addrs: &[String]) {
        let Some(home) = relays
            .iter()
            .filter_map(|r| r.parse::<RelayUrl>().ok())
            .find(|r| self.allowed_relays.contains(r))
        else {
            return;
        };
        if let Err(e) = set_device_relay(&self.list, joiner, Some(&home)) {
            tracing::warn!("could not record a device's relay: {e}");
        }
    }
}

/// The relay the edge offers its devices: the co-hosted one, else `edge.relay_url`.
fn own_relay(config: &EdgeConfig) -> Result<Option<RelayUrl>> {
    if config.relay.enabled {
        let host = normalize_hostname(&config.relay.hostname)?;
        return Ok(Some(format!("https://{host}").parse()?));
    }
    Ok(config.relay_url.as_deref().map(str::parse).transpose()?)
}

/// How one invitation is shown and limited.
pub struct InviteSpec {
    pub tuning: Tuning,
    pub relay: RelayMode,
    pub bind: Vec<SocketAddr>,
    /// Addresses to list in the ticket besides the endpoint's own.
    pub advertise: Vec<SocketAddr>,
    /// Whether the ticket lists the invitation endpoint's addresses. Off, it holds only an id and
    /// a relay, which a device cannot use to reach an endpoint nobody has registered.
    pub with_addrs: bool,
}

/// The invitation a running edge's config implies. It is direct: the edge's own relay admits only
/// the edge and registered devices, so neither the throwaway invitation endpoint nor the device
/// that is about to register could meet there, and n0's public relays are not a thing to send an
/// edge operator through. The ticket lists the machine's addresses (and `edge.invite_advertise`);
/// the device must be able to reach the UDP port (`edge.invite_bind`) while the invitation is open.
pub fn default_invite_spec(config: &EdgeConfig, tuning: Tuning) -> Result<InviteSpec> {
    Ok(InviteSpec {
        tuning,
        relay: RelayMode::Disabled,
        bind: vec![config.invite_bind],
        advertise: config.invite_advertise.clone(),
        with_addrs: true,
    })
}

/// Opens one invitation: a throwaway endpoint and a ticket for a device to scan. The edge's own
/// endpoint is not involved and keeps refusing every incoming connection.
pub async fn open_invite(config: &EdgeConfig, spec: InviteSpec) -> Result<OpenTicket> {
    let edge_id = crate::identity::edge_id(&config.key_path())?;
    let own = own_relay(config)?;
    let card = Card {
        id: edge_id.to_string(),
        name: normalize_hostname(&config.domain)?,
        relays: own.iter().map(ToString::to_string).collect(),
        addrs: Vec::new(),
    };
    OpenTicket::open(PairingSpec {
        purpose: Purpose::Register,
        card,
        registrar: std::sync::Arc::new(EdgeRegistrar::new(config)?),
        bind: spec.bind,
        relay: spec.relay,
        with_addrs: spec.with_addrs,
        advertise: spec.advertise,
        tuning: spec.tuning,
    })
    .await
}
