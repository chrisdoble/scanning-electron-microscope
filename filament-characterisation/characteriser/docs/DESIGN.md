# Filament Characterisation TUI — Specification

A Ratatui application that drives the DIY SEM vacuum rig and steps an operator
through characterising a tungsten filament.

This document specifies **structure only**. The actual characterisation physics
and measurement logic are deliberately stubbed — the deliverable is a skeleton
that compiles, runs against mock hardware, renders the UI, executes a
step-by-step procedure, handles user input, records measurements to a
serialisable struct, shells out to Python for statistics, and cleans up safely
on error.

---

## 1. Goals

1. Reuse the existing `Adc`, `Tmp` and `Controller` types, and the forthcoming
   `PowerSupply` and `Oscilloscope` types, from the `host` crate.
2. Separate concerns into Tokio tasks: hardware polling, procedure execution,
   terminal input, rendering.
3. Model the procedure as a tree of **steps** written as ordinary sequential
   code, rendered as a scrollable, nested list.
4. Support user interaction mid-procedure (confirmations, value entry).
5. Collect measurements — raw samples and the values derived from them — into a
   single in-memory struct written to disk at explicit checkpoints, so a run
   that fails part-way still leaves usable data behind.
6. Run without hardware attached (`--mock`) so the UI can be compiled and
   exercised away from the rig.
7. Delegate statistics and uncertainty propagation to Python scripts shipped
   inside the crate, run through a virtual environment verified at start-up.
8. Guarantee that any error, panic or quit runs a cleanup routine that puts the
   filament system into a safe state.

## 2. Non-goals

- No real characterisation logic. Procedure steps perform sleeps, mock reads
  and placeholder measurements.
- **No emission-current measurement.** That comes in a later version; §17 lists
  the extension points that must stay open for it.
- No charts. Every step renders as text; §9.4 explains what adding a chart later
  would cost.
- No statistics in Rust. The crate records samples and calls Python to turn them
  into a value and an uncertainty.
- No realistic hardware simulation. The mocks return canned values so the UI
  compiles and runs; they are not a model of the rig.
- No fleshing out of the measurement struct. It has almost no fields yet by
  design (§10).

## 3. Assumptions

Implement against these; flag them in the PR description rather than guessing
differently.

| # | Assumption |
|---|------------|
| A1 | `filament-characterisation/` is a new **grouping directory**, a sibling of `host/` and `common/` in the existing Cargo workspace. It is not itself a crate. |
| A2 | The new crate lives at `filament-characterisation/characteriser/`, is named `characteriser`, and defines its binary in `src/main.rs`. Add it to the workspace `members`. |
| A3 | The filament power supply, its polarity relays and the four-terminal sense measurement are reached through `host::power_supply::PowerSupply` and `host::oscilloscope::Oscilloscope`. `PowerSupply` holds its own `Arc<Controller>`, through which it switches the SPDT relays. Both instruments are found by hard-coded USB vendor and product IDs — never by command line argument. |
| A4 | The vacuum hardware is reached exactly as the existing binary does: one `Controller` over serial, shared by `Adc` and `Tmp` via `Arc`. The same `Controller` is shared with `PowerSupply`. |
| A5 | Shared dependency versions are hoisted from `host` into `[workspace.dependencies]` and pinned there (§4.1). Take the versions `host` currently declares; do not upgrade anything as part of the move. |
| A6 | Keep `#[tokio::main(flavor = "current_thread")]` as in the existing binary. All blocking serial I/O already happens inside `spawn_blocking` within `Controller`. |
| A7 | `host` stays free of `serde`. |
| A8 | The Python virtual environment lives at `characteriser/python/.venv` and is created by the operator, not by the application (§12). |
| A9 | Development and operation are on macOS/Linux. Paths like `python/.venv/bin/python` are used directly, without a Windows branch. |

---

## 4. Workspace layout

```
filament-characterisation/
  characteriser/
    Cargo.toml
    python/
      .venv/                        // created by the operator, gitignored
      requirements.txt              // pins `uncertainties`
      mean_and_standard_error.py    // stub: samples in, value and uncertainty out
    src/
      main.rs                 // args, logging, start-up checks, task wiring, error handling
      app.rs                  // App state, event loop, key handling
      hardware/
        mod.rs                // traits, snapshots, the Hardware bundle
        real.rs               // impls over Adc / Tmp / PowerSupply / Oscilloscope
        mock.rs               // canned-value implementations
      procedure/
        mod.rs                // Context and the procedure runner
        characterisation.rs   // the procedure itself
      python.rs               // venv verification and script invocation
      results.rs              // Characterisation, Measurement, serialisation
      steps.rs                // Step, Section, and their rendering
      ui/
        mod.rs                // layout + top-level render
        filament.rs           // filament stats block
        run.rs                // run status block
        steps.rs              // scrollable step list
        vacuum.rs             // vacuum stats block
```

Add `python/.venv/` and `out/` to `.gitignore` if they aren't there already.

### 4.1 Hoisting dependencies to the workspace

