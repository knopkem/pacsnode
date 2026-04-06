export type ClientEngine = 'auto' | 'dicomview' | 'cornerstone'

/**
 * Primary tools available in the stack (single) viewport.
 * Annotation tools are only available with cornerstone.
 */
export type StackPrimaryTool =
  | 'windowLevel'
  | 'pan'
  | 'zoom'
  | 'length'
  | 'angle'
  | 'rectangleRoi'
  | 'ellipticalRoi'

/**
 * Primary tools available in the MPR (quad) viewport.
 * Annotation tools are only available with cornerstone.
 */
export type MprPrimaryTool =
  | 'crosshairs'
  | 'windowLevel'
  | 'pan'
  | 'zoom'
  | 'length'
  | 'angle'
  | 'rectangleRoi'
  | 'ellipticalRoi'

/** Tools that require annotation support (not yet available in dicomview). */
export const ANNOTATION_TOOLS: ReadonlySet<string> = new Set([
  'length',
  'angle',
  'rectangleRoi',
  'ellipticalRoi',
])

/** Returns true if the given tool requires annotation support. */
export function isAnnotationTool(tool: string): boolean {
  return ANNOTATION_TOOLS.has(tool)
}
