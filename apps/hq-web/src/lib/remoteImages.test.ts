import { expect, test } from 'bun:test'
import { blockRemoteImages } from './remoteImages'

test('remote images become text, local ones stay', () => {
  const html = '<p><img src="https://evil.example/x.png?d=secret" alt="a cat"><img src="/api/vault-asset?path=a.png"><img src=\'//cdn.example/y.png\'><img src="blob:abc"><img src="data:image/png;base64,AAA"></p>'
  const out = blockRemoteImages(html)
  expect(out).not.toContain('evil.example')
  expect(out).not.toContain('cdn.example')
  expect(out).toContain('[remote image blocked: a cat]')
  expect(out).toContain('[remote image blocked]')
  expect(out).toContain('src="/api/vault-asset?path=a.png"')
  expect(out).toContain('src="blob:abc"')
  expect(out).toContain('src="data:image/png;base64,AAA"')
})

test('an alt text that holds markup comes back as text, not as a tag', () => {
  // How a serializer writes the attribute: `<` and `>` kept, `&` and `"` escaped.
  const html = '<p><img src="https://evil.example/x.png" alt="<img src=x onerror=alert(1) &amp; more"></p>'
  const out = blockRemoteImages(html)
  expect(out).not.toContain('<img')
  expect(out).toContain('[remote image blocked: &lt;img src=x onerror=alert(1) &amp; more]')
})
