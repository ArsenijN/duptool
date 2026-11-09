use clap::{Arg, ArgAction, ArgMatches, Command};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use md5::Context;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{create_dir_all, rename, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Instant, SystemTime};
use walkdir::WalkDir;

/// Prints a line in normal runs; silenced completely when running under
/// `cargo test` so test output stays clean and progress-bar-free.
macro_rules! vprintln {
    ($($arg:tt)*) => {
        if !cfg!(test) { println!($($arg)*); }
    };
}

/// stderr counterpart of `vprintln!` (warnings, debug traces).
macro_rules! veprintln {
    ($($arg:tt)*) => {
        if !cfg!(test) { eprintln!($($arg)*); }
    };
}

/// Bytes read from the head and from the tail of a file in quick-check mode.
const QUICKCHECK_SIZE: u64 = 8 * 1024 * 1024; // 8MB
/// 1MB I/O buffer: a 100MB file costs ~100 read() calls instead of ~1600.
const IO_BUFFER_SIZE: usize = 1024 * 1024;
/// Subfolder of folder1 that receives exact duplicates (-D/-F).
const DELETED_DIR: &str = "deleted";
/// Subfolder of folder1 that receives near-duplicates (-Z/-Y without -U).
const DIFFER_DIR: &str = "differ";

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
    // -B: "only between folders" is the default behaviour; the flag is kept
    // for compatibility and so clap can reject it together with -b/-1.
    #[allow(dead_code)]
    bidirectional: bool,
    async_compare: bool,
    hdd_optimized: bool,
    delete_duplicates: bool,
    debug: bool,
    enhanced_async: bool,
    force_delete: bool,
    intra_folder: bool,
    // Fuzzy comparison options
    fuzzy: bool,                   // -Z: enable fuzzy mode
    tolerance: Option<u64>,        // -t N: max differing bytes allowed
    tolerance_pct: Option<f64>,    // -T P: max differing bytes as % of file size
    fuzzy_as_dupes: bool,          // -U: treat fuzzy/metadata matches as exact duplicates
    skip_metadata: bool,           // -Y: fuzzy mode 2, compare media payload only
    // Partial name comparison options
    compare_stem: bool,            // -k: compare by file stem (filename without extension)
    compare_ext: bool,             // -e: compare by file extension only
}

impl CompareOptions {
    /// Whether candidates must have the same size.
    ///
    /// Size is the cheap first-pass key for every content-based mode. When the
    /// user asks to match by *name* only (`-n` and/or `-k` with no `-s`, `-c`,
    /// `-C` or `-Z`), requiring equal sizes would silently hide every renamed
    /// or re-encoded copy, which is exactly what those modes are meant to find.
    /// `-e` alone keeps size grouping: "same extension" by itself is not a
    /// meaningful duplicate signal.
    fn uses_size_grouping(&self) -> bool {
        let name_keyed = self.compare_name || self.compare_stem;
        !name_keyed
            || self.compare_size
            || self.compare_content
            || self.quick_content_check
            || self.fuzzy
    }

    fn uses_parallel_hashing(&self) -> bool {
        self.async_compare || self.enhanced_async
    }

    /// The largest number of differing bytes accepted for a fuzzy pair of
    /// files of `size` bytes. With both `-t` and `-T` the stricter one wins.
    fn fuzzy_limit(&self, size: u64) -> u64 {
        let by_bytes = self.tolerance;
        let by_pct = self
            .tolerance_pct
            .map(|pct| ((size as f64) * pct / 100.0).floor() as u64);
        match (by_bytes, by_pct) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => 0,
        }
    }
}

/// How a duplicate group was matched.
#[derive(Debug, Clone, PartialEq)]
enum MatchKind {
    Exact,
    Fuzzy { bytes_differing: u64 },
    /// Same media payload, different metadata (fuzzy mode 2, `-Y`).
    MetadataOnly,
}

