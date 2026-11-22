<script setup lang="ts">
// The frame (spec 3.1): rail, header, the active screen and the overlays.
// It also starts the keymap and pauses the mock's timers while the window is hidden.
import { computed, defineAsyncComponent, watch, type Component } from 'vue'
import { TooltipProvider } from '@/components/ui/tooltip'
import { isOverlayOpen } from '@/composables/useOverlay'
import { useSearch } from '@/composables/useSearch'
import { mockControl } from '@/data/mock/control'
import { useKeymap } from '@/keys/useKeymap'
import { routeByName, type RouteName } from '@/nav/routes'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'
import ActionPanel from './ActionPanel.vue'
import AppHeader from './AppHeader.vue'
import CheatSheet from './CheatSheet.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import HiddenPlaceholder from './HiddenPlaceholder.vue'
import IconRail from './IconRail.vue'
import LogDialog from './LogDialog.vue'
import NoticePill from './NoticePill.vue'

useKeymap()

const nav = useNav()
const search = useSearch()
const { hidden } = shell

// One async wrapper per screen, so coming back to a screen does not wrap its loader again.
const screens = new Map<RouteName, Component>()
function screenFor(name: RouteName): Component {
  let screen = screens.get(name)
  if (!screen) {
    screen = defineAsyncComponent(routeByName[name].component)
    screens.set(name, screen)
  }
  return screen
}
const screen = computed(() => screenFor(nav.displayed.value))

watch(hidden, (isHidden) => mockControl.setPaused(isHidden), { immediate: true })

/** A click on empty space puts the caret back in the search field (spec 3.8). */
function returnFocus(event: MouseEvent) {
  const target = event.target
  if (isOverlayOpen(target)) return
  if (target instanceof Element && target.closest('input, textarea, select, [contenteditable="true"]')) return
  // Leave a text selection alone: it is how a path gets copied.
  if (window.getSelection()?.toString()) return
  search.focus()
}
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <div
      v-show="!hidden"
      class="grid h-full grid-cols-[var(--rail-w)_minmax(0,1fr)] grid-rows-[var(--header-h)_minmax(0,1fr)] bg-background"
      @click="returnFocus"
    >
      <IconRail class="row-span-2" />
      <AppHeader />
      <main class="relative min-h-0 min-w-0 overflow-hidden">
        <component :is="screen" :key="nav.displayed.value" />
        <NoticePill />
      </main>
    </div>
    <HiddenPlaceholder v-if="hidden" />

    <ActionPanel />
    <CheatSheet />
    <LogDialog />
    <ConfirmDialog />
  </TooltipProvider>
</template>
