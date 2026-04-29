import { existsSync } from 'node:fs'
import { execFile as execFileCallback } from 'node:child_process'
import { promisify } from 'node:util'
import { cp, mkdir, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises'
import { createInterface } from 'node:readline'
import os from 'node:os'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import { chromium } from 'playwright-core'

const DEFAULT_URL = 'https://chatgpt.com/'
const ENV_BASE_URL = 'WEBGPT_MCP_CHATGPT_URL'
const ENV_CHROME_BIN = 'WEBGPT_MCP_CHROME_BIN'
const ENV_CDP_URL = 'WEBGPT_MCP_CDP_URL'
const ENV_HEADLESS = 'WEBGPT_MCP_HEADLESS'
const ENV_HIDE_WINDOW = 'WEBGPT_MCP_HIDE_WINDOW'
const ENV_WINDOW_POSITION = 'WEBGPT_MCP_WINDOW_POSITION'
const ENV_WINDOW_SIZE = 'WEBGPT_MCP_WINDOW_SIZE'
const ENV_PROFILE_DIR = 'WEBGPT_MCP_PROFILE_DIR'
const ENV_MANUAL_PROFILE_DIR = 'WEBGPT_MCP_MANUAL_PROFILE_DIR'
const ENV_SELECTORS = 'WEBGPT_MCP_SELECTORS'
const DONE_IDLE_MS = 900
const MENU_TIMEOUT_MS = 2_500
const NAV_TIMEOUT_MS = 30_000
const POST_NAV_SETTLE_MS = 2_000
const SEND_SETTLE_MS = 4_000
const ATTACHMENT_UPLOAD_SETTLE_MS = 120_000
const CONTROL_PROBE_SETTLE_MS = 1_200
const CONTROL_REPLAY_TIMEOUT_MS = 3_500
const READY_STATE_CACHE_MS = 10_000
const ATTACHED_BROWSER_CHALLENGE_RETRY_ATTEMPTS = 30
const ATTACHED_BROWSER_CHALLENGE_RETRY_DELAY_MS = 1_000
const NETWORK_CAPTURE_LIMIT = 24
const NETWORK_BODY_CAPTURE_LIMIT = 12_000
const CHROME_IGNORE_DEFAULT_ARGS = ['--enable-automation']
const CHROME_STEALTH_ARGS = ['--disable-blink-features=AutomationControlled']
const DEFAULT_HIDDEN_WINDOW_POSITION = '2400,1400'
const DEFAULT_HIDDEN_WINDOW_SIZE = '960,780'
const DEFAULT_PROFILE_DIR = path.join(os.homedir(), '.hesperides', 'chatgpt-chrome-profile')
const PROFILE_SNAPSHOT_SUFFIX = '-snapshot'
const PROFILE_SNAPSHOT_INTERVAL_MS = 60_000
const DEFAULT_BOOTSTRAP_SNAPSHOT_DIR = `${DEFAULT_PROFILE_DIR}${PROFILE_SNAPSHOT_SUFFIX}`
const CONTROL_TEMPLATE_FILENAME = 'webgpt-mcp-control-templates.json'
const PROFILE_SKIP_COPY_EXACT = new Set([
  'DevToolsActivePort',
  'SingletonCookie',
  'SingletonLock',
  'SingletonSocket',
])
const PROFILE_SKIP_COPY_PREFIXES = ['BrowserMetrics', 'Crashpad', 'ShaderCache', 'GrShaderCache']
const DEFAULT_SELECTORS_PATH = fileURLToPath(new URL('../selectors.toml', import.meta.url))
const MAX_CITATION_COUNT = 8
const CITATION_SNIPPET_LIMIT = 280
const DONE_CITATION_RETRY_MS = 600
const execFile = promisify(execFileCallback)
let stdoutBroken = false

function log(message) {
  process.stderr.write(`[chatgpt-worker] ${message}\n`)
}

function writeStdoutJson(payload) {
  if (stdoutBroken) return false
  try {
    process.stdout.write(`${JSON.stringify(payload)}\n`)
    return true
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'EPIPE') {
      stdoutBroken = true
      return false
    }
    throw error
  }
}

function respond(id, result) {
  writeStdoutJson({ jsonrpc: '2.0', id, result })
}

function respondError(id, message) {
  writeStdoutJson({ jsonrpc: '2.0', id, error: { code: -32000, message } })
}

function emitEvent(payload) {
  writeStdoutJson(payload)
}

function nowConversationId(url) {
  const match = url.match(/\/c\/([^/?#]+)/)
  return match ? match[1] : ''
}

function unquoteToml(value) {
  return value.replace(/^['"]|['"]$/gu, '')
}

function appendTomlArrayItem(items, value) {
  const trimmed = value.trim()
  if (!trimmed || trimmed === ']') {
    return trimmed === ']'
  }

  const withoutComma = trimmed.endsWith(',') ? trimmed.slice(0, -1).trim() : trimmed
  const closesArray = withoutComma.endsWith(']')
  const item = closesArray ? withoutComma.slice(0, -1).trim() : withoutComma
  if (item) {
    items.push(unquoteToml(item))
  }
  return closesArray
}

function parseToml(content) {
  const result = {}
  let section = null
  let pendingArrayKey = null
  let pendingArrayItems = []

  for (const rawLine of content.split(/\r?\n/u)) {
    const line = rawLine.trim()
    if (!line || line.startsWith('#')) continue

    if (pendingArrayKey) {
      if (appendTomlArrayItem(pendingArrayItems, line)) {
        result[section][pendingArrayKey] = pendingArrayItems
        pendingArrayKey = null
        pendingArrayItems = []
      }
      continue
    }

    if (line.startsWith('[') && line.endsWith(']')) {
      section = line.slice(1, -1).trim()
      result[section] = result[section] ?? {}
      continue
    }
    const eq = line.indexOf('=')
    if (eq === -1 || !section) continue
    const key = line.slice(0, eq).trim()
    const value = line.slice(eq + 1).trim()

    if (value === '[' || (value.startsWith('[') && !value.endsWith(']'))) {
      pendingArrayKey = key
      pendingArrayItems = []
      const remainder = value.slice(1).trim()
      if (remainder && appendTomlArrayItem(pendingArrayItems, remainder)) {
        result[section][pendingArrayKey] = pendingArrayItems
        pendingArrayKey = null
        pendingArrayItems = []
      }
      continue
    }

    if (value.startsWith('[') && value.endsWith(']')) {
      const items = value
        .slice(1, -1)
        .split(',')
        .map((part) => part.trim())
        .filter(Boolean)
        .map(unquoteToml)
      result[section][key] = items
    } else {
      result[section][key] = unquoteToml(value)
    }
  }
  return result
}

function normalizeControlText(value) {
  return value.replace(/\s+/gu, ' ').trim()
}

function normalizeModelKey(value) {
  return normalizeControlText(value).toLowerCase()
}

function resolvedModelLabel(currentModel, availableModels) {
  const normalizedCurrent = normalizeModelKey(currentModel)
  if (normalizedCurrent && normalizedCurrent !== 'chatgpt' && normalizedCurrent !== '모델 선택기') {
    return currentModel
  }
  if (!Array.isArray(availableModels)) return currentModel
  return availableModels.find((option) => option?.selected)?.label || currentModel
}

function normalizeReasoningLevel(value) {
  const normalized = normalizeControlText(value).toLowerCase()
  if (!normalized) return ''
  if (normalized.includes('라이트') || normalized.includes('light')) return 'light'
  if (normalized.includes('확장') || normalized.includes('extended')) return 'extended'
  if (normalized.includes('표준') || normalized.includes('standard')) return 'standard'
  if (normalized.includes('헤비') || normalized.includes('heavy') || normalized.includes('max')) return 'heavy'
  return ''
}

function truncateBody(value) {
  if (typeof value !== 'string') return ''
  return value.length > NETWORK_BODY_CAPTURE_LIMIT
    ? `${value.slice(0, NETWORK_BODY_CAPTURE_LIMIT)}…`
    : value
}

function parseJsonIfPossible(value) {
  if (!value || typeof value !== 'string') return null
  try {
    return JSON.parse(value)
  } catch {
    return null
  }
}

function isHttpUrl(value) {
  if (typeof value !== 'string' || !value.trim()) return false
  try {
    const parsed = new URL(value)
    return parsed.protocol === 'http:' || parsed.protocol === 'https:'
  } catch {
    return false
  }
}

function normalizeCitationText(value) {
  return typeof value === 'string' ? normalizeControlText(value) : ''
}

function truncateCitationSnippet(value) {
  const normalized = normalizeCitationText(value)
  if (!normalized) return ''
  return normalized.length > CITATION_SNIPPET_LIMIT
    ? `${normalized.slice(0, CITATION_SNIPPET_LIMIT)}…`
    : normalized
}

function firstRecordString(record, keys) {
  for (const key of keys) {
    const normalized = normalizeCitationText(record?.[key])
    if (normalized) return normalized
  }
  return ''
}

function isCitationContextKey(key) {
  const normalized = normalizeCitationText(key).toLowerCase()
  if (!normalized) return false
  return [
    'citation',
    'source',
    'reference',
    'footnote',
    'attribution',
    'search_result',
    'searchresult',
    'grounding',
    'url_citation',
  ].some((token) => normalized.includes(token))
}

function normalizeCitationRecord(record, citationContext = false) {
  if (!record || typeof record !== 'object' || Array.isArray(record)) return null

  const rawUrl = firstRecordString(record, [
    'url',
    'source_url',
    'canonical_url',
    'canonicalUrl',
    'href',
    'link',
    'target_url',
  ])
  if (!isHttpUrl(rawUrl)) return null

  const parsedUrl = new URL(rawUrl)
  const title = firstRecordString(record, [
    'title',
    'display_title',
    'displayTitle',
    'name',
    'source_title',
    'sourceTitle',
    'article_title',
    'articleTitle',
    'provider_name',
    'providerName',
  ])
  const snippet = truncateCitationSnippet(firstRecordString(record, [
    'snippet',
    'excerpt',
    'quote',
    'description',
    'summary',
    'text',
    'preview_text',
    'previewText',
  ]))
  if (!citationContext && !title && !snippet) return null

  return {
    title: title || parsedUrl.hostname.replace(/^www\./u, '') || parsedUrl.toString(),
    url: parsedUrl.toString(),
    ...(snippet ? { snippet } : {}),
  }
}

function pushCitation(citations, seen, citation) {
  if (!citation || typeof citation !== 'object') return
  const key = normalizeCitationText(citation.url).toLowerCase()
  if (!key || seen.has(key)) return
  seen.add(key)
  citations.push(citation)
}

function collectCitationsFromJsonValue(value, citations, seen, citationContext = false) {
  if (!value || citations.length >= MAX_CITATION_COUNT) return
  if (Array.isArray(value)) {
    for (const item of value) {
      collectCitationsFromJsonValue(item, citations, seen, citationContext)
      if (citations.length >= MAX_CITATION_COUNT) return
    }
    return
  }
  if (typeof value !== 'object') return

  const recordContext = citationContext || Object.keys(value).some((key) => isCitationContextKey(key))
  pushCitation(citations, seen, normalizeCitationRecord(value, recordContext))
  if (citations.length >= MAX_CITATION_COUNT) return

  for (const [key, nested] of Object.entries(value)) {
    collectCitationsFromJsonValue(
      nested,
      citations,
      seen,
      recordContext || isCitationContextKey(key),
    )
    if (citations.length >= MAX_CITATION_COUNT) return
  }
}

function extractCitationsFromResponseText(value) {
  const raw = typeof value === 'string' ? value.trim() : ''
  if (!raw) return []

  const citations = []
  const seen = new Set()
  const direct = parseJsonIfPossible(raw)
  if (direct) {
    collectCitationsFromJsonValue(direct, citations, seen)
  }

  for (const line of raw.split(/\r?\n/u)) {
    const trimmed = line.trim()
    if (!trimmed) continue
    const payload = trimmed.startsWith('data:')
      ? trimmed.slice(5).trim()
      : trimmed
    if (!payload || payload === '[DONE]') continue
    if (!payload.startsWith('{') && !payload.startsWith('[')) continue
    const parsed = parseJsonIfPossible(payload)
    if (!parsed) continue
    collectCitationsFromJsonValue(parsed, citations, seen)
    if (citations.length >= MAX_CITATION_COUNT) break
  }

  return citations
}

function mergeCitations(...citationLists) {
  const merged = []
  const seen = new Set()
  for (const list of citationLists) {
    if (!Array.isArray(list)) continue
    for (const citation of list) {
      pushCitation(merged, seen, normalizeCitationRecord(citation, true) || citation)
      if (merged.length >= MAX_CITATION_COUNT) return merged
    }
  }
  return merged
}

function controlTargetTokens(kind, target) {
  const normalizedTarget = normalizeControlText(target).toLowerCase()
  if (!normalizedTarget) return []
  if (kind === 'reasoning') {
    const level = normalizeReasoningLevel(target)
    if (level === 'light') return ['라이트', 'light']
    if (level === 'standard') return ['표준', 'standard']
    if (level === 'extended') return ['확장', 'extended']
    if (level === 'heavy') return ['헤비', 'heavy', 'max']
    return []
  }
  return [normalizedTarget]
}

function isSettingsControlUrl(url) {
  return typeof url === 'string' && url.includes('/backend-api/settings/user_last_used_model_config')
}

function prioritizeReplayCandidates(kind, target, candidates) {
  const settingsCandidates = candidates.filter((entry) => isSettingsControlUrl(entry.url))
  if (settingsCandidates.length === 0) {
    return candidates
  }

  const tokens = controlTargetTokens(kind, target)
  if (tokens.length === 0) {
    return settingsCandidates
  }

  const matchingSettings = settingsCandidates.filter((entry) => {
    const haystack = `${entry.url}\n${entry.post_data}`.toLowerCase()
    return tokens.some((token) => haystack.includes(token.toLowerCase()))
  })
  return matchingSettings.length > 0 ? matchingSettings : settingsCandidates
}

function filterReplayHeaders(headers) {
  const filtered = {}
  for (const [rawKey, rawValue] of Object.entries(headers || {})) {
    const key = rawKey.toLowerCase()
    if (!rawValue) continue
    if (
      key === 'content-type'
      || key === 'accept'
      || key.startsWith('oai-')
      || key.startsWith('openai-')
      || key.startsWith('x-')
    ) {
      filtered[rawKey] = rawValue
    }
  }
  return filtered
}

function controlsMatchModelSelection(controls, label) {
  const normalizedLabel = normalizeModelKey(label)
  if (!normalizedLabel || !Array.isArray(controls?.available_models)) return false
  return controls.available_models.some((model) => (
    model?.selected && normalizeModelKey(model.label || '') === normalizedLabel
  ))
}

function controlsMatchReasoningSelection(controls, level) {
  const normalizedLevel = normalizeReasoningLevel(level)
  if (!normalizedLevel) return false
  if (controls?.current_reasoning_level === normalizedLevel) return true
  if (!Array.isArray(controls?.available_reasoning_levels)) return false
  return controls.available_reasoning_levels.some((option) => (
    option?.selected && normalizeReasoningLevel(option.id || option.label || '') === normalizedLevel
  ))
}

async function withTimeout(promise, timeoutMs, label) {
  let timer = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          reject(new Error(`${label} timed out after ${timeoutMs}ms`))
        }, timeoutMs)
      }),
    ])
  } finally {
    if (timer) clearTimeout(timer)
  }
}

