# JEV small model reactions

## Repository architecture and current boundaries

### Takeaway

VPet already has the right split for a bounded reaction planner: Rust Core owns the state transition, while Brain/Body consume the result. A small model should therefore be an **advisory proposer** that emits a narrow, typed reaction intent; it must never receive a mutable `Pet` or write an `Event` directly.

### Cited Findings

- `reduce` is documented as a pure function: it does not read the clock or perform I/O; time arrives as `Event::Tick { minutes, hour }` (`apps/desktop/src-tauri/src/core/state_machine.rs:12-15`).
- The Core component map explicitly states “Core decides everything, Body only performs, Brain only speaks”; `state_machine`, `actions`, `obey`, and `intent` are Rust modules (`docs/02-components.md:1-31`).
- The event algebra is deliberately small and explicit. `Event` includes `Tick`, `Touched`, `Pin`, `Gifted`, `Medicated`, `Request`, `SetBias`, `ClearBias`, `Music`, and `Patch` (`state_machine.rs:225-255`). The model must not invent a new event shape at runtime.
- `Event::Request` carries a target, externally supplied roll, and optional duration; the comment says it is not guaranteed to execute and must pass `obey::judge` (`state_machine.rs:239-241`).
- `request_action` is the only user-intent entry point. The Tauri command samples the roll outside the pure reducer, then applies `Event::Request`; `request_action_quiet` is the same path for chat (`lib.rs:1015-1032`).
- `apply`/`apply_with` are the imperative boundary around `reduce`; they update persistence and emit state/events, while the reducer remains pure (`lib.rs:233-268`).
- The deterministic planner is `decide_with_pin`: physiological emergencies, night sleep, scheduled food, directive/pomodoro holds, work/study/play lanes, numeric needs, and fallback are ordered in Rust (`state_machine.rs:777-900`; summarized in `docs/03-core.md:20-47`).
- `should_switch` protects transient actions, physiological priorities, bedridden behavior, music, directive/pin holds, hysteresis, cooldown, and schedule boundaries (`state_machine.rs:700-775`). A model recommendation must not bypass this arbitration.
- Action legality and effects are data-driven in the embedded `actions.toml`: IDs, activities, tags, hours, duration, cooldown, per-minute effects, earns, requirements, and lines (`core/actions.rs:1-112`). This is the authoritative action vocabulary.
- `obey::judge` is deterministic given action definition, state, pressure, and the supplied roll. It checks prerequisites and bedridden status, calculates conflict, probability, refusal reason, and fixed refusal/acceptance lines (`core/obey.rs:1-35`, `core/obey.rs:130-225`).
- The current natural-language route is intentionally conservative and rule-based. `intent::parse` tries music, timing, focus, timer, bias, user-study-start, and do parsers in a fixed order (`core/intent.rs:88-96`), with tests at `intent.rs:494-626`.
- Chat executes deterministic intent handling before model generation. `handle_intent` calls `request_action_quiet`; only after that does it build “situation” and ask the model to phrase the result (`chat.rs:886-905`, `chat.rs:1003-1045`).
- The system prompt explicitly tells the model that whether she agrees is determined by her body, not by presentation metadata (`chat.rs:1461-1475`). This is an important existing invariant to preserve.
- The model call is local Ollama `/api/chat`, streamed, `think:false`, with response/wire/line caps (`chat.rs:1165-1208`). The endpoint validator accepts only loopback HTTP with no credentials, path, query, or fragment (`chat.rs:328-349`); tests cover remote hosts, paths, credentials, query, and fragments (`chat.rs:1236-1242`).
- Tool infrastructure already models a safe capability boundary: JSON-Schema input, permission levels, user/proactive origin, `Allow`/`Ask`/`Deny`, and audit records (`core/tools.rs:1-16`, `tools.rs:40-95`, `tools.rs:125-170`). It is a useful pattern, but a reaction proposer should be narrower than general tools.
- Existing tests cover the reducer extensively, including requests, directives, ticks, hysteresis, music, health, medicine, and deterministic outcomes (`state_machine.rs:1050-1660`, `state_machine.rs:2024-2095`, plus later tests); chat tests cover local-only routing, streaming, cleanup, speech tags, history, and the “body decides” prompt assertion (`chat.rs:1236-1495`).
- The roadmap intentionally leaves “AgentLoop/model tool use” and “intent second layer (nearest-neighbor/embedding)” unfinished because the local 9B is unreliable and high-frequency intent is currently handled by rules (`docs/06-roadmap.md:39-45`).
- User-visible behavior promises that chat commands enter the state machine and that model output cannot itself change state (`README.md:71-85`; `docs/02-components.md:44-61`; `docs/05-brain.md:1-31`).

### Inferences

- The safest extension point is between **raw user text and `intent::parse` fallback**, not inside `state_machine::reduce`, `decide`, or `obey`.
- The planner should receive a compact immutable observation (for example: current action/tag, mood, urgency flags, allowed action IDs/tags, and recent event facts), not the full mutable persistence object or arbitrary memory.
- The planner should propose a semantic “reaction” such as `acknowledge`, `offer_break`, `suggest_water`, `start_dialogue`, or `request_action(target, duration)`, but only a deterministic Rust adapter may convert a proposal into an existing `Intent` or reject it.

