# LUMYX_GOSL — LUMYX Operating Standard

The operating standard for building the LUMYX LED pixel platform. Architecture lives in
the `led-strip-platform` skill suite (`~/led-strip-platform-skill/`); this document governs
*how a unit of work is allowed to close*.

---

## Definition of Done

> **A task is DONE only when all four gates — compliance, perf, protocol, seam — have
> passed, AND `CLAUDE.md` has been updated. No gate is optional. "It compiles" is not
> done; "the test I wrote is green" is not done until all four gates are green.**

If any gate cannot be evaluated for a given change, that is itself a failing gate: add the
check (a test, a measurement) that lets it be evaluated, or the task is not done.

### Gate 1 — Compliance (the constitution holds)

The cross-cutting invariants (`led-strip-platform` master §4) are intact:

- **Logical/physical split** — effects/sequencer/AI work only in logical space; the **one
  mapping is applied exactly once, at the HAL**; no driver re-maps; nothing above the HAL
  names a socket, universe, or channel.
- **Core never touches hardware** — only `Arc<dyn ProtocolOutput>`.
- **Never stop sending** — heartbeat ≥1 Hz, independent of sequencer state, resends the
  **last valid frame, never zeros**.
- **AI is deterministic / design-time only** — same inputs ⇒ same show; no LLM in the
  frame loop.
- **Audio** — Hann window before every FFT; `sample_rate` explicit, never hardcoded.

**Verify:** the invariant test(s) the change touches are green. If the change *found* a
violation, the fix is routed to `led-self-improvement` so it becomes a permanent invariant,
not a one-off.

### Gate 2 — Perf (within budget, no regression)

- The change stays inside the latency budget (master §6) for the layers it touches.
- **No allocation on the hot path** (render/map/serialize/send per-frame).
- Any "it's fast" claim is backed by a **measurement**, not by assertion.

**Verify:** the allocation test (counting-allocator, steady-state zero) is green for hot
paths; lock-free code passes its stress test and, where `unsafe`, **Miri** (incl.
`-Zmiri-many-seeds`) is clean.

### Gate 3 — Protocol (correct on the wire)

- Output packets are well-formed for their protocol (E1.31/sACN, Art-Net, DDP).
- **Per-universe** wrapping sequence numbers (never global); one universe per datagram.
- Nothing that would trip a controller into safe mode.

