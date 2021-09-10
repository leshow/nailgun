# nailgun

`nailgun` is a DNS performance testing client written in Rust using `trust-dns-proto` and `tokio`. Open to PRs, issues, comments!

`nailgun` is written as a hobby project first and foremost, it's inspired by flamethrower and copies some of its arguments. It is single-threaded by default but configurable to use many threads. You can specify both the number of traffic generators (with `tcount`) and the number of tokio worker threads (with `wcount`), `nailgun` will start `tcount * wcount` generators. Most of the time, this is not necessary as 1 concurrent generator and a single worker thread is more than enough. Testing against a local `dnsdist` instance configured to return instantly with `NXDOMAIN`, (yes, not a real-world benchmark) `nailgun` can do well over 250K QPS (1 worker thread & generator).

`nailgun` uses `tracing` for logging so `RUST_LOG` can be used in order to control the logging output.

## Features

### Rate limiting

You can specify a specific QPS with `-Q`, this allows you to set a desired QPS rate which will be divided up over all the senders.

### Output

`--output` allows you to specify different logging formats, courtesy of `tracing`. "pretty", "json" and "debug" are currently supported with "pretty" as the default. You can use `RUST_LOG` to filter on the output (ex. `RUST_LOG="nailgun=trace"` will print trace level from the nailgun bin only). `nailgun` uses stdout by default, but will log to file if you accompany this with the `-o` flag and a path.

Regardless of these options, a summary is printed to stdout sans-tracing after the run is over.

### Generators

There are multiple generator types available with `-g`, the default is `static` but you can generate queries from file also or with some portion randomly generated.

## Usage

```
nailgun --help
```

By default, `nailgun` will spawn a single threaded tokio runtime and 1 traffic generator:

```
nailgun 0.0.0.0 -p 1953
```

This can be scaled up:

```
nailgun 0.0.0.0 -p 1953 -w 16 -t 1
```

Will spawn 16 OS threads (`w`/`wcount`) on the tokio runtime and 1 traffic generator (`t`/`tcount`) per thread spawned, for a total of 16\*1 = 16 traffic generators.

**Note** If you want a quick server to test against, I've been spinning up `dnsdist` and adding `addAction(AllRule(), RCodeAction(3))` to its config so that all DNS messages immediately return with `NXDomain`.

### DoH (new!)

```
nailgun dns.google -Q 2 -P doh
```

### Building & Installing

To build locally:

```
cargo build --release
```

`nailgun` binary will be present in `target/release/nailgun`

To install locally

```
cargo install
```

### TODO

- throttle based on # of timeouts
- first batch of messages always seems to be double the set QPS (try `--limit-secs 5 -Q 10` for example, it will end up sending 60 queries)
- some TODO cleanups left in the code to address
- arg parsing for generator is very different from flamethrower because clap doesn't seem to support parsing it in the same style-- would be nice to figure this out
- combine logging output from all generators rather than having them run individually
- more generator types?
- maybe leverage other parts of trustdns to make options for interesting types of queries. mDNS? DOH?
- suggestions? bugs?
