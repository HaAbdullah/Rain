<script setup lang="ts">
// Keycaps for a key combo ('Ctrl+Enter' shows as two caps), with an optional text label in front.
import { computed } from 'vue'
import { formatCombo } from '@/lib/keys'

const props = defineProps<{
  /** Combo in keymap form, such as 'Ctrl+Enter' or 'Ctrl+KeyK'. */
  keys: string
  label?: string
}>()

const caps = computed(() => formatCombo(props.keys))
</script>

<template>
  <span class="inline-flex shrink-0 items-center gap-1">
    <span v-if="label" class="mr-1">{{ label }}</span>
    <kbd
      v-for="(cap, i) in caps"
      :key="i"
      data-slot="kbd"
      class="inline-flex h-5 min-w-5 items-center justify-center rounded-sm border border-border bg-surface-2 px-[5px] font-sans text-xs font-medium text-muted-foreground"
    >
      {{ cap }}
    </kbd>
  </span>
</template>
