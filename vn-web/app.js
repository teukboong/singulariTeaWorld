const explicitPacketUrl = new URLSearchParams(window.location.search).get("packet");
const currentApiUrl = "/api/vn/current";
const chooseApiUrl = "/api/vn/choose";
const agentPendingApiUrl = "/api/vn/agent/pending";
const runtimeStatusApiUrl = "/api/vn/runtime-status";
const cgGalleryApiUrl = "/api/vn/cg/gallery";
const worldsApiUrl = "/api/vn/worlds";
const selectWorldApiUrl = "/api/vn/worlds/select";
const newWorldApiUrl = "/api/vn/worlds/new";
const saveWorldApiUrl = "/api/vn/worlds/save";
const loadWorldApiUrl = "/api/vn/worlds/load";
const cgRetryApiUrl = "/api/vn/cg/retry";
const launchSeenKey = "singulari.vn.launchSeen";
const launchTransitionMs = 520;
const LOG_PAGE_SIZE = 5;
const SIDE_DOCK_RIGHT_EXIT_GRACE_MS = 900;
const VISUAL_JOB_POLL_MS = 2500;
const RUNTIME_STATUS_POLL_MS = 3000;
const DEFAULT_VIEW_MODE = "text";
const AGENT_WAITING_BADGE = "흐름 수렴 중";
const AGENT_WAITING_COPY = {
  initial: [
    "세계의 흐름에서 첫 장면을 건져올리는 중.",
    "시드가 아직 선택지가 되기 전의 어둠 속에서, 인물과 장소와 첫 사건이 서로의 이름을 맞춰보고 있다.",
  ],
  next: [
    "세계의 흐름에서 다음 장면을 건져올리는 중.",
    "네 선택이 사건의 결을 바꾸는 동안, 아직 말로 고정되지 않은 가능성들이 조용히 가라앉고 있다.",
  ],
};
const DEFAULT_SETTINGS = {
  cgEnabled: true,
  turnCgMode: "guide_auto",
  cgPrompt: "",
  guideChoiceEnabled: true,
  autoReveal: false,
  textScale: "1",
  reduceMotion: false,
};

const state = {
  packet: null,
  apiAvailable: false,
  selectedCommand: "",
  busy: false,
  storyLines: [],
  visibleLineCount: 0,
  choicesRevealed: false,
  turnLogEntries: [],
  logPage: 0,
  baseImagePrompt: "",
  visualAssets: null,
  runtimeStatus: null,
  cgGallery: null,
  worlds: [],
  activeWorldId: "",
  agentPollTimer: null,
  visualPollTimer: null,
  runtimePollTimer: null,
  awaitingAgent: false,
  settings: loadSettings(),
  viewMode: DEFAULT_VIEW_MODE,
};

let sideDockRightExitTimer = null;

const els = {
  shell: document.querySelector(".vn-shell"),
  launchOverlay: document.getElementById("launchOverlay"),
  continueWorldButton: document.getElementById("continueWorldButton"),
  newWorldModeButton: document.getElementById("newWorldModeButton"),
  previousWorldModeButton: document.getElementById("previousWorldModeButton"),
  newWorldPanel: document.getElementById("newWorldPanel"),
  previousWorldPanel: document.getElementById("previousWorldPanel"),
  newWorldTitle: document.getElementById("newWorldTitle"),
  newWorldSeed: document.getElementById("newWorldSeed"),
  startNewWorldButton: document.getElementById("startNewWorldButton"),
  worldList: document.getElementById("worldList"),
  launchStatus: document.getElementById("launchStatus"),
  sideDock: document.getElementById("sideDock"),
  sidePeekButton: document.getElementById("sidePeekButton"),
  worldBackdrop: document.getElementById("worldBackdrop"),
  sceneImage: document.getElementById("sceneImage"),
  worldTitle: document.getElementById("worldTitle"),
  turnId: document.getElementById("turnId"),
  generateTurnCgButton: document.getElementById("generateTurnCgButton"),
  turnCgStatus: document.getElementById("turnCgStatus"),
  locationId: document.getElementById("locationId"),
  eventId: document.getElementById("eventId"),
  outcomeBadge: document.getElementById("outcomeBadge"),
  textWindow: document.querySelector(".text-window"),
  sceneText: document.getElementById("sceneText"),
  previousButton: document.getElementById("previousButton"),
  advanceButton: document.getElementById("advanceButton"),
  choicePanel: document.getElementById("choicePanel"),
  choices: document.getElementById("choices"),
  freeformInput: document.getElementById("freeformInput"),
  freeformButton: document.getElementById("freeformButton"),
  commandOutput: document.getElementById("commandOutput"),
  imagePrompt: document.getElementById("imagePrompt"),
  copyCommand: document.getElementById("copyCommand"),
  copyPrompt: document.getElementById("copyPrompt"),
  sideTabs: Array.from(document.querySelectorAll("[data-side-tab]")),
  sidePanes: {
    status: document.getElementById("sideStatus"),
    gallery: document.getElementById("sideGallery"),
    textlog: document.getElementById("sideTextLog"),
    settings: document.getElementById("sideSettings"),
    console: document.getElementById("sideConsole"),
  },
  selectedChoiceCard: document.getElementById("selectedChoiceCard"),
  protagonistStatus: document.getElementById("protagonistStatus"),
  currentTextLog: document.getElementById("currentTextLog"),
  fullTurnMarkdown: document.getElementById("fullTurnMarkdown"),
  monitoringGrid: document.getElementById("monitoringGrid"),
  cgGallery: document.getElementById("cgGallery"),
  galleryStatus: document.getElementById("galleryStatus"),
  refreshGalleryButton: document.getElementById("refreshGalleryButton"),
  runtimeStatusPanel: document.getElementById("runtimeStatusPanel"),
  runtimeDetails: document.getElementById("runtimeDetails"),
  scanList: document.getElementById("scanList"),
  turnLogList: document.getElementById("turnLogList"),
  logPrevPage: document.getElementById("logPrevPage"),
  logNextPage: document.getElementById("logNextPage"),
  logPageLabel: document.getElementById("logPageLabel"),
  codexStructured: document.getElementById("codexStructured"),
  settingCgEnabled: document.getElementById("settingCgEnabled"),
  settingTurnCgMode: document.getElementById("settingTurnCgMode"),
  settingCgPrompt: document.getElementById("settingCgPrompt"),
  settingGuideChoice: document.getElementById("settingGuideChoice"),
  settingAutoReveal: document.getElementById("settingAutoReveal"),
  settingTextScale: document.getElementById("settingTextScale"),
  settingReduceMotion: document.getElementById("settingReduceMotion"),
  settingMainMenu: document.getElementById("settingMainMenu"),
  settingSaveWorld: document.getElementById("settingSaveWorld"),
  settingLoadWorld: document.getElementById("settingLoadWorld"),
  settingLoadWorldBundle: document.getElementById("settingLoadWorldBundle"),
  settingWorldStatus: document.getElementById("settingWorldStatus"),
  cgJobPanel: document.getElementById("cgJobPanel"),
  cgJobStatus: document.getElementById("cgJobStatus"),
  requestTurnCgRetry: document.getElementById("requestTurnCgRetry"),
  viewModeButtons: Array.from(document.querySelectorAll("[data-view-mode]")),
};

init();

async function init() {
  try {
    const packet = await loadInitialPacket();
    renderPacket(packet);
    await hydrateWorldLauncher();
    await hydrateAgentPendingState();
  } catch (error) {
    renderLoadError(error);
  }
  bindLauncher();
  els.previousButton.addEventListener("click", retreatStory);
  els.advanceButton.addEventListener("click", advanceStory);
  els.textWindow.addEventListener("click", (event) => {
    if (event.target.closest(".text-nav")) {
      return;
    }
    advanceStory();
  });
  els.freeformButton.addEventListener("click", chooseFreeform);
  els.generateTurnCgButton.addEventListener("click", () => requestTurnCgRetry());
  els.refreshGalleryButton.addEventListener("click", () => refreshCgGallery({ force: true }));
  els.freeformInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      chooseFreeform();
    }
  });
  els.copyCommand.addEventListener("click", () => copyText(els.commandOutput.value));
  els.copyPrompt.addEventListener("click", () => copyText(els.imagePrompt.value));
  els.logPrevPage.addEventListener("click", () => changeLogPage(-1));
  els.logNextPage.addEventListener("click", () => changeLogPage(1));
  bindSettings();
  document.addEventListener("keydown", handleGlobalKeydown);
  for (const button of els.viewModeButtons) {
    button.addEventListener("click", () => setViewMode(button.dataset.viewMode));
  }
  els.sidePeekButton.addEventListener("click", toggleSideDock);
  els.sideDock.addEventListener("pointerenter", openSideDock);
  els.sideDock.addEventListener("pointerleave", closeSideDockFromPointerLeave);
  document.addEventListener("pointerdown", closeSideDockFromOutside);
  for (const tab of els.sideTabs) {
    tab.addEventListener("click", () => {
      selectSideTab(tab.dataset.sideTab);
    });
  }
  applySettings();
  setViewMode(DEFAULT_VIEW_MODE);
}

