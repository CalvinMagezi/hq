import { describe, expect, test } from 'bun:test'
import { layerIsOnTop } from './backLayer'

describe('layerIsOnTop', () => {
  test('is true only for the entry that layer pushed', () => {
    expect(layerIsOnTop({ __hqLayer: 'layer-1', index: 3 }, 'layer-1')).toBe(true)
    expect(layerIsOnTop({ __hqLayer: 'layer-0' }, 'layer-1')).toBe(false)
    expect(layerIsOnTop({ index: 3 }, 'layer-1')).toBe(false)
    expect(layerIsOnTop(null, 'layer-1')).toBe(false)
    expect(layerIsOnTop('layer-1', 'layer-1')).toBe(false)
  })
})
