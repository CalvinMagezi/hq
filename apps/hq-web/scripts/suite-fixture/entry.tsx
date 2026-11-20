// A static page for scripts/suite-mobile-check.cjs: the suite views with invented notes and a small pretend HQ behind
// them (one note, its blocks and its review comments kept in memory, no network). `?view=sheet|slides|dashboard|doc|charts`
// picks one, `?theme=light` the light tokens, `?density=compact` the grid density. It is bundled by the check itself and is
// not part of the app.
import { useCallback, useState } from 'react'
import { createRoot } from 'react-dom/client'
import '../../src/styles/aligned-ui.css'
import { SuiteViewer } from '../../src/components/suite/SuiteViewer'
import { NoteBody } from '../../src/components/suite/NoteBody'
import { publish } from '../../src/lib/connection'

const SHEET = [
  '```sheet name=Stock types="text,int,ugx,ugx,text" id=stock',
  '| item | qty | price | total | note |',
  '|---|---|---|---|---|',
  '| Sugar | 40 | 4200 | =B2*C2 | fine grain |',
  '| Rice | 10 | 5000 | =B3*C3 | |',
  '| Salt | 5 | 1000 | =B4*C4 | |',
  '| Beans | 22 | 3100 | =B5*C5 | long text that makes this column wider than the screen |',
  '```',
  '',
].join('\n')

const SALES = [
  '```sheet name=Sales types="text,text,int"',
  '| month | region | amount |',
  '|---|---|---|',
  '| Jan | North | 10 |',
  '| Jan | South | 20 |',
  '| Feb | North | 30 |',
  '| Feb | South | 5 |',
  '| Mar | North | 25 |',
  '```',
  '',
].join('\n')

const UNITS = [
  '```sheet name=Units types="text,text,int,int"',
  '| month | region | amount | units |',
  '|---|---|---|---|',
  '| Jan | North | 10 | 3 |',
  '| Jan | South | 20 | 5 |',
  '| Feb | North | 30 | 8 |',
  '| Feb | South | 5 | 2 |',
  '| Mar | North | 25 | 6 |',
  '| Mar | South | 18 | 7 |',
  '```',
  '',
].join('\n')

const plot = (attrs: string) => `\`\`\`plot ${attrs}\n\`\`\`\n`

const NOTES: Record<string, string> = {
  sheet: `---\nsuite_kind: sheet\n---\n# Stock\n\n${SHEET}`,
  slides:
    '---\nsuite_kind: slides\n---\n# Board review\n\n## Results\n<!-- notes: Say the number first. -->\n\n- Revenue **up** 12%\n- Uganda strongest\n\n## Plan\n<!-- layout: two-column -->\n<!-- background: #1F3A5F -->\n<!-- color: #FFFFFF -->\n\n1. Hire\n2. Train\n\n## Risks\n\nStock is low.\n',
  dashboard: `---\nsuite_kind: dashboard\n---\n# Sales\n\n${SALES}\n\`\`\`tile from=C2:C7 fn=sum label="Total sales"\n\`\`\`\n\n\`\`\`tile from=A1:C7 fn=sum where="region=North" label="North"\n\`\`\`\n\n${plot('type=bar from=A1:C7 x=month y=amount group=region title="By month and region"')}\n${plot('type=donut from=A1:C7 x=region y=amount title="Share"')}`,
  charts: `---\nsuite_kind: dashboard\n---\n# Chart kinds\n\n${UNITS}\n${plot('type=area from=A1:D7 x=month y=amount group=region title="Area by region"')}\n${plot('type=stacked from=A1:D7 x=month y=amount group=region title="Stacked by region"')}\n${plot('type=scatter from=A1:D7 x=units y=amount group=region title="Amount against units"')}\n${plot('type=radar from=A1:D7 x=month y=amount title="A kind not drawn"')}`,
  doc: '---\nsuite_kind: doc\n---\n# Field notes\n\nA first paragraph that is long enough to wrap on a narrow screen, so the page proves it does not scroll sideways.\n\n## Findings\n\n| item | value |\n|---|---|\n| One | 1 |\n\n## Next steps\n\n- Call the supplier\n- Count the stock\n',
}

const q = new URLSearchParams(location.search)
const view = q.get('view') ?? 'sheet'
if (q.get('theme') === 'light') document.documentElement.setAttribute('data-theme', 'light')
try {
  if (q.get('density')) localStorage.setItem('hq.suite.density', q.get('density') as string)
} catch {
  // A browser without storage uses the default density.
}
const PATH = 'Notebooks/Example.md'

// ---- the pretend HQ: one note in memory, answering the calls the suite views make ----
const fnv = (s: string) => {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i += 1) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193) >>> 0
  return h.toString(16).padStart(8, '0')
}
const hashOf = (src: string) => `${fnv(src)}${fnv(`${src}x`)}`.slice(0, 12)
const FRONT = /^---\n[\s\S]*?\n---\n/

interface Blk {
  hash: string
  kind: string
  src: string
}
/** The note's blocks as hq lists them: a heading, a paragraph or a fence each, without the frontmatter. */
function blocksOf(text: string): Blk[] {
  const out: Blk[] = []
  let cur: string[] = []
  let fenced = false
  const flush = () => {
    if (cur.length === 0) return
    const src = cur.join('\n')
    const info = /^```(\w+)/.exec(src)
    const head = /^(#+)\s/.exec(src)
    out.push({ hash: hashOf(src), kind: info ? info[1] : head ? `heading ${head[1].length}` : 'paragraph', src })
    cur = []
  }
  for (const line of text.replace(FRONT, '').split('\n')) {
    if (fenced) {
      cur.push(line)
      if (line.trim() === '```') {
        fenced = false
        flush()
      }
    } else if (line.startsWith('```')) {
      flush()
      fenced = true
      cur.push(line)
    } else if (line.trim() === '') flush()
    else {
      const heading = /^#+\s/.test(line)
      if (heading) flush()
      cur.push(line)
      if (heading) flush()
    }
  }
  flush()
  return out
}
const joined = (blocks: Blk[]) => `${server.match(FRONT)?.[0] ?? ''}${blocks.map((b) => b.src).join('\n\n')}\n`

