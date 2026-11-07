const IMG_TAG = /<img\b[^>]*>/gi
const REMOTE_SRC = /\ssrc\s*=\s*(?:"\s*((?:https?:)?\/\/[^"]*)"|'\s*((?:https?:)?\/\/[^']*)')/i
const ALT = /\salt\s*=\s*(?:"([^"]*)"|'([^']*)')/i

/**
 * The alt text goes back in as text, after the sanitizer has run. A serialized attribute keeps
 * `<` and `>` as they are, so they are escaped here, or an alt of `<img src=x onerror=...>`
 * would become a live tag. `&` is already escaped by the serializer, so its entities stay.
 */
const asText = (s: string) => s.replace(/</g, '&lt;').replace(/>/g, '&gt;')

/**
 * The CSP blocks remote images, because an image URL is a way to send data out
 * (a prompt-injected note can put secrets in the query string). Replace the
 * dead tag with visible text. Expects already-sanitized HTML.
 */
export function blockRemoteImages(html: string): string {
  return html.replace(IMG_TAG, (tag) => {
    if (!REMOTE_SRC.test(tag)) return tag
    const alt = ALT.exec(tag)
    const label = asText((alt?.[1] ?? alt?.[2] ?? '').trim())
    const text = label ? `[remote image blocked: ${label}]` : '[remote image blocked]'
    return `<span class="md-image-blocked" title="Remote images are blocked by the content security policy">${text}</span>`
  })
}
