use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    // Get build date in YYYYMMDD format
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    let build_date = chrono::NaiveDateTime::from_timestamp_opt(now as i64, 0)
        .unwrap()
        .format("%Y%m%d")
        .to_string();
    
    // Get or create build counter for today
    let counter_file = Path::new(&env::var("OUT_DIR").unwrap()).join("build_counter.txt");
    let stored_date_file = Path::new(&env::var("OUT_DIR").unwrap()).join("build_date.txt");
    
    // Read stored date and counter
    let (stored_date, counter) = if stored_date_file.exists() {
        let stored = fs::read_to_string(&stored_date_file).unwrap_or_default();
        let count = fs::read_to_string(&counter_file)
            .unwrap_or_else(|_| "0".to_string())
            .trim()
            .parse::<u32>()
            .unwrap_or(0);
        (stored.trim().to_string(), count)
    } else {
        (String::new(), 0)
    };
    
    // If date changed, reset counter
    let build_number = if stored_date == build_date {
        counter + 1
    } else {
        0
    };
    
    // Save new date and counter
    fs::write(&stored_date_file, &build_date).unwrap();
    fs::write(&counter_file, build_number.to_string()).unwrap();
    
    // Get git commit hash if available
    let git_hash = Command::new("git")
        .args(&["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    
    // Check if git repo is dirty
    let git_dirty = Command::new("git")
        .args(&["diff", "--quiet"])
        .status()
        .map(|status| !status.success())
        .unwrap_or(false);
    
    let git_suffix = if git_dirty { "-dirty" } else { "" };
    
    // Generate build info
    println!("cargo:rustc-env=BUILD_DATE={}", build_date);
    println!("cargo:rustc-env=BUILD_NUMBER={}", build_number);
    println!("cargo:rustc-env=GIT_HASH={}{}", git_hash, git_suffix);
    
    // Full version string for -V
    let full_version = format!(
        "{}+build{}.{}",
        env::var("CARGO_PKG_VERSION").unwrap(),
        build_date,
        build_number
    );
    println!("cargo:rustc-env=FULL_VERSION={}", full_version);
    
    // Rebuild if git changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
