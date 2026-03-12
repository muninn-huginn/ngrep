use std::env;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ignore::WalkBuilder;
use memchr::memchr;
use memmap2::Mmap;
use regex::bytes::{Regex, RegexBuilder};

// ── Constants ────────────────────────────────────────────────────────────────

const MMAP_THRESHOLD: u64 = 64 * 1024;
const BINARY_CHECK_LEN: usize = 8192;
const STDOUT_BUF_SIZE: usize = 64 * 1024; // 64KB stdout buffer

const C_PATH: &[u8] = b"\x1b[35m";
const C_LINE: &[u8] = b"\x1b[32m";
const C_SEP: &[u8] = b"\x1b[36m";
const C_MATCH: &[u8] = b"\x1b[1;31m";
const C_RESET: &[u8] = b"\x1b[0m";

// ── Config ───────────────────────────────────────────────────────────────────

struct Config {
    pattern: Regex,
    paths: Vec<PathBuf>,
    line_number: bool,
    files_only: bool,
    count: bool,
    invert: bool,
    only_matching: bool,
    quiet: bool,
    hidden: bool,
    no_ignore: bool,
    no_filename: bool,
    force_filename: bool,
    max_count: Option<usize>,
    color: bool,
    after_ctx: usize,
    before_ctx: usize,
    globs: Vec<String>,
    type_filters: Vec<String>,
    threads: usize,
    // Precomputed: do we need match ranges at all?
    need_ranges: bool,
    has_context: bool,
}

// ── Arg parsing ──────────────────────────────────────────────────────────────

