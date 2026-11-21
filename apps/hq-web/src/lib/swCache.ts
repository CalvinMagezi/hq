// Which API reads the service worker keeps a copy of for offline use. Only GETs ever reach the worker's
// cache (the fetch handler returns early for any other method), so a note write, a suite edit or a block
// edit is never stored. Kept apart from sw.ts so a test can read it without a worker.
export const SW_CACHED_API: RegExp[] = [
  /^\/api\/notifications$/,
  /^\/api\/vault-asset$/,
  /^\/api\/search$/,
  /^\/api\/vault\/folders$/,
  /^\/api\/tasks\/[^/]+\/comments$/,
  /^\/api\/threads\/[^/]+\/messages$/,
  // A note and the suite's reads of it, so a sheet, deck or doc opens again with no connection.
  /^\/api\/note$/,
  /^\/api\/suite\/(sheet|plot|tile|blocks|media|preview)$/,
]

export const isCachedApi = (pathname: string): boolean => SW_CACHED_API.some((re) => re.test(pathname))
