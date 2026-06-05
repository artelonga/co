import { describe, it, expect, beforeAll, afterEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

// Load the shared markdown module into the happy-dom window
beforeAll(() => {
  const src = readFileSync(
    resolve(__dirname, '../../static/shared/markdown.js'),
    'utf8'
  )
  // Execute the IIFE — sets window.CoMarkdown on globalThis
  new Function(src)()
})

afterEach(() => {
  delete (globalThis as any).CoEditor
})

// Shorthand
function render(md: string): string {
  return (globalThis as any).CoMarkdown.renderMarkdown(md)
}

describe('CO-362 — http:// asset URL rewriting', () => {
  it('rewrites http:// img src to https:// (markdown image syntax)', () => {
    const html = render('![badge](http://img.shields.io/npm/l/foo.svg)')
    expect(html).toContain('src="https://img.shields.io/npm/l/foo.svg"')
    expect(html).not.toMatch(/src="http:\/\//)
  })

  it('keeps http:// in anchor href unchanged', () => {
    const html = render('[link](http://example.com)')
    expect(html).toContain('href="http://example.com"')
  })

  it('blocks script src http:// in post-processed HTML (editor bundle path)', () => {
    // Simulate the editor bundle producing HTML that contains an http:// script tag
    ;(globalThis as any).CoEditor = {
      renderMarkdown: () => '<p>text</p><script src="http://evil.example.com/x.js"></script>',
    }
    const html = render('ignored')
    expect(html).not.toContain('<script')
    expect(html).toContain('co: http script blocked')
    expect(html).toContain('evil.example.com/x.js')
  })
})
