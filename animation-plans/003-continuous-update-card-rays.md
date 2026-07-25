# 003 — Emit a continuous sixteen-ray field

- **Status**: SUPERSEDED
- **Commit**: af2b9fc5
- **Severity**: LOW
- **Category**: Purpose, cohesion, and accessibility
- **Estimated scope**: 2 files, about 50 changed lines

## Problem

The current eight-ray field settles at 24% opacity after one 220ms entrance.
It reads too faintly and becomes visually inert while the update card remains
open.

```tsx
// apps/desktop/src/components/UpdateCards.tsx:137 — current
<div className="update-card-visual" aria-hidden="true">
  <span />
  <span />
  <span />
  <Icon size={48} />
</div>
```

```css
/* apps/desktop/src/components/UpdateCards.css:126 — current */
.update-card-visual::after,
.update-card-visual > span {
  width: 148px;
  opacity: 0;
  animation: update-card-visual-ray-enter 220ms var(--motion-ease-out) forwards;
}
```

## Target

Use eight line axes to produce sixteen rays. The pseudo-element remains the
0-degree axis; seven spans rotate to 22.5, 45, 67.5, 90, 112.5, 135, and
157.5 degrees.

Every axis runs the same continuous emission:

```css
animation: update-card-visual-ray-flow 3.2s linear infinite;
```

```css
@keyframes update-card-visual-ray-flow {
  0% {
    opacity: 0.08;
    scale: 0.72 1;
  }
  58% {
    opacity: 0.42;
    scale: 1;
  }
  100% {
    opacity: 0;
    scale: 1.08 1;
  }
}
```

Stagger axes by 20ms from 0ms through 140ms. Increase the center color stop in
the ray gradient from 34% to 52% accent. Keep the opaque tile and z-index
masking unchanged.

With `prefers-reduced-motion: reduce`, stop the ray animation and show all
axes at `opacity: 0.32; scale: 1`.

## Repo conventions to follow

- Constant motion uses `linear`, per the animation audit.
- Only opacity and scale animate.
- The update cards are rare first-run UI, so continuous decorative motion is
  acceptable while the dialog is open.
- Reduced-motion rules already live at the bottom of
  `apps/desktop/src/components/UpdateCards.css`.

## Steps

1. In `apps/desktop/src/components/UpdateCards.tsx`, add four empty spans so
   the visual contains seven spans followed by the existing icon.
2. In `apps/desktop/src/components/UpdateCards.css`, add rotations and 20ms
   staggered delays for seven spans at the exact angles above.
3. Increase the ray gradient's accent mix to 52%.
4. Replace `update-card-visual-ray-enter` with the exact 3.2s infinite linear
   `update-card-visual-ray-flow` keyframes above.
5. Update reduced motion to keep all sixteen rays static at 32% opacity and
   full scale.

## Boundaries

- Do NOT change the center tile, icon, layout, release JSON, copy, or
  navigation.
- Do NOT rotate the full ray field.
- Do NOT animate layout properties or add JavaScript timers/dependencies.
- Do NOT animate the icon continuously.
- If the cited markup or selectors no longer match, STOP and report instead
  of improvising.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: use Settings → Developer → Release UI → Show update cards:
  - Sixteen evenly spaced rays are visible.
  - Rays repeatedly expand away from the masked center, then disappear before
    resetting; no inward snap is visible.
  - The 20ms stagger adds texture without reading as rotation.
  - The icon remains sharper and brighter than the rays.
  - All three cards use identical geometry and timing.
  - With `prefers-reduced-motion: reduce`, the field is static.
- **Done when**: the visual reads as an active but subordinate starburst and
  the desktop build passes.
