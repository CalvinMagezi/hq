import { expect, test } from 'bun:test'
import { codespanHtml, noteRefFromCode, vaultLinkTarget } from './noteRefs'

test('noteRefFromCode recognises a backticked vault path', () => {
  expect(
    noteRefFromCode('Notebooks/Projects/Agent-HQ/Markdown-Canonical-Vault-Bidirectional-Conversion-Pipeline.md'),
  ).toBe('Notebooks/Projects/Agent-HQ/Markdown-Canonical-Vault-Bidirectional-Conversion-Pipeline.md')
  expect(noteRefFromCode('Notebooks/Projects/My Project/Plan.md')).toBe('Notebooks/Projects/My Project/Plan.md')
  expect(noteRefFromCode('Notebooks/Inbox/PLAN.MD')).toBe('Notebooks/Inbox/PLAN.MD')
})

test('noteRefFromCode leaves ordinary inline code alone', () => {
  expect(noteRefFromCode('README.md')).toBeNull()
  expect(noteRefFromCode('cargo test --workspace')).toBeNull()
  expect(noteRefFromCode('crates/hq-web/src/lib.rs')).toBeNull()
  expect(noteRefFromCode('Notebooks/Inbox/Plan')).toBeNull()
  expect(noteRefFromCode('cat Notebooks/Inbox/Plan.md > out.md')).toBeNull()
  expect(noteRefFromCode('a/b.md\nc/d.md')).toBeNull()
  expect(noteRefFromCode(' Notebooks/Inbox/Plan.md')).toBeNull()
  expect(noteRefFromCode('')).toBeNull()
  expect(noteRefFromCode(null)).toBeNull()
})

test('noteRefFromCode refuses external and unsafe references', () => {
  expect(noteRefFromCode('https://example.com/notes/plan.md')).toBeNull()
  expect(noteRefFromCode('../secrets/plan.md')).toBeNull()
  expect(noteRefFromCode('Notebooks/../../plan.md')).toBeNull()
  expect(noteRefFromCode('/etc/notes/plan.md')).toBeNull()
  expect(noteRefFromCode(`Notebooks/${'x'.repeat(400)}.md`)).toBeNull()
})

test('vaultLinkTarget resolves vault links and relative markdown links', () => {
  expect(vaultLinkTarget('vault:Notebooks/Spec.md')).toBe('Notebooks/Spec.md')
  expect(vaultLinkTarget('note:Notebooks/Spec.md')).toBe('Notebooks/Spec.md')
  expect(vaultLinkTarget('/api/note?path=Notebooks%2FInbox%2FPlan.md')).toBe('Notebooks/Inbox/Plan.md')
  expect(vaultLinkTarget('Notebooks/Inbox/Plan.md')).toBe('Notebooks/Inbox/Plan.md')
})

test('vaultLinkTarget decodes spaces and drops a heading anchor', () => {
  expect(vaultLinkTarget('Notebooks/My%20Project/Plan.md')).toBe('Notebooks/My Project/Plan.md')
  expect(vaultLinkTarget('Notebooks/Inbox/Plan.md#risks')).toBe('Notebooks/Inbox/Plan.md')
  expect(vaultLinkTarget('vault:Notebooks/My%20Project/Plan.md#risks')).toBe('Notebooks/My Project/Plan.md')
  // A stray percent sign must not throw.
  expect(vaultLinkTarget('Notebooks/100%/Plan.md')).toBe('Notebooks/100%/Plan.md')
})

test('vaultLinkTarget ignores ordinary links', () => {
  expect(vaultLinkTarget('https://example.com/readme.md')).toBeNull()
  expect(vaultLinkTarget('https://example.com')).toBeNull()
  expect(vaultLinkTarget('#section')).toBeNull()
  expect(vaultLinkTarget('mailto:someone@example.com')).toBeNull()
  expect(vaultLinkTarget('//cdn.example.com/plan.md')).toBeNull()
  expect(vaultLinkTarget('')).toBeNull()
  expect(vaultLinkTarget(null)).toBeNull()
})

test('codespanHtml makes a vault path tappable and escapes it', () => {
  const html = codespanHtml('Notebooks/Projects/Plan.md')
  expect(html).toContain('class="md-note-ref"')
  expect(html).toContain('data-vault-note="Notebooks/Projects/Plan.md"')
  expect(html).toContain('role="button"')
  expect(html).toContain('tabindex="0"')
  expect(html).toContain('>Notebooks/Projects/Plan.md</code>')
})

test('codespanHtml keeps plain code plain and escapes markup', () => {
  expect(codespanHtml('cargo build')).toBe('<code>cargo build</code>')
  expect(codespanHtml('<script>alert("x")</script>')).toBe(
    '<code>&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;</code>',
  )
  expect(codespanHtml('a & b')).toBe('<code>a &amp; b</code>')
})
