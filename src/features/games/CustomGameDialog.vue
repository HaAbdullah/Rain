<script setup lang="ts">
// Dialog for adding a game that Steam and Epic do not know (Ctrl+N). The mock only stores it.
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
import { errorMessage } from '@/lib/errors'
import { customGameDialog } from './actions'

const open = customGameDialog.open
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

// Every opening starts with an empty form.
watch(open, (isOpen) => {
  if (!isOpen) return
  title.value = ''
  exePath.value = ''
  coverPath.value = ''
})

async function save() {
  if (!canSave.value) return
  saving.value = true
  try {
    const game = await api.saveCustomGame({
      title: title.value.trim(),
      exePath: exePath.value.trim(),
      coverPath: coverPath.value.trim() || undefined,
    })
    refreshResources('games')
    notice.success(`Added ${game.title}`)
    open.value = false
  } catch (reason) {
    notice.error(errorMessage(reason, 'Could not add the game'))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent>
      <DialogHeader>
        <DialogTitle>Add a game</DialogTitle>
        <DialogDescription>For a game that Steam and Epic do not list.</DialogDescription>
      </DialogHeader>

      <form class="grid gap-3" @submit.prevent="save">
        <div class="grid gap-1.5">
          <Label for="custom-game-title">Name</Label>
          <Input id="custom-game-title" v-model="title" autocomplete="off" placeholder="Celeste" />
        </div>
        <div class="grid gap-1.5">
          <Label for="custom-game-exe">Executable</Label>
          <Input id="custom-game-exe" v-model="exePath" autocomplete="off" placeholder="C:\Games\Celeste\Celeste.exe" />
        </div>
        <div class="grid gap-1.5">
          <Label for="custom-game-cover">Cover image (optional)</Label>
          <Input id="custom-game-cover" v-model="coverPath" autocomplete="off" placeholder="C:\Games\Celeste\cover.png" />
        </div>
        <!-- A hidden submit lets Enter in a field save. -->
        <button type="submit" class="hidden" tabindex="-1" aria-hidden="true" />
      </form>

      <DialogFooter>
        <Button variant="outline" @click="open = false">Cancel</Button>
        <Button :disabled="!canSave" @click="save">Add game</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
