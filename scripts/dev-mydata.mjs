// The development command on a copy of your own data, in a browser tab.   npm run dev
//
// Copies the real data folder to `.rain-dev/mydata` (scripts/lib/mydata.mjs) and runs the
// development command (scripts/dev.mjs) on the copy: http://localhost:1420 shows the same screens
// as the installed Rain, a frontend edit shows in under a second, a Rust edit in a few seconds.
import { prepareMyData } from './lib/mydata.mjs'

const copy = prepareMyData()
if (copy) process.env.RAIN_DATA_DIR = copy
else console.log('dev: no installed Rain data found; starting with empty sandbox folders')
await import('./dev.mjs')
