// The order of the sections in the left rail. The choice lives in localStorage (key
// "rain.rail-order"), like the theme and the zoom: it belongs to this window.
//
// Only the "top" places can be moved. Their keys follow the order (Ctrl+1 is the icon at the
// top), and the places at the bottom (Game mode, Settings) keep their place and their keys.
// The arithmetic is in rail-order.ts.
import { computed, readonly, ref } from 'vue'
import { moveBy, moveToSlot, reconcileOrder, sameOrder, sectionCombo } from './rail-order'
import { routeByName, routes, type RouteDef, type RouteName } from './routes'

export const RAIL_ORDER_KEY = 'rain.rail-order'

/** The movable places in their default order. */
const defaultOrder = routes.filter((def) => def.rail === 'top').map((def) => def.name)

function readStored(): RouteName[] {
  try {
    const raw = globalThis.localStorage?.getItem(RAIL_ORDER_KEY)
    return reconcileOrder(raw ? JSON.parse(raw) : null, defaultOrder)
  } catch {
    return [...defaultOrder]
  }
}

function writeStored(order: readonly RouteName[]) {
  try {
    if (sameOrder(order, defaultOrder)) globalThis.localStorage?.removeItem(RAIL_ORDER_KEY)
    else globalThis.localStorage?.setItem(RAIL_ORDER_KEY, JSON.stringify(order))
  } catch {
    // Storage may be blocked: the order still holds until the window closes.
  }
}

const current = ref<RouteName[]>(readStored())

/** The movable places, top to bottom. */
export const railOrder = readonly(current)

/** The definitions of the movable places, in order. */
export const railTop = computed<RouteDef[]>(() => current.value.map((name) => routeByName[name]))

/** True when the order is the one the app ships with. */
export const isDefaultOrder = computed(() => sameOrder(current.value, defaultOrder))

function set(order: RouteName[]) {
  if (sameOrder(order, current.value)) return
  current.value = order
  writeStored(order)
}

/** Moves a place one step up (-1) or down (+1). */
export function moveSection(name: RouteName, step: -1 | 1) {
  set(moveBy(current.value, name, step))
}

/** Drops a place into a gap of the rail: 0 is above the first icon, the length is below the last. */
export function moveSectionToSlot(name: RouteName, slot: number) {
  set(moveToSlot(current.value, name, slot))
}

export function resetRailOrder() {
  set([...defaultOrder])
}

/**
 * The key that goes to a place now: by its position for the movable places, its own for
 * the places at the bottom. Undefined when there are more than nine places and this is one of them.
 */
export function routeCombo(def: RouteDef): string | undefined {
  if (def.rail !== 'top') return def.combo
  return sectionCombo(current.value.indexOf(def.name))
}

/** The position of a place in the rail, to sort places the way the rail shows them (bottom places last). */
export function railRank(name: RouteName): number {
  const index = current.value.indexOf(name)
  return index >= 0 ? index : current.value.length + routes.findIndex((def) => def.name === name)
}
