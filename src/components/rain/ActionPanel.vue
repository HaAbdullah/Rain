<script setup lang="ts">
// Ctrl+K and right-click (spec 3.7): every action for one target, filterable, with its key.
// Anchored in the bottom-right corner, or at the pointer for a right-click.
import { computed } from 'vue'
import { actionContext, runAction } from '@/actions/context'
import { keysOf, orderActions } from '@/actions/display'
import { actionsFor } from '@/actions/registry'
import type { RainAction } from '@/actions/types'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover'
import { useActionPanel } from '@/composables/useActionPanel'
import { lucideIcon } from '@/lib/icon-map'
import KeyHint from './KeyHint.vue'

const panel = useActionPanel()
const { isOpen, anchor, target } = panel

// Danger items go last (see `orderActions`).
const actions = computed<RainAction[]>(() => {
  if (!isOpen.value || !target.value) return []
  return orderActions(actionsFor(target.value, actionContext()))
})

const anchorStyle = computed(() =>
  anchor.value
    ? { left: `${anchor.value.x}px`, top: `${anchor.value.y}px` }
    : { right: '12px', bottom: '12px' },
)

function choose(action: RainAction) {
  panel.close()
  void runAction(action)
}
</script>

<template>
  <Popover :open="isOpen" @update:open="(open) => !open && panel.close()">
    <PopoverAnchor as-child>
      <div class="pointer-events-none fixed size-0" :style="anchorStyle" />
    </PopoverAnchor>
    <PopoverContent
      :side="anchor ? 'bottom' : 'top'"
      :align="anchor ? 'start' : 'end'"
      class="w-80 gap-0 border border-border-strong p-0 shadow-(--shadow-overlay) ring-0"
    >
      <Command>
        <CommandInput placeholder="Filter actions" />
        <CommandList>
          <CommandEmpty class="text-muted-foreground">No matching action</CommandEmpty>
          <CommandGroup>
            <CommandItem
              v-for="action in actions"
              :key="action.id"
              :value="action.id"
              :class="action.danger && 'text-destructive data-highlighted:text-destructive'"
              @select="choose(action)"
            >
              <component
                :is="lucideIcon(action.icon)"
                :class="action.danger && 'text-destructive'"
              />
              <span class="truncate">{{ action.label }}</span>
              <span v-if="keysOf(action)" data-slot="command-shortcut" class="ml-auto">
                <KeyHint :keys="keysOf(action)!" />
              </span>
            </CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    </PopoverContent>
  </Popover>
</template>
