# Axiom AI — M.10 Persistent Memory

> Base architectural roadmap for integrating persistent memory into the Axiom Agent Runtime.

---

## Purpose

Persistent Memory exists so the Axiom Agent can preserve useful technical knowledge across sessions without turning historical memory into the source of truth.

Examples:

```text
this project uses Pest
PHPStan must run at level 8
previous attempts showed that X causes Y
this module has a known architectural constraint
this approach failed before
```

Memory is historical knowledge derived from experience.

It is not the same thing as Instructions, Trace, Skills, or current deterministic project state.

> **Memory helps the Axiom Agent remember. It does not decide what is true, and it does not autonomously decide what becomes a Skill.**

---

# M.10.1 — Core Principles

The memory architecture must obey these principles:

```text
local-first by default
human-readable where possible
open/exportable representation
no model-provider lock-in
no Agent Runtime lock-in
backend replaceable
database indexes rebuildable
memory survives provider changes
memory survives runtime changes
failure must not block the IDE
no memory work in the editor typing hot path
```

The initial implementation may use `ai-memory`, but Axiom Persistent Memory is **not** defined by `ai-memory`.

```text
Axiom Persistent Memory
        =
MemoryService contract

ai-memory
        =
first backend implementation
```

Possible future backends:

```text
NoMemory
AiMemoryBackend
AxiomMemoryBackend
RemoteTeamMemoryBackend
EnterpriseMemoryBackend
```

---

# M.10.2 — Source of Truth and Portability

The user's accumulated memory must not be trapped inside an opaque Axiom database or a model provider.

Preferred model:

```text
Human-readable durable knowledge
            =
      source of truth

SQLite / FTS / embeddings / graph indexes
            =
       derived indexes
```

Whenever practical, consolidated knowledge should exist in an open and inspectable representation such as Markdown/OKF-compatible files.

This enables:

```text
backup
versioning
inspection
manual editing
migration
import/export
backend replacement
future synchronization
```

A user must not lose years of useful memory simply because Axiom changes its internal backend.

---

# M.10.3 — Memory Is Not Instructions, Trace, or Skills

## Memory

Historical knowledge learned from previous work.

```text
Previous formatting bugs were caused by stale document revisions.
```

Memory is useful context, but it is not automatically authoritative.

## Instructions

Explicit rules declared by the user, project, workspace, or organization.

```text
Controllers must not access Repository directly.
```

Instructions have greater authority than inferred memory.

## Trace

Observable record of a concrete execution.

```text
read_file
find_references
edit_file
run_tests
184/184 PASS
```

Trace is evidence, not consolidated knowledge.

## Skills

Reusable procedural knowledge.

```text
How to investigate and repair failing PHPUnit/Pest tests in this project.
```

Memory may identify a candidate procedure, but it must never silently rewrite an active Skill.

---

# M.10.4 — Memory Authority

Conceptual authority order:

```text
Current deterministic project state
            >
Explicit user/project instructions
            >
Accepted decisions
            >
Consolidated historical memory
            >
Session observations
            >
Model inference
```

Example:

```text
Memory:
PHP version is 8.4

Current composer.json:
PHP 8.5
```

Result:

```text
PHP 8.5 wins.
```

Historical memory can be outdated, incomplete, or contradicted.

---

# M.10.5 — MemoryService Abstraction

Create an Axiom-owned abstraction:

```text
MemoryService
```

The `AgentRuntime` must never depend directly on `ai-memory`.

Conceptually:

```rust
trait MemoryService {
    async fn briefing(...);
    async fn query(...);
    async fn session_start(...);
    async fn observe(...);
    async fn session_end(...);
    async fn handoff(...);
    async fn recent(...);
    async fn history(...);
}
```

The real interface must be designed around **Axiom Agent Runtime needs**, not mirror the external backend API.

Dependency:

```text
AgentRuntime
    │
    ▼
MemoryService
```

Never:

```text
AgentRuntime
    │
    ▼
ai-memory-specific API
```

---

# M.10.6 — NoMemory First

Before integrating any external backend, implement:

```text
NoMemory
```

The Agent Runtime must operate normally when memory is:

```text
disabled
unavailable
unsupported
misconfigured
```

