# DEVLOG
I keep all changes here, so versions can be easily compared between themself

**Note:** Starting from v0.1.10, the full version history can be traced via git timeline in the main branch.

---

## Changes: V0.1.10
- **CRITICAL FIX for Linux:** Fixed file deletion/moving functionality on Linux systems. The previous version only created folder structure but didn't actually move files.
- **PERFORMANCE: Smart move strategy:** Now tries fast `rename()` first (atomic operation), only falls back to copy+remove when needed (cross-device moves). This makes same-filesystem moves nearly instantaneous instead of copying entire files.
- **Improved cross-device move handling:** Automatically detects cross-device errors (EXDEV on Unix) and seamlessly falls back to verified copy+remove.
- **CRITICAL SAFETY: Copy verification before deletion:** Added comprehensive verification that copy completed successfully before removing original file. Checks include:
  - Byte count verification (bytes_copied matches source file size)
  - Destination file existence and size verification
  - If any verification fails, destination file is cleaned up and original file is preserved
- **Guaranteed timestamp preservation:** Timestamps (atime/mtime) are now properly preserved with error reporting if preservation fails (non-fatal).
- **Guaranteed permission preservation:** File permissions are preserved with warning if it fails (non-fatal).
- **Better error handling:** Added verification that source file exists before attempting move, improved error messages, and more robust directory creation with proper error checking.
- **Enhanced debug output:** Added more helpful debug messages including suggestion to use `-F` flag when files don't exist in folder2, plus verification status messages and move method indicators.
- **Code refactoring:** Unified move logic for all platforms - try rename first, fall back to copy+remove on any error.
- **Version bump to 0.1.10.**

---

## Changes: V0.1.9
- **New `-b` / `--intra` flag:** Find duplicates within each folder (intra-folder), as well as between folders.
- **New `-1` / `--single` flag:** Find duplicates only within a single folder (no second folder required).
- **Default behavior:** Only finds duplicates between folders, not within, unless `-b` or `-1` is specified.
- **Version bump to 1.9.**

## Changes: V0.1.8
- **Progress bars improved:** Now both group and file progress are shown using two bars (via indicatif's MultiProgress).
- **ETA and speed reporting:** ETA and speed are updated more frequently and displayed for both progress bars.
- **Debug output:** More detailed debug output for file moving and error handling, especially in delete/force-delete modes.
- **Code structure:** Minor improvements to code structure and comments for clarity.
- **Version bump to 1.8.**

## Changes: V0.1.7
- **Quick check logic is now fully correct and explicit:**
  - If only `-C` is used (no `-A`/`-E`): only the first and last 8MB are compared, and results are based solely on this quick check (no full hash is performed).
  - If `-C` is used together with `-A` or `-E`: files are first filtered by quick check, then all candidates are fully hashed (full content hash).
  - If `-A`/`-E` are used without `-C`: all files are always fully hashed (no quick check).
- Results for only `-C` are based on quick check only, never full hash.
- Improved code comments and structure for clarity.
- Version bump to 1.7.

## Changes: V0.1.6
- Fixed logic for "-C" (quick check) with "-E"/"-A": now files are first filtered by 8MB chunk comparison, only those that pass are grouped for full hash comparison.
- After quick check, **full hash is always performed** for all files in candidate groups, regardless of `-C` flag, ensuring correct duplicate detection and consistent performance.
- Enhanced async mode and async mode now always perform full hash after quick check, not just quick hash.
- Fixed bug with recursive "deleted" folders when moving files.
- Improved ETA and speed calculation (updates every 0.5s or per file).
- Improved debug output and error handling.
- See TODO below for planned improvements.

## Notes on performance: Why is "-ABCEFX" much faster than "-ABEFX"?

- **"-ABCEFX"** uses the quick check (`-C`): it first compares only the first and last 8MB of each file. Only files that match these chunks are then fully hashed. For most files, this means a very fast comparison, since mismatches are detected early and full hashing is avoided for non-duplicates.
- **"-ABEFX"** does not use the quick check: it always reads and hashes the entire file content for every candidate, which is much slower, especially for large files.
- Even though both modes eventually do a full hash for files that pass the quick check, the quick check eliminates most non-duplicates early, so far fewer files need to be fully read and hashed.

**Summary:**  
- `-C` (quick check) acts as a fast filter, drastically reducing the number of full file reads/hashes.
- Without `-C`, every file in a candidate group is fully read and hashed, which is much slower for large files.

## Changes: V0.1.5
- Added "Force delete" by "-F" / "--force-delete". Moves all duplicates from the first folder to the "deleted" subfolder, ignoring the path in the second folder (unlike "-D" which checks for the same relative path in both folders).
- Enhanced async mode ("-E"/"--enhanced"): two threads, one for each folder, process files simultaneously.
- Improved debug output and error handling.
- Fixed usage of "-CA": now files compared fully instead of only first and last 8 MB like with regular "-C"
- Progression and regression in "-E" behaviour: previous version restored, behaviour satisfied
- Better ETA calculation: updates every 0.5 s or with every file

## Changes: V0.1.4
- Added long path and path sanitization support to avoid Windows path errors.
- Added debug mode ("-X"/"--debug") to log detailed information about file operations.
- Added option to move duplicates to "deleted" subfolder ("-D"/"--delete") only if the same relative path exists in both folders.
- Added HDD optimization/deoptimization flags ("-m"/"--hdd", "-M"/"--no-hdd").

## Changes: V0.1.3
- Improved ETA calculation for progress bar based on file size and processing speed.
- Added async mode ("-A"/"--async") for parallel checksum calculation.
- Added quick content comparison ("-C"/"--quick") using first and last 8MB of files.
- Added Everything integration flags for fast name/size comparison ("-N"/"--everything-name", "-S"/"--everything-size").

## Changes: V0.1.2
- Added bidirectional comparison ("-B"/"--bidirectional") to only compare files between folders, not within.
- Added support for comparing by file name ("-n"/"--name") and size ("-s"/"--size").
- Improved grouping and filtering logic for potential duplicates.

## Changes: V0.1.1
- Initial implementation: compare files by content (default), name, or size.
- Scans two folders and finds duplicate files.
- Outputs duplicate groups and summary statistics.

## TODO
- [ ] Make ETA even more accurate by smoothing speed over a moving window.
- [ ] Add option to show progress per file, not just per group. -- What about 2nd bar where it will show processed files instead groups?
- [ ] Add more flexible duplicate handling (e.g., interactive mode).
- [ ] Improve Unicode/path handling for edge cases.
- [ ] Add tests and CI.