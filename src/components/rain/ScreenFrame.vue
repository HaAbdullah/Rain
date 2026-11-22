<script setup lang="ts">
// The layout every screen puts its content in: an optional 40 px strip on top,
// the main area, and an optional fixed-width detail pane on the right.
withDefaults(
  defineProps<{
    /** 'md' is 340 px, 'wide' is 420 px. There is no splitter and no pane toggle. */
    pane?: 'none' | 'md' | 'wide'
  }>(),
  { pane: 'none' },
)
</script>

<template>
  <div class="flex h-full min-h-0 w-full flex-col">
    <div
      v-if="$slots.strip"
      class="flex h-(--strip-h) min-w-0 shrink-0 items-center border-b border-border"
    >
      <slot name="strip" />
    </div>

    <div class="flex min-h-0 flex-1">
      <!--
        A flex column that scrolls: content that sizes itself (an ItemGrid, a settings column)
        scrolls here, and content that fills the height (an ItemList) scrolls on its own.
      -->
      <div class="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
        <slot />
      </div>

      <aside
        v-if="pane !== 'none'"
        class="min-h-0 shrink-0 overflow-hidden border-l border-border bg-surface-1"
        :class="pane === 'wide' ? 'w-(--pane-w-wide)' : 'w-(--pane-w)'"
      >
        <slot name="pane" />
      </aside>
    </div>
  </div>
</template>
