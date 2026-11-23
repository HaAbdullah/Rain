import { createApp } from 'vue'
import './styles/tokens.css'
import App from './App.vue'
import { resetLaunches } from '@/composables/useLaunch'
import { refreshResources } from '@/composables/useResource'
import { onStreamReset } from '@/data'
import { reconcileDisk, resetDisk } from '@/features/disk/actions'

// The backend's stream says "refetch" after every (re)connect. A different backend process
// also forgot the launches and scans it was running, so those are dropped first.
onStreamReset(({ restarted }) => {
  if (restarted) {
    resetLaunches()
    resetDisk()
  } else {
    // A plain reconnect or a lag: a final scan frame may have been missed.
    void reconcileDisk()
  }
  refreshResources()
})

createApp(App).mount('#app')
