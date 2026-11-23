<script setup lang="ts">
// Settings (spec 7.9): one 640 px column of rows, grouped. No Save button: a change is
// sent to the api at once. The rows are selectable with the shared selection, so Enter,
// Ctrl+Enter and Ctrl+K act on the selected row (see actions.ts).
import { computed, useTemplateRef } from 'vue'
import type { ActionTarget, SettingDef } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import SectionHeader from '@/components/rain/SectionHeader.vue'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { useNotice } from '@/composables/useNotice'
import { refreshResources, useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSectionQuery } from '@/composables/useSearch'
import { useSelection, type SelGroup } from '@/composables/useSelection'
import { api } from '@/data'
import { mockControl } from '@/data/mock/control'
import type { Settings, SettingsPatch } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { keepFocus } from '@/lib/focus'
import { useRevealSelected } from '@/lib/reveal'
import { matches } from '@/lib/score'
import StatusDot from '@/components/rain/StatusDot.vue'
import SettingRow from './SettingRow.vue'
import { configIssueText, monitorClaudeRowState, type RowState } from './rowState'
import { settingDefs } from './settingDefs'
import {
  libraryPathPatch,
  patchFor,
  readLibraryPath,
  readValue,
  type SettingValue,
} from './settingValues'

const SKELETON_GROUPS = [4, 6, 1] as const

const notice = useNotice()
const query = useSectionQuery('settings')
const resource = useResource('settings', () => api.getSettings(), { live: 'settings' })
const settings = computed(() => resource.data.value)

// Config files the backend could not read (it keeps the last good values). A failed health
// call adds nothing: Settings itself already shows its own error row.
const health = useResource('health', () => api.getHealth(), { live: 'health' })
const configIssues = computed(() => health.data.value?.configIssues ?? [])

// ---- rows, grouped, filtered by label ----

const groups = computed<SelGroup<SettingDef>[]>(() => {
  const shown = settingDefs.filter((def) => matches(query.value, def.label, [def.group]))
  const titles = [...new Set(shown.map((def) => def.group))]
  return titles.map((title) => ({ id: title, title, items: shown.filter((def) => def.group === title) }))
})

const { selectedId, selected, move, page, group, select } = useSelection(groups, { resetOn: query })

useScreen({
  target: computed<ActionTarget | null>(() =>
    selected.value && settings.value
      ? { type: 'setting', id: selected.value.id, item: selected.value }
      : null,
  ),
  move,
  page,
  group,
})

const column = useTemplateRef<HTMLElement>('column')
useRevealSelected(
  () => column.value,
  () => groups.value,
  () => selectedId.value,
)

// ---- changes ----

async function save(patch: SettingsPatch | null, label: string) {
  if (!patch) return
  try {
    await api.updateSettings(patch)
    refreshResources('settings')
    // Life must never show the data of the old vault.
    if ('vaultPath' in patch || 'dailyNoteFolder' in patch || 'dailyNoteFormat' in patch) {
      refreshResources('vault', 'habits', 'habitDays')
    }
    if ('dataDir' in patch) notice.info('Data folder changed. Rain is restarting its service.')
  } catch (reason) {
    notice.error(errorMessage(reason, `${label} was not saved`))
  }
}

function change(def: SettingDef, value: SettingValue) {
  if (def.id === 'mock-scenario') {
    // The select only offers valid scenarios.
    if (value === 'typical' || value === 'empty' || value === 'busy' || value === 'failing') {
      mockControl.setScenario(value)
    }
  } else if (def.id === 'mock-latency') {
    mockControl.setLatency(value === true)
  } else {
    void save(patchFor(def, value), def.label)
  }
}

function press(def: SettingDef) {
  if (def.id === 'mock-reset') mockControl.reset()
}

/** What the backend says about a row's setting, if it says anything. */
function stateOf(def: SettingDef, current: Settings): RowState | undefined {
  return def.key === 'monitorClaude' ? monitorClaudeRowState(current.monitorClaudeState) : undefined
}

/** Why a row's setting is not working, if it is not. */
function problemOf(def: SettingDef, current: Settings): string | undefined {
  return def.id === 'hotkey' && !current.hotkeyAvailable ? 'Another app already holds this chord' : undefined
}
</script>

<template>
  <ScreenFrame>
    <div ref="column" class="p-6" @mousedown="keepFocus">
      <div class="mx-auto w-full max-w-160">
        <ErrorRow v-if="resource.error.value" message="Settings unavailable" @retry="resource.reload" />

        <div v-else-if="!settings" class="flex flex-col gap-6" aria-hidden="true">
          <template v-if="resource.showSkeleton.value">
            <div v-for="(rows, g) in SKELETON_GROUPS" :key="g" class="flex flex-col gap-1">
              <Skeleton class="mb-1 h-4 w-20" />
              <Skeleton v-for="n in rows" :key="n" class="h-12 w-full" />
            </div>
          </template>
        </div>

        <EmptyState v-else-if="groups.length === 0" icon="settings" title="No setting matches" />

        <template v-else>
          <p
            v-for="issue in configIssues"
            :key="`${issue.file}:${issue.line ?? ''}`"
            class="mb-3 flex items-center gap-2 px-2 text-13 text-muted-foreground"
            :title="issue.message"
          >
            <StatusDot status="failed" :label="issue.message" />
            <span class="min-w-0 truncate">{{ configIssueText(issue) }}. Rain keeps the last good values.</span>
          </p>
          <div v-for="(entry, g) in groups" :key="entry.id" data-group>
            <Separator v-if="g > 0" class="my-3" />
            <SectionHeader :title="entry.title ?? entry.id" data-group-header />
            <div
              v-for="(def, i) in entry.items"
              :key="def.id"
              :data-item-id="def.id"
              :data-group-start="i === 0 ? '' : undefined"
              @click="select(def.id)"
            >
              <SettingRow
                :def="def"
                :selected="def.id === selectedId"
                :value="readValue(def, settings)"
                :path="readLibraryPath(def, settings)"
                :problem="problemOf(def, settings)"
                :row-state="stateOf(def, settings)"
                @change="change(def, $event)"
                @change-path="save(libraryPathPatch(def, $event), def.label)"
                @press="press(def)"
              />
            </div>
          </div>
        </template>
      </div>
    </div>
  </ScreenFrame>
</template>
