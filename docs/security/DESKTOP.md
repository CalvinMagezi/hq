# Desktop app threat model (macOS)

Companion to [docs/DESKTOP.md](../DESKTOP.md) and the Native Bridge sprint plan
([workstream W5](../plans/native-bridge-sprint.md)). It covers `apps/desktop` (the Tauri shell)
and `crates/hq-desktop` (everything the shell calls). Pairing's own protocol, attacks and limits
are in [PAIRING.md](PAIRING.md) and are not repeated here; this page is about what the app adds
around it.

## 1. What is protected

| Asset | Why it matters |
|---|---|
| A1. HQ's web token and API | The web app can do anything the owner can. The app holds the token to sign its own window in |
| A2. The paired list and the bridge key | Whoever is on the list can open streams to this machine's bridge service |
| A3. The person's attention at the digits | Six digits typed by hand are the only thing between a stranger who has seen the QR code and the paired list |
| A4. Processes the owner runs | The app starts and stops one host. It must never stop anything else |
| A5. The consent approval | Approving a claude.ai connection hands a conversation access to HQ's tools |

## 2. Listeners

Nothing in the app listens when it opens. Each row below exists only after the named action and
ends when the app quits (or earlier, as noted).

| Listener | Starts when | Binds | Accepts |
|---|---|---|---|
| The app's local web address (TCP, `proxy_port`, default 47474) | **Open HQ**, **Get a sign-in link**, or **Open in browser** | `127.0.0.1` only; any other address is refused at start | HTTP and the chat socket from this machine, only with this address as `Host`; forwards to HQ's web server on loopback |
| Pairing endpoint (UDP, `bridge_port`, default 47475) | **Switch on** (also **Show pairing code**) until **Switch off** or quit | Every interface | `hq/bridge/1` from devices on the paired list; everyone else is closed at the handshake |
| One temporary pairing endpoint (UDP, random port) | **Show pairing code**, until the code is used, expires (10 min), is cancelled or replaced | Every interface | `hq/pair/<v>` from anyone holding the code, with the limits in PAIRING.md |
| The host (`hq start all`, a child process) | **Start host**, or the launch setting | Whatever HQ's `web_bind` says | HQ's own listeners; documented elsewhere. The app talks to it over loopback only and refuses to run against a `web_bind` that is one non-loopback address |

The pairing endpoints use UDP on all interfaces because a phone has to find the Mac on the local
network. macOS asks the person for Local Network access the first time
(`NSLocalNetworkUsageDescription` in `Info.plist`); without it recent macOS drops that traffic
silently. The app registers no login item and no launch agent, and has no plugin that could
(a test reads `Cargo.toml` and the sources for it).

## 3. The two windows

| | Control window | HQ window |
|---|---|---|
| Shows | Files bundled in the app (`tauri://localhost`) | HQ's web app from `http://127.0.0.1:<proxy_port>` |
| Can call the app | Yes: 20 named commands (`capabilities/control.json`) | No. No capability names it, and Tauri refuses app commands from a non-bundled origin without one |
| CSP | `default-src 'self'; script-src 'self'; style-src 'self'; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. No inline script or style, no `eval`, no `https:`/`ws:`/`data:`/`blob:` | HQ's own policy (hashes for its bootstrap scripts, `connect-src 'self' blob:` plus this host's `ws://`) **and** a second one added by the proxy to every response: `connect-src 'self' blob: ws://127.0.0.1:<port>; frame-ancestors 'none'; object-src 'none'; base-uri 'self'; form-action 'self'`. A browser enforces both, so the second can only narrow |
| Navigation | Only its own bundled page (`window::app_page_allowed`) | Only `http://127.0.0.1:<proxy_port>` (`window::main_page_allowed`: scheme, host and port compared after parsing, so `:474741`, `:47474.evil.com`, userinfo tricks and `https` are refused) |
| New windows | Denied | Denied |
| Reaches over the network | Nothing | The proxy and nothing else: a link to another site does nothing, a script's `fetch` or socket to another host is blocked by the CSP, and the window cannot be navigated there |

