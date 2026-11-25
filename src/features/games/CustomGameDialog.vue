<script setup lang="ts">
// One dialog for two jobs: add a game that Steam, Epic and Riot do not list (Ctrl+N), and edit a local
// game (Ctrl+K, Edit game). A game found in a game folder can be corrected here as well: its
// program file is only a guess until the user says otherwise.
import { computed, onScopeDispose, ref, watch } from 'vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useNotice } from '@/composables/useNotice'
import { useOverlay } from '@/composables/useOverlay'
import { refreshResources } from '@/composables/useResource'
import { api } from '@/data'
import type { Game, GamePatch } from '@/data/types'
import { errorMessage } from '@/lib/errors'
import { customGameDialog } from './actions'

const open = customGameDialog.open
const editing = customGameDialog.editing
useOverlay(open)
// The flag is module state: leaving the screen must not reopen the dialog on the next visit.
onScopeDispose(() => {
  open.value = false
})
const notice = useNotice()

const title = ref('')
const exePath = ref('')
const coverPath = ref('')
const saving = ref(false)

const canSave = computed(() => title.value.trim() !== '' && exePath.value.trim() !== '' && !saving.value)

/** A game found in a game folder (not added by hand): its title and program can go back to the folder name and the guess. */
const isFolderGame = computed(() => editing.value !== null && editing.value.removable !== true)

// Every opening starts with the fields of the game being edited, or empty ones. An empty cover
// field means "unchanged" when editing.
watch(open, (isOpen) => {
  if (!isOpen) return
  title.value = editing.value?.title ?? ''
  exePath.value = editing.value?.exePath ?? ''
  coverPath.value = ''
})

/** The fields that differ from the game being edited: nothing else is sent. */
function changes(game: Game): GamePatch {
  const patch: GamePatch = {}
  if (title.value.trim() !== game.title) patch.title = title.value.trim()
  if (exePath.value.trim() !== (game.exePath ?? '')) patch.exePath = exePath.value.trim()
  if (coverPath.value.trim() !== '') patch.coverPath = coverPath.value.trim()
  return patch
}

async function send(job: () => Promise<Game>, done: (game: Game) => string, failure: string) {
  saving.value = true
  try {
    const game = await job()
    refreshResources('games', 'pinned', 'recent')
    notice.success(done(game))
    open.value = false
  } catch (reason) {
    // The dialog stays open so that the text can be corrected.
    notice.error(errorMessage(reason, failure))
  } finally {
    saving.value = false
  }
}

async function save() {
  if (!canSave.value) return
  const game = editing.value
  if (game === null) {
    await send(
      () => api.saveCustomGame({
        title: title.value.trim(),
        exePath: exePath.value.trim(),
        coverPath: coverPath.value.trim() || undefined,
      }),
      (added) => `Added ${added.title}`,
      'Could not add the game',
    )
    return
  }
  const patch = changes(game)
  if (Object.keys(patch).length === 0) {
    open.value = false
    return
  }
  await send(() => api.updateGame(game.id, patch), (saved) => `Saved ${saved.title}`, 'Could not save the game')
}

/** One field goes back to its default (no cover, the guessed program). */
async function reset(patch: GamePatch) {
  const game = editing.value
  if (game === null || saving.value) return
  await send(() => api.updateGame(game.id, patch), (saved) => `Saved ${saved.title}`, 'Could not save the game')
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent>
      <DialogHeader>
        <DialogTitle>{{ editing ? 'Edit game' : 'Add a game' }}</DialogTitle>
        <DialogDescription>
          {{ editing ? 'Change what Rain starts for this game.' : 'For a game that Steam, Epic and Riot do not list.' }}
        </DialogDescription>
      </DialogHeader>

      <form class="grid gap-3" @submit.prevent="save">
        <div class="grid gap-1.5">
          <Label for="custom-game-title">Name</Label>
          <Input id="custom-game-title" v-model="title" autocomplete="off" placeholder="Example Game" />
        </div>
        <div class="grid gap-1.5">
          <Label for="custom-game-exe">Executable</Label>
          <Input id="custom-game-exe" v-model="exePath" autocomplete="off" placeholder="D:\Games\Example Game\game.exe" />
          <p v-if="editing?.exeGuessed" class="text-xs text-muted-foreground">
            Rain guessed this program from the folder. Change it if it is wrong.
          </p>
          <button
            v-if="isFolderGame && editing?.exeGuessed === false"
            type="button"
            class="w-fit text-xs text-muted-foreground underline-offset-2 hover:underline"
            @click="reset({ exePath: '' })"
          >
            Use the guess again
          </button>
        </div>
        <div class="grid gap-1.5">
          <Label for="custom-game-cover">Cover image (optional)</Label>
          <Input id="custom-game-cover" v-model="coverPath" autocomplete="off" placeholder="D:\Games\Example Game\cover.png" />
          <button
            v-if="editing?.coverUrl && !editing.coverIsIcon"
            type="button"
            class="w-fit text-xs text-muted-foreground underline-offset-2 hover:underline"
            @click="reset({ coverPath: '' })"
          >
            Remove cover
          </button>
        </div>
        <!-- A hidden submit lets Enter in a field save. -->
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>

      <DialogFooter>
        <Button variant="outline" @click="open = false">Cancel</Button>
        <Button :disabled="!canSave" @click="save">{{ editing ? 'Save' : 'Add game' }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
