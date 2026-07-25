# 001 — Animate feature-specific update-card visuals

- **Status**: SUPERSEDED
- **Commit**: af2b9fc5
- **Severity**: MEDIUM
- **Category**: Missed opportunities
- **Estimated scope**: 2 files, about 100 lines

## Problem

Every update card uses the same static orbital treatment, so the illustration
does not explain the feature represented by the configured icon.

```tsx
// apps/desktop/src/components/UpdateCards.tsx:136 — current
<div className="update-card-body">
  <div className="update-card-visual" aria-hidden="true">
    <span />
    <Icon size={48} />
    <span />
  </div>
```

```css
/* apps/desktop/src/components/UpdateCards.css:110 — current */
.update-card-visual::before,
.update-card-visual::after,
.update-card-visual > span {
  position: absolute;
  content: "";
  border: 1px solid color-mix(in srgb, var(--update-card-accent) 34%, transparent);
  border-radius: 999px;
}
```

The component is rare, first-run UI, so a short explanatory entrance can add
delight without slowing frequent work.

## Target

Use the existing JSON `icon` as the visual mode. Do not add another release
schema field:

```tsx
<div className="update-card-visual" data-visual={item.icon} aria-hidden="true">
```

Create three static diagrams with CSS selectors:

- `git-pull-request`: a branching connection with two endpoint nodes.
- `plug`: two connector rails converging on the plug.
- `file-text`: two offset document layers behind the file icon.

Each diagram plays once whenever its card becomes active:

- Container: opacity `0` to `1`, transform `translateY(6px) scale(0.97)` to
  `translateY(0) scale(1)`, `240ms var(--motion-ease-out)`.
- Icon: opacity `0` to `1`, transform `translateY(5px) scale(0.94)` to
  `translateY(0) scale(1)`, `260ms var(--motion-ease-out)`.
- Diagram layers: opacity `0` to `1`, scale `0.9` to `1`,
  `260ms var(--motion-ease-out)`.
- Endpoint nodes use the same animation with delays of `40ms` and `80ms`.
- No looping or ambient animation.

Reduced motion must remove movement and retain only a
`var(--motion-fast)` opacity fade.

## Repo conventions to follow

- Motion tokens already live in `apps/desktop/src/styles.css:463`:
  `--motion-fast: 120ms`, `--motion-standard: 180ms`,
  `--motion-ease-out: cubic-bezier(0.16, 1, 0.3, 1)`.
- Existing update-card transitions use those tokens in
  `apps/desktop/src/components/UpdateCards.css:48`.
- The active card content unmounts and remounts on navigation, so CSS
  one-shot keyframes naturally replay without React state or timers.

## Steps

1. In `apps/desktop/src/components/UpdateCards.tsx`, add
   `data-visual={item.icon}` to `.update-card-visual`. Do not change the JSON
   schema or add elements.
2. In `apps/desktop/src/components/UpdateCards.css`, replace the shared orbital
   geometry with base pseudo-element/node styles plus the three exact
   `[data-visual="..."]` variants above.
3. Add one-shot container, icon, layer, and node entrance keyframes using only
   `transform`, `scale`, and `opacity`.
4. Extend the existing `prefers-reduced-motion` block so all new visual
   animation becomes an opacity-only fade.

## Boundaries

- Do NOT touch release copy, release validation, navigation, or card layout.
- Do NOT add or change fields in `releases.json`.
- Do NOT add dependencies or JavaScript timers.
- Do NOT create infinite animations.
- If the cited markup or selectors no longer match, STOP and report instead
  of improvising.

## Verification

- **Mechanical**: run `pnpm -C apps/desktop build`; it must exit 0.
- **Feel check**: use Settings → Developer → Release UI → Show update cards,
  then confirm:
  - Pull requests, agent runtimes, and Workspace show visibly different
    diagrams.
  - Each diagram settles once on entry and remains still.
  - Rapid Back/Next navigation never delays input.
  - At 10% playback speed, layers and nodes resolve in order without a jump.
  - With `prefers-reduced-motion: reduce`, movement disappears and the visual
    only fades in.
- **Done when**: all three cards have distinct, stable diagrams, replay on
  card entry, respect reduced motion, and the desktop build passes.
