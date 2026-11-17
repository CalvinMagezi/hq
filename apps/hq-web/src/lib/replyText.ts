// Plain words out of a reply's markdown: for a chat's one-line preview and for the title of a note saved from a reply.

/** Markdown as the plain text a person would read: no markers, links as their words, one line. */
export function plainText(markdown: string | null | undefined): string {
  if (!markdown) return ''
  return markdown
    .replace(/```[\s\S]*?(```|$)/g, ' ') // code blocks, even one that is cut off
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1') // pictures keep their alt text
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1') // links keep their words
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+[.)])\s+/gm, '') // heading, quote and list markers at a line start
    .replace(/(^|\s)_{1,2}([^_\s][^_]*?)_{1,2}(?=\s|$)/g, '$1$2') // _emphasis_ (not snake_case)
    .replace(/[*~`]+/g, '') // bold, strike, code ticks, and any left over from a cut-off message
    .replace(/[[\]]/g, '') // the brackets of a link that was cut off
    .replace(/\|/g, ' ') // table cells
    .replace(/\s+/g, ' ')
    .trim()
}

/** One line for the chat list: the reply's plain words, cut at `max` on a word with an ellipsis. */
export function plainPreview(markdown: string | null | undefined, max = 140): string {
  return cut(plainText(markdown), max)
}

function cut(text: string, max: number): string {
  if (text.length <= max) return text
  const head = text.slice(0, max)
  const space = head.lastIndexOf(' ')
  return `${(space > max / 2 ? head.slice(0, space) : head).trimEnd()}…`
}

/** A title for a note made from a reply: its first heading, else its first sentence. Empty when there is nothing to name it by. */
export function replyTitle(markdown: string | null | undefined, max = 60): string {
  if (!markdown) return ''
  const heading = markdown.match(/^\s{0,3}#{1,6}\s+(.+)$/m)
  const source = heading ? plainText(heading[1]) : plainText(markdown.split(/\n\s*\n/)[0])
  const sentence = source.match(/^.*?[.!?](?=\s|$)/)?.[0] ?? source
  return cut(sentence.replace(/[.!?]+$/, ''), max)
}
