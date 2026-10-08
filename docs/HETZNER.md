# Deploying HQ on Hetzner

This sets up a private, self-updating HQ on a Hetzner Cloud server. It never stores a secret in the server's user-data, and the web UI is reachable only from your tailnet. The hosted wizard (`apps/deploy`, served at <https://deploy.agent-hq.online>) and `deploy/hetzner/hcloud.sh` both use the same `cloud-init.yaml`, and a test keeps the wizard's copy identical to the file.

## Flow

1. Create an Ubuntu 24.04 server with an SSH key attached and a Hetzner Cloud Firewall that allows inbound TCP 22 only from your IP. `hcloud.sh` does all of this:
   ```
   bash deploy/hetzner/hcloud.sh --name hq --ssh-key my-key --admin-cidr 203.0.113.7/32 \
       --type <server-type> --location <location> --repo <owner>/<repo>
   ```
   Pick values with `hcloud server-type list` and `hcloud location list`. The commit you deploy must already be pushed.
2. Wait a few minutes. cloud-init downloads `bootstrap.sh` at a pinned commit, checks its sha256, and runs it. It installs HQ from signed releases, generates the web token and MCP key on the server, installs Tailscale without joining it, enables `ufw`, and restricts SSH to keys. Progress is in `/var/lib/hq-bootstrap/status.json`.
3. SSH in as root and run `sudo hq-join`. It runs `tailscale up`, which prints a login link: open it and sign in to your own tailnet. It then publishes HQ with `tailscale serve` on port 8443, allows SSH over the tailnet, and prints the sign-in link `https://hq.example.ts.net:8443/vault#token=...`.
4. Open that link from a device on your tailnet. HQ asks for a model API key on first run.

The tailnet needs MagicDNS and HTTPS certificates enabled, otherwise `tailscale serve` will ask you to turn them on.

## The hosted wizard

`apps/deploy` is a small Next.js app on Vercel. You paste a Hetzner project API token, choose a location, size and SSH key, and it creates the firewall (SSH only from your IP) and the server, then shows the commands for the SSH step. Servers it creates carry the label `managed-by=agent-hq-deploy`, and it will only show, change or delete servers with that label. Deleting needs the server's name typed back.

- The token goes in the request body of each call and is used to call `api.hetzner.cloud`. It is never stored, logged, put in a URL or sent to analytics, and it is gone when the tab closes. Use a token for a project you can delete it from, and delete it in Hetzner when you are done.
- It can only reach your own project, since that is all the token allows. The Vercel app has no rate limiting of its own beyond Hetzner's API limits and Vercel's edge protection.
- It cannot see whether HQ is ready (HQ is tailnet-only), only Hetzner's server status. The page lists the SSH steps and how to read the bootstrap log.
- The site runs with a closed content security policy and loads no third-party scripts.
- The deployment pins the bootstrap with `HQ_BOOTSTRAP_REF` (a 40-character commit) and `HQ_BOOTSTRAP_SHA256` (that file's sha256), optionally `HQ_BOOTSTRAP_REPO`. Bump both together:

  ```
  git show <commit>:deploy/hetzner/bootstrap.sh | shasum -a 256
  ```

## Security model

- **No secrets in user-data.** It holds only the repo, a commit SHA, that file's sha256 and a hostname. The SSH key is attached by Hetzner and is a public key.
- **Token and MCP key are generated on the server**, kept in `/etc/hq/mcp.env` (root-owned directory, mode 0600, so HQ's own agents cannot replace it) and loaded through a systemd drop-in. The hosted wizard, Vercel and Hetzner never see them. The token is the full admin credential for HQ, so treat the sign-in link like a password. If it leaks, replace `HQ_WEB_AUTH_TOKEN` in `/etc/hq/mcp.env` and run `systemctl restart hq`.
- **Pinned and verified.** The bootstrap is fetched at a commit SHA and checked against a sha256. The source is fetched by commit SHA. HQ itself is installed by `deploy/install.sh`, which verifies the release signature. Tailscale comes from its apt repository, and the signing key's fingerprint is checked before it is trusted.
- **Public exposure.** HQ binds to loopback. The only inbound port is SSH, keys only. The Hetzner Cloud Firewall limits it to your IP, and once you have joined the tailnet you should remove that rule (Hetzner console, or `hcloud firewall delete-rule`). The host firewall keeps SSH and Tailscale's UDP port open on purpose, so a server created without the Hetzner firewall is not private until you add one. `hq-join --close-ssh` also closes SSH in `ufw` once you have confirmed SSH works over the tailnet.
- **Not tagged.** The server joins your tailnet as a normal node, so your tailnet ACLs decide who can reach it. The web token is still required. Node keys expire after 180 days by default: disable key expiry for the node in the admin console, or, if it lapses, re-add a Hetzner firewall rule for your IP and run `sudo tailscale up` over SSH.

## Retry and delete

`bootstrap.sh` and `hq-join` can be re-run; after `hq-join` has succeeded, `bootstrap.sh` leaves the firewall and joined state alone. If bootstrap failed, read `/var/log/cloud-init-output.log`, then re-run `bash /root/bootstrap.sh --repo <repo> --ref <sha> --hostname <name>`. To delete everything, remove the server and its firewall in the Hetzner console (or `hcloud server delete`), then remove the machine from your tailnet admin page.

## Cost

You pay Hetzner for the server. Choose a size with at least 4 GB of memory for comfortable headroom; there is no source build on the server.

## Checks

`bash deploy/hetzner/test-bootstrap.sh` runs shellcheck, a dry run (`bootstrap.sh --plan`) and a check that user-data carries no secrets. A real run on a throwaway Hetzner project is still required before relying on this: `tailscale up` and `tailscale serve` behaviour, the firewall flow and the SSH hardening order have not been run on a live server yet.
