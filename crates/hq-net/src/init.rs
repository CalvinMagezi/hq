//! `hq net init`: choose the plane's settings, make the hub's key, and print everything the owner
//! has to review and run as root. This code writes only inside the state directory and runs
//! nothing.

use crate::files;
use crate::key::PrivateKey;
use crate::names::{shell_quote, valid_endpoint};
use crate::store::{
    DEFAULT_INTERFACE, DEFAULT_PORT, HUB_KEY_FILE, NET_FILE, Net, NetError, Result, SPOOL_DIR,
    Store, fail,
};
use crate::subnet::{CGNAT, Subnet, pick};
use std::path::{Path, PathBuf};

const SERVICE_UNIT: &str = include_str!("../../../deploy/net/hq-net-apply@.service");
const PATH_UNIT: &str = include_str!("../../../deploy/net/hq-net-apply@.path");
const TIMER_UNIT: &str = include_str!("../../../deploy/net/hq-net-apply@.timer");
/// Under /opt, not /usr/local: /usr/local is writable by a group on some systems, and a root unit
/// must run a program only root can replace.
pub const APPLIER_PATH: &str = "/opt/hq-net/hq-net-apply";
const WG_DIR: &str = "/etc/wireguard";
const ENV_DIR: &str = "/etc/hq-net";
const UNIT_DIR: &str = "/etc/systemd/system";

#[derive(Debug, Default)]
pub struct InitOptions {
    pub subnet: Option<Subnet>,
    pub port: Option<u16>,
    pub endpoint: Option<String>,
}

/// What `init` learns from the machine, passed in so tests control it.
#[derive(Debug)]
pub struct InitContext {
    /// Subnets this machine already routes, or `None` if the routing table could not be read.
    pub routed: Option<Vec<Subnet>>,
    /// The text of `/proc/sys/net/ipv4/ip_forward`, if readable.
    pub ip_forward: Option<String>,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub spool_dir: PathBuf,
}

impl InitContext {
    pub fn with_defaults(routed: Option<Vec<Subnet>>, ip_forward: Option<String>) -> Self {
        Self {
            routed,
            ip_forward,
            owner_uid: files::effective_uid(),
            owner_gid: files::effective_gid(),
            spool_dir: PathBuf::from(SPOOL_DIR),
        }
    }
}

fn choose_subnet(requested: Option<Subnet>, routed: Option<&[Subnet]>) -> Result<Subnet> {
    let routed = routed.unwrap_or(&[]);
    match requested {
        Some(subnet) => {
            if !subnet.is_private() {
                return fail(format!(
                    "{subnet} is not private address space; use a range inside 10.0.0.0/8, 172.16.0.0/12 or 192.168.0.0/16"
                ));
            }
            if subnet.overlaps(&CGNAT) {
                return fail(format!(
                    "{subnet} overlaps 100.64.0.0/10, where Tailscale lives; choose another range"
                ));
            }
            if let Some(clash) = routed.iter().find(|r| subnet.overlaps(r)) {
                return fail(format!(
                    "{subnet} overlaps {clash}, which this machine already routes; choose another range"
                ));
            }
            Ok(subnet)
        }
        None => pick(routed).ok_or_else(|| {
            NetError(
                "no free /24 was found between 10.77.0.0 and 10.99.255.0; pass --subnet".into(),
            )
        }),
    }
}

fn existing_or_new_key(store: &Store) -> Result<(PrivateKey, bool)> {
    let path = store.hub_key_path();
    if let Some(text) = files::read_private(&path)? {
        let key = PrivateKey::from_text(text.trim())
            .ok_or_else(|| NetError(format!("{} does not hold a WireGuard key", path.display())))?;
        return Ok((key, false));
    }
    let key = PrivateKey::generate();
    files::create_private_new(&path, &format!("{}\n", key.reveal()))?;
    Ok((key, true))
}

