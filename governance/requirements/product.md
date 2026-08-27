# Product requirements

> **Status: NORMATIVE**
> **Requirement prefix: `REQ-PROD`**
> **Primary policy:** `BUS-MIS-*`

### REQ-PROD-001 — Artistic title insertion on moving scenes
Plaque Forge shall add or replace artistic title content on a video writing surface while preserving the scene motion and
relationships required for the title to appear attached to that surface.

### REQ-PROD-002 — Multiple writing-surface shapes
Writing surfaces shall support at least rectangular/rounded, circular/elliptical, polygonal/irregular-mask, and injected
transparent-image forms without assuming one universal rectangle geometry.

### REQ-PROD-003 — Automatic-first workflow with human correction
The normal workflow shall perform surface selection/tracking, writable-region reconstruction, and foreground discovery
automatically where practical, while allowing sparse human scene corrections when automatic intent is visibly wrong.

### REQ-PROD-004 — Analyze once, render many
Once a compatible complete analysis exists for a source/scene, users shall be able to render different titles, fonts or
styles without rerunning scene analysis merely because presentation changed.

### REQ-PROD-005 — Source-title removal is out of scope
Plaque Forge shall not claim to erase or inpaint an existing title from source imagery. Workflows that rely on a clean
writing surface shall require/declare a text-free source or an externally prepared clean plate.

### REQ-PROD-006 — Programmatic workflow access
Major analyze/render/verify/homologate operations shall be callable programmatically without requiring callers to
construct CLI parsing types or shell out to Plaque Forge itself.

### REQ-PROD-007 — Bundled and checkout media semantics
When `bundle-media` is enabled, canonical embedded media/styles/scenes/analysis shall behave equivalently to their checkout
counterparts apart from the source/storage mechanism and explicitly documented external runtime dependencies.
