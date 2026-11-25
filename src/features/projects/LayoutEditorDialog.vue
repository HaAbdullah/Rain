<script setup lang="ts">
// The dialog that changes how a project opens: what opens, and where on the screen it goes. It is
// open while `layoutEditor.target` is set. Pick a preset, then change any item: its zone, its
// monitor, remove it, or add one from a short picker (the steps, and "App..." for Rain's app
// list). "Use for all projects" writes the default layout instead of the project's own.
// Everything is reachable with Tab, the arrow keys and Enter; Ctrl+Enter saves, Esc steps back.
import { PlusIcon, Trash2Icon } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { useNotice } from '@/composables/useNotice'
import { useOverlay } from '@/composables/useOverlay'
import { refreshResources, useResource } from '@/composables/useResource'
import { api } from '@/data'
import type { LayoutItem, LayoutWhat, LayoutZone, ProjectLayout } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { layoutEditor } from './actions'
import LayoutDiagram from './LayoutDiagram.vue'
import {
  PRESETS,
  ZONES,
  addItem,
  canAdd,
  describe,
  itemsOfPreset,
  presetOf,
  removeItem,
  sameItems,
  setMonitor,
  setZone,
} from './layout'
import ZoneGlyph from './ZoneGlyph.vue'

const { target } = layoutEditor
const notice = useNotice()
const apps = useResource('apps', () => api.listApps())

const open = computed({
  get: () => target.value !== null,
  set: (isOpen) => {
    if (!isOpen) target.value = null
  },
})
useOverlay(open)

/** What the backend says about the layout of the project now; null until it answers. */
const loaded = ref<ProjectLayout | null>(null)
const items = ref<LayoutItem[]>([])
const everyProject = ref(false)
const saving = ref(false)
const failure = ref<string | null>(null)
/** `closed`: the list of items; `steps`: the picker of what to add; `apps`: the search of Rain's apps. */
const picker = ref<'closed' | 'steps' | 'apps'>('closed')
/** Set when the dialog opens, so the title does not change while the dialog fades out. */
const title = ref('Layout')

watch(
  target,
  async (current) => {
    if (!current) return
    title.value = `Layout of ${current.name}`
    loaded.value = null
    items.value = []
    everyProject.value = false
    saving.value = false
    failure.value = null
    picker.value = 'closed'
    try {
      const layout = await api.getProjectLayout(current.id)
      // Closed or switched to another project meanwhile: the answer is not for this dialog.
      if (target.value?.id !== current.id) return
      loaded.value = layout
      items.value = layout.items.map((item) => ({ ...item }))
      everyProject.value = layout.scope === 'default'
    } catch (reason) {
      failure.value = errorMessage(reason, 'Could not read the layout')
    }
  },
  { immediate: true },
)

const steps = computed(() => loaded.value?.steps ?? [])
const appList = computed(() => apps.data.value ?? [])
const monitorCount = computed(() => loaded.value?.monitors ?? 1)
const activePreset = computed(() => presetOf(items.value))

/** The monitor choices: this screen, then each monitor the PC has (and one a file names that is gone). */
function monitorChoices(item: LayoutItem): string[] {
  const top = Math.max(monitorCount.value, item.monitor ?? 0)
  return ['this', ...Array.from({ length: top }, (_, i) => String(i + 1))]
}

const hasMonitorChoice = computed(() => monitorCount.value > 1 || items.value.some((item) => item.monitor !== undefined))

const changed = computed(() => loaded.value !== null && !sameItems(items.value, loaded.value.items))
const canSave = computed(
  () =>
    loaded.value !== null &&
    items.value.length > 0 &&
    !saving.value &&
    (changed.value || everyProject.value !== (loaded.value.scope === 'default')),
)
const canRemove = computed(() => loaded.value !== null && loaded.value.scope !== 'none' && !saving.value)

function applyPreset(index: number) {
  items.value = itemsOfPreset(PRESETS[index], steps.value)
  failure.value = null
}

function add(what: LayoutWhat) {
  items.value = addItem(items.value, what)
  picker.value = 'closed'
}

function pickApp(id: string) {
  add({ kind: 'app', id })
}

const stepChoices = computed(() => steps.value)

