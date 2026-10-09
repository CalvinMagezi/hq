# Hetzner deploy: demo video script

About four minutes, screen recording with voice-over. Record on a fresh Hetzner project and a throwaway SSH key. Everything here was run for real on 2026-10-09.

## Before recording

- Hetzner account with a payment method, an empty project named `hq-demo`, and a Read and Write API token ready in the clipboard. Rotate the token right after recording.
- Tailscale signed in, MagicDNS and HTTPS certificates on, and any earlier machine with the same name removed from the admin console.
- A throwaway key: `ssh-keygen -t ed25519 -N "" -f ~/.ssh/hq_demo`, then `pbcopy < ~/.ssh/hq_demo.pub`.
- A model key capped in spend, or a free-tier one, ready to paste.
- Browser at 1440 wide, bookmarks bar hidden, other tabs closed. Terminal font large.
- Never show the token field after pasting, the HQ sign-in link, or the model key. Crop or blur them in edit.

## Shots

| # | Time | Screen | Say |
|---|------|--------|-----|
| 1 | 0:00 | Install page, "One form on Hetzner" section | "Agent HQ runs on your own server. If you have a Hetzner account, one form sets it up." |
| 2 | 0:15 | Hetzner console, project, Security, API tokens, Generate, Read and Write | "Create a project token with read and write access. It belongs to one project, so the form can only touch that project." |
| 3 | 0:40 | deploy.agent-hq.online, paste token, Continue | "Paste it here. It is used to call Hetzner for you, and it is never stored." |
| 4 | 0:55 | The form: name, location, size, SSH key, your IP | "Pick a size with at least 4 gigabytes of memory. The price is shown. Paste your public key, never the private one, and limit SSH to your own address." |
| 5 | 1:25 | Create server, status turns to running | "The server gets a firewall that only lets your address reach SSH, and a setup script that is pinned and checked against a hash." |
| 6 | 1:45 | Cut: wait 3 to 5 minutes (speed up) | "It installs HQ from signed releases by itself." |
| 7 | 2:00 | Terminal: `ssh root@<ip> hq-join` | "One command joins your private network. It prints a Tailscale login link." |
| 8 | 2:20 | Tailscale login page, approve, Login successful | "Approve the machine in your own Tailscale account." |
| 9 | 2:40 | Terminal prints the HQ link (blur it) | "It prints your HQ address. Treat it like a password." |
| 10 | 2:55 | HQ opens, "Connect a model" screen | "On first run HQ asks for a model key." |
| 11 | 3:10 | Paste key (blur), Test connection, chat opens, send "hi" | "It tests the key, saves it, and you can chat right away." |
| 12 | 3:35 | Wizard page: Close public SSH | "Last, close public SSH so only your private network reaches the server." |
| 13 | 3:50 | Delete server, type the name, confirm | "When you are done, one button deletes the server, its firewall and key. You only pay for the hours it ran." |

## After recording

- Delete the Hetzner token, the project's server (if not done in shot 13), and the Tailscale machine.
- Export a 1080p mp4 and a short GIF of shots 3 to 5 for the docs.
- Embed the mp4 in the install page's Hetzner section and link `docs/HETZNER.md`.