Before writing any application code, move the shared dependencies out of
`host/Cargo.toml` (and the existing binary's) into the workspace root:

```toml
# Cargo.toml (workspace root)
[workspace.dependencies]
clap = { version = "<as in host>", features = ["derive"] }
crossterm = "<as in host>"
env_logger = "<as in host>"
log = "<as in host>"
ratatui = "<as in host>"
serialport = "<as in host>"
thiserror = "<as in host>"
tokio = { version = "<as in host>", features = ["macros", "rt", "sync", "time"] }
```

Pin each to the exact version currently resolved in `Cargo.lock` so this move
changes nothing at build time. Then rewrite the member crates to use
`dependency = { workspace = true }`, adding extra features locally where needed.
Verify with `cargo tree` that no version changed, and that `host` and the
existing binary still build, before moving on.

The new crate's manifest:

```toml
[dependencies]
async-trait = "0.1"
clap = { workspace = true }
common = { path = "../../common" }
crossterm = { workspace = true, features = ["event-stream"] }
env_logger = { workspace = true }
futures = "0.3"              # StreamExt, for the crossterm EventStream
host = { path = "../../host" }
log = { workspace = true }
ratatui = { workspace = true }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = { workspace = true }
tokio = { workspace = true, features = ["process"] }
tokio-util = { version = "0.7", features = ["rt"] }   # CancellationToken
```

---

## 5. Command line arguments

Identical to the existing binary, plus `--mock`. Positional arguments become
optional only when `--mock` is given. **No arguments are added for the power
supply, the oscilloscope or the Python environment** — those are located by
hard-coded constants.

```rust
#[derive(Parser)]
struct Arguments {
    /// The path to the controller device, e.g. /dev/tty.usbmodem11201.
    #[arg(required_unless_present = "mock")]
    device_path: Option<String>,

    /// The gauge number to use when communicating with the ADC, e.g. 1.
    #[arg(
        required_unless_present = "mock",
        value_parser = clap::value_parser!(u8).range(1..=2)
    )]
    adc_gauge_number: Option<u8>,

    /// The address of the TMP in the RS-485 Pfeiffer Vacuum Protocol, e.g. 001.
    ///
    /// This corresponds to TMP parameter 797.
    #[arg(required_unless_present = "mock")]
    tmp_address: Option<String>,

    /// Run against simulated hardware instead of the real rig.
    #[arg(long)]
    mock: bool,
}
```

---

## 6. Logging

Copy `init_logging` from the existing binary verbatim, with one fix: create the
output directory first so a fresh checkout doesn't fail.

```rust
/// Initialise `env_logger` to log to `out/app.log`.
fn init_logging() -> Result<(), std::io::Error> {
    fs::create_dir_all("out")?;
    let file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open("out/app.log")?;

    Builder::from_default_env()
        .target(Target::Pipe(Box::new(file)))
        .init();

    info!("Application started");
    Ok(())
}
```

Nothing may ever be written to stdout or stderr while the alternate screen is
active. Log levels:

- `info!` — lifecycle: start, section entered, results written, Python script
  invoked, cleanup run, exit. Also a note at start-up saying whether the
  filament system is real or mocked.
- `debug!` — every hardware read/write, and the full command line of every
  Python invocation.
- `warn!` — a hardware read that failed but is being retried.
- `error!` — before returning any `Err`, matching the existing style.

---

## 7. Hardware abstraction

### 7.1 Snapshots

Plain data, taken once per poll. Every field is a real reading: if an instrument
can't be read, that is an error (§8.1), not a missing field.

```rust
/// A snapshot of the vacuum system, taken by the hardware poll task.
///
/// `Copy` because every field is, and snapshots are passed around by value.
/// Drop it if a non-`Copy` field is ever added. Deliberately not `Default`:
/// there is no such thing as a default reading, and the absence of readings is
/// modelled by the absence of a snapshot (§13).
#[derive(Clone, Copy, Debug)]
pub struct VacuumSnapshot {
    /// The chamber pressure.
    pub pressure: Pressure,

    /// The current draw of the TMP in amperes.
    pub tmp_current: f32,

    /// The current rotation speed of the TMP in hertz.
    pub tmp_current_rotation_speed: u16,

    /// If the TMP is running.
    pub tmp_running: bool,

    /// The target rotation speed of the TMP in hertz.
    pub tmp_target_rotation_speed: u16,
}

/// A snapshot of the filament system, taken by the hardware poll task.
#[derive(Clone, Copy, Debug)]
pub struct FilamentSnapshot {
    /// The voltage across the filament in volts, from four-terminal sensing.
    pub filament_voltage: f64,

    /// The current flowing through the filament in amperes.
    pub heating_current: f64,

    /// If the power supply output is enabled.
    pub output_enabled: bool,

    /// How the filament is connected to the supply.
    pub polarity: Polarity,
}

/// Both snapshots, published together so the UI and the procedure always see a
/// consistent pair.
#[derive(Clone, Copy, Debug)]
pub struct Snapshots {
    pub filament: FilamentSnapshot,
    pub vacuum: VacuumSnapshot,
}
```

No derived quantities live on the snapshots. Resistance is calculated from
recorded measurements by the Python side, with uncertainties, and the TUI never
computes its own version.

### 7.2 Polarity

The filament sits between two SPDT relays, each of which connects its side of
the filament to either rail of the supply. Four relay states exist; three
distinct connections matter, and only these are exposed:

```rust
/// The direction of the current through the filament.
///
/// Each side of the filament is switched by one SPDT relay. With both relays
/// de-energised, both sides sit on the negative rail, no current can flow, and
/// the current has no direction — that's `Nil`. That is the state the
/// controller powers up in, and the state cleanup returns to. Energising one
/// relay or the other puts one side on the positive rail, setting the
/// direction.
///
/// Reversing the direction lets the Seebeck voltages at the filament's
/// junctions cancel when a forward and a reverse measurement are averaged.
///
/// A fourth relay state exists — both energised, both sides on the positive
/// rail — which is equivalent to `Nil` and is never used.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Polarity {
    /// Relay 1 energised: current flows through the filament one way.
    Forward,

    /// Both relays de-energised: no current flows, so there is no polarity.
    #[default]
    Nil,

    /// Relay 2 energised: current flows the other way.
    Reverse,
}
```

This enum belongs in `host` beside `PowerSupply` (§7.5), since the supply is
what switches the relays.

### 7.3 Traits

One trait per subsystem. Both are `dyn`-compatible via `#[async_trait]`, and
both are `Send + Sync + Debug` so they can live in an `Arc` shared between the
poll task and the procedure task. Reads are named `get_*` to match `Tmp` and
`Adc` in `host`.

```rust
/// A vacuum system: a pressure gauge and a turbomolecular pump.
#[async_trait]
pub trait VacuumSystem: std::fmt::Debug + Send + Sync {
    /// Reads every value shown in the vacuum block in a single pass.
    async fn snapshot(&self) -> Result<VacuumSnapshot, HardwareError>;

    /// Turns the TMP on or off.
    ///
    /// IMPORTANT: Callers must confirm the chamber pressure is below
    /// `TMP_MAXIMUM_BACKING_PRESSURE_MBAR` and that the roughing pump is
    /// running before turning the TMP on, otherwise it may be damaged.
    async fn set_tmp_running(&self, running: bool) -> Result<(), HardwareError>;
}

/// The filament system: the power supply, its polarity relays, and the
/// oscilloscope measuring the voltage across the filament.
#[async_trait]
pub trait FilamentSystem: std::fmt::Debug + Send + Sync {
    /// Reads every value shown in the filament block in a single pass.
    async fn snapshot(&self) -> Result<FilamentSnapshot, HardwareError>;

    /// Measures the voltage across the filament in volts.
    ///
    /// This is the sense pair of the four-terminal measurement, so it excludes
    /// the drop across the supply leads and the feedthroughs.
    async fn get_filament_voltage(&self) -> Result<f64, HardwareError>;

    /// Measures the current through the filament in amperes.
    async fn get_heating_current(&self) -> Result<f64, HardwareError>;

    /// Sets the channel's current limit in amperes.
    ///
    /// Must be rejected above `MAXIMUM_HEATING_CURRENT_AMPS`.
    async fn set_heating_current(&self, current: f64) -> Result<(), HardwareError>;

    /// Sets the channel's voltage limit in volts.
    ///
    /// The supply runs in whichever mode its limits make it: with the voltage
    /// limit set high, the current limit is what binds and the channel runs in
    /// constant current. Called once at the start of a run with a large value
    /// so every later `set_heating_current` is the limiting factor.
    async fn set_heating_voltage(&self, voltage: f64) -> Result<(), HardwareError>;

    /// Enables or disables the output.
    async fn set_output_enabled(&self, enabled: bool) -> Result<(), HardwareError>;

    /// Sets the direction of the current through the filament via the relays.
    ///
    /// IMPORTANT: The output must be disabled before the relays are switched,
    /// otherwise the contacts will arc. Implementations must enforce this
    /// rather than trusting the caller.
    async fn set_polarity(&self, polarity: Polarity) -> Result<(), HardwareError>;

    /// Puts the filament system into a safe state.
    ///
    /// Called on every exit path: normal completion, procedure error, user
    /// quit, and panic. Must be idempotent and must not return early on the
    /// first failure — every action is attempted and failures are logged.
    ///
    /// In order: zero the current, disable the output, then
    /// `set_polarity(Polarity::Nil)` to de-energise both relays.
    async fn enter_safe_state(&self);
}
```

`get_filament_voltage` and `get_heating_current` exist separately from
`snapshot` because a measurement takes `SAMPLE_COUNT` samples in quick
succession (§10.1), far faster than the one-per-second the snapshot provides.
`VacuumSystem` needs no such method: nothing wants a one-off vacuum reading,
because the procedure waits on the snapshot stream instead (§11.1).

### 7.4 The `Hardware` bundle

```rust
/// Everything the application can talk to.
///
/// Field-only: with `snapshot` and `enter_safe_state` on the traits themselves,
/// this exists solely so functions take one parameter instead of two.
#[derive(Clone, Debug)]
pub struct Hardware {
    pub filament: Arc<dyn FilamentSystem>,
    pub vacuum: Arc<dyn VacuumSystem>,
}
```

Two separate `Arc`s rather than one combined trait means the real vacuum system
can be paired with a mock filament system — which is where development starts,
since the `host` types may not exist yet. `--mock` is a single flag; partial
mocking is a constructor choice inside `build_hardware`.

### 7.5 Real implementations

`real.rs` holds thin adapters:

- `RealVacuumSystem { adc: Adc, tmp: Tmp }`, constructed from an
  `Arc<Controller>` exactly as the existing binary does, including the
  `set_pressure_unit(PressureUnit::Millibar)` call at start-up.
- `RealFilamentSystem { oscilloscope: Oscilloscope, power_supply: PowerSupply }`.

Each adapter only maps types and errors; no logic lives here. The channel
constants live here too:

```rust
/// The oscilloscope channel probing the filament's sense pair.
const SENSE_CHANNEL: u8 = 1;

/// The power supply channel driving the filament.
const SUPPLY_CHANNEL: u8 = 1;
```

**Before writing these, check what `host` actually exposes.** If
`host::power_supply::PowerSupply` and `host::oscilloscope::Oscilloscope` don't
exist yet, add them to `host` as minimal modules following the exact shape of
`adc.rs` and `tmp.rs` — a `thiserror` error enum wrapping `ControllerError`, a
struct holding its state behind a `tokio::sync::Mutex`, async methods, doc
comments with units — with method bodies returning a `NotImplemented` variant
and a `TODO` naming the instrument and protocol still to be wired up. Both are
found by USB vendor and product ID, declared as constants in their own module
with a comment naming the instrument they identify:

```rust
impl PowerSupply {
    /// `controller` is used to switch the SPDT polarity relays.
    ///
    /// The supply itself is found by its USB vendor and product IDs.
    pub fn new(controller: Arc<Controller>, channel: u8) -> Result<Self, PowerSupplyError>;

    pub async fn get_current(&self) -> Result<f64, PowerSupplyError>;
    pub async fn get_output_enabled(&self) -> Result<bool, PowerSupplyError>;
    pub async fn get_polarity(&self) -> Result<Polarity, PowerSupplyError>;
    pub async fn set_current(&self, current: f64) -> Result<(), PowerSupplyError>;
    pub async fn set_output_enabled(&self, enabled: bool) -> Result<(), PowerSupplyError>;
    pub async fn set_polarity(&self, polarity: Polarity) -> Result<(), PowerSupplyError>;
    pub async fn set_voltage(&self, voltage: f64) -> Result<(), PowerSupplyError>;
}

impl Oscilloscope {
    /// Found by its USB vendor and product IDs.
    pub fn new() -> Result<Self, OscilloscopeError>;

    /// Measures the mean voltage on `channel` in volts.
    pub async fn measure_voltage(&self, channel: u8) -> Result<f64, OscilloscopeError>;
}
```

If the real signatures differ, `real.rs` is the only file that changes. Keep it
that way — nothing outside `hardware/real.rs` may name a `host` instrument type.

While the supply and oscilloscope are stubbed, `build_hardware` substitutes the
mock filament system even without `--mock` and logs that it has done so.

### 7.6 Mock implementations

The mocks exist so the application compiles and the UI can be exercised. They
are **not** a simulation. Each getter returns a canned constant and each setter
stores the value it was given, so the snapshot reflects what was set. Constants
at the top of the file, one line each. No time dependence, no noise, no physics.

Two behaviours are kept because they're contracts rather than simulation:
`set_heating_current` above `MAXIMUM_HEATING_CURRENT_AMPS` errors, and
`set_polarity` errors while the output is enabled.

### 7.7 Errors

```rust
#[derive(Debug, Error)]
pub enum HardwareError {
    #[error("adc error: {0}")]
    Adc(#[from] AdcError),
    #[error("oscilloscope error: {0}")]
    Oscilloscope(#[from] OscilloscopeError),
    #[error("power supply error: {0}")]
    PowerSupply(#[from] PowerSupplyError),
    #[error("tmp error: {0}")]
    Tmp(#[from] TmpError),
    #[error("not implemented")]
    NotImplemented,
    #[error("{0}")]
    Other(String),
}
```

---

## 8. Application architecture

### 8.1 Tasks and shared state

```
        ┌──────────────────┐
        │  hardware poll   │  every POLL_INTERVAL (1 s)
        │      task        │  vacuum.snapshot() + filament.snapshot()
        └────────┬─────────┘
                 │ watch::Sender<Option<Snapshots>>
        ┌────────┴──────────────────────┐
        ▼                               ▼
┌──────────────────┐            ┌───────────────┐
│  app event loop  │            │ procedure     │
│  (main task)     │            │ task          │
│  owns terminal   │            │ Context       │
└────────┬─────────┘            └───────┬───────┘
         │  reads            mutates    │
         └───────► Arc<Mutex<Section>> ◄┘
         ▲
         │ mpsc::Receiver<AppEvent>  (HardwareFailed, ProcedureFinished)
         │ crossterm EventStream
```

- **App event loop (main task).** Owns the `DefaultTerminal` — never shared, so
  only this task draws. Selects over the `AppEvent` receiver, the crossterm
  `EventStream`, the snapshot watch, and a render tick.
- **Hardware poll task.** Sleeps `POLL_INTERVAL` (1 s, a `const` with a doc
  comment), takes both snapshots, publishes `Snapshots` on the watch channel.
  A failed read is retried on the next tick with a `warn!`. After
  `POLL_FAILURE_TOLERANCE` consecutive failures (3) it sends
  `AppEvent::HardwareFailed`, which is fatal: the app shuts down and cleanup
  runs. A serial timeout mid-run shouldn't end a forty-minute pump-down, but
  persistent failure means the rig is no longer under control and the filament
  shouldn't stay powered. Both constants get doc comments saying this.
- **Procedure task.** Runs the procedure against a `Context`. Started **after
  the first snapshot arrives** (§13), so nothing runs against instruments that
  haven't answered yet. Sends `AppEvent::ProcedureFinished(result)` when done.
- **No separate render task.** Rendering happens in the app loop, because the
  terminal would otherwise have to be shared. Say so in a comment.

**What `main` creates and hands out.** Before spawning anything:

- `let cancel = CancellationToken::new();` — cloned into the poll task, into the
  `Context`, and kept by `App`, which calls `cancel.cancel()` as the first step
  of shutdown (§16). It is the only cancellation channel; nothing else signals
  tasks to stop.
- `let root = Arc::new(Mutex::new(Section::default()));` — the step tree, cloned
  into the `Context` and kept by `App`.
- `let characterisation = Characterisation::new();` and
  `let results_path = characterisation.path();` — the run's measurements, moved
  into the `Context`, and where they're written, kept by `main` so it can be
  printed on exit. The `Context` doesn't need the path: `save` knows it.
- `let (snapshots_tx, snapshots_rx) = watch::channel(None);` — the sender goes to
  the poll task, receivers to `App` and to the `Context`.
- `let (events_tx, events_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);` — the
  sender is cloned into the poll task and the procedure task.

Render tick: `RENDER_INTERVAL = 50 ms` (20 fps), enough for spinner animation.

### 8.2 Events

The message channel carries only what can't be expressed as shared state:

```rust
/// An out-of-band message to the application's event loop.
#[derive(Debug)]
pub enum AppEvent {
    /// The hardware has failed persistently and the run can't continue.
    HardwareFailed(HardwareError),

    /// The procedure task has finished, successfully or otherwise.
    ProcedureFinished(Result<(), ProcedureError>),
}
```

Steps are shared state (§9) and snapshots are a watch channel, so neither
appears here.

### 8.3 How `select!` behaves, and why this is safe

`tokio::select!` polls every branch's future concurrently and, when one
completes, drops the others. Dropping a future cancels whatever work it was
doing — so the question isn't whether the other branches get cancelled (they do)
but whether cancelling them **loses anything**. A future is *cancellation-safe*
if dropping it mid-poll leaves no state behind.

All four branches here are cancellation-safe, because their state lives in a
long-lived object outside the future:

- `mpsc::Receiver::recv()` — an unreceived message stays in the channel.
- `futures::StreamExt::next()` on `EventStream` — an unread key event stays in
  the stream's buffer.
- `watch::Receiver::changed()` — the value stays in the channel, and the
  receiver's "seen" marker only advances when `changed()` actually resolves.
- `tokio::time::Interval::tick()` — the deadline lives in the `Interval`.

Each loop iteration creates fresh futures from those long-lived objects, so a
branch that lost the race is simply re-awaited next iteration with nothing
missed:

```rust
loop {
    tokio::select! {
        Some(event) = events.recv() => self.handle_app_event(event),
        Some(Ok(event)) = terminal_events.next() => self.handle_terminal_event(event),
        Ok(()) = snapshots.changed() => self.handle_snapshot(*snapshots.borrow_and_update()),
        _ = ticker.tick() => self.render(&mut terminal)?,
    }
}
```

Put a comment above the loop saying exactly this, and one rule beside it: **only
cancellation-safe futures go directly in a `select!` branch.** Anything that
buffers into a local (a multi-step read, a partially consumed iterator) must be
driven by a task and its result delivered over a channel instead.

### 8.4 Terminal input

Use `crossterm::event::EventStream` so input can be selected alongside
everything else, rather than the blocking `poll`/`read` pair the existing binary
uses. This needs `features = ["event-stream"]` on `crossterm`, which the
workspace pin (§4.1) keeps in step with ratatui's copy.

---

## 9. The step tree

The tree lives in `Arc<Mutex<Section>>` — one root section — shared between the
procedure task (which mutates) and the app loop (which renders). Direct mutation
replaces passing step changes as messages, which would mean enumerating every
kind of mutation as an event variant and naming the node each one applies to.

There is no wrapper type around it and no revision counter. The app redraws on
every tick unconditionally: the running step animates a spinner, so most frames
would redraw anyway, and ratatui diffs the previous and current buffers and
writes only the cells that changed — so a redraw of unchanged content costs a
little line-building and no terminal I/O. A revision counter would save that
line-building only while the procedure is blocked on a prompt. Not worth the
second source of truth.

### 9.1 Types

A step is a struct carrying what every step has, plus an enum for what differs.
Children belong only to sections, so they live in the `Section` struct rather
than on every step.

```rust
/// A single step in the procedure.
#[derive(Debug)]
pub struct Step {
    /// When the step finished, if it has.
    pub finished_at: Option<Instant>,

    pub kind: StepKind,

    pub started_at: Instant,

    pub status: StepStatus,
}

/// What a step displays and how the user interacts with it.
#[derive(Debug)]
pub enum StepKind {
    /// A gate: something the user has to make true, e.g. "Confirm that the
    /// roughing pump is running", before the procedure continues.
    ///
    /// Not a yes/no question. There's no answer to record — the step's status
    /// and `finished_at` say that it was passed and when — and no way to refuse:
    /// a user who can't make it true quits instead.
    Confirm {
        prompt: String,

        /// Taken by the application when the user confirms.
        ///
        /// `Some` exactly while this step is waiting for input, which is what
        /// `Section::pending_mut` looks for.
        responder: Option<oneshot::Sender<()>>,
    },

    /// A value typed by the user.
    Input {
        /// What the user has typed so far.
        buffer: String,

        /// Set when the buffer failed to parse, and shown beside the prompt.
        error: Option<String>,

        prompt: String,

        /// Taken by the application when the user submits a value.
        responder: Option<oneshot::Sender<String>>,

        /// The unit shown after the input field, e.g. "A".
        unit: Option<String>,

        /// The accepted value, set by the procedure once parsed.
        value: Option<String>,
    },

    /// A group of steps.
    Section(Section),

    /// One line of styled text: a remark, a measurement, an error, or something
    /// still in progress.
    ///
    /// These differ only in their spans and their status, so they are one kind
    /// with three constructors. A step that is still `Running` renders with a
    /// spinner and a live elapsed time, which is what makes it a "waiting"
    /// step; it stops the moment the procedure finishes it.
    ///
    /// `Vec<Span>` rather than a `Line` because the renderer prepends the
    /// indentation and status glyph and assembles the `Line` itself; a `Line`
    /// would carry its own alignment and style that the assembly would have to
    /// reconcile.
    Text { spans: Vec<Span<'static>> },
}

/// A group of steps with a title.
///
/// `Default` gives the root of the tree: no children and no title, because the
/// UI renders the root's children rather than the root itself.
#[derive(Debug, Default)]
pub struct Section {
    /// The steps nested beneath this one, in the order they began.
    pub children: Vec<Step>,

    pub title: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StepStatus {
    Done,
    Failed,
    #[default]
    Running,
}
```

All styling lives in constructors, so no caller assembles spans by hand and the
renderer never needs to know why a line looks the way it does:

```rust
impl StepKind {
    /// A fatal error, styled with `ERROR_STYLE`.
    ///
    /// The step's `status` carries the failure; this is only its appearance.
    pub fn error(message: impl Into<String>) -> Self;

    /// A labelled value, e.g. "Forward voltage: 9.61 ± 0.03 mV".
    ///
    /// The label is rendered in the default style and the value in
    /// `VARIABLE_STYLE`, the two-tone idiom the existing binary uses for every
    /// reading. It's what the operator's eye scans for, and where the `±`
    /// uncertainty goes.
    pub fn measurement(label: impl Into<String>, value: impl Display) -> Self;

    /// A plain line of text.
    pub fn text(text: impl Into<String>) -> Self;
}
```

`Section` is a struct rather than an inline variant body so that `push` and the
lookups below can hang off it.

One import note: don't `use ratatui::text::Text` in `steps.rs`, or it will
collide with `StepKind::Text`. Only `Line` and `Span` are needed.

### 9.2 Addressing a step

```rust
/// The position of a step in the tree, as child indices from the root.
///
/// Paths stay valid for the life of the run because steps are only ever
/// appended — never removed, never reordered.
pub type StepPath = Vec<usize>;

impl Section {
    /// Appends a step and returns its index in `children`.
    pub fn push(&mut self, kind: StepKind) -> usize;

    /// The section at `path`, if it exists and is a section.
    pub fn section_at_mut(&mut self, path: &[usize]) -> Option<&mut Section>;

    /// The step at `path`, if it exists.
    pub fn step_at_mut(&mut self, path: &[usize]) -> Option<&mut Step>;

    /// The step waiting on the user, if any.
    ///
    /// The procedure is sequential, so the step awaiting input can only be the
    /// last one in the tree: follow the last child down through nested sections
    /// and check whether what you land on still holds a responder. That's a
    /// walk of the tree's depth, not its size.
    ///
    /// IMPORTANT: if the procedure ever runs two sections concurrently, this
    /// assumption breaks and this has to become a depth-first search for a step
    /// holding a responder.
    pub fn pending_mut(&mut self) -> Option<&mut Step>;
}
```

The root is `Section::default()`, constructed once in `main` (§8.1).

**Why paths survive even though `push` is a `Section` method.** `Context` is
cloned into every nested section and lives across `await` points, so it can't
hold a `&mut Section` — that's a borrow of data inside a `Mutex` that has to be
released before every await. It has to hold something that can re-find the
section after re-locking, and a path is the smallest such thing. `push` then
lands on `Section` exactly as you'd want: the caller does
`root.section_at_mut(&self.parent)?.push(kind)`, so it's impossible to push a
child onto a `Text` step.

Two alternatives, for the record, both rejected:

- **Give each section its own `Arc<Mutex<Vec<Step>>>`** and have `Context` hold
  that handle directly. This does remove paths, but it replaces one lock with
  one per section, makes `Step` no longer plain data, and turns rendering and
  the pending search into recursive locking. Not worth it.
- **Store only the depth** and walk the rightmost spine, which is always the
  open section while the procedure is strictly sequential. Correct today, and
  silently wrong the first time anyone runs two sections concurrently.

### 9.3 How a confirmation works

A confirmation is a gate, not a question: the procedure states something the
operator has to make true — "Confirm that the roughing pump is running" — and
waits until they press `Enter`. There's no "no". An operator who can't make it
true quits, which cancels the token and unblocks the wait with
`ProcedureError::Cancelled`, and cleanup runs as on any other exit (§16).

The responder lives in the step, so no separate request message or pending
registry is needed. The procedure creates the channel, parks the sender in the
tree, and awaits the receiver:

```rust
pub async fn confirm(&self, prompt: impl Into<String>) -> Result<(), ProcedureError> {
    let (responder, confirmation) = oneshot::channel();
    let path = self.push(StepKind::Confirm {
        prompt: prompt.into(),
        responder: Some(responder),
    });

    // The lock is dropped by `push` before we await: the application needs it
    // to render the prompt and to confirm it.
    tokio::select! {
        confirmation = confirmation => confirmation.map_err(|_| ProcedureError::Cancelled)?,
        _ = self.cancel.cancelled() => return Err(ProcedureError::Cancelled),
    };

    self.finish(&path, StepStatus::Done);
    Ok(())
}
```

`push` and `finish` are two of the private `Context` helpers every one of its
methods is built from; they're defined in §11.1.

The application, on `Enter`:

```rust
let mut root = self.root.lock()?;
if let Some(Step { kind: StepKind::Confirm { responder, .. }, .. }) = root.pending_mut() {
    if let Some(responder) = responder.take() {
        // An error here only means the procedure has gone away.
        let _ = responder.send(());
    }
}
```

Three things follow from this arrangement, all worth comments in the code:

- **Taking the responder is what ends the pending state; sending on it is
  what ends the procedure's wait.** Once taken, `pending_mut` stops returning
  the step, so a second `Enter` can't confirm twice. The send wakes the
  procedure — as would dropping the responder unsent, which the receiver sees as
  an error and the procedure treats as cancellation.
- **The procedure owns status.** The app only sends the confirmation; the
  procedure sets `status` and `finished_at` when it wakes. One writer of
  lifecycle state, and it's the one that knows what happens next.
- **Cancellation is the token, not the drop.** The tree is `Arc`-shared and
  outlives the app, so dropping `App` doesn't drop the responder. The `select!`
  on `cancel.cancelled()` is what unblocks a pending prompt at shutdown.

`Context::input` works identically, except the app sends the buffer contents and
the procedure parses them: on failure it writes `error` back into the step,
installs a fresh responder, and awaits again, so the user is re-prompted in
place — and the step stays the last one in the tree, which is what keeps
`pending_mut` correct.

### 9.4 Rendering and scrolling

Every step renders as text, so the tree becomes a list of lines, built by a
method on `Step` that sections call recursively on their children:

```rust
impl Step {
    /// The step as rendered lines.
    ///
    /// Text is never wrapped: a step produces a fixed number of lines — one for
    /// itself, plus whatever its children produce — and anything too wide is
    /// ellipsified to fit. That's what makes scrolling a slice index: the line
    /// count doesn't depend on the width.
    ///
    /// `width` is needed anyway, because a line with a right-hand element — a
    /// section's total elapsed time, a running step's elapsed seconds — lays it
    /// out against the right edge and ellipsifies the left part to fit, so the
    /// time stays visible however long the title is.
    ///
    /// A section renders its title and then calls this on each of its children
    /// with a width `INDENT` columns narrower, indenting what comes back — so
    /// the recursion carries the nesting and no depth parameter is needed.
    ///
    /// Note: this deliberately is not `Widget::render`. Do not implement
    /// `Widget` for `Step` — it would have to draw into a `Rect`, and then
    /// scrolling would need the buffer machinery described below.
    pub fn render(&self, width: u16) -> Vec<Line<'static>>;
}

impl Section {
    /// The section's children as rendered lines.
    ///
    /// Used by `Step::render` for nested sections, and by the UI for the root,
    /// whose own title is never shown.
    pub fn render_children(&self, width: u16) -> Vec<Line<'static>>;
}
```

**Laying out a line.** Each line is assembled left to right: the status glyph,
then the step's own spans, then — if the step has one — a right-hand element
padded flush to `width`. Every line is fitted with a shared helper:

```rust
/// Truncates `spans` to `width` columns, ending the last surviving span with
/// `…` when anything was cut.
fn ellipsify(spans: Vec<Span<'static>>, width: u16) -> Vec<Span<'static>>;
```

A section title is ellipsified to `width - glyph - gap - elapsed`, a running
step's text to the same less its elapsed seconds, and a step with nothing to its
right to `width - glyph`. Everything is ellipsified rather than left to clip at
the block edge, so a truncated line always says so — silent truncation looks
like a short line, and a step list is exactly where a reader would fail to
notice.

**How a section indents its children.** Each line comes back as a
`Line<'static>`, which is a `Vec<Span<'static>>`. Indenting is prepending one
span of spaces:

```rust
/// The columns each level of nesting indents by.
const INDENT: u16 = 2;

fn indent(line: Line<'static>) -> Line<'static> {
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" ".repeat(INDENT as usize)));
    spans.extend(line.spans);
    Line::from(spans)
}
```

So `Section::render_children` is
`self.children.iter().flat_map(|c| c.render(width.saturating_sub(INDENT))).map(indent)`.
Narrowing the width before rendering and indenting after means a nested
section's right-aligned elapsed time still lands in the correct column once the
indent is prepended.

Two rules make this work, both worth comments: **style at the span level, never
the line level**, so prepending a span can't interact with a line-level style;
and **never set `Line::alignment`** — a section pads to `width` with spaces to
right-align its elapsed time, because ratatui's alignment would be computed
against the drawing area rather than the indented width.

Drawing is then:

```rust
let lines = root.render_children(inner.width);
let visible = &lines[state.offset..(state.offset + inner.height as usize).min(lines.len())];
Paragraph::new(visible.to_vec()).render(inner, buf);
```

No `Wrap` on the `Paragraph`: the lines are already fitted to the width, and
without `Wrap` ratatui truncates rather than reflows anything that still
overruns — a backstop, not the mechanism.

Rendering rules:

- Status glyph prefix: `⟳` running (spinner frames `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`, indexed from
  the step's own elapsed time so no tick counter has to be passed in), `✔` done,
  `✖` failed.
- Sections render their title bold, with elapsed time right-aligned.
- A `Running` text step renders its elapsed seconds at the right edge.
- The pending step is highlighted (yellow, as `CONFIRMATION_STYLE` in the
  existing binary) and shows its own hint: `[Enter] Confirm`, or the text buffer
  with a block cursor.

**Why not render the tree recursively into an oversized buffer.** That approach
works, and it's what `tui-scrollview` does: allocate a `Buffer` the full height
of the content, render widgets into it at their own offsets, then copy the
visible window into the frame's buffer. It's the right answer when the content
contains widgets that *can't* be expressed as lines — a `Chart`, a `Gauge`, a
`Sparkline` — because those insist on drawing into a `Rect`. With every step
being one line of text it buys nothing and costs a measure pass, a buffer
allocation per frame, and a second place for the line count to disagree with
the render.

If a chart step is ever added, switch to the buffer approach for the whole list
rather than special-casing one kind — and look at `tui-scrollview` before
writing it by hand. Leave a comment on `Step::render` saying so.

**What ratatui already provides, and what's ours.** Worth knowing so nobody
reimplements the wrong half:

| Ratatui offers | What it does | Why it isn't the whole answer |
|---|---|---|
| `Paragraph::scroll((y, x))` | Skips `y` rendered lines | Stateless: no clamping, no follow behaviour. Equivalent to the slice above, but it re-renders every line each frame. |
| `List` + `ListState` | Keeps an `offset`, tracks a selection, scrolls to keep it visible | The closest fit, and viable now that lines are clipped rather than wrapped — one line per `ListItem`. Its selection semantics still aren't ours (the pending step isn't a cursor the user moves), and driving `offset` directly is about as much code as the slice. Use it if it reads better; nothing else depends on the choice. |
| `Scrollbar` + `ScrollbarState` | Draws a scrollbar | Display only; no scrolling logic. Not used — no scrollbar. |
| `tui-scrollview` (third-party) | The oversized-buffer pattern above | Only needed for non-text content. |

What's genuinely ours is follow-the-tail and keeping the pending step visible —
maybe twenty lines, and neither has a primitive in the library.

### 9.5 Locking rules

- `std::sync::Mutex`, not `tokio::sync::Mutex`. Every critical section is a
  short synchronous mutation or one render pass.
- **The lock is never held across an `.await`.** Put this in a comment where the
  root is declared. `Context` methods take the lock, mutate, and drop it before
  awaiting.
- The app takes the lock once per frame, for the duration of the draw.
- A poisoned lock is fatal: log with `error!` and shut down, matching how
  `Controller` handles its own poisoned mutex.

---

## 10. Measurement data and persistence

### 10.1 The measurement type

Every quantity is measured the same way: take `SAMPLE_COUNT` samples (n = 20),
then ask Python for their mean and standard error. Both halves are kept — the
samples so the analysis can be redone or scrutinised later, the derived pair so
consumers don't have to recompute it.

```rust
/// A measured quantity: the samples taken, and the value derived from them.
///
/// `value` and `uncertainty` are the mean and standard error returned by
/// `mean_and_standard_error.py` (§12), not calculated in Rust.
#[derive(Debug, Deserialize, Serialize)]
pub struct Measurement {
    /// The individual samples, in the order they were taken.
    pub samples: Vec<f64>,

    /// The standard error of the mean.
    pub uncertainty: f64,

    /// The mean of the samples.
    pub value: f64,
}
```

Not implemented yet, but this is the shape every measured field takes, and the
unit goes in the name of the field holding it — `filament_voltage_volts:
Measurement` — since the consumer is another program with no doc comments to
read.

### 10.2 The results struct

```rust
/// Everything measured during one characterisation run.
///
/// Written to disk at checkpoints throughout the procedure so that a run which
/// fails part-way still leaves its partial results behind.
#[derive(Debug, Deserialize, Serialize)]
pub struct Characterisation {
    /// An identifier for the filament under test, entered by the operator.
    pub filament_id: Option<String>,

    /// When the run started, in seconds since the Unix epoch.
    pub started_at: u64,

    // TODO: one field per measured quantity as the measurements are
    // implemented, each a `Measurement` (or a `Vec` of them for a sweep), named
    // with its unit. If any of them needs a timestamp, `Context` grows a method
    // returning seconds since `started_at` — nothing needs one yet.
}

impl Characterisation {
    /// Creates the results for a run starting now.
    pub fn new() -> Self;

    /// Where the results are written, named for `started_at`.
    pub fn path(&self) -> PathBuf;

    /// Writes the results to `path()`.
    ///
    /// Writes to a temporary file and renames it, so killing the program can't
    /// leave a half-written file behind.
    pub fn save(&self) -> Result<(), ResultsError>;
}
```

`filament_id` is `None` until the operator enters it. It's the first thing the
procedure asks for, before any measurement is taken, so no results file that
contains a measurement can lack it. But results are saved on every exit path
(§10.3), so a run quit at that first prompt still writes a file — and `None`,
serialised as `null`, says the identifier was never given where an empty string
would look like one that was.

- File name: `out/characterisation-<started_at>.json`, so successive runs don't
  overwrite each other. `Characterisation::path` builds it from `started_at`
  rather than reading the clock again, so the name always matches the time
  recorded inside.
- Format: `serde_json::to_string_pretty`. Confine the format choice to `save`.
- Write to `<path>_new` and `fs::rename` over the real path, the same idiom the
  existing binary uses in `log_pressure` and `graph_pressure`.
- `fs` calls are inline rather than in `spawn_blocking`: the file is small and
  this matches the existing binary. Note it in a comment.

There is no `Results` wrapper type. The struct owns its own `save`, and the
sharing is folded into `Context` (§11.1) — which has to be shared anyway, since
`Context` is cloned into every nested section.

### 10.3 When to save

Saving is **explicit**: the procedure calls `ctx.save()` after each measurement
completes, and the runner saves once more after the procedure returns, on both
the success and error paths, so the final state always reaches disk. The first
save adds a text step reading "Saved results to …"; later saves are logged with
`info!`.

A failed save never aborts the procedure. Log it with `error!`, push a step
marked `Failed`, and carry on — losing the record is worse than losing the rest
of the run.

---

## 11. The procedure

### 11.1 Context

The handle the procedure uses to emit steps, collect user input, read the
hardware and record measurements. Cheap to clone; a clone with a different
`parent` path is what nesting is built from. Every field is handed to it when
the procedure task is spawned (§8.1).

```rust
/// A handle used by the procedure to drive the UI and record results.
#[derive(Clone, Debug)]
pub struct Context {
    /// Cancelled by the application at shutdown, which unblocks any prompt this
    /// context is waiting on.
    cancel: CancellationToken,

    /// The run's measurements, shared because `Context` is cloned per section.
    characterisation: Arc<Mutex<Characterisation>>,

    /// The section this context's steps are appended to.
    parent: StepPath,

    /// The step tree.
    root: Arc<Mutex<Section>>,

    /// The readings published by the hardware poll task.
    snapshots: watch::Receiver<Option<Snapshots>>,
}

impl Context {
    /// Runs `f` inside a new section, and marks the section done when it ends.
    ///
    /// The context passed to `f` is parented to the new section, so any steps
    /// it emits are nested beneath it. On error the section is marked failed
    /// and the error propagates.
    pub async fn section<F, Fut>(
        &self,
        title: impl Into<String>,
        f: F,
    ) -> Result<(), ProcedureError>
    where
        F: FnOnce(Context) -> Fut,
        Fut: Future<Output = Result<(), ProcedureError>>;

    /// Waits for the user to confirm that something is the case.
    ///
    /// A gate rather than a question (§9.3): it returns once they have, and the
    /// only way past it otherwise is quitting.
    pub async fn confirm(&self, prompt: impl Into<String>) -> Result<(), ProcedureError>;

    /// Asks the user for a value and waits until they enter a valid one.
    pub async fn input<T: FromStr>(
        &self,
        prompt: impl Into<String>,
        unit: Option<&str>,
    ) -> Result<T, ProcedureError>;

    /// Adds a labelled measurement to the step list.
    pub fn measurement(&self, label: impl Into<String>, value: impl Display);

    /// Records measurements into the run's results.
    ///
    /// This only mutates memory. Call `save` to write them to disk.
    pub fn record(&self, f: impl FnOnce(&mut Characterisation));

    /// Writes the run's results to disk.
    ///
    /// Failures are reported as a failed step and logged, but never abort the
    /// procedure.
    pub fn save(&self);

    /// The most recent readings.
    ///
    /// Always `Some` inside the procedure: it isn't started until the first
    /// snapshot has arrived.
    pub fn snapshot(&self) -> Snapshots;

    /// Adds a line of text.
    pub fn text(&self, text: impl Into<String>);

    /// Waits until `predicate` holds for a snapshot, showing a spinner.
    ///
    /// This is how the procedure waits on the chamber — e.g. for the pressure
    /// to fall below the TMP's maximum backing pressure — using the readings
    /// the poll task is already taking rather than issuing its own.
    pub async fn wait_for(
        &self,
        label: impl Into<String>,
        predicate: impl Fn(&Snapshots) -> bool,
    ) -> Result<(), ProcedureError>;

    /// Runs `f`, showing a spinner labelled `label` until it completes.
    ///
    /// Pushes a text step and finishes it when `f` returns; a text step that
    /// hasn't finished is what a spinner is (§9.1).
    pub async fn waiting<F, Fut, T>(&self, label: impl Into<String>, f: F) -> Result<T, ProcedureError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ProcedureError>>;

    // Private helpers. Every method above is built from these three, and they
    // are the only places that take the step tree's lock.

    /// Appends a step beneath this context's section and returns its path.
    fn push(&self, kind: StepKind) -> StepPath;

    /// Applies `f` to the kind of the step at `path`.
    ///
    /// Used to write a parse error back into an input step before
    /// re-prompting, or the value it accepted once one parses.
    fn update(&self, path: &[usize], f: impl FnOnce(&mut StepKind));

    /// Sets the step's status and `finished_at`.
    fn finish(&self, path: &[usize], status: StepStatus);
}
```

Every `await` inside `Context` selects on `self.cancel.cancelled()` and returns
`ProcedureError::Cancelled`. The synchronous methods take the relevant lock,
mutate, and release it; none can fail in a way the caller must handle, which is
why they don't return `Result`.

```rust
#[derive(Debug, Error)]
pub enum ProcedureError {
    #[error("aborted by operator: {0}")]
    Aborted(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
    #[error("hardware error: {0}")]
    Hardware(#[from] HardwareError),
    #[error("python error: {0}")]
    Python(#[from] PythonError),
}
```

### 11.2 The procedure itself

No `Stage` type, no registry, no function pointers. The procedure is one async
function calling others, and `ctx.section` is what makes the nesting visible:

```rust
/// Characterises a filament.
///
/// The top-level sections of the run, in order. Each one is an ordinary async
/// function, so the procedure reads as the sequence of things that happen.
pub async fn characterise(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    ctx.section("Preparing", |ctx| prepare(ctx, hardware.clone())).await?;
    ctx.section("Pumping down chamber", |ctx| pump_down(ctx, hardware.clone())).await?;
    ctx.section("Measuring cold resistance", |ctx| measure_cold_resistance(ctx, hardware.clone())).await?;
    ctx.section("Sweeping heating current", |ctx| sweep_heating_current(ctx, hardware.clone())).await?;
    ctx.section("Finishing", |ctx| finish(ctx, hardware.clone())).await?;
    Ok(())
}
```

Sections nest freely: any of those functions can call `ctx.section` again for a
sub-section, or just push steps. Control flow is ordinary Rust — a loop over
sweep points, an early return on a failed check, a conditional section —
which is the point of doing it this way rather than through a list of stages.

### 11.3 The stub bodies

These exercise every step kind, every interaction, the save path and the Python
path. Real logic is a `TODO`.

```rust
async fn pump_down(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    ctx.confirm("Confirm that the roughing pump is running").await?;

    ctx.wait_for("Waiting for chamber to reach TMP operating pressure", |s| {
        s.vacuum.pressure.value < TMP_MAXIMUM_BACKING_PRESSURE_MBAR
    })
    .await?;

    hardware.vacuum.set_tmp_running(true).await?;
    ctx.text("Turned on the TMP");

    // TODO: wait on the real filament operating pressure rather than the TMP
    // reaching speed.
    ctx.wait_for("Waiting for TMP to reach speed", |s| {
        s.vacuum.tmp_current_rotation_speed >= s.vacuum.tmp_target_rotation_speed
    })
    .await?;

    ctx.measurement(
        "Base pressure",
        format!("{:.1e} mbar", ctx.snapshot().vacuum.pressure.value),
    );
    Ok(())
}

async fn sweep_heating_current(ctx: Context, hardware: Hardware) -> Result<(), ProcedureError> {
    let maximum: f64 = ctx.input("Maximum heating current", Some("A")).await?;

    // TODO: replace with a real sweep that settles at each point, takes
    // SAMPLE_COUNT samples of both quantities, reverses the polarity to cancel
    // the Seebeck voltage, and records a `Measurement` per point.
    hardware.filament.set_output_enabled(true).await?;
    let step = maximum / SWEEP_POINTS as f64;
    let mut current = step;
    while current <= maximum {
        hardware.filament.set_heating_current(current).await?;
        tokio::time::sleep(SWEEP_SETTLING_TIME).await;

        let voltage = hardware.filament.get_filament_voltage().await?;
        ctx.measurement(
            format!("{:.3} A", current),
            format!("{:.1} mV", voltage * 1000.0),
        );

        current += step;
    }

    hardware.filament.set_heating_current(0.0).await?;
    hardware.filament.set_output_enabled(false).await?;
    Ok(())
}
```

The rest:

- `prepare` asks for the filament identifier (`ctx.input` → `ctx.record` →
  `ctx.save`) before anything else — the first save is what tells the operator
  where the results are being written — then calls
  `set_heating_voltage(MAXIMUM_HEATING_VOLTAGE_VOLTS)` once so the channel runs
  in constant current, and confirms the chamber is sealed and the filament is
  mounted.
- `measure_cold_resistance` takes `SAMPLE_COUNT` samples of voltage and current
  at a small current in `Polarity::Forward`, then again in `Polarity::Reverse` —
  disabling the output between them, as `set_polarity` requires — calls
  `python::mean_and_standard_error` on each set, shows the results as
  measurement steps, records them and saves. This is the one stub that runs the
  whole measurement pattern end to end, so make it the reference for the others.
- `finish` zeroes the supply, returns the relays to `Polarity::Nil`, and saves a
  final time.

---

## 12. Python integration

Statistics and uncertainty propagation are done in Python with the
`uncertainties` package, so the crate ships scripts and expects a virtual
environment beside them.

Each script has **its own interface**: arguments in, one JSON object on stdout.
Nothing passes the whole `Characterisation` struct to Python — the Rust side
sends the numbers a particular calculation needs and stores what comes back.

### 12.1 Layout and constants

```rust
/// The directory holding the analysis scripts and their virtual environment.
///
/// Resolved at compile time from the crate root, since the binary is run from
/// the source tree. If the compiled path doesn't exist at run time, fall back
/// to `./python` relative to the working directory.
const PYTHON_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/python");

/// The interpreter inside the virtual environment.
const VENV_PYTHON: &str = ".venv/bin/python";
```

`python/requirements.txt` pins `uncertainties`. A comment at the top of
`python.rs` states the one-off setup:

```
python3 -m venv python/.venv
python/.venv/bin/pip install -r python/requirements.txt
```

### 12.2 Start-up check

Before `ratatui::init()`, so failures print normally rather than into the
alternate screen:

1. Check `PYTHON_DIR` resolves to an existing directory, falling back as above.
2. Check the interpreter at `<python dir>/.venv/bin/python` exists.
3. Run `<interpreter> -c "import uncertainties"` with a short timeout
   (`PYTHON_CHECK_TIMEOUT`, 10 s) and require exit status 0.

Any failure aborts start-up with an error naming the missing piece **and the two
commands above**, so the operator can fix it without reading the source. Log the
resolved interpreter path with `info!` on success.

### 12.3 Invocation

```rust
/// Runs a script from the crate's `python` directory.
///
/// `args` are passed positionally. The script is expected to print a single
/// JSON object on stdout, which is deserialised into `T`. Anything it prints on
/// stderr is logged.
pub async fn run_script<T: DeserializeOwned>(
    script: &str,
    args: &[String],
) -> Result<T, PythonError>;

/// The mean and standard error of a set of samples.
///
/// Wraps `mean_and_standard_error.py`, which is the calculation behind every
/// `Measurement` (§10.1).
pub async fn mean_and_standard_error(samples: &[f64]) -> Result<(f64, f64), PythonError>;
```

- Uses `tokio::process::Command` with the venv interpreter, never the system
  `python3`, and never a shell.
- Wrapped in `tokio::time::timeout(PYTHON_TIMEOUT, ...)`; a script that hangs
  must not hang the procedure.
- A non-zero exit status, a timeout, or unparseable stdout are all errors
  carrying the script name and the captured stderr.
- `PythonError` is a `thiserror` enum.

### 12.4 The first script

`python/mean_and_standard_error.py` takes the samples as positional arguments
and prints `{"value": <mean>, "uncertainty": <standard error>}`. It imports
`uncertainties` even though the mean and standard error don't strictly need it,
so the venv is exercised on the path that will later do the real propagation,
and carries a `TODO` saying that resistance and its propagated uncertainty are
calculated by scripts added beside it.

---

## 13. Application state

```rust
#[derive(Debug)]
struct App {
    /// Cancelled as the first step of shutdown, stopping the poll task and
    /// unblocking any prompt the procedure is waiting on.
    cancel: CancellationToken,

    /// The step tree, shared with the procedure task.
    root: Arc<Mutex<Section>>,

    /// Why the application is shutting down, if it is.
    shutdown: Option<Shutdown>,

    /// The most recent readings, or `None` before the first poll.
    snapshots: Option<Snapshots>,

    /// When the application started.
    start_time: Instant,

    /// Scroll position of the steps list.
    steps_state: StepsState,
}

/// Why the application is shutting down.
///
/// `App::shutdown` is `None` for a normal run. Setting it is what ends the
/// event loop: `Normal` exits at the end of the current iteration, while
/// `Error` keeps rendering until the operator presses a key, so they can read
/// what went wrong before the terminal is restored.
#[derive(Debug)]
enum Shutdown {
    /// The procedure finished, or the operator quit.
    Normal,

    /// The run failed. The message is already shown as a failed text step; this
    /// copy is what `main` returns and prints after the terminal is restored.
    Error { acknowledged: bool, message: String },
}
```

The `None` snapshot is the only start-up state. Once the first one is handled,
it is `Some` and stays that way — a poll failure never clears it, it retries and
then becomes fatal (§8.1). Handling that first snapshot is also what spawns the
procedure task; the app holds the procedure's inputs until then. Note in a
comment that this ordering is deliberate, so nobody "simplifies" it by spawning
the procedure in `main`.

Until then, the vacuum and filament blocks render their border and title with a
centred `Waiting for readings…` in `SUBTLE_TEXT_STYLE` — the same treatment the
existing binary gives `Measuring pressure...`. Each block takes
`Option<&Snapshot>` and handles the `None` case itself, so there is exactly one
empty-state path per block and no placeholder values anywhere.

---

## 14. UI

```
┌ Vacuum ─────────────┐┌ Filament ────────────┐┌ Run ───────────────┐
│ Pressure:  1.2e-05 m││ Current:    1.850 A  ││ Elapsed:  00:12:31 │
│ TMP:       Running  ││ Polarity:   Forward  ││ Section:  Sweeping │
│ Speed:     1500/1500││ Output:     Enabled  ││ Status:   Running  │
│ Current:   0.42 A   ││ Voltage:    482.1 mV ││                    │
└─────────────────────┘└──────────────────────┘└────────────────────┘
┌ Steps ────────────────────────────────────────────────────────────┐
│ ✔ Preparing                                            00:00:42   │
│   ✔ Filament: W-0007                                              │
│   ✔ Confirm that the chamber is sealed                            │
│ ✔ Pumping down chamber                                 00:06:05   │
│   ✔ Confirm that the roughing pump is running                     │
│   ✔ Waiting for chamber to reach TMP operating pressure   3.1 s   │
│   ✔ Turned on the TMP                                             │
│   ✔ Base pressure: 2.4e-06 mbar                                   │
│ ⟳ Measuring cold resistance                            00:00:08   │
└───────────────────────────────────────────────────────────────────┘
 [↑/↓] Scroll   [Enter] Confirm   [Esc] Quit
```

```rust
let [top, steps, shortcuts_bar] = Layout::vertical([
    Constraint::Length(6),
    Constraint::Min(0),
    Constraint::Length(1),
])
.areas(area);

let [vacuum, filament, run] = Layout::horizontal([
    Constraint::Min(24),
    Constraint::Min(24),
    Constraint::Length(24),
])
.areas(top);
```

The run block's `Section` row is the title of the last root-level section in the
tree, since there's no stage list to count against.

Reuse the style constants from the existing binary verbatim —
`BLOCK_TITLE_STYLE`, `CONFIRMATION_STYLE`, `ERROR_STYLE`, `SUBTLE_TEXT_STYLE`,
`VARIABLE_STYLE` — and the same idiom of a label in default style followed by
the value in `VARIABLE_STYLE`. Blocks are `Block::bordered()` with
`Padding::symmetric(2, 1)` where there's room; the steps block can use
`Padding::symmetric(2, 0)` to save vertical space.

Formatting: pressure `{:.1e} mbar`; heating current `{:.3} A`; filament voltage
in mV to one decimal; elapsed times `HH:MM:SS`, or `{:.1} s` under a minute. No
resistance is shown anywhere in the UI (§7.1). The filament block has room for
another row; a later version adds emission current there (§17).

Implement each block as a small struct with a `Widget` impl over a borrowed
snapshot, matching `impl Widget for &State` in the existing binary.

### 14.1 The steps widget

```rust
/// Scroll position of the step list.
#[derive(Debug, Default)]
pub struct StepsState {
    /// If the view follows the end of the list as steps are added.
    follow: bool,

    /// The index of the first visible line.
    offset: usize,
}
```

- Lines are rebuilt from the tree every frame. That's fine: ratatui diffs
  buffers, so an unchanged frame costs no terminal I/O, and the running step's
  spinner means most frames change anyway.
- When `follow` is true (the default), `offset` is pinned so the last line is
  visible. Any upward scroll sets `follow = false`; `End`, or scrolling back to
  the bottom, sets it true again.
- Clamp `offset` to `lines.len().saturating_sub(height)` every frame, since the
  content grows and the terminal can be resized under it.
- Lines are ellipsified to the block width, never wrapped (§9.4). No horizontal
  scrolling.
- No scrollbar. Scrolling alone is enough.
- A pending step must always be visible: if `Section::pending_mut` finds one
  while `follow` is false, scroll to it and re-enable following.

---

## 15. Input handling

Key events are routed in this order:

1. `Ctrl+C` — always quits immediately.
2. If `Section::pending_mut` returns a step, it consumes the event:
   - **Confirm**: `Enter` confirms; `Esc` → quit. There's no key for "no" —
     a confirmation is a gate, not a question (§9.3).
   - **Input**: printable characters append to the buffer; `Backspace` deletes;
     `Enter` takes the responder and sends the buffer; `Esc` → quit.
3. Otherwise:
   - `↑` / `↓`, `PgUp` / `PgDn`, `Home` / `End` — scroll the step list.
   - `q` / `Esc` — quit.

When `App::shutdown` is `Shutdown::Error { acknowledged: false, .. }`, any key
sets `acknowledged` and ends the loop.

Only handle `event.is_press()`, as the existing binary does, so key repeats on
some terminals don't double-fire. The shortcuts bar text changes with context,
again matching the existing binary.

---

## 16. Errors, shutdown and cleanup

`main` keeps the existing shape — an `async` block whose result is captured so
`ratatui::restore()` always runs — with the start-up checks before the terminal
is touched and cleanup after:

```rust
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AnyError> {
    init_logging()?;

    let args = Arguments::parse();

    // Check the Python environment before taking over the terminal, so the
    // operator sees the error and how to fix it.
    python::check_environment().await?;

    let hardware = build_hardware(&args)?;
    let characterisation = Characterisation::new();
    let results_path = characterisation.path();
    info!("Writing results to {}", results_path.display());

    let mut terminal = ratatui::init();
    install_panic_hook();

    // Use an `async` block to ensure we clean up on error.
    let result: Result<(), AnyError> = async {
        // ... create the shared state (§8.1), spawn tasks, run the app loop ...
    }
    .await;

    // Always put the filament system into a safe state. A hung instrument must
    // not stop us restoring the terminal, so time it out.
    if tokio::time::timeout(CLEANUP_TIMEOUT, hardware.filament.enter_safe_state())
        .await
        .is_err()
    {
        error!("timed out putting the filament system into a safe state");
    }

    ratatui::restore();

    println!("Results written to {}", results_path.display());
    println!("The TMP is still running. Use the vacuum control binary to stop it.");

    if let Err(e) = &result {
        eprintln!("Error: {}", e);
    }

    result
}
```

The results are saved by the procedure runner, which owns the `Characterisation`
through `Context` and saves on every exit path — so `main` doesn't need a save
of its own.

`install_panic_hook` chains onto the existing hook, calling `ratatui::restore()`
first so a panic message isn't swallowed by the alternate screen. Note in a
comment that the panic hook **cannot** run the async cleanup — this is why the
procedure task is never allowed to panic, and why every fallible call inside it
returns an error instead.

Shutdown sequence, triggered by the operator quitting, a procedure error, a
fatal hardware failure, or the procedure completing:

1. The app loop sets `App::shutdown` and cancels the `CancellationToken`.
2. A pending `confirm` or `input` unblocks with `ProcedureError::Cancelled` via
   the token (§9.3).
3. The loop waits up to `SHUTDOWN_TIMEOUT` (2 s) for the procedure task's
   `JoinHandle`, then aborts it. An aborted task can't save, which is why saves
   happen at every measurement rather than only at the end.
4. Control returns to `main`, which runs `enter_safe_state`.

On a procedure error or a fatal hardware failure the app pushes a step built
with `StepKind::error`, marks it and the enclosing sections `Failed`, and sets
`Shutdown::Error`, which keeps the UI up until the operator presses a key. Log
the error with `error!` regardless.

---

## 17. Extension points for emission current

Emission current is out of scope for this version, but the structure must make
adding it a local change. Leave a short `// Later: emission current` comment at
each of these points, and nowhere else:

1. `FilamentSystem` — a `get_emission_current` method, and its inclusion in
   `snapshot`.
2. `FilamentSnapshot` — an `emission_current` field.
3. `hardware/real.rs` — the instrument behind it, alongside the oscilloscope.
4. `ui/filament.rs` — one more row in the block, which has the space for it.
5. `results.rs` — one more `Measurement` field on `Characterisation`.
6. `procedure/characterisation.rs` — one more `ctx.section(...)` line in
   `characterise` and one more async function.

If adding emission current would require touching anything outside that list,
the structure is wrong — fix it now rather than working around it later.

---

## 18. Code style

Match the attached code exactly. In particular:

- **Australian/British spelling in prose and comments** — "initialise",
  "characterise", "behaviour". Identifiers follow suit.
- **Doc comments on every public item and every struct field**, written as
  sentences with full stops. Multi-paragraph where a hardware quirk needs
  explaining, with the extra paragraphs after a blank `///` line.
- **Fields and variants in alphabetical order**, as in `App` and `State`. Enum
  variants may be grouped under `// Section` comments (as `AdcError` does) and
  are alphabetical or numerical within a group.
- **Methods**: `new` first, then public methods roughly alphabetically, then
  private helpers. Reads are `get_*`, writes are `set_*`, as in `host`.
- **`thiserror` for all domain errors**, messages lowercase and unpunctuated,
  `#[from]` for wrapped errors; `type AnyError = Box<dyn std::error::Error>` in
  the binary only.
- **`error!` immediately before returning an `Err`**, including the value that
  caused it.
- **Comments explain why, not what** — especially anything protecting hardware.
  Safety-critical constants carry a doc comment citing the manual and any extra
  margin applied, in the manner of `TMP_MAXIMUM_BACKING_PRESSURE_MBAR`.
- Units in names and doc comments (`_mbar`, `_amps`, `in hertz`).
- `std::sync::Mutex` for the shared step tree and results, never held across an
  `.await`; `tokio::sync::Mutex` only where a lock genuinely spans one.
- rustfmt defaults; `cargo clippy --all-targets -- -D warnings` must be clean.
- No `unwrap()` outside tests except where an invariant has just been asserted,
  and then with a comment saying which.
- Python: PEP 8, type hints, a module docstring saying what the script consumes
  and prints.

Reuse rather than redefine: import `Pressure`, `PressureUnit`, `AdcError`,
`TmpError` and `Polarity` from `host`, and lift
`TMP_MAXIMUM_BACKING_PRESSURE_MBAR` (12.0, with its existing comment) into the
new crate's constants alongside `MAXIMUM_HEATING_CURRENT_AMPS` and
`MAXIMUM_HEATING_VOLTAGE_VOLTS`.

---

## 19. Build order

Each step should compile, pass clippy, and run.

1. Hoist the shared dependencies to `[workspace.dependencies]` (§4.1) and
   confirm `host` and the existing binary build unchanged.
2. Crate skeleton, `Arguments`, `init_logging`, terminal init/restore, panic
   hook, empty app loop that quits on `q`.
3. `python.rs` plus `requirements.txt` and `mean_and_standard_error.py`; the
   start-up check fails helpfully with no venv and passes with one.
4. `steps.rs`: `Step`, `Section`, the `StepKind` constructors, paths, `render`,
   and the steps widget driven by a hard-coded tree. Scrolling, indentation,
   clipping, ellipsifying and following all work.
5. `hardware/`: traits, snapshots, mocks. `--mock` is wired up;
   `RealVacuumSystem` follows, then the `host` stubs and the filament adapter.
6. Poll task and watch channel, the empty state, then the three top blocks
   showing live mock data.
7. `results.rs`: `Measurement`, `Characterisation`, `save`.
8. `Context` and the procedure, with the stub bodies. Confirm and input
   round-trip, `wait_for` resolves off the watch channel, saves land on disk,
   and `measure_cold_resistance` gets a real answer back from Python.
9. Shutdown, cancellation, fatal hardware failure and `enter_safe_state` on
   every path.

## 20. Acceptance criteria

- `cargo run -p characteriser -- --mock` runs end to end with no
  hardware: the blocks show the empty state until the first poll, every section
  executes, the confirm and input prompts accept keyboard input, and the step
  list scrolls, indents and follows.
- Narrowing the terminal ellipsifies long step lines rather than wrapping them,
  a section's elapsed time stays visible with its title ellipsified, and the
  scroll offset stays valid across the resize.
- Running without the virtual environment fails before the TUI starts, with a
  message naming the two commands that create it.
- `cargo run -p characteriser -- /dev/tty.usbmodem11201 1 001` reads
  real pressure and TMP values into the top blocks, logging at start-up that the
  filament system is mocked.
- `out/characterisation-<timestamp>.json` exists after a run, parses as JSON,
  carries the filament identifier, and is present and valid even when the run is
  quit half way through or fails.
- `measure_cold_resistance` shows a value and an uncertainty that came back from
  Python, proving the round trip.
- Quitting at any point — including while a prompt is pending — restores the
  terminal, logs the shutdown, and runs `enter_safe_state`, leaving the relays
  de-energised and the TMP running.
- Three consecutive poll failures end the run with an error step, cleanup and a
  non-zero exit; a single failure only logs a warning.
- Nothing is ever printed to stdout/stderr while the TUI is on screen; all
  diagnostics land in `out/app.log`.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## 21. Future work (do not implement)

- Emission current measurement (§17).
- Real bodies for the `host` `PowerSupply` and `Oscilloscope` methods.
- The real measurement pattern: n samples per quantity, both polarities, a
  `Measurement` per recorded field.
- Resistance and its propagated uncertainty, in a Python script beside
  `mean_and_standard_error.py`.
- Chart steps, which would mean switching the step list to the oversized-buffer
  rendering described in §9.4.
- Replaying a recorded run against the mock for UI development.
- Resuming a procedure from a given section.
