---
name: changelog
description: Skill for maintaining CHANGELOG.md
---

# Changelog Skill

## CHANGELOG Format

Follow [Keep a Changelog](https://keepachangelog.com/):

```markdown
# Changelog

## [Unreleased]

### Added
- New feature X

### Changed
- Modified behavior Y

### Fixed
- Bug fix Z

## [1.2.0] - 2024-01-15

### Added
- Feature A
```

## Categories

| Category | Use For |
|----------|---------|
| `Added` | New features |
| `Changed` | Modified behavior |
| `Deprecated` | Soon-to-be removed |
| `Removed` | Removed features |
| `Fixed` | Bug fixes |
| `Security` | Security fixes |

## Entry Style

### Good Entry
```
- Add OAuth2 authentication support (#42)
```

### Bad Entry
```
- Fixed stuff
- Updated code
```

## Process

1. Read PR description from SENTINEL's final-review.md
2. Categorize changes
3. Write concise, user-focused entry
4. Place under `[Unreleased]` section
5. Commit with message: `docs: update CHANGELOG for T-{id}`

## Release

When version is released:
1. Rename `[Unreleased]` to `[version] - date`
2. Add new `[Unreleased]` section
3. Tag release in git
