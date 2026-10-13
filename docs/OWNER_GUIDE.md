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

If the installed app asks you to sign in, paste the same login link into the box it shows.

## If you're signed out

HQ shows a **Sign in to HQ** box. Paste your login link (or just the token) there.

If you've lost the link and your HQ bot is connected on Telegram or Discord, ask it for the link.
Anyone with the link can sign in as you, so keep it out of chat history:

- **Telegram:** send `/login` to the bot in a private chat. Delete its reply once you've used it.
- **Discord:** in a direct message to the bot, type `/login` and pick the slash command. Only you
  see the reply, and Discord doesn't keep it.

Otherwise, ask the person who set up your server. They can print it again by running
`sudo hq-join --print-link` on the server.

## If HQ won't open at all

- Check Tailscale is on, on that device, and signed in to your account.
- Check the server still shows as connected in the Tailscale admin console.