async function writeFileSafe(filePath, content) {
  await writeFile(filePath, content, 'utf8')
}

function preferHeadlessBrowser() {
  return process.env[ENV_HEADLESS] !== '0'
}

function preferHiddenHeadedWindow() {
  return process.env[ENV_HIDE_WINDOW] !== '0'
}

function hiddenHeadedChromeArgs(headless) {
  if (headless || !preferHiddenHeadedWindow()) {
    return []
  }

  const position = process.env[ENV_WINDOW_POSITION] || DEFAULT_HIDDEN_WINDOW_POSITION
  const size = process.env[ENV_WINDOW_SIZE] || DEFAULT_HIDDEN_WINDOW_SIZE
  return [
    '--start-minimized',
    `--window-position=${position}`,
    `--window-size=${size}`,
  ]
}

async function loadSelectors() {
  const selectorsPath = process.env[ENV_SELECTORS] || DEFAULT_SELECTORS_PATH
  const content = await readFile(selectorsPath, 'utf8')
  return parseToml(content)
}

function shouldCopyProfilePath(sourcePath) {
  const name = path.basename(sourcePath)
  if (!name || name === '.' || name === '..') return true
  if (PROFILE_SKIP_COPY_EXACT.has(name)) return false
  return !PROFILE_SKIP_COPY_PREFIXES.some(prefix => name.startsWith(prefix))
}

async function readProcessesUsingProfile(profileDir) {
  try {
    const { stdout } = await execFile('ps', ['-axo', 'pid=,command='])
    return stdout
      .split(/\r?\n/u)
      .map(line => line.trim())
      .filter(Boolean)
      .map(line => {
        const match = line.match(/^(\d+)\s+(.*)$/u)
        if (!match) return null
        return { pid: Number(match[1]), command: match[2] }
      })
      .filter(Boolean)
      .filter(entry =>
        entry.pid !== process.pid
        && entry.command.includes('--user-data-dir=')
        && entry.command.includes(profileDir),
      )
  } catch (error) {
    log(`process scan skipped: ${error instanceof Error ? error.message : String(error)}`)
    return []
  }
}

class ChatGptWorker {
  constructor(selectors) {
    this.selectors = selectors
    this.browser = null
    this.browserContext = null
    this.page = null
    this.health = { state: 'ready' }
    this.currentModel = ''
    this.observerInstalledFor = ''
    this.profileSnapshotDir = ''
    this.manualProfileDir = ''
    this.profileSnapshotInFlight = null
    this.lastProfileSnapshotAt = 0
    this.forceHeaded = false
    this.lastLaunchHeadless = null
    this.cdpEndpointUrl = ''
    this.networkHooksInstalledFor = null
    this.recentNetworkEntries = []
    this.activeProbe = null
    this.lastProbe = null
    this.lastControls = null
    this.lastReadyAt = 0
    this.controlReplayTemplates = {
      model: new Map(),
      reasoning: new Map(),
    }
  }

  async init() {
    const profileDir =
      process.env[ENV_PROFILE_DIR]
      || DEFAULT_PROFILE_DIR
    await mkdir(profileDir, { recursive: true })
    this.profileDir = profileDir
    this.controlTemplatePath = path.join(profileDir, CONTROL_TEMPLATE_FILENAME)
    this.profileSnapshotDir = `${profileDir}${PROFILE_SNAPSHOT_SUFFIX}`
    this.manualProfileDir =
      process.env[ENV_MANUAL_PROFILE_DIR]
      || path.join(os.homedir(), '.hesperides', 'chatgpt-chrome-profile-manual')
    this.cdpEndpointUrl = (process.env[ENV_CDP_URL] || '').trim()
    await this.loadControlReplayTemplates()
  }

  usesAttachedBrowser() {
    return this.cdpEndpointUrl.length > 0
  }

  isProtectedSourceProfileTarget() {
    return this.profileDir === this.manualProfileDir || this.profileDir === DEFAULT_BOOTSTRAP_SNAPSHOT_DIR
  }

  selectorList(name) {
    return this.selectors[name]?.primary ?? []
  }

  async findFirst(selectors, { visible = true, timeout = 1500 } = {}) {
    for (const selector of selectors) {
      const locator = this.page.locator(selector).first()
      try {
        await locator.waitFor({ state: visible ? 'visible' : 'attached', timeout })
        return locator
      } catch {
        continue
      }
    }
    return null
  }

  async matchExists(selectors) {
    for (const selector of selectors) {
      if (await this.page.locator(selector).count()) return true
    }
    return false
  }

