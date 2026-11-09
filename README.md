# duptool

**Advanced and efficient folder comparison tool**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE.md)

`duptool` is a high-performance utility for finding duplicate files between two folders, optimized for speed, accuracy, and practical workflows. Unlike tools like Total Commander, `duptool` focuses on comparing **only between folders**, with advanced filtering and hashing strategies.

---

## ✨ Features

- 🔄 **Between-folder Comparison** (default, `-B`): Only compare files between `folder1` and `folder2`, ignoring internal duplicates.
- ⚡ **Quick Check Mode** (`-C`): Compare first and last 8MB only; with `-A`/`-E` the quick check becomes a filter before a full hash.
- 🚀 **Async Processing** (`-A`/`-E`): Hash in parallel. With `-m` (default) each folder gets one sequential reader (HDD-friendly; two disks are still read in parallel); with `-M`, `-A` uses all CPUs.
- 🗃️ **Name/Stem/Extension/Size Comparison** (`-n`, `-k`, `-e`, `-s`): Coarse, metadata-only matching. `-n`/`-k` alone match by name regardless of size; add `-s` or `-c` to also require equal size/content.
- 📁 **Single Folder Mode** (`-1`): Find duplicates within a single folder — no second folder required.
- 🔁 **Intra-folder Search** (`-b`): Also find duplicates within each folder alongside inter-folder comparison.
- 🌫️ **Fuzzy Comparison** (`-Z -t N` / `-Z -T P`): Detect near-duplicates of the same size that differ in at most N bytes (or P % of the file). Runs on every candidate that failed the exact stages. Matches are moved to a `differ/` folder for review.
- 🖼️ **Metadata-aware Comparison** (`-Y`): Match JPEG / MP4 / MOV files whose image or video payload is identical while their metadata (EXIF, XMP, GPS atoms, trailing atoms) differs — even when their sizes differ.
- 📂 **Smart Deletion**:
  - `-D`: Move a duplicate to `deleted/` only if the file at the same relative path in `folder2` is the matching copy.
  - `-F`: Force-move duplicates from `folder1`, regardless of relative path.
  - Within a folder (`-1`/`-b`) one copy is always kept — the one with the shortest name.
  - `-Z`/`-Y`: Move near-duplicates to `differ/` for review; add `-U` to treat them as exact duplicates (moved to `deleted/`).
  - Never overwrites: an existing `deleted/x` makes the new file `deleted/x~1`. `deleted/` and `differ/` are not rescanned on the next run.
  - Refuses to run on the same folder twice or on nested folders (a file would be "matched" with itself).
- ↕️ **Sorting**: `--sort-size`, `--sort-mtime`, `--sort-reverse` (default: by path).
- 🧠 **Progress Estimation**: Files and bytes progress bars with throughput and ETA.
- 🧰 **Debug Mode** (`-X`): Outputs detailed logs for diagnostics, including fuzzy byte-diff info.
- 🛠️ **HDD Optimization**: `-m` (default) reads in path order with at most one reader per folder; `-M` for SSDs.
- 🔍 **Everything Integration** (`-N`, `-S`): reserved, not implemented yet.

The `match` column of the output shows how a group was verified: `EXACT` (full hash), `QUICK` (`-C` only: first/last 8MB), `UNCHECKED` (name/size/extension only), `FUZZY ~NB`, or `META` (same payload, different metadata).

---

## 🚀 Why duptool?

Traditional comparison tools often:
- Compare within folders (slowing things down)
- Lack efficient filtering
- Require full hashing for every file

`duptool` was built to solve these frustrations with:
- Targeted inter-folder comparison
- Fast filtering via partial content checks
- Asynchronous multithreaded operation
- Command-line power and flexibility

---

## 🔧 Usage Examples

