'use client'

import { useEffect, useState } from 'react'
import { call, hostRange, type ServerInfo } from '~/lib/client'
import { CopyCommand } from './CopyCommand'

const POLL_MS = 5_000
const DELETE_RETRIES = 8
const DELETE_RETRY_MS = 4_000

export function ServerPanel({ token, server, callerIp, onGone }: { token: string; server: ServerInfo; callerIp: string; onGone: () => void }) {
  const [info, setInfo] = useState(server)
  const [message, setMessage] = useState<string | null>(null)
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    const load = () => call<ServerInfo>('status', { token, serverId: server.id }).then(setInfo).catch(() => undefined)
    load()
    const timer = window.setInterval(load, POLL_MS)
    return () => window.clearInterval(timer)
  }, [token, server.id])

  const act = async (run: () => Promise<string>) => {
    setBusy(true)
    setMessage(null)
    try {
      setMessage(await run())
    } catch (e) {
      setMessage(e instanceof Error ? e.message : 'Failed.')
    } finally {
      setBusy(false)
    }
  }

  const closeSsh = () => act(async () => { await call('ssh-rule', { token, serverId: info.id }); return 'Public SSH is closed.' })
  const openSsh = () => act(async () => { await call('ssh-rule', { token, serverId: info.id, openFrom: hostRange(callerIp) }); return 'SSH is open to your IP again.' })
  const remove = () => act(async () => {
    for (let attempt = 0; attempt < DELETE_RETRIES; attempt++) {
      const r = await call<{ deleted: boolean; firewallsRemoved?: boolean }>('delete', { token, serverId: info.id, confirmName: confirm })
      if (r.deleted) {
        onGone()
        return r.firewallsRemoved ? 'Server, firewall and SSH key deleted.' : 'Server deleted. Remove its firewall and key in the Hetzner console.'
      }
      await new Promise((resolve) => window.setTimeout(resolve, DELETE_RETRY_MS))
    }
    return 'Hetzner is still deleting the server. Check the Hetzner console, then press Delete again.'
  })

  return (
    <section className="card stack">
      <h2>{info.name}</h2>
      <p>Hetzner status: <strong>{info.status}</strong>{info.ip && <> at <code>{info.ip}</code></>}</p>
      <ol className="steps">
        <li>
          <strong>Wait about 3 to 5 minutes</strong> after the status shows <code>running</code>. The server installs HQ on its own at first boot. HQ is private to your tailnet, so this page cannot see when it is done.
        </li>
        <li>
          <strong>Join your tailnet.</strong> Run this on your computer. It opens a Tailscale login link: open it and sign in. When it finishes it prints your HQ link.
          <CopyCommand command={`ssh root@${info.ip ?? '<server-ip>'} hq-join`} />
          <small className="muted">First time, answer yes to the host fingerprint. If your SSH key is not your default one, add <code>-i ~/.ssh/your-key</code> after <code>ssh</code>. Tailscale needs MagicDNS and HTTPS certificates turned on in its admin console (DNS).</small>
        </li>
        <li>
          <strong>Open the HQ link</strong> on a device that is on your tailnet. Treat it like a password: it signs you in as admin.
        </li>
        <li>
          <strong>Close public SSH</strong> once HQ opens, so only your tailnet can reach the server.
        </li>
      </ol>
      <p className="muted">If it does not come up, read <code>/var/log/cloud-init-output.log</code> on the server and see the docs below.</p>
      <div className="row">
        <button type="button" disabled={busy} onClick={closeSsh}>Close public SSH</button>
        <button type="button" className="quiet" disabled={busy || !hostRange(callerIp)} onClick={openSsh}>Reopen SSH for my IP</button>
      </div>
      <label>
        Delete this server and its firewall. Type <code>{info.name}</code> to confirm.
        <input value={confirm} onChange={(e) => setConfirm(e.target.value)} />
      </label>
      <button type="button" className="danger" disabled={busy || confirm !== info.name} onClick={remove}>Delete server</button>
      {message && <p role="status">{message}</p>}
    </section>
  )
}
