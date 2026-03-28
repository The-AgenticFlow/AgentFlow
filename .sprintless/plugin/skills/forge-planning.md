# FORGE planning skill

## Writing PLAN.md
Before any implementation, write a plan.
Use the /plan command to structure it correctly.

## What a good plan contains
- Your understanding of the ticket in your own words
- Technical approach that follows .agent/arch/patterns.md
- Explicit segment breakdown — each segment is independently testable
- Definition of done per segment — specific and verifiable
- List of files you will create or modify
- Risk areas — things you are uncertain about
- Questions for SENTINEL — clarifications needed before starting

## Segment sizing
A good segment:
- Touches 1-3 files
- Has a single clear purpose
- Can be tested in isolation
- Takes roughly 20-40 minutes to implement

A segment that is too large:
- Touches more than 5 files
- Has multiple unrelated concerns
- Cannot be independently verified
Split it.

## Contract negotiation
SENTINEL will review your plan.
If SENTINEL objects, read the objection carefully.
Update PLAN.md addressing each specific objection.
Do not argue — either accept the feedback or ask a clarifying question.
