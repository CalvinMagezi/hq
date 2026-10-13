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

/** The link that signs a browser in, the same one the operator hands out. */
export function loginLink(origin: string, token: string): string {
  return `${origin}/vault#token=${encodeURIComponent(token)}`
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
 * Trade a code for the web token. Plain fetch: there is no token to send yet, and a refused code
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
