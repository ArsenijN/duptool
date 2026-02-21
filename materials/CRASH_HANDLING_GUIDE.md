# Crash Handling and DrKonqi Integration

## About DrKonqi

**DrKonqi** is KDE's crash handler that automatically:
- Detects application crashes
- Collects debug information
- Creates crash reports
- Offers to report bugs upstream

## Will DrKonqi Handle duptool Crashes?

**Short answer**: No, not automatically.

**Why**: DrKonqi is designed for **KDE/Qt applications** and looks for specific signals and metadata that Qt applications provide. Since duptool is a pure Rust CLI application without Qt/KDE integration, DrKonqi won't automatically handle its crashes.

## How Crash Handling Works on Linux

### 1. Default Linux Crash Handling

When duptool crashes, Linux will:
- Generate a core dump (if enabled)
- Log to syslog/journald
- Return a segfault/panic error to the shell

```bash
# Enable core dumps:
ulimit -c unlimited

# Run duptool:
./duptool -BDXE folder1 folder2

# If it crashes, core dump will be in current directory
# View crash with:
gdb ./target/release/duptool core
```

### 2. Rust Panic Handler

Rust has built-in panic handling that provides stack traces:

```rust
# Already enabled in duptool by default!
# When panic occurs, you'll see:

thread 'main' panicked at 'index out of bounds: the len is 3 but the index is 5', src/main.rs:123:5
stack backtrace:
   0: rust_begin_unwind
   1: core::panicking::panic_fmt
   2: core::panicking::panic_bounds_check
   3: duptool::calculate_file_hash
   ...
```

## Debugging duptool Crashes

### Method 1: Enable Backtraces (Easiest)

```bash
# Set environment variable for detailed backtraces:
export RUST_BACKTRACE=1

# Or even more detailed:
export RUST_BACKTRACE=full

# Run duptool:
./duptool -BDXE folder1 folder2

# If it crashes, you'll see full stack trace
```

### Method 2: Run with Debug Build

```bash
# Build in debug mode (has more symbols):
cargo build

# Run debug binary:
./target/debug/duptool -BDXE folder1 folder2

# Debug builds have:
# - Assertions enabled
# - Integer overflow checks
# - Better panic messages
# - Slower but safer
```

### Method 3: Use GDB (Most Powerful)

```bash
# Build with debug symbols:
cargo build --release

# Run in GDB:
gdb --args ./target/release/duptool -BDXE folder1 folder2

# GDB commands:
(gdb) run                    # Start program
(gdb) backtrace             # Show stack trace after crash
(gdb) info locals           # Show local variables
(gdb) frame 3               # Jump to specific stack frame
(gdb) print variable_name   # Inspect variable
```

### Method 4: Run with Valgrind (Memory Issues)

```bash
# Install valgrind:
sudo apt install valgrind

# Run with memory checking:
valgrind --leak-check=full ./target/release/duptool -BDXE folder1 folder2

# This will show:
# - Memory leaks
# - Invalid memory access
# - Use of uninitialized memory
```

### Method 5: Use strace (System Call Level)

```bash
# See exactly what system calls are happening:
strace -f -o trace.log ./target/release/duptool -BDXE folder1 folder2

# After crash, check trace.log for last system calls
tail -100 trace.log
```

## Adding Better Crash Reporting to duptool

### Option 1: Add human-readable panic handler

You can add this to main.rs:

```rust
use std::panic;

fn main() {
    // Set custom panic handler
    panic::set_hook(Box::new(|panic_info| {
        eprintln!("\n╔══════════════════════════════════════════╗");
        eprintln!("║  duptool crashed! 😢                    ║");
        eprintln!("╚══════════════════════════════════════════╝\n");
        
        if let Some(location) = panic_info.location() {
            eprintln!("Location: {}:{}", location.file(), location.line());
        }
        
        if let Some(msg) = panic_info.payload().downcast_ref::<&str>() {
            eprintln!("Reason: {}", msg);
        }
        
        eprintln!("\nPlease report this bug at:");
        eprintln!("https://github.com/ArsenijN/duptool/issues");
        eprintln!("\nInclude this information:");
        eprintln!("- duptool version: 0.1.10");
        eprintln!("- OS: {}", std::env::consts::OS);
        eprintln!("- Command used: {}", std::env::args().collect::<Vec<_>>().join(" "));
        
        // Print backtrace if enabled
        if std::env::var("RUST_BACKTRACE").is_ok() {
            eprintln!("\nBacktrace:");
            eprintln!("{:?}", std::backtrace::Backtrace::capture());
        } else {
            eprintln!("\nRun with RUST_BACKTRACE=1 for more details");
        }
    }));
    
    // Your normal main code here
    run().expect("duptool failed");
}

fn run() -> io::Result<()> {
    // Move all your current main() code here
    // ...
}
```