Text that comes from outside (a device's name, a note from HQ) is put into the control window with
`textContent` only; a test fails if the script gains `innerHTML`, `eval`, `fetch`, `WebSocket` or an
inline script. The QR code is a matrix drawn on a canvas, never markup built from the link.

The control window's commands are listed three times and a test requires the three to agree:
`build.rs` (the app manifest, which makes Tauri demand a permission for every command),
`capabilities/control.json`, and the handler in `commands.rs`. Tauri 2.12.4 rejects an app command
from a webview whose URL is not local unless a capability grants it (`webview/mod.rs`:
`(plugin_command || has_app_acl_manifest || !is_local) && acl.is_none()` rejects), and no
capability names the HQ window or any remote URL. That rule is read from the Tauri source and
guarded by the configuration test; it was not exercised with a real web view in this change.

## 4. The proxy

A reverse proxy to one upstream, HQ's web server on loopback (`proxy.rs`). It is the origin the HQ
window uses, and the address the consent page is served on.

| Rule | Why |
|---|---|
| Listens on, and forwards to, loopback only; checked before it binds | A proxy that listens elsewhere is a door onto the machine; one that forwards elsewhere is an open proxy |
| `Host` must be `127.0.0.1:<port>` (421 otherwise). `localhost` is not accepted | A rebound DNS name makes a hostile page same-origin with the proxy. HQ's own Host check only applies when no token is set. `localhost` can resolve to `::1`, where another program could be listening, and the app only hands out `127.0.0.1` |
| An `Origin` header must be the proxy's own (403) | A page on another site cannot make the browser POST through it. `null` is refused |
| A cross-site or same-site browser fetch that is not a page navigation is refused (403) | The consent page is opened by a redirect from Claude, which is a navigation and passes; scripts on other sites are not |
| Absolute-form targets and `CONNECT` are refused (400) | It never fetches other URLs for a client |
| A path with `.` or `..` segments, an encoded dot, slash or backslash, a backslash, or an empty segment is refused (400) | The path is judged as written but forwarded through a URL parser that resolves dot segments, so `/x/../mcp` would pass the next rule and arrive as `/mcp`. HQ's own pages use none of these |
| `/mcp`, `/oauth/token`, `/oauth/revoke`, `/.well-known/*` answer 404 | Those belong to the public machine door, not this one |
| The chat socket is proxied only on `/ws` | No other upgrade is carried |
| Stopping the proxy waits two seconds for open responses, then drops them | An event stream never ends by itself; waiting for it would hang the app's quit |
| **No credential is ever added.** `Host`, `Authorization`, `Cookie` and `X-HQ-Client` go through as the client sent them | A proxy that signed requests in would hand the whole API to any local page or process, because HQ trusts any loopback origin. Consent therefore needs the browser to be signed in to HQ, like any other browser |
| `Access-Control-*` response headers are dropped; a second CSP, `nosniff`, `Referrer-Policy: no-referrer` and `Cross-Origin-Resource-Policy: same-origin` are added | Same-origin for its clients, nothing wider |
| `Set-Cookie` passes through unchanged | See the open item in section 6 |

## 5. Other design choices

- **The expected digits never reach the window.** The pairing snapshot has no field for them; the
  window sends what the person typed and Rust compares (constant time). Three wrong tries decline
  the device. A test serialises the snapshot and fails if the digits appear in it.
- **Revoking goes through the one running bridge**, because only it holds the connections a revoke
  has to close. Revoking in the database alone leaves open connections working.
- **The app only stops what it started.** The host is a child in its own process group; stop sends
  the group a TERM and, once the 12-second grace is over, a KILL to whatever is left of the group,
  including members that ignored the TERM after the leader exited. Start and stop run one at a
  time, so two clicks cannot make two hosts. The app never runs `hq stop` (it kills by
  name and by port) and never signals a pid it did not spawn. If something already answers
  `/health` and is not the child, start and stop both refuse.
- **Quitting closes everything, and is held until it has.** Closing the last window, SIGTERM and
  ctrl-c close the host, the proxy and the pairing endpoints side by side, each with its own bound, so an open event stream or a slow relay cannot keep the host from being
  stopped. A second quit while that runs is held, not let through. On macOS Cmd-Q and logout end the event
  loop without asking first (the app delegate only implements `applicationWillTerminate:`), so the
  same close also runs on the final exit event, with the windows already gone. After the final
  stop nothing can start a host.
- **The token reaches the HQ window in the URL fragment** (`/#token=…`), built in process: it never
  passes through argv, the environment, a log, or the proxy (a fragment is not sent). HQ's web app
  moves it into the window's local storage and removes it from the address bar. The sign-in link
  for another browser carries a one-time code, which does pass through `open`'s argv for an
  instant; it is single use and lasts ten minutes.
- **The control window never receives the web token, the bridge key or the expected pairing
  digits**; they stay in Rust. It does show the pairing link and a one-time sign-in code, which are
  meant to be shown and are single use.
- **Settings are owner-only, unknown keys are refused, and a corrupt file is an error**, not a
  silent reset.

## 6. Open items

- **Consent on the app's own address is not complete.** The proxy serves `/connect`, the consent
  API and `/oauth/authorize`. But `mcp_connector.authorize_url` must be an https origin, HQ matches
  the request's `Host` against it before it answers `/oauth/authorize`, and the request cookie is
  `Secure`; whether WebKit keeps a `Secure` cookie from `http://127.0.0.1` is not verified. So
  Claude is not yet sent to the loopback address. Allowing a loopback http consent origin touches
  `mcp_connector.rs`, which the device-side TLS change also edits, and the question whether
  claude.ai accepts such an `authorization_endpoint` is W6's test. The Connect Claude screen says
  this when it applies.
- **No address for Claude yet.** The connector address is whatever `mcp_connector.public_url`
  says; the device name from `hq edge` and the TLS door come with W1 and W2.
- **The web token is at rest in the HQ window's storage** (WebKit's data store for the app's
  identifier), like a signed-in browser. An XSS in HQ's web app would reach it; HQ's CSP is the
  defense.