/** Esc steps back out of the picker before it closes the dialog. */
function onEscape(event: Event) {
  if (picker.value === 'closed') return
  event.preventDefault()
  picker.value = picker.value === 'apps' ? 'steps' : 'closed'
}

async function send(save: { items: LayoutItem[]; all: boolean }, done: string) {
  const current = target.value
  if (!current) return
  saving.value = true
  failure.value = null
  try {
    await api.saveProjectLayout(current.id, save)
    refreshResources('projects', 'projectLayout')
    notice.success(done)
    target.value = null
  } catch (reason) {
    failure.value = errorMessage(reason, 'Could not save the layout')
    saving.value = false
  }
}

async function save() {
  if (!canSave.value) return
  await send({ items: items.value, all: everyProject.value }, 'Saved the layout')
}

/** Removes the layout that counts for the project now: its own, or the one of all projects. */
async function remove() {
  if (!canRemove.value || !loaded.value) return
  await send({ items: [], all: loaded.value.scope === 'default' }, 'Removed the layout')
}
</script>

<template>
  <Dialog v-model:open="open">
    <!-- Capture phase: the list inside would otherwise take Ctrl+Enter as "pick the highlighted row". -->
    <DialogContent
      class="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-xl"
      :show-close-button="false"
      @keydown.capture.ctrl.enter.stop.prevent="save"
      @escape-key-down="onEscape"
    >
      <DialogHeader>
        <DialogTitle class="text-base font-semibold">{{ title }}</DialogTitle>
        <DialogDescription class="text-13 text-muted-foreground">
          What opens with the project, and where on the screen it goes.
        </DialogDescription>
      </DialogHeader>

      <template v-if="picker === 'closed'">
        <LayoutDiagram v-if="items.length > 0" :items="items" :steps="steps" :apps="appList" />

        <div class="flex flex-col gap-2">
          <span class="text-13 text-muted-foreground">Start from</span>
          <div class="grid grid-cols-2 gap-2">
            <Button
              v-for="(preset, index) in PRESETS"
              :key="preset.id"
              variant="outline"
              class="h-auto justify-start gap-2 whitespace-normal py-2 text-left text-13"
              :class="activePreset?.id === preset.id ? 'border-ring' : ''"
              :aria-pressed="activePreset?.id === preset.id"
              :disabled="loaded === null"
              @click="applyPreset(index)"
            >
              <RIcon :icon="{ kind: 'lucide', value: preset.icon }" :size="16" />
              <span class="min-w-0">{{ preset.label }}</span>
            </Button>
          </div>
        </div>

        <div class="flex flex-col gap-1">
          <span class="text-13 text-muted-foreground">Windows</span>
          <ul class="flex flex-col">
            <li v-for="(item, index) in items" :key="index" class="flex h-10 items-center gap-2">
              <RIcon :icon="describe(item.what, steps, appList).icon" :size="16" />
              <span class="min-w-0 flex-1 truncate text-sm text-foreground">
                {{ describe(item.what, steps, appList).label }}
              </span>
              <Select
                :model-value="item.zone"
                @update:model-value="items = setZone(items, index, $event as LayoutZone)"
              >
                <SelectTrigger
                  size="sm"
                  class="w-44"
                  :aria-label="`Where ${describe(item.what, steps, appList).label} goes`"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem v-for="info in ZONES" :key="info.zone" :value="info.zone">
                    <span class="flex items-center gap-2">
                      <ZoneGlyph :zone="info.zone" :width="18" />
                      {{ info.label }}
                    </span>
                  </SelectItem>
                </SelectContent>
              </Select>
              <Select
                v-if="hasMonitorChoice"
                :model-value="item.monitor === undefined ? 'this' : String(item.monitor)"
                @update:model-value="items = setMonitor(items, index, $event === 'this' ? undefined : Number($event))"
              >
                <SelectTrigger
                  size="sm"
                  class="w-32"
                  :aria-label="`Monitor of ${describe(item.what, steps, appList).label}`"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem v-for="choice in monitorChoices(item)" :key="choice" :value="choice">
                    {{ choice === 'this' ? 'This screen' : `Monitor ${choice}` }}
                  </SelectItem>
                </SelectContent>
              </Select>
              <Button
                variant="ghost"
                size="icon-sm"
                :aria-label="`Remove ${describe(item.what, steps, appList).label}`"
                @click="items = removeItem(items, index)"
              >
                <Trash2Icon :size="16" :stroke-width="1.75" aria-hidden="true" />
              </Button>
            </li>
          </ul>
          <p v-if="items.length === 0 && loaded !== null" class="text-13 text-muted-foreground">
            Nothing yet. Pick a start above, or add a window.
          </p>
          <Button
            variant="ghost"
            class="w-fit gap-2 text-13"
            :disabled="loaded === null || !canAdd(items)"
            @click="picker = 'steps'"
          >
            <PlusIcon :size="16" :stroke-width="1.75" aria-hidden="true" />
            Add a window
          </Button>
        </div>

        <div class="flex items-center justify-between gap-4">
          <div class="min-w-0">
            <Label for="layout-all" class="text-sm text-foreground">Use for all projects</Label>
            <p class="text-xs text-muted-foreground">
              For every project that has no layout of its own.
            </p>
          </div>
          <Switch id="layout-all" v-model="everyProject" />
        </div>
      </template>

      <!-- The picker: what to add. A CommandItem only works inside a CommandGroup. -->
      <Command v-else-if="picker === 'steps'" class="h-72 rounded-lg border border-border bg-surface-0">
        <CommandInput placeholder="What opens" />
        <CommandList class="max-h-none flex-1 p-1">
          <CommandEmpty class="text-muted-foreground">Nothing matches</CommandEmpty>
          <CommandGroup class="p-0">
            <CommandItem
              v-for="step in stepChoices"
              :key="step.name"
              :value="step.name"
              class="gap-3 [&>svg:last-child]:hidden"
              @select="add({ kind: 'step', name: step.name })"
            >
              <RIcon :icon="step.icon" :size="20" class="size-5" />
              <span class="min-w-0 truncate">{{ step.label }}</span>
            </CommandItem>
            <CommandItem
              value="project folder explorer"
              class="gap-3 [&>svg:last-child]:hidden"
              @select="add({ kind: 'folder' })"
            >
              <RIcon :icon="{ kind: 'lucide', value: 'folder-open' }" :size="20" class="size-5" />
              <span class="min-w-0 truncate">Project folder</span>
            </CommandItem>
            <CommandItem
              value="app apps program"
              class="gap-3 [&>svg:last-child]:hidden"
              @select="picker = 'apps'"
            >
              <RIcon :icon="{ kind: 'lucide', value: 'app-window' }" :size="20" class="size-5" />
              <span class="min-w-0 truncate">App...</span>
            </CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>

      <Command v-else class="h-72 rounded-lg border border-border bg-surface-0">
        <CommandInput placeholder="Find an app" />
        <CommandList class="max-h-none flex-1 p-1">
          <CommandEmpty class="text-muted-foreground">No app matches</CommandEmpty>
          <CommandGroup class="p-0">
            <CommandItem
              v-for="app in appList"
              :key="app.id"
              :value="app.name"
              class="gap-3 [&>svg:last-child]:hidden"
              @select="pickApp(app.id)"
            >
              <RIcon :icon="app.icon" :size="20" class="size-5" />
              <span class="min-w-0 truncate">{{ app.name }}</span>
            </CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>

      <p v-if="failure" class="text-13 text-status-failed" role="alert">{{ failure }}</p>

      <DialogFooter class="items-center sm:justify-between">
        <Button
          v-if="picker === 'closed'"
          variant="ghost"
          class="text-13 text-muted-foreground"
          :disabled="!canRemove"
          @click="remove"
        >
          Remove layout
        </Button>
        <Button v-else variant="ghost" class="text-13" @click="picker = picker === 'apps' ? 'steps' : 'closed'">
          Back
          <KeyHint keys="Escape" />
        </Button>
        <div class="flex gap-2">
          <Button variant="ghost" @click="open = false">Cancel</Button>
          <Button class="gap-2" :disabled="!canSave || picker !== 'closed'" @click="save">
            Save
            <KeyHint keys="Ctrl+Enter" />
          </Button>
        </div>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
