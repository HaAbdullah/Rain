<script setup lang="ts">
// Section navigation (spec 3.3): one icon button per route, tooltips with name and key.
import { CloudRainIcon } from '@lucide/vue'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useNeedsYou } from '@/composables/useNeedsYou'
import { keepFocus } from '@/lib/focus'
import { lucideIcon } from '@/lib/icon-map'
import { routes } from '@/nav/routes'
import { useNav } from '@/nav/useNav'
import KeyHint from './KeyHint.vue'

const nav = useNav()
const { waiting, hasFailedJob } = useNeedsYou()

const rails = [
  { name: 'top', defs: routes.filter((def) => def.rail === 'top'), class: '' },
  { name: 'bottom', defs: routes.filter((def) => def.rail === 'bottom'), class: 'mt-auto pb-2' },
]
</script>

<template>
  <nav class="flex h-full flex-col border-r border-sidebar-border bg-sidebar">
    <div data-tauri-drag-region class="flex h-14 shrink-0 items-center justify-center">
      <CloudRainIcon class="pointer-events-none size-5 text-muted-foreground" :stroke-width="1.75" />
    </div>

    <div
      v-for="rail in rails"
      :key="rail.name"
      class="flex flex-col items-center gap-2"
      :class="rail.class"
    >
      <Tooltip v-for="def in rail.defs" :key="def.name">
        <TooltipTrigger as-child>
          <button
            type="button"
            tabindex="-1"
            :aria-label="def.title"
            :aria-current="nav.displayed.value === def.name ? 'page' : undefined"
            class="relative flex size-10 items-center justify-center rounded-lg transition-colors"
            :class="
              nav.displayed.value === def.name
                ? 'bg-selected text-foreground'
                : 'text-sidebar-foreground hover:bg-hover'
            "
            @mousedown="keepFocus"
            @click="nav.go(def.name)"
          >
            <component :is="lucideIcon(def.icon)" class="size-5" :stroke-width="1.75" />
            <template v-if="def.name === 'workflows'">
              <span
                v-if="waiting > 0"
                class="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-status-waiting px-1 text-xs font-semibold text-background"
              >
                {{ waiting }}
              </span>
              <span
                v-else-if="hasFailedJob"
                class="absolute top-1 right-1 size-2 rounded-full bg-status-failed"
              />
            </template>
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">
          <span>{{ def.title }}</span>
          <KeyHint :keys="def.combo" />
        </TooltipContent>
      </Tooltip>
    </div>
  </nav>
</template>