async function loadInitialPacket() {
  if (explicitPacketUrl) {
    return fetchJson(explicitPacketUrl);
  }
  const packet = await fetchJson(currentApiUrl);
  state.apiAvailable = true;
  return packet;
}

function renderPacket(packet) {
  state.packet = packet;
  state.awaitingAgent = false;
  state.activeWorldId = packet.world_id;
  document.title = `${packet.title} · ${packet.turn_id}`;
  els.worldTitle.textContent = packet.title;
  els.turnId.textContent = packet.turn_id;
  els.locationId.textContent = packet.scene.location;
  els.eventId.textContent = packet.scene.current_event;
  els.outcomeBadge.textContent = packet.scene.adjudication?.outcome || packet.mode;
  applyPacketVisualState(packet);
  els.shell.classList.remove("is-loading", "is-awaiting-agent");

  state.storyLines = buildStoryLines(packet);
  state.visibleLineCount = state.storyLines.length ? 1 : 0;
  state.choicesRevealed = false;
  renderStoryProgress();

  renderChoices(packet);
  renderCurrentTextLog();
  state.logPage = 0;

  const firstChoice = packet.choices?.find((choice) => !choice.requires_inline_text);
  if (firstChoice) {
    selectCommand(firstChoice.command_template);
  }
  renderCodexSurface(packet);
  if (state.settings.autoReveal) {
    revealChoices();
  }
  refreshRuntimeStatus();
  refreshCgGallery();
  scheduleRuntimeStatusPoll();
  startVisualJobPolling();
}

async function hydrateWorldLauncher() {
  if (!state.apiAvailable || explicitPacketUrl) {
    markLaunchSeen();
    return;
  }
  await refreshWorldList();
  renderWorldList();
  if (!hasLaunchSeen()) {
    showLaunchOverlay("previous");
  }
}

async function hydrateAgentPendingState() {
  if (!state.apiAvailable || explicitPacketUrl) {
    return;
  }
  try {
    const status = await fetchJson(agentPendingApiUrl);
    if (status.status === "waiting_agent") {
      handleWaitingAgentTurn(status, { initial: state.packet?.turn_id === "turn_0000" });
    }
  } catch (error) {
    console.warn(`agent pending hydration failed: ${error.message}`);
  }
}

function bindLauncher() {
  els.continueWorldButton.addEventListener("click", continueCurrentWorld);
  els.newWorldModeButton.addEventListener("click", () => showLaunchMode("new"));
  els.previousWorldModeButton.addEventListener("click", () => showLaunchMode("previous"));
  els.startNewWorldButton.addEventListener("click", startNewWorld);
}

function showLaunchOverlay(mode) {
  showLaunchMode(mode);
  els.launchOverlay.classList.remove("is-entering");
  els.launchOverlay.hidden = false;
}

function hideLaunchOverlay() {
  els.launchOverlay.classList.remove("is-entering");
  els.launchOverlay.hidden = true;
  setLaunchStatus("");
}

function showLaunchMode(mode) {
  const activeMode = mode === "new" ? "new" : "previous";
  els.newWorldPanel.hidden = activeMode !== "new";
  els.previousWorldPanel.hidden = activeMode !== "previous";
  els.newWorldModeButton.classList.toggle("is-active", activeMode === "new");
  els.previousWorldModeButton.classList.toggle("is-active", activeMode === "previous");
}

async function continueCurrentWorld() {
  if (state.busy) {
    return;
  }
  state.busy = true;
  setLaunchStatus("세계에 접속하는 중...");
  try {
    await enterVnFromLauncher();
  } finally {
    state.busy = false;
  }
}

async function refreshWorldList() {
  const response = await fetchJson(worldsApiUrl);
  state.worlds = response.worlds || [];
  state.activeWorldId = response.active_world_id || state.activeWorldId;
}

function renderWorldList() {
  els.worldList.replaceChildren();
  if (!state.worlds.length) {
    appendEmpty(els.worldList, "아직 선택할 이전 세계가 없다.");
    return;
  }
  for (const world of state.worlds) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "world-option";
    if (world.world_id === state.activeWorldId) {
      button.classList.add("is-active");
    }
    const title = document.createElement("strong");
    title.textContent = world.title;
    const meta = document.createElement("span");
    meta.textContent = `${world.world_id} · ${world.turn_id} · ${world.phase}`;
    button.append(title, meta);
    button.addEventListener("click", () => selectWorld(world.world_id));
    els.worldList.append(button);
  }
}

async function selectWorld(worldId) {
  if (state.busy) {
    return;
  }
  state.busy = true;
  setLaunchStatus("이전 세계를 여는 중...");
  try {
    const response = await fetchJson(selectWorldApiUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ world_id: worldId }),
    });
    await applyWorldSwitchResponse(response);
  } catch (error) {
    setLaunchStatus(`세계 선택 실패: ${error.message}`);
  } finally {
    state.busy = false;
  }
}

async function startNewWorld() {
  if (state.busy) {
    return;
  }
  const seedText = els.newWorldSeed.value.trim();
  if (!seedText) {
    setLaunchStatus("새 세계 시드를 입력해야 해.");
    return;
  }
  state.busy = true;
  setLaunchStatus("세계의 흐름에서 첫 장면을 건져올리는 중...");
  try {
    const title = els.newWorldTitle.value.trim();
    const response = await fetchJson(newWorldApiUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ seed_text: seedText, title: title || null }),
    });
    await applyWorldSwitchResponse(response);
  } catch (error) {
    setLaunchStatus(`새 세계 생성 실패: ${error.message}`);
  } finally {
    state.busy = false;
  }
}

async function saveCurrentWorld() {
  if (state.busy) {
    return;
  }
  if (!state.apiAvailable || explicitPacketUrl) {
    setWorldOperationStatus("세계 저장은 VN 서버에 연결된 화면에서만 가능해.");
    return;
  }
  state.busy = true;
  setWorldOperationStatus("세계 저장 중...");
  try {
    const response = await fetchJson(saveWorldApiUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ world_id: state.activeWorldId || state.packet?.world_id || null }),
    });
    state.worlds = response.worlds || state.worlds;
    renderWorldList();
    if (response.bundle_dir) {
      els.settingLoadWorldBundle.value = response.bundle_dir;
    }
    setWorldOperationStatus(`세계 저장 완료: ${response.bundle_dir}`);
  } catch (error) {
    setWorldOperationStatus(`세계 저장 실패: ${error.message}`);
  } finally {
    state.busy = false;
  }
}

async function loadWorldFromBundle() {
  if (state.busy) {
    return;
  }
  if (!state.apiAvailable || explicitPacketUrl) {
    setWorldOperationStatus("세계 불러오기는 VN 서버에 연결된 화면에서만 가능해.");
    return;
  }
  const bundle = els.settingLoadWorldBundle.value.trim();
  if (!bundle) {
    setSettingWorldStatus("불러올 세계 번들 경로를 입력해야 해.");
    els.settingLoadWorldBundle.focus();
    return;
  }
  state.busy = true;
  setWorldOperationStatus("세계 불러오는 중...");
  try {
    const response = await fetchJson(loadWorldApiUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ bundle }),
    });
    await applyWorldSwitchResponse(response);
    setSettingWorldStatus(`세계 불러오기 완료: ${response.active_world_id}`);
  } catch (error) {
    setWorldOperationStatus(`세계 불러오기 실패: ${error.message}`);
  } finally {
    state.busy = false;
  }
}

