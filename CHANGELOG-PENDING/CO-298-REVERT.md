## CO-298 REVERT — remove `--staging` mode and fault-injection decorators

Removes `co serve --staging` and the four simulation decorators (`LatencyInjectedStorage`, `FlakyBlobStore`, `EvictingCache`, `RetryProneWorkerExecutor`) added in CO-298.

### Why
Random fault injection across every request doesn't simulate real production failures — prod fails in specific shapes (a particular endpoint OOMs, R2 rate-limits one bucket, a worker deadlocks under specific load). 5% generic 503s just makes dev annoying without surfacing real bugs. Tested code paths should be exercised with targeted unit/integration tests at known choke points, not probabilistic always-on injection.

Aligns with `feedback_no_uat.md` philosophy: direct-to-prod + smoke test + CHANGELOG rollback, rather than maintaining artificial intermediate environments that *feel* like fidelity but aren't.

### What stays
The trait foundation (`Storage`, `BlobStore`, `Cache`, `WorkerExecutor`, `AuthProvider`, `SecretsProvider`) and the TestServer testkit remain — those enable real backend swaps (R2 vs LocalFs, future OAuth, Redis cache, etc.). The decorators were the only piece reverted.