impl MatchKind {
    fn label(&self) -> String {
        match self {
            MatchKind::Exact => "EXACT".to_string(),
            MatchKind::Fuzzy { bytes_differing } => format!("FUZZY ~{}B", bytes_differing),
            MatchKind::MetadataOnly => "META".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct DuplicateGroup {
    files_by_folder: Vec<Vec<PathBuf>>,
    /// Size of the largest file in the group (all files when `uniform_size`).
    size: u64,
    /// False when the group mixes sizes (name-only matches, `-Y` matches).
    uniform_size: bool,
    /// Sum of the sizes of the folder1 files in this group.
    folder1_bytes: u64,
    match_kind: MatchKind,
}

/// Build a `DuplicateGroup` from matched files.
fn make_group(files: &[FileInfo], match_kind: MatchKind) -> DuplicateGroup {
    let mut files_by_folder = vec![Vec::new(), Vec::new()];
    let mut folder1_bytes = 0;
    for f in files {
        files_by_folder[f.folder_index].push(f.path.clone());
        if f.folder_index == 0 {
            folder1_bytes += f.size;
        }
    }
    let size = files.iter().map(|f| f.size).max().unwrap_or(0);
    let uniform_size = files.iter().all(|f| f.size == size);
    DuplicateGroup { files_by_folder, size, uniform_size, folder1_bytes, match_kind }
}

/// How the result list is ordered.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SortKey {
    Path,
    Size,
    Mtime,
}

#[derive(Debug)]
struct RunConfig {
    folder1: String,
    folder2: Option<String>,
    options: CompareOptions,
    sort_key: SortKey,
    sort_reverse: bool,
}

fn build_cli() -> Command {
    Command::new("duptool")
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("Advanced duplicate file finder with smart comparison strategies")
        .long_about(concat!(
            "duptool - Advanced duplicate file finder\n\n",
            "Fast, efficient tool for finding duplicate files across directories.\n",
            "Features: async processing, HDD optimization, quick content check,\n",
            "fuzzy and metadata-aware matching, safe file deletion, and\n",
            "cross-device move support.\n\n",
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
                .required_unless_present_any(["version", "verbose_version"])
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
                .help("Compare by file name (sizes may differ unless -s/-c is also given)")
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
                .help("Fast name comparison using Everything (reserved, not implemented yet)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("everything_size")
                .short('S')
                .long("everything-size")
                .help("Fast size comparison using Everything (reserved, not implemented yet)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("bidirectional")
                .short('B')
                .long("bidirectional")
                .help("Only compare files between folders, not within (the default)")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["intra", "single"]),
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
                .help("Hash in parallel: one reader per folder with -m (default), all CPUs with -M")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("hdd_optimized")
                .short('m')
                .long("hdd")
                .help("Optimize for HDD usage: sequential, path-ordered reads (default)")
                .action(ArgAction::SetTrue)
                .conflicts_with("hdd_deoptimized"),
        )
        .arg(
            Arg::new("hdd_deoptimized")
                .short('M')
                .long("no-hdd")
                .help("Deoptimize for HDD usage (SSD: many concurrent readers)")
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
                .help("Enhanced async mode: one reader thread per folder, even with -M")
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
                .help("Enable fuzzy comparison: files within the tolerance are near-duplicates, moved to 'differ/' folder")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("tolerance")
                .short('t')
                .long("tolerance")
                .help("Byte threshold for fuzzy comparison (-Z needs -t and/or -T; specify separately: -Z -t 50)")
                .value_name("BYTES")
                .value_parser(clap::value_parser!(u64)),
        )
        .arg(
            Arg::new("tolerance_pct")
                .short('T')
                .long("tolerance-percent")
                .help("Fuzzy threshold as a percentage of file size (0-100); with -t, the stricter limit wins")
                .value_name("PERCENT")
                .value_parser(clap::value_parser!(f64)),
        )
        .arg(
            Arg::new("fuzzy_as_dupes")
                .short('U')
                .long("fuzzy-as-dupes")
                .help("Treat fuzzy/metadata matches as exact duplicates (requires -Z or -Y); eligible for -D/-F deletion")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("skip_metadata")
                .short('Y')
                .long("skip-metadata")
                .help("Fuzzy mode 2: match JPEG/MP4/MOV files whose media payload is identical but metadata (EXIF, atoms) differs")
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
        .arg(
            Arg::new("sort_size")
                .long("sort-size")
                .help("Sort results by file size (smallest first)")
                .action(ArgAction::SetTrue)
                .conflicts_with("sort_mtime"),
        )
        .arg(
            Arg::new("sort_mtime")
                .long("sort-mtime")
                .help("Sort results by modification time of each group's first file (oldest first)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("sort_reverse")
                .long("sort-reverse")
                .help("Reverse the result order")
                .action(ArgAction::SetTrue),
        )
}

/// cmd.exe turns `"D:\Music\"` into `D:\Music"` (the `\"` escapes the quote),
/// which then matches nothing. Strip that stray quote on Windows.
fn clean_folder_arg(arg: &str) -> String {
    if cfg!(windows) {
        arg.trim_end_matches('"').to_string()
    } else {
        arg.to_string()
    }
}

/// Turn parsed CLI arguments into a validated run configuration.
/// Returns an error message for invalid combinations.
fn config_from_matches(matches: &ArgMatches) -> Result<RunConfig, String> {
    let single_mode = matches.get_flag("single");
    let folder1 = clean_folder_arg(
        matches
            .get_one::<String>("folder1")
            .ok_or("the first folder is required")?,
    );
    let folder2 = if single_mode {
        if matches.get_one::<String>("folder2").is_some() {
            veprintln!("Warning: second folder is ignored in -1/--single mode.");
        }
        None
    } else {
        Some(clean_folder_arg(
            matches
                .get_one::<String>("folder2")
                .ok_or("a second folder is required unless -1/--single is used")?,
        ))
    };

    let fuzzy = matches.get_flag("fuzzy");
    let tolerance = matches.get_one::<u64>("tolerance").copied();
    let tolerance_pct = matches.get_one::<f64>("tolerance_pct").copied();
    let fuzzy_as_dupes = matches.get_flag("fuzzy_as_dupes");
    let skip_metadata = matches.get_flag("skip_metadata");

    if let Some(pct) = tolerance_pct {
        if !pct.is_finite() || !(0.0..=100.0).contains(&pct) {
            return Err(format!("-T/--tolerance-percent must be between 0 and 100, got {}", pct));
        }
    }
    if fuzzy && tolerance.is_none() && tolerance_pct.is_none() {
        return Err(
            "-Z/--fuzzy requires -t/--tolerance and/or -T/--tolerance-percent.\n\
             Example: duptool folder1 folder2 -Z -t 50"
                .to_string(),
        );
    }
    if !fuzzy && (tolerance.is_some() || tolerance_pct.is_some()) {
        veprintln!("Warning: -t/-T have no effect without -Z/--fuzzy.");
    }
    if fuzzy_as_dupes && !fuzzy && !skip_metadata {
        return Err("-U/--fuzzy-as-dupes requires -Z/--fuzzy or -Y/--skip-metadata.".to_string());
    }

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
        tolerance,
        tolerance_pct,
        fuzzy_as_dupes,
        skip_metadata,
        compare_stem: matches.get_flag("stem"),
        compare_ext: matches.get_flag("ext"),
    };

    if options.everything_name || options.everything_size {
        veprintln!("Warning: -N/-S (Everything integration) are not implemented yet and are ignored.");
    }

    // Default behavior: if none of the main comparison options are selected, use content comparison
    if !options.compare_content && !options.compare_name && !options.compare_size
        && !options.compare_stem && !options.compare_ext
    {
        options.compare_content = true;
    }
    // Content comparison implies size comparison.
    if options.compare_content {
        options.compare_size = true;
    }

    let sort_key = if matches.get_flag("sort_size") {
        SortKey::Size
    } else if matches.get_flag("sort_mtime") {
        SortKey::Mtime
    } else {
        SortKey::Path
    };

    Ok(RunConfig {
        folder1,
        folder2,
        options,
        sort_key,
        sort_reverse: matches.get_flag("sort_reverse"),
    })
}

/// Refuse folder pairs that would make a file be compared with itself:
/// `duptool a a` or a folder nested in the other. With -D/-F such a "match"
/// moves away the only copy of the file.
fn validate_folders(folder1: &str, folder2: Option<&str>) -> Result<(), String> {
    let canon = |f: &str| -> Result<PathBuf, String> {
        let p = Path::new(f);
        if !p.is_dir() {
            return Err(format!("'{}' is not a directory", f));
        }
        p.canonicalize().map_err(|e| format!("cannot resolve '{}': {}", f, e))
    };
    let c1 = canon(folder1)?;
    if let Some(folder2) = folder2 {
        let c2 = canon(folder2)?;
        if c1 == c2 {
            return Err(format!(
                "'{}' and '{}' are the same folder; use -1/--single to search one folder",
                folder1, folder2
            ));
        }
        if c1.starts_with(&c2) || c2.starts_with(&c1) {
            return Err(format!(
                "'{}' and '{}' are nested; files would be compared with themselves",
                folder1, folder2
            ));
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let start_time = Instant::now();
    let matches = build_cli().get_matches();

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

    let config = config_from_matches(&matches).unwrap_or_else(|msg| {
        eprintln!("Error: {}", msg);
        std::process::exit(1);
    });
    if let Err(msg) = validate_folders(&config.folder1, config.folder2.as_deref()) {
        eprintln!("Error: {}", msg);
        std::process::exit(1);
    }

    let options = &config.options;
    let folder1 = config.folder1.as_str();
    let folder2 = config.folder2.as_deref().unwrap_or("");

    if options.debug {
        println!("Debug mode enabled");
        println!("Options: {:?}", options);
        println!("Folder1: {}", folder1);
        if config.folder2.is_some() {
            println!("Folder2: {}", folder2);
        }
    }

    println!("Scanning directories...");
    let folder1_files = collect_files(folder1, 0, options.hdd_optimized)?;
    let folder2_files = match &config.folder2 {
        Some(f2) => collect_files(f2, 1, options.hdd_optimized)?,
        None => Vec::new(),
    };

    println!("Found {} files in {}", folder1_files.len(), folder1);
    if config.folder2.is_some() {
        println!("Found {} files in {}", folder2_files.len(), folder2);
    }

    let mut duplicates = find_duplicates(folder1_files, folder2_files, folder1, folder2, options)?;
    // Sort before moving anything: --sort-mtime needs the files in place.
    sort_groups(&mut duplicates, config.sort_key, config.sort_reverse);

    if options.delete_duplicates || options.force_delete {
        if options.fuzzy_as_dupes {
            // -U: fuzzy and metadata-only matches are handled like exact ones.
            let all: Vec<&DuplicateGroup> = duplicates.iter().collect();
            move_groups(&all, folder1, folder2, DELETED_DIR, options)?;
        } else {
            let (exact, near): (Vec<&DuplicateGroup>, Vec<&DuplicateGroup>) = duplicates
                .iter()
                .partition(|g| g.match_kind == MatchKind::Exact);
            move_groups(&exact, folder1, folder2, DELETED_DIR, options)?;
            move_groups(&near, folder1, folder2, DIFFER_DIR, options)?;
        }
    }

    display_results(&duplicates, folder1, folder2, exact_label(options));

    println!("Completed in {:.2} seconds", start_time.elapsed().as_secs_f32());

    Ok(())
}

fn is_output_dir_name(name: &std::ffi::OsStr) -> bool {
    name == DELETED_DIR || name == DIFFER_DIR
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
            .map_err(io::Error::other)?;
        pb.set_style(style);
        pb
    };

    // folder1/deleted and folder1/differ are this tool's own output. Scanning
    // them again on the next run would match them against folder2 and move
    // them into deleted/deleted/...
    let skip_output_dirs = folder_index == 0;
    let walker = WalkDir::new(root).into_iter().filter_entry(move |e| {
        !(skip_output_dirs
            && e.depth() == 1
            && e.file_type().is_dir()
            && is_output_dir_name(e.file_name()))
    });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                progress.suspend(|| veprintln!("Warning: skipping unreadable entry: {}", err));
                continue;
            }
        };
        if entry.file_type().is_file() {
            match entry.metadata() {
                Ok(metadata) => {
                    files.push(FileInfo {
                        path: entry.path().to_path_buf(),
                        size: metadata.len(),
                        folder_index,
                    });
                    progress.inc(1);
                }
                Err(err) => {
                    progress.suspend(|| veprintln!("Warning: cannot stat {}: {}", entry.path().display(), err));
                }
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

/// A group is worth reporting when it has at least two files and, unless
/// intra-folder search (-b/-1) is on, spans both folders.
fn keep_group(files: &[FileInfo], options: &CompareOptions) -> bool {
    files.len() > 1 && (options.intra_folder || has_files_from_both_folders(files))
}

/// The cheap, metadata-only identity of a file used to form candidate groups.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    size: Option<u64>,
    name: Option<OsString>,
    stem: Option<OsString>,
    ext: Option<String>,
}

fn group_key(file: &FileInfo, options: &CompareOptions, use_size: bool) -> GroupKey {
    GroupKey {
        size: use_size.then_some(file.size),
        // -n: full file name, case-sensitive.
        name: options
            .compare_name
            .then(|| file.path.file_name().unwrap_or_default().to_os_string()),
        // -k: file name without the last extension, case-sensitive.
        // "photo.jpg"/"photo.png" -> "photo"; "a.tar.gz" -> "a.tar".
        stem: options
            .compare_stem
            .then(|| file.path.file_stem().unwrap_or_default().to_os_string()),
        // -e: last extension, case-insensitive; no extension -> "".
        ext: options.compare_ext.then(|| {
            file.path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        }),
    }
}

/// Group files by their `GroupKey`. Groups and the files inside them are
/// sorted by path so results are deterministic. No filtering is applied.
fn group_by_key(files: Vec<FileInfo>, options: &CompareOptions, use_size: bool) -> Vec<Vec<FileInfo>> {
    let mut map: HashMap<GroupKey, Vec<FileInfo>> = HashMap::new();
    for file in files {
        map.entry(group_key(&file, options, use_size)).or_default().push(file);
    }
    let mut groups: Vec<Vec<FileInfo>> = map.into_values().collect();
    for g in &mut groups {
        g.sort_by(|a, b| a.path.cmp(&b.path));
    }
    groups.sort_by(|a, b| a[0].path.cmp(&b[0].path));
    groups
}

/// Candidate groups: grouped by key and filtered with `keep_group`.
fn group_candidates(files: Vec<FileInfo>, options: &CompareOptions, use_size: bool) -> Vec<Vec<FileInfo>> {
    group_by_key(files, options, use_size)
        .into_iter()
        .filter(|g| keep_group(g, options))
        .collect()
}

/// For -D: does a folder1 file have *some* file at the same relative path in
/// folder2? Used only as a cheap pre-filter; the move step checks that this
/// counterpart is really part of the matched group.
fn has_same_path_counterpart(file: &FileInfo, folder1: &str, folder2: &str) -> bool {
    file.folder_index == 0
        && file
            .path
            .strip_prefix(folder1)
            .map(|rel| Path::new(folder2).join(rel).exists())
            .unwrap_or(false)
}

fn collect_paths(groups: &[DuplicateGroup]) -> impl Iterator<Item = PathBuf> + '_ {
    groups
        .iter()
        .flat_map(|g| g.files_by_folder.iter().flat_map(|v| v.iter().cloned()))
}

fn find_duplicates(
    folder1_files: Vec<FileInfo>,
    folder2_files: Vec<FileInfo>,
    folder1: &str,
    folder2: &str,
    options: &CompareOptions,
) -> io::Result<Vec<DuplicateGroup>> {
    vprintln!("Comparing files...");

    // Consume the input vectors instead of cloning them. Only -Y needs the
    // full file list again later (its matches may differ in size).
    let all_files: Vec<FileInfo> = folder1_files.into_iter().chain(folder2_files).collect();
    let metadata_pool = if options.skip_metadata { Some(all_files.clone()) } else { None };

    let use_size = options.uses_size_grouping();
    let mut candidates = group_candidates(all_files, options, use_size);
    vprintln!(
        "Found {} potential duplicate groups by {}",
        candidates.len(),
        describe_grouping(options, use_size)
    );

    // -D pre-filter: with delete_duplicates (but not force_delete) we only care about
    // files that have a counterpart at the *same relative path* in folder2. Any group
    // that can never produce such a pair is useless to compare — drop it early.
    // Not applied to intra-folder search, whose groups need no folder2 counterpart.
    if options.delete_duplicates && !options.force_delete && !options.intra_folder && !folder2.is_empty() {
        candidates.retain(|g| g.iter().any(|f| has_same_path_counterpart(f, folder1, folder2)));
        vprintln!("{} groups have a same-path counterpart for -D", candidates.len());
    }

    // Fuzzy candidates are all candidates that do not end up as exact matches,
    // i.e. the files that *failed* the quick check or the full hash.
    let fuzzy_pool = if options.fuzzy { Some(candidates.clone()) } else { None };

    // Content stages:
    // - -C without -A/-E: quick check (first and last 8MB) only, never a full hash.
    // - -C with -A/-E: quick check as a filter, then full hash for files it did not
    //   fully cover.
    // - content without -C: full hash.
    // - no content and no -C: the key groups are the result.
    let run_quick = options.quick_content_check;
    let run_full = options.compare_content && (!run_quick || options.uses_parallel_hashing());

    let mut exact_groups = candidates;
    if run_quick {
        exact_groups = refine_groups_by_hash(exact_groups, HashMode::Quick, options, "quick")?;
        vprintln!("After quick check: {} groups remain", exact_groups.len());
    }
    if run_full {
        // A quick hash of a file up to 2×8MB already covers the whole file.
        let (covered, need_full): (Vec<_>, Vec<_>) = exact_groups
            .into_iter()
            .partition(|g| run_quick && g[0].size <= QUICKCHECK_SIZE * 2);
        exact_groups = refine_groups_by_hash(need_full, HashMode::Full, options, "hash")?;
        exact_groups.extend(covered);
    }

    let mut duplicates: Vec<DuplicateGroup> = exact_groups
        .iter()
        .map(|g| make_group(g, MatchKind::Exact))
        .collect();
    let mut matched: HashSet<PathBuf> = collect_paths(&duplicates).collect();

    // --- FUZZY STAGE (-Z) ---
    if let Some(pool) = fuzzy_pool {
        let fuzzy_candidates: Vec<Vec<FileInfo>> = pool
            .into_iter()
            .map(|g| g.into_iter().filter(|f| !matched.contains(&f.path)).collect::<Vec<_>>())
            .filter(|g| keep_group(g, options))
            .collect();
        if !fuzzy_candidates.is_empty() {
            vprintln!("Running fuzzy comparison on {} candidate groups...", fuzzy_candidates.len());
            let fuzzy_results = fuzzy_compare_groups(&fuzzy_candidates, options)?;
            vprintln!("Fuzzy comparison found {} near-duplicate groups", fuzzy_results.len());
            matched.extend(collect_paths(&fuzzy_results));
            duplicates.extend(fuzzy_results);
        }
    }

    // --- METADATA-SKIP STAGE (-Y, fuzzy mode 2) ---
    if let Some(pool) = metadata_pool {
        let remaining: Vec<FileInfo> = pool
            .into_iter()
            .filter(|f| !matched.contains(&f.path) && media_kind_for(&f.path).is_some())
            .collect();
        // Stripping metadata changes the size, so sizes are never part of this key.
        let groups = group_candidates(remaining, options, false);
        if !groups.is_empty() {
            vprintln!("Running metadata-aware comparison on {} candidate groups...", groups.len());
            let meta_results = metadata_skip_compare(groups, options)?;
            vprintln!("Metadata-aware comparison found {} groups", meta_results.len());
            duplicates.extend(meta_results);
        }
    }

    // Deterministic default order; main() re-sorts according to --sort-*.
    sort_groups(&mut duplicates, SortKey::Path, false);

    Ok(duplicates)
}

fn describe_grouping(options: &CompareOptions, use_size: bool) -> String {
    let mut parts = Vec::new();
    if use_size { parts.push("size"); }
    if options.compare_name { parts.push("name"); }
    if options.compare_stem { parts.push("stem"); }
    if options.compare_ext { parts.push("extension"); }
    if parts.is_empty() { "nothing".to_string() } else { parts.join(" + ") }
}

// ══════════════════════════════════════════════════════════════════════════════
// Hashing
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq)]
enum HashMode {
    /// First and last 8MB only (whole file when it is ≤ 16MB).
    Quick,
    Full,
}

/// How hashing work is spread across threads.
#[derive(Debug, Clone, PartialEq)]
enum HashStrategy {
    /// One thread, path order.
    Sequential,
    /// One reader thread per folder, each in path order. On HDDs this keeps
    /// every disk streaming sequentially instead of seeking between several
    /// concurrent readers, while two folders on two disks still read in
    /// parallel.
    PerFolder,
    /// N threads over all files (SSD / -M).
    Pool(usize),
}

fn hash_strategy(options: &CompareOptions) -> HashStrategy {
    if !options.uses_parallel_hashing() {
        HashStrategy::Sequential
    } else if options.enhanced_async || options.hdd_optimized {
        HashStrategy::PerFolder
    } else {
        HashStrategy::Pool(num_cpus::get().max(1))
    }
}

/// Two progress bars: files hashed and bytes read (with rate and ETA).
struct HashProgress {
    multi: MultiProgress,
    files: ProgressBar,
    bytes: ProgressBar,
}

impl HashProgress {
    fn new(label: &str, total_files: u64, total_bytes: u64) -> Self {
        let multi = MultiProgress::new();
        if cfg!(test) {
            let files = multi.add(ProgressBar::hidden());
            let bytes = multi.add(ProgressBar::hidden());
            return HashProgress { multi, files, bytes };
        }
        let style = |template: &str| {
            ProgressStyle::default_bar()
                .template(template)
                .unwrap_or_else(|_| ProgressStyle::default_bar())
        };
        let files = multi.add(ProgressBar::new(total_files));
        files.set_style(style("{prefix:>5} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files"));
        files.set_prefix(label.to_string());
        let bytes = multi.add(ProgressBar::new(total_bytes));
        bytes.set_style(style(
            "      [{eta_precise} left] {bar:40.green/white} {binary_bytes}/{binary_total_bytes} ({binary_bytes_per_sec})",
        ));
        HashProgress { multi, files, bytes }
    }

    fn advance(&self, bytes: u64) {
        self.files.inc(1);
        self.bytes.inc(bytes);
    }

    fn warn(&self, msg: &str) {
        self.multi.suspend(|| veprintln!("{}", msg));
    }

    fn finish(&self) {
        self.files.finish_and_clear();
        self.bytes.finish_and_clear();
    }
}

fn bytes_read_for(file: &FileInfo, mode: HashMode) -> u64 {
    match mode {
        HashMode::Quick if file.size > QUICKCHECK_SIZE * 2 => QUICKCHECK_SIZE * 2,
        _ => file.size,
    }
}

type HashJob<'a> = (usize, usize, &'a FileInfo);

fn run_hash_jobs(jobs: Vec<HashJob<'_>>, mode: HashMode, progress: &HashProgress) -> Vec<(usize, usize, Option<String>)> {
    jobs.into_iter()
        .map(|(gi, fi, file)| {
            let hash = match calculate_file_hash(file, mode == HashMode::Quick) {
                Ok(h) => Some(h),
                Err(e) => {
                    // One unreadable file must not abort the whole run.
                    progress.warn(&format!("Warning: cannot hash {}: {}", file.path.display(), e));
                    None
                }
            };
            progress.advance(bytes_read_for(file, mode));
            (gi, fi, hash)
        })
        .collect()
}

/// Hash every file in `groups` and return the hashes in the same shape.
/// Files that cannot be read get `None`.
fn hash_all(
    groups: &[Vec<FileInfo>],
    mode: HashMode,
    options: &CompareOptions,
    label: &str,
) -> Vec<Vec<Option<String>>> {
    let mut results: Vec<Vec<Option<String>>> = groups.iter().map(|g| vec![None; g.len()]).collect();

    let mut jobs: Vec<HashJob<'_>> = groups
        .iter()
        .enumerate()
        .flat_map(|(gi, g)| g.iter().enumerate().map(move |(fi, f)| (gi, fi, f)))
        .collect();
    if jobs.is_empty() {
        return results;
    }
    if options.hdd_optimized {
        // Groups come out of a hash map in arbitrary order; reading in path
        // order keeps the disk head moving forward.
        jobs.sort_by(|a, b| a.2.path.cmp(&b.2.path));
    }

    let total_bytes: u64 = jobs.iter().map(|j| bytes_read_for(j.2, mode)).sum();
    let progress = HashProgress::new(label, jobs.len() as u64, total_bytes);

    let strategy = hash_strategy(options);
    let worker_lists: Vec<Vec<HashJob<'_>>> = match strategy {
        HashStrategy::Sequential => vec![jobs],
        HashStrategy::PerFolder => {
            let (f0, f1): (Vec<_>, Vec<_>) = jobs.into_iter().partition(|j| j.2.folder_index == 0);
            [f0, f1].into_iter().filter(|l| !l.is_empty()).collect()
        }
        HashStrategy::Pool(n) => {
            let n = n.min(jobs.len()).max(1);
            let mut lists: Vec<Vec<HashJob<'_>>> = (0..n).map(|_| Vec::new()).collect();
            for (i, job) in jobs.into_iter().enumerate() {
                lists[i % n].push(job);
            }
            lists
        }
    };
    if options.debug {
        progress.warn(&format!(
            "[HASH] {:?} mode with {:?}: {} worker(s)",
            mode, strategy, worker_lists.len()
        ));
    }

    let outputs: Vec<Vec<(usize, usize, Option<String>)>> = if worker_lists.len() <= 1 {
        worker_lists.into_iter().map(|l| run_hash_jobs(l, mode, &progress)).collect()
    } else {
        thread::scope(|s| {
            let progress = &progress;
            let handles: Vec<_> = worker_lists
                .into_iter()
                .map(|l| s.spawn(move || run_hash_jobs(l, mode, progress)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("hash worker thread panicked"))
                .collect()
        })
    };
    progress.finish();

    for (gi, fi, hash) in outputs.into_iter().flatten() {
        results[gi][fi] = hash;
    }
    results
}

/// Split every group by content hash; keep only sub-groups that still qualify.
fn refine_groups_by_hash(
    groups: Vec<Vec<FileInfo>>,
    mode: HashMode,
    options: &CompareOptions,
    label: &str,
) -> io::Result<Vec<Vec<FileInfo>>> {
    let hashes = hash_all(&groups, mode, options, label);
    let mut out = Vec::new();
    for (group, group_hashes) in groups.into_iter().zip(hashes) {
        let mut by_hash: HashMap<String, Vec<FileInfo>> = HashMap::new();
        for (file, hash) in group.into_iter().zip(group_hashes) {
            if let Some(h) = hash {
                by_hash.entry(h).or_default().push(file);
            }
        }
        let mut sub: Vec<Vec<FileInfo>> = by_hash
            .into_values()
            .filter(|g| keep_group(g, options))
            .collect();
        sub.sort_by(|a, b| a[0].path.cmp(&b[0].path));
        out.extend(sub);
    }
    Ok(out)
}

/// Like `Read::read_exact`, but stops at EOF and returns how much was read.
/// A single `read()` may legally return fewer bytes than requested, so code
/// that needs a full buffer (quick check, chunk-aligned fuzzy diff) must loop.
fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Feed `len` bytes starting at `offset` into `hasher`.
fn hash_range<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    len: u64,
    hasher: &mut Context,
    buf: &mut [u8],
) -> io::Result<()> {
    r.seek(SeekFrom::Start(offset))?;
    let mut remaining = len;
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = read_full(r, &mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "file is shorter than expected (changed during the scan?)",
            ));
        }
        hasher.consume(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(())
}

