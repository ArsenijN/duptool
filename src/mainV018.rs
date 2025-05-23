use clap::{Arg, ArgAction, Command};
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};
use md5::Context; // Use Context instead of Md5
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, create_dir_all, rename};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
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
    everything_name: bool,
    everything_size: bool,
    bidirectional: bool,
    async_compare: bool,
    hdd_optimized: bool, // Add flag for HDD optimization
    delete_duplicates: bool, // Add flag for deleting duplicates
    debug: bool, // Add flag for debugging
    enhanced_async: bool, // Add flag for enhanced async mode
    force_delete: bool, // Add flag for force delete mode
}

#[derive(Debug)]
struct DuplicateGroup {
    files_by_folder: Vec<Vec<PathBuf>>,
    size: u64,
}

fn main() -> io::Result<()> {
    let start_time = Instant::now();
    
    // Parse command line arguments
    let matches = Command::new("duptool")
        .version("1.8")
        .author("ArsenijN")
        .about("Finds duplicate files across directories")
        .arg(
            Arg::new("folder1")
                .required(true)
                .help("First folder to compare")
                .index(1),
        )
        .arg(
            Arg::new("folder2")
                .required(true)
                .help("Second folder to compare")
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
        .get_matches();

    let folder1 = matches.get_one::<String>("folder1").unwrap();
    let folder2 = matches.get_one::<String>("folder2").unwrap();
    
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
        hdd_optimized: !matches.get_flag("hdd_deoptimized"), // Default to HDD optimization unless -M is specified
        delete_duplicates: matches.get_flag("delete_duplicates"),
        debug: matches.get_flag("debug"),
        enhanced_async: matches.get_flag("enhanced_async"),
        force_delete: matches.get_flag("force_delete"),
    };

    if options.debug {
        println!("Debug mode enabled");
        println!("Options: {:?}", options);
        println!("Folder1: {}", folder1);
        println!("Folder2: {}", folder2);
    }

    // Default behavior: if none of the main comparison options are selected, use content comparison
    if !options.compare_content && !options.compare_name && !options.compare_size {
        options.compare_content = true;
        options.compare_size = true;  // content comparison implies size comparison
    }
    
    // If content is selected, size is implied
    if options.compare_content {
        options.compare_size = true;
    }

    println!("Scanning directories...");
    let folder1_files = collect_files(folder1, 0, options.hdd_optimized)?;
    let folder2_files = collect_files(folder2, 1, options.hdd_optimized)?;

    println!("Found {} files in {}", folder1_files.len(), folder1);
    println!("Found {} files in {}", folder2_files.len(), folder2);

    let duplicates = find_duplicates(folder1_files, folder2_files, &options)?;

    if options.delete_duplicates || options.force_delete {
        move_duplicates_to_deleted(&duplicates, folder1, folder2, &options)?;
    }

    display_results(&duplicates, folder1, folder2);
    
    println!("Completed in {:.2} seconds", start_time.elapsed().as_secs_f32());
    
    Ok(())
}

