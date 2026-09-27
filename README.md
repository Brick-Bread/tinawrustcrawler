# TINAW Backstage code checker

A Rust checker for the Backstage mode of thisisnotawebsitedotcom.com. It tries evidence-led candidate phrases, prints every response, and saves the full body of each successful response. For successful HTML responses, it also saves linked files from the site's first-party file host.

## Run

Install Rust and ensure `curl.exe` is on your PATH, then from this directory:

```powershell
cargo run --release
```

Results go to `results.jsonl`. Each HTTP 200 also adds a tab-separated `phrase`, `code`, and saved response path to `successes.txt` (or the path given with `--successes`). Successful responses and linked assets go to `tinawrustcrawler-assets` in your Windows Downloads folder by default; `--downloads DIR` changes that location. All distinct first-party asset links found in a successful HTML response are attempted, without a count or file-size cap. In an interactive terminal, successful checks print in green and other responses print in red. The program resumes from its log. It skips candidates found in `known_codes.txt`, an optional `backstage_doc_snapshot.txt`, and prior result logs if present. The included candidate file uses documented patterns such as full production names, first and last names, exact titles, and reversed codes.

```powershell
cargo run --release -- --limit 100 --rate 2
cargo run --release -- --workers 8 --rate 10
cargo run --release -- --assets-only
cargo run --release -- --help
```

## Bounded load test

On Windows, run `run-load-test.cmd` in a separate terminal window. This mode generates `wip` codes from the empty suffix upward using `a-z`, `0-9`, `?`, `∞`, and `§`. It stops after one hour or 18,000 code attempts, whichever happens first. A shared gate limits all code and asset requests to five per second. A watchdog ends the process at the one-hour mark even if a request is still in progress. Responses go to `load_test_results.jsonl`; HTTP 200 codes go to `load_test_successes.txt`, and saved bodies and linked assets go to the Downloads folder described above.

The checker runs continuously until you press Ctrl+C. By default, it checks all currently untested candidates, then scans the wordlist and result logs every 30 seconds for new candidates. `--limit 0` means all candidates per scan; a positive `--limit` selects a smaller batch. Worker count and request rate have no built-in upper limits; choose values appropriate for the server. The defaults are four workers and four requests per second. HTTP 429 pauses the shared request schedule for 30 seconds before retrying. `--assets-only` completes after processing existing HTML responses.

