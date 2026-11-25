<script setup lang="ts">
// The Setup sub-view: plugins, MCP servers, hooks and output styles as names and short facts.
// Nothing here is selectable or opens anything; the backend sends no secret, only these facts.
import { computed } from 'vue'
import EmptyState from '@/components/rain/EmptyState.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import ScreenFrame from '@/components/rain/ScreenFrame.vue'
import SectionHeader from '@/components/rain/SectionHeader.vue'
import { Skeleton } from '@/components/ui/skeleton'
import { useSectionQuery } from '@/composables/useSearch'
import type { Dir } from '@/composables/useScreen'
import { matches } from '@/lib/score'
import { setupCount } from './model'
import { useClaudeLibrary } from './resources'

defineSlots<{ strip(): unknown }>()

const library = useClaudeLibrary()
const query = useSectionQuery('claude')

const hasData = computed(() => library.data.value !== undefined)
const setup = computed(() => library.data.value?.setup)

const plugins = computed(() =>
  (setup.value?.plugins ?? []).filter((plugin) => matches(query.value, plugin.name, [plugin.version ?? ''])),
)
const servers = computed(() =>
  (setup.value?.mcpServers ?? []).filter((server) =>
    matches(query.value, server.name, [server.scope, server.transport ?? '']),
  ),
)
const hooks = computed(() =>
  (setup.value?.hooks ?? []).filter((hook) =>
    matches(query.value, hook.event, [hook.command, hook.matcher ?? '', hook.scope]),
  ),
)
const styles = computed(() =>
  (setup.value?.outputStyles ?? []).filter((style) => matches(query.value, style.name, [style.scope])),
)
const shown = computed(() => plugins.value.length + servers.value.length + hooks.value.length + styles.value.length)
const total = computed(() => (setup.value ? setupCount(setup.value) : 0))

// This view has no list to move in.
defineExpose({
  target: null,
  move: (_dir: Dir) => false,
  page: (_dir: -1 | 1) => undefined,
  group: (_dir: -1 | 1) => undefined,
})
</script>

<template>
  <ScreenFrame>
    <template #strip><slot name="strip" /></template>

    <ErrorRow
      v-if="library.error.value"
      class="m-2"
      message="Claude library unavailable"
      :detail="library.error.value.message"
      @retry="library.reload"
    />
    <div v-if="!hasData && library.showSkeleton.value" class="flex flex-col gap-1 p-2" aria-hidden="true">
      <Skeleton v-for="row in 6" :key="row" class="h-8 rounded-lg" />
    </div>
    <EmptyState
      v-else-if="hasData && library.data.value?.found === false"
      icon="library"
      title="No Claude folder found"
      hint="Claude Code has not run on this PC yet, so there is nothing to show"
    />
    <EmptyState
      v-else-if="hasData && total === 0"
      icon="wrench"
      title="Nothing set up"
      hint="No plugins, MCP servers, hooks or output styles were found"
    />
    <EmptyState v-else-if="hasData && shown === 0" icon="search-x" title="No match" />

    <div v-else-if="hasData" class="flex flex-col gap-4 p-2">
      <section v-if="plugins.length > 0" aria-label="Plugins">
        <SectionHeader title="Plugins" :count="plugins.length" />
        <div
          v-for="plugin in plugins"
          :key="plugin.name"
          class="flex h-8 items-center gap-3 rounded-lg px-2 text-13 hover:bg-hover"
        >
          <span class="min-w-0 flex-1 truncate text-foreground" :class="{ 'opacity-60': plugin.enabled === false }">{{ plugin.name }}</span>
          <span v-if="plugin.enabled === false" class="text-xs text-text-3">Off</span>
          <span v-if="plugin.version" class="font-mono text-xs text-text-3">{{ plugin.version }}</span>
        </div>
      </section>

      <section v-if="servers.length > 0" aria-label="MCP servers">
        <SectionHeader title="MCP servers" :count="servers.length" />
        <div
          v-for="server in servers"
          :key="`${server.scope}:${server.name}`"
          class="flex h-8 items-center gap-3 rounded-lg px-2 text-13 hover:bg-hover"
        >
          <span class="min-w-0 flex-1 truncate text-foreground">{{ server.name }}</span>
          <span v-if="server.transport" class="font-mono text-xs text-text-3">{{ server.transport }}</span>
          <span class="max-w-40 truncate text-xs text-text-3">{{ server.scope }}</span>
        </div>
      </section>

      <section v-if="hooks.length > 0" aria-label="Hooks">
        <SectionHeader title="Hooks" :count="hooks.length" />
        <div
          v-for="(hook, index) in hooks"
          :key="`${hook.scope}:${hook.event}:${index}`"
          class="flex h-8 items-center gap-3 rounded-lg px-2 text-13 hover:bg-hover"
          :title="hook.command"
        >
          <span class="w-36 shrink-0 truncate text-foreground">{{ hook.event }}</span>
          <span v-if="hook.matcher" class="max-w-24 shrink-0 truncate font-mono text-xs text-text-3">{{ hook.matcher }}</span>
          <span class="selectable min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground">{{ hook.command }}</span>
          <span class="max-w-32 truncate text-xs text-text-3">{{ hook.scope }}</span>
        </div>
      </section>

      <section v-if="styles.length > 0" aria-label="Output styles">
        <SectionHeader title="Output styles" :count="styles.length" />
        <div
          v-for="style in styles"
          :key="`${style.scope}:${style.name}`"
          class="flex h-8 items-center gap-3 rounded-lg px-2 text-13 hover:bg-hover"
        >
          <span class="min-w-0 flex-1 truncate text-foreground">{{ style.name }}</span>
          <span class="max-w-40 truncate text-xs text-text-3">{{ style.scope }}</span>
        </div>
      </section>
    </div>
  </ScreenFrame>
</template>
