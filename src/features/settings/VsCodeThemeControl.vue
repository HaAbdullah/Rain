<script setup lang="ts">
// The VS Code part of the Appearance row: the switch "Follow VS Code" and a button that opens a
// searchable list of the themes of the installed VS Code extensions ("From VS Code"). The list
// is read from the backend when it is opened, never before. Picking a theme applies it at once;
// following VS Code takes the theme VS Code uses now (useTheme checks it when Rain starts, when
// the window is summoned and when this screen opens). Enter on the row opens the list.
import { ChevronsUpDownIcon, MoonIcon, SunIcon } from '@lucide/vue'
import { computed, ref } from 'vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import { Button } from '@/components/ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Switch } from '@/components/ui/switch'
import { useNotice } from '@/composables/useNotice'
import { useOverlay } from '@/composables/useOverlay'
import {
  chooseVsCodeTheme,
  followVsCode,
  followVsCodeProblem,
  setFollowVsCode,
  theme,
  vscodeTheme,
} from '@/composables/useTheme'
import { api } from '@/data'
import type { VsCodeTheme } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { keepFocus } from '@/lib/focus'

const notice = useNotice()
const open = ref(false)
useOverlay(open)

const themes = ref<VsCodeTheme[]>([])
const loaded = ref(false)

/** The button text: the VS Code theme in use, else a call to choose one. */
const buttonText = computed(() =>
  theme.value === 'vscode' && vscodeTheme.value ? vscodeTheme.value.label : 'From VS Code',
)
const chosenId = computed(() => (theme.value === 'vscode' ? (vscodeTheme.value?.id ?? '') : ''))

async function loadList() {
  try {
    themes.value = await api.listVsCodeThemes()
  } catch (error) {
    themes.value = []
    notice.error(errorMessage(error, 'Could not read the VS Code themes'))
  } finally {
    loaded.value = true
  }
}

function onOpen(value: boolean) {
  open.value = value
  if (value) {
    loaded.value = false
    void loadList()
  }
}

async function pick(id: string) {
  open.value = false
  try {
    await chooseVsCodeTheme(id)
  } catch (error) {
    notice.error(errorMessage(error, 'Could not read that VS Code theme'))
  }
}

async function follow(on: boolean) {
  await setFollowVsCode(on)
  if (on && followVsCodeProblem.value) notice.error(followVsCodeProblem.value)
}

/** Enter on the row. */
function focus() {
  onOpen(true)
}

defineExpose({ focus })
</script>

<template>
  <div class="flex items-center gap-2" @mousedown="keepFocus">
    <StatusDot v-if="followVsCodeProblem" status="waiting" :label="followVsCodeProblem" />
    <label class="flex items-center gap-2 text-13 text-muted-foreground">
      <span>Follow VS Code</span>
      <Switch
        :model-value="followVsCode"
        aria-label="Follow VS Code"
        tabindex="-1"
        @update:model-value="follow($event)"
      />
    </label>

    <Popover :open="open" @update:open="onOpen">
      <PopoverTrigger as-child>
        <Button variant="outline" tabindex="-1" class="w-52 justify-between" aria-label="Pick a theme from VS Code">
          <span class="truncate">{{ buttonText }}</span>
          <ChevronsUpDownIcon :size="14" :stroke-width="1.75" class="shrink-0 opacity-60" aria-hidden="true" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" class="w-80 p-0">
        <Command class="h-80" @update:model-value="pick(String($event))">
          <CommandInput placeholder="Find a VS Code theme" />
          <CommandList class="max-h-none flex-1 p-1">
            <CommandEmpty class="text-muted-foreground">
              {{ loaded ? 'No VS Code theme found' : 'Reading the VS Code extensions' }}
            </CommandEmpty>
            <!-- A CommandItem only works inside a CommandGroup: the group is what the filter counts. -->
            <CommandGroup class="p-0">
              <CommandItem
                v-for="entry in themes"
                :key="entry.id"
                :value="entry.id"
                :class="entry.id === chosenId ? 'bg-selected' : ''"
                class="gap-2"
              >
                <MoonIcon v-if="entry.dark" :size="14" :stroke-width="1.75" class="shrink-0 text-text-3" aria-hidden="true" />
                <SunIcon v-else :size="14" :stroke-width="1.75" class="shrink-0 text-text-3" aria-hidden="true" />
                <span class="min-w-0 truncate">{{ entry.label }}</span>
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  </div>
</template>
