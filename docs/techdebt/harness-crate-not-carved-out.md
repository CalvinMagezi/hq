# The harness tools are still inside hq-tools, not a crate of their own

**Reason skipped:** the numbers after the extractions do not justify the carve. The plan's decision G2 said to carve a harness crate only if CI stays above target once the cheap extractions are in; they are mostly in, and the measured gain from further carving is small.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/harness_session/`, `crates/hq-tools/src/agent_host/`, `crates/hq-tools/layers.toml` (the remaining `allowed_back_edges`)
**Remove when:** a re-measure shows a carved crate would take `hq-tools` out of the rebuild path of a large share of commits, or the three remaining back edges are cut (`harness_session -> a2a`, `agent_host -> harness_session`, `harness_session -> lane_quotas`), whichever comes first.

What was measured (`python3 -I scripts/ci/affected-report.py --commits 400`, `python3 -I scripts/ci/module-graph.py`), 2026-11-18, after the foundation, web, office and test-module extractions and four back-edge cuts:

- Median share of the workspace rebuilt per commit: 72% (75% at the start). The leaders are `hq-core` (88% of commits rebuild it) and `hq-db` (71%), which every tool crate depends on; `hq-tools` itself is rebuilt by 53%.
- The tangle in `hq-tools` was 16 modules and 75% of its lines at that measurement (17 modules and 76,091 lines at the start). After the later D rows it is 7 modules and 13% (12,532 lines); the layering allowlist is empty: every module reaches only its own layer or lower.
- The suite family of commits fell from a 58% median to 20% once `office_file` left `hq-tools` (that carve was worth doing because it removed a whole family of unrelated rebuilds).

Why not a harness crate now: most of it depends on `tasks`, `a2a`, `lane_quotas` and `helper_setup`, which depend back on it, so a crate boundary would need the three remaining cuts first (a registration or injection design for each), and the gain is bounded by how often the harness code changes, not by the 98k lines it shares a crate with. Splitting CI test time (nextest partitions) and keeping `hq-core` and `hq-db` quiet would move the median more than another carve.
