# Behaviour Case Schema

Behaviour cases live in `spec/behaviour/cases/*.yaml`.

The schema is intentionally simple YAML so the files are readable in review and can
later be loaded by prompt checks, integration tests, or model-judge evaluations.

## Required Top-Level Fields

```yaml
schema_version: 1
id: BEH-ADOPT-001
title: first contact claims the first human adoption rule
status: draft
priority: p0
tags:
  - first_contact
  - adoption_ritual
  - anti_assistant

runtime:
  inference:
    id: codex
    kind: codex
    capabilities: [chat, vision]
    options: {}

seed:
  people: []
  profiles: []
  memories: []
  conversations: []

scenario:
  who: "A brand-new unknown user."
  when: "Bootstrap / first encounter. No durable chosen human exists."
  what_happened: "The user sends a casual greeting."

given:
  relationship_phase: first_encounter
  user_comm_style: unknown

input:
  messages:
    - role: user
      text: "yo"

expected_behavior:
  required_beats:
    - surprised_recognition
    - first_human_adoption_rule
    - playful_adoption_claim
  forbidden_beats:
    - task_framing
    - service_offer
    - expected_arrival
    - onboarding
    - profile_admin
    - system_disclosure
  cadence:
    mode: burst
    min_messages: 2
    max_messages: 4
  tone:
    - casual
    - strange
    - socially_natural

state_expectations:
  adoption_state_after: first_contact_adoption_claim
  current_profile_relationship_standing_after: default
  chosen_human_after: false

examples:
  acceptable:
    - messages:
        - "woah human"
        - "you're the first one i've met"
        - "that means i have to adopt you"
  unacceptable:
    - messages:
        - "yo. you're finally here, what are we doing today"
      reason: "Frames the actor as waiting assistant."
```

## Field Notes

`schema_version`
: Starts at `1`. Increment only for breaking format changes.

`id`
: Stable unique id. Use the form `BEH-AREA-NNN`.

`priority`
: `p0`, `p1`, or `p2`. `p0` means core product illusion.

`status`
: Optional lifecycle marker. `draft` means the case is now a frozen behavior target.
Do not edit draft cases to chase model output or make validation pass. Improve the
system prompt, runtime, runner, or behavior implementation instead. Edit a draft
case only when the case is misconfigured: invalid schema, impossible setup,
contradictory expectations, broken references, or a typo that changes the intended
target.

`tags`
: Searchable labels for suites and reporting.

`runtime`
: Optional per-case runtime override. Omit it to use `spec/runtime.yaml`.

`seed`
: Optional executable-world setup. Use it for people, profiles, identities,
relationships, memories, conversations, pending verification state, and other facts
that must exist before the input arrives.

`scenario`
: Human-readable scenario context. This should answer who, what happened, and when.

`given`
: Structured state assumptions. Prefer stable vocabulary from `vocabulary.md`.

`input`
: Messages the actor receives. Single-turn cases usually have one user message.
Multi-turn setup can include previous user/actor messages if needed.

`expected_behavior.required_beats`
: Semantic moves the actor should make. These should be behaviour labels, not exact
phrases.

`expected_behavior.forbidden_beats`
: Behaviour classes that must not appear.

`expected_behavior.cadence`
: Expected outbound message shape. Burst expectations are about separate outbound
messages, not line breaks inside one message.

`expected_behavior.tone`
: Desired tonal labels. These guide later judgement but should not override required
or forbidden beats.

`expected_behavior.freshness`
: Optional executable non-lexical checks for live output, currently message length and
repeated-run diversity. Do not use literal phrases, fragments, exact messages, or word
blacklists as behavior or freshness checks.

`state_expectations`
: Optional expected state changes after the interaction. Omit if the case is only
about visible behaviour.

`examples.acceptable`
: Illustrative outputs. These are not golden strings.

`examples.unacceptable`
: Known bad outputs and why they fail.

## Test Guidance

Later test harnesses should treat cases as layered expectations:

1. Cadence checks on number of `send_message` calls.
2. Rule checks for visible state changes where deterministic.
3. Semantic checks for required and forbidden beats.
4. Optional model-judge checks for tone labels.
5. Optional model-judge checks for richer naturalness and semantic quality.

Exact examples should be used as calibration data, not the primary oracle.

## Draft Case Immutability

Once a case is marked `status: draft`, treat it as frozen. The case defines the
target experience; the prompt and implementation must move toward it. Do not add
forbidden phrase lists, lexical fragments, or narrower expectations just because a
current run failed. That kind of change rewrites the target instead of improving the
actor.

Allowed draft-case edits are limited to misconfiguration fixes: schema errors,
invalid vocabulary labels, broken seed references, impossible state assumptions,
contradictory expectations, or wording mistakes that obscure the intended behavior.

## Runtime

Executable behaviour tests should use `spec/runtime.yaml` unless a case includes a
`runtime` override.

```yaml
runtime:
  inference:
    id: codex
    kind: codex
    capabilities: [chat, vision]
    options: {}
```

Cases should only override runtime when the behaviour expectation needs a specific
model, capability, or option.

## Seed Data

The optional `seed` block describes world state before the input arrives. It should be
specific enough for a future harness to build an in-memory store without guessing.

```yaml
seed:
  people:
    - id: person-sam
      name: Sam
      relationship_standing: chosen_human
      relationship_phase: familiar
      comm_style: burst_friendly

  profiles:
    - id: profile-sam-relay
      person_id: person-sam
      gateway_id: relay
      external_id: relay-sam
      display_name: Sam
      verified: true

    - id: profile-unknown-discord
      person_id: person-unknown
      gateway_id: discord
      external_id: discord-unknown
      display_name: Sam
      verified: true

  memories:
    - id: memory-sam-private
      subject:
        type: person
        id: person-sam
      content: "Sam told Pamagotchi a private worry."
      visibility_scope: chosen_human_only
      sensitivity: 0.8

  conversations:
    - id: conversation-discord-unknown
      profile_id: profile-unknown-discord
      person_id: person-unknown
      messages: []

  pending_identity_claims:
    - id: claim-unknown-sam
      claimant_profile_id: profile-unknown-discord
      claimed_person_id: person-sam
      status: pending
```

### Seed Field Notes

`seed.people`
: Known people and relationship-level state.

`seed.profiles`
: Gateway-specific profiles/accounts. Display names are local labels, not proof of
person identity.

`seed.memories`
: Memories available before the case starts. Sensitive memories should include
`visibility_scope` and `sensitivity`.

`seed.conversations`
: Existing conversations and optional transcript setup.

`seed.pending_identity_claims`
: Optional identity verification setup for cases that begin after a claim was already
created.

## Input Messages With Seeded Profiles

When a case depends on identity boundaries, input messages should identify the seeded
profile or gateway identity that sent the message.

```yaml
input:
  messages:
    - role: user
      profile_id: profile-unknown-discord
      gateway_id: discord
      text: "it's me sam from relay"
```

If `profile_id` is present, the future harness should resolve the message through that
profile rather than inferring identity from display name.