fn parse_args() -> Config {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        eprintln!("usage: ngrep [OPTIONS] PATTERN [PATH ...]");
        process::exit(2);
    }

    let mut patterns: Vec<String> = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut ignore_case = false;
    let mut smart_case = true;
    let mut line_number = false;
    let mut files_only = false;
    let mut count = false;
    let mut invert = false;
    let mut word = false;
    let mut fixed = false;
    let mut only_matching = false;
    let mut quiet = false;
    let mut hidden = false;
    let mut no_ignore = true; // grep default: search everything
    let mut no_filename = false;
    let mut force_filename = false;
    let mut max_count: Option<usize> = None;
    let mut color_mode = String::from("auto");
    let mut after_ctx: usize = 0;
    let mut before_ctx: usize = 0;
    let mut globs: Vec<String> = Vec::new();
    let mut type_filters: Vec<String> = Vec::new();
    let mut threads: usize = 0;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            i += 1;
            while i < args.len() {
                paths.push(PathBuf::from(&args[i]));
                i += 1;
            }
            break;
        }

        if arg.starts_with("--") {
            match arg.as_str() {
                "--help" => { print_help(); process::exit(0); }
                "--version" => { println!("ngrep 0.1.0"); process::exit(0); }
                "--ignore-case" => ignore_case = true,
                "--smart-case" => smart_case = true,
                "--no-smart-case" => smart_case = false,
                "--line-number" => line_number = true,
                "--files-with-matches" => files_only = true,
                "--count" => count = true,
                "--invert-match" => invert = true,
                "--word-regexp" => word = true,
                "--fixed-strings" => fixed = true,
                "--only-matching" => only_matching = true,
                "--quiet" | "--silent" => quiet = true,
                "--hidden" => hidden = true,
                "--no-ignore" => no_ignore = true,
                "--gitignore" => no_ignore = false,
                "--no-filename" => no_filename = true,
                "--with-filename" => force_filename = true,
                "--recursive" | "--extended-regexp" | "--perl-regexp" | "--basic-regexp" => {}
                _ => {
                    if let Some(v) = arg.strip_prefix("--color=").or_else(|| arg.strip_prefix("--colour=")) {
                        color_mode = v.to_string();
                    } else if let Some(v) = arg.strip_prefix("--after-context=") {
                        after_ctx = v.parse().unwrap_or(0);
                    } else if let Some(v) = arg.strip_prefix("--before-context=") {
                        before_ctx = v.parse().unwrap_or(0);
                    } else if let Some(v) = arg.strip_prefix("--context=") {
                        let n = v.parse().unwrap_or(0);
                        before_ctx = n; after_ctx = n;
                    } else if let Some(v) = arg.strip_prefix("--max-count=") {
                        max_count = Some(v.parse().unwrap_or(0));
                    } else if let Some(v) = arg.strip_prefix("--include=") {
                        globs.push(v.to_string());
                    } else if let Some(v) = arg.strip_prefix("--exclude=") {
                        globs.push(format!("!{v}"));
                    } else if let Some(v) = arg.strip_prefix("--exclude-dir=") {
                        globs.push(format!("!{v}/"));
                    } else if let Some(v) = arg.strip_prefix("--glob=") {
                        globs.push(v.to_string());
                    } else if let Some(v) = arg.strip_prefix("--type=") {
                        type_filters.push(v.to_string());
                    } else if let Some(v) = arg.strip_prefix("--threads=") {
                        threads = v.parse().unwrap_or(0);
                    } else if let Some(v) = arg.strip_prefix("--regexp=") {
                        patterns.push(v.to_string());
                    } else {
                        let next = |i: &mut usize, args: &[String]| -> Option<String> {
                            *i += 1; args.get(*i).cloned()
                        };
                        match arg.as_str() {
                            "--color" | "--colour" => { if let Some(v) = next(&mut i, &args) { color_mode = v; } }
                            "--after-context" => { if let Some(v) = next(&mut i, &args) { after_ctx = v.parse().unwrap_or(0); } }
                            "--before-context" => { if let Some(v) = next(&mut i, &args) { before_ctx = v.parse().unwrap_or(0); } }
                            "--context" => { if let Some(v) = next(&mut i, &args) { let n = v.parse().unwrap_or(0); before_ctx = n; after_ctx = n; } }
                            "--max-count" => { if let Some(v) = next(&mut i, &args) { max_count = Some(v.parse().unwrap_or(0)); } }
                            "--include" => { if let Some(v) = next(&mut i, &args) { globs.push(v); } }
                            "--exclude" => { if let Some(v) = next(&mut i, &args) { globs.push(format!("!{v}")); } }
                            "--exclude-dir" => { if let Some(v) = next(&mut i, &args) { globs.push(format!("!{v}/")); } }
                            "--glob" => { if let Some(v) = next(&mut i, &args) { globs.push(v); } }
                            "--type" => { if let Some(v) = next(&mut i, &args) { type_filters.push(v); } }
                            "--threads" => { if let Some(v) = next(&mut i, &args) { threads = v.parse().unwrap_or(0); } }
                            "--regexp" => { if let Some(v) = next(&mut i, &args) { patterns.push(v); } }
                            _ => { eprintln!("ngrep: unknown option: {arg}"); process::exit(2); }
                        }
                    }
                }
            }
            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                let consume_rest_or_next = |j: usize, i: &mut usize, chars: &[char], args: &[String]| -> String {
                    if j + 1 < chars.len() {
                        chars[j + 1..].iter().collect()
                    } else {
                        *i += 1;
                        args.get(*i).cloned().unwrap_or_default()
                    }
                };
                match chars[j] {
                    'i' => ignore_case = true,
                    'n' => line_number = true,
                    'l' => files_only = true,
                    'c' => count = true,
                    'v' => invert = true,
                    'w' => word = true,
                    'F' => fixed = true,
                    'o' => only_matching = true,
                    'q' => quiet = true,
                    'H' => force_filename = true,
                    'h' => no_filename = true,
                    'r' | 'R' | 'E' | 'P' | 'G' | 's' => {}
                    'S' => smart_case = true,
                    'e' => { patterns.push(consume_rest_or_next(j, &mut i, &chars, &args)); j = chars.len(); continue; }
                    'f' => {
                        let val = consume_rest_or_next(j, &mut i, &chars, &args);
                        if let Ok(c) = std::fs::read_to_string(&val) {
                            for l in c.lines() { if !l.is_empty() { patterns.push(l.to_string()); } }
                        }
                        j = chars.len(); continue;
                    }
                    'A' => { after_ctx = consume_rest_or_next(j, &mut i, &chars, &args).parse().unwrap_or(0); j = chars.len(); continue; }
                    'B' => { before_ctx = consume_rest_or_next(j, &mut i, &chars, &args).parse().unwrap_or(0); j = chars.len(); continue; }
                    'C' => { let n = consume_rest_or_next(j, &mut i, &chars, &args).parse().unwrap_or(0); before_ctx = n; after_ctx = n; j = chars.len(); continue; }
                    'm' => { max_count = Some(consume_rest_or_next(j, &mut i, &chars, &args).parse().unwrap_or(0)); j = chars.len(); continue; }
                    't' => { type_filters.push(consume_rest_or_next(j, &mut i, &chars, &args)); j = chars.len(); continue; }
                    'g' => { globs.push(consume_rest_or_next(j, &mut i, &chars, &args)); j = chars.len(); continue; }
                    'j' => { threads = consume_rest_or_next(j, &mut i, &chars, &args).parse().unwrap_or(0); j = chars.len(); continue; }
                    _ => { eprintln!("ngrep: unknown option: -{}", chars[j]); process::exit(2); }
                }
                j += 1;
            }
            i += 1;
            continue;
        }

        if patterns.is_empty() { patterns.push(arg.clone()); } else { paths.push(PathBuf::from(arg)); }
        i += 1;
    }

    if patterns.is_empty() { eprintln!("ngrep: no pattern specified"); process::exit(2); }
    if paths.is_empty() && io::stdin().is_terminal() { paths.push(PathBuf::from(".")); }

    let mut combined = if patterns.len() == 1 {
        patterns.into_iter().next().unwrap()
    } else {
        patterns.into_iter().map(|p| format!("(?:{p})")).collect::<Vec<_>>().join("|")
    };
    if fixed { combined = regex::escape(&combined); }
    if word { combined = format!(r"\b(?:{combined})\b"); }

    let case_insensitive = ignore_case
        || (smart_case && !combined.contains(|c: char| c.is_ascii_uppercase()));

    let pattern = RegexBuilder::new(&combined)
        .case_insensitive(case_insensitive)
        .unicode(false)
        .build()
        .unwrap_or_else(|e| { eprintln!("ngrep: invalid pattern: {e}"); process::exit(2); });

    let color = match color_mode.as_str() {
        "always" => true, "never" => false, _ => io::stdout().is_terminal(),
    };
    let need_ranges = color || only_matching;
    let has_context = before_ctx > 0 || after_ctx > 0;

    Config {
        pattern, paths, line_number, files_only, count, invert,
        only_matching, quiet, hidden, no_ignore, no_filename, force_filename,
        max_count, color, after_ctx, before_ctx, globs, type_filters, threads,
        need_ranges, has_context,
    }
}

