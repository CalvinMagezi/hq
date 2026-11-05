// The web's strings, from the one catalogue (apps/strings/en.yaml, decisions.md N19).
import { en } from './en.gen'

type Catalogue = typeof en
export type StringKey = { [K in keyof Catalogue]: Catalogue[K] extends string ? K : never }[keyof Catalogue]
export type PluralKey = Exclude<keyof Catalogue, StringKey>
type Params = Record<string, string | number>

function fill(text: string, params: Params): string {
  return text.replace(/\{([a-z][a-z0-9_]*)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole))
}

/** One whole sentence by key, with its names, numbers and times filled in. */
export function t(key: StringKey, params: Params = {}): string {
  const text = en[key] as string | undefined
  // A key built at run time that the web's share of the catalogue left out: never a blank page.
  if (text === undefined) return key
  return fill(text, params)
}

const rules = new Intl.PluralRules('en')

/** A plural sentence: the form is chosen by `count`, which also fills `{n}`. */
export function tn(key: PluralKey, count: number, params: Params = {}): string {
  const forms = en[key] as Record<string, string> | undefined
  if (forms === undefined) return key
  const form = rules.select(count)
  return fill(forms[form] ?? forms.other, { n: count, ...params })
}
