# duptool

**Advanced and efficient folder comparison tool**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

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

```sh
cargo build --release
```

Then copy the binary from `target/release/duptool`.

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