## Recommended bounded reaction-planner design

### Takeaway

Use the small model only as a **second-pass proposer for ambiguous, non-authoritative reactions** after rules fail. Put schema parsing, allowlisting, freshness checks, rate limits, and conversion to existing `Intent` values in Rust; execute only through existing handlers and `request_action_quiet`. Keep autonomous state selection entirely in `decide_with_pin`/`reduce`.

### Cited Findings

- Ollama’s official structured-output documentation says the `/api/chat` `format` field can be `json` or a JSON Schema, and recommends validating the returned JSON against the schema; it also recommends low temperature for reliable structured output ([Ollama Structured Outputs](https://docs.ollama.com/capabilities/structured-outputs)).
- Ollama’s API documentation distinguishes local inference at `http://localhost:11434/api` from cloud access and says local calls use a downloaded model without an authorization header ([Ollama API introduction](https://docs.ollama.com/api/introduction.md)).
- JSON Schema’s official reference describes schemas as a way to constrain and annotate JSON data, supporting explicit properties, types, required fields, and validation ([JSON Schema reference](https://json-schema.org/understanding-json-schema/reference)).
- OWASP’s LLM Top 10 project is an authoritative security reference for risks from LLM applications, including prompt injection and excessive agency; those risks are directly relevant if a reaction model is allowed to produce executable commands ([OWASP LLM Top 10](https://owasp.org/www-project-top-10-for-large-language-model-applications/)).
- NIST describes the AI RMF as a voluntary framework for incorporating trustworthiness and managing AI risks, and provides a GenAI profile for risks specific to generative systems ([NIST AI RMF](https://www.nist.gov/itl/ai-risk-management-framework)).

### Proposed exact insertion point

1. **Keep the current fast path unchanged**: `send_chat_message` → `intent::parse` → `handle_intent` → existing Core route. This remains the authoritative path for explicit commands (`chat.rs:886-905`).
2. **Add a Rust-owned `reaction_planner` adapter in the Brain boundary**, conceptually beside `intent.rs`/`chat.rs`, invoked only when:
   - no deterministic command was recognized;
   - the trigger is a bounded event (for example a user message, a completed action, a threshold crossing already detected by Core, or a nudge opportunity);
   - the per-trigger and per-day budget permits a call.
3. The adapter sends a **small, fixed observation** to a small local model through the already validated loopback Ollama client. Do not send the whole chat transcript, hidden mutable state, filesystem data, or tool registry.
4. The adapter requests exactly one JSON object using Ollama `format` with a schema. Suggested shape:

   ```json
   {
     "kind": "none | say | request_action | suggest",
     "target": "play | rest | work | study | eat | drink | sleep | null",
     "minutes": "number or null",
     "line_key": "fixed key or null",
     "confidence": "number 0..1",
     "reason": "short bounded label"
   }
   ```

   Prefer an enum/key (`line_key`) over free-form text. If free text is retained, it is presentation-only and must never be interpreted as a command.
5. **Validate in Rust before any effect**:
   - deserialize with `serde`;
   - reject unknown enum values, missing required fields, extra fields if practical, NaN/infinite/out-of-range minutes, overlong strings, and confidence outside `0..=1`;
   - allowlist targets against `Catalog` IDs/tags and the explicit non-suppressible policy;
   - attach a request/event timestamp or sequence number and reject stale responses;
   - reject proposals if the observed state snapshot no longer matches the triggering precondition;
   - enforce cooldown, duplicate suppression, daily budget, and cancellation.
6. **Convert only accepted proposals into existing deterministic forms**:
   - `request_action(target, minutes)` for a model-proposed user-facing request. This still samples the roll outside the reducer and runs `obey::judge`.
   - `Intent::Bias`, `Focus`, `Timer`, or `Music` only if a deterministic classifier/adapter explicitly permits that type. Do not let the model directly choose arbitrary tool names or filesystem operations.
   - `say`/`suggest` may emit a fixed `pet:line` key or pass a bounded line to the presentation layer; it must not mutate `Pet`.
7. **Apply through existing `handle_intent`/`apply` plumbing**. The only state-changing call remains an existing Tauri/Core command or an existing `Event`; no model callback calls `reduce` with custom data.
8. If parsing, validation, timeout, model availability, or freshness fails, return `none` and let the deterministic planner continue. Model failure must never block the heartbeat or alter state.

### Deterministic validation that must remain

- State transitions, all numeric updates, action adoption, cooldowns, directives, pins, hysteresis, health, and activity switching: `reduce`, `should_switch`, `decide_with_pin` (`state_machine.rs:388-500`, `700-900`).
- Action existence, requirements, schedule, duration, and effects: `Catalog`/`ActionDef` (`actions.rs:40-112`) and `actions.toml`.
- User request legitimacy and refusal: `request_action`/`request_action_quiet` plus `obey::judge` (`lib.rs:1015-1032`; `obey.rs:130-225`).
- Safety-sensitive direct commands: current rule parser and explicit command ordering (`intent.rs:88-96`, `intent.rs:222-268`, `docs/05-brain.md:17-31`). A model may suggest a candidate for a long-tail paraphrase, but Rust must still decide whether it is an addressed command.
- Local-only network policy, model-name validation, response size caps, stream parsing, and cancellation (`chat.rs:328-390`, `chat.rs:1165-1208`).
- Permissions, origin, and audit for any future tool-like action (`tools.rs:125-200`). A proactive model proposal must be marked `Origin::Proactive`, which already downgrades allowed write/execute actions to `Ask` (`tools.rs:141-155`).
- Human-visible reasons and fixed refusal lines. `obey::Refusal::line` is intentionally deterministic (`obey.rs:48-99`); the model may phrase around the result but cannot change the reason.

### Suggested test plan before implementation

- Unit-test schema decoding and reject cases: unknown kind/target, missing fields, NaN/infinity, negative or oversized duration, oversized reason/line, confidence bounds, extra fields.
- Property/table-test that every accepted `request_action` proposal maps only to an existing `ActionDef`/tag and produces no state change unless `request_action_quiet` accepts it.
- Test stale-response rejection by changing the observation sequence/state between model request and response.
- Test model timeout, malformed JSON, empty output, cancellation, and Ollama unavailable: all must produce `none` and leave `Pet` unchanged.
- Test physiological overrides: a proposed play/work reaction cannot suppress eating, drinking, sleep, bedridden behavior, directive/pin precedence, or transient actions.
- Test determinism by running the same accepted proposal through the same snapshot and comparing the resulting `Pet`; the only allowed stochastic input is the existing externally supplied roll.
- Test that direct explicit commands still bypass the model and preserve current `intent.rs` conservative behavior.
- Reuse the existing local endpoint, stream, and prompt invariants; extend `chat.rs` tests rather than creating a second network client.

### Gaps

- The repository does not currently define a `ReactionIntent` type, a planner trigger taxonomy, or a formal event sequence number. Those should be specified before implementation.
- The user-facing meaning of “JEV” is not defined in the repository; this note treats it as the proposed small local model/planner.
- No external source can guarantee a small model’s semantic reliability. Structured output constrains syntax, not truth or policy compliance; deterministic Rust validation remains mandatory.

## Source map and decision summary

### Takeaway

The recommended architecture is “model proposes, Rust validates and arbitrates, Core reduces.” The model belongs at the ambiguous-language/presentation boundary, never inside the deterministic state machine or as a general-purpose executor.

### Cited Findings

- Core and Body boundary: `docs/02-components.md:1-61`.
- Numeric/state-machine policy: `docs/03-core.md:1-75`.
- Brain/chat/intent policy: `docs/05-brain.md:1-49`.
- Roadmap constraints and existing deferred work: `docs/06-roadmap.md:24-45`.
- User-visible command behavior and local-only Ollama boundary: `README.md:71-85`.
- Pure reducer and event definitions: `apps/desktop/src-tauri/src/core/state_machine.rs:12-15`, `225-255`, `388-500`.
- Deterministic arbitration: `state_machine.rs:700-900`.
- Deterministic obedience: `apps/desktop/src-tauri/src/core/obey.rs:1-35`, `48-225`.
- Conservative parser and parser tests: `apps/desktop/src-tauri/src/core/intent.rs:1-15`, `88-96`, `222-268`, `494-626`.
- Chat integration and fallback behavior: `apps/desktop/src-tauri/src/chat.rs:886-905`, `1003-1045`, `1165-1208`.
- Loopback validation and tests: `chat.rs:328-349`, `1236-1242`.
- Existing tool permission/audit pattern: `apps/desktop/src-tauri/src/core/tools.rs:1-16`, `40-95`, `125-200`, tests `442-566`.
- Ollama structured JSON/schema output: [https://docs.ollama.com/capabilities/structured-outputs](https://docs.ollama.com/capabilities/structured-outputs).
- Ollama local API boundary: [https://docs.ollama.com/api/introduction.md](https://docs.ollama.com/api/introduction.md).
- JSON Schema validation model: [https://json-schema.org/understanding-json-schema/reference](https://json-schema.org/understanding-json-schema/reference).
- LLM application security risks: [https://owasp.org/www-project-top-10-for-large-language-model-applications/](https://owasp.org/www-project-top-10-for-large-language-model-applications/).
- AI risk-management framing: [https://www.nist.gov/itl/ai-risk-management-framework](https://www.nist.gov/itl/ai-risk-management-framework).

### Inferences

- A planner can add expressive reactions without compromising determinism if it is treated as an untrusted, fallible parser/classifier, not as an actor.
- The smallest safe first slice is a `none | say | request_action | suggest` proposal schema, one-shot inference, no tool loop, no autonomous tick-time model calls, and no new state mutation path.
- Keep model-generated wording downstream of fixed Core outcomes. For accepted action requests, the state machine decides whether the action happens, why it is refused, duration protection, and all numbers; the model only makes the response sound natural.

### Gaps

- The external sources document structured output and risk-management principles, not VPet-specific correctness. Repository tests and invariants are the authoritative acceptance criteria for this integration.
