import { expect, test } from 'bun:test'
import { isAdminCidr, isHetznerToken, isServerName, isSshPublicKey } from './validate'

test('server names are DNS labels', () => {
  for (const ok of ['hq', 'hq-1', 'a'.repeat(63)]) expect(isServerName(ok)).toBe(true)
  for (const bad of ['', 'Hq', '-hq', 'hq-', 'a'.repeat(64), 'hq.example', 'hq"x']) expect(isServerName(bad)).toBe(false)
})

test('admin ranges cannot open SSH to the internet', () => {
  for (const ok of ['203.0.113.7/32', '10.0.0.0/16', '2001:db8::1/128', '2001:db8::/48']) expect(isAdminCidr(ok)).toBe(true)
  for (const bad of ['0.0.0.0/0', '0.0.0.0/1', '128.0.0.0/1', '10.0.0.0/8', '::/0', '::/1', '8000::/1', '203.0.113.7', '300.1.1.1/32', '1.2.3.4/33', 'abc/32']) expect(isAdminCidr(bad)).toBe(false)
})

test('only one-line public keys are accepted', () => {
  expect(isSshPublicKey('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc user@host')).toBe(true)
  expect(isSshPublicKey('-----BEGIN OPENSSH PRIVATE KEY-----\nabc')).toBe(false)
  expect(isSshPublicKey('ssh-ed25519 AAAA\nssh-rsa BBBB')).toBe(false)
})

test('tokens are plain alphanumerics of a plausible length', () => {
  expect(isHetznerToken('a'.repeat(64))).toBe(true)
  expect(isHetznerToken('short')).toBe(false)
  expect(isHetznerToken('a'.repeat(63) + '\n')).toBe(false)
})
