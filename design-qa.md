# PR Cockpit Design QA

## Comparison target

- Source visual truth:
  - `C:\Users\USER\AppData\Local\Temp\codex-clipboard-183854df-f6b5-48a2-8cf2-fc29628016ac.png`
  - `C:\Users\USER\AppData\Local\Temp\codex-clipboard-dd301700-cc43-4721-8adf-02934589fe83.png`
- Browser-rendered implementation:
  - `C:\Users\USER\.codex\visualizations\2026\07\25\019f9845-ac78-7e73-9489-5882a1c53530\pr-cockpit-implementation.png`
  - `C:\Users\USER\.codex\visualizations\2026\07\25\019f9845-ac78-7e73-9489-5882a1c53530\pr-cockpit-narrow.png`
- Combined same-input comparison:
  - `C:\Users\USER\.codex\visualizations\2026\07\25\019f9845-ac78-7e73-9489-5882a1c53530\pr-cockpit-comparison.png`
- Reference pixels: cockpit `752 x 1357`; sidebar detail `135 x 124`.
- Implementation pixels: full harness `900 x 1354`; normalized cockpit crop `752 x 1354`; narrow state `600 x 1000`.
- CSS viewport and density: `900 x 1354` at device scale factor `1` for the primary comparison; `600 x 1000` at device scale factor `1` for the narrow-layout check.
- State: dark theme, open non-draft PR, approved review decision, all checks passing, merge-ready, one review, one comment, PR subview selected.

## Evidence

### Full-view comparison

The normalized side-by-side comparison shows the same dense, flat inspector hierarchy as the source: compact identity and state header, branch/review/comment/check rows, two equal-width merge actions, Markdown description, and divided disclosure sections. The implementation intentionally adds the requested Changes / PR #N sub-navigation and renders populated review/comment content. It uses Milim's existing system font, theme tokens, icon family, disclosure behavior, and panel chrome rather than copying foreign window controls from the reference.

### Focused-region comparison

The bottom row of the combined comparison isolates the sidebar treatment. Both source and implementation use a compact pull-request glyph with a small semantic status dot at the row edge. The implementation's accessible name supplies PR number, lifecycle, checks, review state, and readiness so the status is not color-only.

### Required fidelity surfaces

- Fonts and typography: Milim's configured system sans and mono stacks render at the existing 14 px app base, with clear title, metadata, Markdown, and code hierarchy. No unintended serif fallback remains.
- Spacing and layout rhythm: the cockpit preserves the reference's flat sections and wide action row. The `472 px` narrow inspector measured `clientWidth === scrollWidth` for both panel and PR workspace, with actions and metadata reflowing without horizontal overflow.
- Colors and tokens: backgrounds, dividers, text hierarchy, and green merge-ready/check states come from Milim theme variables. Other semantic tones are covered by state-unit tests.
- Image quality and assets: no raster imagery is required. All visible interface symbols use Milim's existing icon component library; there are no handcrafted SVG, CSS-art, emoji, or placeholder substitutes.
- Copy and content: labels are concise and contextual. GitHub-only boundaries remain explicit through “Open on GitHub” and external check targets.

## Interaction and accessibility checks

- Changes / PR #N navigation is present and the PR view is the selected target.
- Description, checks, and comments/reviews are native keyboard-reachable disclosures.
- Review modal opens; switching to Request changes disables submission until a body is present.
- Merge modal opens with Merge commit, Squash, and Rebase; confirmation is disabled before a method is selected and enabled after Squash while displaying the guarded head SHA.
- Prepare to merge drafts the agent prompt without sending it.
- Comment submit is disabled for an empty body.
- The sidebar PR control has a complete accessible label.
- No browser console errors were reported.

## Findings

No actionable P0, P1, or P2 fidelity findings remain. The extra Git sub-navigation, status detail, and populated conversation are intentional requirements rather than source drift.

## Comparison history

- Pass 1: the isolated render lacked the app's theme initialization, producing a serif fallback and missing surface tokens. Fixed by initializing the existing theme store in the temporary QA harness.
- Pass 2: the corrected render used Milim typography and tokens. The normalized full-view and focused sidebar comparison found no actionable P0/P1/P2 differences.

## Follow-up polish

No blocking polish items. Live GitHub content will naturally vary in height and density from the reference.

final result: passed
