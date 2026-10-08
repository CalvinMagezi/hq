import { Wizard } from '~/components/Wizard'

const DOCS_URL = 'https://github.com/CalvinMagezi/hq/blob/main/docs/HETZNER.md'

export default function Page() {
  return (
    <main>
      <h1>Deploy Agent HQ on Hetzner</h1>
      <p className="muted">
        Creates a server in your own Hetzner project that installs signed HQ releases, keeps itself updated and is reachable only from your
        tailnet. Nothing secret goes into the server&apos;s setup data, and the admin token is generated on the server, so this site never sees it.
      </p>
      <Wizard />
      <p className="muted">
        How it works, the security model, retry and delete: <a href={DOCS_URL}>docs/HETZNER.md</a>.
      </p>
    </main>
  )
}