  async firstMatchingSelector(selectors) {
    for (const selector of selectors) {
      if (await this.page.locator(selector).count()) return selector
    }
    return null
  }

  async challengeDetail() {
    const title = await this.page.title()
    const normalizedTitle = title.trim()
    if (normalizedTitle === 'Just a moment...' || normalizedTitle === '잠시만 기다리십시오…') {
      return `challenge page title: ${normalizedTitle}`
    }

    const matchedSelector = await this.firstMatchingSelector(this.selectorList('challenge_hint'))
    if (!matchedSelector) {
      return ''
    }

    const bodySignals = await this.page.evaluate(() => {
      const text = (document.body?.innerText || '').replace(/\s+/gu, ' ').trim().toLowerCase()
      return {
        verifyHuman: text.includes('verify you are human'),
        checkingBrowser: text.includes('checking your browser'),
        attentionRequired: text.includes('attention required'),
        waitMessage:
          text.includes('잠시만 기다리십시오')
          || text.includes('잠시만요')
          || text.includes('브라우저를 확인'),
      }
    }).catch(() => ({
      verifyHuman: false,
      checkingBrowser: false,
      attentionRequired: false,
      waitMessage: false,
    }))

    if (
      bodySignals.verifyHuman
      || bodySignals.checkingBrowser
      || bodySignals.attentionRequired
      || bodySignals.waitMessage
    ) {
      return `challenge page matched ${matchedSelector}`
    }

    return ''
  }

  async hasReusableProfile(dirPath) {
    try {
      const entries = await readdir(dirPath)
      return entries.includes('Local State')
        && entries.some(name => name === 'Default' || name.startsWith('Profile '))
    } catch {
      return false
    }
  }

  async restoreProfileSnapshotIfNeeded() {
    if (this.usesAttachedBrowser()) return
    if (this.isProtectedSourceProfileTarget()) return
    if (!this.profileSnapshotDir || !existsSync(this.profileSnapshotDir)) return
    if (await this.hasReusableProfile(this.profileDir)) return

    await rm(this.profileDir, { recursive: true, force: true })
    await cp(this.profileSnapshotDir, this.profileDir, {
      recursive: true,
      force: true,
      filter: shouldCopyProfilePath,
    })
    emitEvent({
      event: 'profile_restored',
      profile_dir: this.profileDir,
      snapshot_dir: this.profileSnapshotDir,
    })
  }

  async profileMarkerMtimeMs(dirPath) {
    try {
      const marker = await stat(path.join(dirPath, 'Local State'))
      return marker.mtimeMs
    } catch {
      return 0
    }
  }

  async syncManualProfileIfNeeded() {
    if (this.usesAttachedBrowser()) return
    if (this.isProtectedSourceProfileTarget()) return
    if (!this.manualProfileDir || !(await this.hasReusableProfile(this.manualProfileDir))) return

    const manualMtime = await this.profileMarkerMtimeMs(this.manualProfileDir)
    const profileMtime = await this.profileMarkerMtimeMs(this.profileDir)
    const shouldSync = !(await this.hasReusableProfile(this.profileDir)) || manualMtime > profileMtime
    if (!shouldSync) return

    await rm(this.profileDir, { recursive: true, force: true })
    await cp(this.manualProfileDir, this.profileDir, {
      recursive: true,
      force: true,
      filter: shouldCopyProfilePath,
    })
    emitEvent({
      event: 'profile_synced',
      source_profile_dir: this.manualProfileDir,
      profile_dir: this.profileDir,
    })
  }

  async snapshotProfile(reason) {
    if (this.usesAttachedBrowser()) return
    if (this.isProtectedSourceProfileTarget()) return
    if (!(await this.hasReusableProfile(this.profileDir))) return

    const stagingDir = `${this.profileSnapshotDir}.tmp-${Date.now()}`
    await rm(stagingDir, { recursive: true, force: true })
    await cp(this.profileDir, stagingDir, {
      recursive: true,
      force: true,
      filter: shouldCopyProfilePath,
    })
    await rm(this.profileSnapshotDir, { recursive: true, force: true })
    await rename(stagingDir, this.profileSnapshotDir)
    this.lastProfileSnapshotAt = Date.now()
    emitEvent({
      event: 'profile_snapshot',
      reason,
      profile_dir: this.profileDir,
      snapshot_dir: this.profileSnapshotDir,
    })
  }

  async maybeSnapshotProfile(reason) {
    if (this.profileSnapshotInFlight) {
      await this.profileSnapshotInFlight
      return
    }
    if (Date.now() - this.lastProfileSnapshotAt < PROFILE_SNAPSHOT_INTERVAL_MS) {
      return
    }

    this.profileSnapshotInFlight = this.snapshotProfile(reason).catch(error => {
      log(`profile snapshot skipped: ${error instanceof Error ? error.message : String(error)}`)
    }).finally(() => {
      this.profileSnapshotInFlight = null
    })
    await this.profileSnapshotInFlight
  }

  async terminateProfileProcesses() {
    if (this.usesAttachedBrowser()) return
    if (this.isProtectedSourceProfileTarget()) return
    const matching = await readProcessesUsingProfile(this.profileDir)
    if (matching.length === 0) return

    for (const entry of matching) {
      try {
        process.kill(entry.pid, 'SIGTERM')
      } catch {}
    }
    await new Promise(resolve => setTimeout(resolve, 600))

    for (const entry of matching) {
      try {
        process.kill(entry.pid, 0)
        process.kill(entry.pid, 'SIGKILL')
      } catch {}
    }
    await new Promise(resolve => setTimeout(resolve, 300))
  }

  async closeBrowser() {
    const browser = this.browser
    const browserContext = this.browserContext
    this.browser = null
    this.browserContext = null
    this.page = null
    this.currentModel = ''
    this.observerInstalledFor = ''
    this.networkHooksInstalledFor = null
    if (browser) {
      await browser.close().catch(() => {})
      return
    }
    if (browserContext) {
      await browserContext.close().catch(() => {})
    }
  }

  async ensureBrowser() {
    if (this.browserContext && this.page && !this.page.isClosed()) {
      if (this.usesAttachedBrowser()) {
        return
      }
      const manualMtime = await this.profileMarkerMtimeMs(this.manualProfileDir)
      const profileMtime = await this.profileMarkerMtimeMs(this.profileDir)
      if (manualMtime <= profileMtime) {
        return
      }
      await this.closeBrowser()
    }
    if (this.usesAttachedBrowser()) {
      this.browser = await chromium.connectOverCDP(this.cdpEndpointUrl)
      this.browserContext = this.browser.contexts()[0] || null
      if (!this.browserContext) {
        throw new Error(`cdp browser exposed no default context: ${this.cdpEndpointUrl}`)
      }
      this.lastLaunchHeadless = false
    } else {
      await this.syncManualProfileIfNeeded()
      await this.restoreProfileSnapshotIfNeeded()
      await this.terminateProfileProcesses()

      const executablePath =
        process.env[ENV_CHROME_BIN]
        || [
          '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
          '/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary',
          '/Applications/Chromium.app/Contents/MacOS/Chromium',
        ].find((candidate) => existsSync(candidate))

      if (!executablePath) {
        this.setHealth('unavailable', 'no Chrome executable found')
        throw new Error('no Chrome executable found')
      }

      const launchContext = async (headless) => chromium.launchPersistentContext(this.profileDir, {
        executablePath,
        headless,
        ignoreDefaultArgs: CHROME_IGNORE_DEFAULT_ARGS,
        args: [...CHROME_STEALTH_ARGS, ...hiddenHeadedChromeArgs(headless)],
      })
      const preferredHeadless = this.forceHeaded ? false : preferHeadlessBrowser()
      try {
        this.browserContext = await launchContext(preferredHeadless)
        this.lastLaunchHeadless = preferredHeadless
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        const canRetryHeaded = preferredHeadless && process.env[ENV_HEADLESS] == null
        if (!message.includes('existing browser session') && !canRetryHeaded) {
          throw error
        }
        await this.terminateProfileProcesses()
        if (message.includes('existing browser session')) {
          this.browserContext = await launchContext(preferredHeadless)
          this.lastLaunchHeadless = preferredHeadless
        } else {
          log(`headless launch failed, retrying headed: ${message}`)
          this.browserContext = await launchContext(false)
          this.lastLaunchHeadless = false
          this.forceHeaded = true
        }
      }
    }
    if (this.usesAttachedBrowser()) {
      const targetUrl = process.env[ENV_BASE_URL] || DEFAULT_URL
      this.page = this.browserContext.pages().find((page) => page.url().startsWith(targetUrl))
        ?? await this.browserContext.newPage()
    } else {
      this.page = this.browserContext.pages()[0] ?? await this.browserContext.newPage()
    }
    this.page.on('close', () => {
      this.setHealth('degraded', 'page closed')
    })
    this.installNetworkHooks()
  }

  installNetworkHooks() {
    if (!this.page || this.networkHooksInstalledFor === this.page) return
    this.networkHooksInstalledFor = this.page
    this.page.on('requestfinished', (request) => {
      void this.captureNetworkEntry(request, false)
    })
    this.page.on('requestfailed', (request) => {
      void this.captureNetworkEntry(request, true)
    })
  }

  shouldCaptureNetworkEntry(request) {
    if (!this.page) return false
    const method = request.method().toUpperCase()
    if (method === 'OPTIONS' || method === 'HEAD') return false
    const resourceType = request.resourceType()
    if (!['fetch', 'xhr', 'document'].includes(resourceType)) return false
    const baseUrl = new URL(process.env[ENV_BASE_URL] || DEFAULT_URL)
    let requestUrl
    try {
      requestUrl = new URL(request.url())
    } catch {
      return false
    }
    if (requestUrl.origin !== baseUrl.origin) return false
    if (requestUrl.pathname.startsWith('/cdn-cgi/')) return false
    if (!this.activeProbe) {
      return ['POST', 'PUT', 'PATCH', 'DELETE'].includes(method)
    }
    return true
  }

