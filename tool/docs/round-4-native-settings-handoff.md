# Round 4 handoff — drive the client's own settings, don't duplicate them

Written at the end of the session that shipped ATTACK/ITEM's at-spot half. Everything below
is verified against `mod/work/orig_decomp/`, with file and line references, so the next session
starts from facts rather than re-deriving them.

## The correction that reframes the round

The operator pointed out that the game already has a full auto/settings GUI:
*Chức năng → Khác → Nguyên liệu* for material-drop toggles, and a *tự động* menu carrying auto
attack, auto pickup with rarity and HP/MP/gold filters, auto potion and auto buff. That is true,
and it means **the pickup filter shipped in `Zeus.java` is a second mechanism competing with a
native one** — the exact mistake `docs/core/08-module-attack.md` §5.3 warns about for potions,
avoided there and walked into here.

Two concrete consequences:

1. Set "không nhặt" in the game menu and Zeus keeps picking up, because its loop never reads
   `bq.q`. Nobody can tell which mechanism collected an item.
2. The rank model is wrong. `item.ranks=11111` treats the five colours as independent flags. The
   client treats it as a **threshold**: `bq.java:541` skips a drop when `fa5.ct < bq.q.a`, so the
   menu reads "nhặt từ đồ xanh" — from blue *upward*. The server understands the threshold.

## The native settings map, verified

| Setting | Field | Evidence |
|---|---|---|
| Auto potion on/off | `bq.l` (`public static boolean`) | `co.java:112` writes it |
| HP / MP threshold | `ah.e[0]`, `ah.e[1]` — **two elements only** | `ah.java:71`, `co.java:113-114` |
| Pickup config | `bq.q`, class `be`, three bytes | `co.java:118-124`, `ah.java:213` |
| → item rank threshold | `bq.q.a`, 0..5 (option 5 stored as −1) | filter at `bq.java:541` |
| → HP/MP mode | `bq.q.c`, 0..3 | filter at `bq.java:546-552` |
| → gold mode | `bq.q.b`, 0..1 | `ah.java:213` argument order, see below |
| Pickup master off | `bq.q == null` | `cn.java:432-433` sets `g`, `:1069-1070` nulls it |
| Auto buff master | `bq.p` (`public static`) | `co.java:123`, loop at `bq.java:615-626` |
| Buff slots | `ah.f[n][0]` = skill id, `ah.f[n][1]` = on/off, `ah.b` = count | `ah.java:1765-1780` |
| Two more flags | `cf.I`, `cf.J` | `co.java:96-101`, sent by `co.c()` as type 4 |

Option labels, straight from `df.java:819-820`:

```
gw = { "Vật phẩm: ", "MP,HP: ", "Vàng: " }
gL = { {nhặt tất cả, nhặt từ đồ xanh, nhặt từ đồ vàng, nhặt từ đồ tím, nhặt từ đồ cam, không nhặt},
       {nhặt tất cả, chỉ nhặt HP, chỉ nhặt MP, không nhặt},
       {nhặt, không nhặt} }
```

The client **acts on these locally** — `bq.java:973` gates the whole pickup branch on
`cv ∈ {3,4,5,7} && bq.q != null`. So driving `bq.q` is not merely telling the server: it turns the
client's own collector on and off.

### The b/c swap, settled by reading all four paths

An earlier draft of this document called this "verify by experiment". It does not need one; it needed
reading the fourth path. `be`'s constructor swaps its own last two arguments — `be.java:12-16` is
`be(by2, by3, by4) { a = by2; b = by4; c = by3; }` — so the argument order and the field order are
never the same thing, and every call site has to be read with that in mind.

| Path | What it does | Resulting layout |
|---|---|---|
| Menu, `ah.java:213` | `new be(rank, R[1], R[2])` where `R[1]` is HP/MP and `R[2]` is gold | `a`=rank, `c`=HP/MP, `b`=gold |
| Filter, `bq.java:541`, `:546-552` | gates equipment on `q.a`, the two potion kinds on `q.c` | expects `c`=HP/MP |
| Send, `co.java:118-124` | writes `q.a`, `q.b`, `q.c` into bytes 4, 5, 6 | wire = rank, gold, HP/MP |
| Receive, `co.java:52` | `new be(o[4], o[5], o[6])` → `a`=o[4], `b`=o[6], `c`=o[5] | `a`=rank, `b`=HP/MP, `c`=gold |
| Summary text, `co.java:59-61` | prints `gL[1][q.b]` and `gL[2][q.c]` | expects `b`=HP/MP |

The menu and the filter agree. The receive path and the summary text agree with each other and
disagree with both. **So a server echo of the settings packet swaps HP/MP with gold, and this is a
vanilla defect, not something Zeus introduces.**

