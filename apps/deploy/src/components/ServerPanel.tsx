'use client'

import { useEffect, useState } from 'react'
import { call, hostRange, type ServerInfo } from '~/lib/client'

const POLL_MS = 5_000
const SSH_WINDOW_NOTE = 'Close it once the server shows up in your tailnet.'

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
    const r = await call<{ deleted: boolean; firewallsRemoved: boolean }>('delete', { token, serverId: info.id, confirmName: confirm })
    if (!r.deleted) return 'Hetzner has not finished deleting the server. Check the Hetzner console, then try again.'
    onGone()
    return r.firewallsRemoved ? 'Server, firewall and SSH key deleted.' : 'Server deleted. Remove its firewall and key in the Hetzner console.'
  })

  return (
    <section className="card stack">
      <h2>{info.name}</h2>
      <p>Hetzner status: <strong>{info.status}</strong>{info.ip && <> at <code>{info.ip}</code></>}</p>
      <p className="muted">HQ is private to your tailnet, so this page cannot see when it is ready. Give it a few minutes after the server shows running, then:</p>
      <ol className="steps">
        <li>Connect: <code>ssh root@{info.ip ?? '<server-ip>'}</code></li>
        <li>Run <code>sudo hq-join</code>, open the login link it prints, and sign in to your own tailnet.</li>
        <li>Open the sign-in link it prints on a device that is on your tailnet. HQ asks for a model key on first run.</li>
        <li>{SSH_WINDOW_NOTE}</li>
      </ol>
      <p className="muted">If setup did not finish, read <code>/var/log/cloud-init-output.log</code> on the server and re-run <code>bash /root/bootstrap.sh</code> with the same arguments (see the docs below).</p>
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
