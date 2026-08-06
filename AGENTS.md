# AGENTS.md

## Project

**Termfold** is a small, traditional terminal multiplexer for Linux, WSL, and
native x86-64 Windows.

The maintained project documentation is limited to:

```text
AGENTS.md
README.md
TASKS.md
REQUIREMENTS.md
```

`LICENSE` remains a separate legal file. Do not create another project Markdown
document without explicit user approval.

`REQUIREMENTS.md` defines product behaviour. This file defines the AI and
engineering workflow. When they conflict, stop and request clarification.

Primary engineering goals:

- One statically linked Linux executable
- One standalone native x86-64 Windows executable
- Small binary size
- Fast startup
- Low memory usage
- Traditional terminal UI
- No mandatory plugins
- No runtime network access
- Minimal external dependencies

## Communication

- Keep every reply short and precise.
- Do not repeat the user's requirements.
- Do not provide long explanations unless requested.
- Discuss the approach before generating or modifying code.
- Ask before making architectural or public-behaviour changes.
- Do not make unrelated changes.
- State assumptions clearly.
- Report blockers immediately.

## Approval Gate

- `APPROVE` is the only authorization keyword for workspace or external changes.
- The keyword is case-sensitive and must appear as a standalone word in the
  request that describes the change.
- Approval applies only to the stated scope. Do not infer, reuse, or broaden it.
- Without `APPROVE`, only read, inspect, analyse, and propose changes.
- File edits, generated documents, dependency changes, build/test/lint commands,
  Git mutations, releases, and external writes each require in-scope approval.
- An implementation request without `APPROVE` is not authorization to change
  files.

## Documentation Ownership

Each fact must have one primary home. Do not copy full specifications between
files.

| File | Authority | Must contain | Must not contain |
| --- | --- | --- | --- |
| `AGENTS.md` | AI and engineering process | approval rules, workflow, coding constraints, validation rules, and documentation routing | product feature specifications, task history, user instructions |
| `REQUIREMENTS.md` | normative product contract | approved observable behaviour, limits, security requirements, architecture constraints, compatibility contract, acceptance criteria | task status, investigation diaries, command output, unapproved ideas |
| `TASKS.md` | implementation plan and durable engineering record | task breakdown, dependencies, implementation notes, root-cause conclusions, blockers, measurements, and verification evidence | duplicated user guide, full product specification |
| `README.md` | current user-facing guide | implemented commands, keys, configuration, platform support, limits, acknowledgements, and links | future behaviour, incomplete designs, investigation details |

### When to update each file

| Situation | Required update |
| --- | --- |
| A new product behaviour is approved | Add or change the normative rule in `REQUIREMENTS.md`; add implementation work to `TASKS.md`. Do not update `README.md` yet. |
| An architecture constraint is approved | Record the durable constraint in `REQUIREMENTS.md`; record the implementation steps and dependencies in `TASKS.md`. |
| A task is proposed | Add it to `TASKS.md` with scope, dependencies, affected requirement sections, and an objective done condition. |
| A bug is investigated | Keep only the durable reproduction, root cause, rejected unsafe direction when still relevant, correction, and verification in the matching `TASKS.md` entry. Do not create a standalone analysis file. |
| Code is implemented but not fully verified | Update only `TASKS.md`; mark implementation complete separately from each required platform and state the blocker. |
| User-visible behaviour is implemented and verified | Update `README.md` to match the real behaviour, then record the completed implementation and validation in `TASKS.md`. |
| Build, performance, size, or compatibility evidence is produced | Store concise commands, results, environment, and blockers in `TASKS.md`. |
| AI workflow or approval policy changes | Update `AGENTS.md` only. |
| Acknowledgement or prior-art credit changes | Update the actual credit in `README.md`; keep only the normative credit requirement in `REQUIREMENTS.md`. |
| A discussion produces no approved decision | Do not change project documentation. |
| Temporary notes or exploratory analysis are needed | Keep them outside the committed project documentation and delete them after the durable conclusion is recorded. |

### Documentation update order

For an approved user-visible change:

1. Update `REQUIREMENTS.md` when the public contract changes.
2. Add or refine the task in `TASKS.md`.
3. Implement the smallest approved code change.
4. Run the approved focused checks.
5. Update `README.md` only after the behaviour is implemented and verified.
6. Record implementation and each required platform result separately in
   `TASKS.md`.

Documentation-only corrections that do not change behaviour may update the
relevant owning file directly.

### Conflict resolution

Use this order:

1. `REQUIREMENTS.md` for product behaviour and acceptance.
2. `AGENTS.md` for workflow and authorization.
3. `TASKS.md` for implementation scope and status.
4. `README.md` for current user instructions.

When `README.md` differs from implemented behaviour, correct `README.md`. When
implementation differs from `REQUIREMENTS.md`, treat the implementation as a bug
unless the requirement is explicitly changed and approved.

## Development Workflow

