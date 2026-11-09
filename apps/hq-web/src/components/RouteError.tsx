import { useEffect, useState } from 'react'
import { ErrorComponent, type ErrorComponentProps } from '@tanstack/react-router'
import { isStaleBuildError, tryNewBuildReload } from '~/lib/staleBuild'

/** A screen that failed to load. If it only failed because an update replaced its code, reload. */
export function RouteError({ error }: ErrorComponentProps) {
  const stale = isStaleBuildError(error)
  const [gaveUp, setGaveUp] = useState(false)
  useEffect(() => {
    if (stale && !tryNewBuildReload()) setGaveUp(true)
  }, [stale])
  if (stale && !gaveUp) return null
  return <ErrorComponent error={error} />
}