  async captureNetworkEntry(request, failed) {
    if (!this.shouldCaptureNetworkEntry(request)) return
    const response = failed ? null : await request.response().catch(() => null)
    const postData = truncateBody(request.postData() || '')
    const citations = response ? await this.extractResponseCitations(request, response) : []
    const entry = {
      ts: Date.now(),
      method: request.method().toUpperCase(),
      url: request.url(),
      resource_type: request.resourceType(),
      status: response?.status() ?? 0,
      failed,
      headers: filterReplayHeaders(request.headers()),
      content_type: response?.headers()?.['content-type'] || '',
      post_data: postData,
      post_json: parseJsonIfPossible(postData),
      citations,
    }
    this.recentNetworkEntries.push(entry)
    if (this.recentNetworkEntries.length > NETWORK_CAPTURE_LIMIT) {
      this.recentNetworkEntries.splice(0, this.recentNetworkEntries.length - NETWORK_CAPTURE_LIMIT)
    }
    if (this.activeProbe) {
      this.activeProbe.entries.push(entry)
      if (this.activeProbe.entries.length > NETWORK_CAPTURE_LIMIT) {
        this.activeProbe.entries.splice(0, this.activeProbe.entries.length - NETWORK_CAPTURE_LIMIT)
      }
    }
  }

  async extractResponseCitations(request, response) {
    const resourceType = request.resourceType()
    const contentType = response.headers()?.['content-type'] || ''
    if (!['fetch', 'xhr'].includes(resourceType)) return []
    if (!request.url().includes('/backend-api/')) return []
    if (!contentType.includes('json') && !contentType.includes('event-stream')) return []
    const body = await response.text().catch(() => '')
    return extractCitationsFromResponseText(body)
  }

  collectRecentNetworkCitations() {
    const merged = []
    const seen = new Set()
    for (let index = this.recentNetworkEntries.length - 1; index >= 0; index -= 1) {
      const citations = this.recentNetworkEntries[index]?.citations
      if (!Array.isArray(citations) || citations.length === 0) continue
      for (const citation of citations) {
        pushCitation(merged, seen, normalizeCitationRecord(citation, true) || citation)
        if (merged.length >= MAX_CITATION_COUNT) {
          return merged
        }
      }
    }
    return merged
  }

  async collectDomCitations(messageId = '') {
    if (!this.page) return []
    return await this.page.evaluate(({ assistantSelectors, messageId, maxCitations }) => {
      const normalizeText = (value) => (
        typeof value === 'string' ? value.replace(/\s+/gu, ' ').trim() : ''
      )
      const extractUrls = (node) => {
        const matches = []
        if (!(node instanceof HTMLElement)) return matches
        for (const attribute of node.getAttributeNames()) {
          if (!attribute.includes('url') && !attribute.includes('href')) continue
          const value = node.getAttribute(attribute) || ''
          if (!value) continue
          matches.push(value)
        }
        const href = node.getAttribute('href')
        if (href) matches.push(href)
        return matches
      }
      const findAssistantTurns = () => {
        const nodes = []
        for (const selector of assistantSelectors) {
          nodes.push(...document.querySelectorAll(selector))
        }
        return nodes
      }
      const selectTargetTurn = () => {
        const turns = findAssistantTurns()
        if (!messageId) return turns.at(-1) || null
        for (let index = turns.length - 1; index >= 0; index -= 1) {
          const node = turns[index]
          const nodeMessageId =
            node?.getAttribute?.('data-message-id')
            || node?.id
            || node?.closest?.('[data-message-id]')?.getAttribute?.('data-message-id')
            || ''
          if (nodeMessageId === messageId) return node
        }
        return turns.at(-1) || null
      }

      const target = selectTargetTurn()
      if (!(target instanceof HTMLElement)) return []

      const citations = []
      const seen = new Set()
      const candidates = target.querySelectorAll(
        'a[href], button, [role="button"], [data-url], [data-source-url], [data-testid]',
      )
      for (const node of candidates) {
        const urls = extractUrls(node)
        for (const rawUrl of urls) {
          let parsedUrl
          try {
            parsedUrl = new URL(rawUrl, location.href)
          } catch {
            continue
          }
          if (!['http:', 'https:'].includes(parsedUrl.protocol)) continue
          if (
            parsedUrl.origin === location.origin
            && /^\/(?:c|g|share|api|auth)(?:\/|$)/u.test(parsedUrl.pathname)
          ) {
            continue
          }
          const key = parsedUrl.toString()
          if (seen.has(key)) continue
          seen.add(key)
          const label = normalizeText(
            node.getAttribute('aria-label')
            || node.getAttribute('title')
            || node.innerText
            || node.textContent
            || '',
          )
          const title = /^[\[(]?\d{1,2}[\])]?$|^$/u.test(label)
            ? parsedUrl.hostname.replace(/^www\./u, '')
            : label
          citations.push({ title, url: key })
          if (citations.length >= maxCitations) break
        }
        if (citations.length >= maxCitations) break
      }
      return citations
    }, {
      assistantSelectors: this.selectorList('assistant_turn'),
      messageId,
      maxCitations: MAX_CITATION_COUNT,
    }).catch(() => [])
  }

  async collectAnswerCitations(messageId = '') {
    const networkCitations = this.collectRecentNetworkCitations()
    const domCitations = await this.collectDomCitations(messageId)
    return mergeCitations(networkCitations, domCitations)
  }

  async withControlProbe(kind, target, action) {
    const probe = {
      kind,
      target,
      started_at: Date.now(),
      entries: [],
    }
    this.activeProbe = probe
    try {
      const result = await action()
      await this.page.waitForTimeout(CONTROL_PROBE_SETTLE_MS).catch(() => {})
      return result
    } finally {
      if (this.activeProbe === probe) {
        this.activeProbe = null
      }
      this.lastProbe = {
        kind: probe.kind,
        target: probe.target,
        started_at: probe.started_at,
        finished_at: Date.now(),
        entries: probe.entries,
      }
      await this.maybeStoreReplayTemplate(probe)
    }
  }

  replayTemplateCandidates(kind, target) {
    const tokens = controlTargetTokens(kind, target)
    const candidates = [...(this.activeProbe?.entries ?? []), ...(this.lastProbe?.entries ?? [])]
      .filter((entry) =>
        ['POST', 'PUT', 'PATCH', 'DELETE'].includes(entry.method)
        && !entry.failed
        && (entry.url.includes('/backend-api/') || entry.content_type.includes('json') || entry.post_data.startsWith('{'))
      )
    const matching = candidates.filter((entry) => {
      const haystack = `${entry.url}\n${entry.post_data}`.toLowerCase()
      return tokens.some((token) => haystack.includes(token.toLowerCase()))
    })
    return matching.length > 0 ? matching : candidates
  }

  async maybeStoreReplayTemplate(probe) {
    const key = probe.kind === 'reasoning'
      ? normalizeReasoningLevel(probe.target)
      : normalizeModelKey(probe.target)
    if (!key) return
    const candidates = prioritizeReplayCandidates(
      probe.kind,
      probe.target,
      this.replayTemplateCandidates(probe.kind, probe.target),
    )
    const candidate = candidates.at(-1)
    if (!candidate) {
      log(`probe ${probe.kind}:${key} captured no reusable request`)
      return
    }
    this.controlReplayTemplates[probe.kind].set(key, {
      method: candidate.method,
      url: candidate.url,
      headers: candidate.headers,
      body: candidate.post_data || null,
      captured_at: Date.now(),
    })
    await this.persistControlReplayTemplates()
    log(`captured ${probe.kind} replay template for ${key}: ${candidate.method} ${candidate.url}`)
  }

  async tryReplayControlTemplate(kind, target) {
    const key = kind === 'reasoning'
      ? normalizeReasoningLevel(target)
      : normalizeModelKey(target)
    if (!key) return false
    const template = this.controlReplayTemplates[kind].get(key)
    if (!template) return false
    if (!isSettingsControlUrl(template.url)) {
      log(`ignored stale ${kind} replay template for ${key}: ${template.url}`)
      return false
    }
    await this.ensurePageReady()
    const headers = await this.prepareReplayHeaders(template.url, template.headers)
    const result = await withTimeout(this.page.evaluate(async ({ url, method, headers, body, timeoutMs }) => {
      const controller = new AbortController()
      const timer = setTimeout(() => controller.abort('timeout'), timeoutMs)
      const response = await fetch(url, {
        method,
        headers,
        ...(typeof body === 'string' ? { body } : {}),
        credentials: 'include',
        signal: controller.signal,
      }).finally(() => {
        clearTimeout(timer)
      })
      return {
        ok: response.ok,
        status: response.status,
      }
    }, { ...template, headers, timeoutMs: CONTROL_REPLAY_TIMEOUT_MS }), CONTROL_REPLAY_TIMEOUT_MS + 500, `${kind} replay`)
    if (!result.ok) {
      throw new Error(`replay ${kind} request failed with status ${result.status}`)
    }
    await this.page.waitForTimeout(CONTROL_PROBE_SETTLE_MS).catch(() => {})
    log(`replayed ${kind} template for ${key}`)
    return true
  }

  async prepareReplayHeaders(url, headers) {
    const resolved = { ...(headers || {}) }
    if (!url.includes('/backend-api/')) {
      return resolved
    }
    const hasAuthorization = Object.keys(resolved).some((key) => key.toLowerCase() === 'authorization')
    if (hasAuthorization) {
      return resolved
    }
    const accessToken = await this.fetchSessionAccessToken({ ensureReady: false })
    if (!accessToken) {
      throw new Error('session access token unavailable')
    }
    resolved.Authorization = `Bearer ${accessToken}`
    return resolved
  }

