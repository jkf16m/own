# own

A code ownership ledger with human review. Line by line.

## Problem

AI generates code fast. Developers skip understanding it. Nothing tracks: **"have I read and understood every line?"**

## What own Does

Reads source files. Lets you mark lines as reviewed. Tracks your ownership through a codebase.

No version control. No diffs. No PRs. Just: **do you understand this code?**

## Core Concepts

### Ownership States

Each line can be in one of these states:

- `r` (reviewed) — read and understood
- `q` (questioned) — read but don't understand why
- `a` (approved) — understand and agree with the approach
- `x` (rejected) — understand but disagree (needs refactoring)

### The `.own` File Format

Human-readable text files stored alongside your code:

```
# file: src/main.rs
# snapshot: a1b2c3d4e5f6

1:r
2:r
3:a:User struct is well-designed
5-8:q:Why use String instead of &str here?
10-12:x:This approach won't scale
15-20:r
```

**Key features:**
- **Snapshot hash** — tracks file content hash when ownership was recorded
- **Line ranges** — `5-8:q` marks lines 5-8 as questioned
- **Annotations** — `3:a:User struct is well-designed` adds a note

### Handling File Changes

When a file changes after review:
1. Compute hash of current file
2. Compare with stored snapshot hash
3. If different, use diff to migrate ownership:
   - **Unchanged lines** → keep their state
   - **New lines** → unreviewed
   - **Deleted lines** → removed from tracking
4. Update snapshot hash

This means: **you don't have to re-review everything when code changes.**

### Ignore Patterns

Respects `.ownignore` or `.gitignore`:

```bash
# .ownignore
/target
Cargo.lock
*.o
node_modules/
```

- `.git` is always ignored
- Patterns follow gitignore syntax
- Use `.ownignore` for own-specific ignores

## Commands

```bash
# Initialize .own directory
own init

# Review a file line by line
own review src/main.rs

# Scan directory for files needing review
own scan           # scan current directory
own scan src       # scan src/ only

# Check ownership status
own status
own status src/main.rs
```

## TUI Interface

```
┌─────────────────────────────────────────────────────────┐
│ own test.rs  [15/66 lines reviewed]                     │
│ 22% ownership  r:8 q:4 a:7 x:3                         │
├─────────────────────────────────────────────────────────┤
│ ▸   1 │ use std::collections::HashMap;                  │
│   R   2 │                                              │
│   A   3 │ struct User {  ← User struct is well-designed │
│       4 │     name: String,                             │
│   Q   5 │     email: String,  ← Why use String?        │
│   Q   6 │     age: u32,                                 │
│       7 │ }                                             │
└─────────────────────────────────────────────────────────┘
 j/k move  r reviewed  Q questioned  a approved  x rejected  s save  q quit
```

## Data Storage

`.own/` directory contains `.own` files:

```
.own/
├── main.own        # ownership data for src/main.rs
├── lib.own         # ownership data for src/lib.rs
└── test.own        # ownership data for test.rs
```

## Git Integration

The `.own` directory is committed to git:

```bash
# After reviewing
git add .own/
git commit -m "review: track ownership for src/main.rs"
```

This means:
- Team sees what's been reviewed
- Ownership data travels with the code
- History of review progress

## Design Principles

1. **Human-first** — Designed for line-by-line reading, not batch operations
2. **Text-based** — Human-readable `.own` files, easy to diff
3. **Git-aware** — Snapshot hashes detect changes, migrate ownership
4. **Non-invasive** — Doesn't modify your source code
5. **Simple** — One purpose: track ownership

## Tech Stack

- **Language:** Rust
- **Storage:** Text-based `.own` files
- **CLI:** clap
- **TUI:** ratatui + crossterm
- **Hashing:** DefaultHasher for content tracking
- **Ignore:** glob patterns (gitignore-compatible)

## Why This Matters

Code you don't own will bite you. When it breaks at 3am, when you need to extend it, when you need to explain it to someone else.

own makes ownership explicit. You can see exactly what you understand and what you don't.

The goal isn't 100% ownership. The goal is **knowing** what you own and what you don't.

## Future Ideas

- **Smart diff migration** — Use git diff for better ownership migration
- **Team ownership** — Share ownership data across team
- **Editor plugin** — VS Code extension for inline ownership
- **CI integration** — Fail builds if ownership < threshold
- **Blame integration** — Show ownership status in git blame
- **Annotations UI** — Add/edit annotations in TUI
