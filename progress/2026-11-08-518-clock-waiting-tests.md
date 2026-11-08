# #518 clock-waiting tests no longer hold test slots (2026-11-08)

- `.config/nextest.toml`: a `working` group capped at eight, `test-threads = 33` in the `ci` profile, and a filter that puts everything but the named clock-waiting tests in the group.
- `tests/scratch/tests/ci.rs`: `only_clock_waiting_tests_run_beside_the_cap_of_eight`.
- Simulated from 22 full runs: 620 s becomes 293 s (the simulation runs a little above the real 556 s). This pull request's own run is the measurement.
