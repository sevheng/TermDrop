import { ref } from 'vue'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { withMongoSecret } from './useMongoSecret.js'

/**
 * A MongoDB connection's database list and which databases are expanded.
 *
 * The connection is named by host id rather than by URI: the stored URI carries
 * no password, so the backend resolves the full connection string itself.
 */
export function useMongoConnection(hostId) {
  const databases = ref([])
  const expandedDbs = ref(new Set())
  const loading = ref(false)

  async function loadDatabases() {
    if (!hostId.value) return
    loading.value = true
    try {
      const dbNames = await withMongoSecret(hostId.value, () =>
        invoke('mongodb_list_databases', { hostId: hostId.value }),
      )
      databases.value = dbNames.map(name => ({
        name,
        collections: [],
        loading: false,
      }))
    } catch (err) {
      toast('Failed to list databases: ' + err, 'error')
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

/** Load a database's collections in place. */
export async function fetchCollections(hostId, db) {
  db.loading = true
  try {
    db.collections = await withMongoSecret(hostId, () =>
      invoke('mongodb_list_collections', { hostId, db: db.name }),
    )
  } catch (err) {
    toast(`Failed to list collections for ${db.name}: ${err}`, 'error')
  } finally {
    db.loading = false
  }
}
