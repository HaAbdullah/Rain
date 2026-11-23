<script setup lang="ts">
// The dialog that creates or edits a habit. It is open while `habitEditor.draft` is set; Save
// writes it with api.saveHabit, Cancel and Esc drop it. Same rules as the pool editor.
import { computed, ref, watch } from 'vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useNotice } from '@/composables/useNotice'
import { useOverlay } from '@/composables/useOverlay'
import { refreshResources } from '@/composables/useResource'
import { api } from '@/data'
import type { Cadence, PropertySuggestion } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { habitEditor } from './actions'
import { HABIT_ICONS, clampTimes, nameFromProperty, propertyFromName, timesMax } from './habitForm'

const MAX_CHIPS = 8
const CADENCES: { value: Cadence; label: string }[] = [
  { value: 'daily', label: 'Daily' },
  { value: 'weekly', label: 'Weekly' },
  { value: 'monthly', label: 'Monthly' },
]

const { draft } = habitEditor
const notice = useNotice()

const open = computed({
  get: () => draft.value !== null,
  set: (isOpen) => {
    if (!isOpen) draft.value = null
  },
})
useOverlay(open)

const name = ref('')
const property = ref('')
/** True once the user typed in Property (or the habit already had one): then the name no longer fills it. */
const propertyTouched = ref(false)
const icon = ref<string>(HABIT_ICONS[0])
const cadence = ref<Cadence>('daily')
const times = ref(1)
const saving = ref(false)
const failure = ref<string | null>(null)
/** Set when the dialog opens, so the title does not change while the dialog fades out. */
const title = ref('New habit')
/** Checkbox properties found in the notes that no habit uses yet. A local list, not a shared resource. */
const suggestions = ref<PropertySuggestion[]>([])

watch(
  draft,
  (current) => {
    if (!current) return
    title.value = current.id ? 'Edit habit' : 'New habit'
    name.value = current.name
    property.value = current.property
    propertyTouched.value = current.property !== ''
    icon.value = current.icon.kind === 'lucide' ? current.icon.value : HABIT_ICONS[0]
    cadence.value = current.cadence
    times.value = current.times
    saving.value = false
    failure.value = null
    suggestions.value = []
    // A failure here only means there are no chips.
    void api
      .listHabitSuggestions()
      .then((found) => {
        if (draft.value === current) suggestions.value = found
      })
      .catch(() => undefined)
  },
  { immediate: true },
)

// The property follows the name until the user writes their own.
function onName(value: string) {
  name.value = value
  if (!propertyTouched.value) property.value = propertyFromName(value)
}

function onProperty(value: string) {
  property.value = value
  propertyTouched.value = true
}

function pickSuggestion(found: PropertySuggestion) {
  property.value = found.property
  propertyTouched.value = true
  if (name.value.trim() === '') name.value = nameFromProperty(found.property)
}

function pickCadence(value: unknown) {
  // A second click on the lit item would switch it off; one cadence is always on.
  if (value !== 'daily' && value !== 'weekly' && value !== 'monthly') return
  cadence.value = value
  times.value = clampTimes(value, times.value)
}

function onTimes(value: string | number) {
  times.value = clampTimes(cadence.value, Number(value))
}

const chips = computed(() => suggestions.value.slice(0, MAX_CHIPS))
const canSave = computed(() => name.value.trim() !== '' && property.value.trim() !== '' && !saving.value)
const timesLabel = computed(() => (cadence.value === 'weekly' ? 'times a week' : 'times a month'))

async function save() {
  if (!draft.value || !canSave.value) return
  saving.value = true
  failure.value = null
  try {
    const saved = await api.saveHabit({
      id: draft.value.id,
      name: name.value.trim(),
      icon: { kind: 'lucide', value: icon.value },
      property: property.value.trim(),
      cadence: cadence.value,
      times: cadence.value === 'daily' ? 1 : clampTimes(cadence.value, times.value),
    })
    refreshResources('habits', 'habitStats', 'habitDays')
    notice.success(`Saved ${saved.name}`)
    draft.value = null
  } catch (reason) {
    failure.value = errorMessage(reason, 'Could not save the habit')
    saving.value = false
  }
}
</script>

<template>
  <Dialog v-model:open="open">
    <!-- Capture phase: a field inside would otherwise take Ctrl+Enter first. -->
    <DialogContent class="sm:max-w-120" :show-close-button="false" @keydown.capture.ctrl.enter.stop.prevent="save">
      <DialogHeader>
        <DialogTitle class="text-base font-semibold">{{ title }}</DialogTitle>
        <DialogDescription class="text-13 text-muted-foreground">
          A habit is a checkbox property at the top of your daily note.
        </DialogDescription>
      </DialogHeader>

      <div class="flex flex-col gap-2">
        <Label for="habit-name" class="text-13 text-muted-foreground">Name</Label>
        <Input
          id="habit-name"
          :model-value="name"
          placeholder="Gym"
          autofocus
          @update:model-value="onName(String($event))"
        />
      </div>

      <div class="flex flex-col gap-2">
        <Label for="habit-property" class="text-13 text-muted-foreground">Property</Label>
        <Input
          id="habit-property"
          class="font-mono text-xs"
          :model-value="property"
          placeholder="gym"
          @update:model-value="onProperty(String($event))"
        />
        <p class="text-13 text-muted-foreground">
          The checkbox property in your daily note. Changing it changes which notes count.
        </p>
        <div v-if="chips.length > 0" class="flex flex-wrap items-center gap-2">
          <span class="text-13 text-muted-foreground">Found in your notes</span>
          <button
            v-for="found in chips"
            :key="found.property"
            type="button"
            tabindex="-1"
            class="rounded-md bg-surface-2 px-2 py-1 text-xs text-foreground transition-colors hover:bg-hover"
            @click="pickSuggestion(found)"
          >
            {{ found.property }} - {{ found.days }} {{ found.days === 1 ? 'day' : 'days' }}
          </button>
        </div>
      </div>

      <div class="flex flex-col gap-2">
        <span class="text-13 text-muted-foreground">Icon</span>
        <div class="flex flex-wrap gap-1" role="radiogroup" aria-label="Icon">
          <button
            v-for="name_ in HABIT_ICONS"
            :key="name_"
            type="button"
            tabindex="-1"
            role="radio"
            :aria-checked="icon === name_"
            :aria-label="name_"
            class="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
            :class="icon === name_ ? 'bg-selected text-foreground ring-2 ring-ring' : ''"
            @click="icon = name_"
          >
            <RIcon :icon="{ kind: 'lucide', value: name_ }" :size="16" />
          </button>
        </div>
      </div>

      <div class="flex flex-col gap-2">
        <span class="text-13 text-muted-foreground">How often</span>
        <div class="flex items-center gap-3">
          <ToggleGroup type="single" :model-value="cadence" :spacing="1" @update:model-value="pickCadence">
            <ToggleGroupItem v-for="option in CADENCES" :key="option.value" :value="option.value" size="sm" class="text-xs">
              {{ option.label }}
            </ToggleGroupItem>
          </ToggleGroup>
          <template v-if="cadence !== 'daily'">
            <Input
              id="habit-times"
              type="number"
              class="w-16"
              min="1"
              :max="timesMax(cadence)"
              :model-value="times"
              aria-label="Times"
              @update:model-value="onTimes"
            />
            <Label for="habit-times" class="text-13 text-muted-foreground">{{ timesLabel }}</Label>
          </template>
        </div>
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
