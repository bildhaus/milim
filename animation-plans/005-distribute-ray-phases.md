# 005 — Distribute ray phases across the full cycle

- **Status**: DONE
- **Commit**: af2b9fc5
- **Severity**: LOW
- **Category**: Cohesion and physicality
- **Estimated scope**: 1 file, 1 changed line

## Problem

The sixteen rays use a 3.4-second alternating animation, making the full
repeat cycle 6.8 seconds. Their current 120ms stagger covers only 1.8 seconds,
so most rays grow and shrink together and the field still reads as one shape
being squashed and stretched.

```css
/* apps/desktop/src/components/UpdateCards.css */
animation-delay: calc(var(--update-ray-index) * -120ms);
```

## Target

Distribute all sixteen starting phases evenly across the complete 6.8-second
cycle: `6800ms / 16 = 425ms`.

```css
animation-delay: calc(var(--update-ray-index) * -425ms);
```

This keeps some rays growing while others shrink, preserving a stable overall
silhouette while retaining the continuous circular flow.

## Repo conventions to follow

- Keep the existing `3.4s`, easing token, alternate direction, ray lengths,
  opacity range, and scale range.
- Keep the existing reduced-motion behavior.

## Steps

1. In `apps/desktop/src/components/UpdateCards.css`, change the ray animation
   delay multiplier from `-120ms` to `-425ms`.
2. Run the desktop build.

## Boundaries

- Do NOT change markup, keyframes, timing duration, easing, geometry, colors,
  opacity, or reduced-motion behavior.
- Do NOT add dependencies or JavaScript animation.
- If the cited selector no longer matches, STOP and report.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: trigger update cards from Settings → Developer → Release UI.
  Opposing rays should visibly occupy different phases, with no synchronized
  whole-field pulse or squash.
- **Done when**: the field flows continuously around a stable silhouette and
  the desktop build passes.
