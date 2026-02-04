# duptool

**Advanced and efficient folder comparison tool**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE.md)

`duptool` is a high-performance utility for finding duplicate files between two folders, optimized for speed, accuracy, and practical workflows. Unlike tools like Total Commander, `duptool` focuses on comparing **only between folders**, with advanced filtering and hashing strategies.

---

## ✨ Features

- 🔄 **Bidirectional Comparison** (`-B`): Only compare files between `folder1` and `folder2`, ignoring internal duplicates.
- ⚡ **Quick Check Mode** (`-C`): Compare first and last 8MB before full hashing to speed up detection.
- 🚀 **Async Processing** (`-A`/`-E`): Compare files in parallel using multithreading.
- 🗃️ **Name/Size Comparison** (`-n`, `-s`): Optional modes for fast, coarse comparison.
- 📂 **Smart Deletion**:
  - `-D`: Move duplicates to `deleted` subfolder if path matches.
  - `-F`: Force-delete duplicates from `folder1`, regardless of relative path.
- 🔍 **Everything Integration**: Use Everything for rapid name/size checks (`-N`, `-S`).
- 🧠 **Progress Estimation**: Real-time ETA updates and per-file speed feedback.
- 🧰 **Debug Mode** (`-X`): Outputs detailed logs for diagnostics.
- 🧹 **Path Handling**: Handles long paths and Unicode edge cases on Windows.
- 🛠️ **HDD Optimization**: Control caching behavior (`-m`, `-M`) for HDD/SSD.

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

## 🔧 Usage Example

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
* Automated tests and CI

---

## 📜 Changelog

See [DEVLOG.md](DEVLOG.md) for version history and detailed changes.

**Note:** Starting from v0.1.10, the complete code history and changes can be tracked via the git timeline in the main branch.
