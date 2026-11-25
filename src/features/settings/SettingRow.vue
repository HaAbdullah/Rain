<script setup lang="ts">
// One 48 px row of the Settings screen: label and hint on the left, the control on the
// right. Enter on the row asks it to operate its control (settingActivation); a change
// is reported with an event and the screen applies it.
import { computed, ref, useTemplateRef, watch } from 'vue'
import type { SettingDef } from '@/actions/types'
import KeyHint from '@/components/rain/KeyHint.vue'
import StatusDot from '@/components/rain/StatusDot.vue'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { useOverlay } from '@/composables/useOverlay'
import { useSearch } from '@/composables/useSearch'
import { keepFocus } from '@/lib/focus'
import ThemePicker from './ThemePicker.vue'
import ZoomControl from './ZoomControl.vue'
import { settingActivation } from './actions'
import type { SettingValue } from './settingValues'
import { selectChoices } from './settingDefs'
import type { RowState } from './rowState'
import { usePathCheck } from './usePathCheck'

const props = defineProps<{
  def: SettingDef
  selected: boolean
  value: SettingValue
  /** The library path beside a Steam, Epic or Riot switch. */
  path?: string
  /** Set when the row's setting is not working; shown as a failed glyph with this text. */
  problem?: string
  /** What the backend says about this row's setting. Its text replaces the hint and its glyph sits before the control. */
  rowState?: RowState
}>()

const emit = defineEmits<{
  change: [value: SettingValue]
  changePath: [path: string]
}>()

const search = useSearch()
const root = useTemplateRef<HTMLElement>('root')

// The Appearance row shows the theme picker where a Select row shows its Select. The theme is
// kept by the window (useTheme), so the row neither reads nor reports a value.
const isTheme = computed(() => props.def.id === 'theme')
const picker = useTemplateRef<InstanceType<typeof ThemePicker>>('picker')
// The Zoom row is the same: the window keeps the level (useZoom).
const isZoom = computed(() => props.def.id === 'zoom')
const zoomControl = useTemplateRef<InstanceType<typeof ZoomControl>>('zoomControl')

const selectOpen = ref(false)
useOverlay(selectOpen)

// ---- path and text fields ----

/** The text of the row's field: its own value for a path or text row, else the companion path. */
const fieldValue = computed(() => (props.def.kind === 'switch' ? (props.path ?? '') : String(props.value)))
const hasField = computed(() => props.def.kind === 'path' || props.def.kind === 'text' || props.path !== undefined)
const isPathField = computed(() => props.def.kind === 'path' || props.path !== undefined)

const draft = ref(fieldValue.value)
// The text last sent. fieldValue only catches up after the save, so it cannot tell that
// Enter and the blur it causes belong to one edit.
let sent = fieldValue.value
watch(fieldValue, (text) => {
  draft.value = text
  sent = text
})

/** An empty field is "not set", not a mistake, so it has no glyph and shows its placeholder. */
const hasPathGlyph = computed(() => isPathField.value && draft.value.trim() !== '')

/** Whether every folder of the field exists: the backend answers (api.checkPaths); undefined until it has. */
const pathFound = usePathCheck(draft, isPathField)

function commitField() {
  if (draft.value === sent) return
  sent = draft.value
  if (props.def.kind === 'switch') emit('changePath', draft.value)
  else emit('change', draft.value)
}

/** Enter keeps the edit and puts the caret back in the search field. */
function endEdit() {
  commitField()
  search.focus()
}

// ---- Enter on the row ----

watch(settingActivation, (activation) => {
  if (activation?.def.id !== props.def.id) return
  if (props.def.kind === 'switch') emit('change', props.value !== true)
  else if (isTheme.value) picker.value?.focus()
  else if (isZoom.value) zoomControl.value?.focus()
  else if (props.def.kind === 'select') selectOpen.value = true
  else root.value?.querySelector('input')?.focus()
})
</script>

<template>
  <div
    ref="root"
    class="flex min-h-12 items-center gap-4 rounded-lg px-2 py-1 transition-colors"
    :class="selected ? 'bg-selected' : 'hover:bg-hover'"
  >
    <div class="min-w-0 flex-1">
      <p class="truncate text-sm text-foreground">{{ def.label }}</p>
      <p v-if="rowState" class="truncate text-xs text-muted-foreground">{{ rowState.text }}</p>
      <p v-else-if="def.hint" class="truncate text-xs text-muted-foreground">{{ def.hint }}</p>
    </div>

    <div class="flex shrink-0 items-center gap-2" @mousedown="keepFocus">
      <template v-if="hasField">
        <Input
          :model-value="draft"
          :aria-label="def.label"
          :placeholder="def.placeholder"
          :class="def.kind === 'text' ? 'w-40' : 'w-72'"
          :spellcheck="false"
          class="font-mono text-xs md:text-xs"
          @update:model-value="draft = String($event)"
          @change="commitField"
          @keydown.enter="endEdit"
        />
        <span v-if="hasPathGlyph" class="flex size-4 items-center justify-center">
          <StatusDot
            v-if="pathFound !== undefined"
            :status="pathFound ? 'success' : 'failed'"
            :label="pathFound ? 'Found' : 'Missing'"
          />
        </span>
      </template>

      <StatusDot v-if="rowState" :status="rowState.status" :label="rowState.text" />

      <ThemePicker v-if="isTheme" ref="picker" />

      <ZoomControl v-else-if="isZoom" ref="zoomControl" />

      <Switch
        v-else-if="def.kind === 'switch'"
        :model-value="value === true"
        :aria-label="def.label"
        tabindex="-1"
        @update:model-value="emit('change', $event)"
      />

      <Select
        v-else-if="def.kind === 'select'"
        v-model:open="selectOpen"
        :model-value="String(value)"
        @update:model-value="emit('change', String($event))"
      >
        <SelectTrigger class="w-28" :aria-label="def.label" tabindex="-1">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem v-for="choice in selectChoices[def.id]" :key="choice.value" :value="choice.value">
            {{ choice.label }}
          </SelectItem>
        </SelectContent>
      </Select>

      <template v-else-if="def.kind === 'info'">
        <StatusDot v-if="problem" status="failed" :label="problem" />
        <KeyHint v-if="def.key === 'hotkey'" :keys="String(value)" />
        <span v-else class="text-13 text-muted-foreground">{{ value }}</span>
      </template>
    </div>
  </div>
</template>