fn calculate_file_hash(file: &FileInfo, quick_check: bool) -> io::Result<String> {
    let mut file_handle = File::open(&file.path)?;
    let mut hasher = Context::new();
    let mut buffer = vec![0u8; IO_BUFFER_SIZE];

    if quick_check && file.size > QUICKCHECK_SIZE * 2 {
        hash_range(&mut file_handle, 0, QUICKCHECK_SIZE, &mut hasher, &mut buffer)?;
        hash_range(
            &mut file_handle,
            file.size - QUICKCHECK_SIZE,
            QUICKCHECK_SIZE,
            &mut hasher,
            &mut buffer,
        )?;
    } else {
        loop {
            let bytes_read = match file_handle.read(&mut buffer) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if bytes_read == 0 {
                break;
            }
            hasher.consume(&buffer[..bytes_read]);
        }
    }

    Ok(format!("{:x}", hasher.compute()))
}

// ══════════════════════════════════════════════════════════════════════════════
// Fuzzy mode 1 (-Z): byte-diff counting
// ══════════════════════════════════════════════════════════════════════════════

/// Compare two files chunk by chunk, counting differing bytes. Never loads a
/// whole file into RAM (1MB rolling buffers). Stops reading as soon as the
/// count exceeds `limit`, so the returned value is exact only when it is
/// `<= limit`. Files of different length return `u64::MAX`.
fn chunked_fuzzy_compare(path_a: &Path, path_b: &Path, limit: u64) -> io::Result<u64> {
    let mut fa = File::open(path_a)?;
    let mut fb = File::open(path_b)?;

    let mut buf_a = vec![0u8; IO_BUFFER_SIZE];
    let mut buf_b = vec![0u8; IO_BUFFER_SIZE];
    let mut total_diff: u64 = 0;

    loop {
        let na = read_full(&mut fa, &mut buf_a)?;
        let nb = read_full(&mut fb, &mut buf_b)?;
        if na != nb {
            return Ok(u64::MAX);
        }
        if na == 0 {
            break;
        }

        total_diff += buf_a[..na]
            .iter()
            .zip(&buf_b[..nb])
            .filter(|(a, b)| a != b)
            .count() as u64;
        if total_diff > limit {
            break;
        }
    }

    Ok(total_diff)
}

/// Compare every eligible pair of same-size files in each candidate group.
/// Without intra-folder search only cross-folder pairs are compared.
fn fuzzy_compare_groups(
    groups: &[Vec<FileInfo>],
    options: &CompareOptions,
) -> io::Result<Vec<DuplicateGroup>> {
    let mut results = Vec::new();

    for group in groups {
        for (i, a) in group.iter().enumerate() {
            for b in &group[i + 1..] {
                if a.size != b.size || (a.folder_index == b.folder_index && !options.intra_folder) {
                    continue;
                }
                let limit = options.fuzzy_limit(a.size);
                let diff = match chunked_fuzzy_compare(&a.path, &b.path, limit) {
                    Ok(d) => d,
                    Err(e) => {
                        if options.debug {
                            eprintln!(
                                "[FUZZY] Error comparing {} <-> {}: {}",
                                a.path.display(), b.path.display(), e
                            );
                        }
                        continue;
                    }
                };

                if diff <= limit {
                    if options.debug {
                        println!(
                            "[FUZZY MATCH] {} bytes differ (≤{} threshold)\n  {} <-> {}",
                            diff, limit, a.path.display(), b.path.display()
                        );
                    }
                    results.push(make_group(
                        &[a.clone(), b.clone()],
                        MatchKind::Fuzzy { bytes_differing: diff },
                    ));
                } else if options.debug {
                    println!(
                        "[FUZZY EXCEED] more than {} bytes differ — skipped\n  {} <-> {}",
                        limit, a.path.display(), b.path.display()
                    );
                }
            }
        }
    }

    Ok(results)
}

// ══════════════════════════════════════════════════════════════════════════════
// Fuzzy mode 2 (-Y): format-aware metadata skip
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MediaKind {
    Jpeg,
    /// MP4 / MOV / 3GP / M4A (ISO base media file format, QuickTime).
    IsoBmff,
}

fn media_kind_for(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => Some(MediaKind::Jpeg),
        "mp4" | "m4v" | "mov" | "3gp" | "3g2" | "m4a" => Some(MediaKind::IsoBmff),
        _ => None,
    }
}

/// Read exactly `buf.len()` bytes at `pos`; `false` if the file is too short.
fn read_at<R: Read + Seek>(r: &mut R, pos: u64, buf: &mut [u8]) -> io::Result<bool> {
    r.seek(SeekFrom::Start(pos))?;
    Ok(read_full(r, buf)? == buf.len())
}

fn push_range(ranges: &mut Vec<(u64, u64)>, start: u64, end: u64) {
    if end > start {
        ranges.push((start, end - start));
    }
}

/// Byte ranges of a JPEG that are *not* metadata: everything except the
/// APP0–APP15 (EXIF, XMP, ICC, JFIF, …) and COM segments that precede the
/// image scan. From SOS (start of scan) on, the file is kept as-is.
/// Returns `None` if the data is not a well-formed JPEG header.
fn jpeg_payload_ranges<R: Read + Seek>(r: &mut R, size: u64) -> io::Result<Option<Vec<(u64, u64)>>> {
    let mut soi = [0u8; 2];
    if !read_at(r, 0, &mut soi)? || soi != [0xFF, 0xD8] {
        return Ok(None);
    }
    let mut ranges = Vec::new();
    let mut keep_start = 0u64;
    let mut pos = 2u64;
    loop {
        let mut marker = [0u8; 2];
        if !read_at(r, pos, &mut marker)? || marker[0] != 0xFF {
            return Ok(None);
        }
        match marker[1] {
            // Fill byte before a marker.
            0xFF => {
                pos += 1;
                continue;
            }
            // SOS / EOI: the rest is image data (and possibly trailers).
            0xDA | 0xD9 => {
                push_range(&mut ranges, keep_start, size);
                return Ok(Some(ranges));
            }
            // Standalone markers without a length field.
            0x01 | 0xD0..=0xD7 => {
                pos += 2;
                continue;
            }
            0xD8 | 0x00 => return Ok(None),
            _ => {}
        }
        let mut len = [0u8; 2];
        if !read_at(r, pos + 2, &mut len)? {
            return Ok(None);
        }
        let seg_len = u16::from_be_bytes(len) as u64;
        let seg_end = pos + 2 + seg_len;
        if seg_len < 2 || seg_end > size {
            return Ok(None);
        }
        let is_metadata = (0xE0..=0xEF).contains(&marker[1]) || marker[1] == 0xFE;
        if is_metadata {
            push_range(&mut ranges, keep_start, pos);
            keep_start = seg_end;
        }
        pos = seg_end;
    }
}

/// Byte ranges of an MP4/MOV file that hold media data: the payload of every
/// top-level `mdat` atom. Everything else (`moov` with its `udta`/`meta`
/// metadata, `free`, trailing atoms appended by editors or Google Photos, …)
/// is ignored. Returns `None` if the file is not a well-formed atom list.
fn isobmff_payload_ranges<R: Read + Seek>(r: &mut R, size: u64) -> io::Result<Option<Vec<(u64, u64)>>> {
    const FIRST_ATOMS: [&[u8; 4]; 8] =
        [b"ftyp", b"moov", b"mdat", b"free", b"skip", b"wide", b"pnot", b"uuid"];
    let mut ranges = Vec::new();
    let mut pos = 0u64;
    while pos < size {
        let mut hdr = [0u8; 8];
        if !read_at(r, pos, &mut hdr)? {
            return Ok(None);
        }
        let size32 = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let kind = [hdr[4], hdr[5], hdr[6], hdr[7]];
        if pos == 0 && !FIRST_ATOMS.contains(&&kind) {
            return Ok(None);
        }
        if !kind.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
            return Ok(None);
        }
        let (atom_size, header_len) = match size32 {
            0 => (size - pos, 8), // extends to end of file
            1 => {
                let mut large = [0u8; 8];
                if !read_at(r, pos + 8, &mut large)? {
                    return Ok(None);
                }
                (u64::from_be_bytes(large), 16)
            }
            n => (n, 8),
        };
        if atom_size < header_len || atom_size > size - pos {
            return Ok(None);
        }
        if &kind == b"mdat" {
            ranges.push((pos + header_len, atom_size - header_len));
        }
        pos += atom_size;
    }
    Ok(if ranges.is_empty() { None } else { Some(ranges) })
}

/// `(offset, length)` byte ranges of a file.
type ByteRanges = Vec<(u64, u64)>;

/// Media payload of a supported file: (kind, ranges). `None` when the file is
/// not a supported format or has no payload.
fn payload_ranges(path: &Path, size: u64) -> io::Result<Option<(MediaKind, ByteRanges)>> {
    let kind = match media_kind_for(path) {
        Some(k) => k,
        None => return Ok(None),
    };
    let mut f = File::open(path)?;
    let ranges = match kind {
        MediaKind::Jpeg => jpeg_payload_ranges(&mut f, size)?,
        MediaKind::IsoBmff => isobmff_payload_ranges(&mut f, size)?,
    };
    Ok(ranges
        .filter(|r| r.iter().map(|(_, len)| len).sum::<u64>() > 0)
        .map(|r| (kind, r)))
}

fn hash_ranges(path: &Path, ranges: &[(u64, u64)]) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Context::new();
    let mut buf = vec![0u8; IO_BUFFER_SIZE];
    for &(offset, len) in ranges {
        hash_range(&mut f, offset, len, &mut hasher, &mut buf)?;
    }
    Ok(format!("{:x}", hasher.compute()))
}

