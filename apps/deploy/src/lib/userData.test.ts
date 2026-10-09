import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { CLOUD_INIT_TEMPLATE, pinnedFromEnv, renderUserData } from './userData'

const PINNED = { repo: 'owner/repo', ref: 'a'.repeat(40), sha256: 'b'.repeat(64) }

test('the embedded template is the cloud-init file the shell script uses', () => {
  const file = readFileSync(join(import.meta.dir, '../../../../deploy/hetzner/cloud-init.yaml'), 'utf8')
  expect(CLOUD_INIT_TEMPLATE).toBe(file)
})

test('rendering fills every placeholder and carries no secret-shaped input', () => {
  const out = renderUserData(PINNED, 'hq')
  expect(out).not.toMatch(/@[A-Z0-9]+@/)
  expect(out).toContain(`--ref "${PINNED.ref}"`)
  expect(out).toContain(`${PINNED.sha256}  /root/bootstrap.sh`)
  expect(out).toContain('--hostname "hq"')
})

test('a hostname that could break out of the shell line is refused', () => {
  expect(() => renderUserData(PINNED, 'hq"; rm -rf /')).toThrow()
})

test('the pin is read from env and malformed pins are refused', () => {
  expect(pinnedFromEnv({ HQ_BOOTSTRAP_REF: PINNED.ref, HQ_BOOTSTRAP_SHA256: PINNED.sha256 })).toEqual({
    repo: 'CalvinMagezi/hq',
    ref: PINNED.ref,
    sha256: PINNED.sha256,
  })
  expect(pinnedFromEnv({ HQ_BOOTSTRAP_REF: 'main', HQ_BOOTSTRAP_SHA256: PINNED.sha256 })).toBeNull()
  expect(pinnedFromEnv({ HQ_BOOTSTRAP_REF: PINNED.ref })).toBeNull()
  expect(pinnedFromEnv({ HQ_BOOTSTRAP_REPO: 'a b', HQ_BOOTSTRAP_REF: PINNED.ref, HQ_BOOTSTRAP_SHA256: PINNED.sha256 })).toBeNull()
})
