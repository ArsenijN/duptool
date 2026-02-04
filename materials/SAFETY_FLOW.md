# Copy+Remove Safety Flow in duptool v0.1.10

## Operation Flow with Safety Checks

```
┌─────────────────────────────────────────────────────────────────┐
│                    START: Move File Request                      │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                           ▼
                  ┌────────────────┐
                  │ Source Exists? │
                  └────────┬───────┘
                           │ YES
                           ▼
              ┌────────────────────────┐
              │ Get Source Metadata    │
              │ - Size                 │
              │ - Permissions          │
              │ - Timestamps           │
              └────────┬───────────────┘
                       │
                       ▼
           ┌───────────────────────────┐
           │ Create Destination        │
           │ Directory Structure       │
           └───────────┬───────────────┘
                       │
                       ▼
              ┌────────────────────┐
              │ std::fs::copy()    │
              │ returns byte count │
              └────────┬───────────┘
                       │
                       ▼
        ╔══════════════════════════════════╗
        ║   CRITICAL VERIFICATION POINT    ║
        ╚══════════════════════════════════╝
                       │
        ┌──────────────┴──────────────┐
        │                             │
        ▼                             ▼
   ┌─────────┐                  ┌─────────┐
   │ bytes   │                  │ bytes   │
   │ copied  │                  │ copied  │
   │   ==    │                  │   !=    │
   │  size?  │                  │  size?  │
   └────┬────┘                  └────┬────┘
        │ YES                        │ NO
        │                            │
        │                            ▼
        │                   ┌─────────────────┐
        │                   │ ⚠️  DELETE      │
        │                   │ INCOMPLETE DEST │
        │                   │ PRESERVE SOURCE │
        │                   │ RETURN ERROR    │
        │                   └─────────────────┘
        │                            │
        │                            ▼
        │                         [ABORT]
        │
        ▼
   ┌─────────────────────────┐
   │ Verify Destination      │
   │ File Exists & Size OK?  │
   └────────┬────────────────┘
            │
     ┌──────┴──────┐
     │             │
     ▼             ▼
  ┌────┐       ┌────┐
  │YES │       │ NO │
  └─┬──┘       └─┬──┘
    │            │
    │            ▼
    │    ┌─────────────────┐
    │    │ ⚠️  DELETE      │
    │    │ INCOMPLETE DEST │
    │    │ PRESERVE SOURCE │
    │    │ RETURN ERROR    │
    │    └─────────────────┘
    │             │
    │             ▼
    │          [ABORT]
    │
    ▼
┌────────────────────────┐
│ Copy VERIFIED ✓        │
└────────┬───────────────┘
         │
         ▼
┌─────────────────────────┐
│ Preserve Permissions    │
│ (warn if fails)         │
└────────┬────────────────┘
         │
         ▼
┌─────────────────────────┐
│ Preserve Timestamps     │
│ - atime                 │
│ - mtime                 │
│ (warn if fails)         │
└────────┬────────────────┘
         │
         ▼
╔════════════════════════════╗
║   SAFE TO DELETE SOURCE    ║
╚════════════════════════════╝
         │
         ▼
┌─────────────────────────┐
│ std::fs::remove_file()  │
│ Delete Original         │
└────────┬────────────────┘
         │
         ▼
    ┌────────┐
    │ SUCCESS│
    └────────┘
```

## Key Safety Points

### 1. **Pre-Flight Checks**
- Source file must exist
- Source metadata must be readable
- Destination directory must be created successfully

### 2. **Copy Verification (CRITICAL)**
Two-stage verification process:
1. **Immediate**: Check `std::fs::copy` return value matches source size
2. **Post-copy**: Read destination file metadata and verify size matches

### 3. **Failure Handling**
If ANY verification fails:
- ❌ Do NOT delete source file
- 🗑️ Clean up incomplete destination file  
- 📢 Report detailed error message
- ✅ Original file remains safe

### 4. **Metadata Preservation**
- Permissions copied with warning on failure
- Timestamps (atime/mtime) preserved with warning on failure
- Failures are non-fatal (operation continues)

### 5. **Point of No Return**
Original file is ONLY deleted after:
- ✓ Copy completed successfully
- ✓ Byte count verified
- ✓ Destination file verified
- ✓ Metadata preserved (or attempted)

## Example Scenarios

### Scenario 1: Disk Full During Copy
```
1. Copy starts
2. Disk runs out of space mid-copy
3. std::fs::copy returns Err() or incomplete byte count
4. Verification FAILS
5. Incomplete destination deleted
6. Source file PRESERVED ✓
7. User gets error message
```

### Scenario 2: Interrupted Operation (Ctrl+C during copy)
```
1. Copy in progress
2. User presses Ctrl+C
3. Process terminates
4. Source file still exists ✓
5. Incomplete destination file may remain (user can clean up)
6. No data loss ✓
```

### Scenario 3: I/O Error Reading Destination
```
1. Copy completes successfully
2. Attempting to verify destination
3. Cannot read destination metadata (I/O error)
4. Verification FAILS
5. Source file PRESERVED ✓
6. User gets error message
```

### Scenario 4: All Checks Pass
```
1. Copy completes: 10,485,760 bytes
2. Source size: 10,485,760 bytes ✓
3. Destination exists and size matches ✓
4. Permissions preserved ✓
5. Timestamps preserved ✓
6. Source file deleted safely ✓
7. Success!
```

## Code Snippet: The Critical Section

```rust
// Get source size BEFORE copy
let src_metadata = src.metadata()?;
let src_size = src_metadata.len();

// Perform copy
let bytes_copied = std::fs::copy(src, dst)?;

// VERIFY: Did copy return correct byte count?
if bytes_copied != src_size {
    // FAILURE: Clean up and preserve source
    let _ = std::fs::remove_file(dst);
    return Err(...); // Source remains untouched!
}

// VERIFY: Does destination file exist and match size?
match dst.metadata() {
    Ok(dst_metadata) => {
        if dst_metadata.len() != src_size {
            // FAILURE: Clean up and preserve source
            let _ = std::fs::remove_file(dst);
            return Err(...); // Source remains untouched!
        }
    }
    Err(e) => {
        // FAILURE: Can't verify, preserve source
        return Err(...); // Source remains untouched!
    }
}

// All checks passed - NOW it's safe to delete source
std::fs::remove_file(src)?;
```

## Why This Matters

Traditional `rename()` operations are atomic - either they succeed completely or fail completely. When we use `copy+remove` as a fallback for cross-device moves, we lose this atomicity. The verification logic restores safety by ensuring we never remove the source until we're 100% certain the copy succeeded.

This is especially critical for:
- **Large files**: More opportunity for errors during copy
- **Network filesystems**: Higher chance of I/O errors
- **Low disk space situations**: Copy may fail mid-operation
- **Unreliable hardware**: External drives, USB sticks, network storage