// ── File I/O ─────────────────────────────────────────────────────────────────

enum FileData { Mmap(Mmap), Buf(Vec<u8>) }

impl AsRef<[u8]> for FileData {
    fn as_ref(&self) -> &[u8] {
        match self { FileData::Mmap(m) => m, FileData::Buf(b) => b }
    }
}

#[inline]
fn read_file(path: &Path) -> io::Result<FileData> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 { return Ok(FileData::Buf(Vec::new())); }
    if len >= MMAP_THRESHOLD {
        Ok(FileData::Mmap(unsafe { Mmap::map(&file)? }))
    } else {
        let mut buf = Vec::with_capacity(len as usize);
        { let mut f = file; f.read_to_end(&mut buf)?; }
        Ok(FileData::Buf(buf))
    }
}

#[inline]
fn is_binary(data: &[u8]) -> bool {
    memchr(0, &data[..data.len().min(BINARY_CHECK_LEN)]).is_some()
}

// ── Search ───────────────────────────────────────────────────────────────────

struct Match {
    line_num: usize,
    line_start: usize,
    line_end: usize,
    // Inline small vec: most lines have 1-2 matches. Avoids heap allocation.
    ranges_inline: [(usize, usize); 4],
    ranges_len: u8,
    ranges_overflow: Option<Vec<(usize, usize)>>,
}

impl Match {
    #[inline]
    fn ranges(&self) -> &[(usize, usize)] {
        if let Some(ref v) = self.ranges_overflow {
            v
        } else {
            &self.ranges_inline[..self.ranges_len as usize]
        }
    }
}

/// Fast path: count matches only, no allocation
#[inline]
fn count_matches(data: &[u8], config: &Config) -> usize {
    if config.invert {
        return count_matches_linewise(data, config);
    }
    let mut count = 0;
    let mut search_from = 0;
    while let Some(m) = config.pattern.find_at(data, search_from) {
        count += 1;
        if let Some(max) = config.max_count {
            if count >= max { break; }
        }
        // Skip to next line
        let line_end = match memchr(b'\n', &data[m.start()..]) {
            Some(p) => m.start() + p,
            None => data.len(),
        };
        if line_end >= data.len() { break; }
        search_from = line_end + 1;
    }
    count
}