async function requestTurnCgRetry(options = {}) {
  const openConsole = Boolean(options.openConsole);
  if (state.busy || !state.packet) {
    return;
  }
  if (!state.apiAvailable || explicitPacketUrl) {
    setWorldOperationStatus("턴 CG 생성은 VN 서버에 연결된 화면에서만 가능해.");
    setTurnCgStatus("error", "서버 연결 필요");
    return;
  }
  state.busy = true;
  setTurnCgStatus("loading", "요청 중");
  syncTurnCgButton();
  setCgJobStatus("턴 CG 요청 중");
  try {
    const packet = await fetchJson(cgRetryApiUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ turn_id: state.packet.turn_id }),
    });
    renderPacket(packet);
    if (openConsole) {
      selectSideTab("console");
    }
    setCgJobStatus("이미지 연결 대기 중");
    startVisualJobPolling();
  } catch (error) {
    setCgJobStatus(`턴 CG 요청 실패: ${error.message}`);
    setTurnCgStatus("error", "요청 실패");
  } finally {
    state.busy = false;
    syncTurnCgButton();
  }
}

async function openMainMenu(mode = "previous") {
  if (!state.apiAvailable || explicitPacketUrl) {
    setSettingWorldStatus("메인 메뉴는 VN 서버에 연결된 화면에서만 열 수 있어.");
    return;
  }
  closeSideDock();
  try {
    await refreshWorldList();
    renderWorldList();
  } catch (error) {
    setSettingWorldStatus(`세계 목록 갱신 실패: ${error.message}`);
  }
  showLaunchOverlay(mode);
}

async function applyWorldSwitchResponse(response) {
  state.worlds = response.worlds || state.worlds;
  state.activeWorldId = response.active_world_id || response.packet?.world_id || state.activeWorldId;
  if (response.packet) {
    renderPacket(response.packet);
  }
  renderWorldList();
  await enterVnFromLauncher();
  if (response.agent_pending) {
    handleWaitingAgentTurn(response.agent_pending, { initial: true });
  }
}

async function enterVnFromLauncher() {
  await playLaunchEntryTransition();
  markLaunchSeen();
  hideLaunchOverlay();
}

async function playLaunchEntryTransition() {
  if (state.settings.reduceMotion || els.launchOverlay.hidden) {
    return;
  }
  els.launchOverlay.classList.remove("is-entering");
  void els.launchOverlay.offsetWidth;
  els.launchOverlay.classList.add("is-entering");
  await delay(launchTransitionMs);
  els.launchOverlay.classList.remove("is-entering");
}

function setLaunchStatus(text) {
  els.launchStatus.textContent = text;
}

function setSettingWorldStatus(text) {
  els.settingWorldStatus.textContent = text;
}

function setWorldOperationStatus(text) {
  setLaunchStatus(text);
  setSettingWorldStatus(text);
}

function setCgJobStatus(text) {
  if (els.cgJobStatus) {
    els.cgJobStatus.textContent = text || "";
  }
}

function delay(ms) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function applyPacketVisualState(packet) {
  state.baseImagePrompt = packet.image?.image_prompt || "";
  state.visualAssets = packet.visual_assets || null;
  syncImagePrompt();
  applyWorldVisuals(packet);
  applySceneImage(packet);
  renderCgJobPanel();
}

function refreshVisualStateFromPacket(packet) {
  if (
    !state.packet ||
    packet.world_id !== state.packet.world_id ||
    packet.turn_id !== state.packet.turn_id
  ) {
    renderPacket(packet);
    return;
  }
  state.packet.image = packet.image;
  state.packet.visual_assets = packet.visual_assets || null;
  state.packet.generated_at = packet.generated_at;
  applyPacketVisualState(state.packet);
}

function applySceneImage(packet) {
  if (packet.image?.existing_image_url) {
    els.sceneImage.classList.add("has-image");
    els.sceneImage.style.backgroundImage = `url("${cssUrl(packet.image.existing_image_url)}")`;
  } else {
    els.sceneImage.classList.remove("has-image");
    els.sceneImage.style.backgroundImage = "";
  }
}

function applyWorldVisuals(packet) {
  const visuals = packet.visual_assets || {};
  const menu = visuals.menu_background;
  const stage = visuals.stage_background;
  const menuUrl = menu?.exists ? menu.asset_url : "";
  const stageUrl = stage?.exists ? stage.asset_url : "";
  const worldTheme = chooseWorldTheme(packet);
  els.shell.dataset.worldTheme = worldTheme;
  els.shell.classList.toggle("has-grain", worldTheme !== "default");
  els.shell.style.setProperty("--world-menu-image", cssImageValue(menuUrl));
  els.shell.style.setProperty("--world-stage-image", cssImageValue(stageUrl));
  els.shell.classList.toggle("has-world-stage", Boolean(stageUrl));
  if (els.worldBackdrop) {
    els.worldBackdrop.hidden = false;
  }
  const paletteSource = menuUrl || stageUrl;
  if (paletteSource) {
    extractPaletteFromImage(paletteSource).then(applyExtractedPalette).catch(applyMonochromePalette);
  } else {
    applyMonochromePalette();
  }
}

function chooseWorldTheme(packet) {
  const source = [
    packet.title,
    packet.scene?.location,
    packet.scene?.current_event,
    packet.scene?.status,
    packet.image?.image_prompt,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  if (/court|궁정|왕궁|귀족|combat|battle|전투|검|피|blood|ember|화염|불/.test(source)) {
    return "ember-court";
  }
  if (/river|강|post-war|전쟁|폐허|horror|공포|melancholy|우울|망령/.test(source)) {
    return "long-river";
  }
  if (/glass|garden|arcology|future|near-future|surveillance|유리|정원|미래|감시|의식/.test(source)) {
    return "glass-garden";
  }
  if (/dawn|modern|reincarnation|slice|새벽|아침|현대|전생|일상/.test(source)) {
    return "dawn";
  }
  return "default";
}

function cssImageValue(url) {
  return url ? `url("${cssUrl(url)}")` : "none";
}

async function extractPaletteFromImage(url) {
  const image = await loadImage(url);
  const canvas = document.createElement("canvas");
  const size = 32;
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  context.drawImage(image, 0, 0, size, size);
  const data = context.getImageData(0, 0, size, size).data;
  let accent = { r: 216, g: 216, b: 216, score: -1 };
  let totalR = 0;
  let totalG = 0;
  let totalB = 0;
  let count = 0;
  for (let index = 0; index < data.length; index += 4) {
    const alpha = data[index + 3] / 255;
    if (alpha < 0.4) {
      continue;
    }
    const r = data[index];
    const g = data[index + 1];
    const b = data[index + 2];
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const saturation = max - min;
    const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    totalR += r;
    totalG += g;
    totalB += b;
    count += 1;
    const score = saturation * 1.4 + (255 - Math.abs(luma - 168));
    if (score > accent.score && luma > 48 && luma < 230) {
      const candidate = { r, g, b };
      if (!isWarmEarthTone(candidate)) {
        accent = { r, g, b, score };
      }
    }
  }
  if (!count) {
    return null;
  }
  const base = {
    r: Math.round(totalR / count),
    g: Math.round(totalG / count),
    b: Math.round(totalB / count),
  };
  const baseTone = isWarmEarthTone(base)
    ? mixColors(base, { r: 170, g: 174, b: 180 }, 0.82)
    : base;
  const radialFrom = mixColors(baseTone, { r: 32, g: 32, b: 36 }, 0.72);
  const radialTo = mixColors(baseTone, { r: 8, g: 8, b: 10 }, 0.9);
  return {
    accent: rgbString(accent),
    warmAccent: rgbString(mixColors(accent, { r: 232, g: 218, b: 166 }, 0.44)),
    mood: rgbaString(accent, 0.055),
    radialFrom: rgbString(radialFrom),
    radialTo: rgbString(radialTo),
  };
}

function isWarmEarthTone(color) {
  const max = Math.max(color.r, color.g, color.b);
  const min = Math.min(color.r, color.g, color.b);
  const saturation = max - min;
  if (saturation < 24) {
    return false;
  }
  const luma = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
  const hue = hueDegrees(color);
  return hue >= 18 && hue <= 72 && color.r > color.b + 12 && luma > 36 && luma < 238;
}

function hueDegrees(color) {
  const r = color.r / 255;
  const g = color.g / 255;
  const b = color.b / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  if (delta === 0) {
    return 0;
  }
  let hue;
  if (max === r) {
    hue = 60 * (((g - b) / delta) % 6);
  } else if (max === g) {
    hue = 60 * ((b - r) / delta + 2);
  } else {
    hue = 60 * ((r - g) / delta + 4);
  }
  return hue < 0 ? hue + 360 : hue;
}

function loadImage(url) {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = reject;
    image.src = url;
  });
}