/// Match media files whose payload is identical even though their metadata
/// (and therefore their size and full hash) differs. Two passes: parse the
/// container headers (cheap) and bucket by (format, payload length); hash
/// the payload only inside buckets that can still match.
fn metadata_skip_compare(
    groups: Vec<Vec<FileInfo>>,
    options: &CompareOptions,
) -> io::Result<Vec<DuplicateGroup>> {
    let mut results = Vec::new();

    for group in groups {
        let mut buckets: HashMap<(MediaKind, u64), Vec<(FileInfo, ByteRanges)>> = HashMap::new();
        for file in group {
            match payload_ranges(&file.path, file.size) {
                Ok(Some((kind, ranges))) => {
                    let len: u64 = ranges.iter().map(|(_, l)| l).sum();
                    buckets.entry((kind, len)).or_default().push((file, ranges));
                }
                Ok(None) => {
                    if options.debug {
                        println!("[META] Not a parseable media file: {}", file.path.display());
                    }
                }
                Err(e) => veprintln!("Warning: cannot read {}: {}", file.path.display(), e),
            }
        }

        let mut bucket_list: Vec<_> = buckets.into_values().collect();
        bucket_list.sort_by(|a, b| a[0].0.path.cmp(&b[0].0.path));
        for bucket in bucket_list {
            let files: Vec<FileInfo> = bucket.iter().map(|(f, _)| f.clone()).collect();
            if !keep_group(&files, options) {
                continue;
            }
            let mut by_hash: HashMap<String, Vec<FileInfo>> = HashMap::new();
            for (file, ranges) in bucket {
                match hash_ranges(&file.path, &ranges) {
                    Ok(h) => by_hash.entry(h).or_default().push(file),
                    Err(e) => veprintln!("Warning: cannot hash {}: {}", file.path.display(), e),
                }
            }
            let mut matched: Vec<Vec<FileInfo>> = by_hash
                .into_values()
                .filter(|g| keep_group(g, options))
                .collect();
            matched.sort_by(|a, b| a[0].path.cmp(&b[0].path));
            for g in matched {
                if options.debug {
                    println!("[META MATCH] same payload, different metadata: {:?}",
                        g.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>());
                }
                results.push(make_group(&g, MatchKind::MetadataOnly));
            }
        }
    }

    Ok(results)
}

// ══════════════════════════════════════════════════════════════════════════════
// Sorting
// ══════════════════════════════════════════════════════════════════════════════

fn first_path(group: &DuplicateGroup) -> Option<&PathBuf> {
    group.files_by_folder.iter().flatten().next()
}

fn group_mtime(group: &DuplicateGroup) -> Option<SystemTime> {
    first_path(group).and_then(|p| p.metadata().ok()).and_then(|m| m.modified().ok())
}

/// Order groups by path of their first file (default), by size or by the
/// modification time of their first file. Ties fall back to path order.
fn sort_groups(groups: &mut [DuplicateGroup], key: SortKey, reverse: bool) {
    match key {
        SortKey::Path => groups.sort_by(|a, b| first_path(a).cmp(&first_path(b))),
        SortKey::Size => groups.sort_by(|a, b| {
            a.size.cmp(&b.size).then_with(|| first_path(a).cmp(&first_path(b)))
        }),
        SortKey::Mtime => groups.sort_by_cached_key(|g| (group_mtime(g), first_path(g).cloned())),
    }
    if reverse {
        groups.reverse();
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Moving files
// ══════════════════════════════════════════════════════════════════════════════

/// Order the identical files of an intra-folder group so the one kept in place
/// comes first. Heuristic, cheapest signal first: shortest base name wins
/// (copies pick up suffixes like " (1)", "-copy", "_2"), then the shallowest
/// directory, then the lexicographically smallest path so the result is
/// deterministic. mtime is deliberately not used — `cp` without `-p`, unzip,
/// rsync and downloads all rewrite it, so it is not a reliable "original" mark.
fn keep_rank(path: &Path) -> (usize, usize) {
    let name_len = path.file_name().map_or(usize::MAX, |n| n.len());
    let depth = path.components().count();
    (name_len, depth)
}

/// Never overwrite a file a previous run already moved: if `target` exists,
/// use `name~1.ext`, `name~2.ext`, …
fn unique_destination(target: PathBuf) -> PathBuf {
    if !target.exists() {
        return target;
    }
    let stem = target.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let ext = target.extension().map(|e| e.to_string_lossy().to_string());
    for n in 1u32.. {
        let name = match &ext {
            Some(ext) => format!("{}~{}.{}", stem, n, ext),
            None => format!("{}~{}", stem, n),
        };
        let candidate = target.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("u32 range exhausted while picking a destination name")
}

#[derive(Debug, Default, PartialEq)]
struct MoveStats {
    moved: usize,
    skipped: usize,
    failed: usize,
}

/// Move the folder1 files of each group into `folder1/<dest_dir>/`,
/// preserving relative paths.
///
/// - Group with a folder2 copy, `-D`: a folder1 file is moved only if the
///   file at the *same relative path* in folder2 is part of the same match.
/// - Group with a folder2 copy, `-F`: every folder1 file is moved.
/// - Intra-folder group (no folder2 copy): one file is kept in place (see
///   `keep_rank`) and only the others are moved.
fn move_groups(
    groups: &[&DuplicateGroup],
    folder1: &str,
    folder2: &str,
    dest_dir: &str,
    options: &CompareOptions,
) -> io::Result<MoveStats> {
    let dest_root = Path::new(folder1).join(dest_dir);
    let mut stats = MoveStats::default();

    for group in groups {
        let folder2_copies: &[PathBuf] = group.files_by_folder.get(1).map_or(&[], |v| v.as_slice());
        let has_folder2_copy = !folder2_copies.is_empty();

        let mut folder1_files: Vec<&PathBuf> = group.files_by_folder[0].iter().collect();
        folder1_files.sort_by(|a, b| keep_rank(a).cmp(&keep_rank(b)).then_with(|| a.cmp(b)));

        let files_to_move: Vec<&PathBuf> = if has_folder2_copy {
            folder1_files
        } else {
            if folder1_files.len() <= 1 {
                continue;
            }
            if options.debug {
                println!("Intra-folder group: keeping {}", folder1_files[0].display());
            }
            folder1_files.into_iter().skip(1).collect()
        };

        for file_path in files_to_move {
            let relative = match file_path.strip_prefix(folder1) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Cannot compute relative path of {}: {}", file_path.display(), e);
                    stats.failed += 1;
                    continue;
                }
            };

            if !options.force_delete && has_folder2_copy {
                let counterpart = Path::new(folder2).join(relative);
                if !folder2_copies.iter().any(|p| p == &counterpart) {
                    if options.debug {
                        println!(
                            "Skipping {}: {} is not part of this match (use -F to move regardless of folder2 path)",
                            file_path.display(), counterpart.display()
                        );
                    }
                    stats.skipped += 1;
                    continue;
                }
            }

            if !file_path.exists() {
                // e.g. already moved as part of another fuzzy pair
                if options.debug {
                    println!("Source file no longer exists: {}", file_path.display());
                }
                stats.skipped += 1;
                continue;
            }

            let target = unique_destination(dest_root.join(relative));
            if let Some(parent) = target.parent() {
                if let Err(e) = create_dir_all(parent) {
                    eprintln!("Failed to create directory {}: {}", parent.display(), e);
                    stats.failed += 1;
                    continue;
                }
            }

            if options.debug {
                println!("Moving file: {} -> {}", file_path.display(), target.display());
            }

            // Try rename first (fast, atomic on the same filesystem); fall back to
            // a verified copy+remove on any error (cross-device, etc.).
            let result = match rename(file_path, &target) {
                Ok(()) => Ok(()),
                Err(e) => {
                    if options.debug {
                        eprintln!("Rename failed ({}); falling back to copy+remove", e);
                    }
                    copy_and_remove(file_path, &target, options)
                }
            };
            match result {
                Ok(()) => {
                    stats.moved += 1;
                    match &group.match_kind {
                        MatchKind::Exact => println!("Moved to {}/: {}", dest_dir, file_path.display()),
                        kind => println!("Moved to {}/ ({}): {}", dest_dir, kind.label(), file_path.display()),
                    }
                }
                Err(e) => {
                    stats.failed += 1;
                    eprintln!("Failed to move {} to {}: {}", file_path.display(), target.display(), e);
                }
            }
        }
    }

    if stats != MoveStats::default() {
        println!(
            "{}/: {} moved, {} skipped, {} failed",
            dest_dir, stats.moved, stats.skipped, stats.failed
        );
    }
    Ok(stats)
}

fn copy_and_remove(src: &Path, dst: &Path, options: &CompareOptions) -> io::Result<()> {
    if options.debug {
        println!("Copying file as fallback: {} -> {}", src.display(), dst.display());
    }

    // Get source metadata before copying (for verification and preservation)
    let src_metadata = src.metadata()?;
    let src_size = src_metadata.len();
    let src_perms = src_metadata.permissions();
    let src_atime = src_metadata.accessed().ok();
    let src_mtime = src_metadata.modified().ok();

    let bytes_copied = std::fs::copy(src, dst)?;

    // CRITICAL: Verify that the copy was complete before removing original
    let dst_len = dst.metadata().map(|m| m.len());
    if bytes_copied != src_size || dst_len.as_ref().ok() != Some(&src_size) {
        let _ = std::fs::remove_file(dst);
        return Err(io::Error::other(format!(
            "copy verification failed: expected {} bytes, copied {}, destination has {:?}. Original file preserved at {}",
            src_size, bytes_copied, dst_len.ok(), src.display()
        )));
    }

    // Preserve permissions and timestamps - warn if this fails but don't abort
    if let Err(e) = std::fs::set_permissions(dst, src_perms) {
        if options.debug {
            eprintln!("Warning: Failed to preserve permissions on {}: {}", dst.display(), e);
        }
    }
    if let (Some(atime), Some(mtime)) = (src_atime, src_mtime) {
        let fa = filetime::FileTime::from_system_time(atime);
        let fm = filetime::FileTime::from_system_time(mtime);
        if let Err(e) = filetime::set_file_times(dst, fa, fm) {
            if options.debug {
                eprintln!("Warning: Failed to preserve timestamps on {}: {}", dst.display(), e);
            }
        }
    }

    // Only now, after all verifications pass, remove the original file
    std::fs::remove_file(src)
}

// ══════════════════════════════════════════════════════════════════════════════
// Output
// ══════════════════════════════════════════════════════════════════════════════

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

/// What an "exact" group has actually been verified by, for display:
/// a full hash, only the quick check, or no content at all (name/size/ext keys).
fn exact_label(options: &CompareOptions) -> &'static str {
    let full_hash = options.compare_content
        && (!options.quick_content_check || options.uses_parallel_hashing());
    if full_hash {
        "EXACT"
    } else if options.quick_content_check {
        "QUICK"
    } else {
        "UNCHECKED"
    }
}