`applyPickup()` therefore writes the menu's layout — the one the filter honours — and the snapshot
publishes `bq.q` read back live rather than echoing what the tool asked for. If an echo ever does
swap them, the panel shows HP/MP and gold exchanged relative to the configured values, which is a
visible symptom instead of the tool quietly claiming a setting the client is not using.

## What Round 4 should do

**Delete, don't add.** Zeus's pickup loop, `item.ranks`, `item.material`, `item.consumable` and
`item.quest` all go. In their place: write `bq.q` and `bq.l`/`ah.e[]`/`bq.p`/`ah.f[n][1]`, then
`co.b()` once. The control file carries the operator's choices in the *native* shape — a rank
threshold, an HP/MP mode, a gold mode — and the game's own menu and the tool then always show the
same values.

Buff 1/2/3 is the cheapest item in the whole list: the cast loop already exists at
`bq.java:615-626`, already uses the `bq.j` predicate this build widened, and already syncs. Zeus
sets `ah.f[n][1] = 1` and `bq.p = 1`.

### Three defects the live run exposed

The character died after drinking 190 potions at a spot that out-damaged it. The module obeyed its
threshold exactly, so that part is a spot-and-threshold problem, not a logic bug. But it uncovered:

1. **Telemetry lies while dead.** `attack()` bails at `!ready()` because `alive()` is false, so
   `combatOff()` never runs and `atkstate` keeps reporting 0 — claiming it owns the combat fields
   while the character lies on the ground. Release on death and report it.
2. **No revive.** The operator asked for exactly this: on death, use a revive item and resume.
   `docs/core/09-module-item.md` §7 records the intent (`revive.ticketId`, bound once by name then
   used by id) and names `modsrc3/MOD10.java` as the precedent. The sender was not located this
   session — start there.
3. The operator explicitly **does not** want a safety brake that stops on repeated drinking. Revive
   and carry on is the requested behaviour.

## Round 4 outcome

Everything in "What Round 4 should do" was done, plus the settings surface the operator asked for.

**Native settings, not a second mechanism.** Zeus's own pickup loop is gone. `syncNativeSettings()`
writes `ah.e[0..1]`, `bq.q` (via `applyPickup`, mirroring `ah.java:213`), `bq.p` and `ah.f[n][1]`
(via `applyBuffs`, skipping a slot the character has not learned), then sends `co.b()` once — and
only when a fingerprint of those values actually changed, because `co.b()` is a packet.

**The two defects the live run exposed are fixed.** Death is handled before the readiness guard, so
`combatOff()` runs and `atkstate` stops claiming ownership of the combat fields while the character is
on the ground. `revive()` uses the client's own senders: opcode −30 with the on-the-spot NPC id when a
ticket is in the bag, falling through to opcode 31 (town) when it is not. There is no safety brake, as
requested. `revives` is published so the count is visible.

**The settings dialog.** One row per setting the two modules read, in two group columns: mode, the
spot readout, X, Y, attack radius, revive mode, the potion pump with its two thresholds; then item
rank, HP/MP mode, gold, buff 1/2/3, mount and the material dialog. It opens on what the *engine last
confirmed* rather than on what was last typed, so a clamped value is visible instead of silently
disagreeing with the file. Numbers are refused rather than clamped in the dialog, because the engine's
silent clamp would look like the field doing nothing. Geometry and font follow the window's DPI: the
grid is dense enough that a 96-DPI bitmap font clipped controls at 125%.

The quick auto toggle now writes the row's *saved* settings with only the mode changed, so pressing it
can no longer undo the dialog's work. Map and zone always come from the live reading even when the
settings name others: the mod can only work the map its character stands on, and travel is not
implemented, so honouring a configured map would arm auto on a spot it could never reach.

Defaults changed to match what the operator asked for: the potion pump on and revive-by-ticket. Both
are unreachable while the mode is off — the mod returns early on `atk.mode == 0` before it reaches
either — so they arm nothing on their own.

**A defect found by review, fixed.** `medalDialog()` matched on `fu.s.toString()`. Neither `da` nor
its parent `cg` overrides `toString()`, so it returned `ah@1a2b3c` and no wording could ever match:
the feature ran every tick and did nothing. Worse, it then pressed at `(0, 0)` — a pointer press on
the top-left corner, which lands on the OK button only by luck.

