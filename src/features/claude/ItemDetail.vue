<script setup lang="ts">
// The pane for the selected file: its facts (the front matter and the file's own), the text drawn
// as readable markdown, and the three buttons. The text is read from the backend when a file is
// selected, never before.
import { ref, watch } from 'vue'
import DetailPane from '@/components/rain/DetailPane.vue'
import ErrorRow from '@/components/rain/ErrorRow.vue'
import KeyHint from '@/components/rain/KeyHint.vue'
import PathText from '@/components/rain/PathText.vue'
import RelTime from '@/components/rain/RelTime.vue'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { api } from '@/data'
import type { LibraryItem, LibraryText } from '@/data/types'
import { toRainError } from '@/data/rest/errors'
import { keepFocus } from '@/lib/focus'
import { formatBytes } from '@/lib/format'
import FactList from './FactList.vue'
import MarkdownBody from './MarkdownBody'
import { KIND_ICON, KIND_LABEL, factLabel } from './model'
import { useRunAction } from './useRunAction'

/** The shadcn Button presses down with a small transform; the buttons of a pane switch it off (spec 5.5). */
const NO_PRESS = 'active:not-aria-[haspopup]:translate-y-0'
/** The facts that the pane shows in its own way, so the list of front matter leaves them out. */
const SHOWN_ELSEWHERE = ['name', 'description']

const props = defineProps<{ item: LibraryItem }>()

const run = useRunAction()
const text = ref<LibraryText | null>(null)
const failure = ref<string | null>(null)
const loading = ref(false)
let latest = 0

async function load() {
  const mine = ++latest
  loading.value = true
  failure.value = null
  try {
    const answer = await api.getClaudeLibraryItem(props.item.id)
    if (mine === latest) text.value = answer
  } catch (reason) {
    if (mine === latest) {
      text.value = null
      failure.value = toRainError(reason).message
    }
  } finally {
    if (mine === latest) loading.value = false
  }
}

// A new file, or the same file changed on disk (the list was read again): read the text again.
watch(() => [props.item.id, props.item.modifiedAt], load, { immediate: true })

function frontMatter(): { key: string; label: string; value: string }[] {
  return (text.value?.facts ?? [])
    .filter((fact) => !SHOWN_ELSEWHERE.includes(fact.key) && fact.value !== '')
    .map((fact) => ({ key: fact.key, label: factLabel(fact.key), value: fact.value }))
}
</script>

<template>
  <DetailPane :title="item.name" :icon="{ kind: 'lucide', value: KIND_ICON[item.kind] }">
    <div class="flex flex-col gap-4">
      <p v-if="item.description" class="selectable text-13 text-muted-foreground">{{ item.description }}</p>

      <FactList>
        <dt>Kind</dt>
        <dd>{{ item.memoryType ? `${KIND_LABEL[item.kind]}, ${item.memoryType}` : KIND_LABEL[item.kind] }}</dd>
        <dt>Scope</dt>
        <dd>{{ item.scope }}</dd>
        <dt>File</dt>
        <dd><PathText :path="item.path" /></dd>
        <template v-if="item.modifiedAt">
          <dt>Changed</dt>
          <dd><RelTime :iso="item.modifiedAt" /></dd>
        </template>
        <dt>Size</dt>
        <dd>{{ formatBytes(item.size) }}</dd>
        <template v-for="fact in frontMatter()" :key="fact.key">
          <dt class="capitalize">{{ fact.label }}</dt>
          <dd class="selectable" :title="fact.value">{{ fact.value }}</dd>
        </template>
      </FactList>

      <ErrorRow v-if="failure" message="Could not read the file" :detail="failure" @retry="load" />

      <div v-else-if="loading && !text" class="flex flex-col gap-2" aria-hidden="true">
        <Skeleton class="h-4 w-1/3" />
        <Skeleton class="h-4 w-full" />
        <Skeleton class="h-4 w-5/6" />
      </div>

      <section v-else-if="text" class="flex flex-col gap-2 border-t border-border pt-4" aria-label="Text">
        <MarkdownBody v-if="text.body.trim() !== ''" :source="text.body" />
        <p v-else class="text-13 text-muted-foreground">The file has no text below its front matter</p>
        <p v-if="text.truncated" class="text-xs text-text-3">The file is long: only the first 512 KB are shown</p>
      </section>
    </div>

    <template #actions>
      <Button variant="outline" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('open')">
        Open in editor
        <KeyHint keys="Enter" />
      </Button>
      <Button variant="ghost" class="gap-2" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('reveal')">
        Reveal in Explorer
        <KeyHint keys="Ctrl+Enter" />
      </Button>
      <Button variant="ghost" :class="NO_PRESS" tabindex="-1" @mousedown="keepFocus" @click="run('copy')">Copy path</Button>
    </template>
  </DetailPane>
</template>
