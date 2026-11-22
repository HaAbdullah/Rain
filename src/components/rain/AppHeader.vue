<script setup lang="ts">
// Header (spec 3.2, 2.5): section title, the search field, loader line and window controls.
import { HandIcon, MaximizeIcon, MinimizeIcon, PinIcon, XIcon } from '@lucide/vue'
import { computed } from 'vue'
import { toggleKeepOpen } from '@/actions/common'
import { actionContext } from '@/actions/context'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useNeedsYou } from '@/composables/useNeedsYou'
import { anyResourceLoading } from '@/composables/useResource'
import { keepFocus } from '@/lib/focus'
import { routeByName } from '@/nav/routes'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'
import KeyHint from './KeyHint.vue'
import SearchField from './SearchField.vue'

const nav = useNav()
const { keepOpen, fullscreen } = shell
const { waiting } = useNeedsYou()

const title = computed(() => routeByName[nav.displayed.value].title)

const iconButton =
  'flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-hover hover:text-foreground'
</script>

<template>
  <header
    data-tauri-drag-region
    class="relative grid h-14 grid-cols-[132px_minmax(0,1fr)_auto] items-center border-b border-border bg-surface-1"
  >
    <h1 data-tauri-drag-region class="truncate pl-4 text-sm font-semibold text-foreground">
      {{ title }}
    </h1>

    <div data-tauri-drag-region class="flex h-full items-center">
      <SearchField />
    </div>

    <div class="flex items-center gap-1 pr-3 [app-region:no-drag]">
      <Tooltip v-if="waiting > 0">
        <TooltipTrigger as-child>
          <button
            type="button"
            tabindex="-1"
            class="flex h-8 items-center gap-1 rounded-md bg-status-waiting/15 px-2 text-13 font-medium text-status-waiting transition-colors hover:bg-status-waiting/25"
            :aria-label="`${waiting} waiting on you`"
            @mousedown="keepFocus"
            @click="nav.go('workflows', 'sessions')"
          >
            <HandIcon class="size-4" />
            {{ waiting }}
          </button>
        </TooltipTrigger>
        <TooltipContent>{{ waiting }} waiting on you</TooltipContent>
      </Tooltip>

      <Tooltip>
        <TooltipTrigger as-child>
          <button
            type="button"
            tabindex="-1"
            :class="[iconButton, keepOpen && 'text-ring hover:text-ring']"
            aria-label="Keep open"
            :aria-pressed="keepOpen"
            @mousedown="keepFocus"
            @click="toggleKeepOpen(actionContext())"
          >
            <PinIcon class="size-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent>
          <span>Keep open</span>
          <KeyHint keys="F10" />
        </TooltipContent>
      </Tooltip>

      <Tooltip>
        <TooltipTrigger as-child>
          <button
            type="button"
            tabindex="-1"
            :class="iconButton"
            :aria-label="fullscreen ? 'Leave full screen' : 'Full screen'"
            :aria-pressed="fullscreen"
            @mousedown="keepFocus"
            @click="shell.toggleFullscreen()"
          >
            <component :is="fullscreen ? MinimizeIcon : MaximizeIcon" class="size-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent>
          <span>{{ fullscreen ? 'Leave full screen' : 'Full screen' }}</span>
          <KeyHint keys="F11" />
        </TooltipContent>
      </Tooltip>

      <Tooltip>
        <TooltipTrigger as-child>
          <button
            type="button"
            tabindex="-1"
            :class="iconButton"
            aria-label="Hide"
            @mousedown="keepFocus"
            @click="shell.hide()"
          >
            <XIcon class="size-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent>
          <span>Hide</span>
          <KeyHint keys="Escape" />
        </TooltipContent>
      </Tooltip>
    </div>

    <div
      class="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-ring transition-opacity"
      :class="anyResourceLoading ? 'opacity-100' : 'opacity-0'"
    />
  </header>
</template>