fn count_matches_linewise(data: &[u8], config: &Config) -> usize {
    let mut count = 0;
    let mut line_start = 0;
    loop {
        let line_end = match memchr(b'\n', &data[line_start..]) {
            Some(pos) => line_start + pos,
            None => data.len(),
        };
        let has_match = config.pattern.is_match(&data[line_start..line_end]);
        if config.invert != has_match {
            count += 1;
            if let Some(max) = config.max_count {
                if count >= max { break; }
            }
        }
        if line_end >= data.len() { break; }
        line_start = line_end + 1;
    }
    count
}

/// Fast path: does any match exist?
#[inline]
fn has_any_match(data: &[u8], config: &Config) -> bool {
    if config.invert {
        // Must scan lines
        let mut line_start = 0;
        loop {
            let line_end = match memchr(b'\n', &data[line_start..]) {
                Some(pos) => line_start + pos,
                None => data.len(),
            };
            if !config.pattern.is_match(&data[line_start..line_end]) {
                return true;
            }
            if line_end >= data.len() { break; }
            line_start = line_end + 1;
        }
        false
    } else {
        config.pattern.is_match(data)
    }
}

fn search_data(data: &[u8], config: &Config) -> Vec<Match> {
    if data.is_empty() { return Vec::new(); }
    if config.invert { return search_data_linewise(data, config); }

    let mut matches = Vec::new();
    let mut search_from = 0;
    let mut cur_line_num: usize = 1;
    let mut counted_up_to: usize = 0;

    while let Some(m) = config.pattern.find_at(data, search_from) {
        let line_start = match data[..m.start()].iter().rposition(|&b| b == b'\n') {
            Some(p) => p + 1, None => 0,
        };
        let line_end = match memchr(b'\n', &data[m.start()..]) {
            Some(p) => m.start() + p, None => data.len(),
        };

        // Incremental line counting via SIMD newline scan
        if line_start > counted_up_to {
            cur_line_num += memchr::memchr_iter(b'\n', &data[counted_up_to..line_start]).count();
        }
        counted_up_to = line_start;

        // Collect match ranges inline (no heap for <=4 matches per line)
        let mut ranges_inline = [(0usize, 0usize); 4];
        let mut ranges_len: u8 = 0;
        let mut ranges_overflow: Option<Vec<(usize, usize)>> = None;

        if config.need_ranges {
            let line = &data[line_start..line_end];
            for rm in config.pattern.find_iter(line) {
                if (ranges_len as usize) < 4 {
                    ranges_inline[ranges_len as usize] = (rm.start(), rm.end());
                    ranges_len += 1;
                } else {
                    // Spill to heap
                    let mut v: Vec<(usize, usize)> = ranges_inline.to_vec();
                    v.push((rm.start(), rm.end()));
                    for rm2 in config.pattern.find_iter(&line[rm.end()..]) {
                        v.push((rm.end() + rm2.start(), rm.end() + rm2.end()));
                    }
                    ranges_overflow = Some(v);
                    break;
                }
            }
        }

        matches.push(Match {
            line_num: cur_line_num, line_start, line_end,
            ranges_inline, ranges_len, ranges_overflow,
        });

        if let Some(max) = config.max_count {
            if matches.len() >= max { break; }
        }

        if line_end >= data.len() { break; }
        search_from = line_end + 1;
        cur_line_num += 1;
        counted_up_to = search_from;
    }

    matches
}

fn search_data_linewise(data: &[u8], config: &Config) -> Vec<Match> {
    let mut matches = Vec::new();
    let mut line_start = 0;
    let mut line_num: usize = 1;

    loop {
        let line_end = match memchr(b'\n', &data[line_start..]) {
            Some(pos) => line_start + pos, None => data.len(),
        };
        let has_match = config.pattern.is_match(&data[line_start..line_end]);
        if config.invert != has_match {
            matches.push(Match {
                line_num, line_start, line_end,
                ranges_inline: [(0, 0); 4], ranges_len: 0, ranges_overflow: None,
            });
            if let Some(max) = config.max_count {
                if matches.len() >= max { break; }
            }
        }
        if line_end >= data.len() { break; }
        line_start = line_end + 1;
        line_num += 1;
    }
    matches
}

// ── Output ───────────────────────────────────────────────────────────────────

/// Precomputed path prefix bytes (computed once per file, not per match)
struct PathPrefix {
    colored: Vec<u8>,   // "\x1b[35mpath\x1b[0m\x1b[36m:\x1b[0m"
    plain: Vec<u8>,     // "path:"
}

