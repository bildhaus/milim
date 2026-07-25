# 004 — Flow independent god rays around the icon

- **Status**: SUPERSEDED
- **Commit**: af2b9fc5
- **Severity**: LOW
- **Category**: Physicality, cohesion, and accessibility
- **Estimated scope**: 2 files, about 60 changed lines

## Problem

The current visual uses eight full lines crossing the center. Scaling each
full line from its center makes the whole field read as one object being
squashed and stretched rather than individual rays breathing outward.

```tsx
// apps/desktop/src/components/UpdateCards.tsx:137 — current
<div className="update-card-visual" aria-hidden="true">
  <span />
  <span />
  <span />
  <span />
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
  transform-origin: center;
  animation: update-card-visual-ray-flow 3.2s linear infinite;
}
```

## Target

Render sixteen independent half-rays. Each begins at the 38px edge of the
76px icon tile and extends in one direction.

Use a generated ray wrapper:

```tsx
<div className="update-card-rays">
  {Array.from({ length: 16 }, (_, rayIndex) => (
    <span
      key={rayIndex}
      style={{ "--update-ray-index": rayIndex } as CSSProperties}
    />
  ))}
</div>
<Icon size={48} />
```

Each ray rotates by `rayIndex * 22.5deg`, then translates 38px from the
center. Its transform origin is its left edge. Odd rays are 64px long; even
rays are 44px long.

```css
.update-card-rays span {
  left: 50%;
  top: 50%;
  height: 1px;
  transform:
    rotate(calc(var(--update-ray-index) * 22.5deg))
    translateX(38px);
  transform-origin: left center;
  animation: update-card-visual-ray-breathe 3.4s var(--motion-ease-in-out) infinite alternate;
  animation-delay: calc(var(--update-ray-index) * -120ms);
}
```

Use a directional gradient that is strongest at the tile edge and fades
outward. Animate each ray independently:

```css
@keyframes update-card-visual-ray-breathe {
  from {
    opacity: 0.18;
    scale: 0.78 1;
  }
  to {
    opacity: 0.46;
    scale: 1.08 1;
  }
}
```

This is intentional continuous on-screen movement, so use the existing strong
`--motion-ease-in-out` token. With `prefers-reduced-motion: reduce`, show
static rays at `opacity: 0.32; scale: 1`.

## Repo conventions to follow

- Motion tokens live in `apps/desktop/src/styles.css:463`, including
  `--motion-ease-in-out: cubic-bezier(0.65, 0, 0.35, 1)`.
- Keep the opaque `::before` tile at z-index 1 and the icon at z-index 2.
- Animate only opacity and scale; rotation and translation stay static.
- The component already imports `CSSProperties`.

## Steps

1. In `apps/desktop/src/components/UpdateCards.tsx`, replace the seven direct
   spans with `.update-card-rays`, generated from exactly sixteen spans with
   the `--update-ray-index` child variable.
2. In `apps/desktop/src/components/UpdateCards.css`, remove `::after` and
   direct child spans from the ray implementation.
3. Add the absolute ray wrapper and sixteen one-way ray styles. Use 64px for
   odd rays and 44px for even rays.
4. Replace `update-card-visual-ray-flow` with the exact independent breathing
   keyframes above.
5. Update reduced-motion selectors for `.update-card-rays span`.

## Boundaries

- Do NOT change the center tile, icon size, card layout, release JSON, copy,
  or navigation.
- Do NOT animate the whole field, rotate rays during animation, or use
  symmetric lines through the icon.
- Do NOT add dependencies, JavaScript timers, or animated layout properties.
- If the cited markup or selectors no longer match, STOP and report instead
  of improvising.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: use Settings → Developer → Release UI → Show update cards:
  - Sixteen independent beams begin at the tile edge.
  - Long and short rays alternate evenly.
  - Staggered phases produce continuous flowing variation rather than a
    synchronized squash.
  - Ray origins remain fixed while only their outward length changes.
  - The icon stays static, sharp, and dominant.
  - With `prefers-reduced-motion: reduce`, all rays are static.
- **Done when**: the field reads as flowing god rays around a stable center
  icon and the desktop build passes.