/// Makes or updates `net.toml` and returns the text to show. Running it again changes nothing
/// except an `--endpoint` you pass; it never makes a second key, because every device already
/// holds the first one's public half.
pub fn init(store: &Store, options: &InitOptions, ctx: &InitContext) -> Result<String> {
    if let Some(endpoint) = &options.endpoint
        && !valid_endpoint(endpoint)
    {
        return fail(
            "--endpoint must be host:port, for example example.com:51820 (letters, digits, dots and dashes only)",
        );
    }
    if options.port == Some(0) {
        return fail("--port must be between 1 and 65535");
    }
    store.ensure_dir()?;
    let mut note = String::new();
    let net = match store.load_net()? {
        Some(mut net) => {
            if let Some(subnet) = options.subnet
                && subnet.to_string() != net.subnet
            {
                return fail(format!(
                    "this server already uses {}; remove {} to start over (every device would need a new config)",
                    net.subnet,
                    store.dir().display()
                ));
            }
            if options.port.is_some_and(|p| p != net.port) {
                return fail(format!(
                    "this server already listens on port {}; remove {} to start over",
                    net.port,
                    store.dir().display()
                ));
            }
            if let Some(endpoint) = &options.endpoint {
                net.endpoint = Some(endpoint.clone());
                store.save_net(&net)?;
                note.push_str("Updated the server address in net.toml.\n");
            }
            note.push_str("The private plane was already set up, so no new key was made. The steps are shown again.\n");
            net
        }
        None => {
            let subnet = choose_subnet(options.subnet, ctx.routed.as_deref())?;
            let (key, _) = existing_or_new_key(store)?;
            let net = Net {
                interface: DEFAULT_INTERFACE.into(),
                subnet: subnet.to_string(),
                hub: subnet.hub().to_string(),
                port: options.port.unwrap_or(DEFAULT_PORT),
                endpoint: options.endpoint.clone(),
                hub_public_key: key.public(),
            };
            store.save_net(&net)?;
            note.push_str(&format!(
                "Wrote {NET_FILE} (public settings) and {HUB_KEY_FILE} (the hub's private key, mode 0600) under {}.\n",
                store.dir().display()
            ));
            net
        }
    };
    Ok(format!("{note}{}", instructions(store, &net, ctx)))
}

fn forwarding_note(ip_forward: Option<&str>, interface: &str) -> String {
    match ip_forward.map(str::trim) {
        Some(value) => format!(
            "net.ipv4.ip_forward is {value} on this machine. The hub does not need forwarding and does not rely on it: the firewall rule in step 3 and the per-interface setting in step 2 both stop traffic from crossing {interface}, whatever the global value is. Leave it as it is. (Linking two devices needs forwarding turned on for {interface}; docs/PRIVATE_NETWORK.md explains how, and why the firewall table must then be loaded first.)\n"
        ),
        None => format!(
            "Could not read net.ipv4.ip_forward here. The hub does not rely on it: step 2 and step 3 stop traffic from crossing {interface} either way. Check it with: sysctl net.ipv4.ip_forward\n"
        ),
    }
}

fn route_note(subnet: &str, routed: &Option<Vec<Subnet>>) -> String {
    match routed {
        Some(list) => format!(
            "Checked {} routes in this machine's main routing table: none overlaps {subnet} or 100.64.0.0/10. Policy routing tables (Tailscale keeps its routes in table 52) are not read; also run `ip route show table all` and make sure nothing there uses {subnet}.\n",
            list.len()
        ),
        None => format!(
            "The routing table could not be read here. Before step 2, check that nothing else uses {subnet}, in every table: ip route show table all\n"
        ),
    }
}

