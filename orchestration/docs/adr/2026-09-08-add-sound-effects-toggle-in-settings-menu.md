# Add sound effects toggle in settings menu

## Status
Accepted

## Context
Closes #048
Add a settings panel where users can toggle sound effects on/off and adjust volume. Store preferences in localStorage with placeholder audio hooks for future sound files.
- **New**: `frontend/src/lib/soundSettings.js` — pure localStorage store with:
- `getSoundSettings()`, `setSoundSettings()`, `updateSoundSettings()` for persistence
- `playSound(name)` placeholder audio hook with a sound registry (`click`, `success`, `error`)
- Type sanitization and volume clamping (0-1)
- **New**: `frontend/src/components/SettingsPanel.jsx` — modal settings panel with:
- Sound effects toggle (role="switch")

## Decision
Implement changes described in PR #52 for ticket T-048.

## Consequences
Add sound effects toggle in settings menu is now implemented and merged into the main branch. This resolves ticket T-048.

## References
- Ticket: T-048
- PR: #52
- Date: 2026-09-08
