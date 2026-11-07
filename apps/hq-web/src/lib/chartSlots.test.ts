import { expect, test } from 'bun:test'
import { chartSlot, fillChartSlots } from './chartSlots'

test('a held chart replaces its own marker once', () => {
  const marker = chartSlot('<figure>chart</figure>')
  expect(fillChartSlots(`<p>a</p>${marker}`)).toBe('<p>a</p><figure>chart</figure>')
  expect(fillChartSlots(marker)).toBe(marker)
})

test('a marker written into the markdown never picks up a chart', () => {
  const real = chartSlot('<figure>real</figure>')
  const forged = ['0', '1', '00000000000000000000000000000000'].map((k) => `<div data-chart-slot="${k}"></div>`).join('')
  expect(fillChartSlots(forged)).toBe(forged)
  expect(fillChartSlots(real)).toBe('<figure>real</figure>')
})
