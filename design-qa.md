# Pull Requests panel design QA

- Reference: `codex-clipboard-2d8f50fe-6c9b-44ab-ad79-5cf5b2352905.png`
- Prototype: Milim desktop Vite preview at 1447 × 1361 with representative PR data
- Compared state: All filter, first PR selected, Summary tab

## Result

- P0: none
- P1: none
- P2: none
- P3: list rows show repository, PR number, age, and comment count instead of the reference's branch and line totals because GitHub account search does not return branch/diff statistics without loading every PR.

The two-pane hierarchy, dark surfaces, selected-row treatment, search and filter controls, summary navigation, GitHub profile images, PR metadata, Markdown description, checks/comments disclosures, action placement, and density match the reference within Milim's existing resizable sheet shell. The sheet shell is intentionally modal rather than full-window because the requested behavior follows Media Studio.

Interaction checks passed for opening from Tools, All/Reviewing/Authored filtering, search, selection updates, Summary/Timeline/Code navigation, changed-file rendering, outer resize control, keyboard control of the list/detail divider, persisted divider size, immediate restoration of the selected PR and cached details on reopen, background refresh, the five-row Tools reveal without clipping, and zero browser console errors.

final result: passed

---

# Slides thumbnail rail design QA

- Source visual truth: `C:\Users\USER\AppData\Local\Temp\codex-clipboard-a6208221-ca39-4481-b880-5123d0bb00f3.png`
- Source pixels: 180 × 139
- Compared state: selected slide, dark theme, editable deck
- Implementation screenshot, viewport, CSS size, and density: unavailable

## Full-view and focused-region evidence

Blocked: the running Tauri webview could not be attached through the available DevTools connection, so no implementation screenshot or pixel comparison could be captured.

## Findings

- P0: rendered comparison unavailable; thumbnail sizing, action placement, and clipping remain unverified in the actual desktop runtime.

## Comparison history

- Pass 1: inspected the source crop, implemented the thumbnail-rail treatment, and attempted DevTools capture; attachment failed because its browser profile was already in use.

final result: blocked
