// Navigation without a router: one reactive `route`, changed by `go()`.
// In the browser the route is mirrored to location.hash so a refresh keeps the screen.
import { computed, ref, watch, type ComputedRef, type Ref } from 'vue'
import { useSearch } from '@/composables/useSearch'
import { shell } from '@/shell'
import { pathOf, routeByName, routeOfPath, type Route, type RouteName } from './routes'

/** How many earlier screens `back()` can return through. */
const TRAIL_LIMIT = 20

const search = useSearch()
const mirrorsHash = shell.kind === 'browser'

const route = ref<Route>(mirrorsHash ? routeOfPath(location.hash.slice(1)) : { name: 'home' })
const trail: Route[] = []
/** The screens `stepBack()` left, newest last: what `stepForward()` returns to. */
const ahead: Route[] = []
/** The sub-view each route showed last, so Ctrl+5 returns to the tab you left. */
const lastSub: Partial<Record<RouteName, string>> = {}

// A refresh on a section's hash is the same as going there: the chip is shown.
if (routeByName[route.value.name].filterable) search.setScope(route.value.name)

const displayed = computed<RouteName>(() =>
  search.scope.value === null && search.query.value !== '' ? 'home' : route.value.name,
)

function resolveSub(name: RouteName, sub: string | undefined): string | undefined {
  const def = routeByName[name]
  if (!def.subs) return undefined
  if (sub && def.subs.includes(sub)) return sub
  return lastSub[name] ?? def.defaultSub
}

function show(next: Route) {
  if (next.sub) lastSub[next.name] = next.sub
  route.value = next
  search.setScope(routeByName[next.name].filterable ? next.name : null)
  search.clear()
}

function go(name: RouteName, sub?: string, params?: Record<string, string>) {
  if (name !== route.value.name) {
    trail.push(route.value)
    if (trail.length > TRAIL_LIMIT) trail.shift()
    // A new screen starts a new branch, as in a browser: there is nothing ahead of it.
    ahead.length = 0
  }
  show({ name, sub: resolveSub(name, sub), params })
}

function back() {
  ahead.length = 0
  show(trail.pop() ?? { name: 'home' })
}

function stepBack(): boolean {
  const previous = trail.pop()
  if (!previous) return false
  ahead.push(route.value)
  show(previous)
  return true
}

function stepForward(): boolean {
  const next = ahead.pop()
  if (!next) return false
  trail.push(route.value)
  show(next)
  return true
}

function setSub(sub: string) {
  const { name } = route.value
  if (!routeByName[name].subs?.includes(sub)) return
  lastSub[name] = sub
  route.value = { name, sub }
}

if (mirrorsHash) {
  watch(
    route,
    (current) => history.replaceState(null, '', `#${pathOf(current)}`),
    { immediate: true },
  )
}

export interface Nav {
  route: Readonly<Ref<Route>>
  /** What the rail and the title show: 'home' while global results cover the screen. */
  displayed: ComputedRef<RouteName>
  /**
   * Goes to a screen. Adds the scope chip when the route is filterable and clears the query.
   * `params.id` names the item the screen should select ("Show in Workflows").
   */
  go(name: RouteName, sub?: string, params?: Record<string, string>): void
  /** Returns to the screen before this one (Home when there is none). */
  back(): void
  /** One screen back in the history. Returns false at its start (nothing happens). */
  stepBack(): boolean
  /** One screen forward again after `stepBack()`. Returns false when there is none. */
  stepForward(): boolean
  /** Changes only the sub-view (Tab, a ViewStrip click). The query and the chip stay. */
  setSub(sub: string): void
}

const nav: Nav = { route, displayed, go, back, stepBack, stepForward, setSub }

export function useNav(): Nav {
  return nav
}