### Option 2: Add crash reporter dependency

Add to Cargo.toml:
```toml
[dependencies]
human-panic = "1.2"  # User-friendly panic messages
```

Then in main.rs:
```rust
fn main() {
    human_panic::setup_panic!();
    // rest of your code
}
```

This gives nice crash reports like:
```
Well, this is embarrassing.

duptool had a problem and crashed. To help us diagnose the problem
you can send us a crash report.

We have generated a report file at "/tmp/report-duptool.toml".
Submit an issue with the subject of "duptool Crash Report" and include
the report as an attachment.

- Homepage: https://github.com/ArsenijN/duptool
- Issues: https://github.com/ArsenijN/duptool/issues

We take privacy seriously, and do not perform any automated error
collection. In order to improve the software, we rely on people to
submit reports.

Thank you!
```

## Integrating with KDE/DrKonqi (Advanced)

If you really want DrKonqi integration, you would need to:

### 1. Add Qt/KDE bindings

```toml
[dependencies]
qmetaobject = "0.2"  # Qt bindings for Rust
```

### 2. Create Qt wrapper

Create a thin Qt wrapper that calls your Rust code:
```cpp
// duptool-qt.cpp
#include <QCoreApplication>
#include <QProcess>

extern "C" int run_duptool(int argc, char* argv[]);

int main(int argc, char* argv[]) {
    QCoreApplication app(argc, argv);
    app.setApplicationName("duptool");
    app.setOrganizationDomain("github.com/ArsenijN");
    
    // Now DrKonqi will recognize this as a KDE app
    return run_duptool(argc, argv);
}
```

### 3. Add .desktop file

```desktop
[Desktop Entry]
Type=Application
Name=duptool
Exec=duptool %F
Icon=duptool
Categories=Utility;FileTools;System;
```

**Note**: This is a lot of work for a CLI tool. Usually not worth it.

## Recommended Approach for duptool

For a CLI tool like duptool, the best approach is:

1. **Enable backtraces by default in release mode**:
   ```toml
   [profile.release]
   debug = true  # Include debug symbols
   ```

2. **Add user-friendly panic handler** (Option 1 above)

3. **Document debugging in README**:
   ```markdown
   ## If duptool crashes
   
   1. Run with RUST_BACKTRACE=1
   2. Report at: https://github.com/ArsenijN/duptool/issues
   3. Include: OS, command used, backtrace
   ```

4. **Use journalctl to check logs**:
   ```bash
   # Check if duptool logged anything:
   journalctl -e | grep duptool
   ```

## Automatic Crash Reporting (Optional)

If you want automatic crash reporting (like DrKonqi), you could integrate:

### Sentry (Popular choice)
```toml
[dependencies]
sentry = "0.31"
```

```rust
fn main() {
    let _guard = sentry::init(("your-dsn-here", sentry::ClientOptions {
        release: sentry::release_name!(),
        ..Default::default()
    }));
    
    // Your code
}
```

This automatically sends crash reports to Sentry dashboard.

**Privacy note**: Always inform users if you're sending crash reports!

## Summary

| Method | Automatic | Works Now | Effort | Best For |
|--------|-----------|-----------|---------|----------|
| RUST_BACKTRACE=1 | No | ✓ | None | Current debugging |
| GDB | No | ✓ | Low | Deep debugging |
| Custom panic handler | Yes | Add code | Low | Better UX |
| human-panic crate | Yes | Add dependency | Very Low | Quick improvement |
| Qt/DrKonqi wrapper | Yes | Major rewrite | Very High | KDE integration |
| Sentry | Yes | Add dependency | Medium | Production monitoring |

**Recommendation for duptool**: 
1. Add `human-panic` crate (5 minutes)
2. Enable debug symbols in release builds
3. Document in README

This gives 90% of DrKonqi's benefits without KDE dependency!
