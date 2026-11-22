<script setup lang="ts">
// The strip of sub-views under the header (Sessions / Jobs / Idle, Month / Year, game sources).
// Tab cycles it through the keymap; a click picks one. It never takes focus.
import { TriangleAlertIcon } from '@lucide/vue'
import { keepFocus } from '@/lib/focus'
import RIcon from './RIcon.vue'

export interface ViewStripItem {
  id: string
  label: string
  /** Lucide name, or a simple-icons slug when `brand` is set. */
  icon: string
  brand?: boolean
  count?: number
  /** Marks a view that needs attention, such as a source that failed to load. */
  warn?: boolean
}

defineProps<{
  items: ViewStripItem[]
  modelValue: string
}>()

const emit = defineEmits<{ 'update:modelValue': [id: string] }>()
</script>

<template>
  <div class="flex h-(--strip-h) w-full min-w-0 items-center gap-1 px-2" @mousedown="keepFocus">
    <div class="flex min-w-0 items-center gap-1" role="tablist">
      <button
        v-for="item in items"
        :key="item.id"
        type="button"
        role="tab"
        tabindex="-1"
        :aria-selected="item.id === modelValue"
        class="flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2 text-13 transition-colors"
        :class="
          item.id === modelValue
            ? 'bg-selected text-foreground'
            : 'text-muted-foreground hover:bg-hover hover:text-foreground'
        "
        @click="emit('update:modelValue', item.id)"
      >
        <RIcon :icon="{ kind: item.brand ? 'brand' : 'lucide', value: item.icon }" :size="16" />
        <span>{{ item.label }}</span>
        <span
          v-if="item.count !== undefined"
          class="text-xs"
          :class="item.warn ? 'text-status-waiting' : 'text-text-3'"
        >
          {{ item.count }}
        </span>
        <TriangleAlertIcon
          v-if="item.warn"
          :size="12"
          :stroke-width="1.75"
          class="text-status-waiting"
          aria-hidden="true"
        />
      </button>
    </div>

    <div v-if="$slots.end" class="ml-auto flex shrink-0 items-center gap-1 pl-2">
      <slot name="end" />
    </div>
  </div>
</template>