fn collect_files(root: &str, folder_index: usize, hdd_optimized: bool) -> io::Result<Vec<FileInfo>> {
    let mut files = Vec::new();
    
    let progress = ProgressBar::new_spinner();
    progress.set_message(format!("Scanning {}...", root));
    let progress_style = ProgressStyle::default_spinner()
        .template("{spinner:.green} {msg} ({pos} files found)")
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    progress.set_style(progress_style);
    
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
    options: &CompareOptions
) -> io::Result<Vec<DuplicateGroup>> {
    println!("Comparing files...");
    
    // Group files by size as a first pass
    let mut size_groups: HashMap<u64, Vec<FileInfo>> = HashMap::new();
    
    for file in folder1_files.clone() {
        size_groups.entry(file.size).or_default().push(file);
    }
    
    // Only keep size groups with potential duplicates
    let mut potential_duplicates: Vec<Vec<FileInfo>> = Vec::new();
    
    for file in folder2_files {
        if let Some(group) = size_groups.get_mut(&file.size) {
            // Add this file to the group
            group.push(file);
            
            // If this is the second file in the group, it's now a potential duplicate group
            if group.len() == 2 {
                potential_duplicates.push(group.clone());
            }
        } else if !options.bidirectional {
            // If not bidirectional, we need to consider all files
            size_groups.entry(file.size).or_default().push(file);
        }
    }
    
    // Filter out size groups with only one file
    let potential_duplicates: Vec<Vec<FileInfo>> = size_groups
        .into_values()
        .filter(|group| group.len() > 1 && 
                         (!options.bidirectional || has_files_from_both_folders(group)))
        .collect();
    
    println!("Found {} potential duplicate groups by size", potential_duplicates.len());
    
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
        println!("Found {} potential duplicate groups by name", name_filtered_groups.len());
    } else {
        name_filtered_groups = potential_duplicates;
    }

    // --- CORRECT QUICK CHECK LOGIC ---
    // - If only -C: compare only first and last 8MB, never full hash.
    // - If -C and -A/-E: first filter by quick check, then do full hash for those that match.
    // - If -A/-E without -C: always do full hash.
    // - If neither: always do full hash (sync).

    let mut quick_checked_groups = name_filtered_groups.clone();

    if options.quick_content_check && !(options.async_compare || options.enhanced_async) {
        // Only -C: compare only first and last 8MB, never full hash

        // --- Progress bar for quick check only mode ---
        let total_groups = name_filtered_groups.len();
        let total_files: u64 = name_filtered_groups.iter().map(|g| g.len() as u64).sum();

        let m = MultiProgress::new();

        // Group progress bar
        let progress_bar = m.add(ProgressBar::new(total_groups as u64));
        progress_bar.set_style(ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} groups ({eta})")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);

        // File progress bar
        let file_progress = m.add(ProgressBar::new(total_files));
        file_progress.set_style(ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/white} {pos}/{len} files ({eta})")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);

        let mut processed_files = 0u64;

        quick_checked_groups = name_filtered_groups
            .into_iter()
            .enumerate()
            .flat_map(|(idx, group)| {
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
        println!("After quick check: {} groups remain", quick_checked_groups.len());

        // Output results based on quick check only, no full hash
        let mut duplicates = Vec::new();
        for group in quick_checked_groups {
            if !group.is_empty() {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];
                let size = group[0].size;
                for file in group {
                    files_by_folder[file.folder_index].push(file.path);
                }
                duplicates.push(DuplicateGroup { files_by_folder, size });
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
        println!("After quick check: {} groups remain", quick_checked_groups.len());
    }

    // --- FULL HASH FOR ALL FILES IN GROUPS ---
    let mut duplicates = Vec::new();

    if options.compare_content {
        let total_groups = quick_checked_groups.len();
        let total_files: u64 = quick_checked_groups.iter().map(|g| g.len() as u64).sum();

        // --- Use MultiProgress for two progress bars ---
        let m = MultiProgress::new();

        // Group progress bar (per group)
        let progress_bar = m.add(ProgressBar::new(total_groups as u64));
        progress_bar.set_style(ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} groups ({eta})")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);

        // File progress bar (per file)
        let file_progress = m.add(ProgressBar::new(total_files));
        file_progress.set_style(ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/white} {pos}/{len} files ({eta})")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);

        // Run the comparison in a closure so we can join the progress bars after
        let result = if options.async_compare || options.enhanced_async {
            let mut full_hash_options = options.clone();
            full_hash_options.quick_content_check = false;
            let m = &m;
            let progress_bar = progress_bar.clone();
            let file_progress = file_progress.clone();
            std::thread::spawn(move || {
                async_content_compare_with_file_progress(
                    quick_checked_groups, full_hash_options, progress_bar, file_progress
                )
            })
        } else {
            let mut full_hash_options = options.clone();
            full_hash_options.quick_content_check = false;
            let m = &m;
            let progress_bar = progress_bar.clone();
            let file_progress = file_progress.clone();
            std::thread::spawn(move || {
                sync_content_compare_with_file_progress(
                    quick_checked_groups, &full_hash_options, progress_bar, file_progress
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
        let group_bar = m.add(ProgressBar::new(total_groups as u64));
        group_bar.set_style(ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.yellow/blue} {pos}/{len} groups (finalizing) {eta}")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);

        let file_bar = m.add(ProgressBar::new(total_files));
        file_bar.set_style(ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/white} {pos}/{len} files (finalizing) {eta}")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?);

        let mut processed_files = 0u64;
        let mut final_duplicates = Vec::new();

        for group in quick_checked_groups {
            group_bar.inc(1);
            for _ in &group {
                processed_files += 1;
                file_bar.inc(1);
            }
            if !group.is_empty() {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];
                let size = group[0].size;
                for file in group {
                    files_by_folder[file.folder_index].push(file.path);
                }
                final_duplicates.push(DuplicateGroup { files_by_folder, size });
            }
        }
        group_bar.finish();
        file_bar.finish();
        m.clear().unwrap();
        return Ok(final_duplicates);
    } else {
        // If no content comparison, just convert to duplicate groups
        for group in quick_checked_groups {
            if !group.is_empty() {
                let mut files_by_folder = vec![Vec::new(), Vec::new()];
                let size = group[0].size;
                for file in group {
                    files_by_folder[file.folder_index].push(file.path);
                }
                duplicates.push(DuplicateGroup { files_by_folder, size });
            }
        }
    }

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

fn sync_content_compare(
    groups: Vec<Vec<FileInfo>>,
    options: &CompareOptions,
    progress_bar: ProgressBar
) -> io::Result<Vec<DuplicateGroup>> {
    let mut duplicates = Vec::new();
    let mut total_size_processed: u64 = 0;
    let mut total_time_taken: f64 = 0.0;
    let mut last_eta_update = Instant::now();
    let eta_update_interval = std::time::Duration::from_millis(500); // update ETA every 0.5s

    let total_size: u64 = groups.iter().flat_map(|g| g.iter()).map(|f| f.size).sum();
    let start_time = Instant::now();

    for (index, group) in groups.iter().enumerate() {
        let group_start_time = Instant::now();
        progress_bar.inc(1);

        if group.len() <= 1 {
            continue;
        }

        let group_size: u64 = group.iter().map(|file| file.size).sum();
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
                    size: group_size 
                });
            }
        }

        // Update total time taken for group (for compatibility with old logic)
        total_time_taken += group_start_time.elapsed().as_secs_f64();
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
    let mut total_time_taken: f64 = 0.0;
    let mut last_eta_update = Instant::now();
    let eta_update_interval = std::time::Duration::from_millis(500);

    let total_size: u64 = groups.iter().flat_map(|g| g.iter()).map(|f| f.size).sum();
    let start_time = Instant::now();

    for group in &groups {
        progress_bar.inc(1);

        if group.len() <= 1 {
            continue;
        }

        let group_size: u64 = group.iter().map(|file| file.size).sum();
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
                    size: group_size 
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
            for group in chunk {
                progress.inc(1);

                if group.len() <= 1 || !has_files_from_both_folders(&group) {
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
                    // Update processed size and ETA more frequently (per file)
                    {
                        let mut sz = processed_size.lock().unwrap();
                        *sz += file.size;
                        file_progress.inc(1);
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
                    if content_group.len() > 1 && has_files_from_both_folders(&content_group) {
                        let mut files_by_folder = vec![Vec::new(), Vec::new()];

                        for file in content_group {
                            files_by_folder[file.folder_index].push(file.path);
                        }

                        local_duplicates.push(DuplicateGroup { 
                            files_by_folder, 
                            size: file_size 
                        });
                    }
                }

                if !local_duplicates.is_empty() {
                    let mut all_duplicates = duplicates.lock().unwrap();
                    all_duplicates.extend(local_duplicates);
                }
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
fn process_files_independent(
    files: Vec<FileInfo>,
    folder_index: usize,
    duplicates: Arc<Mutex<Vec<DuplicateGroup>>>,
    progress: Arc<ProgressBar>,
    options: CompareOptions,
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
                all_duplicates.push(DuplicateGroup { files_by_folder, size: group[0].size });
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

    // Use a larger buffer size for HDDs to reduce I/O operations
    let buffer_size = 64 * 1024; // 64 KB buffer

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
        let mut buffer = vec![0; buffer_size]; // Use larger buffer

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

fn split_into_chunks<T: Clone>(items: Vec<T>, chunk_count: usize, hdd_optimized: bool) -> Vec<Vec<T>> {
    let mut chunks = Vec::new();
    let max_chunks = if hdd_optimized { chunk_count.min(4) } else { chunk_count }; // Limit threads for HDDs
    let chunk_size = (items.len() + max_chunks - 1) / max_chunks.max(1);

    for chunk in items.chunks(chunk_size.max(1)) {
        chunks.push(chunk.to_vec());
    }

    chunks
}

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
                    size: file_size 
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

            // Create parent directories in the "deleted" folder
            if let Some(parent) = target_path.parent() {
                if options.debug {
                    println!("Ensuring parent directories exist for: {}", parent.display());
                }
                create_dir_all(parent)?;
            }

            // Only check for corresponding_path if not force_delete
            if !options.force_delete {
                let corresponding_path = sanitize_path(&Path::new(folder2).join(relative_path));
                if !corresponding_path.exists() {
                    if options.debug {
                        println!("File does not exist in folder2: {}", corresponding_path.display());
                    }
                    continue;
                }
            }

            // Move the file
            if let Err(e) = rename(&sanitized_file_path, &target_path) {
                if options.debug {
                    eprintln!("Error moving file: {} -> {}: {}", sanitized_file_path.display(), target_path.display(), e);
                }
                return Err(e);
            }

            println!("Moved: {}", sanitized_file_path.display());
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
    
    println!("\nFound {} duplicate groups:", duplicates.len());
    println!("{:40} : {:40} | {}", folder1, folder2, "size");
    println!("{}", "-".repeat(90));
    
    let mut total_folder1_files = 0;
    let mut total_folder2_files = 0;
    let mut total_duplicate_size_folder1 = 0;
    
    for duplicate in duplicates {
        let folder1_files = &duplicate.files_by_folder[0];
        let folder2_files = &duplicate.files_by_folder[1];
        let size_str = format_size(duplicate.size);
        
        total_folder1_files += folder1_files.len();
        total_folder2_files += folder2_files.len();
        // Sum the size of all files in folder1 for this group
        total_duplicate_size_folder1 += folder1_files.len() as u64 * duplicate.size;
        
        // Prepare the left side (folder1)
        let folder1_text = if folder1_files.is_empty() {
            String::new()
        } else {
            folder1_files
                .iter()
                .map(|p| p.strip_prefix(folder1).unwrap_or(p).to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        };
        
        // Prepare the right side (folder2)
        let folder2_text = if folder2_files.is_empty() {
            String::new()
        } else {
            folder2_files
                .iter()
                .map(|p| p.strip_prefix(folder2).unwrap_or(p).to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        };
        
        println!("{:40} : {:40} | {}", folder1_text, folder2_text, size_str);
        
        // Check if there are subdirectories
        let mut has_subdirs = false;
        for path in folder1_files.iter().chain(folder2_files.iter()) {
            if path.parent().unwrap_or(Path::new("")) != Path::new(folder1) &&
               path.parent().unwrap_or(Path::new("")) != Path::new(folder2) {
                has_subdirs = true;
                break;
            }
        }
        
        // If there are subdirectories, show a detailed view
        if has_subdirs {
            display_subdirectory_details(folder1_files, folder2_files, folder1, folder2);
        }
    }
    
    println!("{}", "-".repeat(90));
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