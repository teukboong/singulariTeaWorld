import { mkdir, copyFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '..')
const src = resolve(root, 'src', 'index.js')
const distDir = resolve(root, 'dist')
const dist = resolve(distDir, 'index.js')

await mkdir(distDir, { recursive: true })
await copyFile(src, dist)