const comment = (on: string, text: string, by: string, at: string, state = 'open') => `\`\`\`comment on=${on}${by ? ` by="${by}"` : ''} at=${at} state=${state}\n${text}\n\`\`\``
/** Gives the doc and slides notes a few comments, each written after the block it is about, anchored by the hashes the pretend HQ lists. */
function withComments(text: string, a: Blk, b: Blk): string {
  const out: string[] = []
  for (const blk of blocksOf(text)) {
    out.push(blk.src)
    if (blk.hash === a.hash) out.push(comment(a.hash, 'Is this still true after the audit?', 'Sam', '2026-11-19'), comment(a.hash, 'Checked, it holds.', 'Alex', '2026-11-20', 'resolved'))
    if (blk.hash === b.hash) out.push(comment(b.hash, 'Add a source here.', '', '2026-11-20'))
  }
  return `${text.match(FRONT)?.[0] ?? ''}${out.join('\n\n')}\n`
}

const seed = NOTES[view] ?? NOTES.sheet
let server = seed
if (view === 'doc') {
  const b = blocksOf(seed)
  server = withComments(seed, b[1], b[3])
} else if (view === 'slides') {
  const b = blocksOf(seed)
  server = withComments(seed, b.find((x) => x.src.startsWith('## Plan')) ?? b[1], b[2])
}
let saves = 0

interface Row {
  index: number
  hash: string
  on: string
  by: string | null
  at: string | null
  state: string
  text: string
  orphaned: boolean
  anchor_preview: string | null
}
function commentRows(): { revision: string; comments: Row[] } {
  const blocks = blocksOf(server)
  const comments: Row[] = []
  blocks.forEach((b, index) => {
    if (b.kind !== 'comment') return
    const first = b.src.split('\n')[0]
    const attr = (k: string) => new RegExp(`${k}=(?:"([^"]*)"|(\\S+))`).exec(first)
    const on = attr('on')?.[2] ?? ''
    const by = attr('by')
    const anchor = blocks.find((x) => x.hash === on && x.kind !== 'comment')
    comments.push({
      index,
      hash: b.hash,
      on,
      by: by ? by[1] || null : null,
      at: attr('at')?.[2] ?? null,
      state: attr('state')?.[2] ?? 'open',
      text: b.src.split('\n').slice(1, -1).join('\n'),
      orphaned: !anchor,
      anchor_preview: anchor ? anchor.src.split('\n')[0].slice(0, 80) : null,
    })
  })
  return { revision: fnv(server), comments }
}

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
window.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = String(input)
  const body = init?.body ? JSON.parse(String(init.body)) : {}
  if (url.startsWith('/api/suite/blocks')) return json({ revision: fnv(server), blocks: blocksOf(server).map((b, index) => ({ index, ...b })) })
  if (url === '/api/note' && init?.method === 'PUT') {
    server = body.content
    saves += 1
    return json({ hash: `h${saves}` })
  }
  if (url === '/api/suite/edit' && body.action === 'comments') return json(commentRows())
  if (url === '/api/suite/edit' && body.action === 'add_comment') {
    const blocks = blocksOf(server)
    const at = blocks.findIndex((b) => b.hash === body.on && b.kind !== 'comment')
    if (at < 0) return json({ error: 'that block changed since it was read, so the comment was not added' }, 422)
    let last = at
    while (blocks[last + 1]?.kind === 'comment') last += 1
    blocks.splice(last + 1, 0, { hash: '', kind: 'comment', src: comment(body.on, body.text, body.by ?? 'HQ', '2026-11-20') })
    server = joined(blocks)
    saves += 1
    return json({ summary: 'Added a comment', changed: true, ...commentRows() })
  }
  if (url === '/api/suite/edit' && body.action === 'resolve_comment') {
    const row = commentRows().comments.find((c) => c.index === body.index)
    if (!row || row.hash !== body.hash) return json({ error: 'that comment changed since it was read' }, 422)
    const blocks = blocksOf(server)
    blocks[body.index].src = blocks[body.index].src.replace('state=open', 'state=resolved')
    server = joined(blocks)
    saves += 1
    return json({ summary: `Resolved comment ${body.index}`, changed: true, ...commentRows() })
  }
  return json({ error: 'the pretend HQ does not answer that' }, 404)
}) as typeof fetch
publish('connected')
// The check reads the note back to see what a drag or a size change wrote.
Object.assign(window, { __note: () => server })

function Page() {
  const [text, setText] = useState(server)
  const [hash, setHash] = useState('h0')
  const reload = useCallback(() => {
    setText(server)
    setHash(`r${saves}`)
  }, [])
  return (
    <div className="a-app" style={{ padding: 16, minHeight: '100dvh', overflow: 'auto' }}>
      {view === 'doc' ? <NoteBody content={text} activePath={PATH} hash={hash} onSaved={reload} /> : <SuiteViewer text={text} path={PATH} hash={hash} onSaved={reload} />}
    </div>
  )
}
createRoot(document.getElementById('root') as HTMLElement).render(<Page />)
