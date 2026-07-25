# 009 — Expand the field to 64 rays

- **Status**: DONE
- **Commit**: af2b9fc5
- **Severity**: LOW
- **Category**: Cohesion
- **Estimated scope**: 2 files, 5 changed values

## Problem

The visual currently renders 32 rays at 11.25-degree intervals with 212.5ms
phase spacing:

```tsx
/* apps/desktop/src/components/UpdateCards.tsx:139 — current */
{Array.from({ length: 32 }, (_, rayIndex) => (
```

```css
/* apps/desktop/src/components/UpdateCards.css — current */
rotate(calc(var(--update-ray-index) * 11.25deg))
animation-delay: calc(var(--update-ray-index) * -212.5ms);
```

The requested visual needs 64 rays while preserving full-circle coverage and
the complete 6.8-second phase distribution.

## Target

Render exactly 64 rays. Use `360deg / 64 = 5.625deg` spacing and
`6800ms / 64 = 106.25ms` phase spacing:

```tsx
{Array.from({ length: 64 }, (_, rayIndex) => (
```

```css
rotate(calc(var(--update-ray-index) * 5.625deg))
animation-delay: calc(var(--update-ray-index) * -106.25ms);
```

Apply the angle multiplier to the base transform and both keyframe transforms.

## Repo conventions to follow

- Preserve the generated spans and per-ray CSS variable.
- Preserve the 12px radial offset and local `scaleX()` transform order.
- Preserve alternating 44px/64px lengths, duration, easing, opacity, scale
  range, and reduced-motion behavior.

## Steps

1. In `apps/desktop/src/components/UpdateCards.tsx`, change ray count from 32
   to 64.
2. In `apps/desktop/src/components/UpdateCards.css`, change all three angle
   multipliers from `11.25deg` to `5.625deg`.
3. Change the delay multiplier from `-212.5ms` to `-106.25ms`.
4. Run the desktop build.

## Boundaries

- Do NOT change offset, transform order, lengths, duration, easing, opacity,
  scale range, icon, layout, or reduced-motion behavior.
- Do NOT add dependencies or JavaScript animation.
- If any cited value no longer matches, STOP and report.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: trigger update cards from Settings → Developer → Release UI:
  - Exactly 64 rays fill the circle evenly.
  - Long and short rays still alternate.
  - The 12px origins stay fixed and only ray lengths change.
  - Phases remain distributed with no synchronized field pulse.
  - Reduced motion remains static.
- **Done when**: the field contains 64 evenly spaced, independently phased
  rays and the desktop build passes.
