import { expect, test } from 'bun:test'
import { renderToString } from 'react-dom/server'
import { SessionTerminal } from './SessionTerminal'

test('the terminal shows a loading skeleton, not an empty pane, before the first response', () => {
  const html = renderToString(<SessionTerminal sessionId="hs-1" refreshKey={0} active />)
  expect(html).toContain('data-testid="terminal-skeleton"')
  expect(html).not.toContain('No output yet.')
})

test('an inactive terminal has nothing to load and shows no skeleton', () => {
  const html = renderToString(<SessionTerminal sessionId="hs-1" refreshKey={0} active={false} />)
  expect(html).not.toContain('terminal-skeleton')
})
