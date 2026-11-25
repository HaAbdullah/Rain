// The shared resource of the Claude screen. It is read when the screen opens (the backend scans
// the Claude folders on that call) and again when the person presses Ctrl+R.
import { useResource } from '@/composables/useResource'
import { api } from '@/data'

export const useClaudeLibrary = () => useResource('claudeLibrary', () => api.getClaudeLibrary())