function applyExtractedPalette(palette) {
  resetWorldPaletteOverrides();
  if (!palette) {
    return;
  }
  els.shell.style.setProperty("--world-accent", palette.accent);
  els.shell.style.setProperty("--world-accent-warm", palette.warmAccent);
  els.shell.style.setProperty("--world-mood", palette.mood);
  els.shell.style.setProperty("--world-radial-from", palette.radialFrom);
  els.shell.style.setProperty("--world-radial-to", palette.radialTo);
}

function applyMonochromePalette() {
  resetWorldPaletteOverrides();
}

function resetWorldPaletteOverrides() {
  for (const propertyName of [
    "--world-accent",
    "--world-accent-warm",
    "--world-mood",
    "--world-radial-from",
    "--world-radial-to",
  ]) {
    els.shell.style.removeProperty(propertyName);
  }
}

function mixColors(left, right, rightWeight) {
  const leftWeight = 1 - rightWeight;
  return {
    r: Math.round(left.r * leftWeight + right.r * rightWeight),
    g: Math.round(left.g * leftWeight + right.g * rightWeight),
    b: Math.round(left.b * leftWeight + right.b * rightWeight),
  };
}

function rgbString(color) {
  return `rgb(${color.r}, ${color.g}, ${color.b})`;
}

function rgbaString(color, alpha) {
  return `rgba(${color.r}, ${color.g}, ${color.b}, ${alpha})`;
}

function hasLaunchSeen() {
  try {
    return sessionStorage.getItem(launchSeenKey) === "1";
  } catch (error) {
    console.warn(`launch session read failed: ${error.message}`);
    return false;
  }
}

function markLaunchSeen() {
  try {
    sessionStorage.setItem(launchSeenKey, "1");
  } catch (error) {
    console.warn(`launch session write failed: ${error.message}`);
  }
}

function buildStoryLines(packet) {
  const lines = (packet.scene.text_blocks || [])
    .map(cleanNarrativeLine)
    .filter(Boolean);
  if (lines.length) {
    return lines;
  }
  return [packet.scene.status || "장면이 잠시 숨을 고른다."];
}

function cleanNarrativeLine(text) {
  return String(text)
    .replace(/\s*숨겨진 진실은 아직 베일 뒤에 남겨둔다\.?/g, "")
    .trim();
}

function renderStoryProgress() {
  els.sceneText.replaceChildren();
  for (const text of state.storyLines.slice(0, state.visibleLineCount)) {
    const paragraph = document.createElement("p");
    paragraph.textContent = text;
    els.sceneText.append(paragraph);
  }
  els.choicePanel.hidden = state.awaitingAgent || !state.choicesRevealed;
  els.shell.classList.toggle("choices-ready", !state.awaitingAgent && state.choicesRevealed);
  els.previousButton.disabled = state.visibleLineCount <= 1 && !state.choicesRevealed;
  els.advanceButton.disabled =
    state.awaitingAgent || state.choicesRevealed || !state.storyLines.length;
}

function renderCurrentTextLog() {
  els.currentTextLog.replaceChildren();
  for (const text of state.storyLines) {
    const paragraph = document.createElement("p");
    paragraph.textContent = text;
    els.currentTextLog.append(paragraph);
  }
}