**Verify:** a wire-format test that **parses the actual bytes sent** (not just "send
returned Ok") is green.

### Gate 4 — Seam (contracts honored)

- The shared seam types (master §3) are respected and unbroken: `LogicalFrame`,
  `ProtocolOutput`, `DeviceDriver`/`IDevice`, `CompiledLayout`, `AudioFeatures`,
  `ShowIntent`, `LayoutIntent`, `SharedContext`.
- Each side of a seam remains **testable in isolation** against a fake (e.g. a
  `SimulatorDevice`).
- Any change to a seam type is **deliberate, made in one place**, and reflected on both
  sides + in `CLAUDE.md`.

**Verify:** the cross-layer/integration test is green; seam types are unchanged, or the
change is documented and both sides updated.

### Gate 5 — CLAUDE.md updated

The codebase guide (`CLAUDE.md`) reflects the change: a new crate/module, a new invariant
or test, a changed seam, a new command. **A task that alters behavior or structure without
updating `CLAUDE.md` is not done.**

---

## Hardware Rules (physical edge — non-negotiable)

Part of Gate 1 (compliance) and Gate 3 (protocol). The wire and the controllers do not
forgive these:

- **WiFi is forbidden for live shows.** 5–50 ms jitter causes dropout. Live output runs on
  cabled Ethernet (or wired DMX/SPI). WiFi may exist for config/monitoring only, and the UI
  marks it unsupported for live — it is never debugged as a code bug.
- **The heartbeat always sends the last *valid* frame — never a zeroed frame.** A zero frame
  blacks the rig; silence trips safe mode. Resend the most recent real frame.
- **Max gap between frames to any device is 2.4 s.** Past that, WLED/FPP/Falcon enter safe
  mode. Heartbeat ≥1 Hz keeps the gap far under it: **Warning at 2.0 s, Critical at 2.4 s**,
  surfaced in the UI — never hidden.

---

## Standard commands

Named operations this standard defines. Each maps to a gate and is the canonical way to
enforce it.

### `/seam` — audit the contracts between layers
Checks the §3 seam types — `LogicalFrame`, `AudioFeatures`, `ShowIntent`, `ProtocolOutput`
(plus `DeviceDriver`/`CompiledLayout`/`LayoutIntent`/`SharedContext`) — and asserts **no
layer leaks its details into another**:
- nothing above the HAL references a universe, channel, socket, or RGB order;
- `AudioFeatures` carries `sample_rate` *with* the data — no global rate is assumed;
- the AI layer emits only a validated `ShowIntent`/`LayoutIntent`, never raw effects/channels;
- each side of a seam compiles and tests against a fake of the other.
→ Enforces **Gate 4**. A leak is a failure even if everything compiles.

### `/security` — block insecure output and control
Fails the task on any of:
- **API keys / secrets in code** or committed config — use env vars / a secret store.
- **Sockets bound without restriction** — listening/control sockets bind to an explicit
  interface, not `0.0.0.0`, unless deliberately public *and* authenticated.
- **Multicast without IGMP** — sACN multicast requires IGMP snooping on the path; otherwise
  fall back to directed unicast. Never flood.
- **Control endpoints without authentication** — any API that changes show or device state
  must require auth.
→ A blocking gate; runs before close-out.

### `/phase-gate` — no Phase N before Phase N-1 is confirmed
Blocks starting Phase N work until Phase N-1's **hardware rules and tests are confirmed
green** (master §5 roadmap). A wobbly earlier phase makes a later one undebuggable — you
can't tell a render bug from a wire bug. Every feature is tagged with its phase; building
ahead is rejected, not deferred.

### `/rollback` — revert, don't patch, on an invariant violation
If Gate 1 (compliance) detects an invariant violation, **revert the entire offending file**
to its last-good state — do **not** fix it inline. Report three things:
1. the **invariant** breached,
2. the **file**,
3. the **line**.
Then re-do the change cleanly. (An inline patch to a file that broke an invariant tends to
leave the violation half-fixed; a full revert forces a correct re-implementation.)

### `/changelog` — record the session in CLAUDE.md
At the **end of every session**, append a dated entry to the `## Session changelog` section
of `CLAUDE.md` with four parts:
1. **Done** — what was built/changed this session.
2. **Invariants verified** — which invariants/gates were checked, and how (which test/run).
3. **Pending** — what was left open, and any known risk.
4. **Decisions** — architectural decisions taken (and why), so the next session inherits them.

This is distinct from Gate 5: **Gate 5** keeps `CLAUDE.md` accurate *per task*; **`/changelog`**
adds the *per-session* narrative so history and rationale survive across sessions.

---

## Running the gates

```sh
cargo test --workspace            # compliance / protocol / seam (the test suites)
cargo build --workspace --all-targets   # must be warning-free
# perf — hot-path allocation:
cargo test -p led-hal --test no_alloc
# perf — lock-free unsafe under the C++ memory model, many interleavings:
cargo +nightly miri test -p led-pixel-engine --lib
MIRIFLAGS="-Zmiri-many-seeds=0..24 -Zmiri-preemption-rate=0.1" \
  cargo +nightly miri test -p led-pixel-engine --lib triple::
```

## Close-out checklist

- [ ] Gate 1 — compliance: invariant tests green; hardware rules honored; any found violation handled via `/rollback`
- [ ] Gate 2 — perf: within budget; hot path alloc-free; unsafe is Miri-clean
- [ ] Gate 3 — protocol: wire-format test parses real bytes and passes
- [ ] Gate 4 — seam: `/seam` passes — contracts honored, no layer leaks; isolation/integration test green
- [ ] `/security` passes — no secrets in code, restricted binds, IGMP for multicast, authed control endpoints
- [ ] `/phase-gate` — prior phase's hardware + tests confirmed before this phase's work
- [ ] Gate 5 — `CLAUDE.md` updated to match the change
- [ ] `cargo build --workspace --all-targets` is warning-free
