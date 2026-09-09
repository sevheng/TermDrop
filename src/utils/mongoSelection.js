/**
 * Pure helpers over the MongoDB panel's selection map:
 * Map<dbName, Set<collectionName>>. Every mutator returns a new Map so
 * Vue sees the change.
 */

export function isSelected(selected, db, coll) {
  return selected.get(db)?.has(coll) || false
}

/** 'none' | 'some' | 'all' for a database's checkbox state. */
export function dbSelectionState(selected, db) {
  const set = selected.get(db.name)
  if (!set || set.size === 0) return 'none'
  if (db.collections.length > 0 && set.size === db.collections.length) return 'all'
  return 'some'
}

export function countSelected(selected) {
  let count = 0
  for (const set of selected.values()) {
    count += set.size
  }
  return count
}

/** [{ db, collections: [...] }] in insertion order. */
export function buildEntries(selected) {
  const entries = []
  for (const [db, set] of selected) {
    entries.push({ db, collections: Array.from(set) })
  }
  return entries
}

export function toggleCollection(selected, db, coll) {
  const set = selected.get(db) || new Set()
  const newSet = new Set(set)
  if (newSet.has(coll)) {
    newSet.delete(coll)
  } else {
    newSet.add(coll)
  }
  const newMap = new Map(selected)
  if (newSet.size === 0) {
    newMap.delete(db)
  } else {
    newMap.set(db, newSet)
  }
  return newMap
}

export function withoutDb(selected, db) {
  const newMap = new Map(selected)
  newMap.delete(db)
  return newMap
}

export function withAllCollections(selected, db) {
  const newMap = new Map(selected)
  newMap.set(db.name, new Set(db.collections))
  return newMap
}
