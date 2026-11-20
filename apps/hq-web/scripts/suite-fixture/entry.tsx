// A static page for scripts/suite-mobile-check.cjs: the suite views with invented notes and no HQ behind
// them. `?view=sheet|slides|dashboard|doc` picks one, `?theme=light` the light tokens, `?density=compact` the grid density.
// It is bundled by the check itself and is not part of the app.
import { createRoot } from 'react-dom/client'
import '../../src/styles/aligned-ui.css'
import { SuiteViewer } from '../../src/components/suite/SuiteViewer'
import { NoteBody } from '../../src/components/suite/NoteBody'

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

const NOTES: Record<string, string> = {
  sheet: `---\nsuite_kind: sheet\n---\n# Stock\n\n${SHEET}`,
  slides:
    '---\nsuite_kind: slides\n---\n# Board review\n\n## Results\n<!-- notes: Say the number first. -->\n\n- Revenue **up** 12%\n- Uganda strongest\n\n## Plan\n<!-- layout: two-column -->\n<!-- background: #1F3A5F -->\n<!-- color: #FFFFFF -->\n\n1. Hire\n2. Train\n\n## Risks\n\nStock is low.\n',
  dashboard: `---\nsuite_kind: dashboard\n---\n# Sales\n\n${SALES}\n\`\`\`tile from=C2:C7 fn=sum label="Total sales"\n\`\`\`\n\n\`\`\`tile from=A1:C7 fn=sum where="region=North" label="North"\n\`\`\`\n\n\`\`\`plot type=bar from=A1:C7 x=month y=amount group=region title="By month and region"\n\`\`\`\n\n\`\`\`plot type=donut from=A1:C7 x=region y=amount title="Share"\n\`\`\`\n`,
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
createRoot(document.getElementById('root') as HTMLElement).render(
  <div className="a-app" style={{ padding: 16, minHeight: '100dvh', overflow: 'auto' }}>
    {view === 'doc' ? <NoteBody content={NOTES.doc} activePath={PATH} hash="h" /> : <SuiteViewer text={NOTES[view]} path={PATH} hash="h" />}
  </div>,
)