function renderChoices(packet) {
  els.choices.replaceChildren();
  els.freeformInput.value = "";
  for (const choice of packet.choices || []) {
    if (!state.settings.guideChoiceEnabled && choice.slot === 4) {
      continue;
    }
    if (choice.requires_inline_text) {
      els.freeformInput.placeholder = choice.input_template;
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = "choice-button";
    button.innerHTML = `<span class="choice-label"></span><span class="choice-intent"></span>`;
    button.querySelector(".choice-label").textContent = `${choice.slot}. ${choice.tag}`;
    button.querySelector(".choice-intent").textContent = choice.intent;
    button.addEventListener("click", () => chooseSlot(choice));
    els.choices.append(button);
  }
}

function advanceStory() {
  if (state.busy || !state.packet) {
    return;
  }
  if (state.choicesRevealed) {
    return;
  }
  if (state.visibleLineCount < state.storyLines.length) {
    state.visibleLineCount += 1;
    renderStoryProgress();
    return;
  }
  if (state.awaitingAgent) {
    return;
  }
  revealChoices();
}

function retreatStory() {
  if (state.busy || !state.packet) {
    return;
  }
  if (state.choicesRevealed) {
    state.choicesRevealed = false;
    renderStoryProgress();
    return;
  }
  if (state.visibleLineCount > 1) {
    state.visibleLineCount -= 1;
    renderStoryProgress();
  }
}

function revealAllText() {
  if (!state.packet) {
    return;
  }
  state.visibleLineCount = state.storyLines.length;
  renderStoryProgress();
}

function revealChoices() {
  if (state.awaitingAgent) {
    return;
  }
  revealAllText();
  state.choicesRevealed = true;
  renderStoryProgress();
}

function setViewMode(mode) {
  state.viewMode = mode === "text" ? "text" : "cg";
  applyViewMode();
}

function applyViewMode() {
  els.shell.classList.toggle("text-focus", state.viewMode === "text");
  els.shell.classList.toggle("cg-focus", state.viewMode !== "text");
  for (const button of els.viewModeButtons) {
    const active = button.dataset.viewMode === state.viewMode;
    button.classList.toggle("is-active", active);
    button.setAttribute("aria-pressed", String(active));
  }
}

function toggleViewMode() {
  setViewMode(state.viewMode === "text" ? "cg" : "text");
}

function renderCodexSurface(packet) {
  const surface = packet.codex_surface || {};
  els.fullTurnMarkdown.textContent =
    surface.full_markdown || "이 턴의 Codex 렌더 원문이 아직 없다.";
  renderCurrentTurnStatus(surface.current_turn || {});
  renderProtagonistStatus(surface.protagonist || {});
  renderMonitoring(surface, packet);
  renderScan(surface.scan_targets || []);
  state.turnLogEntries = surface.turn_log || [];
  renderTurnLogPage();
  renderCodexView(surface.codex_view, surface.redaction_policy);
}

function renderCurrentTurnStatus(currentTurn) {
  els.selectedChoiceCard.replaceChildren();
  const summary = document.createElement("p");
  summary.textContent = currentTurn.summary || state.packet?.scene?.status || "이번 턴 요약이 아직 없다.";
  els.selectedChoiceCard.append(summary);
}

function renderProtagonistStatus(protagonist) {
  els.protagonistStatus.replaceChildren();
  if (Array.isArray(protagonist.dashboard_rows) && protagonist.dashboard_rows.length) {
    const statusWindow = document.createElement("div");
    statusWindow.className = "status-window";
    statusWindow.dataset.schema = protagonist.status_schema || "singulari.vn_protagonist_status.v1";
    for (const row of protagonist.dashboard_rows) {
      const rowElement = document.createElement("div");
      rowElement.className = "status-row";
      rowElement.dataset.row = row.row_id;
      for (const cell of row.cells || []) {
        const cellElement = document.createElement("div");
        cellElement.className = "status-cell";
        const label = document.createElement("span");
        label.textContent = sanitizeUiLabel(cell.label || cell.key);
        const value = document.createElement("strong");
        value.textContent = cell.value || "미정";
        cellElement.append(label, value);
        rowElement.append(cellElement);
      }
      statusWindow.append(rowElement);
    }
    els.protagonistStatus.append(statusWindow);
    return;
  }
  appendKeyValue(els.protagonistStatus, "phase", protagonist.phase);
  appendKeyValue(els.protagonistStatus, "location", protagonist.location);
  appendKeyValue(els.protagonistStatus, "event", protagonist.current_event);
  appendKeyValue(els.protagonistStatus, "progress", protagonist.current_event_progress);
  appendListValue(els.protagonistStatus, "body", protagonist.body);
  appendListValue(els.protagonistStatus, "mind", protagonist.mind);
  appendListValue(els.protagonistStatus, "inventory", protagonist.inventory);
  appendListValue(els.protagonistStatus, "open questions", protagonist.open_questions);
}

function sanitizeUiLabel(value) {
  const label = String(value || "")
    .replace(/[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]/gu, "")
    .replace(/\s+/g, " ")
    .trim();
  return label || "상태";
}

function renderMonitoring(surface, packet) {
  const dashboard = surface.dashboard || {};
  els.monitoringGrid.replaceChildren();
  appendKeyValue(els.monitoringGrid, "world", packet.title);
  appendKeyValue(els.monitoringGrid, "packet", packet.schema_version);
  appendKeyValue(els.monitoringGrid, "mode", packet.mode);
  appendKeyValue(els.monitoringGrid, "phase", dashboard.phase);
  appendKeyValue(els.monitoringGrid, "event", dashboard.current_event);
  appendKeyValue(els.monitoringGrid, "outcome", surface.adjudication?.outcome);
  appendKeyValue(els.monitoringGrid, "hidden filter", packet.hidden_filter?.policy);
  appendKeyValue(els.monitoringGrid, "generated", packet.generated_at);
}

async function refreshRuntimeStatus() {
  if (!state.apiAvailable || explicitPacketUrl || !els.runtimeStatusPanel) {
    return;
  }
  try {
    const status = await fetchJson(runtimeStatusApiUrl);
    state.runtimeStatus = status;
    renderRuntimeStatus(status);
  } catch (error) {
    renderRuntimeStatus({
      narrative: { label: "Codex 연결 필요", status: "needs_connection" },
      visual: { label: "Codex 이미지 연결 필요", status: "needs_connection" },
      details: { error: error.message },
    });
  }
}

function scheduleRuntimeStatusPoll() {
  if (state.runtimePollTimer) {
    window.clearTimeout(state.runtimePollTimer);
  }
  if (!state.apiAvailable || explicitPacketUrl) {
    return;
  }
  state.runtimePollTimer = window.setTimeout(async () => {
    await refreshRuntimeStatus();
    scheduleRuntimeStatusPoll();
  }, RUNTIME_STATUS_POLL_MS);
}

function renderRuntimeStatus(status) {
  if (!els.runtimeStatusPanel) {
    return;
  }
  els.runtimeStatusPanel.replaceChildren();
  const narrative = status?.narrative || {};
  const visual = status?.visual || {};
  els.runtimeStatusPanel.append(
    runtimeStatusPill("서사", narrative.label || "Codex 연결 필요", narrative.status),
    runtimeStatusPill("CG", visual.label || "Codex 이미지 연결 필요", visual.status),
  );
  if (els.runtimeDetails) {
    els.runtimeDetails.textContent = JSON.stringify(status?.details || {}, null, 2);
  }
}

function runtimeStatusPill(label, value, status) {
  const pill = document.createElement("article");
  pill.className = "runtime-status-pill";
  if (status) {
    pill.dataset.status = status;
  }
  const key = document.createElement("span");
  key.textContent = label;
  const text = document.createElement("strong");
  text.textContent = value;
  pill.append(key, text);
  return pill;
}

function renderScan(scanTargets) {
  els.scanList.replaceChildren();
  if (!scanTargets.length) {
    appendEmpty(els.scanList, "아직 표시할 감각 스캔이 없다.");
    return;
  }
  for (const target of scanTargets) {
    const row = document.createElement("article");
    row.className = "scan-entry";
    const title = document.createElement("strong");
    title.textContent = target.target;
    const meta = document.createElement("span");
    meta.textContent = `${target.class} · ${target.distance}`;
    const thought = document.createElement("p");
    thought.textContent = target.thought;
    row.append(title, meta, thought);
    els.scanList.append(row);
  }
}

function renderTurnLogPage() {
  els.turnLogList.replaceChildren();
  if (!state.turnLogEntries.length) {
    els.logPrevPage.disabled = true;
    els.logNextPage.disabled = true;
    els.logPageLabel.textContent = "0 / 0";
    appendEmpty(els.turnLogList, "아직 기록된 턴 로그가 없다.");
    return;
  }
  const pageCount = Math.max(1, Math.ceil(state.turnLogEntries.length / LOG_PAGE_SIZE));
  state.logPage = Math.min(Math.max(state.logPage, 0), pageCount - 1);
  const reverseEntries = [...state.turnLogEntries].reverse();
  const start = state.logPage * LOG_PAGE_SIZE;
  const pageEntries = reverseEntries.slice(start, start + LOG_PAGE_SIZE);
  els.logPrevPage.disabled = state.logPage >= pageCount - 1;
  els.logNextPage.disabled = state.logPage === 0;
  els.logPageLabel.textContent = `${state.logPage + 1} / ${pageCount}`;
  for (const entry of pageEntries) {
    const row = document.createElement("article");
    row.className = "turn-log-entry";
    const title = document.createElement("strong");
    title.textContent = `${entry.turn_id} · ${entry.input_kind}`;
    const input = document.createElement("p");
    input.textContent = entry.input;
    const meta = document.createElement("span");
    meta.textContent = [entry.selected_choice, entry.canon_event_id, entry.created_at]
      .filter(Boolean)
      .join(" · ");
    row.append(title, input, meta);
    if (entry.render_markdown) {
      const details = document.createElement("details");
      details.className = "turn-log-render";
      details.addEventListener("toggle", syncTextLogExpansion);
      const summary = document.createElement("summary");
      summary.textContent = "서술 보기";
      const markdown = document.createElement("pre");
      markdown.textContent = entry.render_markdown;
      details.append(summary, markdown);
      row.append(details);
    }
    els.turnLogList.append(row);
  }
}

function changeLogPage(delta) {
  const pageCount = Math.max(1, Math.ceil(state.turnLogEntries.length / LOG_PAGE_SIZE));
  state.logPage = Math.min(Math.max(state.logPage + delta, 0), pageCount - 1);
  renderTurnLogPage();
}

function renderCodexView(view, redactionPolicy) {
  els.codexStructured.replaceChildren();
  if (redactionPolicy) {
    const policy = document.createElement("p");
    policy.className = "redaction-policy";
    policy.textContent = redactionPolicy;
    els.codexStructured.append(policy);
  }
  if (!view) {
    appendEmpty(els.codexStructured, "5번 기록을 선택하면 공개 기록이 여기에 열린다.");
    return;
  }
  appendCodexSection(
    "주인공의 연대기",
    view.protagonist_timeline,
    (item) => `${item.turn_id} ${item.event_id} [${item.kind}] ${item.summary}`,
  );
  appendCodexSection(
    "세계 연감",
    view.world_almanac,
    (item) => `${item.fact_id}: ${item.subject}.${item.predicate} = ${item.object}`,
  );
  appendCodexSection(
    "세계 청사진",
    view.world_blueprint,
    (item) => `${item.entity_id} · ${item.entity_type} · ${item.name} · ${item.status}`,
  );
  appendCodexSection(
    "실시간 분석",
    view.realtime_analysis,
    (item) => `${item.label}: ${item.value}`,
  );
  appendCodexSection(
    "관련 항목 추천",
    view.related_recommendations,
    (item) => `${item.source} -> ${item.target}: ${item.reason}`,
  );
}

function appendCodexSection(title, items, formatItem) {
  const section = document.createElement("section");
  section.className = "codex-section";
  const heading = document.createElement("h3");
  heading.textContent = title;
  section.append(heading);
  if (!items?.length) {
    const empty = document.createElement("p");
    empty.textContent = "아직 공개된 항목이 없다.";
    section.append(empty);
  } else {
    const list = document.createElement("ul");
    for (const item of items) {
      const listItem = document.createElement("li");
      listItem.textContent = formatItem(item);
      list.append(listItem);
    }
    section.append(list);
  }
  els.codexStructured.append(section);
}

function appendKeyValue(parent, key, value) {
  const cell = document.createElement("div");
  cell.className = "dashboard-cell";
  const label = document.createElement("span");
  label.textContent = key;
  const body = document.createElement("strong");
  body.textContent = value || "미정";
  cell.append(label, body);
  parent.append(cell);
}

function appendListValue(parent, key, values) {
  const value = Array.isArray(values) && values.length ? values.join("\n") : "없음";
  appendKeyValue(parent, key, value);
}

function appendEmpty(parent, text) {
  const empty = document.createElement("p");
  empty.className = "empty-line";
  empty.textContent = text;
  parent.append(empty);
}

function selectSideTab(target) {
  for (const tab of els.sideTabs) {
    tab.classList.toggle("is-active", tab.dataset.sideTab === target);
  }
  for (const [name, pane] of Object.entries(els.sidePanes)) {
    pane.classList.toggle("is-active", name === target);
  }
  if (target !== "textlog") {
    els.sideDock.classList.remove("is-log-expanded");
  } else {
    syncTextLogExpansion();
  }
  if (target === "gallery") {
    refreshCgGallery();
  }
}

function toggleSideDock() {
  const shouldOpen = !els.sideDock.classList.contains("is-open");
  setSideDockOpen(shouldOpen);
}

function openSideDock() {
  clearSideDockRightExitTimer();
  setSideDockOpen(true);
}

function closeSideDock() {
  clearSideDockRightExitTimer();
  setSideDockOpen(false);
  if (document.activeElement && els.sideDock.contains(document.activeElement)) {
    document.activeElement.blur();
  }
}

function closeSideDockFromPointerLeave(event) {
  const dockRect = els.sideDock.getBoundingClientRect();
  const withinDockHeight = event.clientY >= dockRect.top && event.clientY <= dockRect.bottom;
  const exitedThroughRightEdge = withinDockHeight && event.clientX >= window.innerWidth - 1;
  if (exitedThroughRightEdge) {
    clearSideDockRightExitTimer();
    sideDockRightExitTimer = window.setTimeout(closeSideDock, SIDE_DOCK_RIGHT_EXIT_GRACE_MS);
    return;
  }
  closeSideDock();
}

function clearSideDockRightExitTimer() {
  if (sideDockRightExitTimer === null) {
    return;
  }
  window.clearTimeout(sideDockRightExitTimer);
  sideDockRightExitTimer = null;
}

function setSideDockOpen(open) {
  els.sideDock.classList.toggle("is-open", open);
  els.sidePeekButton.setAttribute("aria-expanded", String(open));
}

function closeSideDockFromOutside(event) {
  if (els.sideDock.contains(event.target)) {
    return;
  }
  closeSideDock();
}

function syncTextLogExpansion() {
  const expanded = Boolean(els.turnLogList.querySelector(".turn-log-render[open]"));
  els.sideDock.classList.toggle("is-log-expanded", expanded);
  if (expanded) {
    openSideDock();
  }
}

function renderCgJobPanel() {
  if (!els.cgJobPanel) {
    return;
  }
  els.cgJobPanel.replaceChildren();
  const image = state.packet?.image;
  if (!image) {
    appendEmpty(els.cgJobPanel, "턴 CG 상태 없음.");
    return;
  }
  const visualJobs = pendingVisualJobs();
  const savedAssets = savedVisualAssets();
  els.cgJobPanel.append(
    visualSummaryRow("현재 턴 CG", turnCgSummaryText(image), image.exists ? "saved" : image.image_generation_job ? "pending" : "idle"),
    visualSummaryRow("월드 배경", worldBackgroundSummaryText(), missingWorldBackgroundJobs().length ? "pending" : "saved"),
  );
  for (const job of visualJobs) {
    els.cgJobPanel.append(visualJobRow(job));
  }
  if (!visualJobs.length && savedAssets.length) {
    const saved = document.createElement("p");
    saved.className = "cg-job-note";
    saved.textContent = `저장된 비주얼: ${savedAssets.map(visualJobLabel).join(", ")}`;
    els.cgJobPanel.append(saved);
  }
  if (!visualJobs.length && !savedAssets.length) {
    appendEmpty(els.cgJobPanel, "이미지 연결이 아직 열리지 않았다.");
  }
  if (visualJobs.length) {
    const details = document.createElement("details");
    details.className = "raw-markdown";
    const summary = document.createElement("summary");
    summary.textContent = "대기 상세";
    const pre = document.createElement("pre");
    pre.textContent = visualJobs
      .map((job) => {
        const refs = job.reference_paths?.length ? `\nrefs:\n${job.reference_paths.join("\n")}` : "";
        return `[${job.slot}] ${job.prompt}${refs}\n-> ${job.destination_path}`;
      })
      .join("\n\n");
    details.append(summary, pre);
    els.cgJobPanel.append(details);
  }
  if (els.requestTurnCgRetry) {
    els.requestTurnCgRetry.disabled = !turnCgJobsEnabled() || image.exists;
  }
  syncTurnCgButton();
}

async function refreshCgGallery(options = {}) {
  if (!els.cgGallery || !state.apiAvailable || explicitPacketUrl) {
    renderCgGallery(null);
    return;
  }
  const force = Boolean(options.force);
  if (force) {
    setGalleryStatus("갤러리 갱신 중");
  }
  try {
    const gallery = await fetchJson(cgGalleryApiUrl);
    state.cgGallery = gallery;
    renderCgGallery(gallery);
    setGalleryStatus(gallery.items?.length ? `${gallery.items.length}장 준비됨` : "저장된 CG 없음");
  } catch (error) {
    setGalleryStatus(`갤러리 갱신 실패: ${error.message}`);
  }
}

function renderCgGallery(gallery) {
  if (!els.cgGallery) {
    return;
  }
  els.cgGallery.replaceChildren();
  const items = gallery?.items || [];
  if (!items.length) {
    appendEmpty(els.cgGallery, "아직 저장된 턴 CG가 없다.");
    return;
  }
  for (const item of items) {
    els.cgGallery.append(cgGalleryCard(item));
  }
}

function cgGalleryCard(item) {
  const card = document.createElement("article");
  card.className = "cg-gallery-card";

  const image = document.createElement("img");
  image.src = item.asset_url;
  image.alt = `${item.turn_id} CG`;
  image.loading = "lazy";

  const body = document.createElement("div");
  body.className = "cg-gallery-body";

  const meta = document.createElement("div");
  meta.className = "cg-gallery-meta";
  const turn = document.createElement("span");
  turn.textContent = `turn ${item.turn_index}`;
  const id = document.createElement("span");
  id.textContent = item.turn_id;
  meta.append(turn, id);

  const summary = document.createElement("p");
  summary.className = "cg-gallery-summary";
  summary.textContent = item.prompt_summary || "프롬프트 요약 없음";

  const details = document.createElement("details");
  details.className = "cg-gallery-prompt raw-markdown";
  const detailsSummary = document.createElement("summary");
  detailsSummary.textContent = "프롬프트";
  const prompt = document.createElement("pre");
  prompt.textContent = item.image_prompt || "프롬프트 없음";
  details.append(detailsSummary, prompt);

  const actions = document.createElement("div");
  actions.className = "cg-gallery-actions";
  const download = document.createElement("a");
  download.href = item.asset_url;
  download.download = item.download_filename || `${item.turn_id}.png`;
  download.textContent = "다운로드";
  actions.append(download);

  body.append(meta, summary, details, actions);
  card.append(image, body);
  return card;
}

function setGalleryStatus(text) {
  if (els.galleryStatus) {
    els.galleryStatus.textContent = text || "";
  }
}

function visualSummaryRow(label, value, status) {
  const row = document.createElement("div");
  row.className = "cg-job-row cg-job-summary";
  row.dataset.status = status;
  const key = document.createElement("span");
  key.textContent = label;
  const text = document.createElement("strong");
  text.textContent = value;
  row.append(key, text);
  return row;
}

function visualJobRow(job) {
  const row = document.createElement("div");
  row.className = "cg-job-row";
  row.dataset.status = "pending";
  const key = document.createElement("span");
  key.textContent = "생성 대기";
  const text = document.createElement("strong");
  text.textContent = `${visualJobLabel(job.slot)} 준비 중`;
  row.append(key, text);
  return row;
}

function pendingVisualJobs() {
  if (!state.settings.cgEnabled) {
    return [];
  }
  const jobs = [];
  const image = state.packet?.image;
  const worldJobs = state.visualAssets?.image_generation_jobs || [];
  jobs.push(...worldJobs);
  if (image?.image_generation_job && turnCgJobsEnabled()) {
    jobs.push(image.image_generation_job);
  }
  return jobs;
}

function missingWorldBackgroundJobs() {
  if (!state.settings.cgEnabled) {
    return [];
  }
  return (state.visualAssets?.image_generation_jobs || []).filter((job) =>
    ["menu_background", "stage_background"].includes(job.slot),
  );
}

function savedVisualAssets() {
  const assets = [];
  if (state.visualAssets?.menu_background?.exists) {
    assets.push("menu_background");
  }
  if (state.visualAssets?.stage_background?.exists) {
    assets.push("stage_background");
  }
  if (state.packet?.image?.exists) {
    assets.push("turn_cg");
  }
  return assets;
}

function turnCgSummaryText(image) {
  if (image.exists) {
    return "저장됨";
  }
  if (image.image_generation_job && turnCgJobsEnabled()) {
    return "이미지 연결 대기";
  }
  return "이번 턴은 자동 생성 예산을 쓰지 않음";
}

function worldBackgroundSummaryText() {
  if (!state.settings.cgEnabled) {
    return "CG 생성 꺼짐";
  }
  const missing = missingWorldBackgroundJobs().map((job) => visualJobLabel(job.slot));
  if (missing.length) {
    return `${missing.join(", ")} 대기`;
  }
  return "메뉴/VN 배경 준비됨";
}

function visualJobLabel(slot) {
  if (slot === "menu_background") {
    return "메인 메뉴 배경";
  }
  if (slot === "stage_background") {
    return "VN 배경";
  }
  if (slot?.startsWith("turn_cg")) {
    return "현재 턴 CG";
  }
  if (slot?.startsWith("character_sheet")) {
    return "캐릭터 시트";
  }
  if (slot?.startsWith("location_sheet")) {
    return "장소 시트";
  }
  return slot || "비주얼 항목";
}

function startVisualJobPolling() {
  if (state.visualPollTimer) {
    window.clearTimeout(state.visualPollTimer);
  }
  if (!state.apiAvailable || explicitPacketUrl || !hasPendingVisualJobs()) {
    return;
  }
  pollVisualJobs(0);
}

function hasPendingVisualJobs() {
  return pendingVisualJobs().length > 0;
}

function pollVisualJobs(attempt) {
  if (state.visualPollTimer) {
    window.clearTimeout(state.visualPollTimer);
  }
  if (!hasPendingVisualJobs() || attempt > 240) {
    return;
  }
  state.visualPollTimer = window.setTimeout(async () => {
    try {
      const packet = await fetchJson(currentApiUrl);
      refreshVisualStateFromPacket(packet);
      await refreshRuntimeStatus();
    } catch (error) {
      setCgJobStatus(`CG 상태 갱신 실패: ${error.message}`);
    }
    if (hasPendingVisualJobs()) {
      pollVisualJobs(attempt + 1);
    } else {
      setCgJobStatus("CG 반영 완료");
      refreshCgGallery();
    }
  }, VISUAL_JOB_POLL_MS);
}

function turnCgJobsEnabled() {
  return state.settings.cgEnabled && state.settings.turnCgMode !== "off";
}

function syncTurnCgButton() {
  if (!els.generateTurnCgButton) {
    return;
  }
  const image = state.packet?.image;
  const disabled =
    state.busy ||
    !state.apiAvailable ||
    explicitPacketUrl ||
    !turnCgJobsEnabled() ||
    Boolean(image?.exists);
  els.generateTurnCgButton.disabled = disabled;
  els.generateTurnCgButton.title = image?.exists
    ? "이미 이 턴 CG가 저장되어 있어."
    : "현재 턴 서사 기반 CG 생성 요청";
  if (image?.exists) {
    setTurnCgStatus("ready", "저장됨");
  } else if (state.busy) {
    setTurnCgStatus("loading", "요청 중");
  } else if (image?.image_generation_job) {
    setTurnCgStatus("loading", "생성 대기");
  } else if (missingWorldBackgroundJobs().length) {
    setTurnCgStatus("loading", "배경 대기");
  } else {
    setTurnCgStatus("", "");
  }
}

function setTurnCgStatus(kind, text) {
  if (!els.turnCgStatus) {
    return;
  }
  els.turnCgStatus.className = "turn-cg-status";
  if (kind) {
    els.turnCgStatus.classList.add(`is-${kind}`);
  }
  els.turnCgStatus.textContent = text || "";
}

function bindSettings() {
  els.settingCgEnabled.checked = state.settings.cgEnabled;
  els.settingTurnCgMode.value = state.settings.turnCgMode;
  els.settingCgPrompt.value = state.settings.cgPrompt;
  els.settingGuideChoice.checked = state.settings.guideChoiceEnabled;
  els.settingAutoReveal.checked = state.settings.autoReveal;
  els.settingTextScale.value = state.settings.textScale;
  els.settingReduceMotion.checked = state.settings.reduceMotion;

  els.settingMainMenu.addEventListener("click", () => openMainMenu("previous"));
  els.settingSaveWorld.addEventListener("click", saveCurrentWorld);
  els.settingLoadWorld.addEventListener("click", loadWorldFromBundle);
  els.settingLoadWorldBundle.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      loadWorldFromBundle();
    }
  });
  els.requestTurnCgRetry.addEventListener("click", () => requestTurnCgRetry({ openConsole: true }));
  els.settingCgEnabled.addEventListener("change", () => updateSetting("cgEnabled", els.settingCgEnabled.checked));
  els.settingTurnCgMode.addEventListener("change", () => updateSetting("turnCgMode", els.settingTurnCgMode.value));
  els.settingCgPrompt.addEventListener("input", () => updateSetting("cgPrompt", els.settingCgPrompt.value));
  els.settingGuideChoice.addEventListener("change", () => {
    updateSetting("guideChoiceEnabled", els.settingGuideChoice.checked);
    if (state.packet) {
      renderChoices(state.packet);
    }
  });
  els.settingAutoReveal.addEventListener("change", () => updateSetting("autoReveal", els.settingAutoReveal.checked));
  els.settingTextScale.addEventListener("input", () => updateSetting("textScale", els.settingTextScale.value));
  els.settingReduceMotion.addEventListener("change", () => updateSetting("reduceMotion", els.settingReduceMotion.checked));
}

