#!/usr/bin/env node
import { parseArgs } from "node:util";
import { DEFAULT_REPO } from "../lib/release.js";
import { defaultPrefix, hasPrebuilt, installFromSource, installPrebuilt, pathAdvice } from "../lib/install.js";

const HELP = `agent-hq: install HQ, the local-first AI agent hub

usage: npx agent-hq-cli [install] [options]

options:
  --channel <name>   release channel: stable (default) or main
  --prefix <dir>     where to put the hq binary (default ${defaultPrefix()})
  --repo <owner/repo> release repository (default ${DEFAULT_REPO})
  --from-source      build with cargo instead of downloading a binary
  -h, --help         show this help

The download is verified against the project's minisign key before it is installed.`;

const NEXT_STEPS = `
Next steps:
  hq install        scaffold your vault and config
  hq env            add an LLM API key (OpenRouter, Anthropic or Google)
  hq doctor         check the setup
  hq chat           talk to HQ, or: hq start all   (daemon, API and web UI on :5678)

Always-on server: https://github.com/${DEFAULT_REPO}/blob/main/deploy/README.md`;

async function main() {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      channel: { type: "string", default: "stable" },
      prefix: { type: "string", default: defaultPrefix() },
      repo: { type: "string", default: DEFAULT_REPO },
      "from-source": { type: "boolean", default: false },
      help: { type: "boolean", short: "h", default: false },
    },
  });
  const command = positionals[0] ?? "install";
  if (values.help || command === "help") return console.log(HELP);
  if (command !== "install") throw new Error(`unknown command "${command}"; try --help`);

  if (process.platform === "win32") {
    throw new Error("HQ has no native Windows build. Open an Ubuntu (WSL2) terminal and run npx agent-hq-cli there. Guide: https://agent-hq.online/install/#windows");
  }
  if (values["from-source"] || !hasPrebuilt()) {
    if (!values["from-source"]) console.log("No prebuilt binary for this platform; building from source.");
    installFromSource({ repo: values.repo });
  } else {
    await installPrebuilt({ repo: values.repo, channel: values.channel, prefix: values.prefix });
    const advice = pathAdvice(values.prefix);
    if (advice) console.log(`\n${advice}`);
    console.log(`Updates later: hq update --check (see the project docs for the signed updater).`);
  }
  console.log(NEXT_STEPS);
}

main().catch((error) => {
  console.error(`agent-hq: ${error.message}`);
  process.exit(1);
});