```text
memory unavailable
       ↓
Agent continues
       ↓
without persistent memory
```

Never:

```text
memory unavailable
       ↓
Agent fails entirely
```

---

# M.10.7 — ai-memory Integration Strategy

The first real backend will be:

```text
AiMemoryBackend
```

Architecture:

```text
Axiom Agent Runtime
        │
        ▼
   MemoryService
        │
        ▼
  AiMemoryBackend
        │
        ▼
 protocol adapter
        │
   ┌────┴────┐
   │         │
   ▼         ▼
 MCP /     HTTP
 hooks      /api/v1
   │         │
   └────┬────┘
        ▼
   ai-memory
```

Rules:

- The rest of Axiom must not know whether an operation uses MCP, HTTP, hooks, or another backend-specific surface.
- `AiMemoryBackend` owns the translation.
- `/api/v1` should be treated primarily as a read/query surface.
- Mutations must use supported public mutation/lifecycle surfaces.
- Axiom must never write directly to `ai-memory`'s SQLite database.

---

# M.10.8 — Do Not Embed ai-memory Crates Initially

Initial boundary:

```text
Axiom
   ↓
protocol boundary
   ↓
ai-memory process
```

Not:

```text
Axiom workspace
   ├── axiom-app
   ├── axiom-agent
   └── ai-memory internal crates
```

Reasons:

```text
lower coupling
independent upgrades
separate Rust toolchains
less dependency pressure
easier experimentation
backend replacement remains possible
backend internals do not leak into Agent Runtime
```

Only reconsider embedding selected crates after substantial real-world experience.

---

# M.10.9 — Docker Is Not Part of the Axiom Integration

Axiom's integration must not depend on Docker.

```text
Axiom
   ↓
native ai-memory process
   ↓
local persistent data directory
```

Docker may remain useful upstream for development or deployment scenarios, but it is not part of Axiom's runtime architecture.

```text
Docker required = false
```

The intended Windows integration is a native executable managed by Axiom.

---

# M.10.10 — Native Process Manager

Create:

```text
MemoryManager
```

Conceptually:

```text
Axiom starts
    │
    ▼
MemoryManager
    │
    ├── locate configured backend
    ├── detect running instance
    ├── start backend asynchronously
    ├── health check
    ├── establish connection
    └── report status
```

Possible states:

```text
Memory: Disabled
Memory: Starting
Memory: Ready
Memory: Unavailable
```

Never:

```text
IDE startup
   │
   X
   │
wait synchronously for memory
```

Memory startup must never block editor startup.

---

# M.10.11 — Native Windows Policy

Because Axiom targets native Windows, the first integration should be conservative.

Initial policy:

```text
Memory backend: Experimental
Default: Disabled until explicitly enabled during development
```

Validate:

```text
startup
shutdown
restart
SQLite persistence
Unicode paths
long paths
process crashes
project switching
binary upgrades
data migration
model downloads
```

Do not require Windows Service installation initially.

Simplest first lifecycle:

```text
Axiom
  └── child ai-memory process
```

---

# M.10.12 — Axiom-Owned Data Directory

Prefer:

```text
%LOCALAPPDATA%\Axiom\ai-memory\
```

Example:

```text
%LOCALAPPDATA%\Axiom\ai-memory\
├── wiki/
├── raw/
├── db/
├── models/
├── logs/
└── config/
```

Ownership:

```text
Axiom
    owns process lifecycle and configuration location

ai-memory
    owns memory contents and storage invariants
```

Axiom must not manipulate backend database files directly.

---

# M.10.13 — Project Identity and Memory Scope

Project identity must be determined by Axiom, not guessed solely from the filesystem path.

The same logical project may exist as:

```text
C:\dev\Axiom
E:\dev\Axiom
/home/user/Axiom
worktree-a
worktree-b
another machine
```

Initial scopes:

```text
User
Workspace
Project
```

Future:

```text
Team
Organization
```

Architecture:

```text
Axiom ProjectIdentity
        │
        ▼
    MemoryScope
        │
        ▼
 AiMemoryBackend
        │
        ▼
backend-specific workspace/project routing
```

Logical project identity must be separate from checkout/path identity.

---

# M.10.14 — What Gets Stored

