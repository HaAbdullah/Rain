<script setup lang="ts">
// Placeholder rows in the shape of a list, shown while it loads for the first time.
import { Skeleton } from '@/components/ui/skeleton'
import type { RowSize } from '@/lib/list-context'

withDefaults(defineProps<{ size?: RowSize; rows?: number }>(), { size: 'md', rows: 6 })
</script>

<template>
  <div class="flex flex-col gap-1 p-2" aria-hidden="true">
    <Skeleton
      v-for="row in rows"
      :key="row"
      class="rounded-lg"
      :class="size === 'lg' ? 'h-(--row-lg-h)' : 'h-(--row-h)'"
    />
  </div>
</template>
