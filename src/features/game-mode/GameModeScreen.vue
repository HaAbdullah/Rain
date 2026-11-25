<script setup lang="ts">
// Game mode (spec 7.8): review what will be stopped, count down, wind down, done.
// The screen holds no timer; it draws the GameModeState the api pushes, and hands the
// phase's Enter / Ctrl+Enter actions to the shell through primaryOverride.
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { runAction } from '@/actions/context'
import type { ActionTarget, RainAction } from '@/actions/types'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import RIcon from '@/components/rain/RIcon.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { formatBytes } from '@/lib/format'
import { useNotice } from '@/composables/useNotice'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSearch } from '@/composables/useSearch'
import { api } from '@/data'
import type { GameModeChoice, IconRef } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { keepFocus } from '@/lib/focus'
import { lucideIcon } from '@/lib/icon-map'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'
import OtherAppsList from './OtherAppsList.vue'
import WindDownRow from './WindDownRow.vue'

const nav = useNav()
const notice = useNotice()
const search = useSearch()

const mode = useResource('gameMode', () => api.getGameMode(), { live: 'gameMode' })
const games = useResource('games', () => api.listGames())
// The programs that game mode would close. Not a topic: asking for it looks at every process,
// so only this screen asks, and only when it opens and when a run begins.
const apps = useResource('gameModeApps', () => api.getOtherApps())

// The cache may still hold the phase of an earlier visit; nothing is drawn from it until
// the fetch made on mount has replaced it.
const fresh = ref(false)
const state = computed(() => (fresh.value ? mode.data.value : undefined))
const phase = computed(() => state.value?.phase ?? 'off')

async function start() {
  try {
    await api.startGameMode()
  } catch (reason) {
    notice.error(errorMessage(reason, 'Game mode did not start'))
  }
}

// ---- starting and leaving ----

/** True while the list waits for the user to look at it and press Start (nothing runs yet). */
const previewing = ref(false)

/** Programs that would be closed now. With none, there is nothing to look at. */
const hasWork = computed(() => {
  const list = apps.data.value
  return !!list && list.enabled && list.apps.some((app) => app.close)
})

// Going to the route does not start game mode (a game's "Play in game mode" already did).
// With programs to close, the list comes first and the user starts; without, it starts at once.
onMounted(async () => {
  await mode.reload()
  fresh.value = true
  if (mode.data.value?.phase !== 'off') {
    void apps.reload()
    return
  }
  await apps.reload()
  if (hasWork.value) previewing.value = true
  else await start()
})

// A run that begins (from the button, or from a game) has looked at the PC again.
watch(
  () => state.value?.phase,
  (now, before) => {
    if ((now === 'review' || now === 'countdown') && before !== 'review' && before !== 'countdown') {
      previewing.value = false
      void apps.reload()
    }
  },
)

async function setEnabled(value: boolean) {
  try {
    apps.data.value = await api.updateOtherApps({ enabled: value })
  } catch (reason) {
    notice.error(errorMessage(reason, 'The switch was not saved'))
  }
}

async function setClose(id: string, close: boolean) {
  try {
    apps.data.value = await api.updateOtherApps({ app: { id, close } })
  } catch (reason) {
    notice.error(errorMessage(reason, 'The choice was not saved'))
  }
}

/** Cancels the sequence; false when the api refused, after telling the user. */
async function cancelSequence(): Promise<boolean> {
  try {
    await api.resolveGameMode('cancel')
    notice.info('Game mode cancelled')
    return true
  } catch (reason) {
    notice.error(errorMessage(reason, 'Game mode was not cancelled'))
    return false
  }
}

// ---- search field and window while the sequence runs ----

const sequenceRunning = computed(() => ['review', 'countdown', 'windingDown'].includes(phase.value))

watch(
  sequenceRunning,
  (running, _before, onCleanup) => {
    search.disabled.value = running
    if (!running) return
    onCleanup(shell.holdOpen('game-mode'))
  },
  { immediate: true },
)

// Leaving with Ctrl+digit cancels. Esc has cancelled already, so the phase is 'off' by then.
onBeforeUnmount(() => {
  search.disabled.value = false
  if (phase.value === 'review' || phase.value === 'countdown') void cancelSequence()
})

// ---- actions ----

function choose(
  id: string,
  label: string,
  icon: string,
  slot: 'primary' | 'secondary',
  choice: GameModeChoice,
): RainAction {
  // The action awaits, so a refusal of the backend becomes a notice through runAction.
  return { id, label, icon, slot, run: async ({ api: rainApi }) => { await rainApi.resolveGameMode(choice) } }
}

