use clap::{Arg, ArgAction, Command};
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};

/// Prints a line in normal runs; silenced completely when running under
/// `cargo test` so test output stays clean and progress-bar-free.
macro_rules! vprintln {
    ($($arg:tt)*) => {
        if !cfg!(test) { println!($($arg)*); }
    };
}
use md5::Context; // Use Context instead of Md5
use std::collections::{HashMap, HashSet};
use std::fs::{File, create_dir_all, rename};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self};
use std::time::Instant;
use walkdir::WalkDir;

const QUICKCHECK_SIZE: usize = 8 * 1024 * 1024; // 8MB

#[derive(Debug, Clone)]
struct FileInfo {
    path: PathBuf,
    size: u64,
    folder_index: usize,
}

#[derive(Debug, Clone)]
struct CompareOptions {
    compare_content: bool,
    compare_name: bool,
    compare_size: bool,
    quick_content_check: bool,
    #[allow(dead_code)]  // Reserved for future Everything integration
    everything_name: bool,
    #[allow(dead_code)]  // Reserved for future Everything integration
    everything_size: bool,
    bidirectional: bool,
    async_compare: bool,
    hdd_optimized: bool,
    delete_duplicates: bool,
    debug: bool,
    enhanced_async: bool,
    force_delete: bool,
    intra_folder: bool,
    // Fuzzy comparison options
    fuzzy: bool,           // -Z: enable fuzzy mode
    tolerance: u64,        // -t N: max differing bytes allowed
    fuzzy_as_dupes: bool,  // -U: treat fuzzy matches as exact duplicates
    // Partial name comparison options
    compare_stem: bool,    // -k: compare by file stem (filename without extension)
    compare_ext: bool,     // -e: compare by file extension only
}

/// How a duplicate group was matched.
#[derive(Debug, Clone, PartialEq)]
enum MatchKind {
    Exact,
    Fuzzy { bytes_differing: u64 },
}

#[derive(Debug)]
struct DuplicateGroup {
    files_by_folder: Vec<Vec<PathBuf>>,
    size: u64,
    match_kind: MatchKind,
}

