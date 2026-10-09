'use client'

import { useState, type FormEvent } from 'react'
import { hostRange, type Options } from '~/lib/client'

export interface CreateRequest {
  name: string
  location: string
  serverType: string
  adminCidr: string
  backups: boolean
  sshKeyId?: number
  newSshKey?: { name: string; publicKey: string }
}

const NEW_KEY = 'new'

export function ServerForm({ options, busy, error, onSubmit }: { options: Options; busy: boolean; error: string | null; onSubmit: (r: CreateRequest) => void }) {
  const [name, setName] = useState('hq')
  const [location, setLocation] = useState(options.locations[0]?.name ?? '')
  const [serverType, setServerType] = useState('')
  const [cidr, setCidr] = useState(hostRange(options.callerIp))
  const [backups, setBackups] = useState(false)
  const [keyChoice, setKeyChoice] = useState(options.sshKeys[0] ? String(options.sshKeys[0].id) : NEW_KEY)
  const [publicKey, setPublicKey] = useState('')

  const sizes = options.serverTypes.filter((t) => t.monthlyGross[location])
  const price = (t: (typeof sizes)[number]) => `${Number(t.monthlyGross[location]).toFixed(2)} per month incl. tax`

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const base = { name, location, serverType, adminCidr: cidr.trim(), backups }
    onSubmit(keyChoice === NEW_KEY ? { ...base, newSshKey: { name: `hq-${name}`, publicKey: publicKey.trim() } } : { ...base, sshKeyId: Number(keyChoice) })
  }

  return (
    <form onSubmit={submit} className="card stack">
      <h2>Create the server</h2>
      <label>
        Name
        <input value={name} onChange={(e) => setName(e.target.value.toLowerCase())} />
        <small className="muted">Also the machine name on your tailnet.</small>
      </label>
      <label>
        Location
        <select value={location} onChange={(e) => { setLocation(e.target.value); setServerType('') }}>
          {options.locations.map((l) => <option key={l.name} value={l.name}>{l.label}</option>)}
        </select>
      </label>
      <label>
        Size
        <select value={serverType} onChange={(e) => setServerType(e.target.value)} required>
          <option value="" disabled>Choose a size</option>
          {sizes.map((t) => (
            <option key={t.name} value={t.name}>{t.name}: {t.cores} cores, {t.memoryGb} GB memory, {t.diskGb} GB disk, {price(t)}</option>
          ))}
        </select>
        <small className="muted">At least 4 GB of memory is comfortable. Hetzner bills this project directly.</small>
      </label>
      <label>
        SSH key
        <select value={keyChoice} onChange={(e) => setKeyChoice(e.target.value)}>
          {options.sshKeys.map((k) => <option key={k.id} value={k.id}>{k.name}</option>)}
          <option value={NEW_KEY}>Paste a new public key</option>
        </select>
      </label>
      {keyChoice === NEW_KEY && (
        <label>
          Public key
          <textarea rows={3} spellCheck={false} value={publicKey} onChange={(e) => setPublicKey(e.target.value)} placeholder="ssh-ed25519 AAAA... you@laptop" />
          <small className="muted">The contents of your .pub file, for example <code>cat ~/.ssh/id_ed25519.pub</code>. No key yet? Run <code>ssh-keygen -t ed25519</code> first. Never paste a private key.</small>
        </label>
      )}
      <label>
        Your IP, allowed to reach SSH
        <input value={cidr} onChange={(e) => setCidr(e.target.value)} placeholder="203.0.113.7/32" />
        <small className="muted">Only this range can connect, and only until you close the rule after joining your tailnet. Use an IPv4 address you will SSH from, a /32 for one machine or a /16 at widest.</small>
      </label>
      <label className="check">
        <input type="checkbox" checked={backups} onChange={(e) => setBackups(e.target.checked)} />
        Enable Hetzner backups (adds about 20% to the price)
      </label>
      {error && <p role="alert" className="error">{error}</p>}
      <button type="submit" disabled={busy || serverType === ''}>{busy ? 'Creating' : 'Create server'}</button>
    </form>
  )
}
