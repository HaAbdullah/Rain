// Which snapshot of a scan to keep when several arrive in any order (a frame of the 'scan'
// topic, the answer of getScan, a reconnect). A final state beats 'scanning'; of two
// 'scanning' ones the one that has seen more files wins.
import type { DiskScan } from '@/data/types'

export function newer(current: DiskScan | null, incoming: DiskScan): DiskScan {
  if (!current || current.id !== incoming.id) return incoming
  if (current.status !== 'scanning') return current // already final: nothing replaces it
  if (incoming.status === 'scanning' && incoming.filesSeen < current.filesSeen) return current
  return incoming
}