- **Cookies are not scoped to a port.** A cookie HQ sets for `127.0.0.1` (the consent request's
  binder) is also sent to any other local service on another port. It is `HttpOnly`, short lived and
  useless without the request id, but it is shared with whatever else listens on this machine.
- **A host the app started survives a crash of the app** (its own process group). A clean quit
  stops it. A later launch shows it as "Running elsewhere", which cannot be stopped from the app.
- **The sign-in code appears in `ps` for an instant** when opened in the browser.
- **Downloads in the HQ window** (exports) are not handled; WebKit's default applies.
- **The pairing endpoint is on every interface** while switched on. Only paired keys get past the
  handshake, and the service behind it is an echo until the host gate is built on it, but a
  stranger who learned this Mac's id costs it a handshake and a database read per attempt
  (NATIVE-BRIDGE-010).
- **Not verified:** no UI of the built app was driven (screen access was not granted), and the
  HQ window was never loaded through the proxy against the real web app (the tests use a fake
  HQ), so HQ's own CSP together with the proxy's second policy, the chat socket and event streams
  through the proxy, and whether the path rule meets any real request are untested at runtime.
  The built app was launched with an isolated HOME: it opened no socket while idle and ended on
  SIGTERM; quitting with a host running, and Cmd-Q, were not exercised. The IPC isolation of the
  HQ window is from the Tauri source and the configuration test, not from a running attack. The
  real-camera `hqpair://` hand-off is W4's.

## 7. Attack notes

