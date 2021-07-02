# nailgun (WIP)

`nailgun` is a DNS performance testing client written in Rust using `trust-dns` and `tokio`. Open to PRs, issues, comments!

`nailgun` is written as a hobby project first and foremost, it's inspired by flamethrower (as you may have guessed from the name) and copies some of its arguments. It differs in that is single-threaded by default but configurable to use many threads. You can specify both the number of traffic generators (with `tcount`) and the number of worker threads (with `wcount`), `nailgun` will start `tcount * wcount` generators. Most of the time, this is not necessary as it is quite fast and 1 concurrent generator with a single worker thread for the tokio runtime is more than enough. Testing against a local dnsdist instance configured to return instantly with `NXDOMAIN` (yes, not representative of real-world benchmarking) `nailgun` can do well over 250K QPS with a single traffic generator.

## Usage

By default `nailgun` will spawn a single threaded tokio runtime and 1 traffic generator:

```
nailgun -b 0.0.0.0 -p 1953
```

This can be scaled up:

```
nailgun -b 0.0.0.0 -p 1953 -w 16 -t 1
```

Will spawn 16 OS threads (`w`/`wcount`) on the tokio runtime and 1 traffic generator (`t`/`tcount`) per thread spawned, for a total of 16\*1 = 16 traffic generators.
