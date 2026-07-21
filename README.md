# own

> ⚠️ **DISCLAIMER**: This is an ongoing experiment. The code has NOT been reviewed by a human yet. Use at your own risk.

Track your code ownership — human review, line by line.

![Human Code Ownership](https://img.shields.io/badge/human%20reviewed-no%20-red)

## What is this?

`own` is a tool for tracking **human understanding** of code. Not AI-generated slop reviews — real, line-by-line human ownership.

**Status**: Experimental — actively being developed and tested.

## Install

```bash
cargo install --path .
```

## Usage

```bash
# Add files to track
own add src/main.rs

# Review a file (opens TUI)
own review src/main.rs

# Check ownership status
own status

# Extract report
own extract --format md     # Markdown
own extract --format json   # JSON for badges
```

## TUI Controls

| Key | Action |
|-----|--------|
| `j/k` | Move up/down |
| `g/G` | Go to top/bottom |
| `t` | Toggle last tag (first time: pick tag) |
| `T` | Always open tag picker |
| `r` | Remove last tag |
| `v` | Start selection |
| `j/k` (in selection) | Extend selection |
| `n` | Add/edit annotation |
| `a` | View annotation |
| `s` | Save |
| `:q` | Quit |
| `:w` | Save and quit |
| `:wq` | Save and quit |
| `:d` | Quit without saving |

## TUI Visualization

```
│   ▸  1 │ fn main() {
│     2 ├ init();              ← has annotation
│     3 │     run();
│   ┌  4 │ process(data);      ← start of range annotation
│   │  5 │     validate();     ← middle of range
│   │  6 │     save();         ← middle of range
│   └  7 │ }                    ← end of range
│     8 │ 
│     9 ● reviewed             ← has tag
│    10 │ // done
```

| Marker | Meaning |
|--------|---------|
| `●` | Single line annotation OR tag |
| `┌` | Start of range annotation |
| `│` | Middle of range |
| `└` | End of range |
| `▸` | Current line |

## Annotations

Annotations are notes attached to lines or ranges:

```bash
# Add annotation
own review src/main.rs
# Press 'n', type note, Enter

# View annotation
# Press 'a' on annotated line

# Edit annotation
# Press 'n' on existing annotation

# Delete annotation
# Press 'n', clear text, Enter
```

## Tags

```bash
own tags list                    # List tags
own tags create reviewed         # Auto-generate color
own tags create urgent "#ff0000" # Custom color
own tags delete urgent           # Delete tag
```

## How it works

1. `own add` — add files to track
2. `own review` — open TUI, tag lines as reviewed
3. `own status` — see ownership percentage
4. `own extract` — export for reports/badges
5. `make badge` — update README badge

## .own Files

Ownership data stored in `.own/` directory:

```
.own/
├── src/
│   ├── main.rs.own
│   └── lib.rs.own
└── tags
```

Format:
```
snapshot: abc123
1:reviewed:hash1:alice:2024-01-20T10:00:00Z
2:approved:hash2:bob:2024-01-20T10:05:00Z
@5-10:This code looks correct
@3:Single line note
```

## Merge Support

- Each review includes author and timestamp
- Annotations survive file edits (content-based re-anchoring)
- Deleted lines: annotations are removed
- Multiple reviewers: all reviews stored

## License

MIT
