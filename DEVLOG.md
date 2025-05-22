# DEVLOG
I keep all changes here, so versions can be easily compared between themself

---

## Changes: V0.1.6
- Fixed logic for "-C" (quick check) with "-E"/"-A": now files are first filtered by 8MB chunk comparison, only those that pass are grouped for full hash comparison.
- This ensures that only files with matching quick content are processed in the next stage, as intended.
- ETA and speed calculation improved (from previous version).
- See TODO below for planned improvements.

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