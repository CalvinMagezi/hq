import { expect, test } from 'bun:test'
import { DISCORD_BOT_PERMISSIONS, discordInviteUrl, formatCountdown, secondsLeft } from './chatAppsApi'

test('the Discord permission integer is exactly the bits the relay needs', () => {
  const bits = [10n, 11n, 14n, 16n, 6n, 35n, 38n]
  const sum = bits.reduce((acc, b) => acc | (1n << b), 0n)
  expect(DISCORD_BOT_PERMISSIONS).toBe(sum.toString())
})

test('the invite names the application, both scopes and the permissions', () => {
  const url = new URL(discordInviteUrl('1234567890'))
  expect(url.origin + url.pathname).toBe('https://discord.com/oauth2/authorize')
  expect(url.searchParams.get('client_id')).toBe('1234567890')
  expect(url.searchParams.get('scope')).toBe('bot applications.commands')
  expect(url.searchParams.get('permissions')).toBe(DISCORD_BOT_PERMISSIONS)
  expect(url.search).toContain('scope=bot%20applications.commands')
})

test('the countdown never goes negative and pads seconds', () => {
  expect(secondsLeft(1_000, 1_000_000)).toBe(0)
  expect(secondsLeft(1_000, 100_000)).toBe(900)
  expect(formatCountdown(900)).toBe('15:00')
  expect(formatCountdown(65)).toBe('1:05')
  expect(formatCountdown(0)).toBe('0:00')
})
