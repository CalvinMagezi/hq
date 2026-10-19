//! The shipped systemd units and the program they run agree with each other. Nothing here starts
//! systemd; it reads the files in `deploy/net` as text.

const SERVICE: &str = include_str!("../../../deploy/net/hq-net-apply@.service");
const PATH: &str = include_str!("../../../deploy/net/hq-net-apply@.path");
const TIMER: &str = include_str!("../../../deploy/net/hq-net-apply@.timer");

fn setting<'a>(unit: &'a str, name: &str) -> Vec<&'a str> {
    unit.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
        .collect()
}

#[test]
fn the_service_runs_a_root_owned_program_and_never_the_users_hq() {
    let exec = setting(SERVICE, "ExecStart");
    assert_eq!(exec.len(), 1);
    assert!(
        exec[0].starts_with("/opt/hq-net/hq-net-apply "),
        "{}",
        exec[0]
    );
    for line in SERVICE.lines().filter(|l| !l.trim_start().starts_with('#')) {
        assert!(
            !line.contains("/home") && !line.contains('~') && !line.contains("/hq "),
            "{line}"
        );
    }
    assert!(
        setting(SERVICE, "User").is_empty(),
        "the applier needs root; it is confined by capabilities instead"
    );
}

#[test]
fn the_service_is_confined_to_the_two_capabilities_it_needs() {
    assert_eq!(setting(SERVICE, "NoNewPrivileges"), vec!["yes"]);
    assert_eq!(
        setting(SERVICE, "CapabilityBoundingSet"),
        vec!["CAP_NET_ADMIN CAP_DAC_READ_SEARCH"]
    );
    assert_eq!(setting(SERVICE, "ProtectSystem"), vec!["strict"]);
    assert_eq!(setting(SERVICE, "ProtectHome"), vec!["yes"]);
    assert_eq!(
        setting(SERVICE, "RestrictAddressFamilies"),
        vec!["AF_NETLINK AF_UNIX"]
    );
    for (name, value) in [
        ("ProtectKernelTunables", "yes"),
        ("RestrictSUIDSGID", "yes"),
        ("MemoryDenyWriteExecute", "yes"),
        ("LimitCORE", "0"),
        ("UMask", "0077"),
    ] {
        assert_eq!(setting(SERVICE, name), vec![value], "{name}");
    }
}

#[test]
fn a_burst_of_changes_cannot_kill_the_path_unit_or_hang_the_service() {
    // A start limit hit on the service sets the path unit to failed, and it stops watching.
    assert_eq!(setting(SERVICE, "StartLimitIntervalSec"), vec!["0"]);
    assert_eq!(setting(SERVICE, "TimeoutStartSec"), vec!["30s"]);
}

#[test]
fn the_tunnel_is_never_started_by_a_change_to_the_users_file() {
    assert!(setting(SERVICE, "Wants").is_empty());
    assert_eq!(setting(SERVICE, "Requisite"), vec!["wg-quick@%i.service"]);
    assert_eq!(setting(SERVICE, "After"), vec!["wg-quick@%i.service"]);
}

#[test]
fn peers_are_applied_after_every_start_of_the_tunnel_and_again_by_a_timer() {
    // The path unit does not fire when it starts, and it is blind while the service runs.
    assert_eq!(setting(SERVICE, "WantedBy"), vec!["wg-quick@%i.service"]);
    assert_eq!(setting(TIMER, "OnBootSec"), vec!["30s"]);
    assert_eq!(setting(TIMER, "OnCalendar"), vec!["*:0/5"]);
    assert!(
        setting(TIMER, "Unit").is_empty(),
        "the timer starts the service of the same instance"
    );
}

#[test]
fn the_path_unit_watches_the_file_the_service_reads() {
    let watched = setting(PATH, "PathChanged");
    assert_eq!(watched, vec!["/var/lib/hq-net/%i.conf"]);
    let exec = setting(SERVICE, "ExecStart")[0];
    assert!(exec.contains(&format!("--input {}", watched[0])));
    assert_eq!(setting(SERVICE, "ConditionPathExists"), watched);
}

#[test]
fn the_subnet_hub_and_owner_come_from_a_root_owned_file_not_the_input() {
    let env = setting(SERVICE, "EnvironmentFile");
    assert_eq!(env, vec!["/etc/hq-net/%i.env"]);
    let exec = setting(SERVICE, "ExecStart")[0];
    for needle in [
        "--interface %i",
        "--subnet ${HQ_NET_SUBNET}",
        "--hub ${HQ_NET_HUB}",
        "--owner-uid ${HQ_NET_OWNER_UID}",
    ] {
        assert!(exec.contains(needle), "missing {needle}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_exec_line_parses_with_the_applier_when_the_variables_are_filled_in() {
    let exec = setting(SERVICE, "ExecStart")[0];
    let filled = exec
        .replace("%i", "hq0")
        .replace("${HQ_NET_SUBNET}", "10.77.0.0/24")
        .replace("${HQ_NET_HUB}", "10.77.0.1")
        .replace("${HQ_NET_OWNER_UID}", "1000");
    let args = filled.split_whitespace().skip(1).map(String::from);
    let config = hq_net::applier::parse_args(args)
        .expect("the shipped unit must pass the applier's own argument check");
    assert_eq!(config.interface, "hq0");
    assert_eq!(
        config.input,
        std::path::PathBuf::from("/var/lib/hq-net/hq0.conf")
    );
    assert_eq!(config.owner_uid, 1000);
}
