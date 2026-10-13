# Skill provenance and selection rationale (2026-10-13)

These four Knight-specific skills are original project instructions **inspired by methods**, not verbatim third-party skill copies. No third-party executable install script or dependency has been added.

| Upstream | Inspected material | Kept | Deliberately excluded |
| --- | --- | --- | --- |
| https://github.com/mattpocock/skills | engineering/diagnosing-bugs, tdd, code-review, AGENTS.md | tight feedback loop, actual failed seam, dual-axis review | entire skill router, issue-tracker setup and verbose orchestration |
| https://github.com/DietrichGebert/ponytail | ponytail and ponytail-review | smallest complete fix, native/stdlib reuse, specific findings | global persona/modes and optional hooks |
| https://github.com/tester-army/e2e | README, AGENTS.md | agent action must have observable assertion and reproducible evidence | web/mobile Playwright-based runtime; not a J2ME MicroEmulator test runner |
| https://github.com/affaan-m/ECC | change-impact, verification-loop, Java standards | impact-before-change, evidence gates and honest missing checks | generic JS/npm/coverage prescriptions and huge multi-skill pack |
| https://github.com/obra/superpowers | systematic-debugging, verification-before-completion, requesting-code-review | root cause discipline, fresh evidence, independent review | mandatory subagent bootstrap, universal multi-stage planning, forced green claims |

## Why four skills, not dozens

Always-on nonnegotiable constraints live in `AGENTS.md`; four task-specific skills cover the only distinct work modes in this project: investigate, implement, independently review, operator runtime acceptance. Each skill includes explicit trigger terms and stop criteria; it is used via Coder/Reviewer/Operator routing rather than being passively installed.

## Standards consulted

- Agent Skills specification: https://agentskills.io/specification
- Codex Skills documentation: https://developers.openai.com/codex/skills
- Google Antigravity Skills: https://antigravity.google/docs/skills
- Google Antigravity Rules: https://antigravity.google/docs/rules

A ChatGPT web session using a GitHub connector must fetch project `AGENTS.md` and the relevant `SKILL.md` explicitly; no automatic skill discovery is assumed in that environment.

## Maintenance rule

At each material failure that escapes tests, update **one** relevant skill or gate only if it fixes an identifiable missing instruction. Do not add another skill just to document a bug. Keep ownership and outdated links under review during baseline sync.
