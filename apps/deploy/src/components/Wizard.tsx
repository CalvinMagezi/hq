'use client'

import { useState } from 'react'
import { call, type Options, type ServerInfo } from '~/lib/client'
import { TokenStep } from './TokenStep'
import { ServerForm, type CreateRequest } from './ServerForm'
import { ServerPanel } from './ServerPanel'

export function Wizard() {
  const [token, setToken] = useState<string | null>(null)
  const [options, setOptions] = useState<Options | null>(null)
  const [servers, setServers] = useState<ServerInfo[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const attempt = async (run: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await run()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong.')
    } finally {
      setBusy(false)
    }
  }

  const connect = (t: string) =>
    attempt(async () => {
      const opts = await call<Options>('options', { token: t })
      setToken(t)
      setOptions(opts)
      setServers(opts.existing)
    })

  const create = (req: CreateRequest) =>
    attempt(async () => {
      const made = await call<{ serverId: number; name: string; ip: string | null }>('create', { token, ...req })
      setServers((s) => [...s, { id: made.serverId, name: made.name, status: 'initializing', ip: made.ip }])
    })

  if (!token || !options) return <TokenStep busy={busy} error={error} onSubmit={connect} />
  return (
    <>
      {servers.map((s) => (
        <ServerPanel key={s.id} token={token} server={s} callerIp={options.callerIp} onGone={() => setServers((all) => all.filter((x) => x.id !== s.id))} />
      ))}
      <ServerForm options={options} busy={busy} error={error} onSubmit={create} />
    </>
  )
}