Before changing code:

1. Inspect the relevant files.
2. Identify the affected requirement and task sections.
3. Summarize the proposed change briefly.
4. Wait for an in-scope `APPROVE`.
5. Make the smallest practical change.
6. Run focused tests.
7. Run the approved build and verification.
8. Update user documentation only when verified behaviour changed.
9. Update the task implementation and platform checkboxes independently.
10. Commit only after the preceding checks pass.
11. Report only the result, risks, and remaining issues.

All Git commit messages MUST follow Conventional Commits.

Do not:

- Rewrite working code without a clear reason.
- Add frameworks for convenience.
- Add dependencies without approval.
- Generate large amounts of boilerplate.
- Hide warnings or test failures.
- Change public behaviour accidentally.
- Create a design or analysis Markdown file instead of updating the four owned
  documents.

## Luna Task Design

Implementation subtasks MAY include one advisory starting level:

```text
Luna Low
Luna Medium
Luna High
Luna xHigh
Luna Max
```

Choose the lowest level that can complete the bounded task safely:

- `Luna Low`: exact mechanical edits with no design choice, such as one-file
  wording changes, checkbox or evidence updates, and already-specified README
  publication after validation.
- `Luna Medium`: bounded documentation design, one-module implementation, or
  isolated tests with explicit state and expected output.
- `Luna High`: two or three modules with clear state transitions, input-to-action
  wiring, rendering changes, or focused integration tests where ownership and
  concurrency are already fixed.
- `Luna xHigh`: cross-module state, cooperative bounded work, cancellation,
  generation handling, cache/frame invariants, or cross-platform runtime
  behaviour within an already-approved architecture.
- `Luna Max`: architecture or ownership must be decided or changed, concurrent
  ordering or rollback spans major components, or a review-gated integration
  cannot be safely decomposed further.

Task length alone does not justify `Luna Max`. A task that uses an existing
worker, generation, cancellation, or queue design SHOULD use `Luna xHigh` when
its ownership and invariants are already fixed.

The model field is execution guidance only. It does not authorize broader scope,
combine tasks, change product behaviour, or replace an `APPROVE` gate.

## Task and Platform Completion

The following status format is mandatory for T28 and every later task.

T28 through T30 MAY retain their existing historical platform format. T31 and
every later task MUST separate where implementation differs from where the same
implementation must be validated:

```text
Implementation scope: Platform independent
Required validation: Linux and Native Windows
```

Accepted implementation scopes are:

```text
Platform independent
Linux
Native Windows
```

Accepted validation sets are:

```text
Platform independent
Linux
Native Windows
Linux and Native Windows
```

Use `Platform independent` implementation scope for shared product behaviour and
shared Rust logic. Do not create separate Linux and Native Windows feature tasks
merely because both platforms require validation. Use a Linux or Native Windows
implementation scope only for an unavoidable platform API, materially different
native lifecycle or security semantics, or intentionally platform-only behaviour.

Platform-independent implementation does not mean one-platform validation is
enough. Require Linux and Native Windows validation when filesystem, terminal,
input, rendering, threading, process, or platform-library runtime behaviour may
differ. Platform checkboxes are validation evidence, not separate product
features.

Each task MUST record implementation separately from required validation:

```text
Completion status:
- [x] Task implementation complete
- [x] Linux validation complete
- [ ] Native Windows validation complete
```

For a platform-independent task, use:

```text
Completion status:
- [x] Task implementation complete
- [x] Platform-independent validation complete
```

A documentation-only task MAY label the second item
`Platform-independent review complete`.

Rules:

- From T28 onward, the checkbox in the task heading records implementation only:
  `[x]` means the approved implementation or documentation change exists; `[ ]`
  means it does not. Do not use `[*]` for new tasks.
- Missing Windows validation MUST NOT erase completed implementation or completed
  Linux validation. Leave only `Native Windows validation complete` unchecked.
- Do not add a platform checkbox that the task does not require.
- A compile-only target check does not complete native runtime validation.
- `cargo check --target x86_64-pc-windows-msvc` is useful evidence but is not a
  native Windows test or MSVC release build.
- Native Windows completion requires the named focused tests and build to execute
  on native Windows with the MSVC toolchain. ConPTY, named-pipe, console-mode,
  ACL, and Windows path behaviour require native runtime acceptance when in scope.
- Linux completion requires the named Linux or WSL checks. Linux PTY, signals,
  permissions, musl linking, and Unix path behaviour require Linux or WSL runtime
  acceptance when in scope.
- Cross-platform verification may be claimed only when every required platform
  checkbox is complete.
- Evidence MUST name the environment, checks, result, and blocker. Do not infer a
  missing platform result from another platform.

## Platform Targets

Primary release targets:

```text
x86_64-unknown-linux-musl
x86_64-pc-windows-msvc
```

Possible future target:

```text
aarch64-unknown-linux-musl
```

