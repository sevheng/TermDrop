# Progressive Security Audit UX Design

## Problem
The security audit takes 5-10 seconds because it runs 8 SSH exec checks sequentially and returns all results at once. The user stares at a loading spinner with no feedback.

## Root Causes
| Check | Why It's Slow |
|-------|--------------|
| `apt list --upgradable` | Reads full package database on Debian/Ubuntu |
| `journalctl \| grep` | Scans large systemd journal files |
| `sudo ufw status` | May prompt for password or timeout |
| 8 sequential SSH execs | Each creates channel + runs command + collects output |

## Solution: Progressive Streaming via Tauri Events

Run checks sequentially but emit a Tauri event after each one completes. The frontend receives checks one-by-one and renders them immediately. The user sees results appearing progressively — feels much faster even if total time is the same.

### Architecture

```
connect() → start_security_audit(host_id)
                │
                ├──► Background thread
                │    loop checks:
                │      run check[i] ──► emit("security-check", { check, index, total })
                │    end loop
                │    emit("security-complete", { score })
                │
SecurityPanel ──► listen("security-check") → append check to list
                ──► listen("security-complete") → show score, hide skeletons
```

### Backend Changes

**`src-tauri/src/security.rs`**:
- Split `run_security_audit` into individual check functions (already done)
- New `start_security_audit(session, app_handle, host_id)` — spawns thread, runs checks sequentially, emits events

**`src-tauri/src/main.rs`**:
- New `start_security_audit` Tauri command (replaces `run_security_audit`)
- Uses `app_handle.emit()` to send events to frontend

### Event Payloads

```rust
#[derive(Serialize, Clone)]
pub struct SecurityCheckEvent {
    pub host_id: i64,
    pub check: SecurityCheck,
    pub index: usize,
    pub total: usize,
}

#[derive(Serialize, Clone)]
pub struct SecurityCompleteEvent {
    pub host_id: i64,
    pub score: u8,
}
```

### Frontend Changes

**`src/stores/connection.js`**:
- `securityReports` cache: `{ checks: [], completed: false, score: null, loading: true }`
- `addSecurityCheck(hostId, check)` — appends incoming check
- `setSecurityComplete(hostId, score)` — marks done, computes score
- `resetSecurityReport(hostId)` — clears for re-run

**`src/components/SecurityPanel.vue`**:
- Listen for `security-check` events via `listen()`
- Listen for `security-complete` events
- Show skeleton placeholder rows for pending checks
- Progress text: "Check 3 of 8..."
- Score badge appears when complete

### UI States

| State | Visual |
|-------|--------|
| Not started | "Click Run Audit to start" |
| Running (0 checks) | 8 skeleton rows, thin progress bar |
| Running (3 checks) | 3 real rows + 5 skeletons, "Check 3 of 8" |
| Complete | All rows + score badge |

### Why Not Parallel Execution?

`ssh2::Session` is `Send` but not `Sync`. Concurrent channel operations on one session may deadlock or panic. Sequential with streaming is safer and the UX improvement is dramatic enough.
