const SKELETON_ROWS = [0, 1, 2]

/** Grey rows in the shape of a settings list, no announcement of their own. */
export function SkeletonRows() {
  return (
    <div className="a-set" aria-hidden="true">
      {SKELETON_ROWS.map((n) => (
        <div key={n} className="a-li">
          <div className="a-li-main">
            <span className="a-skel t" />
            <span className="a-skel s" />
          </div>
        </div>
      ))}
    </div>
  )
}

/** The rows shown until the first answer arrives, announced once as `label`. */
export function SettingsSkeleton({ label }: { label: string }) {
  return (
    <div role="status" aria-label={label}>
      <SkeletonRows />
    </div>
  )
}
