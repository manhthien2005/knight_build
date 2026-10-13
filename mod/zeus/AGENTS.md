# Zeus J2ME — Module-specific agent rules

Inherits repository root `AGENTS.md`. Apply only to work under `mod/zeus/`.

## Real architecture

- `src/Zeus.java` is the main extension; `tools/PatchZeus.java` patches native client entry points; `tools/*Test.java` contains Java harness tests.
- `build-zeus-jar.sh` describes the patch -> compile -> overlay -> reproducible JAR flow. It compiles Zeus for **Java 6** (`--release 6`); verify classfile major **50** on the built artifact. Do not assume a successful host JVM test means the MIDlet works.
- Source and vendor classes use their **exact** runtime types. Confirm the deployed ABI, notably `CLib.mVector`, `InterfaceComponents.iCommand`, `Model.Menu2`, `InterfaceComponents.MsgDialog`, rather than writing adapters from guessed signatures.
- The vanilla client already provides UI dispatch, login, combat and movement paths. Reuse native behavior unless a traced, documented reason requires another path. No invented game packets.

## High-risk seams: prove, do not guess

- Verify `PatchZeus` injection location **relative to** native method initialization and UI event handlers. A callback at method entry may see unfinished state.
- In server menus, prove item creation, `Menu2` row selection and native `iCommand.perform()` from bytecode/client path; do not call `perform()` on uninitialized raw server commands.
- Foreground `GameCanvas.currentDialog` may supersede `menu2`. Confirm visible UI ownership before acting. Never interact with generic “Mua bán” dialogs by caption alone.
- For NPC automation, validate provenance, range, state, UI identity, command, debounce, retry bounds and cancellation. Unknown/ambiguous state must fail closed.
- Time policy GMT+7 versus countdown, zone-mask mapping and server schedule must be proven per feature; never claim server clock verified solely from ModByTminh compatibility.
- Preserve player movement/fire focus ownership, priority gates and emergency stop. Every early-return path that claims to release control needs a focused check.

## Verification

Use `knight-source-trace` then `knight-implementation` for bugs. Prefer a test through the **real tick and UI dispatch seam**, not a direct helper mock. Test malformed input, stale menu, duplicate dispatch, timeout, disabled state, and control release only when those apply to the changed path. Run focused affected tests first; broader regression only on a justified cross-cutting change.

For live UI behavior, hand off to `knight-runtime-acceptance` and the human operator. No agent-initiated game launch or packets without explicit authorization.
