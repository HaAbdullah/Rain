<script setup lang="ts">
// A file path in mono type that is cut in the middle when it does not fit,
// so the drive and the last folder both stay readable. The text can be selected.
import { computed } from 'vue'

const props = defineProps<{ path: string }>()

/** Head shrinks with an ellipsis; tail (the last segment with its separator) stays whole. */
const parts = computed(() => {
  const trimmed = props.path.replace(/[\\/]+$/, '')
  const cut = Math.max(trimmed.lastIndexOf('\\'), trimmed.lastIndexOf('/'))
  if (cut <= 0) return { head: '', tail: props.path }
  return { head: props.path.slice(0, cut), tail: props.path.slice(cut) }
})
</script>

<template>
  <span class="selectable flex min-w-0 font-mono text-xs text-muted-foreground" :title="path">
    <span v-if="parts.head" class="min-w-0 truncate">{{ parts.head }}</span>
    <!-- The cap keeps one very long folder name from pushing the head out completely. -->
    <span class="truncate" :class="parts.head ? 'max-w-[75%] shrink-0' : 'min-w-0'">{{ parts.tail }}</span>
  </span>
</template>
