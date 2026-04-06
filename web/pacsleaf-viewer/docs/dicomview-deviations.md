# Dicomview Deviations from Cornerstone3D

This document tracks UX and functional differences between `@knopkem/dicomview` (Rust/WASM/WebGPU) and `@cornerstonejs/*` (WebGL2) as used in pacsleaf-viewer. Each item is a candidate for upstream fixes in the dicomview package.

---

## 1. Annotation Tools Not Available

**Status:** Known limitation — v1 scope exclusion  
**Impact:** High — clinical workflows depend on measurements  
**Details:** Length, Angle, RectangleROI, and EllipticalROI tools are disabled when the dicomview engine is active. The toolbar dims these buttons and shows a tooltip explaining they are unavailable.  
**Workaround:** Switch to cornerstone3D engine in Settings when annotations are needed.

## 2. Stack Viewport Uses Volume Engine Internally

**Status:** ✅ Resolved in @knopkem/dicomview 0.2.0  
**Impact:** None  
**Details:** Dicomview 0.2.0 introduces `StackViewer` — a dedicated single-canvas viewer that uses `SingleSliceEngine` internally. No hidden canvases needed.  
**Migration:** ✅ Done — `DicomviewStackViewport.tsx` and `stack.ts` updated to use `StackViewer.create({canvas})`. Hidden container div removed.

## 3. Preset Names Differ

**Status:** ✅ Resolved in @knopkem/dicomview 0.2.0  
**Impact:** None  
**Details:** The WASM layer now accepts both cornerstone PascalCase-hyphen (`CT-Bone`, `CT-Soft-Tissue`) and dicomview kebab-case (`ct-bone`, `ct-soft-tissue`). Case-insensitive matching. The preset-map.ts in pacsleaf-viewer is no longer strictly necessary but still useful for UI label display.

## 4. Mouse Interaction Requires Manual Wiring

**Status:** ✅ Resolved in @knopkem/dicomview 0.2.0  
**Impact:** None  
**Details:** Dicomview 0.2.0 ships `InputHandler` (for slice viewports) and `VolumeInputHandler` (for 3D viewport) classes that automatically wire pointer/wheel events to the Viewer API. Configurable active tool, left/middle/right button differentiation, and wheel handling are built-in.  
**Migration:** pacsleaf-viewer retains its own `mouse-handler.ts` with a new `bindStackInputs()` function tailored to the StackViewer API. The built-in InputHandler can be adopted later once it supports W/L state accumulation and crosshair tool.

## 5. No Progressive Slice-by-Slice Display During Load

**Status:** ✅ Clarified in @knopkem/dicomview 0.2.0  
**Impact:** None  
**Details:** The `DICOMwebLoader` already calls `viewer.render()` after each slice arrives. The `renderDuringLoad` option (default: `true`) is now explicitly documented and configurable. `StackViewer` also supports progressive rendering. The previous integration in pacsleaf-viewer used a progress bar overlay that hid the progressive rendering — this can be updated.

## 6. Container Type Difference (Canvas vs Div)

**Status:** Fundamental architecture — no fix needed  
**Impact:** None — separate React components handle each engine  
**Details:** Cornerstone mounts into `<div>` elements and manages its own internal canvas. Dicomview expects `<canvas>` elements directly. This requires separate React component trees for each engine.

## 7. Segmentation Overlays Not Supported

**Status:** Known limitation — v1 scope exclusion  
**Impact:** Medium — no label map or contour overlay display  
**Details:** Cornerstone supports segmentation label maps and contour overlays. Dicomview has no segmentation support.  
**Workaround:** Switch to cornerstone3D engine when segmentation viewing is needed.

## 8. StackViewer Missing Pan/Zoom

**Status:** Upstream fix needed in @knopkem/dicomview  
**Impact:** Low — stack viewers typically use W/L + scroll as primary interactions  
**Details:** The `StackViewer` class in dicomview 0.2.0 does not expose `pan()` or `zoom()` methods. In stack mode, the Pan and Zoom toolbar buttons are non-functional (silently ignored). Cornerstone's stack viewport supports both.  
**Upstream fix:** Add `pan(dx, dy)` and `zoom(factor)` methods to `StackViewer` / `SingleSliceEngine`.

---

*Last updated: v0.2.0 pacsleaf-viewer integration — StackViewer adopted, `auto` mode now prefers dicomview for all layouts*