fn display_results(duplicates: &[DuplicateGroup], folder1: &str, folder2: &str, exact_label: &str) {
    if duplicates.is_empty() {
        println!("No duplicates found.");
        return;
    }

    let exact_count = duplicates.iter().filter(|g| g.match_kind == MatchKind::Exact).count();
    let fuzzy_count = duplicates
        .iter()
        .filter(|g| matches!(g.match_kind, MatchKind::Fuzzy { .. }))
        .count();
    let meta_count = duplicates.len() - exact_count - fuzzy_count;

    println!(
        "\nFound {} duplicate group(s) ({} {}, {} fuzzy near-duplicate, {} metadata-only):",
        duplicates.len(), exact_count, exact_label.to_lowercase(), fuzzy_count, meta_count
    );
    if exact_label == "UNCHECKED" {
        println!("Note: UNCHECKED groups match by name/size/extension only; file contents were not compared.");
    }
    println!("{:40} : {:40} | {:10} | match", folder1, folder2, "size");
    println!("{}", "-".repeat(100));

    let mut total_folder1_files = 0;
    let mut total_folder2_files = 0;
    let mut total_duplicate_size_folder1 = 0;

    let relative_list = |files: &[PathBuf], base: &str| {
        files
            .iter()
            .map(|p| p.strip_prefix(base).unwrap_or(p).to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    };

    for duplicate in duplicates {
        let folder1_files = &duplicate.files_by_folder[0];
        let folder2_files = &duplicate.files_by_folder[1];
        let size_str = if duplicate.uniform_size {
            format_size(duplicate.size)
        } else {
            format!("<={}", format_size(duplicate.size))
        };

        total_folder1_files += folder1_files.len();
        total_folder2_files += folder2_files.len();
        total_duplicate_size_folder1 += duplicate.folder1_bytes;

        println!(
            "{:40} : {:40} | {:10} | {}",
            relative_list(folder1_files, folder1),
            relative_list(folder2_files, folder2),
            size_str,
            match &duplicate.match_kind {
                MatchKind::Exact => exact_label.to_string(),
                kind => kind.label(),
            }
        );

        let has_subdirs = folder1_files.iter().chain(folder2_files.iter()).any(|path| {
            let parent = path.parent().unwrap_or(Path::new(""));
            parent != Path::new(folder1) && parent != Path::new(folder2)
        });

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
    base_folder2: &str,
) {
    // Group files by their parent directory (sorted for stable output)
    type SideBySide = (Vec<PathBuf>, Vec<PathBuf>);
    let mut subdir_map: BTreeMap<PathBuf, SideBySide> = BTreeMap::new();

    for path in folder1_files {
        if let Some(parent) = path.parent() {
            if parent != Path::new(base_folder1) {
                let rel_parent = parent.strip_prefix(base_folder1).unwrap_or(parent);
                subdir_map.entry(rel_parent.to_path_buf()).or_default().0.push(path.clone());
            }
        }
    }

    for path in folder2_files {
        if let Some(parent) = path.parent() {
            if parent != Path::new(base_folder2) {
                let rel_parent = parent.strip_prefix(base_folder2).unwrap_or(parent);
                subdir_map.entry(rel_parent.to_path_buf()).or_default().1.push(path.clone());
            }
        }
    }

    for (subdir, (files1, files2)) in subdir_map {
        println!("{}{}", subdir.to_string_lossy(), std::path::MAIN_SEPARATOR);

        let names1: BTreeSet<_> = files1.iter().filter_map(|p| p.file_name()).collect();
        let names2: BTreeSet<_> = files2.iter().filter_map(|p| p.file_name()).collect();

        for name in names1.union(&names2) {
            let in_folder1 = names1.contains(name);
            let in_folder2 = names2.contains(name);

            let left_mark = if in_folder1 { "|- " } else { "   " };
            let middle = if in_folder1 && in_folder2 { " : " } else { "   " };
            let right_mark = if in_folder2 { "|- " } else { "   " };

            println!("{}{}{}{}{}",
                left_mark,
                if in_folder1 { name.to_string_lossy() } else { "".into() },
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
                .join(format!("duptool_test_{}_{}", std::process::id(), tag));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir)
                .unwrap_or_else(|e| panic!("create temp dir {:?}: {}", dir, e));
            TempDir(dir)
        }
        fn path(&self) -> &Path { &self.0 }
        fn str(&self) -> &str { self.0.to_str().unwrap() }
    }

    impl Drop for TempDir {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    // ─── helpers ──────────────────────────────────────────────────────────────

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
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
            tolerance:          None,
            tolerance_pct:      None,
            fuzzy_as_dupes:     false,
            skip_metadata:      false,
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
        let f1 = collect_files(d1.str(), 0, false).unwrap();
        let f2 = collect_files(d2.str(), 1, false).unwrap();
        find_duplicates(f1, f2, d1.str(), d2.str(), opts).unwrap()
    }

    /// Convenience: single-folder intra-comparison.
    fn run_find_single(d1: &TempDir, opts: &CompareOptions) -> Vec<DuplicateGroup> {
        let f1 = collect_files(d1.str(), 0, false).unwrap();
        find_duplicates(f1, vec![], d1.str(), "", opts).unwrap()
    }

    fn by_name(files: &[FileInfo]) -> Vec<Vec<FileInfo>> {
        group_by_key(files.to_vec(), &CompareOptions { compare_name: true, ..default_options() }, false)
    }
    fn by_stem(files: &[FileInfo]) -> Vec<Vec<FileInfo>> {
        group_by_key(files.to_vec(), &CompareOptions { compare_stem: true, ..default_options() }, false)
    }
    fn by_ext(files: &[FileInfo]) -> Vec<Vec<FileInfo>> {
        group_by_key(files.to_vec(), &CompareOptions { compare_ext: true, ..default_options() }, false)
    }

    /// A group of same-size files, as the matching stages produce.
    fn group_of(folder1: Vec<PathBuf>, folder2: Vec<PathBuf>, size: u64, kind: MatchKind) -> DuplicateGroup {
        let folder1_bytes = size * folder1.len() as u64;
        DuplicateGroup {
            files_by_folder: vec![folder1, folder2],
            size,
            uniform_size: true,
            folder1_bytes,
            match_kind: kind,
        }
    }

    /// Parse a command line the way main() does.
    fn parse(args: &[&str]) -> Result<RunConfig, String> {
        let mut argv = vec!["duptool"];
        argv.extend_from_slice(args);
        let matches = build_cli().try_get_matches_from(argv).map_err(|e| e.to_string())?;
        config_from_matches(&matches)
    }

    /// Paths of every group, relative-free, for comparing result sets.
    fn path_sets(groups: &[DuplicateGroup]) -> Vec<Vec<PathBuf>> {
        let mut sets: Vec<Vec<PathBuf>> = groups
            .iter()
            .map(|g| {
                let mut v: Vec<PathBuf> = g.files_by_folder.iter().flatten().cloned().collect();
                v.sort();
                v
            })
            .collect();
        sets.sort();
        sets
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // grouping keys: name / stem / ext
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_group_by_name_same_name_grouped() {
        let files = vec![
            make_file_info("/a/photo.jpg", 100, 0),
            make_file_info("/b/photo.jpg", 100, 1),
            make_file_info("/a/other.txt", 100, 0),
        ];
        let groups = by_name(&files);
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
        let groups = by_name(&files);
        assert!(groups.iter().all(|g| g.len() == 1),
            "distinct filenames must each get their own group");
    }

    #[test]
    fn test_group_by_stem_different_extensions_grouped() {
        // photo.jpg / photo.png / photo.bmp all share stem "photo"
        let files = vec![
            make_file_info("/a/photo.jpg", 100, 0),
            make_file_info("/b/photo.png", 100, 1),
            make_file_info("/c/photo.bmp", 100, 0),
        ];
        let groups = by_stem(&files);
        assert_eq!(groups.len(), 1, "all three share stem 'photo'");
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn test_group_by_stem_different_stems_stay_separate() {
        let files = vec![
            make_file_info("/a/photo.jpg",  100, 0),
            make_file_info("/b/backup.jpg", 100, 1),
        ];
        assert_eq!(by_stem(&files).len(), 2, "different stems must produce separate groups");
    }

    #[test]
    fn test_group_by_stem_no_extension_shares_stem() {
        // "README"    -> file_stem = "README"
        // "README.md" -> file_stem = "README"   (stdlib strips only the last dot-suffix)
        let files = vec![
            make_file_info("/a/README",    100, 0),
            make_file_info("/b/README.md", 100, 1),
        ];
        let groups = by_stem(&files);
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
        assert_eq!(by_stem(&files).len(), 1,
            "both share stem 'archive.tar' (stdlib strips only the last suffix)");
    }

    #[test]
    fn test_group_by_stem_case_sensitive() {
        // Stem comparison is case-sensitive (consistent with -n).
        let files = vec![
            make_file_info("/a/Photo.jpg", 100, 0),
            make_file_info("/b/photo.jpg", 100, 1),
        ];
        assert_eq!(by_stem(&files).len(), 2,
            "stem comparison is case-sensitive: 'Photo' != 'photo'");
    }

    #[test]
    fn test_group_by_ext_same_extension_grouped() {
        let files = vec![
            make_file_info("/a/photo.jpg",    100, 0),
            make_file_info("/b/vacation.jpg", 100, 1),
        ];
        let groups = by_ext(&files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_by_ext_case_insensitive() {
        let files = vec![
            make_file_info("/a/photo.JPG",    100, 0),
            make_file_info("/b/vacation.jpg", 100, 1),
        ];
        assert_eq!(by_ext(&files).len(), 1, ".JPG and .jpg must match case-insensitively");
    }

    #[test]
    fn test_group_by_ext_different_extensions_separated() {
        let files = vec![
            make_file_info("/a/photo.jpg",    100, 0),
            make_file_info("/b/document.pdf", 100, 1),
        ];
        assert_eq!(by_ext(&files).len(), 2);
    }

    #[test]
    fn test_group_by_ext_no_extension_grouped_under_empty_key() {
        let files = vec![
            make_file_info("/a/Makefile",   100, 0),
            make_file_info("/b/Dockerfile", 100, 1),
        ];
        let groups = by_ext(&files);
        assert_eq!(groups.len(), 1, "both have no extension → same group");
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_by_ext_no_ext_vs_with_ext_separated() {
        let files = vec![
            make_file_info("/a/Makefile",    100, 0),
            make_file_info("/b/Makefile.mk", 100, 1),
        ];
        assert_eq!(by_ext(&files).len(), 2, "no-extension and .mk must be separate groups");
    }

    #[test]
    fn test_group_by_ext_dotted_name_only_last_suffix_is_ext() {
        // "Act 1. The Noon.mp4" — the dot in "1." is part of the stem, NOT the extension.
        let files = vec![
            make_file_info("/a/Act 1. The Noon.mp4", 100, 0),
            make_file_info("/b/Act 2. The Dusk.mp4", 100, 1),
        ];
        assert_eq!(by_ext(&files).len(), 1, "both have extension 'mp4'; dots in the title are irrelevant");
        let stem = Path::new("Act 1. The Noon.mp4").file_stem().unwrap().to_string_lossy();
        assert_eq!(stem, "Act 1. The Noon",
            "stem should be everything before the last dot, not affected by title dots");
    }

    #[test]
    fn test_group_by_ext_dotted_name_different_ext_separated() {
        let files = vec![
            make_file_info("/a/Act 1. The Noon.mp4", 100, 0),
            make_file_info("/b/Act 1. The Noon.mkv", 100, 1),
        ];
        assert_eq!(by_ext(&files).len(), 2, ".mp4 and .mkv must be in separate groups");
    }

    #[test]
    fn test_group_by_stem_dotted_name_groups_by_full_title() {
        let files = vec![
            make_file_info("/a/Act 1. The Noon.mp4", 100, 0),
            make_file_info("/b/Act 1. The Noon.mkv", 100, 1),
        ];
        let groups = by_stem(&files);
        assert_eq!(groups.len(), 1, "both share stem 'Act 1. The Noon'");
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_key_with_size_splits_by_size() {
        let files = vec![
            make_file_info("/a/x.txt", 10, 0),
            make_file_info("/b/x.txt", 20, 1),
        ];
        let opts = CompareOptions { compare_name: true, ..default_options() };
        assert_eq!(group_by_key(files.clone(), &opts, true).len(), 2, "with size: different sizes split");
        assert_eq!(group_by_key(files, &opts, false).len(), 1, "without size: same name groups");
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // format_size / has_files_from_both_folders / make_group
    // ═══════════════════════════════════════════════════════════════════════════

    #[test] fn test_format_size_zero()         { assert_eq!(format_size(0),               "0 bytes"); }
    #[test] fn test_format_size_small_bytes()  { assert_eq!(format_size(512),             "512 bytes"); }
    #[test] fn test_format_size_boundary_1kb() { assert_eq!(format_size(1024),            "1.00 KB"); }
    #[test] fn test_format_size_2kb()          { assert_eq!(format_size(2048),            "2.00 KB"); }
    #[test] fn test_format_size_boundary_1mb() { assert_eq!(format_size(1024*1024),       "1.00 MB"); }
    #[test] fn test_format_size_boundary_1gb() { assert_eq!(format_size(1024*1024*1024),  "1.00 GB"); }

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

    #[test]
    fn test_make_group_mixed_sizes() {
        let g = make_group(&[
            make_file_info("/a/x", 10, 0),
            make_file_info("/a/y", 30, 0),
            make_file_info("/b/x", 20, 1),
        ], MatchKind::Exact);
        assert_eq!(g.size, 30, "size is the largest file");
        assert!(!g.uniform_size);
        assert_eq!(g.folder1_bytes, 40, "folder1 total is summed per file");
        assert_eq!(g.files_by_folder[0].len(), 2);
        assert_eq!(g.files_by_folder[1].len(), 1);
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // hashing
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

    #[test]
    fn test_hash_known_md5() {
        let d = TempDir::new("hash_md5");
        let f = d.path().join("abc.txt");
        write_file(&f, b"abc");
        let info = FileInfo { path: f, size: 3, folder_index: 0 };
        assert_eq!(calculate_file_hash(&info, false).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
    }

    /// A reader that returns at most one byte per read() call, like a slow
    /// pipe or network filesystem may.
    struct OneByteReader(io::Cursor<Vec<u8>>);
    impl Read for OneByteReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(1);
            self.0.read(&mut buf[..n])
        }
    }
    impl Seek for OneByteReader {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> { self.0.seek(pos) }
    }

    #[test]
    fn test_regression_short_reads_do_not_change_hash() {
        // Before the fix the quick check did a single read() per 8MB block;
        // a short read silently hashed fewer bytes than intended.
        let data = b"0123456789abcdef".to_vec();
        let mut expected = Context::new();
        expected.consume(&data[4..12]);
        let mut hasher = Context::new();
        let mut buf = vec![0u8; 64];
        let mut r = OneByteReader(io::Cursor::new(data));
        hash_range(&mut r, 4, 8, &mut hasher, &mut buf).unwrap();
        assert_eq!(format!("{:x}", hasher.compute()), format!("{:x}", expected.compute()));

        let mut r = OneByteReader(io::Cursor::new(vec![7u8; 10]));
        let mut buf = [0u8; 8];
        assert_eq!(read_full(&mut r, &mut buf).unwrap(), 8, "read_full must fill the buffer");
    }

    #[test]
    fn test_hash_range_past_eof_is_an_error() {
        let mut r = io::Cursor::new(vec![1u8; 4]);
        let mut hasher = Context::new();
        let mut buf = vec![0u8; 16];
        assert!(hash_range(&mut r, 0, 10, &mut hasher, &mut buf).is_err(),
            "a file that shrank since the scan must not hash as if complete");
    }

    #[test]
    fn test_quick_check_large_files_and_async_full_hash() {
        // Two files just above the 16MB quick-check threshold that differ only
        // in the middle: the quick hash cannot tell them apart, a full hash can.
        let d1 = TempDir::new("qk_big1");
        let d2 = TempDir::new("qk_big2");
        let size = (QUICKCHECK_SIZE * 2 + 4096) as usize;
        let a = vec![5u8; size];
        let mut b = a.clone();
        b[size / 2] = 6;
        write_file(&d1.path().join("big.bin"), &a);
        write_file(&d2.path().join("big.bin"), &b);

        let ia = FileInfo { path: d1.path().join("big.bin"), size: size as u64, folder_index: 0 };
        let ib = FileInfo { path: d2.path().join("big.bin"), size: size as u64, folder_index: 1 };
        assert_eq!(calculate_file_hash(&ia, true).unwrap(), calculate_file_hash(&ib, true).unwrap());
        assert_ne!(calculate_file_hash(&ia, false).unwrap(), calculate_file_hash(&ib, false).unwrap());

        // -C alone: quick check only (documented behaviour) → reported.
        let quick_only = CompareOptions { quick_content_check: true, ..content_options() };
        assert_eq!(run_find(&d1, &d2, &quick_only).len(), 1);
        // -C -A: quick check filters, full hash decides → not a duplicate.
        let quick_full = CompareOptions { quick_content_check: true, async_compare: true, ..content_options() };
        assert!(run_find(&d1, &d2, &quick_full).is_empty(),
            "-C with -A must confirm quick-check matches with a full hash");
    }

    #[test]
    fn test_hash_strategy_selection() {
        let sync = content_options();
        assert_eq!(hash_strategy(&sync), HashStrategy::Sequential);
        let hdd_async = CompareOptions { async_compare: true, hdd_optimized: true, ..content_options() };
        assert_eq!(hash_strategy(&hdd_async), HashStrategy::PerFolder,
            "-A on HDD (default -m) must use one reader per folder, not a thread per CPU");
        let ssd_async = CompareOptions { async_compare: true, hdd_optimized: false, ..content_options() };
        assert!(matches!(hash_strategy(&ssd_async), HashStrategy::Pool(_)));
        let enhanced_ssd = CompareOptions { enhanced_async: true, hdd_optimized: false, ..content_options() };
        assert_eq!(hash_strategy(&enhanced_ssd), HashStrategy::PerFolder);
    }

    #[test]
    fn test_all_hash_strategies_agree() {
        let d1 = TempDir::new("strat1");
        let d2 = TempDir::new("strat2");
        for i in 0..6u8 {
            let body = vec![i; 100 + i as usize];
            write_file(&d1.path().join(format!("dup{}.bin", i)), &body);
            write_file(&d2.path().join(format!("sub/dup{}.bin", i)), &body);
            // same size, different content → must never match
            write_file(&d2.path().join(format!("other{}.bin", i)), &vec![i + 100; 100 + i as usize]);
        }
        let sync = run_find(&d1, &d2, &content_options());
        assert_eq!(sync.len(), 6);
        for opts in [
            CompareOptions { async_compare: true, hdd_optimized: true, ..content_options() },
            CompareOptions { async_compare: true, hdd_optimized: false, ..content_options() },
            CompareOptions { enhanced_async: true, ..content_options() },
            CompareOptions { quick_content_check: true, async_compare: true, ..content_options() },
        ] {
            assert_eq!(path_sets(&run_find(&d1, &d2, &opts)), path_sets(&sync),
                "strategy {:?} must give the same result as sequential hashing", hash_strategy(&opts));
        }
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // chunked_fuzzy_compare / fuzzy_limit
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_fuzzy_compare_identical_zero_diff() {
        let d = TempDir::new("fuz_id");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        write_file(&f1, &vec![0u8; 4096]);
        write_file(&f2, &vec![0u8; 4096]);
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, u64::MAX).unwrap(), 0);
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
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, u64::MAX).unwrap(), 1);
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
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, u64::MAX).unwrap(), 3);
    }

    #[test]
    fn test_fuzzy_compare_all_bytes_differ() {
        let d = TempDir::new("fuz_all");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        write_file(&f1, &vec![0x00u8; 256]);
        write_file(&f2, &vec![0xFFu8; 256]);
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, u64::MAX).unwrap(), 256);
    }

    #[test]
    fn test_fuzzy_compare_stops_past_limit() {
        // Two 3MB files that differ everywhere: with limit 10 the result must
        // still exceed the limit (the caller rejects the pair) — the compare
        // stops after the first chunk instead of reading everything.
        let d = TempDir::new("fuz_lim");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        write_file(&f1, &vec![0x00u8; 3 * IO_BUFFER_SIZE]);
        write_file(&f2, &vec![0xFFu8; 3 * IO_BUFFER_SIZE]);
        let diff = chunked_fuzzy_compare(&f1, &f2, 10).unwrap();
        assert!(diff > 10);
        assert!(diff <= IO_BUFFER_SIZE as u64, "must stop after the first chunk, got {}", diff);
    }

    #[test]
    fn test_fuzzy_compare_different_lengths_rejected() {
        let d = TempDir::new("fuz_len");
        let f1 = d.path().join("a.bin");
        let f2 = d.path().join("b.bin");
        write_file(&f1, &[0u8; 10]);
        write_file(&f2, &[0u8; 11]);
        assert_eq!(chunked_fuzzy_compare(&f1, &f2, 100).unwrap(), u64::MAX);
    }

    #[test]
    fn test_fuzzy_limit_bytes_percent_and_both() {
        let bytes = CompareOptions { tolerance: Some(50), ..default_options() };
        assert_eq!(bytes.fuzzy_limit(1_000_000), 50);
        let pct = CompareOptions { tolerance_pct: Some(1.0), ..default_options() };
        assert_eq!(pct.fuzzy_limit(512), 5, "1% of 512 bytes, rounded down");
        assert_eq!(pct.fuzzy_limit(1_000_000), 10_000);
        let both = CompareOptions { tolerance: Some(50), tolerance_pct: Some(1.0), ..default_options() };
        assert_eq!(both.fuzzy_limit(512), 5, "with -t and -T the stricter limit wins");
        assert_eq!(both.fuzzy_limit(1_000_000), 50);
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // find_duplicates — integration tests
    // ═══════════════════════════════════════════════════════════════════════════

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
        assert!(run_find(&d1, &d2, &content_options()).is_empty(), "different content → no duplicate");
    }

    #[test]
    fn test_fd_size_mismatch_eliminated_before_hash() {
        let d1 = TempDir::new("fd_sm1");
        let d2 = TempDir::new("fd_sm2");
        write_file(&d1.path().join("a.txt"), b"short");
        write_file(&d2.path().join("a.txt"), b"much longer content here");
        assert!(run_find(&d1, &d2, &content_options()).is_empty(),
            "size-mismatched files must be eliminated before hashing");
    }

    #[test]
    fn test_fd_multiple_groups_found() {
        let d1 = TempDir::new("fd_mg1");
        let d2 = TempDir::new("fd_mg2");
        write_file(&d1.path().join("alpha.txt"), b"group one match");
        write_file(&d2.path().join("alpha.txt"), b"group one match");
        write_file(&d1.path().join("beta.dat"),  b"group two!!");
        write_file(&d2.path().join("beta.dat"),  b"group two!!");
        assert_eq!(run_find(&d1, &d2, &content_options()).len(), 2,
            "two independent duplicate groups should both be found");
    }

    #[test]
    fn test_fd_results_sorted_by_first_path() {
        let d1 = TempDir::new("fd_sort1");
        let d2 = TempDir::new("fd_sort2");
        for (name, body) in [("c.txt", "ccc"), ("a.txt", "a"), ("b.txt", "bb")] {
            write_file(&d1.path().join(name), body.as_bytes());
            write_file(&d2.path().join(name), body.as_bytes());
        }
        let dups = run_find(&d1, &d2, &content_options());
        let names: Vec<_> = dups.iter()
            .map(|g| g.files_by_folder[0][0].file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["a.txt", "b.txt", "c.txt"]);
    }

    // ── name mode (-n) ─────────────────────────────────────────────────────────

    #[test]
    fn test_fd_name_mode_same_name_same_size_matches() {
        let d1 = TempDir::new("fd_nm1");
        let d2 = TempDir::new("fd_nm2");
        write_file(&d1.path().join("photo.txt"), b"aaaa");
        write_file(&d2.path().join("photo.txt"), b"bbbb"); // same name+size, different content
        let opts = CompareOptions { compare_name: true, ..default_options() };
        assert_eq!(run_find(&d1, &d2, &opts).len(), 1, "same name + same size → match in -n mode");
    }

    #[test]
    fn test_fd_name_mode_different_name_no_match() {
        let d1 = TempDir::new("fd_nm3");
        let d2 = TempDir::new("fd_nm4");
        write_file(&d1.path().join("photo.txt"), b"aaaa");
        write_file(&d2.path().join("other.txt"), b"bbbb");
        let opts = CompareOptions { compare_name: true, ..default_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(), "different names → no match even with same size");
    }

    #[test]
    fn test_regression_name_only_matches_different_sizes() {
        // TODO item: "-BDEXnk and -BDEXk don't compare files by filename only".
        // Grouping by size always ran first, so same-named files of different
        // size never met. -n/-k without -s/-c now match by name alone.
        let d1 = TempDir::new("fd_nosz1");
        let d2 = TempDir::new("fd_nosz2");
        write_file(&d1.path().join("song.mp3"), b"128 kbps version");
        write_file(&d2.path().join("song.mp3"), b"320 kbps version, much larger");
        let opts = CompareOptions { compare_name: true, ..default_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1, "same name, different size → match in -n mode");
        assert!(!dups[0].uniform_size);
    }

    #[test]
    fn test_fd_name_plus_size_requires_same_size() {
        let d1 = TempDir::new("fd_nmsz1");
        let d2 = TempDir::new("fd_nmsz2");
        write_file(&d1.path().join("song.mp3"), b"short");
        write_file(&d2.path().join("song.mp3"), b"much longer");
        let opts = CompareOptions { compare_name: true, compare_size: true, ..default_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(), "-n -s must still require equal sizes");
    }

    #[test]
    fn test_regression_cli_bdexnk_and_bdexk_match_by_name_only() {
        let d1 = TempDir::new("cli_nk1");
        let d2 = TempDir::new("cli_nk2");
        write_file(&d1.path().join("track.mp3"), b"a");
        write_file(&d2.path().join("track.mp3"), b"bbbbbbbb");
        write_file(&d2.path().join("track.flac"), b"ccccccccccccc");

        let cfg = parse(&["-BDEXnk", d1.str(), d2.str()]).unwrap();
        assert!(!cfg.options.compare_content, "-n/-k alone must not enable content comparison");
        assert!(!cfg.options.uses_size_grouping());
        let dups = run_find(&d1, &d2, &cfg.options);
        assert_eq!(dups.len(), 1, "-BDEXnk: track.mp3 ↔ track.mp3");
        assert_eq!(dups[0].files_by_folder[1], vec![d2.path().join("track.mp3")]);

        let cfg = parse(&["-BDEXk", d1.str(), d2.str()]).unwrap();
        let dups = run_find(&d1, &d2, &cfg.options);
        assert_eq!(dups.len(), 1, "-BDEXk: one stem group 'track'");
        assert_eq!(dups[0].files_by_folder[1].len(), 2, "both track.mp3 and track.flac share the stem");
    }

    // ── stem mode (-k) ─────────────────────────────────────────────────────────

    #[test]
    fn test_fd_stem_mode_matches_different_extension() {
        let d1 = TempDir::new("fd_sk1");
        let d2 = TempDir::new("fd_sk2");
        write_file(&d1.path().join("photo.jpg"), b"same");
        write_file(&d2.path().join("photo.png"), b"diff");
        let opts = CompareOptions { compare_stem: true, ..default_options() };
        assert_eq!(run_find(&d1, &d2, &opts).len(), 1, "same stem → match in -k mode");
    }

    #[test]
    fn test_fd_stem_mode_no_match_different_stem() {
        let d1 = TempDir::new("fd_sk3");
        let d2 = TempDir::new("fd_sk4");
        write_file(&d1.path().join("photo.jpg"),  b"same");
        write_file(&d2.path().join("backup.jpg"), b"same"); // different stem
        let opts = CompareOptions { compare_stem: true, ..default_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(), "different stems → no match in -k mode");
    }

    #[test]
    fn test_fd_stem_and_content_no_match_if_content_differs() {
        let d1 = TempDir::new("fd_sk5");
        let d2 = TempDir::new("fd_sk6");
        write_file(&d1.path().join("doc.txt"), b"v1ab");
        write_file(&d2.path().join("doc.pdf"), b"v2ab");
        let opts = CompareOptions { compare_stem: true, ..content_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(),
            "same stem but different content → no match when -k combined with -c");
    }

    // ── extension mode (-e) ────────────────────────────────────────────────────

    #[test]
    fn test_fd_ext_mode_matches_same_extension() {
        let d1 = TempDir::new("fd_ex1");
        let d2 = TempDir::new("fd_ex2");
        write_file(&d1.path().join("a.jpg"), b"aaaa");
        write_file(&d2.path().join("b.jpg"), b"bbbb");
        let opts = CompareOptions { compare_ext: true, ..default_options() };
        assert_eq!(run_find(&d1, &d2, &opts).len(), 1, "same extension + same size → match in -e mode");
    }

    #[test]
    fn test_fd_ext_mode_still_requires_same_size() {
        let d1 = TempDir::new("fd_exsz1");
        let d2 = TempDir::new("fd_exsz2");
        write_file(&d1.path().join("a.jpg"), b"aaaa");
        write_file(&d2.path().join("b.jpg"), b"bbbbbbbb");
        let opts = CompareOptions { compare_ext: true, ..default_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(), "-e alone keeps size grouping");
    }

    #[test]
    fn test_fd_ext_mode_case_insensitive() {
        let d1 = TempDir::new("fd_ex3");
        let d2 = TempDir::new("fd_ex4");
        write_file(&d1.path().join("a.JPG"), b"aaaa");
        write_file(&d2.path().join("b.jpg"), b"bbbb");
        let opts = CompareOptions { compare_ext: true, ..default_options() };
        assert_eq!(run_find(&d1, &d2, &opts).len(), 1, ".JPG and .jpg must match case-insensitively in -e mode");
    }

    #[test]
    fn test_fd_ext_mode_no_match_different_extension() {
        let d1 = TempDir::new("fd_ex5");
        let d2 = TempDir::new("fd_ex6");
        write_file(&d1.path().join("a.jpg"), b"aaaa");
        write_file(&d2.path().join("b.png"), b"bbbb");
        let opts = CompareOptions { compare_ext: true, ..default_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(), "different extensions → no match in -e mode");
    }

    #[test]
    fn test_fd_ext_and_stem_equals_name() {
        // Combining -e and -k is equivalent to -n (full filename).
        let d1 = TempDir::new("fd_ex7");
        let d2 = TempDir::new("fd_ex8");
        write_file(&d1.path().join("photo.jpg"), b"aaaa");
        write_file(&d2.path().join("photo.png"), b"bbbb");
        let opts = CompareOptions { compare_stem: true, compare_ext: true, ..default_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(),
            "with both -k and -e, stem must match AND extension must match (like -n)");
    }

    // ── inter- vs intra-folder ─────────────────────────────────────────────────

    #[test]
    fn test_regression_default_mode_ignores_intra_folder_duplicates() {
        // folder1 holds two identical files; folder2 holds a same-size file
        // with different content. Before the fix the size group passed the
        // cross-folder filter, and the hash stage then reported (and -D moved)
        // the folder1-only pair although -b was not given.
        let d1 = TempDir::new("fd_inter1");
        let d2 = TempDir::new("fd_inter2");
        write_file(&d1.path().join("a.txt"), b"same bytes!");
        write_file(&d1.path().join("b.txt"), b"same bytes!");
        write_file(&d2.path().join("c.txt"), b"other bytes");
        assert!(run_find(&d1, &d2, &content_options()).is_empty(),
            "without -b only folder1↔folder2 duplicates are reported");
        let intra = CompareOptions { intra_folder: true, ..content_options() };
        assert_eq!(run_find(&d1, &d2, &intra).len(), 1, "with -b the folder1 pair is reported");
    }

    #[test]
    fn test_fd_intra_folder_finds_internal_duplicates() {
        let d1 = TempDir::new("fd_if1");
        write_file(&d1.path().join("a.txt"), b"same content here!");
        write_file(&d1.path().join("b.txt"), b"same content here!");
        let opts = CompareOptions { intra_folder: true, ..content_options() };
        assert_eq!(run_find_single(&d1, &opts).len(), 1, "intra-folder mode must find duplicates within one folder");
    }

    #[test]
    fn test_fd_intra_folder_no_match_different_content() {
        let d1 = TempDir::new("fd_if2");
        write_file(&d1.path().join("a.txt"), b"content A is here");
        write_file(&d1.path().join("b.txt"), b"content B is here");
        let opts = CompareOptions { intra_folder: true, ..content_options() };
        assert!(run_find_single(&d1, &opts).is_empty(), "no intra-folder match when content differs");
    }

    #[test]
    fn test_regression_single_folder_quick_check_finds_duplicates() {
        // The quick-check stage filtered with has_files_from_both_folders
        // unconditionally, so `-1 -C` could never find anything.
        let d1 = TempDir::new("fd_1c");
        write_file(&d1.path().join("a.txt"), b"same content here!");
        write_file(&d1.path().join("b.txt"), b"same content here!");
        let opts = CompareOptions { intra_folder: true, quick_content_check: true, ..content_options() };
        assert_eq!(run_find_single(&d1, &opts).len(), 1);
    }

    #[test]
    fn test_regression_async_intra_folder_finds_duplicates() {
        let d1 = TempDir::new("ai_r1");
        write_file(&d1.path().join("a.txt"), b"same content here!!");
        write_file(&d1.path().join("b.txt"), b"same content here!!");
        for opts in [
            CompareOptions { async_compare: true, intra_folder: true, hdd_optimized: true, ..content_options() },
            CompareOptions { async_compare: true, intra_folder: true, hdd_optimized: false, ..content_options() },
        ] {
            assert_eq!(run_find_single(&d1, &opts).len(), 1,
                "async intra-folder mode must find duplicates (not silently skip every group)");
        }
    }

    // ── robustness ─────────────────────────────────────────────────────────────

    #[test]
    fn test_regression_unreadable_file_does_not_abort_run() {
        // A file that disappears (or is unreadable) between scan and hash used
        // to abort the whole run with an error. It is now skipped.
        let d1 = TempDir::new("fd_gone1");
        let d2 = TempDir::new("fd_gone2");
        write_file(&d1.path().join("ok.txt"), b"still here");
        write_file(&d2.path().join("ok.txt"), b"still here");
        write_file(&d1.path().join("gone.txt"), b"vanishing!");
        write_file(&d2.path().join("gone.txt"), b"vanishing!");
        let f1 = collect_files(d1.str(), 0, false).unwrap();
        let f2 = collect_files(d2.str(), 1, false).unwrap();
        fs::remove_file(d1.path().join("gone.txt")).unwrap();
        for opts in [content_options(), CompareOptions { async_compare: true, ..content_options() }] {
            let dups = find_duplicates(f1.clone(), f2.clone(), d1.str(), d2.str(), &opts).unwrap();
            assert_eq!(dups.len(), 1, "the readable pair must still be found");
        }
    }

    #[test]
    fn test_regression_scan_skips_own_output_folders() {
        // Files already moved to folder1/deleted or folder1/differ by a
        // previous run must not be scanned again (they would be re-matched
        // and moved to deleted/deleted/...).
        let d1 = TempDir::new("scan_out1");
        write_file(&d1.path().join("keep.txt"), b"x");
        write_file(&d1.path().join("deleted/old.txt"), b"x");
        write_file(&d1.path().join("differ/near.txt"), b"x");
        write_file(&d1.path().join("sub/deleted/real.txt"), b"x"); // not at top level → scanned
        let files = collect_files(d1.str(), 0, true).unwrap();
        let mut names: Vec<_> = files.iter()
            .map(|f| f.path.strip_prefix(d1.path()).unwrap().to_path_buf())
            .collect();
        names.sort();
        assert_eq!(names, [PathBuf::from("keep.txt"), Path::new("sub").join("deleted").join("real.txt")]);
        // folder2 is never filtered.
        let f2 = collect_files(d1.str(), 1, true).unwrap();
        assert_eq!(f2.len(), 4);
    }

    // ── fuzzy mode (-Z) ────────────────────────────────────────────────────────

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
        let opts = CompareOptions { fuzzy: true, tolerance: Some(5), ..content_options() };
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
        let opts = CompareOptions { fuzzy: true, tolerance: Some(10), ..content_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(), "512-byte difference with tolerance=10 → no match");
    }

    #[test]
    fn test_fd_fuzzy_exact_match_not_reported_as_fuzzy() {
        let d1 = TempDir::new("fd_fz5");
        let d2 = TempDir::new("fd_fz6");
        write_file(&d1.path().join("a.bin"), b"identical content!!");
        write_file(&d2.path().join("b.bin"), b"identical content!!");
        let opts = CompareOptions { fuzzy: true, tolerance: Some(100), ..content_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].match_kind, MatchKind::Exact,
            "identical files must be reported as Exact even when -Z is active");
    }

    #[test]
    fn test_fd_fuzzy_percentage_threshold() {
        // 2 of 512 bytes differ = 0.39%.
        let d1 = TempDir::new("fd_fzp1");
        let d2 = TempDir::new("fd_fzp2");
        let mut cb = vec![0u8; 512];
        cb[1] = 1;
        cb[2] = 1;
        write_file(&d1.path().join("a.bin"), &[0u8; 512]);
        write_file(&d2.path().join("b.bin"), &cb);
        let loose = CompareOptions { fuzzy: true, tolerance_pct: Some(1.0), ..content_options() };
        assert_eq!(run_find(&d1, &d2, &loose).len(), 1, "0.39% ≤ 1% → match");
        let strict = CompareOptions { fuzzy: true, tolerance_pct: Some(0.1), ..content_options() };
        assert!(run_find(&d1, &d2, &strict).is_empty(), "0.39% > 0.1% → no match");
        let both = CompareOptions { fuzzy: true, tolerance: Some(1), tolerance_pct: Some(1.0), ..content_options() };
        assert!(run_find(&d1, &d2, &both).is_empty(), "-t 1 is stricter than -T 1 here");
    }

    #[test]
    fn test_regression_intra_folder_fuzzy() {
        // Fuzzy compared only folder0×folder1 pairs, so -1/-b never found
        // near-duplicates within a folder.
        let d1 = TempDir::new("fd_fzi");
        let mut cb = vec![9u8; 256];
        cb[0] = 0;
        write_file(&d1.path().join("a.bin"), &[9u8; 256]);
        write_file(&d1.path().join("a (1).bin"), &cb);
        let opts = CompareOptions { fuzzy: true, tolerance: Some(3), intra_folder: true, ..content_options() };
        let dups = run_find_single(&d1, &opts);
        assert_eq!(dups.len(), 1);
        assert!(matches!(dups[0].match_kind, MatchKind::Fuzzy { bytes_differing: 1 }));
        assert_eq!(dups[0].files_by_folder[0].len(), 2);
    }

    #[test]
    fn test_regression_quickcheck_plus_fuzzy_catches_near_dupes() {
        // Fuzzy candidates are taken from the pre-hash candidate pool, so
        // near-duplicates that *failed* the quick check are still compared.
        let d1 = TempDir::new("cz_r1");
        let d2 = TempDir::new("cz_r2");
        let ca = vec![42u8; 64];
        let mut cb = vec![42u8; 64];
        cb[32] = 99;
        write_file(&d1.path().join("a.bin"), &ca);
        write_file(&d2.path().join("b.bin"), &cb);
        let opts = CompareOptions {
            quick_content_check: true,
            fuzzy:               true,
            tolerance:           Some(5),
            ..default_options()
        };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1, "near-duplicate must be found even when -C and -Z are combined");
        assert!(matches!(dups[0].match_kind, MatchKind::Fuzzy { .. }));
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // fuzzy mode 2 (-Y): metadata skip
    // ═══════════════════════════════════════════════════════════════════════════

    /// Minimal JPEG: SOI, APP1(meta), DQT, SOS + scan data, EOI.
    fn jpeg(meta: &[u8], scan: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend([0xFF, 0xE1]);
        v.extend(((meta.len() + 2) as u16).to_be_bytes());
        v.extend(meta);
        v.extend([0xFF, 0xDB, 0x00, 0x04, 0x11, 0x22]);
        v.extend([0xFF, 0xDA, 0x00, 0x04, 0x33, 0x44]);
        v.extend(scan);
        v.extend([0xFF, 0xD9]);
        v
    }

    fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(kind);
        v.extend(payload);
        v
    }

    /// Minimal MP4: ftyp, moov{udta}, mdat and optionally a trailing udta.
    fn mp4(udta: &[u8], media: &[u8], trailing: Option<&[u8]>) -> Vec<u8> {
        let mut v = atom(b"ftyp", b"isom\0\0\0\0");
        v.extend(atom(b"moov", &atom(b"udta", udta)));
        v.extend(atom(b"mdat", media));
        if let Some(t) = trailing {
            v.extend(atom(b"udta", t));
        }
        v
    }

    #[test]
    fn test_jpeg_payload_ranges_skip_app_segments() {
        let data = jpeg(b"Exif\0\0rotation=90", b"SCANDATA");
        let size = data.len() as u64;
        let ranges = jpeg_payload_ranges(&mut io::Cursor::new(data.clone()), size).unwrap().unwrap();
        let app1_len = 2 + 2 + 17; // marker + length field + payload
        assert_eq!(ranges, vec![(0, 2), (2 + app1_len as u64, size - 2 - app1_len as u64)]);
        let kept: Vec<u8> = ranges.iter()
            .flat_map(|&(o, l)| data[o as usize..(o + l) as usize].to_vec())
            .collect();
        assert!(!kept.windows(4).any(|w| w == b"Exif"), "EXIF bytes must not be part of the payload");
        assert!(kept.windows(8).any(|w| w == b"SCANDATA"), "image data must be part of the payload");
    }

    #[test]
    fn test_jpeg_and_mp4_parsers_reject_other_data() {
        let garbage = b"definitely not media".to_vec();
        let n = garbage.len() as u64;
        assert!(jpeg_payload_ranges(&mut io::Cursor::new(garbage.clone()), n).unwrap().is_none());
        assert!(isobmff_payload_ranges(&mut io::Cursor::new(garbage), n).unwrap().is_none());
        // A JPEG whose APP1 length runs past the end of the file.
        let truncated = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x10, 0x00, 1, 2, 3];
        assert!(jpeg_payload_ranges(&mut io::Cursor::new(truncated), 9).unwrap().is_none());
        // An MP4 atom claiming more bytes than the file has.
        let mut bad = atom(b"ftyp", b"isom");
        bad.extend(&[0, 0, 0x10, 0, b'm', b'd', b'a', b't', 1, 2]);
        let n = bad.len() as u64;
        assert!(isobmff_payload_ranges(&mut io::Cursor::new(bad), n).unwrap().is_none());
    }

    #[test]
    fn test_isobmff_payload_is_mdat_only() {
        let data = mp4(b"gps=50.4,30.5", b"VIDEOFRAMES", Some(b"trailing"));
        let size = data.len() as u64;
        let ranges = isobmff_payload_ranges(&mut io::Cursor::new(data.clone()), size).unwrap().unwrap();
        assert_eq!(ranges.len(), 1);
        let (o, l) = ranges[0];
        assert_eq!(&data[o as usize..(o + l) as usize], b"VIDEOFRAMES");
    }

    #[test]
    fn test_fd_skip_metadata_jpeg_exif_differs() {
        let d1 = TempDir::new("meta_jpg1");
        let d2 = TempDir::new("meta_jpg2");
        write_file(&d1.path().join("IMG_1.jpg"), &jpeg(b"Exif\0\0orientation=1", b"pixels!!"));
        write_file(&d2.path().join("IMG_1.jpg"), &jpeg(b"Exif\0\0orientation=6;date=2026", b"pixels!!"));
        // Without -Y: sizes differ → nothing.
        assert!(run_find(&d1, &d2, &content_options()).is_empty());
        let opts = CompareOptions { skip_metadata: true, ..content_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].match_kind, MatchKind::MetadataOnly);
        assert!(!dups[0].uniform_size);
    }

    #[test]
    fn test_fd_skip_metadata_jpeg_different_image_no_match() {
        let d1 = TempDir::new("meta_jpg3");
        let d2 = TempDir::new("meta_jpg4");
        write_file(&d1.path().join("a.jpg"), &jpeg(b"Exif\0\0x", b"pixelsAA"));
        write_file(&d2.path().join("a.jpg"), &jpeg(b"Exif\0\0yy", b"pixelsBB"));
        let opts = CompareOptions { skip_metadata: true, ..content_options() };
        assert!(run_find(&d1, &d2, &opts).is_empty(), "different image data must never match");
    }

    #[test]
    fn test_fd_skip_metadata_mp4_with_trailing_atom() {
        // Google Photos-style export: GPS removed from moov/udta and a trailing
        // atom appended — the mdat payload is unchanged.
        let d1 = TempDir::new("meta_mp41");
        let d2 = TempDir::new("meta_mp42");
        write_file(&d1.path().join("VID.mp4"), &mp4(b"gps=50.4501,30.5234", b"frames-frames", None));
        write_file(&d2.path().join("VID.MOV"), &mp4(b"", b"frames-frames", Some(b"exported by app")));
        let opts = CompareOptions { skip_metadata: true, ..content_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].match_kind, MatchKind::MetadataOnly);

        write_file(&d2.path().join("VID.MOV"), &mp4(b"", b"frames-FRAMES", None));
        assert!(run_find(&d1, &d2, &opts).is_empty(), "different mdat must not match");
    }

    #[test]
    fn test_fd_skip_metadata_ignores_non_media_and_keeps_exact() {
        let d1 = TempDir::new("meta_mix1");
        let d2 = TempDir::new("meta_mix2");
        // Identical JPEGs → Exact, not reported twice.
        let same = jpeg(b"Exif\0\0same", b"img");
        write_file(&d1.path().join("same.jpg"), &same);
        write_file(&d2.path().join("same.jpg"), &same);
        // Non-media files with different bytes are never payload-compared.
        write_file(&d1.path().join("x.bin"), b"abc");
        write_file(&d2.path().join("x.bin"), b"abcdef");
        let opts = CompareOptions { skip_metadata: true, ..content_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].match_kind, MatchKind::Exact);
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // sorting
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_sort_groups_by_size_and_reverse() {
        let mut groups = vec![
            group_of(vec![PathBuf::from("/a/big")], vec![], 300, MatchKind::Exact),
            group_of(vec![PathBuf::from("/a/small")], vec![], 10, MatchKind::Exact),
            group_of(vec![PathBuf::from("/a/mid")], vec![], 100, MatchKind::Exact),
        ];
        sort_groups(&mut groups, SortKey::Size, false);
        assert_eq!(groups.iter().map(|g| g.size).collect::<Vec<_>>(), [10, 100, 300]);
        sort_groups(&mut groups, SortKey::Size, true);
        assert_eq!(groups.iter().map(|g| g.size).collect::<Vec<_>>(), [300, 100, 10]);
        sort_groups(&mut groups, SortKey::Path, false);
        assert_eq!(first_path(&groups[0]).unwrap(), Path::new("/a/big"));
    }

    #[test]
    fn test_sort_groups_by_mtime() {
        let d = TempDir::new("sort_mtime");
        let names = ["new.txt", "old.txt", "mid.txt"];
        let times = [3_000_000_000i64, 1_000_000_000, 2_000_000_000];
        for (n, t) in names.iter().zip(times) {
            let p = d.path().join(n);
            write_file(&p, b"x");
            filetime::set_file_mtime(&p, filetime::FileTime::from_unix_time(t, 0)).unwrap();
        }
        let mut groups: Vec<_> = names.iter()
            .map(|n| group_of(vec![d.path().join(n)], vec![], 1, MatchKind::Exact))
            .collect();
        sort_groups(&mut groups, SortKey::Mtime, false);
        let order: Vec<_> = groups.iter()
            .map(|g| first_path(g).unwrap().file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(order, ["old.txt", "mid.txt", "new.txt"]);
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // CLI parsing / validation
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_cli_defaults_to_content() {
        let cfg = parse(&["a", "b"]).unwrap();
        assert!(cfg.options.compare_content && cfg.options.compare_size);
        assert!(cfg.options.hdd_optimized);
        assert_eq!(cfg.sort_key, SortKey::Path);
        assert_eq!(cfg.folder2.as_deref(), Some("b"));
    }

    #[test]
    fn test_cli_second_folder_required_unless_single() {
        assert!(parse(&["a"]).is_err());
        let cfg = parse(&["-1", "a"]).unwrap();
        assert!(cfg.folder2.is_none());
        assert!(cfg.options.intra_folder);
    }

    #[test]
    fn test_cli_fuzzy_validation() {
        assert!(parse(&["-Z", "a", "b"]).is_err(), "-Z needs a tolerance");
        assert_eq!(parse(&["-Z", "-t", "50", "a", "b"]).unwrap().options.tolerance, Some(50));
        assert_eq!(parse(&["-Z", "-T", "0.5", "a", "b"]).unwrap().options.tolerance_pct, Some(0.5));
        assert!(parse(&["-Z", "-T", "150", "a", "b"]).is_err(), "-T above 100 is rejected");
        assert!(parse(&["-Z", "-T=-1", "a", "b"]).is_err(), "-T below 0 is rejected");
        assert!(parse(&["-U", "a", "b"]).is_err(), "-U needs -Z or -Y");
        assert!(parse(&["-YU", "a", "b"]).unwrap().options.fuzzy_as_dupes);
    }

    #[test]
    fn test_cli_conflicts() {
        assert!(parse(&["-B", "-b", "a", "b"]).is_err(), "-B (between only) contradicts -b (also within)");
        assert!(parse(&["-m", "-M", "a", "b"]).is_err());
        assert!(parse(&["--sort-size", "--sort-mtime", "a", "b"]).is_err());
    }

    #[test]
    fn test_exact_label_reflects_what_was_verified() {
        assert_eq!(exact_label(&content_options()), "EXACT");
        assert_eq!(exact_label(&CompareOptions { quick_content_check: true, ..content_options() }), "QUICK");
        assert_eq!(exact_label(&CompareOptions {
            quick_content_check: true, async_compare: true, ..content_options()
        }), "EXACT", "-C -A confirms with a full hash");
        assert_eq!(exact_label(&CompareOptions { compare_name: true, ..default_options() }), "UNCHECKED");
    }

    #[test]
    fn test_cli_sort_flags() {
        let cfg = parse(&["--sort-size", "--sort-reverse", "a", "b"]).unwrap();
        assert_eq!(cfg.sort_key, SortKey::Size);
        assert!(cfg.sort_reverse);
        assert_eq!(parse(&["--sort-mtime", "a", "b"]).unwrap().sort_key, SortKey::Mtime);
    }

    #[test]
    fn test_regression_validate_folders_rejects_same_and_nested() {
        let d = TempDir::new("validate");
        fs::create_dir_all(d.path().join("one/inner")).unwrap();
        fs::create_dir_all(d.path().join("two")).unwrap();
        let one = d.path().join("one");
        let one = one.to_str().unwrap();
        let inner = d.path().join("one/inner");
        let inner = inner.to_str().unwrap();
        let two = d.path().join("two");
        let two = two.to_str().unwrap();
        assert!(validate_folders(one, Some(two)).is_ok());
        assert!(validate_folders(one, None).is_ok());
        assert!(validate_folders(one, Some(one)).is_err(), "same folder");
        assert!(validate_folders(one, Some(inner)).is_err(), "folder2 inside folder1");
        assert!(validate_folders(inner, Some(one)).is_err(), "folder1 inside folder2");
        assert!(validate_folders(&format!("{}/missing", two), None).is_err(), "missing folder");
    }


    // ═══════════════════════════════════════════════════════════════════════════
    // moving files (-D / -F)
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn test_regression_group_size_is_single_file_size_not_sum() {
        let d1 = TempDir::new("gs_r1");
        let d2 = TempDir::new("gs_r2");
        let content = b"exactly sixteen!"; // 16 bytes
        write_file(&d1.path().join("f.txt"), content);
        write_file(&d2.path().join("f.txt"), content);
        let dups = run_find(&d1, &d2, &content_options());
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].size, 16,
            "DuplicateGroup.size must be one file's size (16 B), not the group sum (32 B)");
        assert_eq!(dups[0].folder1_bytes, 16);
    }

    #[test]
    fn test_regression_intra_folder_delete_keeps_one_copy() {
        let d1 = TempDir::new("intra_del_keep1");
        write_file(&d1.path().join("dup_a.bin"), b"identical payload");
        write_file(&d1.path().join("dup_b.bin"), b"identical payload");
        write_file(&d1.path().join("unique.bin"), b"something else entirely");

        let opts = CompareOptions { intra_folder: true, delete_duplicates: true, ..content_options() };
        let group = group_of(
            vec![d1.path().join("dup_a.bin"), d1.path().join("dup_b.bin")],
            vec![], 17, MatchKind::Exact,
        );
        move_groups(&[&group], d1.str(), "", DELETED_DIR, &opts).unwrap();

        let remaining = ["dup_a.bin", "dup_b.bin"].iter().filter(|n| d1.path().join(n).exists()).count();
        assert_eq!(remaining, 1,
            "exactly one copy of an intra-folder duplicate group must be kept in place");
        assert!(d1.path().join("deleted").join("dup_b.bin").exists(),
            "the other copy must be moved into deleted/");
        assert!(d1.path().join("unique.bin").exists(), "unrelated files must never be touched");
    }

    #[test]
    fn test_intra_folder_delete_keeps_shortest_name() {
        let d1 = TempDir::new("intra_del_shortest");
        for n in ["font.woff2", "font (1).woff2", "font-copy.woff2"] {
            write_file(&d1.path().join(n), b"same webfont bytes");
        }
        let opts = CompareOptions { intra_folder: true, delete_duplicates: true, ..content_options() };
        let group = group_of(
            vec![
                d1.path().join("font (1).woff2"),
                d1.path().join("font-copy.woff2"),
                d1.path().join("font.woff2"),
            ],
            vec![], 18, MatchKind::Exact,
        );
        move_groups(&[&group], d1.str(), "", DELETED_DIR, &opts).unwrap();

        assert!(d1.path().join("font.woff2").exists(), "the shortest name must be kept as the original");
        assert!(!d1.path().join("font (1).woff2").exists()
            && !d1.path().join("font-copy.woff2").exists(),
            "the longer-named copies must be moved");
        assert!(d1.path().join("deleted").join("font (1).woff2").exists()
            && d1.path().join("deleted").join("font-copy.woff2").exists(),
            "both copies must land in deleted/");
    }

    #[test]
    fn test_regression_two_folder_delete_still_moves_matched_copy() {
        let d1 = TempDir::new("two_del_r1");
        let d2 = TempDir::new("two_del_r2");
        write_file(&d1.path().join("match.bin"), b"shared bytes");
        write_file(&d2.path().join("match.bin"), b"shared bytes");
        let opts = CompareOptions { delete_duplicates: true, ..content_options() };
        let group = group_of(
            vec![d1.path().join("match.bin")],
            vec![d2.path().join("match.bin")], 12, MatchKind::Exact,
        );
        let stats = move_groups(&[&group], d1.str(), d2.str(), DELETED_DIR, &opts).unwrap();
        assert_eq!(stats, MoveStats { moved: 1, skipped: 0, failed: 0 });
        assert!(!d1.path().join("match.bin").exists());
        assert!(d1.path().join("deleted").join("match.bin").exists());
        assert!(d2.path().join("match.bin").exists(), "folder2 copy (the keeper) must be left alone");
    }

    #[test]
    fn test_regression_delete_requires_matching_counterpart() {
        // folder1/a.txt duplicates folder2/b.txt. folder2/a.txt exists but has
        // different content. -D used to check only that *some* file existed at
        // the same relative path, and moved folder1/a.txt anyway.
        let d1 = TempDir::new("del_cp1");
        let d2 = TempDir::new("del_cp2");
        write_file(&d1.path().join("a.txt"), b"payload one");
        write_file(&d2.path().join("b.txt"), b"payload one");
        write_file(&d2.path().join("a.txt"), b"different!!");
        let opts = CompareOptions { delete_duplicates: true, ..content_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        let refs: Vec<&DuplicateGroup> = dups.iter().collect();
        let stats = move_groups(&refs, d1.str(), d2.str(), DELETED_DIR, &opts).unwrap();
        assert_eq!(stats.moved, 0);
        assert!(d1.path().join("a.txt").exists(), "-D must not move a file whose same-path counterpart differs");

        // -F ignores the path and moves it.
        let force = CompareOptions { force_delete: true, ..content_options() };
        let stats = move_groups(&refs, d1.str(), d2.str(), DELETED_DIR, &force).unwrap();
        assert_eq!(stats.moved, 1);
        assert!(d1.path().join("deleted/a.txt").exists());
    }

    #[test]
    fn test_regression_move_does_not_overwrite_previous_run() {
        let d1 = TempDir::new("del_noover1");
        let d2 = TempDir::new("del_noover2");
        write_file(&d1.path().join("deleted/x.txt"), b"moved by an earlier run");
        write_file(&d1.path().join("x.txt"), b"new duplicate");
        write_file(&d2.path().join("x.txt"), b"new duplicate");
        let opts = CompareOptions { delete_duplicates: true, ..content_options() };
        let group = group_of(vec![d1.path().join("x.txt")], vec![d2.path().join("x.txt")], 13, MatchKind::Exact);
        move_groups(&[&group], d1.str(), d2.str(), DELETED_DIR, &opts).unwrap();
        assert_eq!(fs::read(d1.path().join("deleted/x.txt")).unwrap(), b"moved by an earlier run");
        assert_eq!(fs::read(d1.path().join("deleted/x~1.txt")).unwrap(), b"new duplicate");
    }

    #[cfg(unix)]
    #[test]
    fn test_regression_special_character_names_can_be_moved() {
        // sanitize_path rewrote ':', '?', '*', '"', '|', '<', '>' and trailing
        // spaces in the *source* path, so such files (legal on Linux/macOS)
        // were reported but could never be moved.
        let d1 = TempDir::new("del_chars1");
        let d2 = TempDir::new("del_chars2");
        let name = "Track: Remix? *live* ";
        write_file(&d1.path().join(name), b"song bytes");
        write_file(&d2.path().join(name), b"song bytes");
        let opts = CompareOptions { delete_duplicates: true, ..content_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        let refs: Vec<&DuplicateGroup> = dups.iter().collect();
        let stats = move_groups(&refs, d1.str(), d2.str(), DELETED_DIR, &opts).unwrap();
        assert_eq!(stats.moved, 1);
        assert!(d1.path().join("deleted").join(name).exists(), "the name must be preserved exactly");
    }

    #[test]
    fn test_fuzzy_groups_move_to_differ() {
        let d1 = TempDir::new("differ1");
        let d2 = TempDir::new("differ2");
        let mut cb = vec![1u8; 64];
        cb[3] = 2;
        write_file(&d1.path().join("p.bin"), &[1u8; 64]);
        write_file(&d2.path().join("p.bin"), &cb);
        let opts = CompareOptions { fuzzy: true, tolerance: Some(2), delete_duplicates: true, ..content_options() };
        let dups = run_find(&d1, &d2, &opts);
        assert_eq!(dups.len(), 1);
        let refs: Vec<&DuplicateGroup> = dups.iter().collect();
        move_groups(&refs, d1.str(), d2.str(), DIFFER_DIR, &opts).unwrap();
        assert!(d1.path().join("differ/p.bin").exists());
        assert!(!d1.path().join("deleted").exists(), "nothing is created for an unused destination");
    }
}
