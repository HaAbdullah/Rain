<script setup lang="ts">
// The standard row of an ItemList: icon, title, subtitle, a trailing slot and a status glyph.
// 'md' is 40 px with the subtitle on the title's line; 'lg' is 56 px with it on a second line.
import type { IconRef } from '@/data/types'
import type { RowSize } from '@/lib/list-context'
import type { StatusKind } from '@/lib/status'
import { computed, inject } from 'vue'
import { altHeld } from '@/keys/useKeymap'
import { rowSizeKey } from '@/lib/list-context'
import KeyHint from './KeyHint.vue'
import RIcon from './RIcon.vue'
import StatusDot from './StatusDot.vue'

const props = defineProps<{
  icon?: IconRef
  title: string
  subtitle?: string
  status?: StatusKind
  selected: boolean
  /** Defaults to the `rowSize` of the ItemList around it, else 'md'. */
  size?: RowSize
  /** The row's Alt+digit number. Its keycap shows only while Alt is held. */
  digit?: number
  dimmed?: boolean
}>()

const listSize = inject(rowSizeKey, undefined)
const rowSize = computed(() => props.size ?? listSize?.value ?? 'md')
</script>

<template>
  <div
    class="flex items-center gap-3 rounded-lg px-2 transition-colors"
    :class="[
      rowSize === 'lg' ? 'h-(--row-lg-h)' : 'h-(--row-h)',
      selected ? 'bg-selected' : 'hover:bg-hover',
      { 'opacity-60': dimmed },
    ]"
  >
    <RIcon v-if="icon" :icon="icon" :size="20" :label="title" />

    <div
      class="min-w-0 flex-1"
      :class="rowSize === 'lg' ? 'flex flex-col justify-center' : 'flex items-baseline gap-2'"
    >
      <!-- On one line the title keeps its width (up to 70%) and the subtitle is cut first. -->
      <span
        class="truncate text-sm text-foreground"
        :class="{ 'max-w-[70%] shrink-0': rowSize === 'md' && subtitle }"
      >
        {{ title }}
      </span>
      <span v-if="subtitle" class="min-w-0 truncate text-13 text-muted-foreground">{{ subtitle }}</span>
    </div>

    <div v-if="$slots.trailing" class="flex shrink-0 items-center gap-2 text-13 text-muted-foreground">
      <slot name="trailing" />
    </div>
    <StatusDot v-if="status" :status="status" />
    <KeyHint v-if="digit !== undefined && altHeld" :keys="String(digit)" />
  </div>
</template>
