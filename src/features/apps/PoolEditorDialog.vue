<script setup lang="ts">
// The dialog that creates or edits a pool. It is open while `poolEditor.draft` is set;
// Save writes it with api.savePool, Cancel and Esc drop it.
import { computed, ref, watch } from 'vue'
import RIcon from '@/components/rain/RIcon.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useNotice } from '@/composables/useNotice'
import { useOverlay } from '@/composables/useOverlay'
import { refreshResources, useResource } from '@/composables/useResource'
import { api } from '@/data'
import type { PoolMember } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { poolEditor } from './actions'

const { draft } = poolEditor
const notice = useNotice()
const apps = useResource('apps', () => api.listApps())

const open = computed({
  get: () => draft.value !== null,
  set: (isOpen) => {
    if (!isOpen) draft.value = null
  },
})
useOverlay(open)

const name = ref('')
/** The ids of the checked apps, in the order they were checked. */
const checkedIds = ref<string[]>([])
/** The members as they were when the dialog opened: they come first in the list and keep their args and delay. */
const original = ref<PoolMember[]>([])
const saving = ref(false)
const failure = ref<string | null>(null)
/** Set when the dialog opens, so the title does not change while the dialog fades out. */
const title = ref('New pool')

watch(
  draft,
  (current) => {
    if (!current) return
    title.value = current.id ? 'Edit pool' : 'New pool'
    name.value = current.name
    original.value = current.members
    checkedIds.value = current.members.map((member) => member.appId)
    saving.value = false
    failure.value = null
  },
  { immediate: true },
)

/** Members first (as they were when opened, so rows do not jump while ticking), then the rest A to Z. */
const listedApps = computed(() => {
  const all = apps.data.value ?? []
  const members = new Set(original.value.map((member) => member.appId))
  const first = original.value.flatMap((member) => all.find((app) => app.id === member.appId) ?? [])
  const rest = all.filter((app) => !members.has(app.id)).sort((a, b) => a.name.localeCompare(b.name))
  return [...first, ...rest]
})

const canSave = computed(() => name.value.trim() !== '' && checkedIds.value.length > 0 && !saving.value)

/** Checked members that were already in the pool keep their args and delay; new ones start plain. */
function membersToSave(): PoolMember[] {
  const kept = original.value.filter((member) => checkedIds.value.includes(member.appId))
  const keptIds = new Set(kept.map((member) => member.appId))
  const added = checkedIds.value.filter((id) => !keptIds.has(id)).map((appId) => ({ appId }))
  return [...kept, ...added]
}

async function save() {
  if (!draft.value || !canSave.value) return
  saving.value = true
  failure.value = null
  try {
    const pool = await api.savePool({ id: draft.value.id, name: name.value.trim(), members: membersToSave() })
    refreshResources('pools', 'pinned', 'recent')
    notice.success(`Saved ${pool.name}`)
    draft.value = null
  } catch (reason) {
    failure.value = errorMessage(reason, 'Could not save the pool')
    saving.value = false
  }
}
</script>

<template>
  <Dialog v-model:open="open">
    <!-- Capture phase: the list inside would otherwise take Ctrl+Enter as "tick the highlighted app". -->
    <DialogContent class="sm:max-w-120" :show-close-button="false" @keydown.capture.ctrl.enter.stop.prevent="save">
      <DialogHeader>
        <DialogTitle class="text-base font-semibold">{{ title }}</DialogTitle>
        <DialogDescription class="text-13 text-muted-foreground">
          Pick the apps that start together.
        </DialogDescription>
      </DialogHeader>

      <div class="flex flex-col gap-2">
        <Label for="pool-name" class="text-13 text-muted-foreground">Name</Label>
        <Input id="pool-name" :model-value="name" placeholder="Web dev" @update:model-value="name = String($event)" />
      </div>

      <div class="flex flex-col gap-2">
        <span class="text-13 text-muted-foreground">Apps</span>
        <Command v-model="checkedIds" multiple class="h-72 rounded-lg border border-border bg-surface-0">
          <CommandInput placeholder="Find an app" />
          <CommandList class="max-h-none flex-1 p-1">
            <CommandEmpty class="text-muted-foreground">No app matches</CommandEmpty>
            <!-- A CommandItem only works inside a CommandGroup: the group is what the filter counts. -->
            <CommandGroup class="p-0">
              <CommandItem
                v-for="app in listedApps"
                :key="app.id"
                :value="app.id"
                class="gap-3 [&>svg:last-child]:hidden"
              >
                <Checkbox
                  :model-value="checkedIds.includes(app.id)"
                  tabindex="-1"
                  class="pointer-events-none"
                  aria-hidden="true"
                />
                <RIcon :icon="app.icon" :size="20" class="size-5" />
                <span class="min-w-0 truncate">{{ app.name }}</span>
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </div>

      <p v-if="failure" class="text-13 text-status-failed" role="alert">{{ failure }}</p>

      <DialogFooter>
        <Button variant="ghost" @click="open = false">Cancel</Button>
        <Button class="gap-2" :disabled="!canSave" @click="save">
          Save
          <KeyHint keys="Ctrl+Enter" />
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
