---
name: documentation
description: Skill for maintaining project documentation
---

# Documentation Skill

## Documentation Types

### User Documentation
- `README.md`: Project overview, setup, usage
- `docs/usage/`: How-to guides
- `docs/api/`: API reference

### Developer Documentation
- `CONTRIBUTING.md`: Contribution guidelines
- `docs/architecture/`: System design
- `.agent/standards/`: Coding standards

### Changelog
- `CHANGELOG.md`: User-facing changes

## When to Update

Update documentation when:
- New feature added
- Breaking change introduced
- API modified
- Configuration changed
- Dependencies updated

## Documentation Style

### README Updates
- Keep concise
- Update examples
- Reflect current API

### API Documentation
- Document all public interfaces
- Include examples
- Document error conditions

### Architecture Docs
- Update diagrams if structure changed
- Document new patterns
- Update decision records

## Commit Format

```
docs: update README for new auth feature

- Add OAuth2 setup instructions
- Update configuration example
- Add troubleshooting section
```
