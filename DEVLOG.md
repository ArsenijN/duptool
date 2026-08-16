# DEVLOG
I keep all changes here, so versions can be easily compared between themself

**Note:** Starting from v0.1.10, the older version's history can be traced via Git timeline in the `main` branch.

---

## Changes: V0.1.13
- **NEW: Stem comparison mode (`-k` / `--stem`):** Groups files by their filename stem — the portion before the last `.`. For example `photo.jpg` and `photo.png` both have stem `photo` and will be treated as candidates. Useful for finding renamed copies of the same file where only the format changed. Comparison is case-sensitive, consistent with the existing `-n` mode. Stacks on top of size grouping (same stem + same size), and can be freely combined with `-e`, `-n`, and `-c`.
- **NEW: Extension comparison mode (`-e` / `--ext`):** Groups files that share the same extension. Comparison is **case-insensitive** so `.JPG` and `.jpg` match. Files with no extension are grouped together under an empty key. Like `-k`, stacks on top of size grouping (same extension + same size). Combining `-k` and `-e` together behaves like `-n` — both the stem and the extension must match, which is equivalent to requiring the full filename.
- **FIX: Default-mode guard now recognises `-k` and `-e`:** Previously the guard that enables content comparison when no explicit mode is selected only checked `compare_content`, `compare_name`, and `compare_size`. Passing only `-k` or `-e` would silently enable content comparison on top of the requested filter. The condition now includes all five comparison flags.
- **FIX: `-C -Z` fuzzy candidates were always empty:** The fuzzy stage incorrectly derived its candidate pool from `quick_checked_groups` — the set of files that *passed* quick-check. Because every file in that set had already been promoted to an exact match, the `exact_paths` subtraction left nothing for fuzzy comparison. The real fuzzy candidates are files that *failed* quick-check (or failed the full hash), which are not in `quick_checked_groups` at all. Fix: a `saved_name_filtered` clone is taken before any branch consumes `name_filtered_groups`, and all three fuzzy candidate computation sites now subtract `exact_paths` from `saved_name_filtered` instead. This affects the `-C`-only early-return path, the `-C -A/-E` early-return path, and the main (full-hash) path.
- **FIX: `DuplicateGroup.size` was the total group size instead of a single file's size:** `sync_content_compare_with_file_progress` (the active sync path) and the dead-code `sync_content_compare` both computed `group_size` as `group.iter().map(|f| f.size).sum()`. Because groups are formed by size equality, all files in a group share the same size, so for a group of N files the sum is `N × file_size` instead of `file_size`. Since `display_results` then multiplies `duplicate.size × folder1_files.len()`, the reported "total duplicates" stat was inflated by a factor of N. Both functions now use `group.first().map_or(0, |f| f.size)`. The async path (`async_content_compare_with_file_progress`) already used `group[0].size` correctly and was unaffected.
- **FIX: Async mode with single-folder (`-1 -A` or `-1 -E`) found zero duplicates:** `async_content_compare_with_file_progress` contained an unconditional `!has_files_from_both_folders(&group)` guard. In intra-folder or single-folder mode every file carries `folder_index == 0`, so `has_files_from_both_folders` always returns `false` and every group was silently skipped. The guard is now `!options.intra_folder && !has_files_from_both_folders(&group)`.
- **Tests: Comprehensive `#[cfg(test)]` module added to `src/main.rs` (51 tests total):** Run with `cargo test`. Covers:
  - *Unit tests* for `group_by_name`, `group_by_stem`, `group_by_ext` (including case-insensitivity and compound extensions), `format_size`, `has_files_from_both_folders`, `sanitize_path`, `calculate_file_hash` (same content → same hash, different content → different hash, quick == full for small files), and `chunked_fuzzy_compare` (zero diff, one byte, many bytes, all bytes).
  - *Integration tests* for `find_duplicates` covering: exact content match, size mismatch early-exit, multiple groups, name mode (match and no-match), stem mode (match, no-match, combined with `-c`), extension mode (match, no-match, case-insensitivity, combined `-k -e`), intra-folder mode (match and no-match), fuzzy mode within/exceeding tolerance and the exact-vs-fuzzy classification.
  - *Regression tests* pinning all three bugs fixed in this version: the `group_size` sum bug, the `-C -Z` empty-candidate bug, and the async intra-folder skip bug. Each regression test is documented with a comment explaining the root cause and the fix.
