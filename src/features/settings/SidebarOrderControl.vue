<script setup lang="ts">
// The control of the "Sidebar order" row: the sections in the order of the rail, each with
// move up and move down, and "Reset order". The same order can be set by dragging the icons.
// Enter on the row focuses the control; Left and Right choose a section, Shift+Up and
// Shift+Down move it, Enter puts the caret back in the search field.
import { ArrowDownIcon, ArrowUpIcon, RotateCcwIcon } from '@lucide/vue'
import { ref, useTemplateRef } from 'vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import { Button } from '@/components/ui/button'
import { useSearch } from '@/composables/useSearch'
import { lucideIcon } from '@/lib/icon-map'
import type { RouteName } from '@/nav/routes'
import { isDefaultOrder, moveSection, railTop, resetRailOrder, routeCombo } from '@/nav/useRailOrder'

const search = useSearch()
const group = useTemplateRef<HTMLElement>('group')
/** The section the keys act on. */
const chosen = ref<RouteName | null>(null)

function focus() {
  chosen.value ??= railTop.value[0]?.name ?? null
  group.value?.focus({ preventScroll: true })
}

function stop(event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
}

function choose(step: -1 | 1, event: KeyboardEvent) {
  stop(event)
  const names = railTop.value.map((def) => def.name)
  const index = names.indexOf(chosen.value ?? names[0])
  chosen.value = names[Math.min(Math.max(index + step, 0), names.length - 1)]
}

function move(name: RouteName | null, step: -1 | 1, event?: KeyboardEvent) {
  if (event) stop(event)
  if (!name) return
  moveSection(name, step)
  chosen.value = name
  // The key press moved focus to the search field; the control keeps it for the next step.
  group.value?.focus({ preventScroll: true })
}

function done(event: KeyboardEvent) {
  stop(event)
  search.focus()
}

defineExpose({ focus })
</script>

<template>
  <div
    ref="group"
    role="group"
    aria-label="Sidebar order"
    tabindex="-1"
    class="flex w-72 flex-col gap-0.5 rounded-lg py-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
    @keydown.left="choose(-1, $event)"
    @keydown.right="choose(1, $event)"
    @keydown.shift.up="move(chosen, -1, $event)"
    @keydown.shift.down="move(chosen, 1, $event)"
    @keydown.enter="done"
  >
    <div
      v-for="(def, index) in railTop"
      :key="def.name"
      class="flex items-center gap-2 rounded-md px-1"
      :class="chosen === def.name ? 'bg-hover' : ''"
      @click="chosen = def.name"
    >
      <component :is="lucideIcon(def.icon)" class="size-4 shrink-0 text-muted-foreground" :stroke-width="1.75" />
      <span class="min-w-0 flex-1 truncate text-13 text-foreground">{{ def.title }}</span>
      <KeyHint v-if="routeCombo(def)" :keys="routeCombo(def)!" />
      <Button
        variant="ghost"
        size="icon"
        tabindex="-1"
        class="size-7"
        :aria-label="`Move ${def.title} up`"
        :disabled="index === 0"
        @click.stop="move(def.name, -1)"
      >
        <ArrowUpIcon :size="14" :stroke-width="1.75" aria-hidden="true" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        tabindex="-1"
        class="size-7"
        :aria-label="`Move ${def.title} down`"
        :disabled="index === railTop.length - 1"
        @click.stop="move(def.name, 1)"
      >
        <ArrowDownIcon :size="14" :stroke-width="1.75" aria-hidden="true" />
      </Button>
    </div>
    <Button
      variant="ghost"
      size="sm"
      tabindex="-1"
      class="mt-1 self-end"
      aria-label="Reset order"
      :disabled="isDefaultOrder"
      @click="resetRailOrder()"
    >
      <RotateCcwIcon :size="14" :stroke-width="1.75" aria-hidden="true" />
      Reset order
    </Button>
  </div>
</template>
