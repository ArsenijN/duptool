# New Version System - Complete Guide

## Overview

The new version system provides:
- ✓ Single source of truth (Cargo.toml)
- ✓ Automatic version everywhere
- ✓ Build date tracking
- ✓ Daily build counter (resets each day)
- ✓ Git commit tracking
- ✓ Short and verbose version outputs

## Files Added/Modified

### 1. `build.rs` (NEW)
Place this in the root directory (same level as Cargo.toml):
- Runs at compile time
- Generates build metadata
- Tracks build counter per day
- Gets git commit hash

### 2. `Cargo.toml` (MODIFIED)
Added:
```toml
[build-dependencies]
chrono = "0.4"
```

### 3. `main.rs` (MODIFIED)
- Uses `env!("CARGO_PKG_VERSION")` from Cargo.toml
- Custom `-v` and `-V` flags
- Verbose `--help` restored

## Version Output Examples

### Short Version: `-v` or `--version`
```bash
$ duptool -v
duptool 0.1.10
```

Clean and simple!

### Verbose Version: `-V` or `--verbose-version`
```bash
$ duptool -V
duptool 0.1.10+build20260205.3
Build date: 20260205
Build number: 3
Git commit: a3f9b2c

Author: Arsenii Nochevnyi <arsenij.nocevnyj@gmail.com>
Repository: https://github.com/ArsenijN/duptool
License: MIT
```

Shows everything!

## Help Output Examples

### Compact Help: `-h`
```bash
$ duptool -h
Advanced duplicate file finder with smart comparison strategies

Usage: duptool [OPTIONS] <folder1> [folder2]

Arguments:
  <folder1>  First folder to compare
  [folder2]  Second folder to compare

Options:
  -c, --content    Compare by file content
  -n, --name       Compare by file name
  ...
  -v, --version    Print version
  -V, --verbose-version    Print detailed version information
  -h, --help       Print help

Use -h for compact help, --help for detailed help.
Use -v for version, -V for detailed version.
```

### Detailed Help: `--help`
```bash
$ duptool --help
duptool - Advanced duplicate file finder

Fast, efficient tool for finding duplicate files across directories.
Features: async processing, HDD optimization, quick content check,
safe file deletion, and cross-device move support.

Version: 0.1.10
Author: Arsenii Nochevnyi <arsenij.nocevnyj@gmail.com>
Repository: https://github.com/ArsenijN/duptool

Usage: duptool [OPTIONS] <folder1> [folder2]

Arguments:
  <folder1>
          First folder to compare (or the only folder in --single mode)
  [folder2]
          Second folder to compare (omit for --single mode)

Options:
  -c, --content
          Compare by file content
  ... (full descriptions)

Use -h for compact help, --help for detailed help.
Use -v for version, -V for detailed version.
```

### Error Message: No arguments
```bash
$ duptool
error: the following required arguments were not provided:
  <folder1>

Usage: duptool [OPTIONS] <folder1> [folder2]

For more information, try '-h' for compact help or '--help' for detailed help.
```

(Note: The actual clap error message will say "try '--help'" but your after_help text guides users)

## How Build Counter Works

The build counter increments for each build on the same day:

```
First build today:    0.1.10+build20260205.0
Second build today:   0.1.10+build20260205.1
Third build today:    0.1.10+build20260205.2

Tomorrow first build: 0.1.10+build20260206.0
```

This helps track:
- Which specific build someone is using
- When testing multiple builds per day
- Debug vs release builds (separate counters)

## How It Works

### 1. At Compile Time

`build.rs` runs and:
1. Gets current date: `20260205`
2. Checks last build date in `OUT_DIR/build_date.txt`
3. If same day: increments counter in `OUT_DIR/build_counter.txt`
4. If new day: resets counter to 0
5. Gets git commit hash: `a3f9b2c`
6. Checks if repo is dirty (uncommitted changes)
7. Generates environment variables:
   - `BUILD_DATE=20260205`
   - `BUILD_NUMBER=3`
   - `GIT_HASH=a3f9b2c` (or `a3f9b2c-dirty`)
   - `FULL_VERSION=0.1.10+build20260205.3`

### 2. At Runtime

`main.rs` reads these with:
- `env!("CARGO_PKG_VERSION")` - from Cargo.toml
- `option_env!("BUILD_DATE")` - from build.rs
- `option_env!("BUILD_NUMBER")` - from build.rs
- `option_env!("GIT_HASH")` - from build.rs
- `option_env!("FULL_VERSION")` - from build.rs

`option_env!` is used for build info (returns Option) so it gracefully handles missing values.

## Single Source of Truth

Now you only update version in ONE place:

