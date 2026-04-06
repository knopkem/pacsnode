/**
 * Maps cornerstone-style preset names (PascalCase with hyphens)
 * to dicomview-style preset names (kebab-case).
 *
 * Note: As of dicomview 0.2.0, the WASM layer natively accepts both
 * cornerstone-style (`CT-Bone`) and kebab-case (`ct-bone`) preset names.
 * This mapping is retained for explicit control and reverse lookups
 * (dicomview → cornerstone) needed by the UI preset selector.
 */
const CORNERSTONE_TO_DICOMVIEW: Record<string, string> = {
  'CT-Bone': 'ct-bone',
  'CT-Soft-Tissue': 'ct-soft-tissue',
  'CT-Lung': 'ct-lung',
  'CT-MIP': 'ct-mip',
  'MR-Default': 'mr-default',
  'MR-Angio': 'mr-angio',
  'MR-T2-Brain': 'mr-t2-brain',
  'MR-MIP': 'ct-mip',
}

const DICOMVIEW_TO_CORNERSTONE: Record<string, string> = Object.fromEntries(
  Object.entries(CORNERSTONE_TO_DICOMVIEW).map(([k, v]) => [v, k]),
)

/** Convert a cornerstone-style preset name to a dicomview VolumePreset string. */
export function toDicomviewPreset(cornerstonePreset: string): string {
  return CORNERSTONE_TO_DICOMVIEW[cornerstonePreset] ?? cornerstonePreset.toLowerCase()
}

/** Convert a dicomview preset name back to cornerstone-style. */
export function toCornerstonePreset(dicomviewPreset: string): string {
  return DICOMVIEW_TO_CORNERSTONE[dicomviewPreset] ?? dicomviewPreset
}