fn main() -> io::Result<()> {
    let start_time = Instant::now();
    
    // Parse command line arguments
    let matches = Command::new("duptool")
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("Advanced duplicate file finder with smart comparison strategies")
        .long_about(concat!(
            "duptool - Advanced duplicate file finder\n\n",
            "Fast, efficient tool for finding duplicate files across directories.\n",
            "Features: async processing, HDD optimization, quick content check,\n",
            "safe file deletion, and cross-device move support.\n\n",
            "Version: ", env!("CARGO_PKG_VERSION"), "\n",
            "Author: ", env!("CARGO_PKG_AUTHORS"), "\n",
            "Repository: ", env!("CARGO_PKG_REPOSITORY")
        ))
        .after_help("Use -h for compact help, --help for detailed help.\nUse -v for version, -V for detailed version.")
        .disable_version_flag(true)  // Disable default -V so we can customize
        .arg(
            Arg::new("version")
                .short('v')
                .long("version")
                .help("Print version")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("verbose_version")
                .short('V')
                .long("verbose-version")
                .help("Print detailed version information")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("folder1")
                .required(true)
                .help("First folder to compare (or the only folder in --single mode)")
                .index(1),
        )
        .arg(
            Arg::new("folder2")
                .required(false)
                .help("Second folder to compare (omit for --single mode)")
                .index(2),
        )
        .arg(
            Arg::new("content")
                .short('c')
                .long("content")
                .help("Compare by file content")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("name")
                .short('n')
                .long("name")
                .help("Compare by file name")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("size")
                .short('s')
                .long("size")
                .help("Compare by file size")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("quick")
                .short('C')
                .long("quick")
                .help("Quick content comparison (first and last 8MB)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("everything_name")
                .short('N')
                .long("everything-name")
                .help("Fast name comparison using Everything")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("everything_size")
                .short('S')
                .long("everything-size")
                .help("Fast size comparison using Everything")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("bidirectional")
                .short('B')
                .long("bidirectional")
                .help("Only compare files between folders, not within")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("intra")
                .short('b')
                .long("intra")
                .help("Also find duplicates within each folder (intra-folder search)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("async")
                .short('A')
                .long("async")
                .help("Use asynchronous comparison with checksums")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("hdd_optimized")
                .short('m')
                .long("hdd")
                .help("Optimize for HDD usage (default)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("hdd_deoptimized")
                .short('M')
                .long("no-hdd")
                .help("Deoptimize for HDD usage (multithreading)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("delete_duplicates")
                .short('D')
                .long("delete")
                .help("Move duplicates from the first folder to a 'deleted' subfolder")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("debug")
                .short('X')
                .long("debug")
                .help("Enable debug mode to log detailed information")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("enhanced_async")
                .short('E')
                .long("enhanced")
                .help("Enable enhanced async mode (process files from both folders simultaneously)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("force_delete")
                .short('F')
                .long("force-delete")
                .help("Move all duplicates from the first folder to 'deleted' subfolder, ignoring path in second folder")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("single")
                .short('1')
                .long("single")
                .help("Find duplicates only within the first folder (single folder mode)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("fuzzy")
                .short('Z')
                .long("fuzzy")
                .help("Enable fuzzy comparison: files within --tolerance bytes are near-duplicates, moved to 'differ/' folder")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("tolerance")
                .short('t')
                .long("tolerance")
                .help("Byte threshold for fuzzy comparison (required with -Z, always specify separately: -Z -t 50)")
                .value_name("BYTES")
                .value_parser(clap::value_parser!(u64)),
        )
        .arg(
            Arg::new("fuzzy_as_dupes")
                .short('U')
                .long("fuzzy-as-dupes")
                .help("Treat fuzzy matches as exact duplicates (requires -Z); eligible for -D/-F deletion")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("stem")
                .short('k')
                .long("stem")
                .help("Compare by file stem (filename without extension, e.g. 'photo' from 'photo.jpg')")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("ext")
                .short('e')
                .long("ext")
                .help("Compare by file extension only (e.g. '.jpg'); combined with size grouping")
                .action(ArgAction::SetTrue),
        )
        .get_matches();

    // Handle version flags
    if matches.get_flag("version") {
        println!("duptool {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    
    if matches.get_flag("verbose_version") {
        println!("duptool {}", option_env!("FULL_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")));
        println!("Build date: {}", option_env!("BUILD_DATE").unwrap_or("unknown"));
        println!("Build number: {}", option_env!("BUILD_NUMBER").unwrap_or("0"));
        println!("Git commit: {}", option_env!("GIT_HASH").unwrap_or("unknown"));
        println!();
        println!("Author: {}", env!("CARGO_PKG_AUTHORS"));
        println!("Repository: {}", env!("CARGO_PKG_REPOSITORY"));
        println!("License: MIT");
        return Ok(());
    }

    let single_mode = matches.get_flag("single");
    let folder1 = matches.get_one::<String>("folder1").unwrap();
    let folder2 = if single_mode {
        "" // Not used in single mode
    } else {
        matches.get_one::<String>("folder2").map(|s| s.as_str()).unwrap_or("")
    };

    // Validate fuzzy flags
    let fuzzy = matches.get_flag("fuzzy");
    let tolerance = matches.get_one::<u64>("tolerance").copied();
    let fuzzy_as_dupes = matches.get_flag("fuzzy_as_dupes");

    if fuzzy && tolerance.is_none() {
        eprintln!("Error: -Z/--fuzzy requires -t/--tolerance to be specified.");
        eprintln!("Example: duptool folder1 folder2 -Z -t 50");
        std::process::exit(1);
    }
    if !fuzzy && tolerance.is_some() {
        eprintln!("Warning: -t/--tolerance has no effect without -Z/--fuzzy.");
    }
    if fuzzy_as_dupes && !fuzzy {
        eprintln!("Error: -U/--fuzzy-as-dupes requires -Z/--fuzzy.");
        std::process::exit(1);
    }

    // Define comparison options
    let mut options = CompareOptions {
        compare_content: matches.get_flag("content"),
        compare_name: matches.get_flag("name"),
        compare_size: matches.get_flag("size"),
        quick_content_check: matches.get_flag("quick"),
        everything_name: matches.get_flag("everything_name"),
        everything_size: matches.get_flag("everything_size"),
        bidirectional: matches.get_flag("bidirectional"),
        async_compare: matches.get_flag("async"),
        hdd_optimized: !matches.get_flag("hdd_deoptimized"),
        delete_duplicates: matches.get_flag("delete_duplicates"),
        debug: matches.get_flag("debug"),
        enhanced_async: matches.get_flag("enhanced_async"),
        force_delete: matches.get_flag("force_delete"),
        intra_folder: matches.get_flag("intra") || single_mode,
        fuzzy,
        tolerance: tolerance.unwrap_or(0),
        fuzzy_as_dupes,
        compare_stem: matches.get_flag("stem"),
        compare_ext: matches.get_flag("ext"),
    };

    if options.debug {
        println!("Debug mode enabled");
        println!("Options: {:?}", options);
        println!("Folder1: {}", folder1);
        if !single_mode {
            println!("Folder2: {}", folder2);
        }
    }

    // Default behavior: if none of the main comparison options are selected, use content comparison
    if !options.compare_content && !options.compare_name && !options.compare_size
        && !options.compare_stem && !options.compare_ext
    {
        options.compare_content = true;
        options.compare_size = true;  // content comparison implies size comparison
    }
    
    // If content is selected, size is implied
    if options.compare_content {
        options.compare_size = true;
    }

    println!("Scanning directories...");
    let folder1_files = collect_files(folder1, 0, options.hdd_optimized)?;
    let folder2_files = if single_mode {
        Vec::new()
    } else {
        collect_files(folder2, 1, options.hdd_optimized)?
    };

    println!("Found {} files in {}", folder1_files.len(), folder1);
    if !single_mode {
        println!("Found {} files in {}", folder2_files.len(), folder2);
    }

    let duplicates = find_duplicates(folder1_files, folder2_files, folder1, folder2, &options)?;

    // Separate exact and fuzzy match groups
    let (exact_groups, fuzzy_groups): (Vec<_>, Vec<_>) = duplicates
        .into_iter()
        .partition(|g| g.match_kind == MatchKind::Exact);

    // Handle fuzzy groups: move to 'differ/' unless -U (treat as dupes)
    if !fuzzy_groups.is_empty() {
        if options.fuzzy_as_dupes {
            // Treat as exact duplicates — fall through to normal delete logic below
            // by merging back; handled after this block
        } else {
            // Default fuzzy behaviour: move to 'differ/' folder (report-only if no -D/-F)
            if options.delete_duplicates || options.force_delete {
                move_fuzzy_to_differ(&fuzzy_groups, folder1, folder2, &options)?;
            }
        }
    }

    // Combine for deletion: exact always eligible, fuzzy only if -U
    let mut groups_for_deletion: Vec<&DuplicateGroup> = exact_groups.iter().collect();
    if options.fuzzy_as_dupes {
        groups_for_deletion.extend(fuzzy_groups.iter());
    }

    if (options.delete_duplicates || options.force_delete) && !groups_for_deletion.is_empty() {
        if !single_mode {
            move_duplicates_to_deleted(
                &groups_for_deletion.iter().map(|g| DuplicateGroup {
                    files_by_folder: g.files_by_folder.clone(),
                    size: g.size,
                    match_kind: g.match_kind.clone(),
                }).collect::<Vec<_>>(),
                folder1, folder2, &options
            )?;
        } else {
            move_duplicates_to_deleted(
                &groups_for_deletion.iter().map(|g| DuplicateGroup {
                    files_by_folder: g.files_by_folder.clone(),
                    size: g.size,
                    match_kind: g.match_kind.clone(),
                }).collect::<Vec<_>>(),
                folder1, "", &options
            )?;
        }
    }

    // Merge all groups for display
    let all_groups: Vec<DuplicateGroup> = exact_groups.into_iter()
        .chain(fuzzy_groups.into_iter())
        .collect();

    display_results(&all_groups, folder1, if single_mode { "" } else { folder2 });
    
    println!("Completed in {:.2} seconds", start_time.elapsed().as_secs_f32());
    
    Ok(())
}

fn collect_files(root: &str, folder_index: usize, hdd_optimized: bool) -> io::Result<Vec<FileInfo>> {
    let mut files = Vec::new();

    // Hidden in test mode so cargo test output stays clean.
    let progress = if cfg!(test) {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_message(format!("Scanning {}...", root));
        let style = ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} ({pos} files found)")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        pb.set_style(style);
        pb
    };
    
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(metadata) = entry.metadata() {
                files.push(FileInfo {
                    path: entry.path().to_path_buf(),
                    size: metadata.len(),
                    folder_index,
                });
                progress.inc(1);
            }
        }
    }

    // Sort files by path to improve sequential access on HDDs if hdd_optimized is true
    if hdd_optimized {
        files.sort_by(|a, b| a.path.cmp(&b.path));
    }

    progress.finish_with_message(format!("Scanned {} files in {}", files.len(), root));
    Ok(files)
}

fn find_duplicates(
    folder1_files: Vec<FileInfo>, 
    folder2_files: Vec<FileInfo>, 
    folder1: &str,
    folder2: &str,
    options: &CompareOptions
) -> io::Result<Vec<DuplicateGroup>> {
    vprintln!("Comparing files...");
    
    // Group files by size as a first pass
    let mut size_groups: HashMap<u64, Vec<FileInfo>> = HashMap::new();
    for file in folder1_files.clone() {
        size_groups.entry(file.size).or_default().push(file);
    }
    for file in folder2_files.clone() {
        size_groups.entry(file.size).or_default().push(file);
    }

    // Only keep size groups with potential duplicates
    let mut potential_duplicates: Vec<Vec<FileInfo>> = Vec::new();
    for group in size_groups.values() {
        if group.len() > 1 {
            potential_duplicates.push(group.clone());
        }
    }

    // If not intra_folder and not bidirectional, filter out groups that are only within one folder
    let potential_duplicates: Vec<Vec<FileInfo>> = if options.intra_folder {
        // Keep all groups with more than one file (intra-folder and inter-folder)
        potential_duplicates
    } else if options.bidirectional {
        // Only groups with files from both folders (inter-folder only)
        potential_duplicates
            .into_iter()
            .filter(|group| has_files_from_both_folders(group))
            .collect()
    } else {
        // Default: only inter-folder duplicates
        potential_duplicates
            .into_iter()
            .filter(|group| has_files_from_both_folders(group))
            .collect()
    };

    vprintln!("Found {} potential duplicate groups by size", potential_duplicates.len());
    
    // Next, apply name comparison if needed
    let mut name_filtered_groups = Vec::new();

    if options.compare_name {
        for group in potential_duplicates {
            let name_groups = group_by_name(&group);
            for name_group in name_groups {
                if name_group.len() > 1 && 
                   (!options.bidirectional || has_files_from_both_folders(&name_group)) {
                    name_filtered_groups.push(name_group);
                }
            }
        }
        vprintln!("Found {} potential duplicate groups by name", name_filtered_groups.len());
    } else {
        name_filtered_groups = potential_duplicates;
    }

    // Apply stem filter (-k): further narrow groups by filename-without-extension.
    // e.g. "photo.jpg" and "photo.png" share stem "photo".
    let name_filtered_groups = if options.compare_stem {
        let mut stem_filtered = Vec::new();
        for group in name_filtered_groups {
            for sg in group_by_stem(&group) {
                if sg.len() > 1 && (!options.bidirectional || has_files_from_both_folders(&sg)) {
                    stem_filtered.push(sg);
                }
            }
        }
        vprintln!("Found {} potential duplicate groups by stem", stem_filtered.len());
        stem_filtered
    } else {
        name_filtered_groups
    };

    // Apply extension filter (-e): further narrow groups by file extension.
    // Comparison is case-insensitive (.JPG == .jpg). Files with no extension share "".
    let name_filtered_groups = if options.compare_ext {
        let mut ext_filtered = Vec::new();
        for group in name_filtered_groups {
            for eg in group_by_ext(&group) {
                if eg.len() > 1 && (!options.bidirectional || has_files_from_both_folders(&eg)) {
                    ext_filtered.push(eg);
                }
            }
        }
        vprintln!("Found {} potential duplicate groups by extension", ext_filtered.len());
        ext_filtered
    } else {
        name_filtered_groups
    };

    // -D pre-filter: with delete_duplicates (but not force_delete) we only care about
    // files that have a counterpart at the *same relative path* in folder2.  Any size
    // group that can never produce such a pair is useless to compare — drop it early.
    let name_filtered_groups = if options.delete_duplicates && !options.force_delete && !folder2.is_empty() {
        let folder2_sanitized = sanitize_path(folder2);
        name_filtered_groups
            .into_iter()
            .filter(|group| {
                group.iter().any(|f| {
                    if f.folder_index != 0 { return false; }
                    let sanitized = sanitize_path(&f.path);
                    if let Ok(relative) = sanitized.strip_prefix(sanitize_path(folder1)) {
                        sanitize_path(&folder2_sanitized.join(relative)).exists()
                    } else {
                        false
                    }
                })
            })
            .collect()
    } else {
        name_filtered_groups
    };

    // --- CORRECT QUICK CHECK LOGIC ---
    // - If only -C: compare only first and last 8MB, never full hash.
    // - If -C and -A/-E: first filter by quick check, then do full hash for those that match.
    // - If -A/-E without -C: always do full hash.
    // - If neither: always do full hash (sync).
    //
    // BUG FIX: save name_filtered_groups here so the fuzzy stage can correctly
    // derive candidates as "all files not already in exact matches".  Without this,
    // the fuzzy stage was rebuilding candidates from quick_checked_groups (files that
    // *passed* quick check), which is wrong when -C is active — the true fuzzy
    // candidates are the files that *failed* quick check (or failed full hash).
    let saved_name_filtered = name_filtered_groups.clone();

    let mut quick_checked_groups = name_filtered_groups.clone();

    if options.quick_content_check && !(options.async_compare || options.enhanced_async) {
        // Only -C: compare only first and last 8MB, never full hash

        // --- Progress bar for quick check only mode ---
        let total_groups = name_filtered_groups.len();
        let total_files: u64 = name_filtered_groups.iter().map(|g| g.len() as u64).sum();

        let m = MultiProgress::new();

        // Group progress bar — hidden when running under cargo test.
        let progress_bar = if cfg!(test) {
            m.add(ProgressBar::hidden())
        } else {
            let pb = m.add(ProgressBar::new(total_groups as u64));
            pb.set_style(ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} groups ({eta})")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);
            pb
        };

        // File progress bar — hidden when running under cargo test.
        let file_progress = if cfg!(test) {
            m.add(ProgressBar::hidden())
        } else {
            let fp = m.add(ProgressBar::new(total_files));
            fp.set_style(ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.green/white} {pos}/{len} files ({eta})")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);
            fp
        };

        let mut processed_files = 0u64;

        quick_checked_groups = name_filtered_groups
            .into_iter()
            .enumerate()
            .flat_map(|(_idx, group)| {
                progress_bar.inc(1);
                let mut quick_hash_map: HashMap<String, Vec<FileInfo>> = HashMap::new();
                for file in &group {
                    if let Some(h) = calculate_file_hash(file, true).ok().flatten() {
                        quick_hash_map.entry(h).or_default().push(file.clone());
                    }
                    processed_files += 1;
                    file_progress.inc(1);
                }
                quick_hash_map
                    .into_values()
                    .filter(|files| files.len() > 1 && has_files_from_both_folders(files))
                    .collect::<Vec<Vec<FileInfo>>>()
            })
            .collect();
        progress_bar.finish();
        file_progress.finish();
        // In production this prints above the finished bars; silenced in tests.
        m.suspend(|| vprintln!("After quick check: {} groups remain", quick_checked_groups.len()));

        // Output results based on quick check only, no full hash
        let mut duplicates = Vec::new();
        for group in &quick_checked_groups {
            if !group.is_empty() {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];
                let size = group[0].size;
                for file in group {
                    files_by_folder[file.folder_index].push(file.path.clone());
                }
                duplicates.push(DuplicateGroup { files_by_folder, size, match_kind: MatchKind::Exact });
            }
        }

        // Fuzzy stage: files that passed quick-check became exact matches above.
        // The true fuzzy candidates are those that *failed* quick-check, i.e. files
        // present in saved_name_filtered but absent from the exact duplicates set.
        if options.fuzzy {
            let exact_paths: HashSet<PathBuf> = duplicates
                .iter()
                .flat_map(|g| g.files_by_folder.iter().flat_map(|v| v.iter().cloned()))
                .collect();
            // Use saved_name_filtered (all pre-quick-check candidates) and subtract
            // the files that already became exact matches.
            let fuzzy_candidates: Vec<Vec<FileInfo>> = saved_name_filtered
                .iter()
                .map(|group| {
                    group.iter()
                        .filter(|f| !exact_paths.contains(&f.path))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .filter(|g| g.len() > 1 && has_files_from_both_folders(g))
                .collect();
            if !fuzzy_candidates.is_empty() {
                vprintln!("Running fuzzy comparison on {} candidate groups...", fuzzy_candidates.len());
                let fuzzy_results = fuzzy_compare_groups(&fuzzy_candidates, options)?;
                vprintln!("Fuzzy comparison found {} near-duplicate groups", fuzzy_results.len());
                duplicates.extend(fuzzy_results);
            }
        }

        return Ok(duplicates);
    }

    // If -C and -A/-E: filter by quick check, then do full hash for those that match
    if options.quick_content_check && (options.async_compare || options.enhanced_async) {
        quick_checked_groups = name_filtered_groups
            .clone()
            .into_iter()
            .flat_map(|group| {
                let mut quick_hash_map: HashMap<String, Vec<FileInfo>> = HashMap::new();
                for file in &group {
                    if let Some(h) = calculate_file_hash(file, true).ok().flatten() {
                        quick_hash_map.entry(h).or_default().push(file.clone());
                    }
                }
                quick_hash_map
                    .into_values()
                    .filter(|files| files.len() > 1 && has_files_from_both_folders(files))
                    .collect::<Vec<Vec<FileInfo>>>()
            })
            .collect();
        vprintln!("After quick check: {} groups remain", quick_checked_groups.len());
    }

    // --- FULL HASH FOR ALL FILES IN GROUPS ---
    let mut duplicates = Vec::new();

    if options.compare_content {
        let total_groups = quick_checked_groups.len();
        let total_files: u64 = quick_checked_groups.iter().map(|g| g.len() as u64).sum();

        // --- Use MultiProgress for two progress bars ---
        let m = MultiProgress::new();

        // Group progress bar (per group) — hidden when running under cargo test.
        let progress_bar = if cfg!(test) {
            m.add(ProgressBar::hidden())
        } else {
            let pb = m.add(ProgressBar::new(total_groups as u64));
            pb.set_style(ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} groups ({eta})")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);
            pb
        };

        // File progress bar (per file) — hidden when running under cargo test.
        let file_progress = if cfg!(test) {
            m.add(ProgressBar::hidden())
        } else {
            let fp = m.add(ProgressBar::new(total_files));
            fp.set_style(ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.green/white} {pos}/{len} files ({eta})")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);
            fp
        };

        // Clone before moving into closure so the fuzzy stage can still use
        // quick_checked_groups afterwards.
        let groups_for_thread = quick_checked_groups.clone();

        // Run the comparison in a closure so we can join the progress bars after
        let result = if options.async_compare || options.enhanced_async {
            let mut full_hash_options = options.clone();
            full_hash_options.quick_content_check = false;
            let progress_bar = progress_bar.clone();
            let file_progress = file_progress.clone();
            std::thread::spawn(move || {
                async_content_compare_with_file_progress(
                    groups_for_thread, full_hash_options, progress_bar, file_progress
                )
            })
        } else {
            let mut full_hash_options = options.clone();
            full_hash_options.quick_content_check = false;
            let progress_bar = progress_bar.clone();
            let file_progress = file_progress.clone();
            std::thread::spawn(move || {
                sync_content_compare_with_file_progress(
                    groups_for_thread, &full_hash_options, progress_bar, file_progress
                )
            })
        };

        // Wait for both progress bars to finish
        // Fallback for indicatif versions without join_and_clear/join:
        duplicates = result.join().unwrap()?;
        m.clear().unwrap();
    } else if options.quick_content_check && (options.async_compare || options.enhanced_async) && !options.compare_content {
        // Show a progress bar for the async quick check (when -C and -A/-E, but not --content)
        let total_groups = quick_checked_groups.len();
        let total_files: u64 = quick_checked_groups.iter().map(|g| g.len() as u64).sum();

        let m = MultiProgress::new();
        let group_bar = if cfg!(test) {
            m.add(ProgressBar::hidden())
        } else {
            let pb = m.add(ProgressBar::new(total_groups as u64));
            pb.set_style(ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.yellow/blue} {pos}/{len} groups (finalizing) {eta}")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);
            pb
        };

        let file_bar = if cfg!(test) {
            m.add(ProgressBar::hidden())
        } else {
            let fb = m.add(ProgressBar::new(total_files));
            fb.set_style(ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.green/white} {pos}/{len} files (finalizing) {eta}")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);
            fb
        };

        let mut final_duplicates = Vec::new();

        for group in &quick_checked_groups {
            group_bar.inc(1);
            for _ in group {
                file_bar.inc(1);
            }
            if !group.is_empty() {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];
                let size = group[0].size;
                for file in group {
                    files_by_folder[file.folder_index].push(file.path.clone());
                }
                final_duplicates.push(DuplicateGroup { files_by_folder, size, match_kind: MatchKind::Exact });
            }
        }
        group_bar.finish();
        file_bar.finish();
        m.clear().unwrap();
        // Fuzzy on remaining candidates before the early return.
        // Use saved_name_filtered so files that failed quick-check are included
        // as fuzzy candidates (not just files that passed quick-check but differ
        // on full hash, which is the incorrect set that quick_checked_groups gives).
        if options.fuzzy {
            let exact_paths: HashSet<PathBuf> = final_duplicates
                .iter()
                .flat_map(|g| g.files_by_folder.iter().flat_map(|v| v.iter().cloned()))
                .collect();
            let fuzzy_candidates: Vec<Vec<FileInfo>> = saved_name_filtered
                .iter()
                .map(|group| {
                    group.iter()
                        .filter(|f| !exact_paths.contains(&f.path))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .filter(|g| g.len() > 1 && has_files_from_both_folders(g))
                .collect();
            if !fuzzy_candidates.is_empty() {
                vprintln!("Running fuzzy comparison on {} candidate groups...", fuzzy_candidates.len());
                let fuzzy_results = fuzzy_compare_groups(&fuzzy_candidates, options)?;
                vprintln!("Fuzzy comparison found {} near-duplicate groups", fuzzy_results.len());
                final_duplicates.extend(fuzzy_results);
            }
        }
        return Ok(final_duplicates);
    } else {
        // If no content comparison, just convert to duplicate groups
        for group in &quick_checked_groups {
            if !group.is_empty() {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];
                let size = group[0].size;
                for file in group {
                    files_by_folder[file.folder_index].push(file.path.clone());
                }
                duplicates.push(DuplicateGroup { files_by_folder, size, match_kind: MatchKind::Exact });
            }
        }
    }

    // --- FUZZY STAGE ---
    // If -Z is active, compare files that were NOT promoted to exact duplicates.
    // We derive candidates from saved_name_filtered (the full pre-quick-check pool)
    // rather than quick_checked_groups, so that files which *failed* quick-check
    // (the correct fuzzy candidates when -C is combined with -Z) are included.
    if options.fuzzy {
        // Collect the paths that are already covered by exact matches so we
        // don't double-report them.
        let exact_paths: HashSet<PathBuf> = duplicates
            .iter()
            .flat_map(|g| g.files_by_folder.iter().flat_map(|v| v.iter().cloned()))
            .collect();

        // Build candidate groups from saved_name_filtered: same-size, cross-folder,
        // not already exact.  Using saved_name_filtered (instead of quick_checked_groups)
        // is the critical fix — it includes groups that failed the quick-check gate.
        let fuzzy_candidates: Vec<Vec<FileInfo>> = saved_name_filtered
            .iter()
            .map(|group| {
                group
                    .iter()
                    .filter(|f| !exact_paths.contains(&f.path))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .filter(|g| g.len() > 1 && has_files_from_both_folders(g))
            .collect();

        if !fuzzy_candidates.is_empty() {
            vprintln!("Running fuzzy comparison on {} candidate groups...", fuzzy_candidates.len());
            let fuzzy_results = fuzzy_compare_groups(&fuzzy_candidates, options)?;
            vprintln!("Fuzzy comparison found {} near-duplicate groups", fuzzy_results.len());
            duplicates.extend(fuzzy_results);
        }
    }

    // Sort results by the path of the first file in each group for deterministic output.
    duplicates.sort_by(|a, b| {
        let path_a = a.files_by_folder.iter().flatten().next();
        let path_b = b.files_by_folder.iter().flatten().next();
        path_a.cmp(&path_b)
    });

    Ok(duplicates)
}

fn has_files_from_both_folders(files: &[FileInfo]) -> bool {
    let mut found_folder0 = false;
    let mut found_folder1 = false;
    
    for file in files {
        if file.folder_index == 0 {
            found_folder0 = true;
        } else {
            found_folder1 = true;
        }
        
        if found_folder0 && found_folder1 {
            return true;
        }
    }
    
    false
}

fn group_by_name(files: &[FileInfo]) -> Vec<Vec<FileInfo>> {
    let mut name_groups: HashMap<String, Vec<FileInfo>> = HashMap::new();
    
    for file in files {
        let file_name = file.path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        name_groups.entry(file_name).or_default().push(file.clone());
    }
    
    name_groups.into_values().collect()
}

/// Group files by their stem (filename without any extension).
/// e.g. "photo.jpg" and "photo.png" share stem "photo".
/// Comparison is case-sensitive (consistent with group_by_name).
fn group_by_stem(files: &[FileInfo]) -> Vec<Vec<FileInfo>> {
    let mut stem_groups: HashMap<String, Vec<FileInfo>> = HashMap::new();
    for file in files {
        let stem = file.path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        stem_groups.entry(stem).or_default().push(file.clone());
    }
    stem_groups.into_values().collect()
}

/// Group files by their extension (the part after the last '.').
/// Files without an extension are grouped under the empty string.
/// Comparison is case-insensitive so ".JPG" and ".jpg" match.
fn group_by_ext(files: &[FileInfo]) -> Vec<Vec<FileInfo>> {
    let mut ext_groups: HashMap<String, Vec<FileInfo>> = HashMap::new();
    for file in files {
        let ext = file.path.extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        ext_groups.entry(ext).or_default().push(file.clone());
    }
    ext_groups.into_values().collect()
}

#[allow(dead_code)]
fn sync_content_compare(
    groups: Vec<Vec<FileInfo>>,
    options: &CompareOptions,
    progress_bar: ProgressBar
) -> io::Result<Vec<DuplicateGroup>> {
    let mut duplicates = Vec::new();
    let mut total_size_processed: u64 = 0;
    let mut last_eta_update = Instant::now();
    let eta_update_interval = std::time::Duration::from_millis(500); // update ETA every 0.5s

    let total_size: u64 = groups.iter().flat_map(|g| g.iter()).map(|f| f.size).sum();
    let start_time = Instant::now();

    for group in groups.iter() {
        progress_bar.inc(1);

        if group.len() <= 1 {
            continue;
        }

        // All files in a group share the same size (grouped by size in find_duplicates).
        let group_size: u64 = group.first().map_or(0, |f| f.size);
        let mut content_groups: HashMap<String, Vec<FileInfo>> = HashMap::new();

        for file in group {
            match calculate_file_hash(file, options.quick_content_check)? {
                Some(hash) => {
                    content_groups.entry(hash).or_default().push(file.clone());
                },
                None => continue, // Skip files that couldn't be hashed
            }
            // Update processed size and ETA more frequently (per file)
            total_size_processed += file.size;
            if last_eta_update.elapsed() >= eta_update_interval {
                let elapsed = start_time.elapsed().as_secs_f64();
                let avg_speed = if elapsed > 0.0 { total_size_processed as f64 / elapsed } else { 0.0 };
                let remaining_size = total_size.saturating_sub(total_size_processed);
                let eta = if avg_speed > 0.0 { remaining_size as f64 / avg_speed } else { 0.0 };
                progress_bar.set_message(format!(
                    "ETA: {:.1}s (processed: {:.2} GB, remaining: {:.2} GB, speed: {:.2} MB/s)",
                    eta,
                    total_size_processed as f64 / (1024.0 * 1024.0 * 1024.0),
                    remaining_size as f64 / (1024.0 * 1024.0 * 1024.0),
                    avg_speed / (1024.0 * 1024.0)
                ));
                last_eta_update = Instant::now();
            }
        }

        for (_, content_group) in content_groups {
            if content_group.len() > 1 && 
               (!options.bidirectional || has_files_from_both_folders(&content_group)) {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];

                for file in content_group {
                    files_by_folder[file.folder_index].push(file.path);
                }

                duplicates.push(DuplicateGroup { 
                    files_by_folder, 
                    size: group_size,
                    match_kind: MatchKind::Exact,
                });
            }
        }
    }

    progress_bar.finish();
    Ok(duplicates)
}

// New: sync_content_compare_with_file_progress
fn sync_content_compare_with_file_progress(
    groups: Vec<Vec<FileInfo>>,
    options: &CompareOptions,
    progress_bar: ProgressBar,
    file_progress: ProgressBar,
) -> io::Result<Vec<DuplicateGroup>> {
    let mut duplicates = Vec::new();
    let mut total_size_processed: u64 = 0;
    let mut last_eta_update = Instant::now();
    let eta_update_interval = std::time::Duration::from_millis(500);

    let total_size: u64 = groups.iter().flat_map(|g| g.iter()).map(|f| f.size).sum();
    let start_time = Instant::now();

    for group in &groups {
        progress_bar.inc(1);

        if group.len() <= 1 {
            continue;
        }

        // All files in a group share the same size (grouped by size in find_duplicates).
        // Using the sum would inflate DuplicateGroup.size and corrupt the display totals.
        let group_size: u64 = group.first().map_or(0, |f| f.size);
        let mut content_groups: HashMap<String, Vec<FileInfo>> = HashMap::new();

        for file in group {
            match calculate_file_hash(file, options.quick_content_check)? {
                Some(hash) => {
                    content_groups.entry(hash).or_default().push(file.clone());
                },
                None => continue,
            }
            // Update processed size and ETA more frequently (per file)
            total_size_processed += file.size;
            file_progress.inc(1);
            if last_eta_update.elapsed() >= eta_update_interval {
                let elapsed = start_time.elapsed().as_secs_f64();
                let avg_speed = if elapsed > 0.0 { total_size_processed as f64 / elapsed } else { 0.0 };
                let remaining_size = total_size.saturating_sub(total_size_processed);
                let eta = if avg_speed > 0.0 { remaining_size as f64 / avg_speed } else { 0.0 };
                progress_bar.set_message(format!(
                    "ETA: {:.1}s (processed: {:.2} GB, remaining: {:.2} GB, speed: {:.2} MB/s)",
                    eta,
                    total_size_processed as f64 / (1024.0 * 1024.0 * 1024.0),
                    remaining_size as f64 / (1024.0 * 1024.0 * 1024.0),
                    avg_speed / (1024.0 * 1024.0)
                ));
                last_eta_update = Instant::now();
            }
        }

        for (_, content_group) in content_groups {
            if content_group.len() > 1 && 
               (!options.bidirectional || has_files_from_both_folders(&content_group)) {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];

                for file in content_group {
                    files_by_folder[file.folder_index].push(file.path);
                }

                duplicates.push(DuplicateGroup { 
                    files_by_folder, 
                    size: group_size,
                    match_kind: MatchKind::Exact,
                });
            }
        }
    }

    progress_bar.finish();
    file_progress.finish();
    Ok(duplicates)
}

// New: async_content_compare_with_file_progress
fn async_content_compare_with_file_progress(
    groups: Vec<Vec<FileInfo>>, 
    options: CompareOptions, 
    progress_bar: ProgressBar,
    file_progress: ProgressBar,
) -> io::Result<Vec<DuplicateGroup>> {
    let duplicates = Arc::new(Mutex::new(Vec::new()));
    let progress = Arc::new(progress_bar);
    let file_progress = Arc::new(file_progress);

    let total_size: u64 = groups.iter().flat_map(|g| g.iter()).map(|f| f.size).sum();
    let processed_size = Arc::new(Mutex::new(0u64));
    let start_time = Instant::now();
    let eta_update_interval = std::time::Duration::from_millis(500);
    let last_eta_update = Arc::new(Mutex::new(Instant::now()));

    let num_threads = num_cpus::get();
    let chunks = split_into_chunks(groups, num_threads, options.hdd_optimized);

    let mut handles = Vec::new();

    for chunk in chunks {
        let duplicates = Arc::clone(&duplicates);
        let progress = Arc::clone(&progress);
        let file_progress = Arc::clone(&file_progress);
        let options = options.clone();
        let processed_size = Arc::clone(&processed_size);
        let last_eta_update = Arc::clone(&last_eta_update);

        let handle = thread::spawn(move || -> io::Result<()> {
            // Local counters to reduce lock contention
            let mut local_processed_size = 0u64;
            let mut files_processed = 0u64;
            
            for group in chunk {
                progress.inc(1);

                if group.len() <= 1 {
                    continue;
                }

                let mut content_groups: HashMap<String, Vec<FileInfo>> = HashMap::new();
                let file_size = group[0].size;

                for file in &group {
                    match calculate_file_hash(file, options.quick_content_check)? {
                        Some(hash) => {
                            content_groups.entry(hash).or_default().push(file.clone());
                        },
                        None => continue,
                    }
                    
                    // Update local counters (no locks!)
                    local_processed_size += file.size;
                    files_processed += 1;
                    file_progress.inc(1);
                    
                    // Only update shared state occasionally to reduce lock contention
                    if files_processed % 10 == 0 {  // Update every 10 files instead of every file
                        let mut sz = processed_size.lock().unwrap();
                        *sz += local_processed_size;
                        local_processed_size = 0;  // Reset local counter
                        
                        let mut last = last_eta_update.lock().unwrap();
                        if last.elapsed() >= eta_update_interval {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            let avg_speed = if elapsed > 0.0 { *sz as f64 / elapsed } else { 0.0 };
                            let remaining_size = total_size.saturating_sub(*sz);
                            let eta = if avg_speed > 0.0 { remaining_size as f64 / avg_speed } else { 0.0 };
                            progress.set_message(format!(
                                "ETA: {:.1}s (processed: {:.2} GB, remaining: {:.2} GB, speed: {:.2} MB/s)",
                                eta,
                                *sz as f64 / (1024.0 * 1024.0 * 1024.0),
                                remaining_size as f64 / (1024.0 * 1024.0 * 1024.0),
                                avg_speed / (1024.0 * 1024.0)
                            ));
                            *last = Instant::now();
                        }
                    }
                }

                let mut local_duplicates = Vec::new();

                for (_, content_group) in content_groups {
                    // Match the sync-path guard: without -B, any content group with >1
                    // file is reported (covers intra-folder where all files share
                    // folder_index 0).  With -B, only cross-folder groups are kept.
                    if content_group.len() > 1 &&
                       (!options.bidirectional || has_files_from_both_folders(&content_group)) {
                        let mut files_by_folder = vec![Vec::new(), Vec::new()];

                        for file in content_group {
                            files_by_folder[file.folder_index].push(file.path);
                        }

                        local_duplicates.push(DuplicateGroup { 
                            files_by_folder, 
                            size: file_size,
                            match_kind: MatchKind::Exact,
                        });
                    }
                }

                if !local_duplicates.is_empty() {
                    let mut all_duplicates = duplicates.lock().unwrap();
                    all_duplicates.extend(local_duplicates);
                }
            }
            
            // Flush any remaining local counter at the end
            if local_processed_size > 0 {
                let mut sz = processed_size.lock().unwrap();
                *sz += local_processed_size;
            }

            Ok(())
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap()?;
    }

    progress.finish();
    file_progress.finish();

    let result = Arc::try_unwrap(duplicates)
        .expect("Still have multiple owners of duplicates")
        .into_inner()
        .unwrap();

    Ok(result)
}

// Helper: process files for one folder independently
#[allow(dead_code)]
fn process_files_independent(
    files: Vec<FileInfo>,
    folder_index: usize,
    duplicates: Arc<Mutex<Vec<DuplicateGroup>>>,
    progress: Arc<ProgressBar>,
    _options: CompareOptions,
) -> io::Result<()> {
    // Group files by size
    let mut size_map: HashMap<u64, Vec<FileInfo>> = HashMap::new();
    for f in files {
        size_map.entry(f.size).or_default().push(f);
    }
    for (_size, group) in size_map {
        if group.len() <= 1 { continue; }
        // Group by full hash
        let mut hash_map: HashMap<String, Vec<FileInfo>> = HashMap::new();
        for f in &group {
            if let Some(h) = calculate_file_hash(f, false).ok().flatten() {
                hash_map.entry(h).or_default().push(f.clone());
            }
        }
        for (_hash, dup_group) in hash_map {
            if dup_group.len() > 1 {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];
                for f in dup_group {
                    files_by_folder[folder_index].push(f.path.clone());
                }
                let mut all_duplicates = duplicates.lock().unwrap();
                all_duplicates.push(DuplicateGroup { files_by_folder, size: group[0].size, match_kind: MatchKind::Exact });
            }
        }
        progress.inc(1);
    }
    Ok(())
}

fn calculate_file_hash(file: &FileInfo, quick_check: bool) -> io::Result<Option<String>> {
    let path = &file.path;
    let mut file_handle = File::open(path)?;
    let file_size = file.size;

    // Use a much larger buffer size to reduce I/O system calls
    // 1MB buffer reduces syscalls dramatically: 100MB file = 100 syscalls instead of 1600
    let buffer_size = 1024 * 1024; // 1 MB buffer

    if quick_check && file_size > QUICKCHECK_SIZE as u64 * 2 {
        let mut hasher = Context::new();

        // Read first QUICKCHECK_SIZE bytes
        let mut buffer = vec![0; QUICKCHECK_SIZE];
        let bytes_read = file_handle.read(&mut buffer)?;
        buffer.truncate(bytes_read);
        hasher.consume(&buffer);

        // Read last QUICKCHECK_SIZE bytes
        file_handle.seek(SeekFrom::End(-(QUICKCHECK_SIZE as i64)))?;
        let mut buffer = vec![0; QUICKCHECK_SIZE];
        let bytes_read = file_handle.read(&mut buffer)?;
        buffer.truncate(bytes_read);
        hasher.consume(&buffer);

        let result = hasher.compute();
        Ok(Some(format!("{:x}", result)))
    } else {
        let mut hasher = Context::new();
        let mut buffer = vec![0; buffer_size]; // Use 1MB buffer

        loop {
            let bytes_read = file_handle.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.consume(&buffer[..bytes_read]);
        }

        let result = hasher.compute();
        Ok(Some(format!("{:x}", result)))
    }
}

/// Compare two same-size files chunk by chunk, counting differing bytes.
/// Never loads the full file into RAM — uses a 1MB rolling buffer matching
/// the existing I/O strategy. Always reads both files completely so the
/// caller gets the exact differing byte count even when it exceeds threshold.
/// Returns None if either file cannot be opened/read.
fn chunked_fuzzy_compare(path_a: &Path, path_b: &Path, debug: bool) -> io::Result<u64> {
    const CHUNK: usize = 1024 * 1024; // 1 MB, matches existing buffer size

    let mut fa = File::open(path_a)?;
    let mut fb = File::open(path_b)?;

    let mut buf_a = vec![0u8; CHUNK];
    let mut buf_b = vec![0u8; CHUNK];
    let mut total_diff: u64 = 0;

    loop {
        let na = fa.read(&mut buf_a)?;
        let nb = fb.read(&mut buf_b)?;

        // Both files are same size (precondition from size grouping), so na == nb
        // at every chunk. If they somehow diverge, stop — something is wrong.
        if na == 0 && nb == 0 {
            break;
        }
        if na != nb {
            if debug {
                eprintln!(
                    "[FUZZY] Chunk size mismatch ({} vs {}) for {} <-> {} — aborting",
                    na, nb, path_a.display(), path_b.display()
                );
            }
            // Treat divergent read as maximum difference so the pair is rejected.
            return Ok(u64::MAX);
        }

        let chunk_diff = buf_a[..na]
            .iter()
            .zip(buf_b[..nb].iter())
            .filter(|(a, b)| a != b)
            .count() as u64;
        total_diff += chunk_diff;
    }

    Ok(total_diff)
}

/// Run fuzzy comparison across all size-groups that already failed exact
/// hashing (or were never hashed when -Z is used standalone).
/// Groups files from both folders by size, then for every cross-folder pair
/// within each size bucket runs chunked_fuzzy_compare.
/// Returns (fuzzy_matches, exceeded_pairs_for_debug).
fn fuzzy_compare_groups(
    groups: &[Vec<FileInfo>],
    options: &CompareOptions,
) -> io::Result<Vec<DuplicateGroup>> {
    let mut results = Vec::new();

    for group in groups {
        // Collect files from each folder separately
        let folder0: Vec<&FileInfo> = group.iter().filter(|f| f.folder_index == 0).collect();
        let folder1: Vec<&FileInfo> = group.iter().filter(|f| f.folder_index == 1).collect();

        // Cross-product: every f0 against every f1 of the same size
        for f0 in &folder0 {
            for f1 in &folder1 {
                let diff = match chunked_fuzzy_compare(&f0.path, &f1.path, options.debug) {
                    Ok(d) => d,
                    Err(e) => {
                        if options.debug {
                            eprintln!(
                                "[FUZZY] Error comparing {} <-> {}: {}",
                                f0.path.display(), f1.path.display(), e
                            );
                        }
                        continue;
                    }
                };

                if diff <= options.tolerance {
                    if options.debug {
                        println!(
                            "[FUZZY MATCH] {} bytes differ (≤{} threshold)\n  {} <-> {}",
                            diff, options.tolerance,
                            f0.path.display(), f1.path.display()
                        );
                    }
                    let files_by_folder = vec![
                        vec![f0.path.clone()],
                        vec![f1.path.clone()],
                    ];
                    results.push(DuplicateGroup {
                        files_by_folder,
                        size: f0.size,
                        match_kind: MatchKind::Fuzzy { bytes_differing: diff },
                    });
                } else if options.debug {
                    println!(
                        "[FUZZY EXCEED] {} bytes differ (>{} threshold) — skipped\n  {} <-> {}",
                        diff, options.tolerance,
                        f0.path.display(), f1.path.display()
                    );
                }
            }
        }
    }

    Ok(results)
}

fn split_into_chunks<T: Clone>(items: Vec<T>, chunk_count: usize, hdd_optimized: bool) -> Vec<Vec<T>> {
    let mut chunks = Vec::new();
    let max_chunks = if hdd_optimized { chunk_count.min(4) } else { chunk_count }; // Limit threads for HDDs
    let chunk_size = (items.len() + max_chunks - 1) / max_chunks.max(1);

    for chunk in items.chunks(chunk_size.max(1)) {
        chunks.push(chunk.to_vec());
    }

    chunks
}

#[allow(dead_code)]
fn process_groups(
    groups: Vec<Vec<FileInfo>>, 
    duplicates: Arc<Mutex<Vec<DuplicateGroup>>>, 
    progress: Arc<ProgressBar>, 
    options: CompareOptions
) -> io::Result<()> {
    for group in groups {
        progress.inc(1);

        if group.len() <= 1 {
            continue;
        }

        let mut content_groups: HashMap<String, Vec<FileInfo>> = HashMap::new();
        let file_size = group[0].size;

        for file in &group {
            match calculate_file_hash(file, options.quick_content_check)? {
                Some(hash) => {
                    content_groups.entry(hash).or_default().push(file.clone());
                },
                None => continue,
            }
        }

        let mut local_duplicates = Vec::new();

        for (_, content_group) in content_groups {
            if content_group.len() > 1 && 
               (!options.bidirectional || has_files_from_both_folders(&content_group)) {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];

                for file in content_group {
                    files_by_folder[file.folder_index].push(file.path);
                }

                local_duplicates.push(DuplicateGroup { 
                    files_by_folder, 
                    size: file_size,
                    match_kind: MatchKind::Exact,
                });
            }
        }

        if !local_duplicates.is_empty() {
            let mut all_duplicates = duplicates.lock().unwrap();
            all_duplicates.extend(local_duplicates);
        }
    }

    Ok(())
}

/// Move fuzzy-matched files from folder1 into a `differ/` subfolder,
/// preserving relative paths. Respects the same -D/-F semantics as
/// move_duplicates_to_deleted:
///   -D (delete_duplicates, !force_delete): only move if the counterpart
///      exists at the *same relative path* inside folder2.
///   -F (force_delete): move regardless of folder2 structure.
fn move_fuzzy_to_differ(
    fuzzy_groups: &[DuplicateGroup],
    folder1: &str,
    folder2: &str,
    options: &CompareOptions,
) -> io::Result<()> {
    let differ_folder = sanitize_path(&Path::new(folder1).join("differ"));
    if !differ_folder.exists() {
        if options.debug {
            println!("Creating 'differ' folder at: {}", differ_folder.display());
        }
        create_dir_all(&differ_folder)?;
    }

    for group in fuzzy_groups {
        let bytes_diff = match &group.match_kind {
            MatchKind::Fuzzy { bytes_differing } => *bytes_differing,
            _ => 0,
        };

        for file_path in &group.files_by_folder[0] {
            let sanitized = sanitize_path(file_path);
            let relative = match sanitized.strip_prefix(sanitize_path(folder1)) {
                Ok(p) => p,
                Err(e) => {
                    if options.debug {
                        eprintln!("Error stripping prefix for {}: {}", sanitized.display(), e);
                    }
                    continue;
                }
            };

            // -D: only move if the counterpart exists at the same relative
            // path in folder2. -F or single-mode: skip this check.
            if !options.force_delete && !folder2.is_empty() {
                let corresponding = sanitize_path(&Path::new(folder2).join(relative));
                if !corresponding.exists() {
                    if options.debug {
                        println!(
                            "[FUZZY] Skipping {} — no match at same path in folder2: {} (use -F to force)",
                            sanitized.display(), corresponding.display()
                        );
                    }
                    continue;
                }
            }

            let target = sanitize_path(&differ_folder.join(relative));

            if options.debug {
                println!(
                    "[FUZZY] Moving ({} bytes differ): {} -> {}",
                    bytes_diff, sanitized.display(), target.display()
                );
            }

            if let Some(parent) = target.parent() {
                if let Err(e) = create_dir_all(parent) {
                    eprintln!("Failed to create directory {}: {}", parent.display(), e);
                    continue;
                }
            }

            if !sanitized.exists() {
                if options.debug {
                    eprintln!("Source file does not exist: {}", sanitized.display());
                }
                continue;
            }

            match rename(&sanitized, &target) {
                Ok(_) => {
                    println!(
                        "Moved to differ/ ({} bytes differ): {}",
                        bytes_diff, sanitized.display()
                    );
                }
                Err(_) => {
                    if let Err(e) = copy_and_remove(&sanitized, &target, options) {
                        eprintln!(
                            "Failed to move {} to differ/: {}",
                            sanitized.display(), e
                        );
                    } else {
                        println!(
                            "Moved to differ/ ({} bytes differ): {}",
                            bytes_diff, sanitized.display()
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

fn move_duplicates_to_deleted(
    duplicates: &[DuplicateGroup],
    folder1: &str,
    folder2: &str,
    options: &CompareOptions,
) -> io::Result<()> {
    let deleted_folder = sanitize_path(&Path::new(folder1).join("deleted"));
    if !deleted_folder.exists() {
        if options.debug {
            println!("Creating 'deleted' folder at: {}", deleted_folder.display());
        }
        create_dir_all(&deleted_folder)?;
    }

    for duplicate in duplicates {
        for file_path in &duplicate.files_by_folder[0] {
            let sanitized_file_path = sanitize_path(file_path);
            let relative_path = match sanitized_file_path.strip_prefix(sanitize_path(folder1)) {
                Ok(path) => path,
                Err(e) => {
                    if options.debug {
                        eprintln!("Error stripping prefix for file: {}: {}", sanitized_file_path.display(), e);
                    }
                    continue;
                }
            };

            let target_path = sanitize_path(&deleted_folder.join(relative_path));

            if options.debug {
                println!("Moving file: {}", sanitized_file_path.display());
                println!("Target path: {}", target_path.display());
            }

            // Only check for corresponding_path if not force_delete
            if !options.force_delete {
                let corresponding_path = sanitize_path(&Path::new(folder2).join(relative_path));
                if !corresponding_path.exists() {
                    if options.debug {
                        println!("File does not exist in folder2: {}", corresponding_path.display());
                        println!("Skipping file (use -F to force delete regardless of folder2 path)");
                    }
                    continue;
                }
            }

            // Create parent directories in the "deleted" folder
            if let Some(parent) = target_path.parent() {
                if options.debug {
                    println!("Ensuring parent directories exist for: {}", parent.display());
                }
                if let Err(e) = create_dir_all(parent) {
                    eprintln!("Failed to create directory {}: {}", parent.display(), e);
                    continue;
                }
            }

            // Verify source file exists before attempting move
            if !sanitized_file_path.exists() {
                if options.debug {
                    eprintln!("Source file does not exist: {}", sanitized_file_path.display());
                }
                continue;
            }

            // Try rename first (fast, atomic operation if on same filesystem)
            // Falls back to copy+remove only if rename fails (cross-device, etc.)
            match rename(&sanitized_file_path, &target_path) {
                Ok(_) => {
                    if options.debug {
                        println!("File moved successfully using rename()");
                    }
                    println!("Moved: {}", sanitized_file_path.display());
                }
                Err(e) => {
                    if options.debug {
                        eprintln!("Rename failed: {} -> {}: {}", sanitized_file_path.display(), target_path.display(), e);
                        
                        // Check if it's a cross-device error
                        #[cfg(unix)]
                        if let Some(code) = e.raw_os_error() {
                            if code == 18 {
                                println!("Cross-device move detected (EXDEV); falling back to copy+remove");
                            } else {
                                println!("Rename failed with error code {}; falling back to copy+remove", code);
                            }
                        }
                        
                        #[cfg(not(unix))]
                        println!("Falling back to copy+remove");
                    }
                    
                    // Fallback to verified copy+remove
                    if let Err(copy_err) = copy_and_remove(&sanitized_file_path, &target_path, options) {
                        eprintln!("Failed to move file {} to {}: {}", sanitized_file_path.display(), target_path.display(), copy_err);
                        continue;
                    }
                    println!("Moved: {}", sanitized_file_path.display());
                }
            }
        }
    }

    Ok(())
}

fn sanitize_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::Prefix(prefix) => sanitized.push(prefix.as_os_str()),
            Component::RootDir => sanitized.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => sanitized.push("."),
            Component::ParentDir => sanitized.push(".."),
            Component::Normal(os_str) => {
                let comp_str = os_str.to_string_lossy();
                let sanitized_str = comp_str
                    .trim_end_matches(' ')
                    .replace(['<', '>', ':', '"', '|', '?', '*'], "_");
                sanitized.push(sanitized_str);
            }
        }
    }
    sanitized
}

fn copy_and_remove(src: &Path, dst: &Path, options: &CompareOptions) -> io::Result<()> {
    if options.debug {
        println!("Copying file as fallback: {} -> {}", src.display(), dst.display());
    }

    // Get source metadata before copying (for verification and preservation)
    let src_metadata = src.metadata()?;
    let src_size = src_metadata.len();
    let src_perms = src_metadata.permissions();
    
    // Store timestamps for preservation
    let src_atime = src_metadata.accessed().ok();
    let src_mtime = src_metadata.modified().ok();

    // Perform copy
    let bytes_copied = std::fs::copy(src, dst)?;

    // CRITICAL: Verify that the copy was complete before removing original
    if bytes_copied != src_size {
        // Copy didn't complete fully - DO NOT remove original file
        let err_msg = format!(
            "Copy verification failed: expected {} bytes, got {} bytes. Original file preserved at {}",
            src_size, bytes_copied, src.display()
        );
        eprintln!("ERROR: {}", err_msg);
        // Clean up the incomplete destination file
        let _ = std::fs::remove_file(dst);
        return Err(io::Error::new(io::ErrorKind::Other, err_msg));
    }

    // Verify destination file exists and has correct size
    match dst.metadata() {
        Ok(dst_metadata) => {
            if dst_metadata.len() != src_size {
                let err_msg = format!(
                    "Destination file size mismatch: expected {} bytes, got {} bytes. Original file preserved at {}",
                    src_size, dst_metadata.len(), src.display()
                );
                eprintln!("ERROR: {}", err_msg);
                let _ = std::fs::remove_file(dst);
                return Err(io::Error::new(io::ErrorKind::Other, err_msg));
            }
        }
        Err(e) => {
            let err_msg = format!(
                "Cannot verify destination file: {}. Original file preserved at {}",
                e, src.display()
            );
            eprintln!("ERROR: {}", err_msg);
            return Err(io::Error::new(io::ErrorKind::Other, err_msg));
        }
    }

    if options.debug {
        println!("Copy verified: {} bytes successfully copied", bytes_copied);
    }

    // Preserve permissions - warn if this fails but don't abort
    if let Err(e) = std::fs::set_permissions(dst, src_perms) {
        if options.debug {
            eprintln!("Warning: Failed to preserve permissions on {}: {}", dst.display(), e);
        }
    }

    // Preserve timestamps - warn if this fails but don't abort
    if let (Some(atime), Some(mtime)) = (src_atime, src_mtime) {
        let fa = filetime::FileTime::from_system_time(atime);
        let fm = filetime::FileTime::from_system_time(mtime);
        if let Err(e) = filetime::set_file_times(dst, fa, fm) {
            if options.debug {
                eprintln!("Warning: Failed to preserve timestamps on {}: {}", dst.display(), e);
            }
        } else if options.debug {
            println!("Timestamps preserved successfully");
        }
    }

    // Only now, after all verifications pass, remove the original file
    if options.debug {
        println!("Verification complete, removing original file: {}", src.display());
    }
    std::fs::remove_file(src)?;

    Ok(())
}

#[allow(dead_code)]
fn to_long_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    if cfg!(windows) {
        if let Some(path_str) = path.to_str() {
            if path_str.len() > 260 && !path_str.starts_with(r"\\?\") {
                return PathBuf::from(format!(r"\\?\{}", path_str));
            }
        }
    }
    path.to_path_buf()
}

fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} bytes", size)
    }
}

fn display_results(duplicates: &[DuplicateGroup], folder1: &str, folder2: &str) {
    if duplicates.is_empty() {
        println!("No duplicates found.");
        return;
    }

    let exact_count = duplicates.iter().filter(|g| g.match_kind == MatchKind::Exact).count();
    let fuzzy_count = duplicates.len() - exact_count;

    println!("\nFound {} duplicate group(s) ({} exact, {} fuzzy near-duplicate):",
        duplicates.len(), exact_count, fuzzy_count);
    println!("{:40} : {:40} | {:10} | {}", folder1, folder2, "size", "match");
    println!("{}", "-".repeat(100));
    
    let mut total_folder1_files = 0;
    let mut total_folder2_files = 0;
    let mut total_duplicate_size_folder1 = 0;
    
    for duplicate in duplicates {
        let folder1_files = &duplicate.files_by_folder[0];
        let folder2_files = &duplicate.files_by_folder[1];
        let size_str = format_size(duplicate.size);

        let match_label = match &duplicate.match_kind {
            MatchKind::Exact => "EXACT".to_string(),
            MatchKind::Fuzzy { bytes_differing } => format!("FUZZY ~{}B", bytes_differing),
        };
        
        total_folder1_files += folder1_files.len();
        total_folder2_files += folder2_files.len();
        total_duplicate_size_folder1 += folder1_files.len() as u64 * duplicate.size;
        
        let folder1_text = if folder1_files.is_empty() {
            String::new()
        } else {
            folder1_files
                .iter()
                .map(|p| p.strip_prefix(folder1).unwrap_or(p).to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        };
        
        let folder2_text = if folder2_files.is_empty() {
            String::new()
        } else {
            folder2_files
                .iter()
                .map(|p| p.strip_prefix(folder2).unwrap_or(p).to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        };
        
        println!("{:40} : {:40} | {:10} | {}", folder1_text, folder2_text, size_str, match_label);
        
        let mut has_subdirs = false;
        for path in folder1_files.iter().chain(folder2_files.iter()) {
            if path.parent().unwrap_or(Path::new("")) != Path::new(folder1) &&
               path.parent().unwrap_or(Path::new("")) != Path::new(folder2) {
                has_subdirs = true;
                break;
            }
        }
        
        if has_subdirs {
            display_subdirectory_details(folder1_files, folder2_files, folder1, folder2);
        }
    }
    
    println!("{}", "-".repeat(100));
    println!("Total ratio: {}:{}", total_folder1_files, total_folder2_files);
    println!("Total duplicates size (from 1st folder): {}", format_size(total_duplicate_size_folder1));
}

fn display_subdirectory_details(
    folder1_files: &[PathBuf], 
    folder2_files: &[PathBuf],
    base_folder1: &str,
    base_folder2: &str
) {
    // Get common parent paths
    let mut subdir_map: HashMap<PathBuf, (Vec<PathBuf>, Vec<PathBuf>)> = HashMap::new();
    
    // Group files by their parent directory
    for path in folder1_files {
        if let Some(parent) = path.parent() {
            if parent != Path::new(base_folder1) {
                let rel_parent = parent.strip_prefix(base_folder1).unwrap_or(parent);
                let entry = subdir_map.entry(rel_parent.to_path_buf()).or_default();
                entry.0.push(path.clone());
            }
        }
    }
    
    for path in folder2_files {
        if let Some(parent) = path.parent() {
            if parent != Path::new(base_folder2) {
                let rel_parent = parent.strip_prefix(base_folder2).unwrap_or(parent);
                let entry = subdir_map.entry(rel_parent.to_path_buf()).or_default();
                entry.1.push(path.clone());
            }
        }
    }
    
    // Display subdirectory details
    for (subdir, (files1, files2)) in subdir_map {
        let subdir_str = subdir.to_string_lossy();
        println!("{}\\", subdir_str);
        
        // Create sets of filenames from both folders
        let names1: HashSet<_> = files1.iter()
            .filter_map(|p| p.file_name())
            .collect();
        
        let names2: HashSet<_> = files2.iter()
            .filter_map(|p| p.file_name())
            .collect();
        
        // Find common and unique filenames
        for name in names1.union(&names2) {
            let in_folder1 = names1.contains(name);
            let in_folder2 = names2.contains(name);
            
            let left_mark = if in_folder1 { "|- " } else { "   " };
            let middle = if in_folder1 && in_folder2 { " : " } else { "   " };
            let right_mark = if in_folder2 { "|- " } else { "   " };
            
            println!("{}{}{}{}{}",
                left_mark,
                name.to_string_lossy(),
                middle,
                right_mark,
                if in_folder2 { name.to_string_lossy() } else { "".into() }
            );
        }
    }
}
// ══════════════════════════════════════════════════════════════════════════════
// Tests
// Run with: cargo test
// Run with output:  cargo test -- --nocapture
// Run one test:     cargo test test_name
// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;

    // ─── RAII temp-dir guard ──────────────────────────────────────────────────
    // Automatically deletes the directory on drop, even when a test panics.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("duptool_test_{}", tag));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir)
                .unwrap_or_else(|e| panic!("create temp dir {:?}: {}", dir, e));
            TempDir(dir)
        }
        fn path(&self) -> &Path { &self.0 }
    }

    impl Drop for TempDir {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    // ─── helpers ──────────────────────────────────────────────────────────────

    fn write_file(path: &Path, content: &[u8]) {
        let mut f = File::create(path)
            .unwrap_or_else(|e| panic!("create {:?}: {}", path, e));
        f.write_all(content).unwrap();
    }

    fn make_file_info(path: &str, size: u64, folder_index: usize) -> FileInfo {
        FileInfo { path: PathBuf::from(path), size, folder_index }
    }

    /// A fully-zeroed CompareOptions. Use struct-update syntax to set only
    /// what a specific test needs: `CompareOptions { compare_name: true, ..default_options() }`
    fn default_options() -> CompareOptions {
        CompareOptions {
            compare_content:    false,
            compare_name:       false,
            compare_size:       false,
            quick_content_check: false,
            everything_name:    false,
            everything_size:    false,
            bidirectional:      false,
            async_compare:      false,
            hdd_optimized:      false,
            delete_duplicates:  false,
            debug:              false,
            enhanced_async:     false,
            force_delete:       false,
            intra_folder:       false,
            fuzzy:              false,
            tolerance:          0,
            fuzzy_as_dupes:     false,
            compare_stem:       false,
            compare_ext:        false,
        }
    }

    /// Options for a plain content-based comparison (the most common mode).
    fn content_options() -> CompareOptions {
        CompareOptions { compare_content: true, compare_size: true, ..default_options() }
    }

    /// Convenience: collect + find for a two-folder run.
    fn run_find(d1: &TempDir, d2: &TempDir, opts: &CompareOptions) -> Vec<DuplicateGroup> {
        let f1 = collect_files(d1.path().to_str().unwrap(), 0, false).unwrap();
        let f2 = collect_files(d2.path().to_str().unwrap(), 1, false).unwrap();
        find_duplicates(
            f1, f2,
            d1.path().to_str().unwrap(),
            d2.path().to_str().unwrap(),
            opts,
        ).unwrap()
    }

    /// Convenience: single-folder intra-comparison.
    fn run_find_single(d1: &TempDir, opts: &CompareOptions) -> Vec<DuplicateGroup> {
        let f1 = collect_files(d1.path().to_str().unwrap(), 0, false).unwrap();
        find_duplicates(f1, vec![], d1.path().to_str().unwrap(), "", opts).unwrap()
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // group_by_name
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_group_by_name_same_name_grouped() {
        let files = vec![
            make_file_info("/a/photo.jpg", 100, 0),
            make_file_info("/b/photo.jpg", 100, 1),
            make_file_info("/a/other.txt", 100, 0),
        ];
        let groups = group_by_name(&files);
        let photo: Vec<_> = groups.iter()
            .filter(|g| g[0].path.file_name().unwrap() == "photo.jpg")
            .collect();
        assert_eq!(photo.len(), 1, "both photo.jpg entries must be in one group");
        assert_eq!(photo[0].len(), 2);
    }

    #[test]
    fn test_group_by_name_all_different_stays_separate() {
        let files = vec![
            make_file_info("/a/a.txt", 100, 0),
            make_file_info("/b/b.txt", 100, 1),
        ];
        let groups = group_by_name(&files);
        assert!(groups.iter().all(|g| g.len() == 1),
            "distinct filenames must each get their own group");
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // group_by_stem  (new in v0.1.13)
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_group_by_stem_different_extensions_grouped() {
        // photo.jpg / photo.png / photo.bmp all share stem "photo"
        let files = vec![
            make_file_info("/a/photo.jpg", 100, 0),
            make_file_info("/b/photo.png", 100, 1),
            make_file_info("/c/photo.bmp", 100, 0),
        ];
        let groups = group_by_stem(&files);
        assert_eq!(groups.len(), 1, "all three share stem 'photo'");
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn test_group_by_stem_different_stems_stay_separate() {
        let files = vec![
            make_file_info("/a/photo.jpg",  100, 0),
            make_file_info("/b/backup.jpg", 100, 1),
        ];
        let groups = group_by_stem(&files);
        assert_eq!(groups.len(), 2, "different stems must produce separate groups");
    }

    #[test]
    fn test_group_by_stem_no_extension_shares_stem() {
        // "README"    -> file_stem = "README"
        // "README.md" -> file_stem = "README"   (stdlib strips only the last dot-suffix)
        let files = vec![
            make_file_info("/a/README",    100, 0),
            make_file_info("/b/README.md", 100, 1),
        ];
        let groups = group_by_stem(&files);
        assert_eq!(groups.len(), 1, "README and README.md share stem 'README'");
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_by_stem_compound_extension() {
        // file_stem strips only the *last* suffix:
        // "archive.tar.gz"  -> "archive.tar"
        // "archive.tar.bz2" -> "archive.tar"   → same group
        let files = vec![
            make_file_info("/a/archive.tar.gz",  100, 0),
            make_file_info("/b/archive.tar.bz2", 100, 1),
        ];
        let groups = group_by_stem(&files);
        assert_eq!(groups.len(), 1,
            "both share stem 'archive.tar' (stdlib strips only the last suffix)");
    }

    #[test]
    fn test_group_by_stem_case_sensitive() {
        // Stem comparison is case-sensitive (consistent with group_by_name).
        let files = vec![
            make_file_info("/a/Photo.jpg", 100, 0),
            make_file_info("/b/photo.jpg", 100, 1),
        ];
        let groups = group_by_stem(&files);
        assert_eq!(groups.len(), 2,
            "stem comparison is case-sensitive: 'Photo' != 'photo'");
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // group_by_ext  (new in v0.1.13)
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_group_by_ext_same_extension_grouped() {
        let files = vec![
            make_file_info("/a/photo.jpg",    100, 0),
            make_file_info("/b/vacation.jpg", 100, 1),
        ];
        let groups = group_by_ext(&files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_by_ext_case_insensitive() {
        // .JPG and .jpg must land in the same group.
        let files = vec![
            make_file_info("/a/photo.JPG",    100, 0),
            make_file_info("/b/vacation.jpg", 100, 1),
        ];
        let groups = group_by_ext(&files);
        assert_eq!(groups.len(), 1, ".JPG and .jpg must match case-insensitively");
    }

    #[test]
    fn test_group_by_ext_different_extensions_separated() {
        let files = vec![
            make_file_info("/a/photo.jpg",    100, 0),
            make_file_info("/b/document.pdf", 100, 1),
        ];
        let groups = group_by_ext(&files);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_group_by_ext_no_extension_grouped_under_empty_key() {
        // Files without an extension share the empty-string key.
        let files = vec![
            make_file_info("/a/Makefile",   100, 0),
            make_file_info("/b/Dockerfile", 100, 1),
        ];
        let groups = group_by_ext(&files);
        assert_eq!(groups.len(), 1, "both have no extension → same group");
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_by_ext_no_ext_vs_with_ext_separated() {
        let files = vec![
            make_file_info("/a/Makefile",    100, 0),
            make_file_info("/b/Makefile.mk", 100, 1),
        ];
        let groups = group_by_ext(&files);
        assert_eq!(groups.len(), 2, "no-extension and .mk must be separate groups");
    }

    #[test]
    fn test_group_by_ext_dotted_name_only_last_suffix_is_ext() {
        // "Act 1. The Noon.mp4" — the dot in "1." is part of the stem, NOT the extension.
        // Path::extension() returns only the text after the *last* dot, so the
        // extension is "mp4" and not ". The Noon.mp4" or anything else.
        let files = vec![
            make_file_info("/a/Act 1. The Noon.mp4", 100, 0),
            make_file_info("/b/Act 2. The Dusk.mp4", 100, 1),
        ];
        let groups = group_by_ext(&files);
        assert_eq!(groups.len(), 1, "both have extension 'mp4'; dots in the title are irrelevant");
        // Confirm the stem is also correct: everything before the last dot.
        let stem = Path::new("Act 1. The Noon.mp4")
            .file_stem()
            .unwrap()
            .to_string_lossy();
        assert_eq!(stem, "Act 1. The Noon",
            "stem should be everything before the last dot, not affected by title dots");
    }

    #[test]
    fn test_group_by_ext_dotted_name_different_ext_separated() {
        // Same title pattern, different container format → different extension → different group.
        let files = vec![
            make_file_info("/a/Act 1. The Noon.mp4", 100, 0),
            make_file_info("/b/Act 1. The Noon.mkv", 100, 1),
        ];
        let groups = group_by_ext(&files);
        assert_eq!(groups.len(), 2, ".mp4 and .mkv must be in separate groups");
    }

    #[test]
    fn test_group_by_stem_dotted_name_groups_by_full_title() {
        // In -k mode, "Act 1. The Noon.mp4" and "Act 1. The Noon.mkv" share the
        // same stem "Act 1. The Noon" and should be grouped together.
        let files = vec![
            make_file_info("/a/Act 1. The Noon.mp4", 100, 0),
            make_file_info("/b/Act 1. The Noon.mkv", 100, 1),
        ];
        let groups = group_by_stem(&files);
        assert_eq!(groups.len(), 1, "both share stem 'Act 1. The Noon'");
        assert_eq!(groups[0].len(), 2);
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // format_size
    // ═══════════════════════════════════════════════════════════════════════════

    #[test] fn test_format_size_zero()         { assert_eq!(format_size(0),               "0 bytes"); }
    #[test] fn test_format_size_small_bytes()  { assert_eq!(format_size(512),             "512 bytes"); }
    #[test] fn test_format_size_boundary_1kb() { assert_eq!(format_size(1024),            "1.00 KB"); }
    #[test] fn test_format_size_2kb()          { assert_eq!(format_size(2048),            "2.00 KB"); }
    #[test] fn test_format_size_boundary_1mb() { assert_eq!(format_size(1024*1024),       "1.00 MB"); }
    #[test] fn test_format_size_boundary_1gb() { assert_eq!(format_size(1024*1024*1024),  "1.00 GB"); }


    // ═══════════════════════════════════════════════════════════════════════════
    // has_files_from_both_folders
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_has_both_folders_true() {
        let files = vec![
            make_file_info("/a/f.txt", 10, 0),
            make_file_info("/b/f.txt", 10, 1),
        ];
        assert!(has_files_from_both_folders(&files));
    }

    #[test]
    fn test_has_both_folders_only_folder0() {
        let files = vec![
            make_file_info("/a/f.txt", 10, 0),
            make_file_info("/a/g.txt", 10, 0),
        ];
        assert!(!has_files_from_both_folders(&files));
    }

    #[test]
    fn test_has_both_folders_empty_slice() {
        assert!(!has_files_from_both_folders(&[]));
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // sanitize_path
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_sanitize_path_trailing_space_stripped() {
        let p = sanitize_path(Path::new("/some/path/name with space "));
        let last = p.file_name().unwrap().to_string_lossy();
        assert!(!last.ends_with(' '),
            "trailing space must be stripped from the last component");
    }

    #[test]
    fn test_sanitize_path_regular_path_unchanged() {
        let p = sanitize_path(Path::new("/normal/path/file.txt"));
        assert_eq!(p, PathBuf::from("/normal/path/file.txt"));
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // calculate_file_hash
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_hash_identical_content_same_hash() {
        let d = TempDir::new("hash_same");
        let f1 = d.path().join("a.txt");
        let f2 = d.path().join("b.txt");
        write_file(&f1, b"hello world");
        write_file(&f2, b"hello world");
        let i1 = FileInfo { path: f1, size: 11, folder_index: 0 };
        let i2 = FileInfo { path: f2, size: 11, folder_index: 1 };
        assert_eq!(
            calculate_file_hash(&i1, false).unwrap(),
            calculate_file_hash(&i2, false).unwrap(),
        );
    }

    #[test]
    fn test_hash_different_content_different_hash() {
        let d = TempDir::new("hash_diff");
        let f1 = d.path().join("a.txt");
        let f2 = d.path().join("b.txt");
        write_file(&f1, b"hello");
        write_file(&f2, b"world");
        let i1 = FileInfo { path: f1, size: 5, folder_index: 0 };
        let i2 = FileInfo { path: f2, size: 5, folder_index: 1 };
        assert_ne!(
            calculate_file_hash(&i1, false).unwrap(),
            calculate_file_hash(&i2, false).unwrap(),
        );
    }

    #[test]
    fn test_hash_quick_equals_full_for_small_file() {
        // Files smaller than 2 × QUICKCHECK_SIZE are read entirely in both
        // modes, so the resulting hash must be identical.
        let d = TempDir::new("hash_qk");
        let f = d.path().join("small.bin");
        write_file(&f, b"tiny content here!");
        let info = FileInfo { path: f, size: 18, folder_index: 0 };
        assert_eq!(
            calculate_file_hash(&info, false).unwrap(),
            calculate_file_hash(&info, true).unwrap(),
            "quick hash must equal full hash for files well below the quick-check threshold",
        );
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // chunked_fuzzy_compare
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_fuzzy_compare_identical_zero_diff() {
        let d = TempDir::new("fuz_id");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        write_file(&f1, &vec![0u8; 4096]);
        write_file(&f2, &vec![0u8; 4096]);
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, false).unwrap(), 0);
    }

    #[test]
    fn test_fuzzy_compare_one_byte_difference() {
        let d = TempDir::new("fuz_1b");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        let mut cb = vec![0u8; 4096];
        cb[2048] = 0xFF;
        write_file(&f1, &vec![0u8; 4096]);
        write_file(&f2, &cb);
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, false).unwrap(), 1);
    }

    #[test]
    fn test_fuzzy_compare_multiple_scattered_diffs() {
        let d = TempDir::new("fuz_mb");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        let ca = vec![0u8; 1024];
        let mut cb = vec![0u8; 1024];
        cb[0]   = 1;
        cb[100] = 1;
        cb[500] = 1;  // 3 bytes differ
        write_file(&f1, &ca);
        write_file(&f2, &cb);
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, false).unwrap(), 3);
    }

    #[test]
    fn test_fuzzy_compare_all_bytes_differ() {
        let d = TempDir::new("fuz_all");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        write_file(&f1, &vec![0x00u8; 256]);
        write_file(&f2, &vec![0xFFu8; 256]);
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, false).unwrap(), 256);
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // find_duplicates — integration tests
    // ═══════════════════════════════════════════════════════════════════════════

    // ── basic content matching ─────────────────────────────────────────────────

    #[test]
    fn test_fd_exact_content_match_found() {
        let d1 = TempDir::new("fd_ec1");
        let d2 = TempDir::new("fd_ec2");
        write_file(&d1.path().join("a.txt"), b"duplicate content");
        write_file(&d2.path().join("a.txt"), b"duplicate content");
        let dups = run_find(&d1, &d2, &content_options());
        assert_eq!(dups.len(), 1, "one duplicate group expected");
        assert_eq!(dups[0].match_kind, MatchKind::Exact);
    }

    #[test]
    fn test_fd_different_content_no_match() {
        let d1 = TempDir::new("fd_dc1");
        let d2 = TempDir::new("fd_dc2");
        write_file(&d1.path().join("a.txt"), b"content A side!");
        write_file(&d2.path().join("b.txt"), b"content B side!");
        let dups = run_find(&d1, &d2, &content_options());
        assert!(dups.is_empty(), "different content → no duplicate");
    }

    #[test]
    fn test_fd_size_mismatch_eliminated_before_hash() {
        let d1 = TempDir::new("fd_sm1");
        let d2 = TempDir::new("fd_sm2");
        write_file(&d1.path().join("a.txt"), b"short");
        write_file(&d2.path().join("a.txt"), b"much longer content here");
        let dups = run_find(&d1, &d2, &content_options());
        assert!(dups.is_empty(), "size-mismatched files must be eliminated before hashing");
    }

    #[test]
    fn test_fd_multiple_groups_found() {
        let d1 = TempDir::new("fd_mg1");
        let d2 = TempDir::new("fd_mg2");
        write_file(&d1.path().join("alpha.txt"), b"group one match");
        write_file(&d2.path().join("alpha.txt"), b"group one match");
        write_file(&d1.path().join("beta.dat"),  b"group two!!");
        write_file(&d2.path().join("beta.dat"),  b"group two!!");
        let dups = run_find(&d1, &d2, &content_options());
        assert_eq!(dups.len(), 2, "two independent duplicate groups should both be found");
    }

    // ── name mode (-n) ─────────────────────────────────────────────────────────

    #[test]
    fn test_fd_name_mode_same_name_same_size_matches() {
        let d1 = TempDir::new("fd_nm1");
        let d2 = TempDir::new("fd_nm2");
        write_file(&d1.path().join("photo.txt"), b"aaaa");
        write_file(&d2.path().join("photo.txt"), b"bbbb"); // same name+size, different content
        let opts = CompareOptions { compare_name: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1, "same name + same size → match in -n mode");
    }

    #[test]
    fn test_fd_name_mode_different_name_no_match() {
        let d1 = TempDir::new("fd_nm3");
        let d2 = TempDir::new("fd_nm4");
        write_file(&d1.path().join("photo.txt"), b"aaaa");
        write_file(&d2.path().join("other.txt"), b"bbbb");
        let opts = CompareOptions { compare_name: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert!(dups.is_empty(), "different names → no match even with same size");
    }

    // ── stem mode (-k) ─────────────────────────────────────────────────────────

    #[test]
    fn test_fd_stem_mode_matches_different_extension() {
        let d1 = TempDir::new("fd_sk1");
        let d2 = TempDir::new("fd_sk2");
        write_file(&d1.path().join("photo.jpg"), b"same");
        write_file(&d2.path().join("photo.png"), b"diff"); // same stem "photo", same size
        let opts = CompareOptions { compare_stem: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1, "same stem + same size → match in -k mode");
    }

    #[test]
    fn test_fd_stem_mode_no_match_different_stem() {
        let d1 = TempDir::new("fd_sk3");
        let d2 = TempDir::new("fd_sk4");
        write_file(&d1.path().join("photo.jpg"),  b"same");
        write_file(&d2.path().join("backup.jpg"), b"same"); // different stem
        let opts = CompareOptions { compare_stem: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert!(dups.is_empty(), "different stems → no match in -k mode");
    }

    #[test]
    fn test_fd_stem_and_content_no_match_if_content_differs() {
        let d1 = TempDir::new("fd_sk5");
        let d2 = TempDir::new("fd_sk6");
        // Same stem + same size, but different content → with -k -c: no match.
        write_file(&d1.path().join("doc.txt"), b"v1ab");
        write_file(&d2.path().join("doc.pdf"), b"v2ab");
        let opts = CompareOptions {
            compare_stem:    true,
            compare_content: true,
            compare_size:    true,
            ..default_options()
        };
        let dups = run_find(&d1, &d2, &opts);
        assert!(dups.is_empty(),
            "same stem but different content → no match when -k combined with -c");
    }

    // ── extension mode (-e) ────────────────────────────────────────────────────

    #[test]
    fn test_fd_ext_mode_matches_same_extension() {
        let d1 = TempDir::new("fd_ex1");
        let d2 = TempDir::new("fd_ex2");
        write_file(&d1.path().join("a.jpg"), b"aaaa");
        write_file(&d2.path().join("b.jpg"), b"bbbb"); // same ext, same size
        let opts = CompareOptions { compare_ext: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1, "same extension + same size → match in -e mode");
    }

    #[test]
    fn test_fd_ext_mode_case_insensitive() {
        let d1 = TempDir::new("fd_ex3");
        let d2 = TempDir::new("fd_ex4");
        write_file(&d1.path().join("a.JPG"), b"aaaa");
        write_file(&d2.path().join("b.jpg"), b"bbbb");
        let opts = CompareOptions { compare_ext: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1, ".JPG and .jpg must match case-insensitively in -e mode");
    }

    #[test]
    fn test_fd_ext_mode_no_match_different_extension() {
        let d1 = TempDir::new("fd_ex5");
        let d2 = TempDir::new("fd_ex6");
        write_file(&d1.path().join("a.jpg"), b"aaaa");
        write_file(&d2.path().join("b.png"), b"bbbb");
        let opts = CompareOptions { compare_ext: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert!(dups.is_empty(), "different extensions → no match in -e mode");
    }

    #[test]
    fn test_fd_ext_and_stem_equals_name() {
        // Combining -e and -k is approximately equivalent to -n (full filename).
        let d1 = TempDir::new("fd_ex7");
        let d2 = TempDir::new("fd_ex8");
        write_file(&d1.path().join("photo.jpg"), b"aaaa");
        write_file(&d2.path().join("photo.png"), b"bbbb"); // same stem, different ext
        let opts = CompareOptions {
            compare_stem: true,
            compare_ext:  true,
            ..default_options()
        };
        let dups = run_find(&d1, &d2, &opts);
        assert!(dups.is_empty(),
            "with both -k and -e, stem must match AND extension must match (like -n)");
    }

    // ── intra-folder / single-folder mode (-1 / -b) ───────────────────────────

    #[test]
    fn test_fd_intra_folder_finds_internal_duplicates() {
        let d1 = TempDir::new("fd_if1");
        write_file(&d1.path().join("a.txt"), b"same content here!");
        write_file(&d1.path().join("b.txt"), b"same content here!");
        let opts = CompareOptions {
            compare_content: true,
            compare_size:    true,
            intra_folder:    true,
            ..default_options()
        };
        let dups = run_find_single(&d1, &opts);
        assert_eq!(dups.len(), 1, "intra-folder mode must find duplicates within one folder");
    }

    #[test]
    fn test_fd_intra_folder_no_match_different_content() {
        let d1 = TempDir::new("fd_if2");
        write_file(&d1.path().join("a.txt"), b"content A is here");
        write_file(&d1.path().join("b.txt"), b"content B is here");
        let opts = CompareOptions {
            compare_content: true,
            compare_size:    true,
            intra_folder:    true,
            ..default_options()
        };
        let dups = run_find_single(&d1, &opts);
        assert!(dups.is_empty(), "no intra-folder match when content differs");
    }

    // ── fuzzy mode (-Z -t N) ──────────────────────────────────────────────────

    #[test]
    fn test_fd_fuzzy_within_tolerance_found() {
        let d1 = TempDir::new("fd_fz1");
        let d2 = TempDir::new("fd_fz2");
        let ca = vec![0u8; 512];
        let mut cb = vec![0u8; 512];
        cb[10] = 1;
        cb[20] = 1; // exactly 2 bytes differ
        write_file(&d1.path().join("a.bin"), &ca);
        write_file(&d2.path().join("b.bin"), &cb);
        let opts = CompareOptions {
            compare_content: true,
            compare_size:    true,
            fuzzy:           true,
            tolerance:       5,
            ..default_options()
        };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1, "2-byte difference with tolerance=5 → fuzzy match");
        assert!(
            matches!(dups[0].match_kind, MatchKind::Fuzzy { bytes_differing: 2 }),
            "match kind must be Fuzzy with bytes_differing=2, got {:?}", dups[0].match_kind
        );
    }

    #[test]
    fn test_fd_fuzzy_exceeds_tolerance_not_found() {
        let d1 = TempDir::new("fd_fz3");
        let d2 = TempDir::new("fd_fz4");
        write_file(&d1.path().join("a.bin"), &vec![0x00u8; 512]);
        write_file(&d2.path().join("b.bin"), &vec![0xFFu8; 512]); // 512 bytes differ
        let opts = CompareOptions {
            compare_content: true,
            compare_size:    true,
            fuzzy:           true,
            tolerance:       10,
            ..default_options()
        };
        let dups = run_find(&d1, &d2, &opts);
        assert!(dups.is_empty(), "512-byte difference with tolerance=10 → no match");
    }

    #[test]
    fn test_fd_fuzzy_exact_match_not_reported_as_fuzzy() {
        // A file pair with zero byte difference must be reported as Exact,
        // not Fuzzy, even when -Z is active.
        let d1 = TempDir::new("fd_fz5");
        let d2 = TempDir::new("fd_fz6");
        write_file(&d1.path().join("a.bin"), b"identical content!!");
        write_file(&d2.path().join("b.bin"), b"identical content!!");
        let opts = CompareOptions {
            compare_content: true,
            compare_size:    true,
            fuzzy:           true,
            tolerance:       100,
            ..default_options()
        };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].match_kind, MatchKind::Exact,
            "identical files must be reported as Exact even when -Z is active");
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // Regressions
    // ═══════════════════════════════════════════════════════════════════════════

    // ── BUG: DuplicateGroup.size was the sum of all files, not one file's size ──

    #[test]
    fn test_regression_group_size_is_single_file_size_not_sum() {
        // Before the fix, sync_content_compare_with_file_progress computed
        // group_size = Σ file.size (e.g. 2 × 16 B = 32 B).
        // display_results then multiplied by folder1_files.len(), giving 2×32=64
        // instead of the correct 1×16=16.  The fix uses group[0].size.
        let d1 = TempDir::new("gs_r1");
        let d2 = TempDir::new("gs_r2");
        let content = b"exactly sixteen!"; // 16 bytes
        write_file(&d1.path().join("f.txt"), content);
        write_file(&d2.path().join("f.txt"), content);
        let dups = run_find(&d1, &d2, &content_options());
        assert_eq!(dups.len(), 1);
        assert_eq!(
            dups[0].size, 16,
            "DuplicateGroup.size must be one file's size (16 B), not the group sum (32 B)"
        );
    }

    // ── BUG: -C + -Z fuzzy candidates were always empty ───────────────────────

    #[test]
    fn test_regression_quickcheck_plus_fuzzy_catches_near_dupes() {
        // Before the fix, the fuzzy stage subtracted exact_paths from
        // quick_checked_groups (files that *passed* quick-check).  Because those
        // files all became exact matches, the result was an empty candidate set —
        // near-duplicates that *failed* quick-check were never considered.
        //
        // Fix: all three fuzzy sites now subtract from saved_name_filtered, which
        // is a snapshot of all candidates taken before any branch consumes them.
        let d1 = TempDir::new("cz_r1");
        let d2 = TempDir::new("cz_r2");
        let ca = vec![42u8; 64];
        let mut cb = vec![42u8; 64];
        cb[32] = 99; // 1 byte differs → different hash → fails quick-check gate
        write_file(&d1.path().join("a.bin"), &ca);
        write_file(&d2.path().join("b.bin"), &cb);
        let opts = CompareOptions {
            quick_content_check: true,
            fuzzy:               true,
            tolerance:           5,
            ..default_options()
        };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1,
            "near-duplicate must be found even when -C and -Z are combined");
        assert!(matches!(dups[0].match_kind, MatchKind::Fuzzy { .. }),
            "must be reported as a fuzzy match, not exact");
    }

    // ── BUG: async -1 (single-folder) found zero duplicates ───────────────────

    #[test]
    fn test_regression_async_intra_folder_finds_duplicates() {
        // Before the fix, the inner content-group guard inside the async thread
        // was a bare `has_files_from_both_folders(&content_group)` with no
        // bidirectional / intra-folder bypass.  In intra-folder mode every file
        // carries folder_index == 0, so this guard always returned false and no
        // DuplicateGroup was ever emitted — even though hashing had found
        // identical files.  Fixed to `!options.bidirectional ||
        // has_files_from_both_folders(...)`, matching the sync path.
        let d1 = TempDir::new("ai_r1");
        write_file(&d1.path().join("a.txt"), b"same content here!!");
        write_file(&d1.path().join("b.txt"), b"same content here!!");
        let opts = CompareOptions {
            compare_content: true,
            compare_size:    true,
            async_compare:   true,
            intra_folder:    true,
            ..default_options()
        };
        let dups = run_find_single(&d1, &opts);
        assert_eq!(dups.len(), 1,
            "async intra-folder mode must find duplicates (not silently skip every group)");
    }
}