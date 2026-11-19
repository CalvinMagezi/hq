export const AUDIENCES = [
  { id: 'start', title: 'Starting out', blurb: 'Install HQ, connect your first computer and get your first answer.' },
  { id: 'owner', title: 'Looking after your HQ', blurb: 'Backups, updates, the Mac app, notifications and who can reach HQ.' },
  { id: 'server', title: 'On a server', blurb: 'Keep HQ running all day on a machine you rent or own.' },
  { id: 'build', title: 'For developers', blurb: 'How HQ works inside: agents, sessions, hosts and checks.' },
] as const;

export type Audience = (typeof AUDIENCES)[number]['id'];
