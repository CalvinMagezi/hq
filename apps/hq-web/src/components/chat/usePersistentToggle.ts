import { useCallback, useState } from 'react'

// A streaming turn remounts as a saved message when it ends; this keeps an open panel open across that.
const remembered = new Map<string, boolean>()

/** useState for an open/closed panel whose value outlives the component, keyed by `key`. */
export function usePersistentToggle(key: string, initial: boolean): [boolean, (open: boolean) => void] {
  const [open, setOpen] = useState(() => remembered.get(key) ?? initial)
  const set = useCallback(
    (next: boolean) => {
      remembered.set(key, next)
      setOpen(next)
    },
    [key],
  )
  return [open, set]
}
