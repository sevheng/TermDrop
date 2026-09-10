import { ref } from 'vue'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { withMongoSecret } from './useMongoSecret.js'

/**
 * One side of the MongoDB panel (remote or local): its database list,
 * which databases are expanded, and the loading flag for the list.
 *
 * The side is named rather than passed as a URI: the stored URI carries no
 * password, so the backend resolves the full connection string itself.
 * `label` names the side in error toasts.
 */
export function useMongoSide(hostId, side, label) {
  const databases = ref([])
  const expandedDbs = ref(new Set())
  const loading = ref(false)

  async function loadDatabases() {
    if (!hostId.value) return
    loading.value = true
    try {
      const dbNames = await withMongoSecret(hostId.value, side, () =>
        invoke('mongodb_list_databases', { hostId: hostId.value, side }),
      )
      databases.value = dbNames.map(name => ({
        name,
        collections: [],
        loading: false,
      }))
    } catch (err) {
      toast(`Failed to list ${label} databases: ` + err, 'error')
    } finally {
      loading.value = false
    }
  }

  function reset() {
    databases.value = []
    expandedDbs.value = new Set()
  }

  return { databases, expandedDbs, loading, loadDatabases, reset }
}

/**
 * Load a database's collections in place. `roleLabel` ('source'/'dest')
 * names the role in the error toast, matching the panel's wording.
 */
export async function fetchCollections(hostId, side, db, roleLabel) {
  db.loading = true
  try {
    db.collections = await withMongoSecret(hostId, side, () =>
      invoke('mongodb_list_collections', { hostId, side, db: db.name }),
    )
  } catch (err) {
    toast(`Failed to list ${roleLabel} collections for ${db.name}: ${err}`, 'error')
  } finally {
    db.loading = false
  }
}
