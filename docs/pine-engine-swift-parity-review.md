# Pine Engine Swift Parity Review

This document compares the Rust Pine engine in this repository with the Swift
implementation in `../DegenView`. The Rust engine is substantial for historical
batch execution, but it is not yet behaviorally equivalent to the Swift engine.
The largest correctness gaps are in realtime execution, alert lifecycle, state
rollback, and builtin dispatch.

## Status

Findings 1–14 are fixed on the `pine-swift-parity-fixes` branch. Charts and Pine alert
subscriptions both run through `pine::execution::ExecutionController`: charts on a background
worker (latest-wins, generation-tagged), alerts on one persistent controller per active
subscription (`PineSubscriptionHost::run_active`). The initial snapshot and any rebuild replay
history without notifying. The batch `run_script_alerts*` replay path is removed.

Still open: finding 15 (module-wide `#![allow(dead_code)]` and the separate builtin catalogues)
and the Swift/Rust shared parity fixture suite from the recommended repair order.

The findings below are ordered by severity. Line links identify the relevant
implementation at the time of the review and may move as the code changes.

## Critical findings

### 1. Indicators and strategies do not update on intrabar changes

The chart reruns Pine only when the source, inputs, or candle count changes.
Replacing the forming candle with updated OHLCV data does not change the count,
so cached Pine output remains stale:

