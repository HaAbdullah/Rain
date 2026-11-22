// Lets an ItemList tell the rows inside it which size they are, so a screen
// sets `rowSize` once on the list and not again on every ListRow.

import type { InjectionKey, Ref } from 'vue'

export type RowSize = 'md' | 'lg'

export const rowSizeKey: InjectionKey<Readonly<Ref<RowSize>>> = Symbol('rain.rowSize')