- **Version bump to 0.1.13.**

---

## Changes: V0.1.12.1 (urgent patch):
- **FIX the code errors due to new sort feature, silently processed to the commit because of skipped `cargo clean` before build attempt**

---

## Changes: V0.1.12 (planned)
- **NEW: Fuzzy comparison mode (`-Z` / `--fuzzy`):** Detects near-duplicate files that differ only in embedded metadata (e.g. JPEG EXIF rotation tag, EXIF `ModifyDate`, GPS metadata stripped by Google Photos from video files). Uses a chunked byte-by-byte comparison that never loads full files into RAM — reads both files simultaneously in 1MB chunks, counting differing bytes across the whole file.
- **NEW: Byte tolerance threshold (`-t N` / `--tolerance N`):** Specifies how many bytes may differ for a pair to be considered a near-duplicate. Required when `-Z` is used. Always specified separately from combined flags (e.g. `-ABDEZ -t 50`), since `-t` takes a value. Percentage-based threshold planned as `-T` for a future version.
- **NEW: `differ/` folder:** Fuzzy-matched files from folder1 are moved to a `differ/` subfolder (distinct from `deleted/`), so the user can review them before any destructive action. Semantics: "probably the same content, but not byte-identical — inspect before deciding."
- **NEW: Fuzzy-as-dupes flag (`-U` / `--fuzzy-as-dupes`):** When specified alongside `-Z`, fuzzy matches are treated exactly like exact duplicates and become eligible for `-D`/`-F` deletion into `deleted/`. Requires `-Z`.
- **NEW: `MatchKind` enum:** All `DuplicateGroup` instances now carry a `MatchKind` tag (`Exact` or `Fuzzy { bytes_differing }`), enabling distinct display and routing logic.
- **DISPLAY: Match kind column:** Output table now includes a `match` column showing `EXACT` or `FUZZY ~NB` (where N is the byte diff count), and the summary line reports exact and fuzzy counts separately.
- **`-C` + `-Z` interaction:** When both flags are active, `-C` acts as a three-way gate — exact quick-hash matches go directly to the normal duplicate pipeline, hash mismatches (which would previously be discarded) become fuzzy candidates, size mismatches are still eliminated. No redundancy.
- **Planned (not yet implemented):** Format-aware metadata skip mode (Mode 2) — will understand JPEG EXIF blocks and MP4/MOV metadata atoms (including trailing atoms as used in Google Photos video exports) and hash only the payload. Percentage-based threshold (`-T`). Fuzzy as automatic fallback in the normal exact-match pipeline.
- **Old materials was removed, keeping the only those from the opened ToDo, access them via Git tree**
- **Version bump to 0.1.12.**

---

## Changes: V0.1.11rc1
- **PERFORMANCE FIX: Eliminated lock contention:** Fixed severe performance bottleneck in async mode where threads were waiting for locks on every file. Now uses local counters and updates shared state only every 10 files, dramatically reducing CPU wait time.
- **PERFORMANCE FIX: Increased buffer size:** Increased I/O buffer from 64KB to 1MB, reducing system calls by 16x. For a 100MB file: 100 read() calls instead of 1,600.
- **Version bump to 0.1.11.**

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
- **Version bump to 0.1.10.** Reason: since current state of the `duptool` is not an ready for production, and it still may have or had the problems

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
- [x] ~~Add option to show progress per file, not just per group.~~ — **Implemented in v0.1.8** (dual progress bars via `MultiProgress`)
- [ ] Add more flexible duplicate handling (e.g., interactive mode).
- [ ] Improve Unicode/path handling for edge cases.
- [x] ~~Add tests and CI.~~ — **Implemented in v0.1.13** (51-test `#[cfg(test)]` module; `cargo test`)