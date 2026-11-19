import { links } from './site';

export interface Fact {
  label: string;
  value: string;
  note: string;
  alert?: boolean;
}

export interface Step {
  title: string;
  body: string;
  code?: string;
  link?: { href: string; text: string };
}

export const DEVICES = [
  { id: 'android', label: 'Android', detected: 'An Android phone' },
  { id: 'iphone', label: 'iPhone', detected: 'An iPhone' },
  { id: 'mac', label: 'Mac', detected: 'A Mac' },
  { id: 'windows', label: 'Windows', detected: 'A Windows PC' },
  { id: 'linux', label: 'Linux', detected: 'A Linux computer' },
  { id: 'server', label: 'Server', detected: 'A server' },
] as const;

export const SIZE_NOTE = '88.6 MB';

export const android = {
  facts: [
    { label: 'Time', value: 'About 10 minutes', note: 'Plus getting an AI key.' },
    { label: 'Download', value: SIZE_NOTE, note: 'Use Wi-Fi if you can.' },
    { label: 'You need', value: 'Android 8 or newer', note: 'A 64-bit phone, and an AI key.' },
    { label: 'Cost', value: 'Free', note: 'The AI service charges for what you ask. You set a limit in HQ.' },
    { label: 'Heads up', value: 'Android will ask', note: 'HQ is not on the Play Store yet. Step 3 shows what you will see.', alert: true },
  ] satisfies Fact[],
  steps: [
    { title: 'Download the app', body: 'Tap Download for Android. Your browser may say this type of file can harm your device. That message appears for every app that does not come from a store. Tap Keep.' },
    { title: 'Let your browser install it', body: 'Android asks once whether your browser may install apps. Turn on Allow from this source, then press back.' },
    { title: 'Confirm the install, and let Play Protect scan it', body: 'Android asks you to confirm; on an update the button says Update. Google Play Protect may then say App scan recommended, because HQ is not on the Play Store yet. Tap Scan app, wait a few seconds for This app looks safe, then tap Install. More details also offers Install without scanning, which asks for your PIN or fingerprint. The file is signed, and you can check it yourself below.' },
    { title: 'Open HQ and answer three short screens', body: 'Welcome, your AI service, then Start. If you are moving from another phone, you can restore a backup from here.' },
    { title: 'Paste your AI key', body: 'Choose OpenRouter, Anthropic or another service, and paste the key. Tap Check key to make sure it works. HQ keeps it sealed on the phone and never writes it to a file.' },
  ] satisfies Step[],
};

export const mac = {
  facts: [
    { label: 'Time', value: 'About 15 minutes', note: 'Mostly waiting.' },
    { label: 'You need', value: 'macOS and Terminal', note: 'Terminal is already on your Mac. A small tool that checks the download is genuine comes first.' },
    { label: 'Cost', value: 'Free', note: 'An AI key only if this Mac runs HQ on its own.' },
    { label: 'Heads up', value: 'The Mac app is not signed yet', note: 'HQ Hub exists, but it is not offered as a download until it is signed.', alert: true },
  ] satisfies Fact[],
  steps: [
    { title: 'Open Terminal', body: 'Press Command and Space, type Terminal, press Return. A window with a blinking cursor opens. That is all it is.' },
    { title: 'Install the checker, once', body: 'HQ checks that every download is signed by the project, and a Mac needs one small tool for that. Paste this line and press Return.', code: 'brew install minisign', link: { href: 'https://brew.sh', text: 'If Terminal says command not found: brew, install Homebrew first' } },
    { title: 'Paste the HQ line', body: 'This installs HQ without the web app and shows a code for your phone to scan.', code: 'curl -fsSL https://agent-hq.online/install.sh | bash -s -- --pair' },
    { title: 'Pair with your phone', body: 'The Mac shows a QR code. On your phone, tap Scan the code. The phone shows six digits. Type them on the Mac. No account and no VPN.' },
    { title: 'Check it worked', body: 'Your phone lists this Mac under Helpers. To run HQ on the Mac itself, leave off --pair.' },
  ] satisfies Step[],
};