### Cargo.toml
```toml
[package]
version = "0.1.11"  # ← ONLY PLACE TO CHANGE!
```

Everything else is automatic:
- ✓ `--help` shows correct version
- ✓ `-v` shows correct version
- ✓ `-V` shows correct version with build info
- ✓ Code can access via `env!("CARGO_PKG_VERSION")`

## Build Examples

### Development build:
```bash
$ cargo build
$ ./target/debug/duptool -V
duptool 0.1.10+build20260205.0
Build date: 20260205
Build number: 0
Git commit: a3f9b2c-dirty
...
```

### Release build:
```bash
$ cargo build --release
$ ./target/release/duptool -V
duptool 0.1.10+build20260205.1
Build date: 20260205
Build number: 1
Git commit: a3f9b2c
...
```

Note: Debug and release have separate counters (different OUT_DIR).

## Git Integration

### Clean repository:
```bash
$ git status
# On branch main
# nothing to commit, working tree clean

$ cargo build --release
$ ./target/release/duptool -V
Git commit: a3f9b2c
```

### Dirty repository (uncommitted changes):
```bash
$ # Edit some files
$ cargo build --release
$ ./target/release/duptool -V
Git commit: a3f9b2c-dirty
```

This helps identify if someone is running a modified version!

## Build Counter Storage

The counter is stored in:
```
target/debug/build/duptool-<hash>/out/build_counter.txt
target/debug/build/duptool-<hash>/out/build_date.txt
```

This persists across builds until you run `cargo clean`.

## Advantages

### Before (Manual Versioning):
```rust
// In main.rs:
.version("0.1.10")  // ← Must update here

// In Cargo.toml:
version = "0.1.10"  // ← And here

// In README:
duptool 0.1.10      // ← And here

// In CHANGELOG:
## Version 0.1.10    // ← And here
```

Risk: Forgetting to update one place → inconsistent versions!

### After (Automatic):
```toml
# In Cargo.toml:
version = "0.1.11"  # ← ONLY HERE!
```

Everything else updates automatically!

## Version Bump Workflow

### Old way:
1. Edit Cargo.toml version
2. Edit main.rs version  
3. Edit README.md version
4. Edit CHANGELOG.md version
5. Commit: "Bump version to 0.1.11"

### New way:
1. Edit Cargo.toml version
2. Commit: "Bump version to 0.1.11"

Done! Build system handles everything else.

## Customization Options

### Want to show time too?
In `build.rs`, change:
```rust
.format("%Y%m%d")  // Just date
```
to:
```rust
.format("%Y%m%d-%H%M%S")  // Date and time
```

Result: `0.1.10+build20260205-143052.0`

### Want different counter reset?
Currently resets daily. To reset weekly:
```rust
.format("%Y-W%W")  // Year-Week
```

Result: `0.1.10+build2026-W05.0`

### Don't want git hash?
Remove from `build.rs`:
```rust
// Comment out or remove:
// let git_hash = Command::new("git")...
// println!("cargo:rustc-env=GIT_HASH={}", git_hash);
```

## Troubleshooting

### "BUILD_DATE not found"
- Solution: Make sure `build.rs` is in project root
- Check `[build-dependencies]` in Cargo.toml

### Counter doesn't increment
- Solution: `cargo clean` and rebuild
- The counter is in `target/` which cargo clean removes

### Git hash shows "unknown"
- Solution: Initialize git repo: `git init`
- Or it's fine - just means not in a git repo

### Wrong version shown
- Solution: Make sure you updated Cargo.toml
- Rebuild: `cargo build --release`

## Testing the System

```bash
# Clean build
cargo clean

# First build today
cargo build --release
./target/release/duptool -V
# Should show: build20260205.0

# Second build today
touch src/main.rs  # Trigger rebuild
cargo build --release
./target/release/duptool -V
# Should show: build20260205.1

# Test version formats
./target/release/duptool -v
# Should show: duptool 0.1.10

./target/release/duptool -V
# Should show: Full version with build info

# Test help
./target/release/duptool -h
# Should show: Compact help

./target/release/duptool --help
# Should show: Detailed help with version info

# Test error message
./target/release/duptool
# Should suggest: try '-h' or '--help'
```

## Summary

You now have a professional versioning system that:
- ✅ Automatically pulls version from Cargo.toml
- ✅ Shows simple version with `-v`
- ✅ Shows detailed version with `-V` including:
  - Build date (YYYYMMDD)
  - Build number (increments daily)
  - Git commit hash
  - Git dirty status
- ✅ Provides compact help with `-h`
- ✅ Provides detailed help with `--help`
- ✅ Single source of truth for version numbers
- ✅ No manual version updates in code needed!

Just update `version = "x.y.z"` in Cargo.toml and rebuild!