| # | Attacker | Tries | Result | Guard (test) |
|---|---|---|---|---|
| 1 | A web page in the owner's browser | Reaches the proxy through a rebound DNS name | 421 before HQ sees it | `judge`, Host (`a_request_for_another_host_never_reaches_hq`) |
| 2 | The same | A cross-origin POST or fetch to the consent API | 403; HQ's `X-HQ-Client` rule is a second layer | Origin and Sec-Fetch rules (`another_sites_origin_is_refused_before_hq_sees_it`, `a_cross_site_fetch_is_refused_but_a_cross_site_navigation_is_not`) |
| 3 | A local client | Uses the proxy to fetch another URL, or `CONNECT` | 400 | `it_does_not_fetch_other_urls_for_a_client` |
| 4 | Anyone on the network | Reaches the proxy | It is not bound there; a non-loopback address is refused before bind | `only_loopback_ends_are_accepted`, `a_second_proxy_on_a_taken_port_says_so_and_a_non_loopback_one_never_binds` |
| 5 | A local client | Uses the proxy to reach `/mcp` or the OAuth token routes | 404, HQ never sees the call | `the_machine_endpoints_are_not_served_here` |
| 6 | A local client | Counts on the proxy to sign its requests | It adds nothing | `a_request_is_forwarded_with_its_own_host_and_credentials_and_none_added` |
| 7 | Script in the HQ window | Calls the app's commands | Refused by Tauri: no capability names that window | `only_the_control_window_has_a_capability_and_it_grants_exactly_the_app_commands` (config); the rule itself is from the Tauri source |
| 8 | A link or script in HQ's pages | Moves a window elsewhere or opens another | Refused | `the_web_window_stays_on_the_proxy_origin`, `the_control_window_stays_on_its_bundled_page`, `both_windows_are_made_in_code_with_a_navigation_guard_and_no_new_windows` |
| 9 | A device with a hostile name | Script or markup in the control window | `textContent` only; control characters and look-alike overrides are replaced first | `the_control_window_loads_nothing_from_outside`, `printable` in hq-bridge |
| 10 | The window or a script in it | Reads the expected digits | There is no field for them | `the_screen_never_gets_the_digits_it_has_to_compare` |
| 11 | A habitual click | Pairs a device without typing | There is no accept button; declining and wrong digits pair nobody | `wrong_digits_decline_after_the_allowed_tries_and_pair_nobody`, `declining_pairs_nobody` |
| 12 | A slow person | Answers after the device left | Nothing paired | `a_late_answer_pairs_nobody` |
| 13 | A revoked device | Keeps its connection | Closed at once; a new one gets no request through | `a_device_pairs_when_the_digits_are_typed_connects_and_is_cut_by_a_revoke` |
| 14 | A mistaken stop | Kills a daemon or session the app did not start | Refused; the process is untouched | `a_host_this_app_did_not_start_is_reported_and_never_stopped` |
| 15 | A stop of the host | Leaves its children running | The whole process group gets the signal | `start_runs_the_child_and_stop_ends_its_whole_group` |
| 16 | A future change | Adds a login item, a plugin, a remote URL or a looser CSP | The configuration tests fail | `shell_config.rs` |
| 17 | A page-supplied address | Asks the app to open it in the browser | Only an address on the proxy's origin is opened | `main_page_allowed` in `open_in_browser` |
| 18 | A settings file | Carries an unknown key or a port clash | Refused | `unknown_keys_are_refused`, `ports_and_relays_are_checked` |
| 19 | A raw-socket local client | Reaches `/mcp` or the token route through `/x/../mcp`, `/%2e/mcp`, `//mcp` | 400 before HQ sees it | `a_path_two_parsers_could_read_differently_is_refused`, `paths_that_a_url_parser_would_resolve_to_a_machine_endpoint_never_reach_hq` |
| 20 | An open event stream | Keeps the proxy, and so the quit, waiting | The proxy stops after two seconds; the host is stopped in parallel | `stopping_the_proxy_does_not_wait_for_a_stream_that_never_ends` |
| 21 | A double click, or a launch-time start plus a click | Starts two hosts, one untracked | Start and stop run one at a time | `two_starts_at_once_make_one_host` |
| 22 | A host process that ignores the polite signal | Survives a stop after its leader exited | The group is killed at the end of the grace period | `stop_kills_group_members_that_ignore_the_polite_signal` |
| 24 | The macOS quit command | Ends the app without the exit request, leaving the host running | The close also runs on the exit event | read from the tao and Tauri sources; not exercised (see section 6) |
| 25 | A host that crashed | Leaves group members behind and a status poll has already reaped the leader | The group is remembered until it is empty, and a stop still reaches it | `stop_reaches_group_members_after_a_status_poll_reaped_the_leader` |
| 26 | A launch-time start, or a click, during quit | Starts a host that nothing will stop | The final stop closes the door under the same gate | `nothing_starts_after_the_final_stop` |
| 27 | A slow relay or revoke | Freezes the screen's refresh | The refresh reads a cached card, not the bridge lock | `the_screen_refresh_does_not_wait_for_the_bridge_lock` |
| 23 | A script in the control window | Passes a backslash URL to `open` that two parsers read differently | Refused; the address goes out in the parser's own spelling | `only_a_plain_address_on_the_proxy_is_handed_to_the_browser` |

## 8. Guard checks

Each guard below was broken one at a time and `cargo test -p hq-desktop` run; every break made at least the tests named fail. `python3 scripts/check-desktop-guards.py` repeats the run (about half an hour, so a pre-release check rather than part of CI).

