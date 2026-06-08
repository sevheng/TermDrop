# Fast Tab Switching Design

> **Scope:** Eliminate terminal flicker and lag when switching between SSH session tabs.

---

## 1. Problem Statement

Switching between SSH session tabs is currently laggy. The terminal flickers or takes noticeable time to appear. Root causes identified from code analysis:

| # | Cause | Location | Impact |
|---|-------|----------|--------|
| 1 | **8 SSH exec commands fire synchronously on every tab activation** | `TerminalTab.vue` `onActivated` → `startStatusPolling` → `fetchSystemStatus()` | Blocks the UI thread with 8 parallel `ssh_exec` calls (loadavg, free, df, uptime, os-release, uname -r, uname -m, nproc) |
| 2 | **Right panel components remount on every tab switch** | `MainWindow.vue` — `SftpPanel`, `DockerPanel`, `PortForwardPanel` use `v-if`, not cached | SFTP directory resets, Docker container list reloads, port forward status re-fetches |
| 3 | **`fitAddon.fit()` called with `setTimeout` delays** | `TerminalTab.vue` — 50ms on activation, 150ms on mount | Terminal renders at wrong size briefly, then jumps |
| 4 | **`onActivated`/`onDeactivated` overhead** | `TerminalTab.vue` — resize observer disconnect/reconnect, polling start/stop | Unnecessary work on every switch |

## 2. Solution: Hybrid Pre-mount + KeepAlive

Render all terminal instances in the DOM simultaneously (hidden via `v-show`), cache right panel state per tab via `<KeepAlive>`, and share system status across tabs via a Pinia cache. This is the same pattern VS Code uses for editor tabs.

### 2.1 Pre-mount All Terminals

Instead of `<KeepAlive>` swapping a single `TerminalTab` instance, render one `TerminalTab` per open tab. Inactive terminals use `v-show="false"` (`display: none`).

**Why this helps:**
- No DOM node creation/destruction on tab switch
- No `onActivated`/`onDeactivated` lifecycle overhead
- xterm buffers continue receiving data while tab is inactive
- Switching is instant — just a CSS `display` toggle

**Memory trade-off:** Each xterm instance holds a screen buffer. At 80×24 with 1000 scrollback lines, ~50KB per tab. 20 tabs = ~1MB — negligible.

### 2.2 KeepAlive Right Panels Per Tab

Wrap `SftpPanel`, `DockerPanel`, `PortForwardPanel` in `<KeepAlive>` with a composite key: `{panelType}-{sessionId/hostId}`. Each tab's right panel state (current directory, container list, port forwards) is preserved when switching away.

**Key format:**
- SFTP: `sftp-{sftpSessionId}`
- Tunnels: `tunnels-{hostId}`
- Docker: `docker-{hostId}`

### 2.3 Cached System Status

Add a `systemStatus` cache to the Pinia store (`Map<hostId, StatusData>`). Terminal tabs read from cache on mount, and only the **active** tab triggers background polling.

**Flow:**
1. Tab becomes active → read cached status immediately (zero SSH calls)
2. Start 5s interval → fetch status in background → update cache
3. Switch to another tab → cached data shows instantly
4. Multiple tabs to the same host share one cache entry

### 2.4 Remove setTimeout Delays

Replace `setTimeout(() => fitAddon.fit(), 50)` with `requestAnimationFrame(() => fitAddon.fit())` — the browser paints at the next frame, no arbitrary delay.

## 3. Architecture

### 3.1 File Changes

| File | Change |
|------|--------|
| `src/views/MainWindow.vue` | Pre-mount all `TerminalTab`s with `v-show`; wrap right panels in `KeepAlive` with composite keys |
| `src/components/TerminalTab.vue` | Add `isActive` prop; watch it to start/stop resize observer + status polling; remove `onActivated`/`onDeactivated`; replace setTimeout with rAF |
| `src/stores/connection.js` | Add `systemStatus` Map + `getSystemStatus(hostId)` + `setSystemStatus(hostId, data)` |

### 3.2 MainWindow.vue — Terminal Area

```vue
<!-- Before: single TerminalTab with KeepAlive -->
<KeepAlive :max="5">
  <TerminalTab
    v-if="store.activeTab"
    :key="store.activeTab.id"
    :sessionId="store.activeTab.id"
    :hostId="store.activeTab.hostId"
  />
</KeepAlive>

<!-- After: all terminals pre-mounted -->
<TerminalTab
  v-for="tab in store.tabs"
  :key="tab.id"
  v-show="tab.id === store.activeTabId"
  :sessionId="tab.id"
  :hostId="tab.hostId"
  :isActive="tab.id === store.activeTabId"
  class="w-full h-full absolute top-0 left-0"
/>
```

