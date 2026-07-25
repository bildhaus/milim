# 007 — Double the ray density

- **Status**: DONE
- **Commit**: af2b9fc5
- **Severity**: LOW
- **Category**: Cohesion
- **Estimated scope**: 2 files, 5 changed values

## Problem

The visual currently renders sixteen rays at 22.5-degree intervals:

```tsx
/* apps/desktop/src/components/UpdateCards.tsx:139 — current */
{Array.from({ length: 16 }, (_, rayIndex) => (
```

```css
/* apps/desktop/src/components/UpdateCards.css — current */
rotate(calc(var(--update-ray-index) * 22.5deg))
animation-delay: calc(var(--update-ray-index) * -425ms);
```

The requested visual needs twice the ray density without changing the radial
coverage, pinned-origin behavior, or full-cycle phase distribution.

## Target

Render exactly 32 rays. Space them evenly around 360 degrees:
`360deg / 32 = 11.25deg`.

Keep the 3.4-second alternating animation, whose complete phase cycle is 6.8
seconds, evenly distributed across all rays:
`6800ms / 32 = 212.5ms`.

```tsx
{Array.from({ length: 32 }, (_, rayIndex) => (
```

```css
rotate(calc(var(--update-ray-index) * 11.25deg))
animation-delay: calc(var(--update-ray-index) * -212.5ms);
```

Apply the 11.25-degree multiplier both to the static transform and to both
keyframe transforms.

## Repo conventions to follow

- Preserve the generated span structure and per-ray CSS variable.
- Preserve alternating 44px/64px lengths through the existing `nth-child`.
- Preserve local `scaleX()` after `translateX(38px)` so origins remain pinned.
- Preserve existing duration, easing, opacity, scale, and reduced-motion rules.

## Steps

1. In `apps/desktop/src/components/UpdateCards.tsx`, change the ray count from
   16 to 32.
2. In `apps/desktop/src/components/UpdateCards.css`, change all three ray angle
   multipliers from `22.5deg` to `11.25deg`.
3. Change the delay multiplier from `-425ms` to `-212.5ms`.
4. Run the desktop build.

## Boundaries

- Do NOT change layout, ray lengths, transform order, duration, easing,
  opacity, scale range, icon, or reduced-motion behavior.
- Do NOT add dependencies or JavaScript animation.
- If any cited value no longer matches, STOP and report.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: trigger update cards from Settings → Developer → Release UI:
  - Exactly 32 rays fill the circle evenly with no angular gap.
  - Long and short rays still alternate.
  - Inner endpoints remain fixed; only length changes.
  - Phases remain evenly spread, with no synchronized whole-field pulse.
  - Reduced motion remains static.
- **Done when**: ray density is doubled without changing the established
  motion behavior and the desktop build passes.
