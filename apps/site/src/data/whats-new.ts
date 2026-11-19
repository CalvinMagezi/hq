export interface NewItem {
  title: string;
  body: string;
}

// Written by hand from CHANGELOG.md for people, not developers. The full changelog stays on GitHub.
export const CHECKED_ON = '2026-11-19';

export const whatsNew: NewItem[] = [
  { title: 'The app has four places: Vault, Chat, Inbox and Settings', body: 'The web and desktop app were rebuilt around the same four places as the phone. The Vault has a folder tree, Back and Forward, and a Move button for notes and folders.' },
  { title: 'Connect a computer with one line', body: 'With no computer paired, the phone\'s Start a helper sheet shows the exact line to paste, with a Copy button, and a Scan the code button for the pairing.' },
  { title: 'Answer a helper\'s question from the notification', body: 'When a coding helper waits for your OK, the notification says what it asks and has buttons such as Allow once, so you do not have to open the app first.' },
  { title: 'Long chats fold instead of forgetting', body: 'Older messages are summarised into blocks you can search and restore, instead of being dropped after about 20 messages.' },
  { title: 'Code from your phone with your own GitHub', body: 'HQ Coder does real work on GitHub from a phone. You make your own private GitHub App, so nothing is shared with anyone else.' },
  { title: 'Make a big document smaller', body: 'HQ Suite can turn a large document into a note: text and tables kept, pictures stored once and recompressed, with a report of what was saved and left out.' },
  { title: 'Suggestions under a reply', body: 'HQ may offer one next step under an answer, such as turning a pasted list into tasks, with Yes, Not now and Schedule. Those suggestions are chosen on your device.' },
  { title: 'A chat keeps your place', body: 'A finished reply no longer jumps the chat, and older messages added above stay put.' },
];
