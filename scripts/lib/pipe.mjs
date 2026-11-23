// Small helpers that talk to the Rain backend over its named pipe. Shared by the scripts.
// Node speaks HTTP over a Windows pipe when it is given `socketPath`.
import http from 'node:http'
import net from 'node:net'

export const API = '/api/v1'

export function pipePath(pipeName) {
  return `\\\\.\\pipe\\${pipeName}`
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/** One request over the pipe. Resolves with `{ status, body }` (body is the text); rejects when the pipe cannot be reached. */
export function pipeRequest(pipeName, path, { method = 'GET', body } = {}) {
  return new Promise((resolve, reject) => {
    const headers = body === undefined ? {} : { 'content-type': 'application/json' }
    const request = http.request({ socketPath: pipePath(pipeName), method, path, headers }, (response) => {
      let text = ''
      response.setEncoding('utf8')
      response.on('data', (chunk) => (text += chunk))
      response.on('end', () => resolve({ status: response.statusCode, body: text }))
    })
    request.on('error', reject)
    request.setTimeout(3000, () => request.destroy(new Error('request timed out')))
    request.end(body === undefined ? undefined : JSON.stringify(body))
  })
}

/** GET `/ping`: the parsed answer, or null when nothing (or nothing of Rain) answers. */
export async function ping(pipeName) {
  try {
    const { status, body } = await pipeRequest(pipeName, `${API}/ping`)
    return status === 200 ? JSON.parse(body) : null
  } catch {
    return null
  }
}

/** Waits until `/ping` answers; returns the answer, or null after `timeoutMs`. */
export async function waitForPing(pipeName, timeoutMs = 10000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const answer = await ping(pipeName)
    if (answer) return answer
    await sleep(100)
  }
  return null
}

/** True while something listens on the pipe or the pipe name is taken. */
export function pipeExists(pipeName) {
  return new Promise((resolve) => {
    const socket = net.connect(pipePath(pipeName))
    socket.on('connect', () => {
      socket.destroy()
      resolve(true)
    })
    socket.on('error', (error) => resolve(error.code !== 'ENOENT'))
  })
}

/**
 * Opens the stream and resolves with its first `count` frames, then closes the connection:
 * an open client would keep the pipe name alive after the backend ended.
 */
export function readFirstFrames(pipeName, count) {
  return new Promise((resolve, reject) => {
    const frames = []
    const request = http.request({ socketPath: pipePath(pipeName), path: `${API}/stream` }, (response) => {
      if (response.statusCode !== 200) return reject(new Error(`stream answered ${response.statusCode}`))
      let buffer = ''
      response.setEncoding('utf8')
      response.on('data', (chunk) => {
        buffer += chunk
        let end
        while ((end = buffer.indexOf('\n\n')) !== -1) {
          const text = buffer.slice(0, end)
          buffer = buffer.slice(end + 2)
          const data = text.split('\n').find((line) => line.startsWith('data:'))
          if (data) frames.push(JSON.parse(data.slice(5).trim()))
        }
        if (frames.length >= count) {
          request.destroy()
          resolve(frames)
        }
      })
    })
    // Destroying the request on purpose raises an error after the promise is settled: ignore it.
    request.on('error', reject)
    request.setTimeout(5000, () => request.destroy(new Error(`no ${count} stream frames within 5 s`)))
    request.end()
  })
}

/** True while a process with this id exists. */
export function isAlive(pid) {
  try {
    process.kill(pid, 0) // signal 0 only tests that the process exists
    return true
  } catch {
    return false
  }
}