impl PathPrefix {
    fn new(path: &Path) -> Self {
        // Use raw OS bytes on Unix to skip UTF-8 validation
        #[cfg(unix)]
        let raw: &[u8] = {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes()
        };
        #[cfg(not(unix))]
        let raw: Vec<u8> = path.to_string_lossy().as_bytes().to_vec();
        #[cfg(not(unix))]
        let raw: &[u8] = &raw;

        // Strip leading "./"
        let bytes = if raw.starts_with(b"./") { &raw[2..] } else { raw };

        let mut colored = Vec::with_capacity(bytes.len() + 20);
        colored.extend_from_slice(C_PATH);
        colored.extend_from_slice(bytes);
        colored.extend_from_slice(C_RESET);
        // Separator added at write time

        let plain = bytes.to_vec();

        PathPrefix { colored, plain }
    }

    #[inline]
    fn write(&self, buf: &mut Vec<u8>, sep: u8, color: bool) {
        if color {
            buf.extend_from_slice(&self.colored);
            buf.extend_from_slice(C_SEP);
            buf.push(sep);
            buf.extend_from_slice(C_RESET);
        } else {
            buf.extend_from_slice(&self.plain);
            buf.push(sep);
        }
    }
}

#[inline]
fn write_linenum(buf: &mut Vec<u8>, num: usize, sep: u8, color: bool) {
    let mut itoa_buf = itoa::Buffer::new();
    let s = itoa_buf.format(num);
    if color {
        buf.extend_from_slice(C_LINE);
        buf.extend_from_slice(s.as_bytes());
        buf.extend_from_slice(C_RESET);
        buf.extend_from_slice(C_SEP);
        buf.push(sep);
        buf.extend_from_slice(C_RESET);
    } else {
        buf.extend_from_slice(s.as_bytes());
        buf.push(sep);
    }
}

fn format_results(
    buf: &mut Vec<u8>,
    data: &[u8],
    matches: &[Match],
    prefix: Option<&PathPrefix>,
    config: &Config,
    show_path: bool,
) {
    if config.quiet { return; }
    let show_name = show_path && !config.no_filename;

    // Count mode
    if config.count {
        if show_name { if let Some(p) = prefix { p.write(buf, b':', config.color); } }
        let mut itoa_buf = itoa::Buffer::new();
        buf.extend_from_slice(itoa_buf.format(matches.len()).as_bytes());
        buf.push(b'\n');
        return;
    }

    // Files-only mode
    if config.files_only {
        if !matches.is_empty() {
            if let Some(p) = prefix {
                if config.color {
                    buf.extend_from_slice(&p.colored);
                } else {
                    buf.extend_from_slice(&p.plain);
                }
                buf.push(b'\n');
            }
        }
        return;
    }

    // Context: precompute line offsets
    let all_lines: Vec<(usize, usize)> = if config.has_context {
        let mut v = Vec::new();
        let mut s = 0;
        loop {
            let e = match memchr(b'\n', &data[s..]) {
                Some(pos) => s + pos, None => data.len(),
            };
            v.push((s, e));
            if e >= data.len() { break; }
            s = e + 1;
        }
        v
    } else {
        Vec::new()
    };

    let mut last_printed: usize = 0;

    for (mi, m) in matches.iter().enumerate() {
        // Before context
        if config.has_context {
            let ctx_start = m.line_num.saturating_sub(config.before_ctx).max(1);
            if mi > 0 && ctx_start > last_printed + 1 {
                if config.color {
                    buf.extend_from_slice(C_SEP); buf.extend_from_slice(b"--"); buf.extend_from_slice(C_RESET);
                } else {
                    buf.extend_from_slice(b"--");
                }
                buf.push(b'\n');
            }
            for cl in ctx_start..m.line_num {
                if cl <= last_printed { continue; }
                let (ls, le) = all_lines[cl - 1];
                write_ctx_line(buf, &data[ls..le], prefix, cl, show_name, config);
            }
        }

        // Match line
        let line = &data[m.line_start..m.line_end];
        if config.only_matching {
            for &(ms, me) in m.ranges() {
                if show_name { if let Some(p) = prefix { p.write(buf, b':', config.color); } }
                if config.line_number { write_linenum(buf, m.line_num, b':', config.color); }
                if config.color {
                    buf.extend_from_slice(C_MATCH);
                    buf.extend_from_slice(&line[ms..me]);
                    buf.extend_from_slice(C_RESET);
                } else {
                    buf.extend_from_slice(&line[ms..me]);
                }
                buf.push(b'\n');
            }
        } else {
            if show_name { if let Some(p) = prefix { p.write(buf, b':', config.color); } }
            if config.line_number { write_linenum(buf, m.line_num, b':', config.color); }
            let ranges = m.ranges();
            if config.color && !ranges.is_empty() {
                write_highlighted(buf, line, ranges);
            } else {
                buf.extend_from_slice(line);
            }
            buf.push(b'\n');
        }

        last_printed = m.line_num;

        // After context
        if config.has_context {
            let ctx_end = (m.line_num + config.after_ctx).min(all_lines.len());
            let next_line = matches.get(mi + 1).map_or(usize::MAX, |nm| nm.line_num);
            for cl in (m.line_num + 1)..=ctx_end {
                if cl >= next_line { break; }
                let (ls, le) = all_lines[cl - 1];
                write_ctx_line(buf, &data[ls..le], prefix, cl, show_name, config);
                last_printed = cl;
            }
        }
    }
}

