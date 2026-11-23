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
import { useNotice } from '@/composables/useNotice'
import { useResource } from '@/composables/useResource'
import { useScreen } from '@/composables/useScreen'
import { useSearch } from '@/composables/useSearch'
import { api } from '@/data'
import { mockControl } from '@/data/mock/control'
import type { GameModeChoice, IconRef } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { keepFocus } from '@/lib/focus'
import { lucideIcon } from '@/lib/icon-map'
import { useNav } from '@/nav/useNav'
import { shell } from '@/shell'
import WindDownRow from './WindDownRow.vue'

const nav = useNav()
const notice = useNotice()
const search = useSearch()

const mode = useResource('gameMode', () => api.getGameMode(), { live: 'gameMode' })
const games = useResource('games', () => api.listGames())

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

// Going to the route does not start game mode (a game's "Play in game mode" already did).
onMounted(async () => {
  await mode.reload()
  fresh.value = true
  if (mode.data.value?.phase === 'off') await start()
})

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
  return { id, label, icon, slot, run: ({ api: rainApi }) => void rainApi.resolveGameMode(choice) }
}

const restart: RainAction = {
  id: 'restart',
  label: 'Restart prototype',
  icon: 'rotate-ccw',
  slot: 'primary',
  run({ nav: rainNav }) {
    mockControl.reset()
    rainNav.go('home')
  },
}

const actions = computed<RainAction[]>(() => {
  const current = state.value
  if (!current) return []
  switch (current.phase) {
    case 'review':
      return [
        ...(current.waiting ? [] : [choose('wait', 'Wait for them', 'hourglass', 'primary', 'wait')]),
        choose('stop-anyway', 'Stop anyway', 'octagon-x', 'secondary', 'stopAnyway'),
      ]
    case 'countdown':
      return [choose('close-now', 'Close now', 'power', 'primary', 'closeNow')]
    case 'done':
      return [restart]
    default:
      return []
  }
})

const primaryOverride = computed<RainAction[] | null>(() => (actions.value.length > 0 ? actions.value : null))

useScreen({ target: shallowRef<ActionTarget | null>(null), move: () => false, primaryOverride })

const canCancel = computed(() => phase.value === 'review' || phase.value === 'countdown')

async function cancel() {
  if (await cancelSequence()) nav.back()
}

// ---- what is drawn ----

const interruptCount = computed(
  () => state.value?.items.filter((item) => item.effect === 'interrupts').length ?? 0,
)

function things(count: number): string {
  return count === 1 ? '1 thing' : `${count} things`
}

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
      <ErrorRow v-if="mode.error.value" message="Game mode unavailable" @retry="mode.reload" />

      <div v-else-if="!state" class="flex flex-col items-center gap-6" aria-hidden="true">
        <template v-if="mode.showSkeleton.value">
          <Skeleton class="size-8" />
          <Skeleton class="h-7 w-64" />
          <div class="flex w-full flex-col gap-1">
            <Skeleton v-for="n in 5" :key="n" class="h-10 w-full" />
          </div>
        </template>
      </div>

      <EmptyState
        v-else-if="phase === 'off'"
        icon="circle-power"
        title="Game mode is off"
        hint="It closes Claude sessions, scheduled jobs and idle jobs, then the Rain window."
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

          <p v-if="phase === 'done'" class="max-w-sm text-13 text-muted-foreground">
            Nothing is left running. Open Rain again from the Start menu.
          </p>
          <p v-else-if="phase === 'review'" class="max-w-sm text-13 text-muted-foreground">
            {{ state.waiting ? 'Rain goes on as soon as they are done.' : 'Stopping them now loses unfinished work.' }}
          </p>
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
            v-for="item in state.items"
            :key="item.id"
            :item="item"
            :phase="state.phase"
            :waiting="state.waiting"
          />
        </div>

        <p
          v-if="sequenceRunning && nextGame && nextGameIcon"
          class="flex items-center justify-center gap-2 text-13 text-muted-foreground"
        >
          Then launch
          <RIcon :icon="nextGameIcon" :size="24" :label="nextGame.title" />
          <span class="min-w-0 truncate text-foreground">{{ nextGame.title }}</span>
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