fn instructions(store: &Store, net: &Net, ctx: &InitContext) -> String {
    let staged = store.hub_key_path();
    let staged_present = staged.exists();
    let (interface, subnet, hub, port) = (&net.interface, &net.subnet, &net.hub, net.port);
    let prefix = net.subnet().map(|s| s.prefix()).unwrap_or(24);
    let set = crate::links::set_name(interface).unwrap_or_default();
    let spool = shell_quote(&ctx.spool_dir.display().to_string());
    let staged_q = shell_quote(&staged.display().to_string());
    let (uid, gid) = (ctx.owner_uid, ctx.owner_gid);
    let mut out = String::new();
    out.push_str(&format!(
        "\nPrivate plane: {subnet} on {interface}, hub {hub}, UDP port {port}.\nHub public key: {}\n",
        net.hub_public_key
    ));
    match &net.endpoint {
        Some(endpoint) => out.push_str(&format!("Server address for devices: {endpoint}\n")),
        None => out.push_str("Server address for devices: not set yet. Before adding a device, run: hq net init --endpoint example.com:51820\n"),
    }
    out.push_str(&route_note(subnet, &ctx.routed));
    out.push_str(&forwarding_note(ctx.ip_forward.as_deref(), interface));
    out.push_str(
        "\nNothing below has been run. Read each step. Lines starting with sudo need root.\n\n",
    );

    out.push_str("Step 1. Move the hub's private key where only root can read it:\n");
    if staged_present {
        out.push_str(&format!(
            "  sudo install -m 0600 -o root -g root {staged_q} {WG_DIR}/{interface}.key && rm {staged_q}\n(One line on purpose: the copy is deleted only if the install worked. Until the key is moved, `hq net peer add` and `hq net apply` refuse to run, because the key must not sit where the HQ user and its agents can read it.)\n\n"
        ));
    } else {
        out.push_str(&format!(
            "  {staged_q} is gone, so the key was moved already. Check that {WG_DIR}/{interface}.key exists.\n\n"
        ));
    }
    out.push_str(&format!(
        "Step 2. The WireGuard interface. The key is read from its file, so it never appears in this config:\n  sudo tee {WG_DIR}/{interface}.conf >/dev/null <<'EOF'\n[Interface]\nAddress = {hub}/{prefix}\nListenPort = {port}\nPostUp = wg set %i private-key {WG_DIR}/%i.key\nPostUp = sysctl -q -w net.ipv4.conf.%i.forwarding=0\nEOF\n  sudo chmod 600 {WG_DIR}/{interface}.conf\n\n"
    ));
    out.push_str(&format!(
        "Step 3. Do not forward between devices, except between the pairs you name later with `hq net link add` (none yet; linking also needs step 2's forwarding setting changed, see docs/PRIVATE_NETWORK.md, \"Linking two devices\"). This adds its own nftables table, replacing an earlier copy of it, and leaves your other rules alone. The pairs live in the set; the rules never change, and the applier only replaces the set's contents (after replacing the table, run `sudo systemctl start hq-net-apply@{interface}.service` to fill the set again):\n  sudo nft -f - <<'EOF'\nadd table inet hq_net\ndelete table inet hq_net\ntable inet hq_net {{\n  set {set} {{\n    type ipv4_addr . ipv4_addr\n  }}\n  chain forward {{\n    type filter hook forward priority -10; policy accept;\n    iifname \"{interface}\" oifname \"{interface}\" ip saddr . ip daddr @{set} accept\n    iifname \"{interface}\" drop\n    oifname \"{interface}\" drop\n  }}\n}}\nEOF\nThat is lost at reboot: include the same table from your nftables configuration to keep it (the set may start empty). If another firewall (ufw, firewalld) also filters forwarded traffic, it has to let {interface} traffic back out of {interface} for pairs to work, and this table is then the only thing limiting who may talk: load it at boot, and after every change check that two devices you did not link cannot reach each other. With iptables instead there is no per-pair form; use these, and do not link devices:\n  sudo iptables -I FORWARD -i {interface} -j DROP\n  sudo iptables -I FORWARD -o {interface} -j DROP\nIf your firewall drops inbound UDP by default, also allow {port}/udp.\n\n"
    ));
    out.push_str(&format!(
        "Step 4. The applier: a small program that runs as root, accepts only a public key, one address and an optional preshared key per device, and refuses everything else. Build it without the rest of hq, and install it where only root can change it (a directory under /opt that only root owns; /usr/local can be writable by a group). Never point the unit at ~/bin/hq, which your user can rewrite. Build it as an account other than the one HQ runs as, in a clean checkout, because HQ's agents can rewrite files in the HQ account between the build and the install. If you must build in that account, note the hash and check it again just before the install:\n  cargo build --release --locked -p hq-net --no-default-features --bin hq-net-apply\n  sha256sum target/release/hq-net-apply\n  sudo install -d -o root -g root -m 0755 {}\n  sudo install -o root -g root -m 0755 target/release/hq-net-apply {APPLIER_PATH}\n  sudo install -d -o {uid} -g {gid} -m 0700 {spool}\n  sudo install -d -o root -g root -m 0755 {ENV_DIR}\n  sudo tee {ENV_DIR}/{interface}.env >/dev/null <<'EOF'\nHQ_NET_SUBNET={subnet}\nHQ_NET_HUB={hub}\nHQ_NET_OWNER_UID={uid}\nEOF\n  sudo tee {UNIT_DIR}/hq-net-apply@.service >/dev/null <<'EOF'\n{SERVICE_UNIT}EOF\n  sudo tee {UNIT_DIR}/hq-net-apply@.path >/dev/null <<'EOF'\n{PATH_UNIT}EOF\n  sudo tee {UNIT_DIR}/hq-net-apply@.timer >/dev/null <<'EOF'\n{TIMER_UNIT}EOF\n  sudo systemctl daemon-reload\n\n",
        Path::new(APPLIER_PATH).parent().map(|p| p.display().to_string()).unwrap_or_default()
    ));
    out.push_str(&format!(
        "Step 5. Start it. The service applies the peers after every start of the tunnel, the path unit applies them when you change them, and the timer applies them again every five minutes in case an event was missed:\n  sudo systemctl enable --now wg-quick@{interface}\n  sudo systemctl enable --now hq-net-apply@{interface}.service hq-net-apply@{interface}.path hq-net-apply@{interface}.timer\nTo reconfigure the tunnel use restart, never reload: systemctl reload wg-quick@{interface} runs wg syncconf with a config that has no private key and no peers, and would clear both.\n\nThen add a device: hq net peer add <label> --pubkey <the device's public key> --kind laptop|phone\nThen check from outside the tunnel that only UDP {port} (and ssh) answer.\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(routed: Option<Vec<Subnet>>) -> InitContext {
        InitContext {
            routed,
            ip_forward: Some("1\n".into()),
            owner_uid: 1000,
            owner_gid: 1000,
            spool_dir: PathBuf::from(SPOOL_DIR),
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("net"));
        (dir, store)
    }

    #[test]
    fn the_first_run_makes_a_key_and_prints_steps_without_the_private_key() {
        let (_dir, store) = store();
        let text = init(
            &store,
            &InitOptions {
                endpoint: Some("example.com:51820".into()),
                ..Default::default()
            },
            &ctx(Some(vec![])),
        )
        .unwrap();
        let private = std::fs::read_to_string(store.hub_key_path()).unwrap();
        assert!(!private.trim().is_empty());
        assert!(
            !text.contains(private.trim()),
            "SR-N2: the private key must never be printed"
        );
        let net = store.load_net().unwrap().unwrap();
        assert_eq!(net.subnet, "10.77.0.0/24");
        assert_eq!(net.hub, "10.77.0.1");
        assert!(text.contains(&net.hub_public_key));
        assert!(
            text.contains("net.ipv4.ip_forward is 1"),
            "the real value is reported, not assumed"
        );
        assert!(text.contains("sudo install -m 0600 -o root -g root"));
        assert!(text.contains("PostUp = wg set %i private-key /etc/wireguard/%i.key"));
        assert!(text.contains("Address = 10.77.0.1/24"));
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(store.hub_key_path())
                .unwrap()
                .permissions(),
        ) & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn the_embedded_units_are_the_shipped_files_and_name_the_same_paths() {
        let (_dir, store) = store();
        let text = init(&store, &InitOptions::default(), &ctx(Some(vec![]))).unwrap();
        assert!(
            text.contains(SERVICE_UNIT) && text.contains(PATH_UNIT) && text.contains(TIMER_UNIT)
        );
        let unit = |kind: &str| format!("hq-net-apply@{}.{kind}", "hq0");
        let enable = format!(
            "enable --now {} {} {}",
            unit("service"),
            unit("path"),
            unit("timer")
        );
        assert!(text.contains(&enable));
        assert!(text.contains("never reload"));
        assert!(SERVICE_UNIT.contains(&format!("ExecStart={APPLIER_PATH} ")));
        assert!(SERVICE_UNIT.contains(&format!("--input {SPOOL_DIR}/%i.conf")));
        assert!(PATH_UNIT.contains(&format!("PathChanged={SPOOL_DIR}/%i.conf")));
        assert!(text.contains("sudo install -d -o 1000 -g 1000 -m 0700 /var/lib/hq-net"));
        assert!(text.contains("HQ_NET_OWNER_UID=1000"));
    }

    #[test]
    fn a_second_run_reuses_the_key_and_only_updates_the_endpoint() {
        let (_dir, store) = store();
        init(&store, &InitOptions::default(), &ctx(Some(vec![]))).unwrap();
        let first = store.load_net().unwrap().unwrap();
        let again = init(
            &store,
            &InitOptions {
                endpoint: Some("vpn.example.com:51821".into()),
                ..Default::default()
            },
            &ctx(Some(vec![])),
        )
        .unwrap();
        let second = store.load_net().unwrap().unwrap();
        assert_eq!(first.hub_public_key, second.hub_public_key);
        assert_eq!(second.endpoint.as_deref(), Some("vpn.example.com:51821"));
        assert!(again.contains("no new key was made"));
        let other_subnet = Subnet::parse("10.88.0.0/24").unwrap();
        assert!(
            init(
                &store,
                &InitOptions {
                    subnet: Some(other_subnet),
                    ..Default::default()
                },
                &ctx(Some(vec![]))
            )
            .is_err()
        );
        assert!(
            init(
                &store,
                &InitOptions {
                    port: Some(1234),
                    ..Default::default()
                },
                &ctx(Some(vec![]))
            )
            .is_err()
        );
        assert!(
            init(
                &store,
                &InitOptions {
                    subnet: Some(Subnet::parse("10.77.0.0/24").unwrap()),
                    ..Default::default()
                },
                &ctx(Some(vec![]))
            )
            .is_ok()
        );
    }

    #[test]
    fn a_staged_key_left_by_an_interrupted_run_is_reused_not_replaced() {
        let (_dir, store) = store();
        store.ensure_dir().unwrap();
        let key = PrivateKey::generate();
        files::create_private_new(&store.hub_key_path(), &format!("{}\n", key.reveal())).unwrap();
        init(&store, &InitOptions::default(), &ctx(None)).unwrap();
        assert_eq!(
            store.load_net().unwrap().unwrap().hub_public_key,
            key.public()
        );
    }

    #[test]
    fn a_routed_subnet_is_avoided_and_an_explicit_clash_is_refused() {
        let (_dir, store) = store();
        let busy = vec![Subnet::parse("10.77.0.0/24").unwrap()];
        init(&store, &InitOptions::default(), &ctx(Some(busy.clone()))).unwrap();
        assert_eq!(store.load_net().unwrap().unwrap().subnet, "10.77.1.0/24");
        let (_dir2, store2) = self::store();
        let clash = InitOptions {
            subnet: Some(Subnet::parse("10.77.0.0/24").unwrap()),
            ..Default::default()
        };
        assert!(init(&store2, &clash, &ctx(Some(busy))).is_err());
        assert!(
            store2.load_net().unwrap().is_none(),
            "a refused run leaves no settings behind"
        );
        for bad in ["8.8.8.0/24", "100.64.1.0/24"] {
            let opt = InitOptions {
                subnet: Some(Subnet::parse(bad).unwrap()),
                ..Default::default()
            };
            assert!(init(&store2, &opt, &ctx(Some(vec![]))).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_unreadable_routing_table_and_forward_setting_are_said_not_assumed() {
        let (_dir, store) = store();
        let blind = InitContext {
            routed: None,
            ip_forward: None,
            ..ctx(None)
        };
        let text = init(&store, &InitOptions::default(), &blind).unwrap();
        assert!(text.contains("routing table could not be read"));
        assert!(text.contains("Could not read net.ipv4.ip_forward"));
    }

    #[test]
    fn a_bad_endpoint_or_port_changes_nothing() {
        let (_dir, store) = store();
        let bad = InitOptions {
            endpoint: Some("bad endpoint".into()),
            ..Default::default()
        };
        assert!(init(&store, &bad, &ctx(None)).is_err());
        assert!(
            init(
                &store,
                &InitOptions {
                    port: Some(0),
                    ..Default::default()
                },
                &ctx(None)
            )
            .is_err()
        );
        assert!(store.load_net().unwrap().is_none());
        assert!(!store.hub_key_path().exists());
    }

    #[test]
    fn the_copy_of_the_hub_key_is_deleted_only_if_the_install_worked() {
        let (_dir, store) = store();
        let text = init(&store, &InitOptions::default(), &ctx(Some(vec![]))).unwrap();
        let line = text
            .lines()
            .find(|l| l.contains("install -m 0600 -o root -g root"))
            .expect("the install step");
        assert!(
            line.contains(" && rm "),
            "install and rm must be one command: {line}"
        );
        assert!(
            !text.lines().any(|l| l.trim_start().starts_with("rm ")),
            "no separate rm line"
        );
    }

    #[test]
    fn printed_paths_are_quoted_so_a_directory_name_cannot_inject_a_command() {
        let dir = tempfile::tempdir().unwrap();
        let hostile = dir.path().join("my dir; touch pwned #'x");
        let store = Store::new(hostile.join("net"));
        let mut context = ctx(Some(vec![]));
        context.spool_dir = hostile.join("spool dir");
        let text = init(&store, &InitOptions::default(), &context).unwrap();
        let staged = store.hub_key_path().display().to_string();
        assert!(
            !text.contains(&format!("root {staged}")),
            "the staged path must be quoted"
        );
        let quoted = shell_quote(&staged);
        assert!(quoted.starts_with('\''));
        assert!(text.contains(&format!(
            "root -g root {quoted} /etc/wireguard/hq0.key && rm {quoted}"
        )));
        let spool = shell_quote(&context.spool_dir.display().to_string());
        assert!(text.contains(&format!("-m 0700 {spool}\n")));
    }

    #[test]
    fn the_route_check_says_it_read_only_the_main_table() {
        let (_dir, store) = store();
        let text = init(&store, &InitOptions::default(), &ctx(Some(vec![]))).unwrap();
        assert!(text.contains("main routing table") && text.contains("ip route show table all"));
        let (_dir2, store2) = self::store();
        let blind = init(&store2, &InitOptions::default(), &ctx(None)).unwrap();
        assert!(blind.contains("could not be read") && blind.contains("show table all"));
    }

    #[test]
    fn the_build_step_warns_about_building_in_the_account_hq_runs_as() {
        let (_dir, store) = store();
        let text = init(&store, &InitOptions::default(), &ctx(Some(vec![]))).unwrap();
        assert!(
            text.contains("account other than the one HQ runs as")
                && text.contains("sha256sum target/release/hq-net-apply")
        );
    }

    #[test]
    fn step_3_holds_one_set_and_a_rule_for_it_before_the_drops_and_nothing_else_is_forwarded() {
        let (_dir, store) = store();
        let text = init(&store, &InitOptions::default(), &ctx(Some(vec![]))).unwrap();
        let step = text
            .split("Step 3.")
            .nth(1)
            .and_then(|rest| rest.split("Step 4.").next())
            .expect("step 3");
        let table = &step[step.find("table inet hq_net {").unwrap()..step.find("\nEOF").unwrap()];
        assert_eq!(table.matches("set links_hq0 {").count(), 1, "{table}");
        assert!(table.contains("type ipv4_addr . ipv4_addr"));
        let accept = table.find("ip saddr . ip daddr @links_hq0 accept").unwrap();
        let first_drop = table.find("drop").unwrap();
        assert!(
            accept < first_drop,
            "the pairs must be accepted before the drops: {table}"
        );
        assert_eq!(table.matches("drop").count(), 2, "{table}");
        assert!(table.contains("iifname \"hq0\" drop") && table.contains("oifname \"hq0\" drop"));
        assert!(
            step.find("add table inet hq_net").unwrap()
                < step.find("delete table inet hq_net").unwrap(),
            "replacing the table must not fail when it is missing"
        );
        assert!(step.contains("docs/PRIVATE_NETWORK.md"));
    }
}
