# HQ on a work computer

For a computer your employer manages: VS Code is there, perhaps with a company GitHub Copilot seat,
and you may not be able to install WSL2 or even run a program you downloaded. This page is the
checklist. The edition for this situation is [HQ Lite](HQ_LITE.md); Full HQ in WSL2 is better
wherever your IT policy allows it ([WINDOWS.md](WINDOWS.md)).

Nothing here works around a policy. If your organization has blocked a route, the answer is a
conversation with the people who own the policy, and this page tells you what to ask for.

## Use the lightest thing that works

Try these in order. Each needs less of the computer than the next.

1. **Nothing installed here.** HQ runs somewhere else (a server you or your team control, over
   HTTPS), and VS Code connects to it. No program, no admin, immune to program-blocking policies.
   Needs MCP allowed for Copilot and the server's address reachable.
2. **HQ Lite in your user profile.** One program, no administrator. Needs the policy to let
   programs from your profile run.
3. **Full HQ in WSL2.** Everything, but WSL2 has to be turned on by an administrator and allowed by
   policy.

## 1. Connect VS Code to an HQ that runs elsewhere

Ask whoever runs that HQ for a **tasks key** (not the full key; the tasks key can use tasks and
nothing else: no notes, no sessions, no code execution). Then in your project:

```
hq mcp install --target project --url https://mcp.example.com/mcp
```

or write `.vscode/mcp.json` yourself:

```json
{
  "inputs": [
    { "id": "agent-hq-key", "type": "promptString", "description": "Agent HQ MCP key", "password": true }
  ],
  "servers": {
    "agent-hq": {
      "type": "http",
      "url": "https://mcp.example.com/mcp",
      "headers": { "Authorization": "Bearer ${input:agent-hq-key}" }
    }
  }
}
```

VS Code asks for the key when it starts the server; it is not stored in the file. Details:
[VPS_AGENT_CONNECT.md](VPS_AGENT_CONNECT.md).

## 2. HQ Lite on this computer

```
irm https://agent-hq.online/install.ps1 | iex
```

Choose Lite when it recommends it. Then:

```
hq.exe mcp install --target vscode --scope tasks     # VS Code launches HQ over stdio, no port
hq.exe web                                           # the web app, on this computer only
hq.exe doctor --egress                               # what, if anything, could leave this computer
```

HQ Lite refuses to start while anything that would send your notes to another service is
configured, so a Lite instance cannot be turned into a path around your company's data rules by
accident. The only model route it accepts without being told is your company's own Copilot seat
through GitHub's CLI (`hq copilot link`).

If MCP is switched off for Copilot but terminal commands are allowed, agents can use the same tasks
and notes through `hq task` and `hq search --json`; `hq copilot init` writes the instructions that
tell Copilot how. See [HQ_LITE.md](HQ_LITE.md).

### If the program will not start

The installer runs `hq.exe --version` once. If Windows answers that your organization blocked it,
that is AppLocker, Windows Defender Application Control or Smart App Control working as designed.
The installer removes the file and stops. Your options:

- Use route 1 above; it needs no program here.
- Ask IT to allow the program. The installer prints the SHA-256 of `hq.exe` itself when it is
  refused (AppLocker and WDAC hash rules match the program, not the zip), plus the release it came
  from; say it is unsigned for now. Code signing is
  the planned fix and will make this a publisher rule instead of a per-file one.
- Ask whether WSL2 is permitted, and use Full HQ.

### What to ask IT, in one message

> I would like to use a local task and notes tool with VS Code Copilot. It is a single open-source
> program (HQ Lite) that runs in my user profile with no administrator rights, listens on the
> loopback address only, and sends nothing to any service unless the company Copilot is used. Could
> you allow `hq.exe` (SHA-256 on the release page), or confirm that "MCP servers in Copilot" is
> allowed for this server address?

## Before you start: a read-only look at your computer

These only read. Paste the results to IT if they ask.

```powershell
$ExecutionContext.SessionState.LanguageMode            # FullLanguage is needed for the installer
wsl --status                                           # is WSL2 present?
(Get-CimInstance Win32_ComputerSystem).HypervisorPresent
(Get-CimInstance Win32_Processor).VirtualizationFirmwareEnabled
Get-AppLockerPolicy -Effective -ErrorAction SilentlyContinue | Select-Object -ExpandProperty RuleCollections
```

An empty last line usually means AppLocker is not enforcing. The installer does the first four for
you.

## Keep work data on work systems

- Notes and tasks you put into HQ on a work computer are work data. Keep that HQ on the computer, or
  on a server your employer approves. Do not point a work HQ at a personal model key; Lite refuses to.
- Copilot Business and Enterprise do not train on your prompts. Retention differs between the editor
  and the CLI and GitHub's documentation is the authority on it; ask your administrator if it
  matters for what you will put in.
- `hq doctor --egress` is the check to run after any configuration change.
