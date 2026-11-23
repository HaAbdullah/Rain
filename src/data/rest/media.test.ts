import { describe, expect, it } from 'vitest'
import { resolveMedia } from './media'

const toUrl = (reference: string) => `http://host/${reference}`

describe('resolveMedia', () => {
  it('rewrites value of an icon whose kind is url', () => {
    const answer = { icon: { kind: 'url', value: 'media/app-icon/app-vscode?v=3fa9' } }
    expect(resolveMedia(answer, toUrl).icon.value).toBe('http://host/media/app-icon/app-vscode?v=3fa9')
  })

  it('rewrites coverUrl', () => {
    expect(resolveMedia({ coverUrl: 'media/game-cover/g1' }, toUrl).coverUrl).toBe('http://host/media/game-cover/g1')
  })

  it('walks arrays and nested objects', () => {
    const answer = {
      games: [
        { title: 'A', coverUrl: 'media/game-cover/a' },
        { title: 'B', coverUrl: 'media/game-cover/b' },
      ],
    }
    const urls = resolveMedia(answer, toUrl).games.map((game) => game.coverUrl)
    expect(urls).toEqual(['http://host/media/game-cover/a', 'http://host/media/game-cover/b'])
  })

  it('leaves other icon kinds and other fields alone, even when they look like references', () => {
    const answer = {
      icon: { kind: 'lucide', value: 'media/looks-like-a-reference' },
      title: 'media/also-not-an-image',
      note: { value: 'media/no-kind-here' },
    }
    expect(resolveMedia(structuredClone(answer), toUrl)).toEqual(answer)
  })

  it('leaves an address that is not a media reference alone', () => {
    const answer = { coverUrl: 'https://example.com/c.png', icon: { kind: 'url', value: 'data:image/png;base64,AAAA' } }
    expect(resolveMedia(structuredClone(answer), toUrl)).toEqual(answer)
  })

  it('returns scalars and null unchanged', () => {
    expect(resolveMedia(null, toUrl)).toBeNull()
    expect(resolveMedia('media/x', toUrl)).toBe('media/x')
    expect(resolveMedia(42, toUrl)).toBe(42)
  })

  it('survives a very deep tree', () => {
    let deep: Record<string, unknown> = { coverUrl: 'media/deep' }
    for (let level = 0; level < 50_000; level++) deep = { child: deep }
    expect(() => resolveMedia(deep, toUrl)).not.toThrow()
  })
})