```sh
duptool folder1 folder2 -ABCEFX
````

This compares files **between** `folder1` and `folder2`, using:

* `A`: Async hashing
* `B`: Bidirectional comparison
* `C`: Quick check (first/last 8MB)
* `E`: Enhanced async mode
* `F`: Force delete duplicates from folder1
* `X`: Debug logging

**Fuzzy / near-duplicate detection** (note: `-t` always specified separately since it takes a value):

```sh
duptool folder1 folder2 -Z -t 50
```

Finds files that differ by 50 bytes or fewer — moves them to `folder1/differ/` for review.

```sh
duptool folder1 folder2 -ABCZ -t 50
```

Full pipeline with async, bidirectional, quick check, and fuzzy fallback for quick-check misses.

```sh
duptool folder1 folder2 -Z -t 50 -U -F
```

Treat fuzzy matches as exact duplicates and force-move them to `deleted/`.

```sh
duptool folder1 folder2 -Z -T 0.1
```

Percentage threshold: files of the same size that differ in at most 0.1 % of their bytes. With both `-t` and `-T`, the stricter limit wins.

```sh
duptool photos backup -Y -D
```

Match photos/videos whose EXIF/XMP or MP4/MOV metadata was changed or stripped (e.g. by a phone or cloud export) and move them to `photos/differ/`.

```sh
duptool music1 music2 -BDnk --sort-size --sort-reverse
```

Match by file name only (sizes may differ), move matches from `music1` to `music1/deleted/`, largest first.

For a full list of flags, run:

```sh
duptool --help
```

---

## 📦 Installation

### 🔧 Building from Source

To build `duptool` from source:

1. **Install Rust and Cargo**

   Ensure you have the Rust toolchain installed. If not, install it using [rustup](https://rustup.rs/):

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

   After installation, restart your terminal or run:

   ```bash
   source $HOME/.cargo/env
   ```

   Confirm the installation:

   ```bash
   rustc --version
   cargo --version
   ```

2. **Clone the Repository**

   Clone the `duptool` repository from GitHub:

   ```bash
   git clone https://github.com/ArsenijN/duptool.git
   cd duptool
   ```

3. **Build the Project**

   Compile the project in release mode for optimized performance:

   ```bash
   cargo build --release
   ```

   The compiled binary will be located at:

   ```
   target/release/duptool
   ```

   You can move this binary to a directory in your system's `PATH` for easier access.

### 📥 Downloading Precompiled Binaries

Precompiled binaries for various platforms are available on the [Releases](https://github.com/ArsenijN/duptool/releases) page. These binaries are digitally signed using a self-signed certificate to verify their authenticity and integrity.

**Verifying the Digital Signature:**

Each release includes:

* The signed binary (e.g., `duptool.exe`)
* The detached signature file (e.g., `duptool.exe.sig`)
* The public certificate file (e.g., `duptool_public.cer`)

To verify the signature:

1. **Obtain the Public Certificate**

   Download the `duptool_public.cer` file from the release assets.

2. **Verify the Signature**

   Use the `signtool` utility (available in the Windows SDK) to verify the signature:

   ```bash
   signtool verify /pa /v duptool.exe
   ```

   This command checks the signature against the public certificate and confirms the binary's integrity.

*Note: Since the certificate is self-signed, Windows may not recognize it as trusted by default. Users can manually install the `duptool_public.cer` certificate into their Trusted Root Certification Authorities store to establish trust.*

---

**Cross-platform note**

- `duptool` is cross-platform (Linux, macOS, Windows). On Linux and other Unix-like systems moving files between different mounted filesystems may require a copy+remove fallback instead of a simple rename; the tool now uses a safe fallback so `-D`/`--delete` behavior works across mounts.

**Developer notes**

Developer-focused signing and CI instructions were moved to `DEVNOTES.md`. See that file for guidance on creating checksums, GPG detached signatures and optional Authenticode signing for Windows releases.

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).

---

## 🛣️ Roadmap

Planned improvements include:

* More accurate ETA smoothing
* Per-file progress bar
* Interactive duplicate management mode
* Better path/unicode edge case handling
* CI (automated tests exist: `cargo test`)

---

## 📜 Changelog

See [DEVLOG.md](DEVLOG.md) for version history and detailed changes.

**Note:** Starting from v0.1.10, the complete code history and changes can be tracked via the git timeline in the main branch.
