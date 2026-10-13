# Your HQ: a guide for owners

Someone set up an HQ server for you. This page covers what you do on your side. You won't need a
terminal.

## Before the server joins (about 10 minutes)

HQ is private: only devices on your own Tailscale network can open it.

1. Create a free account at [tailscale.com](https://tailscale.com).
2. In the Tailscale admin console, open **DNS**. Turn on **MagicDNS**, then **HTTPS
   Certificates**. HQ can't publish itself without both.
3. Install the Tailscale app on your phone and on any computer you'll use, and sign in to the
   same account on each.
4. Tell the person setting up your server when you're ready. They'll send you a Tailscale sign-in
   link: open it and approve the server.

Once the server shows up in your Tailscale admin console, open its menu and choose **Disable key
expiry**. Otherwise it drops off your network after 180 days and needs the setup person again.

## Signing in

You'll receive an HQ login link that ends in `#token=...`. **Treat it like a password.** Anyone
on your Tailscale network who has it can use your HQ.

1. With Tailscale on, open the link. HQ remembers the sign-in on that browser.
2. The first time, HQ asks for a model key: an OpenRouter, Anthropic or Google AI key. The page
   links to where you get one.

## Putting HQ on your phone's home screen

- **iPhone (Safari):** open HQ, tap **Share**, then **Add to Home Screen**.
- **Android (Chrome):** open HQ, tap the menu, then **Install app** or **Add to Home screen**.

If the installed app asks you to sign in, paste the same login link into the box it shows, or
use a code from another device (next section).

## Signing in on another device

Your login link signs in one browser. To add a phone, a tablet or another computer, use a device
that is already signed in. The new device needs Tailscale on and signed in to your account first.

1. On the signed-in device, open **Settings** and find **Devices**.
2. Press **Sign in another device**. HQ shows a code like `ABCDE-FGH2J` and a QR code.
3. On the new device, either scan the QR code with the camera and open the link, or open HQ and
   type the code into the **Sign in to HQ** box. Capitals, spaces and the hyphen don't matter.

The code works once and lasts ten minutes. **New code** gives you another and stops the old one.
After five wrong tries, wait a few minutes.

Treat the QR code like a password: anyone who scans it is signed in to your HQ. Show it only to
your own devices, and press **Hide** when you're done. When a device signs in, HQ tells you in its
inbox (and on your phone, if notifications are on). If you didn't expect that, tell whoever runs
your server.

## If you're signed out

HQ shows a **Sign in to HQ** box. Paste your login link there, or enter a code from a device
that is still signed in ([Signing in on another device](#signing-in-on-another-device)).

If you've lost the link and your HQ bot is connected on Telegram or Discord, ask it for the link.
Anyone with the link can sign in as you, so keep it out of chat history:

- **Telegram:** send `/login` to the bot in a private chat. Delete its reply once you've used it.
- **Discord:** in a direct message to the bot, type `/login` and pick the slash command. Only you
  see the reply, and Discord doesn't keep it.

Otherwise, ask the person who set up your server. They can print it again by running
`sudo hq-join --print-link` on the server.

## When something looks wrong

| You see | What to do |
|---------|------------|
| **Can't reach your HQ** | Switch Tailscale on, on this device, and tap **Try again**. If Tailscale is on, wait a minute (HQ may be updating). Still nothing: check your HQ server shows as connected in the Tailscale admin console, and if it doesn't, ask whoever set it up. |
| **This device is offline** | You have no internet. HQ shows what it saved and reconnects by itself. |
| **Sign in to HQ** | Paste your login link into the box. Lost it? See [If you're signed out](#if-youre-signed-out). |
| A page won't open at all, not even saved data | Same as "Can't reach your HQ": Tailscale first. |
