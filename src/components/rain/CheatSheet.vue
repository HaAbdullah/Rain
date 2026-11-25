<script setup lang="ts">
// The shortcut sheet (F1, Ctrl+/), generated from the key table so it cannot drift from the real bindings.
import { computed } from 'vue'
import { keysOf, orderActions, targetName } from '@/actions/display'
import { lucideIcon } from '@/lib/icon-map'
import { useScreenState } from '@/composables/useScreen'
import { useNav } from '@/nav/useNav'
import { routeByName } from '@/nav/routes'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { useCheatSheet } from '@/composables/useCheatSheet'
import { combosOf, keymap, type KeyBinding, type KeyGroup } from '@/keys/keymap'
import KeyHint from './KeyHint.vue'

const { isOpen, close } = useCheatSheet()

// The selected item's actions come from the same source as the action bar (useScreenState).
const screenState = useScreenState()
const nav = useNav()
const itemActions = computed(() => (isOpen.value ? orderActions(screenState.actions.value) : []))
const itemTitle = computed(() => {
  const target = screenState.target.value
  if (target) return `Selected: ${targetName(target)}`
  return `This screen: ${routeByName[nav.displayed.value].title}`
})

const GROUPS: KeyGroup[] = ['Window', 'Go', 'Move', 'Act', 'Command']
const sections = GROUPS.map((title) => ({
  title,
  bindings: keymap.filter((binding) => binding.group === title),
}))

/** The combos to draw for a binding. A long run (Alt+1 .. Alt+9) shows its two ends. */
function isRange(binding: KeyBinding): boolean {
  return combosOf(binding).length > 3
}

function shownCombos(binding: KeyBinding): string[] {
  const combos = combosOf(binding)
  return isRange(binding) ? [combos[0], combos[combos.length - 1]] : combos
}
</script>

<template>
  <Dialog :open="isOpen" @update:open="(open) => !open && close()">
    <DialogContent class="max-h-[calc(100vh-4rem)] gap-4 overflow-y-auto sm:max-w-[720px]">
      <DialogTitle class="text-base font-semibold">Keyboard shortcuts</DialogTitle>
      <DialogDescription class="sr-only">Every key Rain responds to.</DialogDescription>
      <section v-if="itemActions.length > 0" data-testid="item-actions">
        <h3 class="mb-1 truncate text-xs font-medium text-muted-foreground">{{ itemTitle }}</h3>
        <ul class="grid grid-cols-2 gap-x-8">
          <li
            v-for="action in itemActions"
            :key="action.id"
            class="flex min-h-8 items-center justify-between gap-4 text-13"
            :class="action.danger && 'text-destructive'"
          >
            <span class="flex min-w-0 items-center gap-2">
              <component :is="lucideIcon(action.icon)" class="size-3.5 shrink-0" />
              <span class="truncate">{{ action.label }}</span>
            </span>
            <KeyHint v-if="keysOf(action)" :keys="keysOf(action)!" />
          </li>
        </ul>
      </section>
      <div class="grid grid-cols-2 gap-x-8 gap-y-5">
        <section v-for="section in sections" :key="section.title">
          <h3 class="mb-1 text-xs font-medium text-muted-foreground">{{ section.title }}</h3>
          <ul>
            <li
              v-for="binding in section.bindings"
              :key="binding.id"
              class="flex min-h-8 items-center justify-between gap-4 text-13"
            >
              <span class="truncate">{{ binding.label }}</span>
              <span class="flex shrink-0 items-center gap-1.5">
                <template v-for="(combo, index) in shownCombos(binding)" :key="combo">
                  <span v-if="index > 0" class="text-xs text-text-3">{{ isRange(binding) ? 'to' : 'or' }}</span>
                  <KeyHint :keys="combo" />
                </template>
              </span>
            </li>
          </ul>
        </section>
      </div>
    </DialogContent>
  </Dialog>
</template>
