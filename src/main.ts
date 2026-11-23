import { createApp } from 'vue'
import './styles/tokens.css'
import App from './App.vue'
import { resetLaunches } from '@/composables/useLaunch'
import { refreshResources, resetResources } from '@/composables/useResource'
import { resetViewState } from '@/composables/useViewState'
import { onStreamReset } from '@/data'
import { routing } from '@/data/routing'
import { mockControl } from '@/data/mock/control'
import { resetDisk } from '@/features/disk/actions'

// Changing the sample-data scenario (or resetting it) drops everything that
// pointed into the old data: cached lists, remembered selections, running launches and scans.
mockControl.onReseed(() => {
  resetViewState()
  resetLaunches()
  resetDisk()
  resetResources()
})

// The backend's stream says "refetch" after every (re)connect. A different backend process
// also forgot the launches and scans it was running, so those are dropped first. While a
// topic is still answered by the mock, its state does not live in the backend and stays.
onStreamReset(({ restarted }) => {
  if (restarted) {
    if (routing.topics.launch === 'real') resetLaunches()
    if (routing.topics.scan === 'real') resetDisk()
  }
  refreshResources()
})

createApp(App).mount('#app')
