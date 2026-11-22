<script setup lang="ts">
// One row of the Home list: the shared ListRow plus Home's trailing type icon,
// or the member icons of a pool.
import IconStack from '@/components/rain/IconStack.vue'
import ListRow from '@/components/rain/ListRow.vue'
import type { Status } from '@/data/types'
import { lucideIcon } from '@/lib/icon-map'
import type { HomeRow } from './homeRows'

defineProps<{
  row: HomeRow
  selected: boolean
  /** A launch in progress overrides the status the item has by itself. */
  status?: Status
  digit?: number
  subtitle?: string
}>()
</script>

<template>
  <ListRow
    :icon="row.icon"
    :title="row.title"
    :subtitle="subtitle"
    :status="status"
    :selected="selected"
    :digit="digit"
  >
    <template #trailing>
      <IconStack v-if="row.stack" :icons="row.stack" />
      <component :is="lucideIcon(row.typeIcon)" v-else :size="16" :stroke-width="1.75" class="text-text-3" aria-hidden="true" />
    </template>
  </ListRow>
</template>