function updateSetting(key, value) {
  state.settings[key] = value;
  saveSettings();
  applySettings();
  renderCgJobPanel();
}

function loadSettings() {
  try {
    const loaded = JSON.parse(localStorage.getItem("singulari.vn.settings") || "{}");
    delete loaded.defaultView;
    return { ...DEFAULT_SETTINGS, ...loaded };
  } catch (error) {
    console.warn(`VN settings load failed: ${error.message}`);
    return { ...DEFAULT_SETTINGS };
  }
}

function saveSettings() {
  try {
    localStorage.setItem("singulari.vn.settings", JSON.stringify(state.settings));
  } catch (error) {
    console.warn(`VN settings save failed: ${error.message}`);
  }
}

function applySettings() {
  const textScale = Number.parseFloat(state.settings.textScale || "1");
  const safeScale = Number.isFinite(textScale) ? textScale : 1;
  document.documentElement.style.setProperty("--story-font-size", `${(1.16 * safeScale).toFixed(2)}rem`);
  document.documentElement.style.setProperty("--story-focus-font-size", `${(1.28 * safeScale).toFixed(2)}rem`);
  els.shell.classList.toggle("reduce-motion", Boolean(state.settings.reduceMotion));
  syncImagePrompt();
}

function syncImagePrompt() {
  if (!els.imagePrompt) {
    return;
  }
  if (!state.settings.cgEnabled) {
    els.imagePrompt.value = "CG generation disabled in VN settings.";
    return;
  }
  const visualJobs = state.visualAssets?.image_generation_jobs || [];
  const turnCgJob = turnCgJobsEnabled() ? state.packet?.image?.image_generation_job : null;
  const turnCgText = turnCgJob
    ? `turn CG background job:\n[${turnCgJob.slot}] ${turnCgJob.prompt}\n-> ${turnCgJob.destination_path}`
    : state.baseImagePrompt;
  const worldJobs = visualJobs.length
    ? `\n\nworld visual asset jobs:\n${visualJobs
        .map((job) => `[${job.slot}] ${job.prompt}\n-> ${job.destination_path}`)
        .join("\n\n")}`
    : "";
  els.imagePrompt.value = `${state.settings.cgPrompt.trim() || turnCgText}${worldJobs}`;
}


