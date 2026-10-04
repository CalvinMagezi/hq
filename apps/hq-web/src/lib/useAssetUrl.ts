import { useEffect, useState } from 'react'
import { hasWebToken, hqFetch } from './hqAuth'
import { assetUrl } from './vaultApi'

/**
 * A URL an <img> or a download link can use for a vault file. Without a web
 * token that is the plain API URL. With one, elements cannot send the header,
 * so the file is fetched with it and served from a blob URL instead.
 */
export function useAssetUrl(path: string): string | null {
  const direct = !hasWebToken()
  const [blobUrl, setBlobUrl] = useState<string | null>(null)

  useEffect(() => {
    if (direct) return
    let objectUrl: string | null = null
    let active = true
    setBlobUrl(null)
    hqFetch(assetUrl(path))
      .then((res) => (res.ok ? res.blob() : Promise.reject(new Error(`${res.status}`))))
      .then((blob) => {
        if (!active) return
        objectUrl = URL.createObjectURL(blob)
        setBlobUrl(objectUrl)
      })
      .catch((e) => console.error('Failed to load vault asset', path, e))
    return () => {
      active = false
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [path, direct])

  return direct ? assetUrl(path) : blobUrl
}
