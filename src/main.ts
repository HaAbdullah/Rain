import { createApp } from 'vue'
import './styles/tokens.css'
import App from './App.vue'
import { resetLaunches } from '@/composables/useLaunch'
import { resetResources } from '@/composables/useResource'
import { resetViewState } from '@/composables/useViewState'
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

createApp(App).mount('#app')
