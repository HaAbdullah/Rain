// The column layout shared by the Jobs header row and every JobRow (spec 7.5).
// Status, name, schedule, next run, run strip, paused marker. Schedule and next give way on narrow windows.
export const JOB_GRID =
  'grid grid-cols-[24px_minmax(120px,1fr)_minmax(88px,168px)_minmax(64px,88px)_104px_24px] items-center gap-3'