Only semantically meaningful Agent Runtime events should feed persistent memory.

```text
user task
important tool results
important discoveries
explicit decisions
mutations
test results
validation results
task outcome
handoff information
repeated failures
repeated successful procedures
```

Example:

```text
Task:
Fix failing UserService tests

Discovery:
Project uses Pest through composer test.

Changes:
UserService.php

Validation:
184/184 tests passed.

Important decision:
Use composer test instead of invoking vendor/bin/phpunit directly.
```

---

# M.10.15 — What Must Never Enter the Memory Hot Path

Never automatically send:

```text
keypress
cursor movement
completion invocation
hover
every diagnostics refresh
every syntax parse
every render
every document revision
every semantic refresh
streaming token
```

Never perform during typing:

```text
HTTP calls
MCP calls
embedding generation
SQLite writes
memory lookup
memory consolidation
filesystem scans
directory walks
```

Rule:

```text
Editor hot path
      │
      X
      │
Persistent Memory
```

Memory observes the **Agent Runtime**, not the keyboard.

---

# M.10.16 — Performance Guardrails

Forbidden:

```text
blocking UI thread
SQLite writes on UI thread
embedding generation on UI thread
memory retrieval during typing
filesystem canonicalization per keystroke
full-project scans for memory
project/vendor scans
unbounded queues
unbounded session payloads
blocking HTTP on UI thread
```

Operations must be:

```text
background
bounded
cancelable
timeout-aware
gracefully degradable
```

Do not introduce a local Tokio runtime merely for memory if Axiom's existing async/background primitives are sufficient.

---

# M.10.17 — Sensitive Data Sanitization

Never automatically persist:

```text
API keys
passwords
access tokens
private keys
authentication headers
known secret patterns
raw environment secrets
```

Future tool results should expose sensitivity metadata:

```text
ToolResult
├── content
└── sensitivity
```

Only approved content reaches `MemoryService`.

---

# M.10.18 — Trace Remains Axiom-Owned

```text
Agent Runtime
      │
      ▼
Axiom Trace Store
      │
      ├───────────────┐
      ▼               ▼
Evaluator         MemoryService
                      │
                      ▼
                 ai-memory
```

Example Trace:

```text
Tool call #31
find_references
17 ms
```

Possible Memory:

```text
Resident semantic reference lookup is preferred
over project-wide textual search.
```

Do not turn raw telemetry into consolidated memory by default.

---

# M.10.19 — Session Capture

```text
Agent Task starts
      │
      ▼
memory.session_start()
      │
      ▼
Agent execution
      │
      ├── important observations
      ├── decisions
      ├── mutations
      ├── validation
      └── outcome
      │
      ▼
memory.session_end()
```

A session should preserve enough information to answer:

```text
What were we trying to do?
What did we discover?
What changed?
What worked?
What failed?
What remains?
What decisions were made?
```

---

# M.10.20 — Memory Briefing

```text
User Task
   │
   ▼
AgentRuntime
   │
   ▼
MemoryService.briefing()
   │
   ▼
relevant project knowledge
   │
   ▼
ContextEngine
   │
   ▼
Model
```

Example:

```text
Memory briefing:
- Tests use Pest.
- composer test is the canonical command.
- PHPStan level 8 must pass before completion.
- Previous failures in this module involved stale DTO assumptions.
```

Memory briefing is auxiliary context, not a substitute for checking the current repository.

---

# M.10.21 — Memory Query

Example:

```text
memory.query(
    "Previous decisions involving UserService"
)
```

The backend may internally combine:

```text
full-text search
entity matching
semantic embeddings
graph relations
temporal relevance
authority/confidence
project scope
```

The Agent Runtime receives normalized Axiom-owned results:

```text
MemoryResult[]
```

Useful metadata:

```text
source
timestamp
scope
confidence
session
relations
backend
```

---

# M.10.22 — Read-Only Integration First

The first live `AiMemoryBackend` should start with retrieval:

```text
query
briefing
recent
history
```

Goals:

```text
prove process lifecycle
prove connectivity
prove project scoping
prove retrieval
prove failure isolation
prove backend substitution boundary
```

Do not enable automatic memory writes in the first live integration.

