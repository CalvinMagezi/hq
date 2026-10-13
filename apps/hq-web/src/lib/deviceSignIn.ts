// Signing in another device with a one-time code from one that is already signed in.

import { CLIENT_HEADER, CLIENT_HEADER_VALUE, hqJson } from './hqAuth'

const DEVICE_CODE_URL = '/api/auth/device-code'
const EXCHANGE_URL = '/api/auth/exchange'
/** Letters and digits in a code, without the hyphen shown in the middle. */
export const CODE_LENGTH = 10
const MS_PER_SECOND = 1000
const SECONDS_PER_MINUTE = 60

export interface DeviceCode {
  code: string
  expires_at: number
  expires_in: number
}

/** Ask the server for a fresh code; any earlier one stops working. */
export function requestDeviceCode(): Promise<DeviceCode> {
  return hqJson<DeviceCode>(DEVICE_CODE_URL, 'POST')
}

/** Letters and digits only, uppercased, so a code typed with spaces or in lowercase still matches. */
export function normalizeCode(raw: string): string {
  return raw.replace(/[^a-z0-9]/gi, '').toUpperCase()
}

/**
 * The link the QR code carries: the one-time code, not a token, so a photo of the screen is worth
 * one sign-in for ten minutes and the new device gets a token of its own.
 */
export function codeLink(origin: string, code: string): string {
  return `${origin}/vault#code=${encodeURIComponent(normalizeCode(code))}`
}

/** Seconds left until `deadlineMs`, never below zero. */
export function secondsLeft(deadlineMs: number, nowMs: number): number {
  return Math.max(0, Math.ceil((deadlineMs - nowMs) / MS_PER_SECOND))
}

/** `m:ss` for a countdown. */
export function formatCountdown(seconds: number): string {
  const minutes = Math.floor(seconds / SECONDS_PER_MINUTE)
  const rest = String(seconds % SECONDS_PER_MINUTE).padStart(2, '0')
  return `${minutes}:${rest}`
}

const HTTP_UNAUTHORIZED = 401
const HTTP_FORBIDDEN = 403
const HTTP_NOT_FOUND = 404
const HTTP_TOO_MANY = 429

/** What to tell the person when the server refuses a code. */
export function exchangeErrorMessage(status: number): string {
  switch (status) {
    case HTTP_UNAUTHORIZED:
      return 'That code is wrong, already used or expired. Get a new one on the signed-in device.'
    case HTTP_TOO_MANY:
      return 'Too many tries. Wait a few minutes, then get a new code.'
    case HTTP_NOT_FOUND:
      return 'This server has no sign-in to hand out.'
    case HTTP_FORBIDDEN:
      return 'This page is not allowed to sign in to this server.'
    default:
      return `Could not sign in (error ${status}). Try again.`
  }
}

/**
 * Trade a code for a device token of its own. Plain fetch: there is no token to send yet, and a refused code
 * must not mark the stored token as rejected.
 */
export async function exchangeCode(raw: string): Promise<string> {
  const code = normalizeCode(raw)
  if (code.length !== CODE_LENGTH) throw new Error(`A code has ${CODE_LENGTH} letters and digits.`)
  const res = await fetch(EXCHANGE_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', [CLIENT_HEADER]: CLIENT_HEADER_VALUE },
    body: JSON.stringify({ code }),
  })
  if (!res.ok) throw new Error(exchangeErrorMessage(res.status))
  const data = (await res.json().catch(() => ({}))) as { token?: unknown }
  if (typeof data.token !== 'string' || !data.token) throw new Error('The server answered without a token. Try again.')
  return data.token
}

const DEVICES_URL = '/api/auth/devices'
const ROTATE_URL = '/api/auth/rotate'

export interface SignedInDevice {
  id: string
  label: string
  created_at: number
  last_used_at: number | null
  expires_at: number | null
  current: boolean
}

export interface DeviceList {
  devices: SignedInDevice[]
  operator_link_active: boolean
  this_device_uses_operator_link: boolean
}

export function listDevices(): Promise<DeviceList> {
  return hqJson<DeviceList>(DEVICES_URL)
}

/** Sign one device out; its next request is refused. */
export function signOutDevice(id: string): Promise<void> {
  return hqJson<void>(`${DEVICES_URL}/${encodeURIComponent(id)}`, 'DELETE')
}

/** Sign out every device and the operator's link; returns the one token this device keeps. */
export async function signOutEverywhere(): Promise<string> {
  const data = await hqJson<{ token?: unknown }>(ROTATE_URL, 'POST')
  if (typeof data.token !== 'string' || !data.token) throw new Error('The server answered without a token.')
  return data.token
}

const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR

/** "just now", "5 min ago", "3 h ago" or "2 days ago" for a unix time, from `nowMs`. */
export function lastUsedText(unixSeconds: number | null, nowMs: number): string {
  if (unixSeconds === null) return 'not used yet'
  const ago = Math.max(0, Math.floor(nowMs / MS_PER_SECOND) - unixSeconds)
  if (ago < SECONDS_PER_MINUTE) return 'just now'
  if (ago < SECONDS_PER_HOUR) return `${Math.floor(ago / SECONDS_PER_MINUTE)} min ago`
  if (ago < SECONDS_PER_DAY) return `${Math.floor(ago / SECONDS_PER_HOUR)} h ago`
  const days = Math.floor(ago / SECONDS_PER_DAY)
  return `${days} ${days === 1 ? 'day' : 'days'} ago`
}
