# ngrep

Drop-in `grep` replacement. Same interface, 5x–41x faster.

Built from scratch in Rust. Uses SIMD-accelerated matching, memory-mapped I/O, and parallel directory walking. Works on ARM and x86.

## Benchmarks

Single file, 39MB, 500K lines:

| Test | grep | ngrep | Speedup |
|------|------|-------|---------|
| Sparse (444 hits) | 79ms | 15ms | **5.3x** |
| Dense (1,926 hits) | 329ms | 13ms | **25x** |
| Case-insensitive | 536ms | 13ms | **41x** |
| Regex `l[io]ne.*[0-9]+` | 383ms | 14ms | **27x** |
| Count `-c` | 318ms | 11ms | **29x** |
| Word match `-w` | 375ms | 14ms | **27x** |
| Invert `-v` | 80ms | 18ms | **4.4x** |
| Fixed string `-F` | 92ms | 15ms | **6.1x** |
| Context `-C3` | 74ms | 16ms | **4.6x** |
| Piped stdin | 63ms | 38ms | **1.7x** |

Recursive directory (multi-project, ~/dev/):

| Test | grep | ngrep | Speedup |
|------|------|-------|---------|
| Recursive `-rn` | 2,474ms | 102ms | **24x** |
| `--include=*.ts` | 233ms | 34ms | **6.9x** |
| `--exclude-dir` | 19ms | 7ms | **2.7x** |
| Files only `-rl` | 660ms | 106ms | **6.2x** |
| Count `-rc` | 1,359ms | 162ms | **8.4x** |

Compared against BSD grep 2.6.0 (macOS default) on Apple M4. ngrep performs on par with ripgrep 15.1.0.

## Install

```bash
cargo install --path .
```

Or build manually:

```bash
cargo build --release
cp target/release/ngrep /usr/local/bin/
```

### Alias as grep

```bash
# .zshrc / .bashrc
alias grep='ngrep'
```

## Usage

```
ngrep [OPTIONS] PATTERN [PATH ...]
command | ngrep [OPTIONS] PATTERN
```

Recursive by default. Skips binary files. Smart-case (case-insensitive when pattern is all lowercase).

### Examples

```bash
ngrep "TODO" src/                    # recursive search
ngrep -n "function" lib/             # with line numbers
ngrep -i "error" /var/log/           # case-insensitive
ngrep -rn --include="*.py" "import"  # filter by file type
ngrep -w "main" .                    # whole word match
ngrep -c "TODO" .                    # count matches per file
ngrep -l "fixme" .                   # list matching files
ngrep -v "debug" app.log             # invert match
ngrep -C3 "panic" .                  # 3 lines of context
ngrep -F "foo.bar()" .               # fixed string, no regex
cat log.txt | ngrep "ERROR"          # piped input
ngrep --gitignore "TODO" .           # respect .gitignore
ngrep -t py "import" .               # filter by file type
```

## Options

```
-i              Case-insensitive
-S              Smart case (on by default)
-n              Line numbers
-l              Filenames only
-c              Count matches per file
-v              Invert match
-w              Whole words
-F              Fixed strings (no regex)
-o              Only matching parts
-q              Quiet (exit code only)
-h              Suppress filename
-H              Always show filename
-e PATTERN      Pattern (repeatable)
-f FILE         Patterns from file
-A NUM          Lines after match
-B NUM          Lines before match
-C NUM          Context lines
-m NUM          Max matches per file
-t TYPE         File type filter (ts, py, go, rust, js, etc.)
-g GLOB         Glob filter
-j NUM          Threads
--hidden        Include hidden files
--gitignore     Respect .gitignore (off by default)
--include=GLOB  Include glob (grep compat)
--exclude=GLOB  Exclude glob (grep compat)
--exclude-dir=D Exclude directory (grep compat)
--color=WHEN    auto/always/never
```

## How it's fast

- **Match-jump search** — `regex.find_at()` jumps directly to matches instead of scanning every line. For sparse matches, this skips 99%+ of file content.
- **SIMD** — Rust `regex` + `memchr` crates use ARM NEON / x86 SSE2+AVX2 for byte scanning, auto-detected at runtime.
- **Memory-mapped I/O** — files >64KB use mmap (zero-copy), small files use buffered read.
- **Parallel directory walking** — `ignore` crate traverses directories across all CPU cores.
- **Zero-alloc output path** — `itoa` for line numbers, inline match ranges (no heap for ≤4 matches per line), thread-local reusable buffers, precomputed path prefixes.
- **Dedicated fast paths** — count, files-only, and quiet modes skip `Match` struct allocation entirely.
- **Raw byte paths** — `OsStr::as_bytes()` on Unix skips UTF-8 validation for file paths.

## Portability

Works on any platform Rust targets. No architecture-specific code:

| Component | ARM | x86_64 | Other |
|-----------|-----|--------|-------|
| `memchr` | NEON | SSE2/AVX2 | scalar fallback |
| `regex` | NEON | SSE2/AVX2 | scalar fallback |
| `mmap` | yes | yes | all Unix + Windows |

Cross-compile:

```bash
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu
```

## License

MIT