const startAction: RainAction = {
  id: 'start-game-mode',
  label: 'Start game mode',
  icon: 'circle-power',
  slot: 'primary',
  run: async ({ api: rainApi }) => {
    await rainApi.startGameMode()
    previewing.value = false
  },
}

const actions = computed<RainAction[]>(() => {
  const current = state.value
  if (!current) return []
  switch (current.phase) {
    case 'off':
      return previewing.value ? [startAction] : []
    case 'review':
      return [
        ...(current.waiting ? [] : [choose('wait', 'Wait for them', 'hourglass', 'primary', 'wait')]),
        choose('stop-anyway', 'Stop anyway', 'octagon-x', 'secondary', 'stopAnyway'),
      ]
    case 'countdown':
      return [choose('close-now', 'Close now', 'power', 'primary', 'closeNow')]
    default:
      return []
  }
})

const primaryOverride = computed<RainAction[] | null>(() => (actions.value.length > 0 ? actions.value : null))

useScreen({ target: shallowRef<ActionTarget | null>(null), move: () => false, primaryOverride })

const canCancel = computed(
  () => phase.value === 'review' || phase.value === 'countdown' || (phase.value === 'off' && previewing.value),
)

async function cancel() {
  if (phase.value === 'off') nav.back()
  else if (await cancelSequence()) nav.back()
}

// ---- what is drawn ----

const interruptCount = computed(
  () => state.value?.items.filter((item) => item.effect === 'interrupts').length ?? 0,
)

function things(count: number): string {
  return count === 1 ? '1 thing' : `${count} things`
}

/** Open Claude sessions are never stopped; the done screen says that they are still running. */
const leftRunning = computed(() => state.value?.items.find((item) => item.effect === 'leftRunning')?.count ?? 0)
const doneHint = computed(() =>
  leftRunning.value > 0
    ? 'Your Claude sessions are still running. Open Rain again from the Start menu.'
    : 'Open Rain again from the Start menu.',
)
const reviewHint = computed(() => {
  if (state.value?.waiting) return 'Rain goes on as soon as they are done.'
  return state.value?.waitTimedOut
    ? 'They did not finish in 3 minutes. Stopping them now loses unfinished work.'
    : 'Stopping them now loses unfinished work.'
})

/** The checklist leaves out the row of the other programs when there are none to close. */
const rows = computed(() =>
  (state.value?.items ?? []).filter((item) => item.kind !== 'otherApp' || item.count > 0 || phase.value === 'windingDown'),
)

/** What the preview says it frees, in one line. */
const previewHint = computed(() => {
  const list = apps.data.value
  return list && list.freedBytes > 0
    ? `Frees about ${formatBytes(list.freedBytes)}. Apps with open windows are asked first.`
    : 'Apps with open windows are asked first.'
})

const heading = computed(() => {
  if (phase.value === 'review') {
    return state.value?.waiting
      ? `Waiting for ${things(interruptCount.value)} to finish`
      : `${things(interruptCount.value)} will be interrupted`
  }
  if (phase.value === 'windingDown') return 'Winding down'
  return 'Rain is off'
})

/** The game that starts after Rain has closed everything, if game mode was started from a game. */
const nextGame = computed(() =>
  games.data.value?.games.find((game) => game.id === state.value?.thenLaunchGameId),
)
const nextGameIcon = computed<IconRef | null>(() => {
  const game = nextGame.value
  if (!game) return null
  return game.coverUrl ? { kind: 'url', value: game.coverUrl } : { kind: 'letter', value: game.title }
})

/** Share of the countdown still left, 100 down to 0. */
const remaining = computed(() => {
  const current = state.value
  return current && current.countdownSeconds > 0
    ? (current.secondsLeft / current.countdownSeconds) * 100
    : 0
})
</script>

