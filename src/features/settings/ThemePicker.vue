<script setup lang="ts">
// The theme picker of the Appearance row: one small swatch card per theme, applied at once.
// It is a radio group. Enter on the row focuses the chosen card (see SettingRow); Left and
// Right then switch theme, and Enter puts the caret back in the search field. A click does
// not take focus, like every other control of the screen.
import { CheckIcon } from '@lucide/vue'
import { useTemplateRef } from 'vue'
import { setTheme, theme, themes, type ThemeId } from '@/composables/useTheme'
import { useSearch } from '@/composables/useSearch'

const search = useSearch()
const group = useTemplateRef<HTMLElement>('group')

function cards(): HTMLElement[] {
  return Array.from(group.value?.querySelectorAll<HTMLElement>('[role="radio"]') ?? [])
}

/** Puts keyboard focus on the card of the theme in use. */
function focus() {
  cards()
    .find((card) => card.dataset.themeId === theme.value)
    ?.focus({ preventScroll: true })
}

function choose(id: ThemeId) {
  setTheme(id)
}

function step(delta: number, event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
  const index = themes.findIndex((entry) => entry.id === theme.value)
  const next = themes[(index + delta + themes.length) % themes.length]
  if (!next) return
  choose(next.id)
  cards()
    .find((card) => card.dataset.themeId === next.id)
    ?.focus({ preventScroll: true })
}

function done(event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
  search.focus()
}

defineExpose({ focus })
</script>

<template>
  <div
    ref="group"
    role="radiogroup"
    aria-label="Theme"
    class="flex gap-2"
    @keydown.left="step(-1, $event)"
    @keydown.right="step(1, $event)"
    @keydown.enter="done"
  >
    <button
      v-for="entry in themes"
      :key="entry.id"
      type="button"
      role="radio"
      tabindex="-1"
      :aria-checked="entry.id === theme"
      :aria-label="entry.name"
      :data-theme-id="entry.id"
      class="flex w-20 flex-col gap-1 rounded-lg p-1 outline-none transition-colors hover:bg-hover focus-visible:ring-2 focus-visible:ring-ring"
      @click="choose(entry.id)"
    >
      <!-- The swatch shows the theme's own colours, so these stay fixed whichever theme is in use. -->
      <span
        class="flex h-9 items-center justify-between rounded-md px-2 ring-1 ring-inset"
        :class="entry.id === theme ? 'ring-2 ring-ring' : 'ring-border-strong'"
        :style="{ backgroundColor: entry.swatch.surface, color: entry.swatch.text }"
      >
        <span class="text-13 leading-none font-semibold">Aa</span>
        <span class="size-3 rounded-full" :style="{ backgroundColor: entry.swatch.accent }" />
      </span>
      <span
        class="flex items-center justify-center gap-1 text-xs leading-4"
        :class="entry.id === theme ? 'text-foreground' : 'text-muted-foreground'"
      >
        <span class="truncate">{{ entry.name }}</span>
        <CheckIcon v-if="entry.id === theme" :size="12" :stroke-width="2" class="shrink-0" aria-hidden="true" />
      </span>
    </button>
  </div>
</template>
