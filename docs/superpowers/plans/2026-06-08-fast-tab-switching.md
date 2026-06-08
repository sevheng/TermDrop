# Fast Tab Switching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate terminal flicker and lag when switching between SSH session tabs by pre-mounting terminals, caching right panel state, and deferring expensive work.

**Architecture:** All `TerminalTab` instances stay mounted in the DOM (hidden via `v-show`), removing DOM swap overhead. Right panels are cached per tab via `<KeepAlive>`. System status is cached in Pinia and shared across tabs. `setTimeout` delays are replaced with `requestAnimationFrame`.

**Tech Stack:** Vue 3 (Composition API, `<script setup>`), Pinia, @xterm/xterm, Tauri v2, TailwindCSS

---

## File Structure

| File | Responsibility | Action |
|------|---------------|--------|
| `src/stores/connection.js` | Pinia store — add `systemStatus` cache + getters/setters | Modify |
| `src/components/TerminalTab.vue` | Terminal component — add `isActive` prop, watch it to toggle expensive ops, remove `onActivated`/`onDeactivated`, replace `setTimeout` with `rAF` | Modify |
| `src/views/MainWindow.vue` | Layout — pre-mount all `TerminalTab`s with `v-show`, wrap right panels in `<KeepAlive>` with composite keys | Modify |

---

### Task 1: Add System Status Cache to Pinia Store

**Files:**
- Modify: `src/stores/connection.js`

Add a `systemStatus` reactive Map, a getter, and a setter. Place them after the existing `settings` ref and before `loadHosts`.

- [ ] **Step 1: Add `systemStatus` ref and helper functions**

Replace the `settings` ref block (lines 13-16) and the empty line after it with the expanded block including `systemStatus`:

```javascript
  const settings = ref({
    font_size: '14',
    download_path: '',
  })

  const systemStatus = ref(new Map())

  function getSystemStatus(hostId) {
    return systemStatus.value.get(hostId) || null
  }

  function setSystemStatus(hostId, data) {
    systemStatus.value.set(hostId, { ...data, timestamp: Date.now() })
  }
```

- [ ] **Step 2: Export new functions in the return object**

In the `return` block at the bottom (around line 280), add `systemStatus`, `getSystemStatus`, and `setSystemStatus`:

Find:
```javascript
    settings,
    loadSettings,
    saveSettings,
```

Replace with:
```javascript
    settings,
    loadSettings,
    saveSettings,
    systemStatus,
    getSystemStatus,
    setSystemStatus,
```

- [ ] **Step 3: Verify build**

Run:
```bash
cd ssh-client && npx vue-tsc --noEmit 2>&1 | head -20
```

Expected: No TypeScript errors related to the new exports.

- [ ] **Step 4: Commit**

```bash
cd ssh-client && git add src/stores/connection.js && git commit -m "perf: add systemStatus cache to Pinia store"
```

---

### Task 2: Update TerminalTab.vue — isActive Prop & Watch Handler

**Files:**
- Modify: `src/components/TerminalTab.vue`

- [ ] **Step 1: Add `isActive` prop**

Find the `defineProps` block (lines 140-149):

```javascript
const props = defineProps({
  sessionId: {
    type: String,
    required: true,
  },
  hostId: {
    type: Number,
    default: null,
  },
})
```

Replace with:

```javascript
const props = defineProps({
  sessionId: {
    type: String,
    required: true,
  },
  hostId: {
    type: Number,
    default: null,
  },
  isActive: {
    type: Boolean,
    default: false,
  },
})
```

- [ ] **Step 2: Import `watch` from Vue**

Find the imports line (line 130):

```javascript
import { ref, onMounted, onUnmounted, onActivated, onDeactivated, watch, nextTick } from 'vue'
```

`watch` is already imported. Keep it as-is. Remove `onActivated` and `onDeactivated`:

```javascript
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue'
```

- [ ] **Step 3: Replace `startStatusPolling` — read cache, do not fetch immediately**

Find the `startStatusPolling` function (lines 322-327):

```javascript
function startStatusPolling() {
  if (statusInterval) clearInterval(statusInterval)
  if (!props.hostId) return
  fetchSystemStatus()
  statusInterval = setInterval(fetchSystemStatus, 5000)
}
```

Replace with:

```javascript
function startStatusPolling() {
  if (statusInterval) clearInterval(statusInterval)
  if (!props.hostId) return
  // Read from cache immediately instead of firing SSH execs
  const cached = store.getSystemStatus(props.hostId)
  if (cached) {
    status.value = cached
  }
  statusInterval = setInterval(fetchSystemStatus, 5000)
}
```

- [ ] **Step 4: Update `fetchSystemStatus` to write to cache**

Find the end of `fetchSystemStatus` where `status.value` is assigned (around line 308):

```javascript
    status.value = {
      load,
      ram,
      disk,
      uptime,
      os: osParts.join(' · '),
      cores,
    }
    statusError.value = ''
```