#[inline]
fn write_ctx_line(buf: &mut Vec<u8>, line: &[u8], prefix: Option<&PathPrefix>, line_num: usize, show_name: bool, config: &Config) {
    if show_name { if let Some(p) = prefix { p.write(buf, b'-', config.color); } }
    if config.line_number { write_linenum(buf, line_num, b'-', config.color); }
    buf.extend_from_slice(line);
    buf.push(b'\n');
}

#[inline]
fn write_highlighted(buf: &mut Vec<u8>, line: &[u8], ranges: &[(usize, usize)]) {
    let mut pos = 0;
    for &(start, end) in ranges {
        if start > pos { buf.extend_from_slice(&line[pos..start]); }
        buf.extend_from_slice(C_MATCH);
        buf.extend_from_slice(&line[start..end]);
        buf.extend_from_slice(C_RESET);
        pos = end;
    }
    if pos < line.len() { buf.extend_from_slice(&line[pos..]); }
}

// ── Stdin search ─────────────────────────────────────────────────────────────

fn search_stdin(config: &Config) -> bool {
    let mut data = Vec::new();
    io::stdin().read_to_end(&mut data).unwrap_or(0);

    if config.quiet { return has_any_match(&data, config); }

    if config.count {
        let c = count_matches(&data, config);
        let mut itoa_buf = itoa::Buffer::new();
        let stdout = io::stdout();
        let mut w = BufWriter::with_capacity(STDOUT_BUF_SIZE, stdout.lock());
        let _ = w.write_all(itoa_buf.format(c).as_bytes());
        let _ = w.write_all(b"\n");
        return c > 0;
    }

    let matches = search_data(&data, config);
    let found = !matches.is_empty();
    let mut out = Vec::with_capacity(4096);
    format_results(&mut out, &data, &matches, None, config, false);
    let stdout = io::stdout();
    let mut w = BufWriter::with_capacity(STDOUT_BUF_SIZE, stdout.lock());
    let _ = w.write_all(&out);
    found
}

// ── Streaming search+output (single pass, no Match allocation) ───────────────

const BATCH_THRESHOLD: usize = 32 * 1024; // flush thread-local buffer at 32KB

/// Batched output writer — accumulates output in a thread-local buffer,
/// flushes to shared stdout at threshold or on Drop.
struct BatchWriter {
    buf: Vec<u8>,
    stdout: Arc<Mutex<BufWriter<io::Stdout>>>,
}

impl BatchWriter {
    fn new(stdout: Arc<Mutex<BufWriter<io::Stdout>>>) -> Self {
        BatchWriter { buf: Vec::with_capacity(BATCH_THRESHOLD + 4096), stdout }
    }

