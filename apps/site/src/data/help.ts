export interface HelpItem {
  symptom: string;
  fix: string;
  link?: { href: string; text: string };
}

export interface HelpGroup {
  title: string;
  items: HelpItem[];
}

// Symptoms are in the words people use. Each fix comes from the docs or the app's own messages.
export const helpGroups: HelpGroup[] = [
  {
    title: 'Installing on Android',
    items: [
      { symptom: 'My browser says the file may be harmful', fix: 'Every app that does not come from a store gets this message. Tap Keep.' },
      { symptom: 'Play Protect blocked it', fix: 'Tap More details, then Install anyway. It shows because HQ is not on the Play Store yet. The file is signed.', link: { href: '/guides/install-android/', text: 'The install guide' } },
      { symptom: 'HQ stops answering when my phone sleeps', fix: 'Android can pause apps that run in the background. Leave the phone on a charger if you want HQ to reply while you sleep, and follow the app\'s Keep running screen, which lists what to change.', link: { href: '/docs/hq-mobile/', text: 'The phone guide' } },
    ],
  },
  {
    title: 'The AI service',
    items: [
      { symptom: 'It says auth error (401)', fix: 'The service refused the key. Look for a stray space or character, an expired key, or a key from a different service.', link: { href: '/guides/ai-key/', text: 'Get an AI key' } },
      { symptom: "HQ says the AI service isn't answering", fix: 'HQ itself is fine, and your Vault and tasks still work. Check the key under Settings, AI service and keys, and that the account still has credit.' },
      { symptom: 'It says rate limited or overloaded', fix: 'The service is busy or the key hit its cap. Try again later or raise the cap.' },
      { symptom: 'Chat says no provider is available', fix: 'The server runs an older release. Update with hq update --apply, or use an OpenRouter key.', link: { href: '/docs/first-run/', text: 'First run' } },
    ],
  },
  {
    title: 'Computers and servers',
    items: [
      { symptom: 'Terminal says command not found: brew', fix: 'Install Homebrew first from brew.sh, then run the minisign line again.' },
      { symptom: 'Windows will not run HQ Lite', fix: 'Windows may refuse a program that is not code-signed yet. The installer tells you and stops, and nothing is broken. Full HQ in WSL2 is the other edition.', link: { href: '/docs/corporate-workstation/', text: 'HQ on a work computer' } },
      { symptom: 'The web page loads but every panel says 401', fix: 'The browser did not keep the sign-in token. Open the sign-in link again in a fresh tab.', link: { href: '/docs/hetzner/', text: 'Deploying on Hetzner' } },
      { symptom: 'I do not know what is wrong', fix: 'Run hq doctor. It checks your setup and says what is wrong in plain words.' },
    ],
  },
];
