import type { ClientEngine } from './types'

let webGPUCached: boolean | undefined

/** Returns true if the browser exposes the WebGPU API. */
export function hasWebGPUSupport(): boolean {
  if (webGPUCached === undefined) {
    webGPUCached = typeof navigator !== 'undefined' && 'gpu' in navigator
  }
  return webGPUCached
}

/**
 * Resolve which concrete engine should be used based on the user's
 * preference and the current viewport layout.
 *
 * - `auto` (default): use dicomview if WebGPU is available (both stack
 *   and quad layouts are supported since dicomview 0.2.0 with StackViewer).
 * - `dicomview`: force dicomview everywhere, but fall back to cornerstone
 *   if WebGPU is not available.
 * - `cornerstone`: always use cornerstone.
 */
export function resolveEngine(
  preference: ClientEngine,
  layout: 'single' | 'quad',
): 'dicomview' | 'cornerstone' {
  void layout // both layouts now supported by dicomview

  if (preference === 'cornerstone') {
    return 'cornerstone'
  }

  // auto or dicomview: use dicomview when WebGPU is available
  return hasWebGPUSupport() ? 'dicomview' : 'cornerstone'
}

/**
 * Returns true when the user wanted dicomview but we had to fall back
 * to cornerstone (e.g. because WebGPU is not supported).
 */
export function isEngineFallback(
  preference: ClientEngine,
  resolved: 'dicomview' | 'cornerstone',
): boolean {
  return preference === 'dicomview' && resolved === 'cornerstone'
}
