<script setup lang="ts">
// The centred "nothing here" state of a screen or a pane: icon, one line, at most one button.
import { Button } from '@/components/ui/button'
import { keepFocus } from '@/lib/focus'
import { lucideIcon } from '@/lib/icon-map'
import KeyHint from './KeyHint.vue'

defineProps<{
  /** Lucide name, normally the section's icon. */
  icon: string
  title: string
  hint?: string
  actionLabel?: string
  /** Combo that runs the same action, shown as keycaps on the button. */
  actionKeys?: string
}>()

const emit = defineEmits<{ action: [] }>()
</script>

<template>
  <div class="flex h-full min-h-48 w-full flex-col items-center justify-center gap-2 p-6 text-center">
    <component :is="lucideIcon(icon)" :size="32" :stroke-width="1.75" class="mb-2 text-text-3" aria-hidden="true" />
    <p class="text-xl font-semibold text-foreground">{{ title }}</p>
    <p v-if="hint" class="max-w-sm text-13 text-muted-foreground">{{ hint }}</p>
    <Button
      v-if="actionLabel"
      variant="outline"
      class="mt-3 gap-2"
      tabindex="-1"
      @mousedown="keepFocus"
      @click="emit('action')"
    >
      {{ actionLabel }}
      <KeyHint v-if="actionKeys" :keys="actionKeys" />
    </Button>
  </div>
</template>
