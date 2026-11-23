<script setup lang="ts">
// "N folders not accessible": a lock and the count. The tooltip says what it means (spec 7.7).
import { LockIcon } from '@lucide/vue'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { formatNumber } from '@/lib/format'

defineProps<{ count: number }>()
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <Tooltip>
      <TooltipTrigger as-child>
        <span
          class="flex shrink-0 items-center gap-1 text-xs text-muted-foreground"
          role="img"
          :aria-label="`${formatNumber(count)} folders not accessible`"
        >
          <LockIcon :size="12" :stroke-width="1.75" aria-hidden="true" />
          {{ formatNumber(count) }}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        {{ formatNumber(count) }} folders that Windows did not let Rain read. Their size is not in the total.
      </TooltipContent>
    </Tooltip>
  </TooltipProvider>
</template>
