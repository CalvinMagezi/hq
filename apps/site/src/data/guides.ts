import { android, mac } from './get';
import type { Step } from './get';

export interface Guide {
  id: string;
  title: string;
  time: string;
  summary: string;
  intro: string;
  steps: Step[];
  wrong: { symptom: string; fix: string }[];
  doc?: { href: string; text: string };
}

// Every label below is a string the app shows (apps/strings/en.yaml), so a guide that names a
// renamed screen is a bug to fix when the strings change.
export const guides: Guide[] = [
  {
    id: 'install-android',
    title: 'Install HQ on your Android phone',
    time: '10 minutes',
    summary: 'Download the app, get past the two Android warnings and open it.',
    intro: 'HQ is not on the Play Store yet, so Android asks you to confirm twice. Both prompts are normal for an app that does not come from a store.',
    steps: android.steps,
    wrong: [
      { symptom: 'My browser says the file may be harmful', fix: 'Every app that does not come from a store gets this message. Tap Keep.' },
      { symptom: 'Install is greyed out', fix: "Open the browser's install setting and turn on Allow from this source, then go back and try again." },
      { symptom: 'Play Protect says App scan recommended', fix: 'Tap Scan app, wait for This app looks safe, then tap Install. You can also tap More details and choose Install without scanning, which asks for your PIN or fingerprint. The file is signed.' },
    ],
    doc: { href: '/docs/hq-mobile/', text: 'The Android guide in full' },
  },
  {
    id: 'ai-key',
    title: 'Get an AI key and set a spending limit',
    time: '5 minutes',
    summary: 'HQ needs a key from an AI service to think. You pay that service only for what you ask, and you set a cap.',
    intro: 'A key is a long password that lets HQ ask an AI service questions on your account. HQ keeps it on your device and never shows it again.',
    steps: [
      { title: 'Pick a service and make a key', body: 'OpenRouter reaches many models with one key. Anthropic gives you Claude directly. Google AI gives you Gemini directly. Each service has a page where you create a key, and the account needs a little credit.', link: { href: '/docs/first-run/', text: 'The links to each service' } },
      { title: 'Cap the key where you made it', body: 'Most services let you set a spending cap or an expiry on the key itself. Do that first: it is the limit that works even if something goes wrong on your side.' },
      { title: 'Paste it into HQ', body: 'On the first run HQ asks for the key. Paste it and tap Check key. If you skipped it, open Settings, then AI service and keys.' },
      { title: 'Set a limit inside HQ too', body: 'In the web app, open Spending to set daily, weekly or monthly limits, for all AI use or for one service or model. On the phone, Settings then Spending limits shows your budgets and a forecast for the month. Changing a limit is in the web app for now.' },
    ],
    wrong: [
      { symptom: 'It says auth error (401)', fix: 'The service refused the key. Look for a stray space or character, an expired key, or a key from a different service.' },
      { symptom: 'It says rate limited or overloaded', fix: 'The service is busy or the key hit its cap. Try again later or raise the cap.' },
      { symptom: "HQ says the AI service isn't answering", fix: 'HQ itself is fine and your Vault and tasks still work. Check the key under Settings, AI service and keys, and that the account still has credit.' },
    ],
    doc: { href: '/docs/first-run/', text: 'First run, with the fix for each message' },
  },
  {
    id: 'first-note',
    title: 'Save your first note',
    time: '2 minutes',
    summary: 'Notes live in your Vault as plain markdown files you own.',
    intro: 'The Vault is where HQ keeps what you save: notes, memory and chat history, as ordinary text files on your device.',
    steps: [
      { title: 'Open the Vault', body: 'Tap Vault in the bar at the bottom.' },
      { title: 'Tap New note', body: 'Give it a title and write a line or two, then tap Save. It saves to your Inbox folder unless you pick another.' },
      { title: 'Find it again', body: 'Your note is listed under Recent in the Vault. Tap it to read it, or tap Edit to change it.' },
    ],
    wrong: [{ symptom: 'I do not see my note', fix: 'Look in the Inbox folder, which is where a new note is saved unless you pick another folder.' }],
    doc: { href: '/docs/hq-mobile/', text: 'More about the Vault on the phone' },
  },
  {
    id: 'first-question',
    title: 'Ask your first question',
    time: '3 minutes',
    summary: 'Chat is where you ask HQ for things. It can read your Vault and tasks, and it answers to you.',
    intro: 'HQ reads what you add to a chat before it answers. It can read and write your Vault and tasks, and anything that changes something outside them waits for your OK.',
    steps: [
      { title: 'Open Chat', body: 'Tap Chat in the bar. An empty chat suggests things HQ can do on this install.' },
      { title: 'Type in the box that says Message HQ', body: 'Ask anything, for example to summarise the note you just saved.' },
      { title: 'Keep what you like', body: 'Under a reply you can open a note in the Vault or turn the answer into a task.' },
    ],
    wrong: [
      { symptom: 'It does not answer', fix: 'Check the AI key under Settings, AI service and keys. The guide on keys has the usual causes.' },
      { symptom: 'HQ asks me to approve something', fix: 'A call that changes things outside your Vault and tasks waits for you in the Inbox. Allow once, or decline.' },
    ],
    doc: { href: '/docs/owner-guide/', text: 'What HQ asks you and why' },
  },
  {
    id: 'connect-computer',
    title: 'Connect your Mac so HQ can run helpers',
    time: '15 minutes',
    summary: 'One line, a QR code and six digits. No account and no VPN.',
    intro: 'A computer you connect runs the coding helpers your phone starts. The easiest start is on the phone: Helpers, then Start a helper, then Copy shows the exact line to paste.',
    steps: mac.steps,
    wrong: [
      { symptom: 'Terminal says command not found: brew', fix: 'Install Homebrew first from brew.sh, then run the minisign line again.' },
      { symptom: 'The Mac is not listed under Helpers', fix: 'Run the pairing again and check the six digits were typed on the Mac. The pairing guide lists the causes.' },
    ],
    doc: { href: '/docs/pairing/', text: 'Pairing in full' },
  },
  {
    id: 'backup',
    title: 'Back up your Vault',
    time: '3 minutes',
    summary: 'One file that brings your notes and tasks back if the phone is lost.',
    intro: 'The Vault is your notes and tasks. If the only copy lives on one device, losing it loses them, so make a copy somewhere else.',
    steps: [
      { title: 'Open Settings, then Backup and moving phones', body: 'Scroll down to the Your data and this app group. The page says plainly that your Vault is only on this phone until you save a copy.' },
      { title: 'Tap Save a copy now', body: 'Choose a place that is not this phone, such as a cloud drive folder, then tap Save. The copy is not locked with a password, so anyone with the file can read your Vault. Your keys are not in it.' },
      { title: "Restore on a new phone", body: "On the new phone's first screen choose Restore from a backup or a copy, and pick the file." },
    ],
    wrong: [
      { symptom: 'It says the phone is almost full', fix: 'Free up the space it names and try again, or choose another place.' },
      { symptom: 'It says the place stopped answering', fix: 'Choose another place and try again.' },
    ],
    doc: { href: '/docs/backup/', text: 'Backups on a computer, encrypted and scheduled' },
  },
];