Terminals are `position: absolute` so they stack. Only the active one is visible.

### 3.3 MainWindow.vue — Right Panel Area

```vue
<KeepAlive :max="15">
  <PortForwardPanel
    v-if="rightPanelTab === 'tunnels' && store.activeTab"
    :key="'tunnels-' + store.activeTab.hostId"
    :hostId="store.activeTab.hostId"
  />
  <DockerPanel
    v-else-if="rightPanelTab === 'docker' && store.activeTab"
    :key="'docker-' + store.activeTab.hostId"
    :hostId="store.activeTab.hostId"
    @exec="onDockerExec"
  />
  <SftpPanel
    v-else-if="rightPanelTab === 'sftp' && store.activeTab?.sftpSessionId"
    :key="'sftp-' + store.activeTab.sftpSessionId"
    :sftpSessionId="store.activeTab.sftpSessionId"
  />
</KeepAlive>
```

### 3.4 TerminalTab.vue — Active State Watch

```js
const props = defineProps({
  sessionId: { type: String, required: true },
  hostId: { type: Number, default: null },
  isActive: { type: Boolean, default: false },  // NEW
})

watch(() => props.isActive, (active) => {
  if (!term) return
  if (active) {
    // Becoming active
    term.focus()
    requestAnimationFrame(() => {
      if (fitAddon) fitAddon.fit()
    })
    startStatusPolling()
    if (terminalContainer.value && resizeObserver) {
      resizeObserver.observe(terminalContainer.value)
    }
  } else {
    // Becoming inactive
    term.blur()
    stopStatusPolling()
    if (resizeObserver) resizeObserver.disconnect()
  }
})
```

Remove `onActivated` and `onDeactivated` hooks entirely.

### 3.5 TerminalTab.vue — Status Polling (No Immediate Fetch)

```js
function startStatusPolling() {
  if (statusInterval) clearInterval(statusInterval)
  if (!props.hostId) return
  // Do NOT fetch immediately — read from cache
  const cached = store.getSystemStatus(props.hostId)
  if (cached) {
    status.value = cached
  }
  // Background polling only
  statusInterval = setInterval(fetchSystemStatus, 5000)
}
```

`fetchSystemStatus()` still fetches 8 SSH execs, but only every 5 seconds, and only for the active tab. After fetching, it writes to the store cache.

### 3.6 connection.js — System Status Cache

```js
const systemStatus = ref(new Map())

function getSystemStatus(hostId) {
  return systemStatus.value.get(hostId) || null
}

function setSystemStatus(hostId, data) {
  systemStatus.value.set(hostId, { ...data, timestamp: Date.now() })
}
```

## 4. Data Flow

```
User clicks tab A
        │
        ▼
store.activeTabId = "session-A"
        │
        ├──► MainWindow: all TerminalTabs re-render (v-show toggle)
        │    TerminalTab-A.isActive = true  → focus + rAF fit + start polling
        │    TerminalTab-B.isActive = false → blur + stop polling
        │
        ├──► MainWindow: right panel KeepAlive shows cached panel for tab A
        │    (no remount, no re-fetch)
        │
        └──► TerminalTab-A: reads cached status from store (instant)
             starts 5s interval → fetchSystemStatus → writes to store cache
```

## 5. Edge Cases

| Scenario | Handling |
|----------|----------|
| **Window resize while tab is inactive** | ResizeObserver is disconnected for inactive tabs. On activation, `rAF(fit)` runs, catching up. |
| **Tab closed while inactive** | `onUnmounted` fires normally, terminal disposed. No leak. |
| **Same host, multiple tabs** | Share one `systemStatus` cache entry. Both tabs show the same cached data. |
| **Right panel tab switched within same SSH tab** | KeepAlive caches by `{type}-{id}`, so switching from SFTP → Docker → SFTP restores the exact previous SFTP state. |
| **No active tab** | `v-show` hides all terminals. Empty state shows. Right panel is hidden. |

## 6. Success Criteria

- [ ] Switching between tabs feels instant (< 50ms perceived lag)
- [ ] Terminal does not flicker or resize-jump on switch
- [ ] System status bar shows data immediately (from cache), not blank then filled
- [ ] SFTP directory listing persists when switching away and back
- [ ] Docker container list persists when switching away and back
- [ ] No increase in memory usage > 2MB for 10 open tabs
