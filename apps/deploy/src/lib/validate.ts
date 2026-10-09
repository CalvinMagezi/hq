const DNS_LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/
const HETZNER_TOKEN = /^[A-Za-z0-9]{32,128}$/
const IPV4_CIDR = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\/(\d{1,2})$/
const IPV6_CIDR = /^[0-9a-fA-F:]{2,39}\/(\d{1,3})$/
const SSH_PUBLIC_KEY =
  /^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(?:256|384|521)|sk-ssh-ed25519@openssh\.com|sk-ecdsa-sha2-nistp256@openssh\.com) [A-Za-z0-9+/]+={0,3}( [^\r\n]{1,100})?$/
const MAX_KEY_NAME = 64
const IPV4_MAX_PREFIX = 32
const IPV6_MAX_PREFIX = 128
// Wide enough for an office or ISP range, narrow enough that SSH is never opened to a large share of the internet.
const IPV4_MIN_PREFIX = 16
const IPV6_MIN_PREFIX = 48
const OCTET_MAX = 255

export function isHetznerToken(value: unknown): value is string {
  return typeof value === 'string' && HETZNER_TOKEN.test(value)
}

export function isServerName(value: unknown): value is string {
  return typeof value === 'string' && DNS_LABEL.test(value)
}

/** A single admin address or range for SSH. Ranges wider than /16 (IPv4) or /48 (IPv6) are refused. */
export function isAdminCidr(value: unknown): value is string {
  if (typeof value !== 'string') return false
  const v4 = IPV4_CIDR.exec(value)
  if (v4) {
    const octets = v4.slice(1, 5).map(Number)
    const prefix = Number(v4[5])
    return octets.every((o) => o <= OCTET_MAX) && prefix >= IPV4_MIN_PREFIX && prefix <= IPV4_MAX_PREFIX
  }
  const v6 = IPV6_CIDR.exec(value)
  if (!v6 || !value.includes(':')) return false
  const prefix = Number(v6[1])
  return prefix >= IPV6_MIN_PREFIX && prefix <= IPV6_MAX_PREFIX
}

/** One-line OpenSSH public key. Private keys and anything multi-line are refused. */
export function isSshPublicKey(value: unknown): value is string {
  return typeof value === 'string' && SSH_PUBLIC_KEY.test(value.trim())
}

export function isKeyName(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= MAX_KEY_NAME && !/[\r\n]/.test(value)
}

export function isId(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
}
