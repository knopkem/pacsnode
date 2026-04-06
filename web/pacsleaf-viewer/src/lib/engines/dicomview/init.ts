import { ensureDicomviewWasm } from '@knopkem/dicomview'

let initPromise: Promise<void> | undefined

/**
 * Initialise the dicomview WASM module (singleton — only runs once).
 * Resolves when the WASM binary has been fetched, compiled and instantiated.
 */
export async function ensureDicomviewReady(): Promise<void> {
  if (!initPromise) {
    initPromise = ensureDicomviewWasm().then(() => undefined)
  }
  return initPromise
}
