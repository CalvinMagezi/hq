// Single switch for what the install page may present as available.
// Flip npx.available to true only after the package is published and tested.
export const installChannels = {
  cargo: {
    available: true,
    title: 'Build from source',
  },
  server: {
    available: true,
    title: 'Server with signed releases',
  },
  hetzner: {
    available: true,
    title: 'One form on Hetzner',
  },
  tailscale: {
    available: true,
    title: 'Remote web access over Tailscale',
  },
  curl: {
    available: true,
    title: 'Install script',
    command: 'curl -fsSL https://agent-hq.online/install.sh | bash',
  },
  brew: {
    available: true,
    title: 'Homebrew',
    command: 'brew install CalvinMagezi/tap/agent-hq',
  },
  docker: {
    available: true,
    title: 'Docker',
    command: 'docker run -d --name hq --restart unless-stopped -p 127.0.0.1:5678:5678 -v hq-data:/data ghcr.io/calvinmagezi/hq',
  },
  npx: {
    available: true,
    title: 'npx installer',
    command: 'npx agent-hq-cli',
  },
} as const;
