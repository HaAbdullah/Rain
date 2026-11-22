<script setup lang="ts">
// Browser-only stand-in for the hidden window (spec 2.7).
import KeyHint from './KeyHint.vue'
</script>

<template>
  <div class="flex h-full items-center justify-center gap-3 bg-background text-13 text-text-3">
    <span>Rain is hidden</span>
    <KeyHint keys="Ctrl+Backquote" />
  </div>
</template>