Replace with:

```javascript
    const data = {
      load,
      ram,
      disk,
      uptime,
      os: osParts.join(' · '),
      cores,
    }
    status.value = data
    store.setSystemStatus(props.hostId, data)
    statusError.value = ''
```

- [ ] **Step 5: Remove `onActivated` / `onDeactivated` hooks**

Find and delete these blocks (lines 518-524):

```javascript
onActivated(() => {
  startActiveOperations()
})

onDeactivated(() => {
  stopActiveOperations()
})
```

- [ ] **Step 6: Add `watch(() => props.isActive, ...)` handler**

Add this `watch` immediately after the existing `watch(() => props.sessionId, ...)` block (after line 535, before the closing `</script>` tag):

```javascript
watch(() => props.isActive, (active) => {
  if (!term) return
  if (active) {
    term.focus()
    requestAnimationFrame(() => {
      if (fitAddon) fitAddon.fit()
    })
    if (terminalContainer.value && resizeObserver) {
      resizeObserver.observe(terminalContainer.value)
    }
    startStatusPolling()
  } else {
    term.blur()
    stopStatusPolling()
    if (resizeObserver) {
      resizeObserver.disconnect()
    }
  }
})
```

- [ ] **Step 7: Replace `setTimeout` with `requestAnimationFrame` in `startActiveOperations`**

Find `startActiveOperations` (lines 453-471):

```javascript
function startActiveOperations() {
  if (!term) return
  // Handle resize
  if (!resizeObserver) {
    resizeObserver = new ResizeObserver(() => {
      if (fitAddon) {
        fitAddon.fit()
      }
    })
  }
  if (terminalContainer.value) {
    resizeObserver.observe(terminalContainer.value)
  }
  // Fit on activation in case size changed while inactive
  if (fitAddon) {
    setTimeout(() => fitAddon.fit(), 50)
  }
  // Start status polling if already connected
  startStatusPolling()
}
```

Replace with:

```javascript
function startActiveOperations() {
  if (!term) return
  // Handle resize
  if (!resizeObserver) {
    resizeObserver = new ResizeObserver(() => {
      if (fitAddon) {
        fitAddon.fit()
      }
    })
  }
  if (terminalContainer.value) {
    resizeObserver.observe(terminalContainer.value)
  }
  // Start status polling if already connected
  startStatusPolling()
}
```

The `setTimeout(() => fitAddon.fit(), 50)` is removed — `requestAnimationFrame` in the `isActive` watch handles it. Also remove the duplicate `startStatusPolling()` call since the `isActive` watch will trigger it.

- [ ] **Step 8: Replace delayed fit in `initTerminal`**

Find the delayed fit block in `initTerminal` (lines 356-359):

```javascript
  // Delayed fit to handle flex layout settling
  setTimeout(() => {
    if (fitAddon) fitAddon.fit()
  }, 150)
```

Replace with:

```javascript
  // Fit after flex layout settles
  requestAnimationFrame(() => {
    if (fitAddon) fitAddon.fit()
  })
```

- [ ] **Step 9: Verify build**

Run:
```bash
cd ssh-client && npx vue-tsc --noEmit 2>&1 | head -30
```

Expected: No TypeScript errors.

- [ ] **Step 10: Commit**

```bash
cd ssh-client && git add src/components/TerminalTab.vue && git commit -m "perf: TerminalTab isActive prop, rAF fit, cached status, remove onActivated"
```

---

### Task 3: Update MainWindow.vue — Pre-mount Terminals + KeepAlive Right Panels

**Files:**
- Modify: `src/views/MainWindow.vue`

- [ ] **Step 1: Replace KeepAlive-wrapped TerminalTab with v-for + v-show**

Find the terminal area block (lines 62-81):

```vue
        <div class="flex-1 relative min-w-0">
          <KeepAlive :max="5">
            <TerminalTab
              v-if="store.activeTab"
              :key="store.activeTab.id"
              :sessionId="store.activeTab.id"
              :hostId="store.activeTab.hostId"
              class="w-full h-full"
            />
          </KeepAlive>
          <div
            v-if="!store.activeTabId"
            class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500"
          >
            <div class="text-center">
              <TerminalIcon :size="40" class="mx-auto mb-3 opacity-50" />
              <p class="text-base">Select a host to connect</p>
            </div>
          </div>
        </div>
```

Replace with:

```vue
        <div class="flex-1 relative min-w-0">
          <TerminalTab
            v-for="tab in store.tabs"
            :key="tab.id"
            v-show="tab.id === store.activeTabId"
            :sessionId="tab.id"
            :hostId="tab.hostId"
            :isActive="tab.id === store.activeTabId"
            class="w-full h-full absolute top-0 left-0"
          />
          <div
            v-if="!store.activeTabId"
            class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500 absolute inset-0"
          >
            <div class="text-center">
              <TerminalIcon :size="40" class="mx-auto mb-3 opacity-50" />
              <p class="text-base">Select a host to connect</p>
            </div>
          </div>
        </div>
```