Both were **drift from the documentation, not gaps in it**: `docs/core/09-module-item.md` §4.3
already specified walking `ah.C` and calling `bt.a()` on the button whose accent-stripped caption
contains "ok". It now does that, reading the text from `da.q` (the wrapped body lines, which is what
`modsrc3/MOD06.java:94` reads too) and pressing through a fifth ASM patch that widens `ah.C` from
private to public. The wording match is narrowed to the server's own phrasings so the crafting NPC's
prompts (`df.u`, `df.gd`) cannot be dismissed out from under the operator.

**A second defect found by review, fixed.** The mod published `v=1` while the engine's parser required
`v=2`, so every snapshot would have been refused and the panel would have shown "no data" forever.

### Verified on 2026-09-02, without a game

The settings surface was driven end to end on the assembled package: type values into the dialog,
save, reopen, read the controls back. Radius 200, HP 70%, MP 25%, item rank 3 (nhặt từ đồ tím), HP/MP
mode 1 (chỉ nhặt HP), gold 1 (không nhặt), buff 1 on, material dialog off — all survived dialog → app
→ port → worker → `zeus-control.txt` → read back → dialog. The file on disk carried exactly those
eighteen keys at `v=2`.

Arming a mode still needs a live character, so mode remained off for that run: the engine refuses a
mode change with no spot to anchor on, which is the intended refusal.

### What Round 5 still needs the operator online for

- Arming a mode from the dialog against a live character, and confirming the configured X/Y is the
  spot the character actually holds.
- The zone list and auto-picking the emptiest zone (`cs.o[]` is retained; the names are not).
- Cross-map travel, which moves the character.
- The attack-radius circle drawn in the game.
- Whether a server echo really does swap HP/MP with gold, which the panel will now show if it happens.



## Still unknown

- **The "Nguyên liệu" close-drop toggles.** Settled: there is nothing to drive. `docs/core/09-module-item.md`
  §4.2 already had this right, and a second sweep on 2026-09-02 confirmed it from three directions.
  The whole string table across all four language classes holds exactly one material string —
  `df.ee = "Nguyên liệu:"`, a field label. All five settings channels are accounted for
  (`y`→0, `fc`→1 and 2, `co`→3 pickup/potion/buff, `co`→4 `cf.I`/`cf.J`) and none carries a material
  list. `fa.dH` is the material-*box* template list (`er.java:2169-2172`), tested by
  `fa.f(int)` — the debug string in its catch block is literally `isMaHopNguyenLieu` — and its only
  callers open a box (`fo.java:777`, `fo.java:1230`, `er.java:5371`).

  So the close-drop is **server state**, announced by a dialog whose text the server sends. The only
  thing any client-side code can do is press OK on the announcement, which is what the medal row
  does. `modsrc3/MOD06.java:94` matches the same wording on the same field, which is the independent
  confirmation.
- **Zone names.** Opcode 54 → `er.V()` (`al.java:350`) fills `cs.v` (count) and `cs.o[i]` (load),
  but the names are read into a **local** `String[]` and only used to build the menu, then dropped.

  The claim that followed this — that auto-picking the emptiest zone needs no names because `cs.o[]`
  is retained — is **wrong**, and `docs/core/12-control-transport.md` §10 now carries the correction.
  `modsrc3/MOD13.java` does not touch `cs.o[]`: it walks to the zone NPC (`cn.j` entity with
  `cv == 2` and `cu == -43` or a name containing `"Khu"`), sends **opcode 23** with that NPC's `cu`,
  then parses the player count out of each menu caption `"Khu N (M/K)"` and sends opcode 23 again
  with `N - 1`. Both directions are opcode 23, not 51, and the count comes from text.
- **Whether gold is a ground entity.** `cv == 6` exists in the client but is not one of the four
  drop kinds `er.A` builds. `bq.q.c` suggests the client handles gold itself.
- **Cross-map travel.** Still the 77-entry survey table. Now testable with a real account, but it
  moves the character, so it needs its own round with the operator watching.

## What already works, verified against a real account

Do not re-litigate these; they were observed live on 2026-09-02 with account `emthienbantia`,
level 80, at map 43 zone 4 (228, 164):

- AUTH logs itself in and enters the character with no simulated input.
- PLAYER publishes the real character; the panel renders every field correctly.
- The control file round-trips: `ctl` goes 0 → 1 on a valid body, back to 0 on one unknown key.
- ATTACK takes the combat fields (`atkstate` 0), holds a target, and **pins the position** —
  `px`/`py` did not move once in 90 seconds.
- ITEM picked up 173 drops, then **stopped exactly when the bag hit 42/42**, which is the
  bag-full guard doing its job unprompted.
- Potions fired 190 times; `stuck` stayed 0; XP permille rose and the rate measured 1,2%/giờ.
- A confirmed stop removes the process, the snapshot and the settings file.
