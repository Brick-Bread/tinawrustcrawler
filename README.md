# TINAW Backstage code checker

A Rust checker for the Backstage mode of thisisnotawebsitedotcom.com. It tries evidence-led candidate phrases, prints every response, and saves the full body of each successful response. For successful HTML responses, it also saves up to 20 linked files from the site's first-party file host.

## Run

Install Rust and ensure `curl.exe` is on your PATH, then from this directory:

```powershell
cargo run --release
```

Results go to `results.jsonl`; successful responses and linked assets go to `downloads/`. The program resumes from its log. It skips candidates found in `known_codes.txt`, an optional `backstage_doc_snapshot.txt`, and prior result logs if present. The included candidate file uses documented patterns such as full production names, first and last names, exact titles, and reversed codes.

```powershell
cargo run --release -- --limit 100 --rate 2
cargo run --release -- --assets-only
cargo run --release -- --help
```

Each invocation is finite, with a maximum of 500 attempts, four workers, and four requests per second. HTTP 429 pauses the shared request schedule for 30 seconds before retrying. The program does not run indefinitely because an earlier automatic approval review rejected that request load.