<template>
  <ScreenFrame>
    <div class="mx-auto my-auto flex w-full max-w-120 flex-col gap-6 px-4 py-8">
      <ErrorRow v-if="mode.error.value" message="Game mode unavailable" :detail="mode.error.value.message" @retry="mode.reload" />

      <div v-else-if="!state" class="flex flex-col items-center gap-6" aria-hidden="true">
        <template v-if="mode.showSkeleton.value">
          <Skeleton class="size-8" />
          <Skeleton class="h-7 w-64" />
          <div class="flex w-full flex-col gap-1">
            <Skeleton v-for="n in 5" :key="n" class="h-10 w-full" />
          </div>
        </template>
      </div>

      <template v-else-if="phase === 'off' && previewing && apps.data.value">
        <div class="flex flex-col items-center gap-2 text-center">
          <component
            :is="lucideIcon('circle-power')"
            :size="32"
            :stroke-width="1.75"
            class="mb-2 text-muted-foreground"
            aria-hidden="true"
          />
          <p class="text-xl font-semibold text-foreground">Ready for game mode</p>
          <p class="max-w-sm text-13 text-muted-foreground">{{ previewHint }}</p>
        </div>
        <OtherAppsList :list="apps.data.value" editable @enabled="setEnabled" @close="setClose" />
        <div class="flex flex-wrap items-center justify-center gap-2">
          <Button class="gap-2" tabindex="-1" @mousedown="keepFocus" @click="runAction(startAction)">
            {{ startAction.label }}
            <KeyHint keys="Enter" />
          </Button>
          <Button variant="ghost" class="gap-2" tabindex="-1" @mousedown="keepFocus" @click="cancel">
            Cancel
            <KeyHint keys="Escape" />
          </Button>
        </div>
      </template>

      <EmptyState
        v-else-if="phase === 'off'"
        icon="circle-power"
        title="Game mode is off"
        hint="It closes your other apps, stops Rain's jobs and closes the Rain window. Your Claude sessions keep running."
        action-label="Start game mode"
        @action="start"
      />

      <template v-else>
        <div class="flex flex-col items-center gap-2 text-center">
          <component
            :is="lucideIcon('circle-power')"
            :size="32"
            :stroke-width="1.75"
            class="mb-2 text-muted-foreground"
            aria-hidden="true"
          />

          <p v-if="phase === 'countdown'" class="flex items-baseline gap-2 text-xl font-semibold text-foreground">
            Closing in
            <span class="text-countdown tabular-nums">{{ state.secondsLeft }}</span>
            s
          </p>
          <p v-else class="text-xl font-semibold text-foreground">{{ heading }}</p>

          <p v-if="phase === 'done'" class="max-w-sm text-13 text-muted-foreground">{{ doneHint }}</p>
          <p v-else-if="phase === 'review'" class="max-w-sm text-13 text-muted-foreground">{{ reviewHint }}</p>
        </div>

        <!-- Drawn here by width: the shared Progress slides its indicator with a transform (spec 5.5). -->
        <div
          v-if="phase === 'countdown'"
          class="h-1 w-full overflow-hidden rounded-full bg-muted"
          role="progressbar"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-valuenow="Math.round(remaining)"
        >
          <div class="h-full rounded-full bg-primary" :style="{ width: `${remaining}%` }" />
        </div>

        <div
          v-if="sequenceRunning"
          class="flex flex-col divide-y divide-border rounded-lg border border-border bg-surface-1"
        >
          <WindDownRow
            v-for="item in rows"
            :key="item.id"
            :item="item"
            :phase="state.phase"
            :waiting="state.waiting"
          />
        </div>

        <OtherAppsList
          v-if="(phase === 'review' || phase === 'countdown') && apps.data.value"
          :list="apps.data.value"
          editable
          @enabled="setEnabled"
          @close="setClose"
        />

        <p
          v-if="sequenceRunning && nextGame && nextGameIcon"
          class="flex items-center justify-center gap-2 text-13 text-muted-foreground"
        >
          Then launch
          <RIcon :icon="nextGameIcon" :size="24" :label="nextGame.title" />
          <span class="min-w-0 truncate text-foreground">{{ nextGame.title }}</span>
        </p>
        <p v-if="state.launchError" class="text-center text-13 text-status-failed">
          The game did not start: {{ state.launchError }}
        </p>

        <div v-if="actions.length > 0 || canCancel" class="flex flex-wrap items-center justify-center gap-2">
          <Button
            v-for="action in actions"
            :key="action.id"
            :variant="action.slot === 'primary' ? 'default' : 'outline'"
            class="gap-2"
            tabindex="-1"
            @mousedown="keepFocus"
            @click="runAction(action)"
          >
            {{ action.label }}
            <KeyHint :keys="action.slot === 'primary' ? 'Enter' : 'Ctrl+Enter'" />
          </Button>
          <Button v-if="canCancel" variant="ghost" class="gap-2" tabindex="-1" @mousedown="keepFocus" @click="cancel">
            Cancel
            <KeyHint keys="Escape" />
          </Button>
        </div>
      </template>
    </div>
  </ScreenFrame>
</template>
