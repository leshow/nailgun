# nailgun

`nailgun` is a DNS performance testing client in the vein of flamethrower or dnsperf but written in Rust using `trust-dns` and `tokio`.

## Usage

By default `nailgun` will spawn a single threaded tokio runtime and 10 traffic generators:

```
nailgun -b 0.0.0.0 -p 9953
```

(default single threaded runtime and 10 traffic generators)

This can easily be scaled up:

```
nailgun -b 0.0.0.0 -p 9953 -wcount 16 -tcount 10
```

Will spawn 16 OS threads (`wcount`) for the tokio runtime and 10 traffic generators (`tcount`) per thread spawned, for a total of 16\*10 = 160 traffic generators.

Obviously, if you are testing and using up all the available IO/CPU on your machine, make sure you're hitting a host that's not on the same box, as you'll be eating lots of resources just generating the traffic.
