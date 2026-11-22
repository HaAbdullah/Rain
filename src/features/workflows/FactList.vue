<script setup lang="ts">
// Label and value pairs of a detail pane: put <dt> and <dd> pairs in the slot.
// A <dd> stays a block so a long value ends in an ellipsis; rows that need a control wrap it themselves.
</script>

<template>
  <dl
    class="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-2 text-13 [&_dd]:min-w-0 [&_dd]:truncate [&_dd]:text-foreground [&_dt]:text-muted-foreground"
  >
    <slot />
  </dl>
</template>
