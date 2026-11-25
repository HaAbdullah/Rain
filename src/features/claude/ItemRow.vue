<script setup lang="ts">
// One file of the library in the list: icon, name, a line about it, and two quiet labels
// (the kind or memory type, and the scope).
import { computed } from 'vue'
import ListRow from '@/components/rain/ListRow.vue'
import type { LibraryItem } from '@/data/types'
import { KIND_ICON, KIND_LABEL } from './model'

const props = defineProps<{ item: LibraryItem; selected: boolean }>()

const icon = computed(() => ({ kind: 'lucide' as const, value: KIND_ICON[props.item.kind] }))
/** A memory file says what it is (user, feedback ...); the others say their kind. */
const label = computed(() => props.item.memoryType ?? KIND_LABEL[props.item.kind])
</script>

<template>
  <ListRow :icon="icon" :title="item.name" :subtitle="item.description" :selected="selected" size="lg">
    <template #trailing>
      <span class="text-xs text-text-3">{{ label }}</span>
      <span class="max-w-36 truncate text-xs text-text-3" :title="item.scope">{{ item.scope }}</span>
    </template>
  </ListRow>
</template>
