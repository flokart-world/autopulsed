# Git Hooks

This directory contains git hooks for the autopulsed project.

## Setup

To use these hooks, run:

```bash
git config core.hooksPath .githooks
```

## Available Hooks

### pre-commit
Runs before each commit to ensure:
- Rust code is properly formatted (`cargo fmt`)
- No clippy warnings (`cargo clippy`)
- All files have final newlines
- No trailing spaces in code files

## Bypass Hook (Emergency Only)

If you absolutely need to commit without checks:

```bash
git commit --no-verify -m "Emergency commit"
```

**Warning**: Only use this in emergencies. Always fix issues before pushing.

## Future Improvements

If specific files need exceptions (e.g., test data with trailing spaces):
1. Create `.formatignore` file
2. Update hooks to respect ignore patterns
3. Document exceptions clearly