---

# M.10.23 — Explicit Writes Second

Then allow explicit writes:

```text
remember this
create handoff
save this decision
record this gotcha
```

Rules:

```text
use public backend mutation surfaces
never write SQLite directly
sanitize before write
scope explicitly
record provenance
```

---

# M.10.24 — Automatic Runtime Observation Third

Only after explicit writes are stable:

```text
important discovery
accepted decision
meaningful tool result
mutation summary
test result
validation result
task outcome
repeated failure
repeated successful procedure
```

Do not send:

```text
every tool output
full terminal dumps
entire source files by default
every diagnostic
every model token
```

Capture must be selective and bounded.

---

# M.10.25 — Consolidation

```text
raw observations
      ↓
session
      ↓
consolidation
      ↓
useful durable knowledge
```

Consolidation should identify:

```text
important decisions
stable project facts
repeated gotchas
successful procedures
failed approaches
unresolved questions
```

---

# M.10.26 — Local First

Default:

```text
Memory storage        Local
Embeddings            Local
Cloud sync            Off
Team sync             Off
Remote memory         Off
```

Persistent memory must not require a cloud account.

---

# M.10.27 — Retrieval Before Embeddings

Initial retrieval:

```text
FTS
symbolic matching
metadata filtering
project scope
recency
```

Later:

```text
semantic embeddings
entity relationships
temporal graph
```

Fallback:

```text
embeddings unavailable
→ FTS continues
```

---

# M.10.28 — Local Embeddings

When semantic retrieval is enabled:

```text
Embeddings
    ↓
local by default
```

Do not automatically send project knowledge to an external embedding service.

---

# M.10.29 — Handoff

Persistent memory should support continuity between:

```text
sessions
models
providers
Agent Runtimes
eventually machines
```

A handoff should contain:

```text
Goal
Work completed
Files changed
Important discoveries
Current failures
Open questions
Recommended next actions
```

Do not copy the entire previous context window.

---

# M.10.30 — Temporal Memory

Longer term, support questions such as:

```text
When did this decision appear?
When did this rule change?
What context existed before this regression?
What procedure was used previously?
```

Useful relations:

```text
causes
fixes
contradicts
supersedes
related_to
```

Advanced capability; not required for the initial milestone.

---

# M.10.31 — Experience Analysis

Repeated sessions may reveal patterns:

```text
Session 12
Forgot composer script.

Session 31
Forgot composer script.

Session 57
Forgot composer script.
```

Potential experience:

```text
Before running PHPUnit directly,
inspect composer scripts because this project
wraps Pest through composer test.
```

This can become `Procedure Memory`, but not automatically a Skill.

---

# M.10.32 — Memory → Skill Boundary

```text
Memory
   │
   ▼
Experience
   │
   ▼
Procedure Candidate
   │
   ▼
Skill Candidate
   │
   ▼
Validation
   │
   ▼
Human Review
   │
   ▼
SKILL.md
```

Never:

```text
Memory detects pattern
      ↓
rewrites active SKILL.md
```

---

# M.10.33 — Human Review

Favor human review for:

```text
project rules
procedures
architecture decisions
shared team knowledge
knowledge contradicting previous accepted knowledge
```

---

# M.10.34 — Memory UI

Only build full UI after real backend data exists.

Initial:

```text
AI
├── Chat
├── Agent
├── Tasks
└── Memory
```

Memory:

```text
Overview
Sessions
Decisions
Gotchas
Procedures
Timeline
```

Later:

```text
Entities
Relationships
History
Proposals
```

---

# M.10.35 — Memory Editing

Eventually support:

```text
Open
Inspect
Edit
Pin
Archive
Forget
View history
View evidence
```

Memory should be:

```text
visible
inspectable
correctable
portable
```

---

# M.10.36 — Forget and Retention

Examples:

```text
Forget this memory.
Forget memories from this session.
Forget memories related to this project.
Do not retain terminal output matching secrets.
```

Possible retention:

```text
raw observations       short-lived
sessions               medium/long
accepted decisions     long-lived
procedures              long-lived
temporary failures     decayable
```

---

# M.10.37 — Failure Isolation

Every memory call must support:

```text
timeout
cancellation
bounded work
graceful fallback
```

