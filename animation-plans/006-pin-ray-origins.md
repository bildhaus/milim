# 006 — Pin ray origins while changing length

- **Status**: DONE
- **Commit**: af2b9fc5
- **Severity**: MEDIUM
- **Category**: Physicality and origin
- **Estimated scope**: 1 file, about 8 changed lines

## Problem

The ray's fixed rotation and 38px offset use `transform`, while its animated
length uses the independent `scale` property:

```css
/* apps/desktop/src/components/UpdateCards.css:142 — current */
transform:
  rotate(calc(var(--update-ray-index) * 22.5deg))
  translateX(38px);
transform-origin: left center;

/* apps/desktop/src/components/UpdateCards.css:194 — current */
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

CSS composes the independent scale with the transform, so the scale also
changes the translated offset. The ray's inner endpoint visibly moves instead
of remaining pinned to the icon tile.

## Target

Animate `scaleX()` inside the same transform list, after the fixed rotation
and translation:

```css
@keyframes update-card-visual-ray-breathe {
  from {
    opacity: 0.18;
    transform:
      rotate(calc(var(--update-ray-index) * 22.5deg))
      translateX(38px)
      scaleX(0.78);
  }
  to {
    opacity: 0.46;
    transform:
      rotate(calc(var(--update-ray-index) * 22.5deg))
      translateX(38px)
      scaleX(1.08);
  }
}
```

Transform functions apply from right to left, so `scaleX()` changes only the
ray's local length before the fixed 38px translation and rotation place it.
The left transform origin remains pinned.

Remove the now-unused `scale: 1` declaration from the reduced-motion rule.
With animation disabled, the existing static transform remains unchanged.

## Repo conventions to follow

- Keep the existing GPU-only `transform` and `opacity` animation.
- Keep the existing 3.4s duration, easing token, alternate direction, 425ms
  phase spacing, ray geometry, colors, and opacity values.
- Keep `transform-origin: left center`.

## Steps

1. In `apps/desktop/src/components/UpdateCards.css`, replace the independent
   `scale` values in `update-card-visual-ray-breathe` with the exact full
   transforms shown above.
2. Remove only `scale: 1` from the reduced-motion ray rule.
3. Run the desktop build.

## Boundaries

- Do NOT change markup, duration, easing, delays, ray count, geometry, colors,
  opacity, or icon animation.
- Do NOT add dependencies, JavaScript, or layout-property animation.
- If the cited selectors no longer match, STOP and report.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: trigger update cards from Settings → Developer → Release UI,
  then use slow motion or frame-by-frame inspection:
  - Every ray's inner endpoint remains fixed at the icon tile edge.
  - Only its outer endpoint moves as its length changes.
  - Rotation and radial position never change.
  - Reduced motion shows static rays at the same fixed positions.
- **Done when**: ray origins do not move at any animation phase and the
  desktop build passes.