  async fetchSessionAccessToken({ ensureReady = true } = {}) {
    if (ensureReady) {
      await this.ensurePageReady()
    }
    return await withTimeout(this.page.evaluate(async ({ timeoutMs }) => {
      const controller = new AbortController()
      const timer = setTimeout(() => controller.abort('timeout'), timeoutMs)
      const response = await fetch('/api/auth/session', {
        credentials: 'include',
        signal: controller.signal,
      }).finally(() => {
        clearTimeout(timer)
      })
      if (!response.ok) {
        return ''
      }
      const data = await response.json().catch(() => null)
      return typeof data?.accessToken === 'string' ? data.accessToken : ''
    }, { timeoutMs: CONTROL_REPLAY_TIMEOUT_MS }), CONTROL_REPLAY_TIMEOUT_MS + 500, 'session access token fetch')
  }

  rememberControls(controls) {
    this.lastControls = controls
    return controls
  }

  cachedControlsWithSelection({ modelLabel = '', reasoningLevel = '' }) {
    const base = this.lastControls || {
      current_model: this.currentModel || '',
      current_reasoning_level: '',
      available_models: [],
      available_reasoning_levels: [],
    }

    const nextModel = normalizeControlText(modelLabel)
    const nextReasoning = normalizeReasoningLevel(reasoningLevel)
    const controls = {
      current_model: nextModel || base.current_model || '',
      current_reasoning_level: nextReasoning || base.current_reasoning_level || '',
      available_models: Array.isArray(base.available_models)
        ? base.available_models.map((option) => ({
            ...option,
            selected: nextModel
              ? normalizeModelKey(option.label || option.id || '') === normalizeModelKey(nextModel)
              : !!option.selected,
          }))
        : [],
      available_reasoning_levels: Array.isArray(base.available_reasoning_levels)
        ? base.available_reasoning_levels.map((option) => ({
            ...option,
            selected: nextReasoning
              ? normalizeReasoningLevel(option.id || option.label || '') === nextReasoning
              : !!option.selected,
          }))
        : [],
    }
    return this.rememberControls(controls)
  }

