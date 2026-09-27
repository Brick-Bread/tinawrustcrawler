//! Bounded Backstage code checker. Saves the complete body of every HTTP 200.
//! Uses Windows' curl.exe for multipart HTTPS; no Cargo dependencies are needed.

use std::collections::HashSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ENDPOINT: &str = "https://codes.thisisnotawebsitedotcom.com/";

#[derive(Clone)]
struct Options {
    wordlist: PathBuf,
    known: PathBuf,
    doc_file: PathBuf,
    results: PathBuf,
    downloads: PathBuf,
    skip_logs: Vec<PathBuf>,
    limit: usize,
    workers: usize,
    rate: f64,
    timeout: u64,
    assets_only: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            wordlist: PathBuf::from("backstage_reference_candidates.txt"),
            known: PathBuf::from("known_codes.txt"),
            doc_file: PathBuf::from("backstage_doc_snapshot.txt"),
            results: PathBuf::from("results.jsonl"),
            downloads: PathBuf::from("downloads"),
            skip_logs: vec![
                PathBuf::from("backstage_500_results.jsonl"),
                PathBuf::from("backstage_next_results.jsonl"),
                PathBuf::from("backstage_fast_results.jsonl"),
                PathBuf::from("backstage_live_results.jsonl"),
            ],
            limit: 500,
            workers: 4,
            rate: 4.0,
            timeout: 20,
            assets_only: false,
        }
    }
}

fn options() -> Result<Options, String> {
    let mut o = Options::default();
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!("Usage: cargo run --release -- [--assets-only] [--wordlist FILE] [--known FILE] [--doc-file FILE] [--results FILE] [--downloads DIR] [--skip-log FILE] [--limit 1..500] [--workers 1..4] [--rate 0..4] [--timeout SECONDS]");
            std::process::exit(0);
        }
        if flag == "--assets-only" { o.assets_only = true; continue; }
        let value = args.next().ok_or_else(|| format!("Missing value for {flag}"))?;
        match flag.as_str() {
            "--wordlist" => o.wordlist = value.into(),
            "--known" => o.known = value.into(),
            "--doc-file" => o.doc_file = value.into(),
            "--results" => o.results = value.into(),
            "--downloads" => o.downloads = value.into(),
            "--skip-log" => o.skip_logs.push(value.into()),
            "--limit" => o.limit = value.parse().map_err(|_| "Invalid limit")?,
            "--workers" => o.workers = value.parse().map_err(|_| "Invalid workers")?,
            "--rate" => o.rate = value.parse().map_err(|_| "Invalid rate")?,
            "--timeout" => o.timeout = value.parse().map_err(|_| "Invalid timeout")?,
            _ => return Err(format!("Unknown option: {flag}")),
        }
    }
    if !(1..=500).contains(&o.limit)
        || !(1..=4).contains(&o.workers)
        || !(o.rate > 0.0 && o.rate <= 4.0)
        || o.timeout == 0
    {
        return Err("Use limit 1..500, workers 1..4, rate >0..4, timeout >0".into());
    }
    Ok(o)
}

fn normalize(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(*c, '?' | '∞' | '§'))
        .collect()
}

fn lines(path: &Path) -> io::Result<Vec<String>> {
    let file = File::open(path)?;
    BufReader::new(file)
        .lines()
        .filter_map(|line| match line {
            Ok(text) if text.trim().is_empty() || text.trim_start().starts_with('#') => None,
            other => Some(other.map(|s| s.trim().to_owned())),
        })
        .collect()
}

fn logged_codes(path: &Path, out: &mut HashSet<String>) -> io::Result<()> {
    if !path.exists() { return Ok(()); }
    for line in lines(path)? {
        let status = line.find("\"status\":")
            .and_then(|at| line[at + 9..].trim_start().split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse::<u16>().ok());
        if !matches!(status, Some(200 | 404)) { continue; }
        if let Some(start) = line.find("\"code\":") {
            let rest = line[start + 7..].trim_start();
            if let Some(value) = rest.strip_prefix('"') {
                if let Some(end) = value.find('"') {
                    out.insert(value[..end].to_owned());
                }
            }
        }
    }
    Ok(())
}

