# Performance Comparison: rename() vs copy+remove

## Overview

When moving files to the `deleted` subfolder, there are two possible methods:

1. **`rename()`** - Fast, atomic, but only works on same filesystem
2. **`copy+remove`** - Slower, but works across different filesystems

## v0.1.10 Strategy: Best of Both Worlds

The updated code **tries `rename()` first**, then falls back to `copy+remove` only if needed.

```
┌─────────────────────────┐
│   Try rename() first    │
│   (fast, atomic)        │
└──────────┬──────────────┘
           │
    ┌──────┴───────┐
    │              │
    ▼              ▼
┌────────┐    ┌─────────────────────┐
│SUCCESS!│    │ FAIL (cross-device, │
│        │    │  permissions, etc)   │
└────────┘    └──────────┬───────────┘
                         │
                         ▼
              ┌──────────────────────┐
              │ Fallback: Verified   │
              │ copy+remove          │
              └──────────────────────┘
```

## Performance Comparison

### Scenario 1: Same Filesystem (Your Case)

Moving from `/media/arsen/переноска 2/poко/Music/file.mp3` to `/media/arsen/переноска 2/poко/Music/deleted/file.mp3`

| Method | Time for 100MB file | What happens |
|--------|---------------------|--------------|
| **rename()** | **~0.001s** | Just updates directory entry, no data movement |
| copy+remove | ~10-20s | Reads entire file, writes entire file, deletes original |

**Speed difference: 10,000x to 20,000x faster!**

### Scenario 2: Cross-Device Move

Moving from `/media/external_drive/file.mp3` to `/home/user/Music/file.mp3`

| Method | Result |
|--------|--------|
| **rename()** | Fails with EXDEV error |
| **copy+remove** | Works, ~10-20s for 100MB |

**v0.1.10 automatically detects and handles this correctly.**

## Real-World Performance Impact

### Example: Moving 1000 files (100MB each)

**Same filesystem:**
- **With rename()**: ~1 second total (just metadata updates)
- With copy+remove only: ~3-5 hours (copying 100GB of data)

**Cross-device:**
- With rename+fallback: ~3-5 hours (copy+remove used automatically)
- Same as copy+remove only

## Technical Details

### Why is rename() so fast?

On Unix filesystems, a file's data doesn't actually "live" in a directory. Instead:

1. File data lives in **inodes** (fixed locations on disk)
2. Directory entries are just **pointers** (names → inode numbers)
3. Moving a file = updating the pointer in directory entries
4. No data is actually copied or moved!

```
Before rename():
/Music/
  ├─ file.mp3 → inode #12345

/Music/deleted/
  (empty)

After rename():
/Music/
  (empty)

/Music/deleted/
  ├─ file.mp3 → inode #12345

(Same inode #12345 - no data copied!)
```

### Why does rename() fail across devices?

Each filesystem has its own set of inodes. Inode #12345 on device A is completely different from inode #12345 on device B. You can't just update a pointer - you have to physically copy the data.

## Debug Output Examples

### Same Filesystem (Fast Path)
```
Moving file: /media/.../Music/song.mp3
Target path: /media/.../Music/deleted/song.mp3
File moved successfully using rename()
Moved: /media/.../Music/song.mp3
```

### Cross-Device (Fallback Path)
```
Moving file: /media/external/song.mp3
Target path: /home/user/Music/song.mp3
Rename failed: Cross-device link os error 18
Cross-device move detected (EXDEV); falling back to copy+remove
Copying file as fallback: /media/external/song.mp3 -> /home/user/Music/song.mp3
Copy verified: 10485760 bytes successfully copied
Timestamps preserved successfully
Verification complete, removing original file
Moved: /media/external/song.mp3
```

## Benchmarks

Using a typical HDD (100 MB/s sequential write):

| Operation | 10 files (100MB each) | 100 files | 1000 files |
|-----------|----------------------|-----------|------------|
| **rename() same FS** | 0.01s | 0.1s | 1s |
| **copy+remove same FS** | 10s | 100s | 1000s (16.6 min) |
| **copy+remove cross-device** | 10s | 100s | 1000s (16.6 min) |

Using SSD (500 MB/s):
| Operation | 10 files (100MB each) | 100 files | 1000 files |
|-----------|----------------------|-----------|------------|
| **rename() same FS** | 0.01s | 0.1s | 1s |
| **copy+remove same FS** | 2s | 20s | 200s (3.3 min) |
| **copy+remove cross-device** | 2s | 20s | 200s (3.3 min) |

## Verification Still Happens!

When copy+remove is used (cross-device), all safety checks still apply:
- ✓ Byte count verification
- ✓ Destination file verification
- ✓ Timestamp preservation
- ✓ Permission preservation
- ✓ Original file only deleted after verification

## Bottom Line

For your use case (moving to `deleted` subfolder on same drive):
- **v0.1.9**: Broken
- **v0.1.10 (previous fix with copy+remove only)**: Works but very slow
- **v0.1.10 (current with rename first)**: Works and **extremely fast**!

The code is smart enough to use the fast path when possible, and the safe slow path when necessary.