  async loadControlReplayTemplates() {
    if (!this.controlTemplatePath || !existsSync(this.controlTemplatePath)) return
    try {
      const raw = await readFile(this.controlTemplatePath, 'utf8')
      const parsed = JSON.parse(raw)
      for (const kind of ['model', 'reasoning']) {
        this.controlReplayTemplates[kind].clear()
        for (const [key, template] of Object.entries(parsed?.[kind] || {})) {
          if (!template || typeof template !== 'object') continue
          if (!isSettingsControlUrl(template.url)) continue
          this.controlReplayTemplates[kind].set(key, template)
        }
      }
    } catch (error) {
      log(`control template load failed: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  async persistControlReplayTemplates() {
    if (!this.controlTemplatePath) return
    try {
      const payload = {
        model: Object.fromEntries(this.controlReplayTemplates.model.entries()),
        reasoning: Object.fromEntries(this.controlReplayTemplates.reasoning.entries()),
      }
      await writeFileSafe(this.controlTemplatePath, JSON.stringify(payload, null, 2))
    } catch (error) {
      log(`control template persist failed: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  debugProbe() {
    return {
      last_probe: this.lastProbe,
      recent_entries: this.recentNetworkEntries,
      templates: {
        model: [...this.controlReplayTemplates.model.keys()],
        reasoning: [...this.controlReplayTemplates.reasoning.keys()],
      },
    }
  }

  async inspectControlSurface() {
    await this.ensurePageReady()
    return await this.page.evaluate(() => {
      const isVisible = (node) => {
        if (!(node instanceof HTMLElement)) return false
        const rect = node.getBoundingClientRect()
        if (rect.width <= 0 || rect.height <= 0) return false
        const style = window.getComputedStyle(node)
        return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0'
      }

      const nodes = [...document.querySelectorAll(
        "button, [role='button'], [aria-haspopup='menu'], [aria-haspopup='dialog'], [data-testid]",
      )]
        .filter(isVisible)
        .map((node) => ({
          tag: node.tagName.toLowerCase(),
          role: node.getAttribute('role') || '',
          text: (node.textContent || '').replace(/\s+/gu, ' ').trim(),
          aria_label: node.getAttribute('aria-label') || '',
          aria_haspopup: node.getAttribute('aria-haspopup') || '',
          data_testid: node.getAttribute('data-testid') || '',
          class_name: node.className || '',
        }))
        .filter((entry) => entry.text || entry.aria_label || entry.data_testid)

      return nodes.slice(0, 80)
    })
  }

  async ensurePageReady() {
    if (
      this.browserContext
      && this.page
      && !this.page.isClosed()
      && this.health.state === 'ready'
      && Date.now() - this.lastReadyAt < READY_STATE_CACHE_MS
    ) {
      this.emitSessionInfo()
      return
    }

    const openReadyPage = async () => {
      await this.ensureBrowser()
      const targetUrl = process.env[ENV_BASE_URL] || DEFAULT_URL
      let navigated = false
      if (!this.page.url().startsWith(targetUrl)) {
        await this.page.goto(targetUrl, { waitUntil: 'domcontentloaded', timeout: NAV_TIMEOUT_MS })
        navigated = true
      }
      if (navigated) {
        await this.page.waitForTimeout(POST_NAV_SETTLE_MS)
      }
      await this.installObserver()
      await this.refreshHealth()
      await this.retryAttachedBrowserTransientChallenge()
      if (
        this.health.state === 'challenge_page'
        && this.lastLaunchHeadless
        && process.env[ENV_HEADLESS] == null
      ) {
        throw new Error('__webgpt_retry_headed_after_challenge__')
      }
      if (this.health.state !== 'ready') {
        throw new Error(this.health.detail || `worker not ready: ${this.health.state}`)
      }
      this.lastReadyAt = Date.now()
      this.emitSessionInfo()
    }

    try {
      await openReadyPage()
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      const needsRetry =
        message.includes('__webgpt_retry_headed_after_challenge__')
        ||
        message.includes('Target page, context or browser has been closed')
        || message.includes('Browser has been closed')
      if (!needsRetry) {
        throw error
      }
      if (message.includes('__webgpt_retry_headed_after_challenge__')) {
        this.forceHeaded = true
        log('challenge detected in headless mode, retrying headed')
      }
      await this.closeBrowser()
      await this.terminateProfileProcesses()
      await openReadyPage()
    }
  }

  async retryAttachedBrowserTransientChallenge() {
    if (!this.usesAttachedBrowser() || this.health.state !== 'challenge_page') {
      return
    }

    for (let attempt = 0; attempt < ATTACHED_BROWSER_CHALLENGE_RETRY_ATTEMPTS; attempt += 1) {
      await this.page.waitForTimeout(ATTACHED_BROWSER_CHALLENGE_RETRY_DELAY_MS)
      await this.refreshHealth()
      if (this.health.state !== 'challenge_page') {
        return
      }
    }
  }

  setHealth(state, detail = undefined) {
    const changed = this.health.state !== state || this.health.detail !== detail
    this.health = detail ? { state, detail } : { state }
    if (state !== 'ready') {
      this.lastReadyAt = 0
    }
    if (changed) {
      emitEvent({
        event: 'health_change',
        state,
        detail,
        conversation_id: this.page ? nowConversationId(this.page.url()) : '',
      })
    }
  }

  async refreshHealth() {
    if (!this.page) {
      this.setHealth('ready')
      return
    }
    const challengeDetail = await this.challengeDetail()
    if (challengeDetail) {
      this.setHealth('challenge_page', challengeDetail)
      return
    }
    if (await this.matchExists(this.selectorList('rate_limit_hint'))) {
      this.setHealth('rate_limited', 'ChatGPT reports a rate limit')
      return
    }
    const composerSelector = await this.firstMatchingSelector(this.selectorList('composer'))
    if (composerSelector) {
      await this.page.locator(composerSelector).first().waitFor({ state: 'visible', timeout: 400 }).catch(() => null)
      this.currentModel = await this.readModelLabel()
      await this.maybeSnapshotProfile('ready')
      this.setHealth('ready')
      return
    }
    if (await this.matchExists(this.selectorList('login_hint'))) {
      this.setHealth('expired_session', 'login required')
      return
    }
    this.setHealth('selector_drift', 'composer selector missing')
  }

  emitSessionInfo() {
    emitEvent({
      event: 'session',
      conversation_id: this.page ? nowConversationId(this.page.url()) : '',
      model: this.currentModel,
      url: this.page?.url() || '',
    })
  }

  async readModelLabel() {
    const locator = await this.findFirst(this.selectorList('model_label'), { timeout: 500 })
    return locator ? (await locator.textContent())?.trim() || '' : ''
  }

  async readReasoningLabel() {
    const locator = await this.findFirst(this.selectorList('reasoning_button'), { timeout: 500 })
    return locator ? normalizeReasoningLevel((await locator.textContent())?.trim() || '') : ''
  }

  async openMenu(triggerSelectors, detail) {
    const trigger = await this.findFirst(triggerSelectors, { timeout: 1500 })
    if (!trigger) {
      throw new Error(`${detail} trigger not found`)
    }
    await trigger.click()
    await this.findFirst(this.selectorList('menu_surface'), { timeout: MENU_TIMEOUT_MS }).catch(() => null)
    await this.page.waitForTimeout(120)
  }

  async closeMenu() {
    await this.page.keyboard.press('Escape').catch(() => {})
    await this.page.waitForTimeout(80)
  }

  async collectVisibleMenuOptions() {
    return await this.page.evaluate(({ surfaceSelectors }) => {
      const isVisible = (node) => {
        if (!(node instanceof HTMLElement)) return false
        const rect = node.getBoundingClientRect()
        if (rect.width <= 0 || rect.height <= 0) return false
        const style = window.getComputedStyle(node)
        return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0'
      }

      const roots = []
      for (const selector of surfaceSelectors) {
        roots.push(...document.querySelectorAll(selector))
      }
      const visibleRoots = roots.filter(isVisible)
      const searchRoots = visibleRoots.length > 0 ? visibleRoots : [document.body]
      const seen = new Set()
      const options = []

      for (const root of searchRoots) {
        const interactive = root.querySelectorAll(
          "button, [role='button'], [role='menuitem'], [role='menuitemradio'], [role='option']",
        )
        for (const node of interactive) {
          if (!isVisible(node)) continue
          const rawText = (node.innerText || node.textContent || '').trim()
          const normalized = rawText.replace(/\s+/gu, ' ').trim()
          if (!normalized || normalized.length < 2) continue
          if (seen.has(normalized)) continue
          seen.add(normalized)
          const lines = rawText
            .split(/\r?\n/gu)
            .map((line) => line.trim())
            .filter(Boolean)
          const label = lines[0] || normalized
          const detail = lines.slice(1).join(' ').trim()
          const selected =
            node.getAttribute('aria-checked') === 'true'
            || node.getAttribute('aria-selected') === 'true'
            || node.getAttribute('data-state') === 'checked'
            || /(?:^|\s)[✓✔](?:\s|$)/u.test(rawText)
          options.push({
            id: label
              .toLowerCase()
              .replace(/[^a-z0-9가-힣]+/gu, '-')
              .replace(/^-+|-+$/gu, '')
              || 'option',
            label,
            detail: detail || undefined,
            selected,
          })
        }
      }

      return options.slice(0, 24)
    }, { surfaceSelectors: this.selectorList('menu_surface') })
  }

  async clickVisibleMenuOption({ label, reasoningLevel }) {
    return await this.page.evaluate(({ surfaceSelectors, label, reasoningLevel }) => {
      const isVisible = (node) => {
        if (!(node instanceof HTMLElement)) return false
        const rect = node.getBoundingClientRect()
        if (rect.width <= 0 || rect.height <= 0) return false
        const style = window.getComputedStyle(node)
        return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0'
      }

      const normalize = (value) => value.replace(/\s+/gu, ' ').trim().toLowerCase()
      const roots = []
      for (const selector of surfaceSelectors) {
        roots.push(...document.querySelectorAll(selector))
      }
      const visibleRoots = roots.filter(isVisible)
      const searchRoots = visibleRoots.length > 0 ? visibleRoots : [document.body]
      const activate = (node) => {
        node.scrollIntoView({ block: 'center', inline: 'nearest' })
        node.focus?.()
        for (const eventName of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {
          node.dispatchEvent(new MouseEvent(eventName, { bubbles: true, cancelable: true, composed: true }))
        }
        if (node instanceof HTMLElement) {
          node.click()
        }
      }
      const expectedTokens = reasoningLevel === 'light'
        ? ['라이트', 'light']
        : reasoningLevel === 'extended'
        ? ['확장', 'extended']
        : reasoningLevel === 'standard'
        ? ['표준', 'standard']
        : reasoningLevel === 'heavy'
        ? ['헤비', 'heavy']
        : []
      const normalizedLabel = label ? normalize(label) : ''

      for (const root of searchRoots) {
        const interactive = root.querySelectorAll(
          "button, [role='button'], [role='menuitem'], [role='menuitemradio'], [role='option']",
        )
        for (const node of interactive) {
          if (!isVisible(node)) continue
          const text = (node.innerText || node.textContent || '').trim()
          if (!text) continue
          const normalizedText = normalize(text)
          const lines = text
            .split(/\r?\n/gu)
            .map((line) => line.trim())
            .filter(Boolean)
          const normalizedPrimary = normalize(lines[0] || text)
          const isMatch = normalizedLabel
            ? normalizedText === normalizedLabel
              || normalizedPrimary === normalizedLabel
              || normalizedText.includes(normalizedLabel)
            : expectedTokens.some((token) => normalizedPrimary.includes(token) || normalizedText.includes(token))
          if (!isMatch) continue
          activate(node)
          return true
        }
      }
      return false
    }, {
      surfaceSelectors: this.selectorList('menu_surface'),
      label: label || '',
      reasoningLevel: reasoningLevel || '',
    })
  }

  async controlsInfo() {
    await this.ensurePageReady()
    const buttonModelLabel = await this.readModelLabel()
    this.currentModel = buttonModelLabel
    let currentReasoningLevel = await this.readReasoningLabel()
    let availableModels = []
    let availableReasoningLevels = []

    const modelTrigger = await this.findFirst(this.selectorList('model_label'), { timeout: 500 })
    if (modelTrigger) {
      try {
        await this.openMenu(this.selectorList('model_label'), 'model selector')
        availableModels = await this.collectVisibleMenuOptions()
      } catch {
        availableModels = []
      } finally {
        await this.closeMenu()
      }
    }

    this.currentModel = resolvedModelLabel(buttonModelLabel, availableModels)

    const reasoningTrigger = await this.findFirst(this.selectorList('reasoning_button'), { timeout: 500 })
    if (reasoningTrigger) {
      try {
        await this.openMenu(this.selectorList('reasoning_button'), 'reasoning selector')
        availableReasoningLevels = (await this.collectVisibleMenuOptions())
          .map((option) => {
            const normalizedLevel = normalizeReasoningLevel(option.label)
            if (!normalizedLevel) return null
            return {
              id: normalizedLevel,
              label: option.label,
              detail: option.detail,
              selected: option.selected,
            }
          })
          .filter(Boolean)
        if (!currentReasoningLevel) {
          currentReasoningLevel = availableReasoningLevels.find((option) => option.selected)?.id || ''
        }
      } catch {
        availableReasoningLevels = []
      } finally {
        await this.closeMenu()
      }
    }

    return this.rememberControls({
      current_model: this.currentModel || '',
      current_reasoning_level: currentReasoningLevel || '',
      available_models: availableModels,
      available_reasoning_levels: availableReasoningLevels,
    })
  }

  async selectModel(label) {
    const normalizedLabel = normalizeControlText(label)
    if (!normalizedLabel) {
      throw new Error('model label is required')
    }

    await this.ensurePageReady()
    try {
      if (await this.tryReplayControlTemplate('model', normalizedLabel)) {
        this.currentModel = normalizedLabel
        const replayControls = this.cachedControlsWithSelection({ modelLabel: normalizedLabel })
        this.emitSessionInfo()
        return replayControls
      }
    } catch (error) {
      log(`model replay failed for ${normalizedLabel}: ${error instanceof Error ? error.message : String(error)}`)
    }
    try {
      await this.withControlProbe('model', normalizedLabel, async () => {
        await this.openMenu(this.selectorList('model_label'), 'model selector')
        const clicked = await this.clickVisibleMenuOption({ label: normalizedLabel })
        if (!clicked) {
          throw new Error(`model option not found: ${normalizedLabel}`)
        }
      })
    } finally {
      await this.closeMenu()
    }

    await this.page.waitForTimeout(220)
    this.currentModel = normalizedLabel
    const selectedControls = this.cachedControlsWithSelection({ modelLabel: normalizedLabel })
    this.emitSessionInfo()
    return selectedControls
  }

  async selectReasoningLevel(level) {
    const normalizedLevel = normalizeReasoningLevel(level)
    if (!normalizedLevel) {
      throw new Error(`unsupported reasoning level: ${level}`)
    }

    await this.ensurePageReady()
    try {
      if (await this.tryReplayControlTemplate('reasoning', normalizedLevel)) {
        const replayControls = this.cachedControlsWithSelection({ reasoningLevel: normalizedLevel })
        this.emitSessionInfo()
        return replayControls
      }
    } catch (error) {
      log(`reasoning replay failed for ${normalizedLevel}: ${error instanceof Error ? error.message : String(error)}`)
    }
    try {
      await this.withControlProbe('reasoning', normalizedLevel, async () => {
        await this.openMenu(this.selectorList('reasoning_button'), 'reasoning selector')
        const clicked = await this.clickVisibleMenuOption({ reasoningLevel: normalizedLevel })
        if (!clicked) {
          throw new Error(`reasoning option not found: ${normalizedLevel}`)
        }
      })
    } finally {
      await this.closeMenu()
    }

    await this.page.waitForTimeout(180)
    const selectedControls = this.cachedControlsWithSelection({ reasoningLevel: normalizedLevel })
    this.emitSessionInfo()
    return selectedControls
  }

  async installObserver() {
    const url = this.page.url()
    if (this.observerInstalledFor === url) return

    await this.page.exposeBinding('__webgptMcpEmit', async (_source, payload) => {
      if (!payload || typeof payload.event !== 'string') return
      if (payload.event !== 'done') {
        emitEvent(payload)
        return
      }

      try {
        let citations = await this.collectAnswerCitations(payload.message_id || '')
        if (citations.length === 0) {
          await this.page.waitForTimeout(DONE_CITATION_RETRY_MS).catch(() => {})
          citations = await this.collectAnswerCitations(payload.message_id || '')
        }
        emitEvent({
          ...payload,
          citations,
        })
      } catch (error) {
        log(`citation capture skipped: ${error instanceof Error ? error.message : String(error)}`)
        emitEvent(payload)
      }
    }).catch(() => {})

    await this.page.evaluate(
      ({ assistantSelectors, assistantContentSelectors, stopSelectors, idleMs }) => {
        if (window.__webgptMcpObserverCleanup) {
          window.__webgptMcpObserverCleanup()
        }

        let chunkSeq = 0
        let lastAssistantText = ''
        let lastChangedAt = 0
        let doneSentFor = ''
        let timer = null

        const currentConversationId = () => {
          const match = location.pathname.match(/\/c\/([^/?#]+)/)
          return match ? match[1] : ''
        }

        const findAssistantTurns = () => {
          const nodes = []
          for (const selector of assistantSelectors) {
            nodes.push(...document.querySelectorAll(selector))
          }
          return nodes
        }

        const stopVisible = () => stopSelectors.some((selector) => {
          const el = document.querySelector(selector)
          return !!el && el.getAttribute('aria-hidden') !== 'true'
        })

        const isVisible = (node) => {
          if (!(node instanceof HTMLElement)) return false
          const rect = node.getBoundingClientRect()
          if (rect.width <= 0 || rect.height <= 0) return false
          const style = window.getComputedStyle(node)
          return style.display !== 'none' && style.visibility !== 'hidden'
        }

        const normalizeTranscriptText = (value) => (
          value
            .replace(/\u00a0/gu, ' ')
            .replace(/\n{3,}/gu, '\n\n')
            .replace(/[ \t]+\n/gu, '\n')
            .trim()
        )

        const extractAssistantText = (turnRoot) => {
          if (!(turnRoot instanceof HTMLElement)) return ''
          const contentRoot = assistantContentSelectors
            .map((selector) => turnRoot.querySelector(selector))
            .find((node) => node instanceof HTMLElement && isVisible(node))
            || turnRoot
          const clone = contentRoot.cloneNode(true)
          if (!(clone instanceof HTMLElement)) {
            return normalizeTranscriptText(contentRoot.innerText || contentRoot.textContent || '')
          }
          clone.querySelectorAll([
            'button',
            '[role="button"]',
            'svg',
            'style',
            'script',
            'noscript',
            'textarea',
            'input',
            'form',
            '[aria-hidden="true"]',
            '.sr-only',
            '[data-testid*="copy"]',
            '[data-testid*="action"]',
            '[data-message-action]',
          ].join(',')).forEach((node) => node.remove())
          return normalizeTranscriptText(clone.innerText || clone.textContent || '')
        }

        const readAssistantText = () => {
          const turns = findAssistantTurns()
          const last = turns.at(-1)
          const text = last ? extractAssistantText(last) : ''
          const messageId =
            last?.getAttribute('data-message-id')
            || last?.id
            || last?.closest?.('[data-message-id]')?.getAttribute?.('data-message-id')
            || ''
          return { text, messageId }
        }

        const maybeEmitDone = () => {
          const { text, messageId } = readAssistantText()
          if (!text || stopVisible()) return
          if (Date.now() - lastChangedAt < idleMs) return
          if (doneSentFor === text) return
          doneSentFor = text
          window.__webgptMcpEmit({
            event: 'done',
            text,
            conversation_id: currentConversationId(),
            message_id: messageId,
          })
        }

        const onMutation = () => {
          const { text, messageId } = readAssistantText()
          if (text && text !== lastAssistantText) {
            const nextChunk = text.startsWith(lastAssistantText)
              ? text.slice(lastAssistantText.length)
              : text
            lastAssistantText = text
            lastChangedAt = Date.now()
            doneSentFor = ''
            window.__webgptMcpEmit({
              event: 'partial',
              text: nextChunk,
              chunk_seq: chunkSeq,
              conversation_id: currentConversationId(),
              message_id: messageId,
            })
            chunkSeq += 1
          }
          if (timer) clearTimeout(timer)
          timer = setTimeout(maybeEmitDone, idleMs)
        }

        const observer = new MutationObserver(onMutation)
        const baseline = readAssistantText()
        if (baseline.text) {
          lastAssistantText = baseline.text
          doneSentFor = baseline.text
          lastChangedAt = Date.now()
        }
        observer.observe(document.body, {
          subtree: true,
          childList: true,
          characterData: true,
        })

        window.__webgptMcpObserverCleanup = () => {
          observer.disconnect()
          if (timer) clearTimeout(timer)
        }
      },
      {
        assistantSelectors: this.selectorList('assistant_turn'),
        assistantContentSelectors: this.selectorList('assistant_content'),
        stopSelectors: this.selectorList('stop_button'),
        idleMs: DONE_IDLE_MS,
      },
    )

    this.observerInstalledFor = url
  }

  async fillComposer(text) {
    const composer = await this.findFirst(this.selectorList('composer'))
    if (!composer) {
      await this.refreshHealth()
      throw new Error(this.health.detail || 'composer not found')
    }

    await composer.click()
    const tagName = await composer.evaluate((node) => node.tagName.toLowerCase())
    const isTextAreaLike = tagName === 'textarea' || tagName === 'input'

    if (isTextAreaLike) {
      await composer.fill('')
      await composer.fill(text)
    } else {
      const modifier = process.platform === 'darwin' ? 'Meta' : 'Control'
      await composer.focus()
      await this.page.keyboard.press(`${modifier}+A`).catch(() => {})
      await this.page.keyboard.press('Backspace').catch(() => {})
      await this.page.keyboard.insertText(text)
    }

    const filledValue = await composer.evaluate((node) => {
      if (node instanceof HTMLTextAreaElement || node instanceof HTMLInputElement) {
        return node.value
      }
      return node.innerText || node.textContent || ''
    })
    if (!normalizeControlText(filledValue).includes(normalizeControlText(text))) {
      throw new Error('composer text did not stick')
    }
  }

  async clickSend() {
    const button = await this.findFirst(this.selectorList('send_button'))
    if (button) {
      const disabled = await button.evaluate((node) => {
        if (!(node instanceof HTMLElement)) return false
        return node.getAttribute('aria-disabled') === 'true'
          || node.hasAttribute('disabled')
      }).catch(() => false)
      if (!disabled) {
        await button.click()
      } else {
        await this.page.keyboard.press('Enter')
      }
    } else {
      await this.page.keyboard.press('Enter')
    }

    await this.page.waitForFunction(
      ({ stopSelectors, composerSelectors }) => {
        const isVisible = (node) => {
          if (!(node instanceof HTMLElement)) return false
          const rect = node.getBoundingClientRect()
          if (rect.width <= 0 || rect.height <= 0) return false
          const style = window.getComputedStyle(node)
          return style.display !== 'none' && style.visibility !== 'hidden'
        }

        const stopVisible = stopSelectors.some((selector) => {
          const node = document.querySelector(selector)
          return node && isVisible(node) && node.getAttribute('aria-hidden') !== 'true'
        })
        if (stopVisible) return true

        for (const selector of composerSelectors) {
          const node = document.querySelector(selector)
          if (!node) continue
          const value = node instanceof HTMLTextAreaElement || node instanceof HTMLInputElement
            ? node.value
            : node.textContent || node.innerText || ''
          if (!value.trim()) return true
        }
        return false
      },
      {
        stopSelectors: this.selectorList('stop_button'),
        composerSelectors: this.selectorList('composer'),
      },
      { timeout: SEND_SETTLE_MS },
    ).catch(() => {})

    const postSendState = await this.page.evaluate(
      ({ stopSelectors, composerSelectors }) => {
        const isVisible = (node) => {
          if (!(node instanceof HTMLElement)) return false
          const rect = node.getBoundingClientRect()
          if (rect.width <= 0 || rect.height <= 0) return false
          const style = window.getComputedStyle(node)
          return style.display !== 'none' && style.visibility !== 'hidden'
        }

        const stopVisible = stopSelectors.some((selector) => {
          const node = document.querySelector(selector)
          return node && isVisible(node) && node.getAttribute('aria-hidden') !== 'true'
        })
        const composerText = composerSelectors
          .map((selector) => document.querySelector(selector))
          .filter((node) => node && isVisible(node))
          .map((node) => {
            if (node instanceof HTMLTextAreaElement || node instanceof HTMLInputElement) {
              return node.value
            }
            return node.textContent || node.innerText || ''
          })
          .join('\n')
          .replace(/\s+/gu, ' ')
          .trim()

        return { stopVisible, composerHasText: composerText.length > 0 }
      },
      {
        stopSelectors: this.selectorList('stop_button'),
        composerSelectors: this.selectorList('composer'),
      },
    )
    if (!postSendState.stopVisible && postSendState.composerHasText) {
      throw new Error(`send did not start: composer still contains text after ${SEND_SETTLE_MS}ms`)
    }
  }

  async waitForSendReadyAfterUpload() {
    await this.page.waitForFunction(
      ({ sendSelectors }) => {
        const isVisible = (node) => {
          if (!(node instanceof HTMLElement)) return false
          const rect = node.getBoundingClientRect()
          if (rect.width <= 0 || rect.height <= 0) return false
          const style = window.getComputedStyle(node)
          return style.display !== 'none' && style.visibility !== 'hidden'
        }
        const isEnabled = (node) => (
          node instanceof HTMLElement
          && isVisible(node)
          && node.getAttribute('aria-disabled') !== 'true'
          && !node.hasAttribute('disabled')
        )
        return sendSelectors.some((selector) => {
          const node = document.querySelector(selector)
          return isEnabled(node)
        })
      },
      { sendSelectors: this.selectorList('send_button') },
      { timeout: ATTACHMENT_UPLOAD_SETTLE_MS },
    ).catch(() => {
      throw new Error(`attachment upload did not enable send button within ${ATTACHMENT_UPLOAD_SETTLE_MS}ms`)
    })
  }

  async sendText(text) {
    this.forceHeaded = true
    await this.ensurePageReady()
    this.recentNetworkEntries = []
    emitEvent({ event: 'status', msg: 'working', conversation_id: nowConversationId(this.page.url()) })
    await this.fillComposer(text)
    await this.clickSend()
  }

  async sendAttachments(filePaths, text = '') {
    this.forceHeaded = true
    await this.ensurePageReady()
    this.recentNetworkEntries = []
    const attachButton = await this.findFirst(this.selectorList('attach_button'), { timeout: 1000 })
    if (attachButton) {
      await attachButton.click()
    }
    const input = await this.findFirst(this.selectorList('file_input'), { visible: false, timeout: 3000 })
    if (!input) {
      this.setHealth('selector_drift', 'file input selector missing')
      throw new Error('file input selector missing')
    }

    await input.setInputFiles(filePaths)
    emitEvent({ event: 'status', msg: 'working', conversation_id: nowConversationId(this.page.url()) })
    if (text) {
      await this.fillComposer(text)
    }
    await this.waitForSendReadyAfterUpload()
    await this.clickSend()
  }

  async generatedImageSources() {
    await this.ensurePageReady()
    return await this.page.evaluate(() => (
      Array.from(document.querySelectorAll('img'))
        .filter((node) => node instanceof HTMLImageElement)
        .filter((node) => {
          const alt = node.getAttribute('alt') || ''
          const hasImageGenFrame = !!node.closest('[class*="imagegen-image"]')
          const isUploadedReference = !!node.closest('[data-message-author-role="user"]')
            || alt.includes('업로드한 이미지')
            || alt.toLowerCase().includes('uploaded image')
          return !isUploadedReference
            && (hasImageGenFrame
              || alt.includes('생성된 이미지')
              || alt.toLowerCase().includes('generated image'))
        })
        .map((node) => node.currentSrc || node.src)
        .filter((src) => src && src.includes('/backend-api/estuary/content'))
    ))
  }

  async extractGeneratedImages(maxImages = 4, excludeSrcs = []) {
    await this.ensurePageReady()
    const boundedMaxImages = Math.max(1, Math.min(Number(maxImages) || 4, 12))
    return await this.page.evaluate(async ({ maxImages: limit, excludeSrcs: rawExcludeSrcs }) => {
      const excludeSrcs = new Set(Array.isArray(rawExcludeSrcs) ? rawExcludeSrcs : [])
      const isVisible = (node) => {
        if (!(node instanceof HTMLElement)) return false
        const rect = node.getBoundingClientRect()
        if (rect.width <= 0 || rect.height <= 0) return false
        const style = window.getComputedStyle(node)
        return style.display !== 'none' && style.visibility !== 'hidden'
      }

      const imageNodes = Array.from(document.querySelectorAll('img'))
        .filter((node) => node instanceof HTMLImageElement)
        .filter((node) => node.currentSrc || node.src)
        .filter((node) => {
          const src = node.currentSrc || node.src
          const alt = node.getAttribute('alt') || ''
          const hasImageGenFrame = !!node.closest('[class*="imagegen-image"]')
          const isUploadedReference = !!node.closest('[data-message-author-role="user"]')
            || alt.includes('업로드한 이미지')
            || alt.toLowerCase().includes('uploaded image')
          return !isUploadedReference
            && src.includes('/backend-api/estuary/content')
            && (hasImageGenFrame
              || alt.includes('생성된 이미지')
              || alt.toLowerCase().includes('generated image'))
        })
        .filter((node) => isVisible(node))

      const uniqueImages = []
      const seen = new Set()
      for (const node of imageNodes) {
        const src = node.currentSrc || node.src
        if (excludeSrcs.has(src)) continue
        if (!src || seen.has(src)) continue
        seen.add(src)
        uniqueImages.push(node)
        if (uniqueImages.length >= limit) break
      }

      const images = []
      for (const node of uniqueImages) {
        node.scrollIntoView({ block: 'center', inline: 'center' })
        if (!node.complete || node.naturalWidth === 0 || node.naturalHeight === 0) {
          await node.decode().catch(() => {})
        }
        if (node.naturalWidth === 0 || node.naturalHeight === 0) {
          images.push({
            alt: node.getAttribute('alt') || '',
            src: node.currentSrc || node.src,
            natural_width: node.naturalWidth || 0,
            natural_height: node.naturalHeight || 0,
            mime_type: '',
            data_url: '',
            error: 'image not decoded',
          })
          continue
        }

        const canvas = document.createElement('canvas')
        canvas.width = node.naturalWidth
        canvas.height = node.naturalHeight
        const context = canvas.getContext('2d')
        context.drawImage(node, 0, 0)
        let dataUrl = ''
        let error = ''
        try {
          dataUrl = canvas.toDataURL('image/png')
        } catch (caught) {
          error = caught instanceof Error ? caught.message : String(caught)
        }
        images.push({
          alt: node.getAttribute('alt') || '',
          src: node.currentSrc || node.src,
          natural_width: node.naturalWidth,
          natural_height: node.naturalHeight,
          mime_type: dataUrl.startsWith('data:image/png;base64,') ? 'image/png' : '',
          data_url: dataUrl,
          error,
        })
      }

      const match = location.pathname.match(/\/c\/([^/?#]+)/)
      return {
        conversation_id: match ? match[1] : '',
        page_url: location.href,
        images,
      }
    }, { maxImages: boundedMaxImages, excludeSrcs })
  }

  async generateImage(prompt, maxImages = 1, timeoutSecs = 900, referencePaths = []) {
    const beforeSources = await this.generatedImageSources()
    const boundedReferencePaths = Array.isArray(referencePaths)
      ? referencePaths.filter((path) => typeof path === 'string' && path.trim()).slice(0, 4)
      : []
    if (boundedReferencePaths.length > 0) {
      await this.sendAttachments(boundedReferencePaths, prompt)
    } else {
      await this.sendText(prompt)
    }
    const deadline = Date.now() + Math.max(60, Number(timeoutSecs) || 900) * 1000
    let latest = null
    while (Date.now() < deadline) {
      latest = await this.extractGeneratedImages(maxImages, beforeSources)
      if (latest.images.some((image) => image.data_url && !image.error)) {
        return latest
      }
      await this.page.waitForTimeout(2000)
    }
    throw new Error(`generated image timeout after ${Math.max(60, Number(timeoutSecs) || 900)}s`)
  }

  async openLogin() {
    const targetUrl = process.env[ENV_BASE_URL] || DEFAULT_URL
    await this.ensureBrowser()
    await this.page.goto(targetUrl, { waitUntil: 'domcontentloaded', timeout: NAV_TIMEOUT_MS })
    await this.page.waitForTimeout(POST_NAV_SETTLE_MS)
    await this.installObserver()
    await this.refreshHealth()
    await this.page.bringToFront().catch(() => {})
    await this.page.evaluate(() => window.focus()).catch(() => {})
    this.emitSessionInfo()
  }

  async openNewConversation() {
    await this.openLogin()
    if (!this.page) {
      throw new Error('page is required after opening ChatGPT')
    }
    if (!nowConversationId(this.page.url())) {
      return
    }

    const newChat = await this.findFirst(this.selectorList('new_chat'), { timeout: 2500 })
    if (newChat) {
      await newChat.click()
      await this.page.waitForTimeout(POST_NAV_SETTLE_MS)
    } else {
      const baseUrl = (process.env[ENV_BASE_URL] || DEFAULT_URL).replace(/\/+$/u, '')
      await this.page.goto(`${baseUrl}/?singulari_new=${Date.now()}`, {
        waitUntil: 'domcontentloaded',
        timeout: NAV_TIMEOUT_MS,
      })
      await this.page.waitForTimeout(POST_NAV_SETTLE_MS)
    }

    await this.installObserver()
    await this.refreshHealth()
    if (this.health.state !== 'ready') {
      throw new Error(this.health.detail || `new conversation not ready: ${this.health.state}`)
    }
    if (nowConversationId(this.page.url())) {
      throw new Error(`failed to open a new ChatGPT conversation: url=${this.page.url()}`)
    }
    this.emitSessionInfo()
  }

  async openSession(conversationId) {
    if (!conversationId) {
      throw new Error('conversation_id is required')
    }
    await this.ensureBrowser()
    const baseUrl = (process.env[ENV_BASE_URL] || DEFAULT_URL).replace(/\/+$/u, '')
    await this.page.goto(`${baseUrl}/c/${conversationId}`, {
      waitUntil: 'domcontentloaded',
      timeout: NAV_TIMEOUT_MS,
    })
    await this.page.waitForTimeout(POST_NAV_SETTLE_MS)
    await this.installObserver()
    await this.refreshHealth()
    this.emitSessionInfo()
  }

  async interrupt() {
    await this.ensureBrowser()
    const stopButton = await this.findFirst(this.selectorList('stop_button'), { timeout: 1000 })
    if (stopButton) {
      await stopButton.click()
    }
  }

  async healthInfo() {
    if (!this.page) {
      return this.health
    }
    await this.refreshHealth()
    return this.health
  }

  sessionInfo() {
    return {
      conversation_id: this.page ? nowConversationId(this.page.url()) : '',
      model: this.currentModel,
      current_reasoning_level: '',
    }
  }
}

const selectors = await loadSelectors()
const worker = new ChatGptWorker(selectors)
await worker.init()

process.stdout.on('error', (error) => {
  if (error instanceof Error && 'code' in error && error.code === 'EPIPE') {
    stdoutBroken = true
    process.exit(0)
  }
  log(`stdout error: ${error instanceof Error ? error.message : String(error)}`)
})

const rl = createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
})

for await (const line of rl) {
  const trimmed = line.trim()
  if (!trimmed) continue

  let request
  try {
    request = JSON.parse(trimmed)
  } catch (error) {
    log(`invalid request JSON: ${error}`)
    continue
  }

  const { id = null, method, params = {} } = request
  try {
    switch (method) {
      case 'send_text':
        await worker.sendText(params.text || '')
        respond(id, null)
        break
      case 'send_attachment':
        await worker.sendAttachments([params.path], params.text || '')
        respond(id, null)
        break
      case 'send_attachments':
        await worker.sendAttachments(params.paths || [], params.text || '')
        respond(id, null)
        break
      case 'extract_generated_images':
        respond(id, await worker.extractGeneratedImages(params.max_images || 4))
        break
      case 'generate_image':
        respond(
          id,
          await worker.generateImage(
            params.prompt || '',
            params.max_images || 1,
            params.timeout_secs || 900,
            params.reference_paths || [],
          ),
        )
        break
      case 'interrupt':
        await worker.interrupt()
        respond(id, null)
        break
      case 'open_login':
        await worker.openLogin()
        respond(id, null)
        break
      case 'open_new_conversation':
        await worker.openNewConversation()
        respond(id, null)
        break
      case 'open_session':
        await worker.openSession(params.conversation_id || '')
        respond(id, null)
        break
      case 'controls':
        respond(id, await worker.controlsInfo())
        break
      case 'debug_probe':
        respond(id, worker.debugProbe())
        break
      case 'debug_controls_dom':
        respond(id, await worker.inspectControlSurface())
        break
      case 'select_model':
        respond(id, await worker.selectModel(params.model || ''))
        break
      case 'select_reasoning':
        respond(id, await worker.selectReasoningLevel(params.reasoning_level || ''))
        break
      case 'health':
        respond(id, await worker.healthInfo())
        break
      case 'session_info':
        respond(id, worker.sessionInfo())
        break
      default:
        respondError(id, `unknown method: ${method}`)
        break
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    emitEvent({
      event: 'error',
      msg: message,
      conversation_id: worker.page ? nowConversationId(worker.page.url()) : '',
    })
    respondError(id, message)
  }
}
