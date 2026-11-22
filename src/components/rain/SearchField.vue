<script setup lang="ts">
// The one text input (spec 3.2, 3.5). It is always focused; the scope chip sits in front of the caret.
import { SearchIcon, XIcon } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, useTemplateRef } from 'vue'
import { registerSearchField, useSearch } from '@/composables/useSearch'
import { keepFocus } from '@/lib/focus'
import { lucideIcon } from '@/lib/icon-map'
import { routeByName } from '@/nav/routes'

const { query, scope, setScope, disabled } = useSearch()
const input = useTemplateRef<HTMLInputElement>('input')

const section = computed(() => (scope.value ? routeByName[scope.value] : null))
const placeholder = computed(() => (section.value ? `Filter ${section.value.title}` : 'Search everything'))

onMounted(() => registerSearchField(input.value))
onBeforeUnmount(() => registerSearchField(null))

defineExpose({ focus: () => input.value?.focus({ preventScroll: true }) })
</script>

<template>
  <div
    class="flex h-9 w-full max-w-[720px] items-center gap-2 rounded-lg border border-border-strong bg-surface-2 px-2.5 [app-region:no-drag]"
  >
    <button
      v-if="section"
      type="button"
      tabindex="-1"
      class="flex h-6 shrink-0 items-center gap-1 rounded-md bg-selected px-1.5 text-foreground"
      :title="`Remove the ${section.title} filter`"
      @mousedown="keepFocus"
      @click="setScope(null)"
    >
      <component :is="lucideIcon(section.icon)" class="size-4" :stroke-width="1.75" />
      <XIcon class="size-3 text-muted-foreground" />
    </button>
    <SearchIcon v-else class="size-4 shrink-0 text-muted-foreground" />
    <input
      ref="input"
      v-model="query"
      :disabled="disabled"
      data-search-field
      type="text"
      spellcheck="false"
      autocomplete="off"
      :placeholder="placeholder"
      class="min-w-0 flex-1 select-text bg-transparent text-base text-foreground caret-ring outline-none placeholder:text-text-3 disabled:opacity-50"
    />
  </div>
</template>