```text
memory timeout/error
       ↓
Agent continues
```

Never freeze or fail the IDE because memory is unavailable.

---

# M.10.38 — Initial Implementation Sequence

Do **not** implement M.10 mechanically from top to bottom.

## M.10-A — Memory Contract

Implement only:

```text
MemoryService
NoMemory
MemoryScope
MemoryResult
MemoryObservation
MemorySessionId
```

Tests:

```text
AgentRuntime works with NoMemory
backend absence changes no Agent logic
```

## M.10-B — Fake Memory Backend

Create a deterministic in-memory fake backend.

Prove:

```text
session_start
observe
session_end
query
briefing
handoff
```

without starting `ai-memory`.

## M.10-C — MemoryManager

Implement:

```text
locate ai-memory binary
configured data dir
start asynchronously
health check
Ready / Unavailable
shutdown policy
```

Rules:

```text
native Windows
no Docker
no UI blocking
no local Tokio runtime solely for memory
```

## M.10-D — AiMemoryBackend Read-Only

Implement:

```text
query
briefing
recent
history
```

Through supported public read surfaces.

Acceptance:

```text
project-scoped retrieval works
failure degrades gracefully
Agent Runtime remains backend-agnostic
```

## M.10-E — Explicit Memory Writes

Implement explicit:

```text
remember
decision
gotcha
handoff
```

Use supported mutation surfaces.

Never direct SQLite writes.

Sanitize before persistence.

## M.10-F — Session Lifecycle

Connect:

```text
Agent Task starts
→ session_start

important events
→ observe

Agent Task ends
→ session_end
```

No typing/render events.

## M.10-G — Memory Briefing

```text
task
→ MemoryService.briefing()
→ bounded relevant knowledge
→ ContextEngine
```

Current deterministic state must still win.

## M.10-H — Memory Query Tool

Expose a small Axiom-owned Agent tool surface:

```text
memory_search
memory_get
```

Do not expose dozens of backend-specific tools directly to the model.

## M.10-I — Basic Memory UI

Add:

```text
Overview
Sessions
Decisions
Gotchas
```

UI must use Axiom view models / `MemoryService`, not backend database assumptions.

## M.10-J — Consolidation

Enable/validate:

```text
raw observations
→ sessions
→ consolidated knowledge
```

only after capture quality is proven.

## M.10-K — Local Semantic Retrieval

Enable local embeddings only after FTS/basic retrieval is stable.

Fallback:

```text
embeddings unavailable
→ FTS continues
```

## M.10-L — Advanced Memory

Later:

```text
temporal graph
advanced authority
experience analysis
procedure candidates
team memory
cloud synchronization
organization scope
```

Not part of the first milestone.

---

# M.10.39 — Initial Milestone Scope

The first useful milestone should deliver only:

```text
MemoryService abstraction
NoMemory
Fake backend
MemoryManager
AiMemoryBackend
native local process
no Docker
session start/end
important observations
memory briefing
memory query
basic handoff
project/workspace scopes
basic Memory UI
local-first configuration
failure isolation
sanitization
```

Leave for later:

```text
temporal graph
advanced authority scoring
experience analysis
automatic skill candidates
team memory
cloud synchronization
organization scopes
advanced entity graph UI
```

---

# M.10.40 — Definition of Done

The initial milestone is complete only when:

```text
1. Agent Runtime works normally with NoMemory.

2. ai-memory can be enabled without changing Agent Runtime logic.

3. Axiom can start/use the local native backend without Docker.

4. Memory backend startup never blocks IDE startup.

5. New Agent sessions can retrieve useful knowledge from previous sessions.

6. Memory is scoped correctly per project/workspace.

7. Project identity does not depend only on a filesystem path.

8. Model/provider can change without losing memory.

9. Memory backend can fail without freezing or breaking the IDE.

10. No memory operation occurs in the typing/render/completion hot path.

11. Sensitive information is filtered before persistence.

12. User can inspect the main memories stored for a project.

13. Memory cannot modify active Skills automatically.

14. Historical memory never overrides current deterministic project state.

15. Stored durable knowledge can be exported or inspected in an open,
    human-readable representation.

16. Axiom never writes directly into ai-memory's SQLite internals.

17. Agent Runtime contains no ai-memory-specific protocol logic.

18. Backend-specific HTTP/MCP/hook details stay inside AiMemoryBackend.

19. Retrieval still works in reduced mode when embeddings are unavailable.

20. Memory operations are bounded, timeout-aware, cancelable where applicable,
    and gracefully degradable.
```

