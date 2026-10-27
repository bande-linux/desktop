---
id: "task-000"
title: "Imperative task description"
repository: "compositor" # desktop | compositor | shell | greeter | portal | toolkit | apps | factory | runners
priority: "P1" # P0 | P1 | P2 | P3
component: "crate-or-subsystem-name"
dependencies: [] # e.g. ["task-001", "task-002"]
verification_tier: 3 # 0 to 5
session_reuse_compatible: true
discovered_by:
  agent_role: "worker" # worker | reviewer | planner
  origin_task: "task-000"
  commit: "HEAD"
---

### Problem & Objective
<!-- Clear, concise description of the functional deliverable or bug fix -->

### Acceptance Criteria
- [ ] Criterion 1 (e.g. Unit test asserting invariant)
- [ ] Criterion 2 (e.g. Clean `cargo clippy` without warnings)
- [ ] Criterion 3 (e.g. Mutation test kill score >= 90%)

### Architecture & Interface Notes
<!-- Reference relevant sections in docs/ARCHITECTURE.md -->

### Test Plan
<!-- Explicit proptest or offscreen render verification details -->
