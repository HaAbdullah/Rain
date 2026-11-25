import { describe, expect, it } from 'vitest'
import { pathOf, routeByName, routeOfPath, routes } from './routes'

describe('the Hardware route', () => {
  it('takes the place of Disk in the rail, on the same key', () => {
    const names = routes.filter((def) => def.rail === 'top').map((def) => def.name)
    expect(names).toEqual(['home', 'apps', 'projects', 'games', 'workflows', 'life', 'hardware', 'claude'])
    expect(routeByName.hardware.combo).toBe('Ctrl+Digit7')
    expect(routeByName.hardware.icon).toBe('cpu')
    expect(routes.some((def) => def.path.startsWith('/disk'))).toBe(false)
  })

  it('shows the tile grid by default and does not remember the Disk screen', () => {
    const def = routeByName.hardware
    expect(def.subs).toEqual(['devices', 'disk'])
    expect(def.defaultSub).toBe('devices')
    expect(def.rememberSub).toBe(false)
  })

  it('has a path for each sub-view, and reads it back', () => {
    expect(pathOf({ name: 'hardware', sub: 'disk' })).toBe('/hardware/disk')
    expect(pathOf({ name: 'hardware' })).toBe('/hardware/devices')
    expect(routeOfPath('/hardware/disk')).toEqual({ name: 'hardware', sub: 'disk' })
    expect(routeOfPath('/hardware/devices')).toEqual({ name: 'hardware', sub: 'devices' })
    // An unknown sub-view and the old address of the Disk screen are not errors.
    expect(routeOfPath('/hardware/nonsense')).toEqual({ name: 'hardware', sub: 'devices' })
    expect(routeOfPath('/disk')).toEqual({ name: 'home' })
  })

  it('is the only route that forgets its sub-view', () => {
    const forgetting = routes.filter((def) => def.rememberSub === false).map((def) => def.name)
    expect(forgetting).toEqual(['hardware'])
  })
})
