# 0009: Public from the first commit

Status: accepted
Date: 2026-10-01

## Context

The plan was a private repo until the Phase 06 launch, so a failed Phase 01
gate would not leave a half-built tool on the brand's GitHub, and so the
launch would not be preempted.

The owner decided otherwise: build in the open. On a free account, a public
repo also switches on what a private one cannot have: CodeQL, rulesets,
OpenSSF Scorecard, build attestations, private vulnerability reporting and
SonarQube Cloud, all free.

## Decision

`speedsharmaai/phpm` is public from the first commit. Every public-repo
security feature is enabled in Phase 00.

## Consequences

- The research and decisions are readable by anyone, including the
  competitors in decision 0007. That is accepted; the code was never the moat.
- If Phase 01's gate fails, the repo stays up with `gate.md` explaining why,
  and is archived. An honest negative result is fine to have in public.
- The launch in Phase 06 is a release, not a reveal. The launch post and
  benchmark page still carry it.
- Trademark risk on the name is higher once public; the search in the
  Phase 01 tasks moves to the top of the list.