| Guard removed or broken | Tests that failed |
|---|---|
| Digits compare forced true | `a_wrong_try_then_the_right_digits_still_pairs`, `only_equal_digits_match`, `wrong_digits_decline_after_the_allowed_tries_and_pair_nobody` |
| Revoke through the database instead of the running bridge | `a_device_pairs_when_the_digits_are_typed_connects_and_is_cut_by_a_revoke` |
| Expected digits written into the request view | `the_screen_never_gets_the_digits_it_has_to_compare` |
| Unlimited digit tries | `wrong_digits_decline_after_the_allowed_tries_and_pair_nobody` |
| Proxy Host check removed | `a_request_for_another_host_never_reaches_hq`, `the_proxys_own_names_pass_and_every_other_host_does_not` |
| Non-loopback bind allowed | `a_second_proxy_on_a_taken_port_says_so_and_a_non_loopback_one_never_binds`, `only_loopback_ends_are_accepted` |
| Non-loopback upstream allowed | `only_loopback_ends_are_accepted` |
| Absolute-form and CONNECT allowed | `it_does_not_fetch_other_urls_for_a_client`, `it_never_acts_as_a_forward_proxy` |
| Origin check removed | `another_sites_origin_is_refused_before_hq_sees_it`, `a_foreign_or_null_origin_is_refused_and_the_proxys_own_passes`, `the_chat_socket_passes_through_and_other_upgrades_do_not` |
| Cross-site fetch allowed | `a_cross_site_fetch_is_refused_but_a_cross_site_navigation_is_not` |
| Machine endpoints served | `the_machine_endpoints_stay_off_this_door`, `the_machine_endpoints_are_not_served_here` |
| Proxy adds a credential | `a_request_is_forwarded_with_its_own_host_and_credentials_and_none_added` |
| Any websocket path carried | `the_chat_socket_passes_through_and_other_upgrades_do_not` |
| Second CSP not added | `responses_gain_a_second_policy_and_lose_cross_origin_grants` |
| CORS grants kept | `responses_gain_a_second_policy_and_lose_cross_origin_grants` |
| Web window origin compared as a prefix | `the_web_window_stays_on_the_proxy_origin` |
| Stopping a host the app did not start is allowed | `a_host_this_app_did_not_start_is_reported_and_never_stopped` |
| Starting over a host that is already running is allowed | `a_host_this_app_did_not_start_is_reported_and_never_stopped` |
| Stop signals the child, not its group | `start_runs_the_child_and_stop_ends_its_whole_group` |
| Unknown settings keys accepted | `unknown_keys_are_refused` |
| Host starts at launch whatever the setting says | `opening_the_app_starts_nothing_and_saves_nothing_it_was_not_told_to` |
| CSP allows inline scripts | `the_csp_is_strict_and_names_no_remote_origin` |
| CSP allows any https connection | `the_csp_is_strict_and_names_no_remote_origin` |
| Global Tauri object on | `the_web_view_is_locked_down` |
| A remote URL granted app commands | `only_the_control_window_has_a_capability_and_it_grants_exactly_the_app_commands` |
| A command granted that is not in the manifest | `only_the_control_window_has_a_capability_and_it_grants_exactly_the_app_commands` |
| An updater plugin added | `the_shell_has_no_plugin_and_so_no_login_item_shell_access_or_updater` |
| InnerHTML in the control window script | `the_control_window_loads_nothing_from_outside` |
| A window without a navigation guard | `both_windows_are_made_in_code_with_a_navigation_guard_and_no_new_windows` |
| Ambiguous paths allowed | `paths_that_a_url_parser_would_resolve_to_a_machine_endpoint_never_reach_hq`, `a_path_two_parsers_could_read_differently_is_refused` |
| Proxy stop waits for streams that never end | `stopping_the_proxy_does_not_wait_for_a_stream_that_never_ends` |
| Two starts can race | `two_starts_at_once_make_one_host` |
| Group members that ignore the polite signal are left alone | `stop_kills_group_members_that_ignore_the_polite_signal` |
| Localhost accepted as the proxy's name | `a_request_for_another_host_never_reaches_hq`, `a_foreign_or_null_origin_is_refused_and_the_proxys_own_passes`, `the_proxys_own_name_passes_and_every_other_host_does_not` |
| Backslash allowed in the address handed to the browser | `only_a_plain_address_on_the_proxy_is_handed_to_the_browser` |
| IPv6 loopback accepted as HQ's web bind | `the_app_reaches_hq_over_loopback_or_not_at_all` |
| A host's leftover group members are forgotten once the leader is reaped | `stop_reaches_group_members_after_a_status_poll_reaped_the_leader` |
| A host can start after the final stop | `nothing_starts_after_the_final_stop` |
| A host that ignores the polite signal is never killed | `stop_kills_a_host_that_ignores_the_polite_signal`, `stop_kills_group_members_that_ignore_the_polite_signal` |
| The screen refresh waits on the bridge lock | `the_screen_refresh_does_not_wait_for_the_bridge_lock` |

40 guards, 40 caught. The script edits the source, runs the tests and restores the file; it covers the proxy, the host, the pairing controller, settings, the navigation allow-lists and the shell's configuration files.