function chooseFreeform() {
  if (state.awaitingAgent) {
    return;
  }
  const packet = state.packet;
  if (!packet) {
    return;
  }
  const choice = packet.choices.find((item) => item.requires_inline_text);
  if (!choice) {
    return;
  }
  const action = els.freeformInput.value.trim();
  if (!action) {
    showTransientLine("7번 자유서술은 행동을 같이 써야 해.");
    revealChoices();
    selectCommand(
      `singulari-world turn --world-id ${packet.world_id} --input ${shellQuote(`${choice.slot} <action>`)} --render`,
    );
    return;
  }
  const input = `${choice.slot} ${action}`;
  if (state.apiAvailable) {
    submitTurnInput(input);
    return;
  }
  selectCommand(
    `singulari-world turn --world-id ${packet.world_id} --input ${shellQuote(input)} --render`,
  );
}

function chooseSlot(choice) {
  if (state.awaitingAgent) {
    return;
  }
  if (state.apiAvailable) {
    submitTurnInput(String(choice.slot));
    return;
  }
  selectCommand(choice.command_template);
}

function chooseShortcutSlot(slot) {
  if (state.awaitingAgent || !state.choicesRevealed || !state.packet) {
    return false;
  }
  const choice = state.packet.choices.find((item) => item.slot === slot);
  if (!choice || (!state.settings.guideChoiceEnabled && choice.slot === 4)) {
    return false;
  }
  if (choice.requires_inline_text) {
    els.freeformInput.focus();
    if (els.freeformInput.value.trim()) {
      chooseFreeform();
    }
    return true;
  }
  chooseSlot(choice);
  return true;
}

