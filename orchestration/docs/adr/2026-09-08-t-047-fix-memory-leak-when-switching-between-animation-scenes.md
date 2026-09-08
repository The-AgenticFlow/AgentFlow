# T-047: Fix memory leak when switching between animation scenes

## Status
Accepted

## Context
Fixes a memory leak when switching between animation scenes (T-047).
`GamingBackground.jsx` creates an `Image()` with `onload`/`onerror` handlers inside a `useEffect` with no cleanup. Rapid scene-switch remounts leak the `Image` objects and their event listeners.
The codebase has no `<canvas>` usage — animations are pure DOM/framer-motion, which already clean up on unmount. The `Image` preloader was the leak root.
- `useEffect` now returns a cleanup that detaches `onload`/`onerror` and clears `src` to abort in-flight requests.
- Late image loads are guarded so they cannot set state after unmount.
- Added regression tests covering handler attach, detach, and src-clear on unmount.
- Test suite: 57 tests passed across 7 files
- Lint: 0 errors, 0 warnings

## Decision
Adopt the implementation approach described in T-047: Fix memory leak when switching between animation scenes.

## Consequences
T-047: Fix memory leak when switching between animation scenes is now implemented and merged into the main branch. This resolves ticket T-047.

## References
- Ticket: T-047
- PR: #51
- Date: 2026-09-08