fn consider(phrase: String, seen: &mut HashSet<String>, done: &HashSet<String>,
            known: &HashSet<String>, doc: &[String], output: &mut Vec<(String, String)>) {
    let normalized = normalize(&phrase);
    let code = format!("wip{normalized}");
    if normalized.len() < 2 || !seen.insert(normalized.clone()) || done.contains(&code)
        || known.contains(&normalized) || doc.iter().any(|line| line.contains(&normalized))
    { return; }
    output.push((phrase, code));
}

fn candidates(o: &Options) -> io::Result<Vec<(String, String)>> {
    let words = lines(&o.wordlist)?;
    let known: HashSet<String> = lines(&o.known)?.iter().map(|x| normalize(x)).collect();
    let doc: Vec<String> = if o.doc_file.exists() {
        lines(&o.doc_file)?.iter().map(|x| normalize(x)).collect()
    } else { Vec::new() };
    let mut done = HashSet::new();
    logged_codes(&o.results, &mut done)?;
    for path in &o.skip_logs { logged_codes(path, &mut done)?; }
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for seed in &words {
        let (kind, phrase) = seed.split_once('|').unwrap_or(("phrase", seed));
        let phrase = phrase.trim();
        let mut variants = vec![phrase.to_owned()];
        match kind.trim() {
            // The observed full-name hits also work under first or last names.
            "name" => {
                let parts: Vec<&str> = phrase.split_whitespace().collect();
                if parts.len() > 1 {
                    variants.push(parts[parts.len() - 1].to_owned());
                    variants.push(parts[0].to_owned());
                }
            }
            // Old codes include exact titles with and without articles.
            "title" => {
                if let Some(short) = phrase.strip_prefix("The ").or_else(|| phrase.strip_prefix("A ")) {
                    variants.push(short.to_owned());
                }
            }
            // Reverse spellings occur among the documented codes.
            "reverse" => variants.push(normalize(phrase).chars().rev().collect()),
            "phrase" => (),
            other => {
                eprintln!("Skipping unknown seed category: {other}");
                continue;
            }
        }
        for variant in variants {
            consider(variant, &mut seen, &done, &known, &doc, &mut output);
            if output.len() >= o.limit { return Ok(output); }
        }
    }
    Ok(output)
}

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Gate { next: Instant }
impl Gate {
    fn wait(&mut self, rate: f64) -> Duration {
        let now = Instant::now();
        let slot = self.next.max(now);
        self.next = slot + Duration::from_secs_f64(1.0 / rate);
        slot.saturating_duration_since(now)
    }
    fn backoff(&mut self, delay: Duration) {
        self.next = self.next.max(Instant::now() + delay);
    }
}

fn extension(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or("").trim() {
        "text/html" => "html",
        "text/plain" => "txt",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

fn filename_stem(code: &str) -> String {
    code.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn save_assets(html_path: &Path, stem: &str, o: &Options, gate: &Mutex<Gate>) -> Vec<String> {
    const PREFIX: &str = "https://files.thisisnotawebsitedotcom.com/";
    let html = match fs::read_to_string(html_path) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut saved = Vec::new();
    for (position, _) in html.match_indices(PREFIX) {
        if seen.len() >= 20 { break; }
        let rest = &html[position..];
        let end = rest.find(|c: char| matches!(c, '"' | '\'' | '<' | '>' | ' ' | '\r' | '\n'))
            .unwrap_or(rest.len());
        let url = &rest[..end];
        if !seen.insert(url.to_owned()) { continue; }
        let basename = url.rsplit('/').next().unwrap_or("asset.bin").split('?').next().unwrap_or("asset.bin");
        let safe_name: String = basename.chars().map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }
        }).collect();
        let target = o.downloads.join(format!("{stem}_asset_{:02}_{safe_name}", seen.len()));
        if target.exists() {
            saved.push(target.to_string_lossy().into_owned());
            continue;
        }
        let pause = gate.lock().unwrap().wait(o.rate);
        thread::sleep(pause);
        let result = Command::new("curl.exe")
            .args(["--silent", "--show-error", "--fail", "--location", "--max-time", "60",
                "--max-filesize", "25000000", "--output"])
            .arg(&target)
            .arg(url)
            .output();
        match result {
            Ok(result) if result.status.success() => {
                println!("       asset -> {}", target.display());
                saved.push(target.to_string_lossy().into_owned());
            }
            _ => { let _ = fs::remove_file(&target); }
        }
    }
    saved
}

