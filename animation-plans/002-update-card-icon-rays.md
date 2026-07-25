# 002 — Radiate update-card lines from the icon

- **Status**: SUPERSEDED
- **Commit**: af2b9fc5
- **Severity**: LOW
- **Category**: Cohesion and missed opportunities
- **Estimated scope**: 2 files, about 45 changed lines

## Problem

The current visual has one horizontal line and two endpoint dots. The line is
visible through the translucent center tile, so it competes with the icon
instead of reading as light emitted from behind it.

```tsx
// apps/desktop/src/components/UpdateCards.tsx:137 — current
<div className="update-card-visual" aria-hidden="true">
  <span />
  <Icon size={48} />
  <span />
</div>
```

```css
/* apps/desktop/src/components/UpdateCards.css:114 — current */
.update-card-visual::before {
  z-index: 0;
  width: 76px;
  height: 76px;
  background: color-mix(in srgb, var(--bg-tertiary) 66%, transparent);
}

.update-card-visual::after {
  z-index: 0;
  width: 130px;
  height: 1px;
}
```

## Target

Render four faint line axes behind the icon tile, producing eight rays at
0, 45, 90, 135, 180, 225, 270, and 315 degrees.

- The tile must use an opaque `var(--bg-secondary)` base and `z-index: 1`, so
  no line is visible underneath its 76px square.
- The icon remains `z-index: 2`.
- Every ray is 148px by 1px with a transparent-edge linear gradient.
- Rays settle at `opacity: 0.24`.
- Rays animate from `scale: 0.76 1` to `scale: 1` using
  `220ms var(--motion-ease-out)`.
- Stagger the four axes by `0ms`, `20ms`, `40ms`, and `60ms`; total motion
  remains under 300ms.
- Remove endpoint dots and their node animation.
- Reduced motion keeps the rays at their final opacity and scale with no
  movement; the existing visual fade remains.

## Repo conventions to follow

- The project uses plain CSS and existing tokens:
  `--motion-fast: 120ms` and
  `--motion-ease-out: cubic-bezier(0.16, 1, 0.3, 1)` in
  `apps/desktop/src/styles.css:463`.
- `apps/desktop/src/components/UpdateCards.css` owns this component's motion.
- Animate only opacity and transform/scale. Do not animate layout properties.

## Steps

1. In `apps/desktop/src/components/UpdateCards.tsx`, change the visual markup
   to three empty spans followed by the existing icon. The pseudo-element plus
   three spans provide the four axes.
2. In `apps/desktop/src/components/UpdateCards.css`, make `::before` the
   opaque center mask at `z-index: 1`.
3. Style `::after` and all three spans as identical ray axes at `z-index: 0`.
   Rotate the spans to 45, 90, and 135 degrees.
4. Replace the signal/node keyframes with one ray keyframe from
   `opacity: 0; scale: 0.76 1` to `opacity: 0.24; scale: 1`.
5. Update reduced-motion rules so rays have `animation: none`,
   `opacity: 0.24`, and `scale: 1`.

## Boundaries

- Do NOT change the update-card layout, copy, icon size, release JSON, or
  navigation.
- Do NOT add SVG assets, dependencies, JavaScript timers, or looping motion.
- Do NOT make rays feature-specific; all cards use the same visual system.
- If the cited markup or selectors no longer match, STOP and report instead
  of improvising.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: use Settings → Developer → Release UI → Show update cards:
  - Eight rays expand outward from behind the tile.
  - The tile fully masks every ray at the center.
  - Rays settle faintly and remain subordinate to the icon.
  - All three cards use identical ray geometry and timing.
  - At 10% playback speed, the 20ms stagger reads as one coherent burst.
  - With `prefers-reduced-motion: reduce`, the rays do not move.
- **Done when**: the icon is the unobstructed center point of one quiet,
  consistent eight-ray burst and the desktop build passes.