async function submitTurnInput(input) {
  if (state.busy || state.awaitingAgent) {
    return;
  }
  state.busy = true;
  let waitingForAgent = false;
  state.choicesRevealed = false;
  els.choicePanel.hidden = true;
  els.shell.classList.add("is-loading");
  els.outcomeBadge.textContent = AGENT_WAITING_BADGE;
  try {
    const response = await fetchJson(chooseApiUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ input }),
    });
    if (response.status === "waiting_agent") {
      waitingForAgent = true;
      handleWaitingAgentTurn(response);
      return;
    }
    const packet = response;
    renderPacket(packet);
    selectCommand(
      `singulari-world turn --world-id ${packet.world_id} --input ${shellQuote(input)} --render`,
    );
  } catch (error) {
    showTransientLine(`진행 실패: ${error.message}`);
    revealChoices();
  } finally {
    state.busy = false;
    if (!waitingForAgent) {
      els.shell.classList.remove("is-loading");
    }
  }
}

function handleWaitingAgentTurn(pending, options = {}) {
  renderAgentWaitingStage(Boolean(options.initial));
  selectCommand(
    pending.command_hint || `singulari-world agent-next --world-id ${pending.world_id} --json`,
  );
  pollCommittedAgentTurn(pending.turn_id, 0);
}

function renderAgentWaitingStage(initial) {
  state.awaitingAgent = true;
  state.choicesRevealed = false;
  state.storyLines = initial ? AGENT_WAITING_COPY.initial : AGENT_WAITING_COPY.next;
  state.visibleLineCount = state.storyLines.length;
  els.outcomeBadge.textContent = AGENT_WAITING_BADGE;
  els.choices.replaceChildren();
  els.freeformInput.value = "";
  els.choicePanel.hidden = true;
  els.shell.classList.add("is-loading", "is-awaiting-agent");
  renderStoryProgress();
  renderCurrentTextLog();
}

function pollCommittedAgentTurn(turnId, attempt) {
  if (state.agentPollTimer) {
    window.clearTimeout(state.agentPollTimer);
  }
  state.agentPollTimer = window.setTimeout(async () => {
    try {
      const status = await fetchJson(agentPendingApiUrl);
      if (status.status === "waiting_agent") {
        if (attempt < 120) {
          pollCommittedAgentTurn(turnId, attempt + 1);
        }
        return;
      }
      const packet = await fetchJson(currentApiUrl);
      if (packet.turn_id === turnId) {
        renderPacket(packet);
        els.outcomeBadge.textContent = packet.scene.adjudication?.outcome || packet.mode;
        return;
      }
    } catch (error) {
      try {
        const packet = await fetchJson(currentApiUrl);
        if (packet.turn_id === turnId) {
          renderPacket(packet);
          els.outcomeBadge.textContent = packet.scene.adjudication?.outcome || packet.mode;
          return;
        }
      } catch (refreshError) {
        showTransientLine(
          `세계의 흐름을 다시 읽지 못했어: pending=${error.message}; current=${refreshError.message}`,
        );
      }
    }
    if (attempt < 120) {
      pollCommittedAgentTurn(turnId, attempt + 1);
    }
  }, 1500);
}

function selectCommand(command) {
  state.selectedCommand = command;
  els.commandOutput.value = command;
}

function renderLoadError(error) {
  els.worldTitle.textContent = "Singulari World VN";
  els.turnId.textContent = "no packet";
  els.locationId.textContent = explicitPacketUrl || currentApiUrl;
  els.eventId.textContent = "load_error";
  els.outcomeBadge.textContent = "missing";
  state.storyLines = [`VN packet을 읽지 못했어: ${error.message}`];
  state.visibleLineCount = 1;
  state.choicesRevealed = false;
  renderStoryProgress();
  els.commandOutput.value =
    "singulari-world vn-serve --world-id <world_id> --port 4177";
}

async function fetchJson(url, options = {}) {
  const response = await fetch(url, { cache: "no-store", ...options });
  const text = await response.text();
  let parsed = null;
  if (text) {
    parsed = JSON.parse(text);
  }
  if (!response.ok) {
    throw new Error(parsed?.error || `${response.status} ${response.statusText}`);
  }
  return parsed;
}

function showTransientLine(text) {
  state.storyLines.push(text);
  state.visibleLineCount = state.storyLines.length;
  renderStoryProgress();
}

function handleGlobalKeydown(event) {
  const activeTag = document.activeElement?.tagName;
  const textEditing =
    activeTag === "INPUT" || activeTag === "TEXTAREA" || activeTag === "SELECT";
  const buttonFocused = activeTag === "BUTTON";
  const focusInSideDock = Boolean(document.activeElement && els.sideDock.contains(document.activeElement));
  const launchOpen = els.launchOverlay && !els.launchOverlay.hidden;
  if (launchOpen) {
    if (event.key === "Escape" && hasLaunchSeen() && !state.busy) {
      event.preventDefault();
      hideLaunchOverlay();
    }
    return;
  }
  if (event.key === "Escape") {
    if (state.choicesRevealed) {
      event.preventDefault();
      retreatStory();
    } else {
      closeSideDock();
    }
    document.activeElement?.blur();
    return;
  }
  if (textEditing) {
    return;
  }
  const lowerKey = event.key.toLowerCase();
  if (event.key === "Tab") {
    event.preventDefault();
    closeSideDock();
    toggleViewMode();
    return;
  }
  if (lowerKey === "c" || lowerKey === "l" || lowerKey === "s") {
    event.preventDefault();
    openSideDock();
    selectSideTab({ c: "status", l: "textlog", s: "settings" }[lowerKey]);
    return;
  }
  if (focusInSideDock || (buttonFocused && (event.key === " " || event.key === "Enter"))) {
    return;
  }
  if (/^[1-7]$/.test(event.key) && chooseShortcutSlot(Number(event.key))) {
    event.preventDefault();
    return;
  }
  if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
    event.preventDefault();
    retreatStory();
    return;
  }
  if (
    event.key === " " ||
    event.key === "Enter" ||
    event.key === "ArrowDown" ||
    event.key === "ArrowRight"
  ) {
    event.preventDefault();
    advanceStory();
  }
}

function copyText(value) {
  if (!value) {
    return;
  }
  if (navigator.clipboard?.writeText) {
    navigator.clipboard
      .writeText(value)
      .catch((error) => console.warn(`clipboard write failed: ${error.message}`));
  }
}

function cssUrl(value) {
  return String(value).replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`;
}