Note: The parent `<div>` already has `relative`, so `absolute` children stack correctly. The empty state also gets `absolute inset-0` so it fills the space without pushing terminals.

- [ ] **Step 2: Wrap right panels in KeepAlive with composite keys**

Find the right panel content block (lines 138-161):

```vue
          <div class="flex-1 overflow-hidden">
            <PortForwardPanel
              v-if="rightPanelTab === 'tunnels'"
              :hostId="store.activeTab.hostId"
              @add="showForwardModal = true"
              class="w-full h-full"
            />
            <DockerPanel
              v-else-if="rightPanelTab === 'docker'"
              :hostId="store.activeTab.hostId"
              @exec="onDockerExec"
              class="w-full h-full"
            />
            <template v-else>
              <SftpPanel
                v-if="store.activeTab.sftpSessionId"
                :sftpSessionId="store.activeTab.sftpSessionId"
                class="w-full h-full"
              />
              <div v-else class="flex-1 flex flex-col items-center justify-center text-[#6e6e6e] h-full">
                <Loader2 :size="24" class="animate-spin mb-2" />
                <span class="text-sm">Connecting SFTP...</span>
              </div>
            </template>
          </div>
```

Replace with:

```vue
          <div class="flex-1 overflow-hidden">
            <KeepAlive :max="15">
              <PortForwardPanel
                v-if="rightPanelTab === 'tunnels' && store.activeTab"
                :key="'tunnels-' + store.activeTab.hostId"
                :hostId="store.activeTab.hostId"
                @add="showForwardModal = true"
                class="w-full h-full"
              />
              <DockerPanel
                v-else-if="rightPanelTab === 'docker' && store.activeTab"
                :key="'docker-' + store.activeTab.hostId"
                :hostId="store.activeTab.hostId"
                @exec="onDockerExec"
                class="w-full h-full"
              />
              <SftpPanel
                v-else-if="rightPanelTab === 'sftp' && store.activeTab?.sftpSessionId"
                :key="'sftp-' + store.activeTab.sftpSessionId"
                :sftpSessionId="store.activeTab.sftpSessionId"
                class="w-full h-full"
              />
              <div
                v-else-if="rightPanelTab === 'sftp' && store.activeTab && !store.activeTab.sftpSessionId"
                class="flex-1 flex flex-col items-center justify-center text-[#6e6e6e] h-full"
              >
                <Loader2 :size="24" class="animate-spin mb-2" />
                <span class="text-sm">Connecting SFTP...</span>
              </div>
            </KeepAlive>
          </div>
```

- [ ] **Step 3: Verify build**

Run:
```bash
cd ssh-client && npx vue-tsc --noEmit 2>&1 | head -30
```

Expected: No TypeScript errors.

- [ ] **Step 4: Commit**

```bash
cd ssh-client && git add src/views/MainWindow.vue && git commit -m "perf: pre-mount all terminals, KeepAlive right panels per tab"
```

---

### Task 4: Full Build Verification

- [ ] **Step 1: Run Vue type check**

```bash
cd ssh-client && npx vue-tsc --noEmit
```

Expected: `error TS0: no errors` or empty output.

- [ ] **Step 2: Run Vite production build**

```bash
cd ssh-client && npx vite build 2>&1 | tail -10
```

Expected: Build completes with no errors. Look for `dist/` output.

- [ ] **Step 3: Run Cargo build**

```bash
cd ssh-client/src-tauri && cargo build 2>&1 | tail -10
```

Expected: `Finished dev` message, no Rust compile errors.

- [ ] **Step 4: Commit**

```bash
cd ssh-client && git commit --allow-empty -m "perf: fast tab switching — build verified"
```

---

## Self-Review

**1. Spec coverage:**
- ✅ Pre-mount all terminals — Task 3, Step 1
- ✅ KeepAlive right panels per tab — Task 3, Step 2
- ✅ Cached system status — Task 1 + Task 2, Steps 3-4
- ✅ Remove setTimeout delays — Task 2, Steps 7-8
- ✅ isActive watch handler — Task 2, Step 6
- ✅ Remove onActivated/onDeactivated — Task 2, Step 5

**2. Placeholder scan:**
- ✅ No TBD/TODO
- ✅ No vague "add error handling" steps
- ✅ No "similar to Task N" shortcuts
- ✅ All code blocks contain complete, copy-pasteable code

**3. Type consistency:**
- ✅ `isActive` prop is `Boolean` in TerminalTab, passed as `tab.id === store.activeTabId` in MainWindow
- ✅ `getSystemStatus(hostId)` and `setSystemStatus(hostId, data)` signatures match usage
- ✅ `systemStatus` is `ref(new Map())` — consistent across store and consumers
- ✅ KeepAlive keys use `{type}-{id}` format consistently