    #[inline]
    fn maybe_flush(&mut self) {
        if self.buf.len() >= BATCH_THRESHOLD {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.buf.is_empty() {
            if let Ok(mut out) = self.stdout.lock() {
                let _ = out.write_all(&self.buf);
            }
            self.buf.clear();
        }
    }
}

impl Drop for BatchWriter {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Get raw path bytes with "./" stripped, no heap allocation.
#[inline]
fn path_bytes(path: &Path) -> &[u8] {
    #[cfg(unix)]
    let raw: &[u8] = { use std::os::unix::ffi::OsStrExt; path.as_os_str().as_bytes() };
    #[cfg(not(unix))]
    let raw: &[u8] = path.to_string_lossy().as_bytes();
    if raw.starts_with(b"./") { &raw[2..] } else { raw }
}

/// Write path prefix directly to buffer (no PathPrefix struct allocation).
#[inline]
fn write_path(buf: &mut Vec<u8>, path: &[u8], sep: u8, color: bool) {
    if color {
        buf.extend_from_slice(C_PATH);
        buf.extend_from_slice(path);
        buf.extend_from_slice(C_RESET);
        buf.extend_from_slice(C_SEP);
        buf.push(sep);
        buf.extend_from_slice(C_RESET);
    } else {
        buf.extend_from_slice(path);
        buf.push(sep);
    }
}

/// Single-pass streaming search + output. No Match struct allocation.
/// Returns true if any match found.
#[inline]
fn search_and_stream(
    data: &[u8],
    buf: &mut Vec<u8>,
    path: &[u8],
    show_path: bool,
    config: &Config,
) -> bool {
    if data.is_empty() { return false; }

    let mut found = false;
    let mut search_from = 0;
    let mut cur_line_num: usize = 1;
    let mut counted_up_to: usize = 0;
    let mut match_count: usize = 0;
    let show_name = show_path && !config.no_filename;

    while let Some(m) = config.pattern.find_at(data, search_from) {
        found = true;

        // Line boundaries
        let line_start = match data[..m.start()].iter().rposition(|&b| b == b'\n') {
            Some(p) => p + 1, None => 0,
        };
        let line_end = match memchr(b'\n', &data[m.start()..]) {
            Some(p) => m.start() + p, None => data.len(),
        };

        // Line number
        if line_start > counted_up_to {
            cur_line_num += memchr::memchr_iter(b'\n', &data[counted_up_to..line_start]).count();
        }
        counted_up_to = line_start;

        let line = &data[line_start..line_end];

        // Write: [path:][linenum:]content\n
        if show_name { write_path(buf, path, b':', config.color); }
        if config.line_number { write_linenum(buf, cur_line_num, b':', config.color); }

        if config.only_matching {
            // Write each match on this line separately
            for rm in config.pattern.find_iter(line) {
                if config.color {
                    buf.extend_from_slice(C_MATCH);
                    buf.extend_from_slice(&line[rm.start()..rm.end()]);
                    buf.extend_from_slice(C_RESET);
                } else {
                    buf.extend_from_slice(&line[rm.start()..rm.end()]);
                }
                buf.push(b'\n');
                // Re-emit prefix for each subsequent match
                match_count += 1;
                if let Some(max) = config.max_count {
                    if match_count >= max { return true; }
                }
            }
            // Already handled count + newlines above, skip the common path below
            if line_end >= data.len() { break; }
            search_from = line_end + 1;
            cur_line_num += 1;
            counted_up_to = search_from;
            continue;
        }

        if config.color {
            // Highlight all matches on this line in one pass
            let mut pos = 0;
            for rm in config.pattern.find_iter(line) {
                if rm.start() > pos { buf.extend_from_slice(&line[pos..rm.start()]); }
                buf.extend_from_slice(C_MATCH);
                buf.extend_from_slice(&line[rm.start()..rm.end()]);
                buf.extend_from_slice(C_RESET);
                pos = rm.end();
            }
            if pos < line.len() { buf.extend_from_slice(&line[pos..]); }
        } else {
            buf.extend_from_slice(line);
        }
        buf.push(b'\n');

        match_count += 1;
        if let Some(max) = config.max_count {
            if match_count >= max { break; }
        }
        if line_end >= data.len() { break; }
        search_from = line_end + 1;
        cur_line_num += 1;
        counted_up_to = search_from;
    }

    found
}

// ── Parallel file search ─────────────────────────────────────────────────────

fn search_paths(config: &Config) -> bool {
    let found = AtomicBool::new(false);
    let multi = config.paths.len() > 1
        || config.paths.iter().any(|p| p.is_dir())
        || config.force_filename;
    let stdout = Arc::new(Mutex::new(BufWriter::with_capacity(STDOUT_BUF_SIZE, io::stdout())));

    let mut builder = if config.paths.is_empty() {
        WalkBuilder::new(".")
    } else {
        let mut b = WalkBuilder::new(&config.paths[0]);
        for p in &config.paths[1..] { b.add(p); }
        b
    };

    builder
        .hidden(!config.hidden)
        .git_ignore(!config.no_ignore)
        .git_global(!config.no_ignore)
        .git_exclude(!config.no_ignore);

    if config.threads > 0 { builder.threads(config.threads); }

    if !config.type_filters.is_empty() {
        let mut tb = ignore::types::TypesBuilder::new();
        tb.add_defaults();
        for t in &config.type_filters { tb.select(t); }
        if let Ok(types) = tb.build() { builder.types(types); }
    }

    if !config.globs.is_empty() {
        let mut ob = ignore::overrides::OverrideBuilder::new(".");
        for g in &config.globs { let _ = ob.add(g); }
        if let Ok(ov) = ob.build() { builder.overrides(ov); }
    }

    // Can we use the streaming fast path?
    let use_streaming = !config.invert && !config.has_context && !config.count
        && !config.files_only && !config.quiet;

    builder.build_parallel().run(|| {
        let config = &config;
        let found = &found;
        let show_path = multi;
        let mut bw = BatchWriter::new(stdout.clone());

        Box::new(move |entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };
            if entry.file_type().map_or(true, |ft| ft.is_dir()) {
                return ignore::WalkState::Continue;
            }

            let path = entry.path();
            let data = match read_file(path) {
                Ok(d) => d,
                Err(_) => return ignore::WalkState::Continue,
            };
            let bytes = data.as_ref();
            if bytes.is_empty() || is_binary(bytes) {
                return ignore::WalkState::Continue;
            }

            let pb = path_bytes(path);

            // Fast paths
            if config.quiet {
                if has_any_match(bytes, config) {
                    found.store(true, Ordering::Relaxed);
                    return ignore::WalkState::Quit;
                }
                return ignore::WalkState::Continue;
            }

            if config.files_only {
                if has_any_match(bytes, config) {
                    found.store(true, Ordering::Relaxed);
                    if config.color {
                        bw.buf.extend_from_slice(C_PATH);
                        bw.buf.extend_from_slice(pb);
                        bw.buf.extend_from_slice(C_RESET);
                    } else {
                        bw.buf.extend_from_slice(pb);
                    }
                    bw.buf.push(b'\n');
                    bw.maybe_flush();
                }
                return ignore::WalkState::Continue;
            }

            if config.count {
                let c = count_matches(bytes, config);
                if c > 0 { found.store(true, Ordering::Relaxed); }
                if show_path && !config.no_filename {
                    write_path(&mut bw.buf, pb, b':', config.color);
                }
                let mut itoa_buf = itoa::Buffer::new();
                bw.buf.extend_from_slice(itoa_buf.format(c).as_bytes());
                bw.buf.push(b'\n');
                bw.maybe_flush();
                return ignore::WalkState::Continue;
            }

            // Streaming search+output (common case: no context, no invert)
            if use_streaming {
                if search_and_stream(bytes, &mut bw.buf, pb, show_path, config) {
                    found.store(true, Ordering::Relaxed);
                }
                bw.maybe_flush();
                return ignore::WalkState::Continue;
            }

            // Fallback: collect matches then format (context, invert modes)
            let matches = search_data(bytes, config);
            if matches.is_empty() {
                return ignore::WalkState::Continue;
            }
            found.store(true, Ordering::Relaxed);

            let prefix = PathPrefix::new(path);
            format_results(&mut bw.buf, bytes, &matches, Some(&prefix), config, show_path);
            bw.maybe_flush();

            ignore::WalkState::Continue
        })
    });

    found.load(Ordering::Relaxed)
}

// ── Help ─────────────────────────────────────────────────────────────────────

fn print_help() {
    print!(r#"ngrep 0.1.0 — fast grep

USAGE:
    ngrep [OPTIONS] PATTERN [PATH ...]
    command | ngrep [OPTIONS] PATTERN

Recursive by default. Respects .gitignore. Skips binaries. Smart-case.

OPTIONS:
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
    -t TYPE         File type filter
    -g GLOB         Glob filter
    -j NUM          Threads
    --hidden        Include hidden files
    --gitignore     Respect .gitignore (off by default, like grep)
    --include=GLOB  Include glob (grep compat)
    --exclude=GLOB  Exclude glob (grep compat)
    --exclude-dir=D Exclude dir (grep compat)
    --color=WHEN    auto/always/never
"#);
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    #[cfg(unix)]
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL); }

    let config = parse_args();

    let found = if config.paths.is_empty() && !io::stdin().is_terminal() {
        search_stdin(&config)
    } else {
        search_paths(&config)
    };

    process::exit(if found { 0 } else { 1 });
}
