# 008 — Reduce the ray offset

- **Status**: DONE
- **Commit**: af2b9fc5
- **Severity**: LOW
- **Category**: Physicality and cohesion
- **Estimated scope**: 1 file, 3 changed values

## Problem

Each ray currently begins 38px from the visual center in its static transform
and both animation endpoints:

```css
/* apps/desktop/src/components/UpdateCards.css — current */
translateX(38px)
```

The requested experiment is a tighter 12px radial offset.

## Target

Replace all three ray-specific `translateX(38px)` values with
`translateX(12px)`: the base transform, the `from` keyframe, and the `to`
keyframe.

```css
translateX(12px)
```

The opaque icon tile remains above the rays and masks their inner portions.
Transform order remains unchanged, so each origin stays fixed and only local
`scaleX()` changes length.

## Repo conventions to follow

- Preserve the full transform list and local `scaleX()` animation.
- Preserve ray count, angle spacing, phase distribution, alternating lengths,
  duration, easing, opacity, and reduced-motion behavior.

## Steps

1. In `apps/desktop/src/components/UpdateCards.css`, replace exactly the three
   ray-transform occurrences of `translateX(38px)` with `translateX(12px)`.
2. Run the desktop build.

## Boundaries

- Do NOT change markup, ray count, angles, delays, lengths, scale range,
  opacity, icon tile, or z-index.
- Do NOT add dependencies or JavaScript animation.
- If there are not exactly three matching ray transforms, STOP and report.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: trigger update cards from Settings → Developer → Release UI:
  - Rays originate 12px from center and their inner sections remain masked.
  - Their fixed endpoints do not move between animation phases.
  - Only visible length changes; angle and radial position stay fixed.
  - Reduced motion remains static.
- **Done when**: all three transforms use 12px and the desktop build passes.
