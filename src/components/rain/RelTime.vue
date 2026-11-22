<script lang="ts">
// A relative time ("2m", "3h", "yesterday", "in 9 h") that keeps itself current while Rain is visible.
import type { Ref } from 'vue'
import type { ISODateTime } from '@/data/types'
import { computed, effectScope } from 'vue'
import { useNow } from '@/composables/useNow'
import { formatClock, formatDayLabel, relTime } from '@/lib/time'

// One clock for every RelTime on screen: a long list would otherwise start a timer per row.
// It lives in a detached scope so it does not stop when the first row that asked for it unmounts.
let sharedNow: Ref<number> | undefined

function useSharedNow(): Ref<number> {
  sharedNow ??= effectScope(true).run(() => useNow())!
  return sharedNow
}
</script>

<script setup lang="ts">
const props = defineProps<{
  iso: ISODateTime
  /** Compact form for tight spots: "1d" instead of "yesterday". */
  short?: boolean
}>()

const now = useSharedNow()

const text = computed(() => relTime(props.iso, now.value, props.short))
const exact = computed(() => `${formatDayLabel(props.iso)} ${formatClock(props.iso)}`)
</script>

<template>
  <time :datetime="iso" :title="exact" class="shrink-0 whitespace-nowrap">{{ text }}</time>
</template>
