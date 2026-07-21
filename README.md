# own

Track your code ownership — human review, line by line.

![Human Code Ownership](https://img.shields.io/badge/human-0.0%25-blue)

## What is this?

`own` is a tool for tracking **human understanding** of code. Not AI-generated slop reviews — real, line-by-line human ownership.

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
| `t` | Toggle last tag |
| `T` | Pick a tag |
| `r` | Remove tag |
| `v` | Select lines |
| `n` | Add annotation |
| `a` | View annotation |
| `s` | Save |
| `:q` | Quit |

## Tags

```bash
own tags list                    # List tags
own tags create reviewed         # Auto-generate color
own tags create urgent "#ff0000" # Custom color
```

## How it works

1. `own add` — add files to track
2. `own review` — open TUI, tag lines as reviewed
3. `own status` — see ownership percentage
4. `own extract` — export for reports/badges

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
1:reviewed
5-10:approved
@5-10:This code looks correct
```

## License

MIT
