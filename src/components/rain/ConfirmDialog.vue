<script setup lang="ts">
// The one confirmation dialog (spec 3.7). Focus starts on Cancel, so Enter never confirms by accident.
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { useConfirmState } from '@/composables/useConfirm'

const { isOpen, options, answer } = useConfirmState()
</script>

<template>
  <AlertDialog :open="isOpen" @update:open="(open) => !open && answer(false)">
    <AlertDialogContent v-if="options">
      <AlertDialogHeader>
        <AlertDialogTitle>{{ options.title }}</AlertDialogTitle>
        <AlertDialogDescription>{{ options.body }}</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>Cancel</AlertDialogCancel>
        <!-- A plain Button, not AlertDialogAction: the store closes the dialog itself,
             and a second close event would resolve the promise as "no". -->
        <Button :variant="options.danger ? 'destructive' : 'default'" @click="answer(true)">
          {{ options.confirmLabel }}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