---

# M.10.41 — Resulting Architecture

```text
                         Axiom Agent Runtime
                                │
              ┌─────────────────┼─────────────────┐
              │                 │                 │
              ▼                 ▼                 ▼
           Context             Trace             Memory
              │                 │                 │
              │          Axiom Trace Store   MemoryService
              │                                   │
              │                      ┌────────────┴────────────┐
              │                      │                         │
              │                      ▼                         ▼
              │                  NoMemory               AiMemoryBackend
              │                                                │
              │                                         protocol adapter
              │                                                │
              │                                        ┌───────┴───────┐
              │                                        ▼               ▼
              │                                      MCP /           HTTP
              │                                      hooks           read API
              │                                        │               │
              │                                        └───────┬───────┘
              │                                                ▼
              │                                           ai-memory
              │                                                │
              │                             ┌──────────────────┼─────────────────┐
              │                             ▼                  ▼                 ▼
              │                           Wiki               SQLite             Raw
              │                      human-readable      derived index       observations
              │
              └──────── Current deterministic Axiom intelligence
```

Future:

```text
MemoryService
   ├── NoMemory
   ├── AiMemoryBackend
   ├── AxiomMemoryBackend
   ├── RemoteTeamMemoryBackend
   └── EnterpriseMemoryBackend
```

---

# M.10.42 — Relationship to the Wider Axiom AI Roadmap

Keep:

```text
M.0  Architecture
  ↓
M.1  Providers
  ↓
M.2  Chat
  ↓
M.3  Context
  ↓
M.4  Read-only Tools
  ↓
M.5  Agent Runtime
  ↓
M.6  Permissions
  ↓
M.7  Mutating Tools
  ↓
M.8  Agent Tasks/UI
  ↓
M.9  Trace/Evals
  ↓
M.10 Persistent Memory
```

Do not move ai-memory earlier merely because Chat exists.

Persistent Memory becomes valuable when the Agent Runtime is producing structured experience worth remembering.

After M.10:

```text
M.11 Skills
  ↓
M.12 Closed Learning Loop
```

---

# M.10.43 — Relationship to MCP

Do not confuse:

## Internal backend protocol

During M.10, `AiMemoryBackend` may internally use MCP as one of the ai-memory protocol surfaces.

## Generic Axiom MCP Platform

A later phase remains responsible for:

```text
generic external MCP clients
Axiom Tool Registry integration
permission mediation
Axiom as an MCP server
third-party MCP servers
```

Therefore:

```text
M.10:
AiMemoryBackend may speak MCP internally.

M.14:
Axiom becomes a general MCP platform.
```

---

# M.10.44 — Implementation Rule for Coding Agents

For every microphase:

```text
audit current source
        ↓
identify smallest missing contract
        ↓
implement one microphase
        ↓
run focused tests
        ↓
run broader validation
        ↓
manual validation when process/UI behavior is involved
        ↓
only then advance
```

Every coding prompt touching memory must repeat:

```text
no heavy UI-thread work
zero new document.content() per keystroke
no filesystem work in editor typing/completion/render paths
no full parse per keystroke
no project/vendor scans in hot paths
no blocking HTTP on UI thread
no blocking process waits on UI thread
no memory lookup during typing
no memory writes during typing
no embedding generation during typing
bounded background work
cooperative cancellation
failure isolation
```

---

# Final Principle

```text
Axiom
    understands the current codebase deterministically

Agent Runtime
    executes work

Trace
    records what happened

Evaluator
    measures what happened

Memory
    preserves useful experience

Skills
    encode reusable procedures

Human
    governs permanent procedural evolution
```

Persistent Memory exists to let the Axiom Agent **stop starting from zero**.

It must never turn historical LLM output into unquestioned truth, never become part of the editor hot path, and never make `ai-memory` an inseparable dependency of the IDE.