- [Rust chart cache](../src/ui/chart_card.rs#L409)
- [Rust forming-candle replacement](../src/workspace.rs#L1285)
- [Rust batch execution host](../src/pine_host.rs#L121)

The Rust host executes the complete candle set as historical bars on each run.
By contrast, Swift retains a persistent execution controller, distinguishes
historical bars from the forming bar, and processes tick and close events through
a scheduler:

- [Swift execution controller](../../DegenView/DegenView/Pine/Runtime/PineExecutionController.swift#L34)
- [Swift execution scheduler](../../DegenView/DegenView/Pine/Runtime/PineExecutionScheduler.swift#L3)

Consequences include stale intrabar indicators, incorrect `barstate.*` values,
and ineffective `calc_on_every_tick` behavior.

**Suggested fix:** Introduce one persistent execution session per applied script
instance. Rebuild only when history, source, or inputs change; feed updates to the
forming candle as ticks and commit them explicitly when the candle closes.

### 2. Pine alerts run at the wrong point in the bar lifecycle

The app evaluates alerts only when the candle's opening timestamp changes:

- [Rust alert evaluation gate](../src/app.rs#L700)
- [Rust alert replay as a realtime close](../src/pine_host.rs#L156)

The newly observed candle is replayed as `RealtimeClose`, even though it is
normally a newly opened, incomplete candle. This means:

- `alert.freq_once_per_bar_close` can fire on the first update of a forming bar.
- `alert.freq_all` cannot run on every update.
- A condition evaluated on the first partial candle is not reevaluated at the
  actual close.

Swift routes alerts from the real execution lifecycle rather than rerunning an
independent batch:

- [Swift alert coordinator](../../DegenView/DegenView/Service/PineAlertCoordinator.swift#L25)

**Suggested fix:** Route alert events directly from the persistent chart
execution session, with explicit tick and close phases.

### 3. Alert subscriptions lose instance identity and custom inputs

Every Rust subscription is currently created with `instance_id: None` and
`script_id: None`:

- [Rust subscription creation](../src/app.rs#L481)
- [Rust subscription filtering](../src/pine_subscription_host.rs#L33)
- [Rust alert execution inputs](../src/pine_host.rs#L163)

The subscription host does not validate the currently applied instance or source
hash. Alert execution also supplies an empty input map, so customized `input.*`
values revert to defaults. Although `ScriptChanged` exists as a state, the runtime
lifecycle does not appear to mark subscriptions when their source changes or an
applied instance disappears.

Swift subscriptions remain tied to the actual instance and source hash:

- [Swift alert lifecycle](../../DegenView/DegenView/Service/PineAlertCoordinator.swift#L53)

**Suggested fix:** Persist instance identity and applied inputs in each
subscription, validate the current source hash, and transition removed or changed
instances out of the active state. Prefer subscribing to events from the chart's
live execution session instead of maintaining a second execution path.

### 4. Alert and webhook deduplication can suppress unrelated or later alerts

Rust alert IDs are allocated from the session's local reference counter:

- [Rust alert ID allocation](../src/pine/alert_builtins.rs#L82)
- [Rust webhook uniqueness constraint](../src/store/mod.rs#L175)
- [Rust webhook dispatch](../src/alert_host.rs#L339)

Because alert sessions are reconstructed, IDs are deterministic for a given
execution shape rather than globally unique. Webhook uniqueness is only
`(source, event_id, endpoint_id)`. Different subscriptions can therefore collide,
and later alerts can reuse IDs once the loaded history window stops growing.

Swift deduplicates using subscription, call site, bar, and frequency:

- [Swift alert frequency guard](../../DegenView/DegenView/Service/PineAlertFrequencyGuard.swift#L19)

**Suggested fix:** Use a persistent key such as
`subscription_id + call_site + bar_open_ms + frequency`. For `freq_all`, allocate
a genuinely unique event ID. Persist the Pine event before notification and
webhook dispatch.

## High-severity findings

### 5. TA state is outside realtime rollback state

`RuntimeState` has committed and working copies, but `ta_states` lives directly on
`Session`:

- [Rust session state](../src/pine/session.rs#L45)
- [Rust realtime rollback](../src/pine/session.rs#L350)
- [Swift TA state handling](../../DegenView/DegenView/Pine/Runtime/PineRuntimeSession+TA.swift#L3)

Tick rollback resets `working`, not `ta_states`. Repeated updates to one forming
bar can therefore advance SMA, EMA, ATR, VWAP, `barssince`, correlation, pivots,
SAR, and similar state as though multiple bars had elapsed. Swift stores TA call
history inside working state, so rollback covers it.

**Suggested fix:** Move all TA state into `RuntimeState`, or derive it from the
existing working call histories. Add a regression test asserting that several
ticks followed by one close produce exactly one committed history entry.

### 6. Duplicate builtin dispatch makes valid implementations unreachable

The session sends `math.*` and `str.*` through the scalar handler first:

- [Rust scalar dispatch](../src/pine/session.rs#L1925)
- [Rust TA support catalogue](../src/pine/ta.rs#L84)
- [Rust canonical string implementation](../src/pine/strings.rs#L13)
- [Rust duplicate scalar string implementation](../src/pine/scalar_builtins.rs#L216)

This shadows the canonical modules later in dispatch. Concrete examples include:

- `math.sum` is implemented and advertised by the TA module, but the scalar math
  handler intercepts it and returns `na`.
- `str.match` has a regex implementation in `strings.rs`, while the production
  scalar path returns an empty string.
- `str.split` is advertised, but the scalar path intercepts it before the
  array-producing implementation can run.

This duplication lets module-level tests pass while compiled scripts use a
different implementation.

**Suggested fix:** Keep one canonical dispatch implementation per namespace.
Handle series- or reference-returning functions such as `math.sum` and
`str.split` before scalar dispatch. Add compiler-to-session tests for every
advertised symbol.

### 7. Several TA overloads bind the wrong arguments

The generic TA branch assumes argument zero is always a source and argument one
is always the length:

- [Rust generic TA dispatch](../src/pine/session.rs#L1439)
- [Swift TA default sources](../../DegenView/DegenView/Pine/Runtime/PineRuntimeSession+TA.swift#L6)

This breaks cases including:

- `ta.highest(length)` and `ta.lowest(length)`, which should default to `high` and
  `low`.
- `ta.highestbars(length)` and `ta.lowestbars(length)`.
- `ta.linreg(source, length, offset)`, whose third argument is ignored and whose
  second argument is reused incorrectly.

**Suggested fix:** Replace the generic positional shortcut with per-builtin
signature specifications or dedicated handlers. Test every overload, including
named arguments.

### 8. Array calls can evaluate arguments twice

Array dispatch first binds and evaluates arguments, then calls
`eval_all(arguments)` again:

- [Rust array dispatch](../src/pine/session.rs#L1622)
- [Swift array dispatch](../../DegenView/DegenView/Pine/Runtime/PineRuntimeSession+Arrays.swift#L1)

An expression such as `array.push(xs, f())` can execute `f()` twice, duplicating
side effects, alerts, allocations, or mutations.

**Suggested fix:** Produce one evaluated argument structure that preserves order
and names. Special-case `array.from` before ordinary binding instead of
reevaluating every array call.

### 9. Declared drawing limits are parsed but ignored

The compiler stores `max_lines_count`, `max_labels_count`, `max_boxes_count`, and
`max_polylines_count`, but runtime drawing calls always use
`DrawingLimits::default()`:

- [Rust declaration parsing](../src/pine/compiler.rs#L211)
- [Rust program declaration](../src/pine/program.rs#L68)
- [Rust drawing dispatch](../src/pine/session.rs#L1447)

Scripts therefore receive the default limit even when their declaration requests
another supported value.

**Suggested fix:** Construct drawing limits from `Program.declaration` when the
session is created and use them for both function and method dispatch. Add tests
for each declared cap and oldest-object deletion.

### 10. Execution limits are not enforced, and Pine can block the UI

The limits model includes a deadline, but historical execution has no deadline or
cancellation check:

- [Rust execution limits](../src/pine/token.rs#L127)
- [Rust history execution loop](../src/pine/session.rs#L272)
- [Rust synchronous chart execution](../src/ui/chart_card.rs#L414)
- [Swift cancellation and deadline checks](../../DegenView/DegenView/Pine/Runtime/PineRuntimeSession.swift#L170)

Rust runs scripts synchronously during chart rendering. A long script, or
blocking `request.security` provider work, can freeze the egui frame.

**Suggested fix:** Run Pine through the existing background work-queue pattern
with cooperative cancellation and deadline checks. Tag results with a generation
or dataset key so stale background work cannot overwrite newer output.

## Medium-severity findings and code smells

### 11. Compiler-supplied lexer limits are ignored

Compilation constructs the lexer with default limits rather than the limits
passed to `compile`:

- [Rust compiler setup](../src/pine/compiler.rs#L43)

**Suggested fix:** Construct the lexer with the supplied limits and add tests
using deliberately small source and token limits.

### 12. Invalid compilation can look successful

An invalid program can return empty output without a runtime error:

- [Rust checked script runner](../src/pine_host.rs#L121)

The chart can consequently treat empty output as a valid new result and replace
the last valid output.

**Suggested fix:** Return compilation diagnostics separately from runtime errors,
and update `last_valid` only after both compilation and execution succeed.

### 13. Visible-range support is not connected to the chart viewport

The runtime supports a visible range, but host-created sessions pass no range.
`chart.left_visible_bar_time` and `chart.right_visible_bar_time` consequently
describe the loaded history rather than the actual viewport.

**Suggested fix:** Include the visible bar range in host execution inputs and
invalidate scripts that depend on it when the viewport changes.

### 14. Source hashing differs from the Swift identity contract

Rust uses FNV-1a, while Swift uses SHA-256:

- [Rust source hash](../src/script_manager/store.rs#L683)
- [Swift source hash](../../DegenView/DegenView/Service/ScriptSourceHash.swift#L4)

The weaker hash is also used outside a purely local cache context, including
alert source identity. A collision can cause changed source to be treated as
unchanged, and the persisted identity format differs between implementations.

**Suggested fix:** Use one SHA-256 source-hash utility throughout the Rust app and
recompute or migrate cached and subscription hashes.

### 15. Warning suppression and multiple sources of truth hide stale code

The Pine module suppresses dead-code and unused-import warnings for the entire
feature:

- [Rust Pine warning suppression](../src/pine/mod.rs#L1)
- [Rust empty symbol-catalog member lookup](../src/pine/symbol_catalog.rs#L292)

The empty `members()` placeholder and separate completion, signature, support,
and runtime catalogues make drift likely. The duplicate builtin implementations
are one manifestation of this structure.

**Suggested fix:** Remove module-wide warning suppression incrementally and
generate completion, signatures, support checks, and dispatch metadata from one
builtin registry where practical.

## Recommended repair order

1. Port the persistent execution controller and scheduler design, then make
   charts and alerts consume the same event stream.
2. Correct alert identity, applied-input capture, source lifecycle, and
   deduplication.
3. Move TA state into rollback state; then fix dispatch shadowing, TA overloads,
   and double evaluation.
4. Enforce declaration and resource limits and move execution off the UI thread.
5. Consolidate builtin catalogues and add a Swift/Rust parity fixture suite that
   covers outputs, diagnostics, ticks, closes, alerts, drawing limits, and
   overloads.

## Verification and test gaps

At the time of this review:

- `cargo test` passed with 1,113 tests passed and one ignored.
- `cargo clippy --all-targets --all-features -- -D warnings` passed.
- The optional community Pine corpus was absent, so that suite was skipped.

The committed corpus tests focus mainly on compilation status and diagnostic
codes rather than numerical output equivalence. The Rust project has roughly 226
Pine-related tests, compared with approximately 846 Swift Pine test methods. The
largest coverage gap is around realtime lifecycle, alert routing, scheduling,
and semantic regression cases.

A useful next step is a shared fixture format containing Pine source, bars,
execution events, expected diagnostics, and expected output values. Running the
same fixtures through both implementations would turn parity from an informal
comparison into a continuously checked contract.
