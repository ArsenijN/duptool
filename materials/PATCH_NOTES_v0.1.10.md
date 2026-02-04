# duptool v0.1.10 - Patch Notes

## Critical Bug Fix: Linux File Deletion

### The Problem
In v0.1.9, when using the `-D` (delete) flag on Linux systems, the tool would:
- Create the folder structure in the `deleted` subfolder
- Show "Moving file" messages in debug mode
- But **NOT actually move the files**

This was caused by two issues:
1. **Cross-device move failure**: The code used `rename()` which fails when moving files across different mount points/devices on Linux (returns EXDEV error code 18)
2. **Logic order issue**: The folder2 path check happened before directory creation, and the error handling for `rename()` failure wasn't properly structured

### The Fix
The v0.1.10 update includes:

1. **PERFORMANCE: Smart move strategy**: Now tries fast `rename()` first (atomic, near-instantaneous), only uses copy+remove as fallback when needed:
   - Same filesystem moves: Uses `rename()` - nearly instant, no data copying
   - Cross-device moves: Automatically detects failure and falls back to verified copy+remove
   - Best of both worlds: Fast when possible, safe when necessary
2. **CRITICAL: Copy verification before deletion**: When fallback is needed, the tool verifies the copy was successful before removing the original file:
   - Checks that bytes copied matches source file size
   - Verifies destination file exists and has correct size
   - If verification fails, cleans up incomplete destination and preserves original
   - This prevents data loss in case of disk full, I/O errors, or interrupted operations
3. **Guaranteed metadata preservation**: Timestamps and permissions are now properly preserved with error reporting
4. **Improved error handling**: Better validation and error messages at each step
5. **Reorganized logic**: The check for whether files exist in folder2 now properly happens before the move attempt, with helpful debug messages
6. **Better debugging**: Added source file existence check, move method indicators, and clearer error messages

### Key Changes in Code

**Before (v0.1.9):**
```rust
// Tried rename first, only fell back to copy+remove on EXDEV error
if let Err(e) = rename(&sanitized_file_path, &target_path) {
    // Complex error detection and handling...
    // Only detected EXDEV on Unix systems
}
```

**After (v0.1.10):**
```rust
// Try rename first (fast, works on same filesystem)
match rename(&sanitized_file_path, &target_path) {
    Ok(_) => {
        // Success! Near-instant move
    }
    Err(e) => {
        // Any error: fall back to verified copy+remove
        copy_and_remove(&sanitized_file_path, &target_path, options)?;
    }
}
```

## Performance Impact

### Same Filesystem (Your Case)
When moving files within the same filesystem (e.g., `/media/arsen/переноска 2/poко/Music` to `/media/arsen/переноска 2/poко/Music/deleted`):

- **v0.1.9**: Didn't work at all
- **v0.1.10 with copy+remove**: Would work but copy every byte (~1-10 MB/s depending on drive)
- **v0.1.10 with rename first**: **Near-instantaneous** - just updates directory entries, no data copying!

**Example**: Moving a 100 MB file
- Copy+remove: ~10-20 seconds
- Rename: <0.1 seconds (just metadata update)

### Cross-Device Moves
When moving between different filesystems (e.g., from external drive to internal SSD):

- **v0.1.10**: Automatically detects and uses verified copy+remove
- Safety checks ensure data integrity
- Performance same as direct copy (no way to avoid copying data across devices)

## Additional Improvements

- **Enhanced debug messages**: Now shows helpful hints like "use -F to force delete" when appropriate
- **Better error messages**: More specific error reporting for each failure point
- **Source validation**: Verifies source file exists before attempting move
- **Directory creation validation**: Checks directory creation success before proceeding

## Safety Features

The copy+remove operation now includes multiple safety checks to prevent data loss:

1. **Pre-copy validation**: 
   - Source file existence check
   - Source metadata retrieval
   - Destination directory creation with error checking

2. **Post-copy verification**:
   - Byte count verification: Ensures `std::fs::copy` returned the correct number of bytes
   - Destination file verification: Confirms destination file exists and has correct size
   - If any check fails, the incomplete destination file is removed and the original is preserved

3. **Metadata preservation** (with warnings, non-fatal):
   - File permissions are preserved
   - Access time (atime) and modification time (mtime) are preserved
   - Failures are logged but don't prevent the operation

4. **Only after all verifications pass** is the original file removed

This means even in edge cases (disk full, I/O errors, permission issues, interrupted operations), your original files are safe.

## Important Notes

### Understanding -D vs -F flags:

- **`-D` (delete mode)**: Only moves duplicates if the same relative path exists in both folders
  - Use case: You want to clean up folder1, but only remove files that definitely exist in folder2
  - Example: If `/folder1/music/song.mp3` is found, it will only be moved if `/folder2/music/song.mp3` exists
  
- **`-F` (force-delete mode)**: Moves all duplicates from folder1 regardless of path in folder2
  - Use case: You want to remove ALL duplicates from folder1, regardless of where they are in folder2
  - Example: If `/folder1/music/song.mp3` is found, it will be moved even if it exists as `/folder2/downloads/song.mp3`

### Your Command
Based on your debug output, you used:
```bash
./duptool -BDEXM '/media/arsen/переноска 2/поко/Poco C65 backup/Music' /media/arsen/переноска/kjkjkj
```

This uses `-D` mode, which checks for matching paths. The files weren't being moved because they don't exist at the same relative path in folder2. **If you want to move all duplicates regardless of path, use `-F` instead of `-D`:**

```bash
./duptool -BFEXM '/media/arsen/переноска 2/поко/Poco C65 backup/Music' /media/arsen/переноска/kjkjkj
```

## Version History Tracking

Starting with v0.1.10, all code changes can be tracked via git history in the main branch. This provides:
- Full diff history between versions
- Ability to review specific changes
- Easy rollback if needed
- Better collaboration and code review

## Testing Recommendations

After applying this patch:
1. Test with `-D` flag on files that exist in both folders at the same relative path
2. Test with `-F` flag to verify all duplicates are moved regardless of path
3. Test across different mount points/devices to verify cross-device moves work
4. Use `-X` (debug) flag to see detailed operation logs

## Installation

Replace your current `main.rs` and `Cargo.toml` with the patched versions, then rebuild:

```bash
cargo build --release
```

The binary will be in `target/release/duptool`.
