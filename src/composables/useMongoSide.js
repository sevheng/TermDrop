import { ref } from 'vue'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'

/**
 * One side of the MongoDB panel (remote or local): its database list,
 * which databases are expanded, and the loading flag for the list.
 * `uri` is a ref to the connection string; `label` names the side in
 * error toasts.
 */
export function useMongoSide(uri, label) {
  const databases = ref([])
  const expandedDbs = ref(new Set())
  const loading = ref(false)

  async function loadDatabases() {
    if (!uri.value) return
    loading.value = true
    try {
      const dbNames = await invoke('mongodb_list_databases', { uri: uri.value })
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
export async function fetchCollections(uriValue, db, roleLabel) {
  db.loading = true
  try {
    const colls = await invoke('mongodb_list_collections', {
      uri: uriValue,
      db: db.name,
    })
    db.collections = colls
  } catch (err) {
    toast(`Failed to list ${roleLabel} collections for ${db.name}: ${err}`, 'error')
  } finally {
    db.loading = false
  }
}