fn check(index: usize, phrase: &str, code: &str, o: &Options, gate: &Mutex<Gate>) -> String {
    let stem = filename_stem(code);
    let temporary = o.downloads.join(format!(".{stem}.{index}.part"));
    for attempt in 1..=3 {
        let pause = gate.lock().unwrap().wait(o.rate);
        thread::sleep(pause);
        let response = Command::new("curl.exe")
            .args(["--silent", "--show-error", "--location", "--max-redirs", "3",
                "--max-time", &o.timeout.to_string(), "--request", "POST",
                "--form", &format!("code={code}"), "--output"])
            .arg(&temporary)
            .args(["--write-out", "%{http_code}\t%{content_type}", ENDPOINT])
            .output();
        let (status, mime, error) = match response {
            Ok(result) => {
                let s = String::from_utf8_lossy(&result.stdout);
                let mut parts = s.splitn(2, '\t');
                let status = if result.status.success() { parts.next().unwrap_or("0").trim().parse::<u16>().unwrap_or(0) } else { 0 };
                let mime = parts.next().unwrap_or("").trim().to_owned();
                let error = String::from_utf8_lossy(&result.stderr).trim().to_owned();
                (status, mime, error)
            }
            Err(e) => (0, String::new(), e.to_string()),
        };
        if status == 429 && attempt < 3 {
            let delay = Duration::from_secs(30);
            gate.lock().unwrap().backoff(delay);
            let _ = fs::remove_file(&temporary);
            continue;
        }
        let mut saved = String::new();
        let mut bytes = 0;
        let mut assets = Vec::new();
        if status == 200 {
            let target = o.downloads.join(format!("{stem}.{}", extension(&mime)));
            if let Ok(metadata) = fs::metadata(&temporary) { bytes = metadata.len(); }
            match fs::rename(&temporary, &target) {
                Ok(()) => {
                    saved = target.to_string_lossy().into_owned();
                    if mime.starts_with("text/html") { assets = save_assets(&target, &stem, o, gate); }
                }
                Err(e) => saved = format!("SAVE ERROR: {e}"),
            }
        } else {
            let _ = fs::remove_file(&temporary);
        }
        println!("[{index:04}] {phrase} -> {code} -> HTTP {status} | {mime} | {bytes} bytes | {saved} {error}");
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let assets_json = assets.iter().map(|s| json(s)).collect::<Vec<_>>().join(",");
        return format!("{{\"checked_at_unix\":{timestamp},\"phrase\":{},\"code\":{},\"status\":{status},\"content_type\":{},\"bytes\":{bytes},\"saved\":{},\"assets\":[{assets_json}],\"error\":{}}}\n",
            json(phrase), json(code), json(&mime), json(&saved), json(&error));
    }
    unreachable!()
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let o = options()?;
    if o.assets_only {
        let gate = Mutex::new(Gate { next: Instant::now() });
        let mut total = 0;
        for entry in fs::read_dir(&o.downloads)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "html") {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                total += save_assets(&path, &stem, &o, &gate).len();
            }
        }
        println!("Saved or confirmed {total} first-party assets from existing HTML responses.");
        return Ok(());
    }
    let work = candidates(&o)?;
    if work.is_empty() {
        println!("No untested candidates in this batch.");
        return Ok(());
    }
    fs::create_dir_all(&o.downloads)?;
    if let Some(parent) = o.results.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let log = Arc::new(Mutex::new(OpenOptions::new().create(true).append(true).open(&o.results)?));
    let gate = Arc::new(Mutex::new(Gate { next: Instant::now() }));
    let index = Arc::new(AtomicUsize::new(0));
    let work = Arc::new(work);
    let o = Arc::new(o);
    println!("Checking {} codes at up to {} requests/second; saving every HTTP 200 to {}", work.len(), o.rate, o.downloads.display());
    thread::scope(|scope| {
        for _ in 0..o.workers {
            let (work, index, gate, log, o) = (work.clone(), index.clone(), gate.clone(), log.clone(), o.clone());
            scope.spawn(move || loop {
                let i = index.fetch_add(1, Ordering::Relaxed);
                if i >= work.len() { break; }
                let (phrase, code) = &work[i];
                let record = check(i + 1, phrase, code, &o, &gate);
                let mut file = log.lock().unwrap();
                if let Err(e) = file.write_all(record.as_bytes()).and_then(|_| file.flush()) {
                    eprintln!("Could not write result log: {e}");
                }
            });
        }
    });
    println!("Finished this bounded batch. Results: {}", o.results.display());
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