export const windows = {
  facts: [
    { label: 'Cost', value: 'Free', note: 'An AI key only if this computer runs HQ on its own.' },
    { label: 'You need', value: 'Windows 10 or 11', note: 'And PowerShell, which is already there.' },
    { label: 'Heads up', value: 'You can read it first', note: 'The installer is one pasted line. To read it first, download install.ps1 and open it in Notepad.', alert: true },
  ] satisfies Fact[],
  steps: [
    { title: 'Open PowerShell', body: 'Press the Windows key, type PowerShell, press Enter.' },
    { title: 'Paste the installer line', body: 'Right-click in the window to paste, then press Enter. It checks your computer, suggests an edition and asks you to confirm.', code: 'irm https://agent-hq.online/install.ps1 | iex' },
    { title: 'Say yes, restart if asked, paste the line again', body: 'If Windows asks to restart, restart, then run the same line. It picks up where it stopped.' },
    { title: 'Open HQ', body: 'In your browser, go to http://localhost:5678. To add this computer to the HQ on your phone as a helper, follow the pairing steps for Linux inside WSL2.', link: { href: links.windows, text: 'The Windows guide' } },
  ] satisfies Step[],
};

export const windowsEditions = [
  { tag: 'Full HQ', green: true, title: 'If your computer lets you install things', body: 'Coding helpers run inside WSL2, the Linux that Windows includes. Windows asks for administrator approval and may restart once or twice. You make a short Linux username and password.', time: 'About 20 minutes, mostly waiting.' },
  { tag: 'HQ Lite', green: false, title: 'If it is a work computer that blocks installers', body: 'Needs no administrator rights. Includes the web app, tasks and your Vault. No coding helpers. Windows may refuse it because the file is not code-signed yet. The installer tells you and stops. Nothing is broken.', time: 'About 5 minutes.' },
];

export const linux = {
  facts: [
    { label: 'Time', value: 'About 10 minutes', note: 'hq doctor says what to fix, if anything.' },
    { label: 'You need', value: 'A terminal', note: 'And an AI key if this computer runs HQ on its own.' },
    { label: 'Remove it', value: 'Two folders', note: 'Delete ~/.local/bin/hq and ~/.local/share/agent-hq. Your Vault is separate and stays.' },
  ] satisfies Fact[],
  commands: [
    { title: 'Install', code: 'curl -fsSL https://agent-hq.online/install.sh | bash' },
    { title: 'Set up, add a key, check', code: 'hq install && hq env && hq doctor' },
    { title: 'Start HQ, then open http://localhost:5678', code: 'hq start all' },
  ],
};

export const server = {
  facts: [
    { label: 'You need', value: 'Five small things', note: 'A Hetzner account with a card, a Hetzner access token, a free Tailscale account, an SSH key and an AI key.' },
    { label: 'Time', value: '3 to 5 minutes', note: 'Of waiting, once the accounts are ready.' },
    { label: 'Remove it', value: 'The same form', note: 'The form page deletes the server, its firewall and its key.' },
  ] satisfies Fact[],
  steps: [
    { title: 'Make a Hetzner access token', body: 'In the Hetzner Cloud console, open Security, then API tokens, and make one with Read and Write.' },
    { title: 'Fill in the form', body: 'Token, a name, where the server lives, its size, your SSH public key and your own IP address.', link: { href: 'https://deploy.agent-hq.online', text: 'Open the deploy form' } },
    { title: 'Connect and sign in to Tailscale', body: 'When setup is done, run the line below, sign in to Tailscale, and note the HQ link it prints.', code: 'ssh root@<server-ip> hq-join' },
    { title: 'Open the link and add your key', body: 'Open the link on a device that is on your Tailscale network. HQ asks for your AI key. Then press the button on the form page to close public SSH.' },
  ] satisfies Step[],
  docker: 'docker run -d --name hq --restart unless-stopped -p 127.0.0.1:5678:5678 -v hq-data:/data ghcr.io/calvinmagezi/hq',
};
