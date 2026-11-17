import { describe, expect, test } from 'bun:test'
import { plainPreview, plainText, replyTitle } from './replyText'

describe('plain text from markdown', () => {
  test('drops bold, code ticks and heading marks', () => {
    expect(plainText('## Plan\n\n**Next:** run `cargo test` first')).toBe('Plan Next: run cargo test first')
  })

  test('keeps a link as its words and a picture as its alt text', () => {
    expect(plainText('See [the docs](https://example.com/a) and ![a chart](x.png)')).toBe('See the docs and a chart')
  })

  test('reads a preview that was cut off in the middle of markup', () => {
    expect(plainText('**Drive switched off for session kan')).toBe('Drive switched off for session kan')
    expect(plainText('Created **[Host Fleet and Worksp')).toBe('Created Host Fleet and Worksp')
    expect(plainText('Saved the plan at **`Notebooks/P')).toBe('Saved the plan at Notebooks/P')
  })

  test('drops code blocks, even one that is cut off', () => {
    expect(plainText('Run this:\n```bash\nrm -rf x\n```\nThen stop.')).toBe('Run this: Then stop.')
    expect(plainText('Run this:\n```bash\ncargo te')).toBe('Run this:')
  })

  test('reads lists and quotes without their markers', () => {
    expect(plainText('- one\n- two\n1. three\n> quoted')).toBe('one two three quoted')
  })

  test('leaves snake_case words alone and reads _emphasis_', () => {
    expect(plainText('set main_branch to _this_ value')).toBe('set main_branch to this value')
  })

  test('is empty for nothing', () => {
    expect(plainText(null)).toBe('')
    expect(plainText(undefined)).toBe('')
    expect(plainText('')).toBe('')
  })
})

describe('the one-line preview', () => {
  test('is the plain words on one line', () => {
    expect(plainPreview('Line one\n\nLine **two**')).toBe('Line one Line two')
  })

  test('cuts a long line on a word and says so', () => {
    const long = 'word '.repeat(60)
    const out = plainPreview(long, 40)
    expect(out.endsWith('…')).toBe(true)
    expect(out.length).toBeLessThanOrEqual(41)
    expect(out).not.toContain('wor…')
  })
})

describe('a title for a note saved from a reply', () => {
  test('is the first heading', () => {
    expect(replyTitle('Some intro\n\n## Samsung battery steps\n\nOpen Settings.')).toBe('Samsung battery steps')
  })

  test('is the first sentence when there is no heading', () => {
    expect(replyTitle('I wrote the steps down. There are four, starting with Settings.')).toBe('I wrote the steps down')
  })

  test('is cut to a length a title can be', () => {
    const t = replyTitle('This is a very long opening sentence that keeps going well past what any sensible note title would hold')
    expect(t.length).toBeLessThanOrEqual(61)
    expect(t.endsWith('…')).toBe(true)
  })

  test('is empty when the reply has no words to name it by', () => {
    expect(replyTitle('')).toBe('')
    expect(replyTitle('```\ncode only\n```')).toBe('')
  })
})