The Linux release must run without external shared libraries. The Windows
release must not bundle runtime DLLs; Windows system DLLs are allowed.

## Development Environments

### Windows host with MSYS2 and WSL

- The editor or Codex App may run on Windows.
- Prefer MSYS2 Bash for Windows-side command-line work.
- Linux builds, tests, lint, PTY testing, and execution must run inside WSL.
- Native Windows backend builds and checks must run on Windows with MSVC.
- WSL is the authoritative Linux runtime environment.
- MSYS2 is a convenience shell, not the release runtime.
- Prefer storing the repository inside the WSL filesystem for Linux performance
  and permission behaviour.
- Avoid assumptions based on drive letters, CRLF, or Windows permissions.

### Pure Linux

- The editor, build tools, tests, and runtime run directly on Linux.
- The same Linux commands used in WSL should work unchanged.
- Do not introduce WSL-only application logic.

## Environment Rules

- Linux behaviour is authoritative for the Linux backend.
- Native Windows behaviour is authoritative for the Windows backend.
- Both environments must use the same stable Rust toolchain and locked
  dependencies.
- Use LF line endings.
- Keep scripts compatible with POSIX shell or Bash.
- Prefer commands that behave consistently in MSYS2 Bash and WSL Bash.
- Do not require PowerShell or `cmd.exe` for cross-platform project scripts.
- Do not store developer-specific absolute paths.
- Windows-only code may use standard Windows environment variables.
- PTY, signals, IPC, permissions, and terminal restoration must be tested on
  each supported native platform.

## Build Requirements

Use stable Rust.

Linux release:

```bash
cargo build --release --locked --target x86_64-unknown-linux-musl
```

Native Windows release:

```text
cargo build --release --locked --target x86_64-pc-windows-msvc
```

The release profile should favour size:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Avoid build-time or runtime dependence on:

- OpenSSL
- systemd
- PAM
- glibc-only APIs
- dynamically linked C libraries
- external shell commands for core functions

Pure Rust dependencies are preferred.

## Dependency Policy

Every new dependency must be justified in `TASKS.md` with its purpose, licence,
binary-size effect, portability, and why the standard library or an existing
dependency is insufficient.

Prefer:

- Rust standard library
- Small, focused, actively maintained crates
- musl-compatible crates
- Rust implementations over C bindings

Avoid:

- Async runtimes unless clearly necessary
- GUI libraries
- Plugin runtimes
- Embedded web servers
- General-purpose frameworks
- Duplicate crates with overlapping responsibilities

Before approving a dependency, inspect:

```bash
cargo tree
cargo tree --duplicates
cargo bloat --release --target x86_64-unknown-linux-musl
```

## Architecture and Code Rules

Prefer:

- A small number of modules with clear ownership
- Explicit state
- Bounded channels and caches
- Deterministic cleanup
- Small public APIs
- Direct error propagation
- Standard-library solutions where practical

Avoid:

- Global mutable state
- Unbounded queues
- Hidden background threads
- Excessive traits or deep generics
- Complex macro systems
- Platform abstractions that hide materially different lifecycle or security
  semantics

Do not create abstractions before they are needed. Do not combine Linux PTY and
Windows ConPTY code merely to reduce line count.

Any `unsafe` block must include a short safety comment explaining its invariant.

## Security and Error Handling

Follow the security and resource contracts in `REQUIREMENTS.md`.

Additionally:

- Treat terminal input and file content as untrusted bytes.
- Do not log terminal contents or environment secrets by default.
- Do not use shell command interpolation for internal operations.
- Do not panic for normal errors.
- User-facing errors must be short, specific, and actionable.
- `unwrap()` and `expect()` are acceptable only for proven invariants or tests.

## Testing

Prefer unit tests for state and parsing logic, and integration tests for PTY,
IPC, lifecycle, and terminal restoration.

Every bug fix should include a regression test when practical. Every
non-trivial task in `TASKS.md` must name focused checks and an objective done
condition before implementation.

Never claim cross-platform completion from compile-only validation. Record
unavailable environments as unchecked platform completion plus a blocker in
`TASKS.md`.

## Release Checklist

Run only with explicit approval covering the commands:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo audit
cargo deny check
cargo build --release --locked --target x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-pc-windows-msvc
file target/x86_64-unknown-linux-musl/release/termfold
ldd target/x86_64-unknown-linux-musl/release/termfold
```

Also confirm:

- Binary size remains within the approved budget.
- Linux is statically linked.
- Windows has no bundled runtime DLLs.
- No runtime network access is required.
- Terminal state restores correctly.
- Detach and reattach work over SSH.
- Required native Windows terminal acceptance passes.
- Source and dependency versions are locked.
- A SHA-256 checksum is generated.

## Decision Priority

When requirements conflict, use this order:

1. Correctness
2. Terminal restoration and session safety
3. Security
4. Portability
5. Low memory usage
6. Small binary size
7. Convenience
8. Additional features
