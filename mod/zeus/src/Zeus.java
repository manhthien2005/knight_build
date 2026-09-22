/*
 * Zeus_Knight — local-only assistant for KnightOnline_402 (J2ME MIDlet).
 *
 * Modules: AUTH (enter the game), PLAYER (publish a read-only snapshot),
 * ATTACK (hold a spot and fight there) and ITEM (pick up drops, ride, dismiss a dialog).
 *
 * PLAYER sends no packet and writes no native field. It reads cn.g every tick and
 * writes a small key=value file the tool polls; see docs/core/11-player-transport.md
 * for why a file and not a socket. Field semantics are verified in
 * docs/core/10-module-player.md — the two that are easy to get wrong:
 *
 *   - cn.g.bA is XP as PERMILLE of the current level (0..1000), not absolute XP.
 *     Proof: the client renders bA/10 + "," + bA%10 + "%" (cf.java:795) and sizes
 *     the bar as bA/10*77/100 (cf.java:798).
 *   - cn.g.bD is gold (readLong) and cn.g.bC is gem (readInt), and BOTH arrive
 *     only with the inventory packet, opcode 16 (er.java:4982-4983, reached from
 *     er.o via al.java case 16). The chest packet, opcode 65, carries no wallet.
 *     So before opcode 16 both read 0, which is why walletKnown exists: an unknown
 *     wallet must render as a dash, never as zero.
 *
 * ATTACK and ITEM read their settings from the file the tool writes, named by
 * -Dzeus.ctl.in; see docs/core/12-control-transport.md. Both fail closed: anything
 * unparsable turns every module off rather than acting on half a setting.
 *
 * Neither module invents a packet. ATTACK sets the client's own auto fields and lets
 * bq.q() pick targets and bq's own loop swing; ITEM sends the same opcode 20 the
 * client sends when the operator taps a drop. Every action travels through code the
 * client already ships.
 *
 * AUTH, verified against orig_decomp/:
 *
 *   - bs.c() (login-screen constructor) reads RMS user_pass and, when present,
 *     calls bs.i() + fu.o() + bs.a(i,j) — i.e. the CLIENT logs itself in.
 *     (bs.java:127-139)
 *   - After login the client shows the character-select screen (fu.i, class x).
 *   - x.a() ticks that screen; when ah.k == true it selects the slot x.k and
 *     enters the game. (x.java:132-146)
 *   - The vanilla reconnect loop (bv.a + ah.a, 30 s) is untouched — Auth does
 *     not swallow the disconnect dialog in round 1, so the native loop keeps
 *     running on its own. (bv.java, ah.java:1289-1302)
 *
 * So the ONLY thing AUTH does: when the character-select screen appears, set
 * the slot cursor and raise ah.k. Everything else is the client's own code.
 */
public final class Zeus {

    /** Character slot to enter (0..2). Override with -Dzeus.auth.slot. Returns -1 if malformed or outside 0..2. */
    private static int slot() {
        try {
            String v = System.getProperty("zeus.auth.slot");
            if (v == null) {
                return 0; // legacy default is Slot 1 (internal index 0)
            }
            v = v.trim();
            if (v.length() == 0) {
                return -1;
            }
            int val = Integer.parseInt(v);
            if (val >= 0 && val <= 2) {
                return val;
            }
            return -1;
        } catch (Throwable t) {
            return -1;
        }
    }

    private static boolean armed = false;   // one-shot per char-select visit
    private static final int AUTH_RETRY_INTERVAL_TICKS = 75; // 3.0s at 25 t/s
    private static final int AUTH_MAX_ATTEMPTS = 3;
    private static int authAttempts = 0;
    private static int authWaitTicks = 0;
    private static boolean authExhaustedTraced = false;
    private static boolean authRefusalTraced = false;

    public static void authReset() {
        armed = false;
        authAttempts = 0;
        authWaitTicks = 0;
        authExhaustedTraced = false;
        authRefusalTraced = false;
    }

    /** Called at the end of fu.b() every tick. */
    public static void tick() {
        if (fu.a == null) {
            sessionReset();
            return;
        }
        sessionTick();
        auth();
        control();
        dialogRecovery();
        travel();
        // ---- ENHANCE ----------------------------------------------------------
        // After travel() and before drops(): like travel it walks to an NPC and drives a menu,
        // so it has to run ahead of the modules that gate on ready() and on no menu being open.
        enhance();
        // ---- end ENHANCE ------------------------------------------------------
        // ---- DUNGEON ----------------------------------------------------------
        // Outside items(), for the reason drops() and zone() are outside it: items() gates on
        // ready(), which requires no dialog, and an NPC menu is exactly the state this module has
        // to act in. Gated there it would wait for the operator to dismiss a menu it had opened
        // itself. After travel() and before drops(): both this and a trip the operator armed reach
        // for the movement lock, and what keeps them from fighting is canMove() inside travelMove(),
        // which makes the second caller bail rather than overwrite a walk already in flight — not the
        // call order, which only decides which of the two gets to be first.
        dungeon();
        // ---- end DUNGEON ------------------------------------------------------
        // Outside items(): that gates on ready(), which requires no menu open, and an open menu is
        // exactly the state these two have to act in. Gated there, each one waited for the operator
        // to dismiss a menu it had opened itself. DROPS first, because its confirmation dialog is
        // what medalDialog() inside items() would otherwise press away.
        drops();
        zone();
        // Before attack(), and outside it. Reviving is not part of holding a spot: attack() returns
        // early whenever `atk.mode == 0` or no spot is set, and revive nested inside it meant a
        // character configured to revive but not to farm stayed on the ground with `atk.revive=1`
        // sitting in the control file doing nothing.
        revive();
        // Outside attack(): that returns early whenever atkMode == 0 or no spot is set, and
        // potions nested inside it meant a character configured to auto-drink but not to farm
        // never drank at all. Same reasoning that moved revive() out.
        potions();
        attack();
        items();
        player();
        // Last, so a trace records what the modules above already did this tick, and so the flush
        // is the final thing that happens: a client crash still leaves everything recorded.
        traceCheck(dx.a());
        traceTick();
        if (probeArmed) {
            probeArmed = false;
            probeStone();
        }
        if (spotArmed) {
            spotArmed = false;
            probeSpots();
        }
        spotSidecarTick(dx.a());
        traceFlush();
    }

    // ---- AUTH ----------------------------------------------------------------

    private static void auth() {
        try {
            if (fu.a == fu.i) {
                // Character-select screen. Strict positional validation — CHAR-SLOT-02.
                int targetSlot = slot();
                if (targetSlot < 0 || targetSlot > 2) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot invalid internal=" + targetSlot + " reason=MALFORMED_OR_OUT_OF_RANGE");
                    }
                    return;
                }

                if (x.a == null) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot refused internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=0 reason=LIST_NULL");
                    }
                    return;
                }

                int count = x.a.c();
                if (targetSlot >= count) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot refused internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=" + count + " reason=INDEX_OUT_OF_BOUNDS");
                    }
                    return;
                }

                if (x.a.a(targetSlot) == null) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot refused internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=" + count + " reason=NULL_CHARACTER_OBJECT");
                    }
                    return;
                }

                // fu.a == fu.i && x.a != null && targetSlot >= 0 && targetSlot < x.a.c() && x.a.a(targetSlot) != null
                if (authAttempts == 0) {
                    // x.k is private in vanilla; PatchZeus widens it to public.
                    // Access through the live screen instance fu.i.
                    fu.i.k = targetSlot;
                    // ah.k makes x.a() select the slot and enter the game.
                    ah.k = true;
                    armed = true;
                    authAttempts = 1;
                    authWaitTicks = 0;
                    trace("AUTH select character slot internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=" + count + " attempt=1");
                } else if (authAttempts < AUTH_MAX_ATTEMPTS) {
                    if (++authWaitTicks >= AUTH_RETRY_INTERVAL_TICKS) {
                        authWaitTicks = 0;
                        ++authAttempts;
                        fu.i.k = targetSlot;
                        ah.k = true;
                        trace("AUTH retry character select slot=" + fu.i.k + " attempt=" + authAttempts);
                    }
                } else {
                    if (!authExhaustedTraced) {
                        authExhaustedTraced = true;
                        trace("AUTH character select retry exhausted attempts=" + authAttempts);
                    }
                }
            } else if (fu.a != fu.i) {
                // Any other screen: re-arm for the next visit.
                authReset();
            }
        } catch (Throwable t) {
            // Never let a mod failure stall the client tick.
        }
    }

    // ---- CONTROL --------------------------------------------------------------
    //
    // Settings arrive as a file the tool replaces atomically. Everything here fails
    // closed: one unreadable byte and every module turns off, because a half-read file
    // could carry another account's spot. docs/core/12-control-transport.md §5.

    /**
     * Trace marker and log names, declared here rather than beside the trace code because the
     * static initializer below derives their paths and a later declaration is a forward reference.
     */
    private static final String TRACE_MARKER = "zeus-trace.on";
    private static final String TRACE_FILE = "zeus-trace.txt";
    /**
     * One-shot probe marker. Its own file, not a control key, for the same reasons as the trace.
     *
     * It exists to answer one question no amount of reading the client can: whether the *server*
     * refuses a board interaction sent from far away. The client does not check — `cn.a(0,·)` calls
     * `i.k()` for any `cv != 0` target without the hostility test it applies to players, and
     * `ez.k()` then sends opcode 23 with the board's `cu` (`ez.java:239-243`). So only a live send
     * from a distance can settle it, and the answer decides whether the zone feature has to walk
     * the character to the board or can leave it where it stands.
     */
    private static final String PROBE_MARKER = "zeus-probe.on";

    /**
     * Marker for the monster-spot probe. Separate from the stone probe because it sends nothing and
     * only reads the scene — so it is safe to flip repeatedly, once per spot the operator stands in.
     */
    private static final String SPOT_MARKER = "zeus-spot.on";
    private static final String SPOT_REQ_FILE = "zeus-spot.req";
    private static final String SPOT_RESULT_PAYLOAD_FILE = "zeus-spot-result.tmp";
    private static final String SPOT_RESULT_READY_FILE = "zeus-spot-result.ready";

    /**
     * Derived paths for the two above, declared here for a sharper reason: a static field with an
     * initializer runs at its own position in the file, so declaring these below the static
     * initializer meant `= null` executed *after* the block had filled them in and silently undid
     * it. Tracing then could never turn on, with nothing to see anywhere.
     */
    private static String traceMarkerPath;
    private static String tracePath;
    private static String probeMarkerPath;
    private static String spotMarkerPath;
    private static String spotReqPath;
    private static String spotPayloadPath;
    private static String spotReadyPath;

    /** Settings source, or null to keep every module off. Set by -Dzeus.ctl.in. */
    private static String ctlPath = null;
    /** How often the settings are re-read, in ms. */
    private static final long CTL_EVERY_MS = 500L;
    private static long ctlReadAt = 0L;

    /** 0 off, 1 stand and fight, 2 follow the target. */
    private static int atkMode = 0;
    private static int atkMap = 0;
    private static int atkZone = -1;
    private static int atkX = -1;
    private static int atkY = -1;
    private static int atkRadius = 120;
    private static boolean atkHpOn = false;
    private static boolean atkMpOn = false;
    private static int atkHpPct = 50;
    private static int atkMpPct = 50;
    /**
     * REVIVE's own settings. Deliberately not `atk*`: this module runs from `tick()` whether or not
     * a spot is armed, so naming it after ATTACK is what led to it being nested inside `attack()`
     * and never running with auto off.
     *
     * `reviveOn` is the switch. `reviveMode` picks the path: 1 uses a ticket where the character
     * fell, 2 walks it back from town. Mode is meaningless while the switch is off.
     */
    private static boolean reviveOn = false;
    private static int reviveMode = 1;

    // ---- ENHANCE --------------------------------------------------------------
    /**
     * ENHANCE's own settings, named for the module rather than for ATTACK. Like REVIVE it runs
     * from `tick()` whether or not a spot is armed: a trip to the blacksmith is not a fight, and
     * nesting it inside `attack()` is exactly how REVIVE ended up never running.
     *
     * `enhanceOn` is the switch. `enhanceMaxLevel` is the level to stop at. `enhanceCharmType`
     * picks the charm each attempt is allowed to spend: 0 none, 1 three-leaf clover,
     * 2 four-leaf clover, 3 smart.
     */
    private static boolean enhanceOn = false;
    private static int enhanceMaxLevel = 10;
    private static int enhanceCharmType = 0;
    /**
     * ENHANCE's own state, published so a trip that is not moving says why rather than looking
     * identical to a switch that is off.
     *
     * `enhancePhase`: 0 idle, 1 walk, 2 openNPC, 3 menu, 4 item, 5 charm, 6 confirm, 7 wait.
     * `enhanceWhy`: 0 ok, 1 no NPC, 2 no charm, 3 item gone, 4 max reached.
     * `enhanceWait` is the ticks left in a phase that has to let the server answer before it
     * asks again. `enhanceDone` counts finished attempts and survives a settings change: it is a
     * tally of what already happened, not a piece of in-flight state.
     */
    private static int enhancePhase = 0;
    private static int enhanceWhy = 0;
    private static int enhanceDone = 0;
    private static int enhanceWait = 0;
    // ---- end ENHANCE ----------------------------------------------------------

    /**
     * Pickup settings in the client's own shape, not a shape of our own invention.
     *
     * `itemRank` is a threshold, not a set of flags: `bq.java:541` skips a drop when
     * `fa.ct < bq.q.a`, so 1 means "blue and better". Index 5 in the client's own option list is
     * "don't pick equipment", which it stores as −1 (`ah.java:208-212`). `itemMpHp` and `itemGold`
     * are the other two bytes of the same native record. Zeus writes them exactly the way the
     * game's own menu does and lets the client's collector do the work — a second filter here
     * would fight the native one and nobody could tell which had collected an item.
     */
    private static int itemRank = 5;
    private static int itemMpHp = 3;
    private static int itemGold = 1;
    private static boolean itemMedal = false;

    /**
     * MOUNT's own settings, named for the module rather than for ITEM.
     *
     * `mountOn` is the switch; `mountId` is 0 for "whichever mount is in the bag", or one of
     * {62..66} for exactly that one.
     */
    private static boolean mountOn = false;
    private static int mountId = 0;

    /** Buff slots the operator can address. The client's own count is `ah.b`. */
    private static final int BUFF_SLOTS = 3;

    /**
     * Materials whose drop the server can close, in the menu order measured from a trace.
     *
     * Declared here, above the arrays sized by it, because a static field initializer runs at its own
     * position in the file — the same trap that once left the trace paths null.
     */
    private static final int MATERIAL_SLOTS = 6;

    /** One flag per buff slot, so slot 1 cannot silently become slot 2. */
    private static final boolean[] atkBuff = new boolean[BUFF_SLOTS];

    /** 0 keep the current zone, 1 move to the emptiest, 2 move to the one the operator named. */
    private static int atkZoneMode = 0;
    private static int atkZonePick = 1;

    /** Whether the tool drives the material close-drop at all, and the state it wants per material. */
    private static boolean dropsManaged = false;
    private static final boolean[] dropWanted = new boolean[MATERIAL_SLOTS];
    /** Slots already confirmed to be in the wanted state, so a pass does not repeat itself. */
    private static final boolean[] dropDone = new boolean[MATERIAL_SLOTS];

    /**
     * Whether the tool and this jar agree about the settings: -1 no property, 0 nothing readable,
     * 1 the last read parsed.
     *
     * Published because failing closed is silent by design. Without this, "auto does nothing" and
     * "the settings file is one key out of date" look identical from outside the JVM.
     */
    private static int ctlState = -1;

    /** Turns every module off. The only state a failed read may leave behind. */
    private static void allOff() {
        ctlState = 0;
        atkMode = 0;
        atkX = -1;
        atkY = -1;
        atkHpOn = false;
        atkMpOn = false;
        reviveOn = false;
        reviveMode = 1;
        reviveDelay = 0;
        reviveReset();
        for (int i = 0; i < BUFF_SLOTS; i++) {
            atkBuff[i] = false;
        }
        // The client's own "don't pick" values, so failing closed leaves the native collector off
        // rather than leaving whatever the last good file asked for.
        itemRank = 5;
        itemMpHp = 3;
        itemGold = 1;
        mountOn = false;
        mountId = 0;
        itemMedal = false;
        // The zone walk and the material walk both stop where they are: neither leaves a half-done
        // pass armed, and neither undoes what it already changed — the server owns that state.
        atkZoneMode = 0;
        zonePhase = 0;
        dropsManaged = false;
        dropSlot = -1;
        dropPhase = 0;
        // Travel stops too, and stops mid-route rather than finishing: a route was authorised by a
        // control file that is no longer trusted.
        travelReset();
        // The overlay goes with them: it is drawn from settings, and a ring left on screen after the
        // settings were rejected would be showing thresholds nothing is enforcing.
        ringOn = false;
        atkFarmOnArrival = false;
        // ---- ENHANCE ----------------------------------------------------------
        // Enhance stops too, and stops mid-trip rather than finishing: the walk was authorised
        // by a control file that is no longer trusted. `enhanceDone` survives, because a tally
        // of what already happened is not a setting.
        enhanceOn = false;
        enhancePhase = 0;
        enhanceWhy = 0;
        enhanceWait = 0;
        // ---- end ENHANCE ------------------------------------------------------
        // ---- DUNGEON ----------------------------------------------------------
        // The dungeon trip stops too, and stops mid-menu rather than finishing: the run was
        // authorised by a control file that is no longer trusted, so it stops where it is. The
        // switch has to go with it. dungeonReset() clears dungeonWhy, and dungeon() re-arms DN_OFF
        // into DN_IDLE whenever why is zero, so a module left enabled would simply start the trip
        // again on the next tick — off a file that was just rejected. enhanceOn above is the
        // precedent; dungeonRuns survives it, for the reason dungeonReset()'s own note gives.
        dungeonEnabled = false;
        dungeonReset();
        // ---- end DUNGEON ------------------------------------------------------
    }

    private static void control() {
        if (ctlPath == null) {
            return;
        }
        try {
            long now = dx.a();
            if (now - ctlReadAt < CTL_EVERY_MS) {
                return;
            }
            ctlReadAt = now;
            String body = read(ctlPath);
            if (body == null || !parseControl(body)) {
                allOff();
            } else {
                ctlState = 1;
            }
            // `co.b()` is a packet, so it is sent when a setting actually changed rather than every
            // half second. The signature covers exactly the values that live in native fields.
            int signature = nativeSignature();
            if (signature != syncedSignature && inGame() && cn.g != null) {
                syncedSignature = signature;
                syncNativeSettings();
            }
        } catch (Throwable t) {
            allOff();
        }
    }

    /** Fingerprint of every setting that is mirrored into a native field. */
    private static int nativeSignature() {
        int signature = atkHpPct * 101 + atkMpPct * 103 + itemRank * 107 + itemMpHp * 109
                + itemGold * 113 + (atkHpOn ? 127 : 0) + (atkMpOn ? 131 : 0);
        for (int i = 0; i < BUFF_SLOTS; i++) {
            signature = signature * 31 + (atkBuff[i] ? 1 : 0);
        }
        return signature;
    }

    /** Signature last pushed with `co.b()`. Deliberately impossible to match on the first read. */
    private static int syncedSignature = Integer.MIN_VALUE;

    /** Reads a whole small file, or null when it is absent, too big, or unreadable. */
    private static String read(String path) {
        java.io.InputStream stream = null;
        try {
            java.io.File file = new java.io.File(path);
            if (!file.exists()) {
                return null;
            }
            long size = file.length();
            if (size <= 0L || size > 4096L) {
                return null;
            }
            byte[] buffer = new byte[(int) size];
            stream = new java.io.FileInputStream(file);
            int filled = 0;
            while (filled < buffer.length) {
                int step = stream.read(buffer, filled, buffer.length - filled);
                if (step < 0) {
                    return null;      // shorter than advertised: a racing write
                }
                filled += step;
            }
            return new String(buffer, 0, filled, "UTF-8");
        } catch (Throwable t) {
            return null;
        } finally {
            if (stream != null) {
                try {
                    stream.close();
                } catch (Throwable t) {
                    // nothing useful to do on a failed close
                }
            }
        }
    }

    /** Control keys, in the order their values sit in the parse buffer. */
    private static final String[] CTL_KEYS = {
        "v", "atk.mode", "atk.map", "atk.zone", "atk.x", "atk.y", "atk.radius",
        "atk.hpOn", "atk.hpPct", "atk.mpOn", "atk.mpPct", "revive.mode", "atk.buffs",
        "atk.zoneMode", "atk.zonePick",
        "item.rank", "item.mphp", "item.gold", "mount.on", "mount.id",
        "item.medalDialog", "item.dropsOn", "item.drops",
        "nav.target", "ui.ring", "atk.farmOnArrival", "nav.detectSpots", "revive.delay",
        "revive.on",
        // ---- ENHANCE ----------------------------------------------------------
        "enhance.on", "enhance.maxLv", "enhance.charm",
        // ---- end ENHANCE ------------------------------------------------------
        // ---- DUNGEON ----------------------------------------------------------
        "dungeon.on", "dungeon.max", "dungeon.schedule",
        // ---- end DUNGEON ------------------------------------------------------
    };
    private static final int K_V = 0, K_MODE = 1, K_MAP = 2, K_ZONE = 3, K_X = 4, K_Y = 5;
    private static final int K_RADIUS = 6, K_HPON = 7, K_HP = 8, K_MPON = 9, K_MP = 10, K_REVIVEMODE = 11;
    private static final int K_BUFFS = 12, K_ZONEMODE = 13, K_ZONEPICK = 14;
    private static final int K_RANK = 15, K_MPHP = 16, K_GOLD = 17;
    private static final int K_MOUNT = 18, K_MOUNTID = 19, K_MEDAL = 20;
    private static final int K_DROPSON = 21, K_DROPS = 22;
    // A destination in its own right, not the spot's map. atk.travel walks to where the spot is;
    // nav.target walks to a map the operator named, with no spot involved and no fighting on arrival.
    // -1 is off, which is what every existing account is configured for.
    private static final int K_NAVTARGET = 23;
    // Purely local: it draws, it never sends. Off by default because it changes what the operator
    // sees, and a ring nobody asked for over a game they are watching is an imposition.
    private static final int K_RING = 24;
    // Arriving at the spot parks the character; fighting there is a second decision. Off by default on
    private static final int K_FARM = 25;
    private static final int K_DETECT = 26;
    // The revive module's own three keys. It is not part of ATTACK and shares no field with it: a
    // character can be set to revive without ever being armed to fight, which is the whole point of
    // the switch. `revive.on` is the switch, `revive.mode` picks the path, `revive.delay` is the
    // pause in seconds before the first attempt.
    private static final int K_REVIVEDELAY = 27;
    private static final int K_REVIVEON = 28;
    // ---- ENHANCE --------------------------------------------------------------
    // The enhance module's own three keys. It is not part of ATTACK and shares no field with it:
    // upgrading a piece of equipment is a trip to the blacksmith rather than a fight, so the
    // switch has to work on a character that is not armed to swing at anything. `enhance.on` is
    // the switch, `enhance.maxLv` is the level to stop at, `enhance.charm` picks which charm the
    // trip is allowed to spend.
    private static final int K_ENHANCE_ON = 29;
    private static final int K_ENHANCE_MAXLV = 30;
    private static final int K_ENHANCE_CHARM = 31;
    // ---- end ENHANCE ----------------------------------------------------------
    // ---- DUNGEON --------------------------------------------------------------
    // The dungeon module's own three keys. Not part of ATTACK and sharing no field with it: a run
    // is a trip to one NPC and back, so the switch has to work on a character that is not armed to
    // hold a spot — the reason REVIVE and ENHANCE are their own modules too. `dungeon.on` is the
    // switch, `dungeon.max` is how many runs to make before stopping, `dungeon.schedule` is the
    // half-hour slot of day to leave at.
    //
    // Both of the last two take -1 as their off sentinel rather than 0, because 0 is a real value
    // inside each range: zero runs would mean "stop before starting" and slot 0 is 00:00. -1 sits
    // outside both, so it cannot be mistaken for a setting, and it is what every existing account
    // is configured for — which is also what makes the module fail closed.
    private static final int K_DUNGEON_ON = 32;
    private static final int K_DUNGEON_MAX = 33;
    private static final int K_DUNGEON_SCHED = 34;
    // ---- end DUNGEON ----------------------------------------------------------
    /** Format version this jar accepts. Bumped when the key set changed shape. */
    private static final int CTL_VERSION = 13;

    /**
     * Parses one control body, returning false the moment anything is wrong.
     *
     * Strict on purpose: an unknown key means the tool and this jar disagree about the
     * format, and guessing which half to trust is how a spot from one account ends up
     * steering another. Ranges are re-checked here even though the tool clamps, because
     * the tool is not the only thing that can put a file on disk.
     *
     * `atk.buffs` is the one non-numeric value, so it is kept aside; every other key parses into
     * one slot of a fixed buffer, which is what keeps this from becoming an eighteen-argument call.
     */
    private static boolean parseControl(String body) {
        int[] value = new int[CTL_KEYS.length];
        boolean[] present = new boolean[CTL_KEYS.length];
        String buffs = null;
        String drops = null;
        int from = 0;
        while (from < body.length()) {
            int end = body.indexOf('\n', from);
            if (end < 0) {
                end = body.length();
            }
            String line = body.substring(from, end);
            from = end + 1;
            if (line.length() == 0) {
                continue;
            }
            int split = line.indexOf('=');
            if (split <= 0) {
                return false;
            }
            String key = line.substring(0, split);
            String text = line.substring(split + 1);
            int slot = -1;
            for (int i = 0; i < CTL_KEYS.length; i++) {
                if (CTL_KEYS[i].equals(key)) {
                    slot = i;
                    break;
                }
            }
            if (slot < 0 || present[slot]) {
                return false;         // a key this jar does not know, or a repeat
            }
            present[slot] = true;
            if (slot == K_BUFFS) {
                buffs = text;
            } else if (slot == K_DROPS) {
                drops = text;
            } else {
                value[slot] = num(text);
            }
        }
        for (int i = 0; i < present.length; i++) {
            if (!present[i]) {
                return false;         // a missing key means the two sides disagree
            }
        }
        return acceptControl(value, buffs, drops);
    }

    /**
     * Range-checks one parsed body and, only if every value is good, applies it.
     *
     * Split from the parser so no partially applied setting can survive a rejection: either every
     * value is accepted together, or none is.
     */
    private static boolean acceptControl(int[] value, String buffs, String drops) {
        if (value[K_V] != CTL_VERSION || buffs == null || buffs.length() != BUFF_SLOTS) {
            return false;
        }
        if (drops == null || drops.length() != MATERIAL_SLOTS) {
            return false;
        }
        if (value[K_MODE] < 0 || value[K_MODE] > 2) {
            return false;
        }
        if (value[K_MAP] < 0 || value[K_MAP] > 255 || value[K_ZONE] < -1 || value[K_ZONE] > 127) {
            return false;
        }
        if (value[K_RADIUS] < 60 || value[K_RADIUS] > 240) {
            return false;
        }
        if (value[K_HP] < 1 || value[K_HP] > 99 || value[K_MP] < 1 || value[K_MP] > 99) {
            return false;
        }
        if (!flag(value[K_REVIVEON])) {
            return false;
        }
        // 1 and 2 only: "off" is `revive.on=0`, not a third mode. Two ways to express the same
        // state is two ways for the tool and the jar to disagree about it.
        if (value[K_REVIVEMODE] < 1 || value[K_REVIVEMODE] > 2) {
            return false;
        }
        // Seconds, and bounded: 300 s is five minutes on the ground, past which the setting is a
        // mistake rather than a choice. Multiplied by TICKS_PER_SECOND below, so the product has to
        // stay well inside an int.
        if (value[K_REVIVEDELAY] < 0 || value[K_REVIVEDELAY] > 300) {
            return false;
        }
        // ---- ENHANCE ----------------------------------------------------------
        if (!flag(value[K_ENHANCE_ON])) {
            return false;
        }
        // 1..15 is the ladder the client itself offers. Past +15 there is no next level to ask
        // for; below +1 the trip would stop before it started. Refused rather than clamped, so
        // the tool and this jar cannot end up meaning two different levels.
        if (value[K_ENHANCE_MAXLV] < 1 || value[K_ENHANCE_MAXLV] > 15) {
            return false;
        }
        // 0..3, the four charms the upgrade dialog offers: none, three-leaf clover, four-leaf
        // clover, smart. A value past them would index outside that list.
        if (value[K_ENHANCE_CHARM] < 0 || value[K_ENHANCE_CHARM] > 3) {
            return false;
        }
        // ---- end ENHANCE ------------------------------------------------------
        // ---- DUNGEON ----------------------------------------------------------
        if (!flag(value[K_DUNGEON_ON])) {
            return false;
        }
        // -1 is unlimited, 1..10 is a finite trip. The ceiling is the client's own combo list,
        // which stops at ten runs; past it there is no count to ask for. Refused rather than
        // clamped, so the tool and this jar cannot end up meaning two different trip lengths.
        if (value[K_DUNGEON_MAX] < -1 || value[K_DUNGEON_MAX] > 10) {
            return false;
        }
        // -1 is no timer, else a half-hour slot of day: 48 of them, 0 = 00:00 through 47 = 23:30.
        if (value[K_DUNGEON_SCHED] < -1 || value[K_DUNGEON_SCHED] > 47) {
            return false;
        }
        // ---- end DUNGEON ------------------------------------------------------
        // Option counts come from the client's own lists: six item ranks, four MP/HP modes, two
        // gold modes (df.java:820). A value past the end would index outside them.
        if (value[K_RANK] < 0 || value[K_RANK] > 5) {
            return false;
        }
        if (value[K_MPHP] < 0 || value[K_MPHP] > 3 || value[K_GOLD] < 0 || value[K_GOLD] > 1) {
            return false;
        }
        // 0 is "any mount"; anything else has to be a template the client's own menu recognises.
        if (value[K_MOUNTID] != 0
                && (value[K_MOUNTID] < MOUNT_ID_MIN || value[K_MOUNTID] > MOUNT_ID_MAX)) {
            return false;
        }
        // Half a spot cannot be an anchor: both axes are known, or neither is.
        if (value[K_X] < 0 != value[K_Y] < 0) {
            return false;
        }
        if (!flag(value[K_HPON]) || !flag(value[K_MPON]) || !flag(value[K_MOUNT]) || !flag(value[K_MEDAL])) {
            return false;
        }
        if (!flag(value[K_DROPSON])) {
            return false;
        }
        // -1 is off; anything else has to be a map id this build can route on. MAP_MAX bounds the
        // adjacency table, so a value past it would index outside it.
        if (value[K_NAVTARGET] < -1 || value[K_NAVTARGET] >= MAP_MAX) {
            return false;
        }
        if (!flag(value[K_RING])) {
            return false;
        }
        if (!flag(value[K_FARM])) {
            return false;
        }
        if (!flag(value[K_DETECT])) {
            return false;
        }
        if (value[K_ZONEMODE] < 0 || value[K_ZONEMODE] > 2) {
            return false;
        }
        if (value[K_ZONEPICK] < 1 || value[K_ZONEPICK] > 99) {
            return false;
        }
        for (int i = 0; i < BUFF_SLOTS; i++) {
            char c = buffs.charAt(i);
            if (c != '0' && c != '1') {
                return false;
            }
        }
        for (int i = 0; i < MATERIAL_SLOTS; i++) {
            char c = drops.charAt(i);
            if (c != '0' && c != '1') {
                return false;
            }
        }

        int prevGoal = goal();
        // A new spot means walk to it, whatever state the fight was in. atkState starts at FIGHTING
        // and only became TO_SPOT after drifting too far, so arming auto for the first time fought
        // wherever the character happened to stand — the recorded spot was never approached at all.
        // Stand mode has to reach the exact recorded place; move mode roams from it.
        boolean newSpot = value[K_MODE] != 0
                && (value[K_MODE] != atkMode
                        || value[K_MAP] != atkMap
                        || value[K_X] != atkX
                        || value[K_Y] != atkY);
        atkMode = value[K_MODE];
        atkMap = value[K_MAP];
        atkZone = value[K_ZONE];
        atkX = value[K_X];
        atkY = value[K_Y];
        if (newSpot) {
            atkState = TO_SPOT;
        }
        atkRadius = value[K_RADIUS];
        atkHpOn = value[K_HPON] == 1;
        atkHpPct = value[K_HP];
        atkMpOn = value[K_MPON] == 1;
        atkMpPct = value[K_MP];
        reviveOn = value[K_REVIVEON] == 1;
        reviveMode = value[K_REVIVEMODE];
        reviveDelay = value[K_REVIVEDELAY];
        // ---- ENHANCE ----------------------------------------------------------
        boolean wantEnhance = value[K_ENHANCE_ON] == 1;
        // The transition, not every read: control() re-reads the file every CTL_EVERY_MS, and
        // resetting on each pass would wipe a phase the trip had already reached. The shape
        // dropsManaged uses above.
        if (wantEnhance != enhanceOn) {
            enhanceOn = wantEnhance;
            if (!wantEnhance) {
                // A half-finished trip was authorised by a switch that is now off, so it stops
                // where it is rather than finishing — travelReset()'s rule.
                enhanceReset();
            }
        }
        enhanceMaxLevel = value[K_ENHANCE_MAXLV];
        enhanceCharmType = value[K_ENHANCE_CHARM];
        // ---- end ENHANCE ------------------------------------------------------
        // ---- DUNGEON ----------------------------------------------------------
        boolean wantDungeon = value[K_DUNGEON_ON] == 1;
        // The transition, not every read: control() re-reads the file every CTL_EVERY_MS, and
        // resetting on each pass would wipe a state the trip had already reached. The shape
        // dropsManaged uses above.
        if (wantDungeon != dungeonEnabled) {
            dungeonEnabled = wantDungeon;
            // Arming starts a fresh trip, so the tally goes with it: a max of three runs left over
            // from the last trip would be a max of three runs already spent, and with the limit
            // already reached dungeonIdle() would re-stop with why 4 on the very next tick — the
            // module permanently dead until the jar restarts. Switching OFF keeps the tally, so the
            // panel can still say how many runs the trip that was just stopped actually did.
            dungeonReset();
            if (wantDungeon) {
                dungeonRuns = 0;
            }
        }
        dungeonMaxRuns = value[K_DUNGEON_MAX];
        dungeonSchedule = value[K_DUNGEON_SCHED];
        // ---- end DUNGEON ------------------------------------------------------
        for (int i = 0; i < BUFF_SLOTS; i++) {
            atkBuff[i] = buffs.charAt(i) == '1';
        }
        itemRank = value[K_RANK];
        itemMpHp = value[K_MPHP];
        itemGold = value[K_GOLD];
        mountOn = value[K_MOUNT] == 1;
        mountId = value[K_MOUNTID];
        itemMedal = value[K_MEDAL] == 1;
        // A change to the desired material states restarts the walk through them: the previous pass
        // may have been half done, and there is no way to know a slot's state without touching it.
        boolean wantDrops = value[K_DROPSON] == 1;
        for (int i = 0; i < MATERIAL_SLOTS; i++) {
            boolean want = drops.charAt(i) == '1';
            if (want != dropWanted[i]) {
                dropWanted[i] = want;
                dropDone[i] = false;
            }
        }
        if (wantDrops != dropsManaged) {
            dropsManaged = wantDrops;
            for (int i = 0; i < MATERIAL_SLOTS; i++) {
                dropDone[i] = false;
            }
            dropSlot = -1;
            dropPhase = 0;
        }
        int zoneMode = value[K_ZONEMODE];
        int zonePick = value[K_ZONEPICK];
        if (zoneMode != atkZoneMode || zonePick != atkZonePick) {
            atkZoneMode = zoneMode;
            atkZonePick = zonePick;
            zonePhase = zoneMode == 0 ? 0 : 1;   // a fresh request, not a repeat of the last one
        }
        // One destination, not two. `atk.travel` used to walk to wherever the spot was saved while
        // `nav.target` walked to a named map, and with both set the walker arrived at the first and was
        // immediately dragged toward the second — the operator saw it reach the right map and leave.
        // A change restarts the route: a half-finished walk toward the old map is not a step toward the
        // new one.
        if (value[K_NAVTARGET] != navTarget) {
            navTarget = value[K_NAVTARGET];
            navDone = false;
        }
        if (goal() != prevGoal) {
            travelReset();
        }
        ringOn = value[K_RING] == 1;
        atkFarmOnArrival = value[K_FARM] == 1;
        // Fires once per press: the tool writes it false again on the next save, and this arms the probe
        // for the next tick rather than reading the scene from inside the settings parser.
        if (value[K_DETECT] == 1) {
            spotArmed = true;
        }
        return true;
    }

    /** Parses a decimal, or a sentinel that fails every range check above. */
    private static int num(String value) {
        try {
            return Integer.parseInt(value.trim());
        } catch (Throwable t) {
            return Integer.MIN_VALUE;
        }
    }

    private static boolean flag(int value) {
        return value == 0 || value == 1;
    }

    // ---- PLAYER --------------------------------------------------------------
    //
    // Pure sensor. Reads native fields, writes one file. No packet, no native write,
    // no exclusive claim, no dialog swallowing.

    /** Snapshot destination, or null to publish nothing. Set by -Dzeus.player.out. */
    private static String outPath = null;
    /** How often the snapshot is written, in ms. */
    private static long writeEveryMs = 1000L;
    /** How often XP is sampled for the rate, in ms. */
    private static long sampleEveryMs = 5000L;
    /** Window the XP rate is measured over, in ms. */
    private static long xpWindowMs = 300000L;

    private static long wroteAt = 0L;
    private static long sampledAt = 0L;

    /**
     * True once the wallet has actually been delivered.
     *
     * Gold and gem initialise to 0 and are only assigned by the inventory packet, so
     * "0" is indistinguishable from "not yet known" by value alone. Latching a flag on
     * the first nonzero reading is the closest the client allows: there is no native
     * flag for it (docs/core/10 §3.2). A genuinely broke account therefore reads as
     * unknown until it next earns anything, which is the safe direction to be wrong in.
     */
    private static boolean walletKnown = false;

    /** XP samples: parallel ring buffers of timestamp, level and permille. */
    private static final int SAMPLE_MAX = 64;
    private static final long[] sampleAt = new long[SAMPLE_MAX];
    private static final int[] sampleLevel = new int[SAMPLE_MAX];
    private static final int[] samplePermille = new int[SAMPLE_MAX];
    private static int sampleCount = 0;
    private static int sampleHead = 0;

    static {
        outPath = System.getProperty("zeus.player.out");
        if (outPath != null && outPath.length() == 0) {
            outPath = null;
        }
        ctlPath = System.getProperty("zeus.ctl.in");
        if (ctlPath != null && ctlPath.length() == 0) {
            ctlPath = null;
        }
        // The trace lives beside the snapshot, so it needs no launch argument of its own.
        String home = null;
        if (outPath != null) {
            int cut = outPath.lastIndexOf('/');
            int back = outPath.lastIndexOf('\\');
            if (back > cut) {
                cut = back;
            }
            home = cut >= 0 ? outPath.substring(0, cut + 1) : "";
        } else if (ctlPath != null) {
            int cut = ctlPath.lastIndexOf('/');
            int back = ctlPath.lastIndexOf('\\');
            if (back > cut) {
                cut = back;
            }
            home = cut >= 0 ? ctlPath.substring(0, cut + 1) : "";
        } else {
            String uh = System.getProperty("user.home");
            if (uh != null && uh.length() > 0) {
                if (!uh.endsWith("/") && !uh.endsWith("\\")) {
                    uh += java.io.File.separator;
                }
                home = uh;
            }
        }
        if (home != null) {
            traceMarkerPath = home + TRACE_MARKER;
            tracePath = home + TRACE_FILE;
            probeMarkerPath = home + PROBE_MARKER;
            spotMarkerPath = home + SPOT_MARKER;
            spotReqPath = home + SPOT_REQ_FILE;
            spotPayloadPath = home + SPOT_RESULT_PAYLOAD_FILE;
            spotReadyPath = home + SPOT_RESULT_READY_FILE;
        }
        writeEveryMs = (long) intProp("zeus.player.writeMs", 1000);
        if (writeEveryMs < 200L) {
            writeEveryMs = 200L;
        }
        xpWindowMs = (long) intProp("zeus.player.xpWindowMs", 300000);
        if (xpWindowMs < 10000L) {
            xpWindowMs = 10000L;
        }
    }

    // ---- ZONE -----------------------------------------------------------------
    //
    // Measured, not inferred (docs/core/12-control-transport.md §10 and §10.1). The zone board is an
    // entity like any other: `cv == 2` with a name containing "Khu". A map carries several, each with
    // its own `cu`, so the name is what identifies them and `cu` is read off whichever one was found.
    // Opening its menu is opcode 23 with that `cu`, and the server answered a probe sent from 678 px
    // away — so this never moves the character.
    //
    // The switch is SENT, not tapped. Every board button is `new bt(caption, 13, index, cn.b())`
    // (er.java:4102-4104), and `bt.a()` reaches `cn.a(13, index)`, whose entire body is
    // `q.a().d((byte) index)` — opcode 51, one byte. So a switch is one packet and the menu is not
    // part of it.
    //
    // What the menu is still needed for: the captions. `cs.v` is the entry count and `cs.o[]` a
    // per-entry decoration, but the zone numbers and the player counts live in the caption text,
    // which `er.V` builds locally and never stores. So the board is opened ONCE per map to read the
    // roster, dismissed immediately, and every switch after that is a bare packet.
    //
    // Three entries are never entered. Zone 2 costs a ticket or gems and the trade zone is not a
    // farming zone — both on the operator's instruction — and the two-hour zone costs the operator
    // something as well.

    /** 0 idle, 1 open the board, 2 waiting for its menu, 3 roster in hand, send the switch. */
    private static int zonePhase = 0;
    private static int zoneWait = 0;
    private static int zoneTries = 0;
    /** Map the last completed switch was for, so arriving somewhere new re-arms it. */
    private static int zoneMapDone = -1;
    /** Zone that costs a ticket or gems, never entered on the operator's behalf. */
    private static final int ZONE_TICKETED = 2;
    /** Map the roster below was read on; any other map means it has to be read again. */
    private static int zoneRosterMap = Integer.MIN_VALUE;
    /** Button index per entry — the one byte opcode 51 carries. */
    private static byte[] zoneRosterSub = null;
    /** Zone number from the caption, or -1 for an entry that carries no number. */
    private static int[] zoneRosterNum = null;
    /** Players in that zone, or -1 when the caption did not say. */
    private static int[] zoneRosterCount = null;
    /** Entries never to enter: zone 2, the trade zone, the two-hour zone. */
    private static boolean[] zoneRosterSkip = null;

    private static void zone() {
        try {
            if (atkZoneMode == 0) {
                zonePhase = 0;
                return;
            }
            // Deliberately NOT ready(): that requires no menu open, and an open board is exactly the
            // state phase 2 waits in. Gating on it here is a deadlock — the menu blocks the module
            // and only the module dismisses the menu.
            if (!inGame() || !sceneReady() || !alive() || captcha()
                    || cn.g == null || fu.q == null) {
                return;
            }
            if (zonePhase != 2 && !noDialog()) {
                return;
            }
            int here = fu.q.d;
            // A zone belongs to one map, so landing on a different one re-arms the switch.
            if (here != zoneMapDone && zonePhase == 0) {
                zonePhase = 1;
                zoneTries = 0;
            }
            // Naming a zone is checkable without opening anything: selecting button `sub` lands on
            // `cs.u == sub`, and the caption for that button reads "khu sub+1".
            if (atkZoneMode == 2 && cs.u == atkZonePick - 1) {
                zonePhase = 0;
                zoneMapDone = here;
                return;
            }
            // Refused here rather than filtered later: the operator named a zone this module will not
            // enter, and quietly going somewhere else would be worse than doing nothing.
            if (atkZoneMode == 2 && atkZonePick == ZONE_TICKETED) {
                zonePhase = 0;
                zoneMapDone = here;
                trace("ZONE refused: khu " + ZONE_TICKETED + " needs a ticket or gems");
                return;
            }
            if (zoneWait > 0) {
                --zoneWait;
                if (zonePhase != 2) {
                    return;
                }
            }
            // A roster already read on this map is enough to switch with: no second open.
            if (zonePhase == 1 && zoneRosterMap == here) {
                zonePhase = 3;
            }
            if (zonePhase == 1) {
                fa board = zoneBoard();
                if (board == null) {
                    zonePhase = 0;              // no board here; nothing to do on this map
                    zoneMapDone = here;
                    trace("ZONE no board on map " + here);
                    return;
                }
                zoneRosterMap = Integer.MIN_VALUE;
                q.a().a((byte) board.cu);
                zonePhase = 2;
                zoneWait = 100;                 // 5 s for the server to answer
                trace("ZONE opened board cu=" + board.cu + " on map " + here);
                return;
            }
            if (zonePhase == 2) {
                if (zoneRosterMap != here) {
                    if (zoneWait > 0) {
                        return;                 // still inside the deadline
                    }
                    if (++zoneTries >= 3) {
                        zonePhase = 0;
                        zoneMapDone = here;
                        trace("ZONE gave up: no menu after 3 tries");
                        return;
                    }
                    zonePhase = 1;
                    return;
                }
                zonePhase = 3;
            }
            if (zonePhase == 3) {
                // The menu was only ever a source of captions, so down it goes before the switch:
                // the operator never has to dismiss a board this module opened.
                zoneCloseMenu();
                zonePhase = 0;
                zoneMapDone = here;
                sendZone();
            }
        } catch (Throwable t) {
            zonePhase = 0;
        }
    }

    /** Dismisses the board menu the way the client's own Back does. */
    private static void zoneCloseMenu() {
        try {
            if (fu.p != null && fu.p.a) {
                fu.p.f();
                fu.m();
            }
        } catch (Throwable t) {
            // A menu that will not close is not worth failing the switch over.
        }
    }

    /**
     * Reads the board roster out of a menu this module asked for.
     *
     * Called from the `fr.a` prologue, where the button list is already assembled. Every fact a
     * switch needs is here and nowhere else: `bt.f` is the byte opcode 51 carries, and the caption
     * carries the zone number and the population.
     */
    private static void zoneRoster(et items) {
        try {
            int count = items == null ? 0 : items.c();
            byte[] sub = new byte[count];
            int[] num = new int[count];
            int[] players = new int[count];
            boolean[] skip = new boolean[count];
            for (int i = 0; i < count; i++) {
                num[i] = -1;
                players[i] = -1;
                Object entry = items.a(i);
                if (!(entry instanceof bt)) {
                    skip[i] = true;
                    continue;
                }
                bt button = (bt) entry;
                String text = norm(button.a);
                sub[i] = button.f;
                num[i] = captionZone(text);
                players[i] = captionCount(text);
                // `e != 13` is not a zone button at all. The rest are zones this module will not
                // enter: the trade zone, the two-hour zone, and the ticketed one.
                skip[i] = button.e != 13
                        || text.indexOf("khu") < 0
                        || text.indexOf("buon") >= 0
                        || text.indexOf("2h") >= 0
                        || num[i] == ZONE_TICKETED;
            }
            zoneRosterSub = sub;
            zoneRosterNum = num;
            zoneRosterCount = players;
            zoneRosterSkip = skip;
            zoneRosterMap = fu.q == null ? Integer.MIN_VALUE : fu.q.d;
            trace("ZONE roster map=" + zoneRosterMap + " entries=" + count
                    + " enterable=" + zoneEnterable());
        } catch (Throwable t) {
            zoneRosterMap = Integer.MIN_VALUE;
        }
    }

    /** How many roster entries this module is willing to enter, for the trace line. */
    private static int zoneEnterable() {
        int n = 0;
        for (int i = 0; i < zoneRosterSkip.length; i++) {
            if (!zoneRosterSkip[i]) {
                ++n;
            }
        }
        return n;
    }

    /**
     * Sends the switch for the wanted zone: one byte, opcode 51.
     *
     * `q.a().d((byte) sub)` is the whole of `cn.a(13, sub)`, which is where `bt.a()` arrives when the
     * operator taps a board button — the same packet, without the menu.
     */
    private static void sendZone() {
        if (zoneRosterSub == null || zoneRosterNum == null) {
            return;
        }
        int at = -1;
        if (atkZoneMode == 2) {
            for (int i = 0; i < zoneRosterNum.length; i++) {
                if (zoneRosterNum[i] == atkZonePick && !zoneRosterSkip[i]) {
                    at = i;
                    break;
                }
            }
            if (at < 0) {
                trace("ZONE khu " + atkZonePick + " is not an enterable entry on this board");
                return;
            }
        } else {
            int fewest = Integer.MAX_VALUE;
            for (int i = 0; i < zoneRosterNum.length; i++) {
                if (zoneRosterSkip[i] || zoneRosterCount[i] < 0) {
                    continue;
                }
                if (zoneRosterCount[i] < fewest) {
                    fewest = zoneRosterCount[i];
                    at = i;
                }
            }
            if (at < 0) {
                trace("ZONE no enterable entry on this board");
                return;
            }
        }
        // Already there: the switch would be a packet that changes nothing.
        if (zoneRosterSub[at] == cs.u) {
            trace("ZONE already in khu " + zoneRosterNum[at]);
            return;
        }
        q.a().d(zoneRosterSub[at]);
        trace("ZONE sent op=51 sub=" + zoneRosterSub[at] + " khu=" + zoneRosterNum[at]
                + " players=" + zoneRosterCount[at] + " from khu " + (cs.u + 1));
    }

    /** The nearest zone board on this map, or null when there is none. */
    private static fa zoneBoard() {
        if (cn.j == null || cn.g == null) {
            return null;
        }
        fa best = null;
        int bestDistance = Integer.MAX_VALUE;
        for (int i = 0; i < cn.j.c(); i++) {
            Object entry = cn.j.a(i);
            if (!(entry instanceof fa)) {
                continue;
            }
            fa candidate = (fa) entry;
            // The name, not the template id: a map carries several boards and their ids differ.
            if (candidate.cv != 2 || candidate.cC == null
                    || norm(candidate.cC).indexOf("khu") < 0) {
                continue;
            }
            int distance = abs(cn.g.aZ - candidate.aZ) + abs(cn.g.ba - candidate.ba);
            if (distance < bestDistance) {
                bestDistance = distance;
                best = candidate;
            }
        }
        return best;
    }

    /** The zone number in a caption like "khu 3 (1)", or -1. */
    private static int captionZone(String text) {
        int at = text.indexOf("khu");
        if (at < 0) {
            return -1;
        }
        return firstNumber(text, at + 3);
    }

    /**
     * The player count in a caption like "khu 2 (1) (2h)-3", or -1.
     *
     * The FIRST parenthesised number: a caption can carry decoration after it, and reading the last
     * one would compare the wrong quantity.
     */
    private static int captionCount(String text) {
        int open = text.indexOf('(');
        if (open < 0) {
            return -1;
        }
        return firstNumber(text, open + 1);
    }

    /** The first run of digits at or after `from`, or -1 when there is none. */
    private static int firstNumber(String text, int from) {
        int i = from;
        while (i < text.length() && (text.charAt(i) < '0' || text.charAt(i) > '9')) {
            if (text.charAt(i) == '(' && i > from) {
                break;              // do not run past the group we were asked about
            }
            ++i;
        }
        int start = i;
        int value = 0;
        while (i < text.length() && text.charAt(i) >= '0' && text.charAt(i) <= '9') {
            value = value * 10 + (text.charAt(i) - '0');
            ++i;
        }
        return i == start ? -1 : value;
    }

    // ---- MATERIAL CLOSE-DROP --------------------------------------------------
    //
    // Two packets per material, not three. Measured from two traced sessions
    // (docs/core/09-module-item.md §12): opcode −30 (125, 125, 0) enters the materials submenu, then
    // opcode −30 (−125, −125, index) flips one material. The sign distinguishes the menu level, not
    // on and off.
    //
    // The third packet the operator's own taps send — opcode −91 sub 5, the board's "Khác" branch —
    // is deliberately NOT sent. It exists to make the server build a panel, and that panel is not an
    // overlay: `er.aE` answers it with `fu.w.a(cn.b())`, and `ev extends p`, whose `a(p)` assigns
    // `fu.a = this`. So the panel BECOMES the active screen, `inGame()` goes false, and every module
    // — including this one — stops. The walk used to send packet 1 and then die, which is exactly
    // what the operator saw: a menu opened and nothing was ever selected.
    //
    // `q.b(short,byte,byte)` writes four bytes and flushes; it reads no client state at all, so the
    // two −30 packets do not need the panel to have been built. Whether the SERVER requires it is a
    // separate question, and the trace answers it: a confirmation dialog means no, it does not.
    //
    // It is a TOGGLE, so there is no way to read the current state without changing it. That is why
    // this only runs when the operator asked for it, and why every flip is verified by reading the
    // server's own confirmation dialog rather than assumed.

    /** Accent-stripped fragments the confirmation dialog uses, one per material, in wire order. */
    private static final String[] DROP_NAMES = {
        "me day trang", "me day vang", "me day tim", "me day xanh",
        "nguyen lieu tinh tu", "lua tinh tu",
    };

    /** Known state per material: 0 unknown, 1 open, 2 closed. Published in the snapshot. */
    private static final int[] dropState = new int[MATERIAL_SLOTS];

    /** -1 none in flight, else the slot being flipped. */
    private static int dropSlot = -1;
    /** 0 idle, 1 waiting for the material menu, 2 waiting for the confirmation. */
    private static int dropPhase = 0;
    private static int dropWait = 0;
    private static int dropTries = 0;
    /** idNPC of the material menu the server sent, or MIN_VALUE while none is in hand. */
    private static int dropMenuNpc = Integer.MIN_VALUE;
    private static int dropMenuId = 0;

    private static void drops() {
        try {
            if (!dropsManaged) {
                dropSlot = -1;
                dropPhase = 0;
                dropMenuNpc = Integer.MIN_VALUE;
                return;
            }
            // Deliberately NOT ready(): that requires no menu open, and the server answers packet 1
            // with a menu of its own (traced: SMENU npc=-125 menu=-125 count=6). Gated on ready(),
            // phase 1 never ran — the module waited for the operator to dismiss its own menu.
            if (!inGame() || !sceneReady() || !alive() || captcha() || cn.g == null) {
                return;
            }
            if (dropPhase == 0 && !noDialog()) {
                return;                         // start clean; never open a menu behind another
            }
            // A wait here is a DEADLINE, not a delay. Phases 1 and 2 have to act on the tick their
            // answer lands: the server's submenu is on screen from the moment it arrives, so
            // returning until the deadline expired is exactly why it stayed up for seconds before
            // closing itself. Only phase 0's pause between slots is a real delay. Same shape ZONE
            // has always had — it exempts the phase that waits for its own menu, and that is why
            // the zone switch never looked slow.
            if (dropWait > 0) {
                --dropWait;
                if (dropPhase == 0) {
                    return;
                }
            }
            switch (dropPhase) {
                case 0: {
                    dropSlot = nextDrop();
                    if (dropSlot < 0) {
                        return;                 // every material confirmed
                    }
                    dropTries = 0;
                    dropMenuNpc = Integer.MIN_VALUE;
                    // Armed BEFORE the packet: the reply is built on the network thread, and a menu
                    // that lands between the socket write and this assignment would not be
                    // recognised as this module's own — it would be shown and then time out.
                    dropPhase = 1;
                    dropWait = 60;              // 3 s for the menu to arrive
                    q.a().b((short) 125, (byte) 125, (byte) 0);
                    return;
                }
                case 1: {
                    if (dropMenuNpc == Integer.MIN_VALUE) {
                        if (dropWait > 0) {
                            return;             // still inside the deadline
                        }
                        if (++dropTries >= 2) {
                            trace("DROP no material menu, stopping");
                            dropsManaged = false;
                            dropPhase = 0;
                            return;
                        }
                        dropPhase = 0;          // ask again
                        return;
                    }
                    // No close call: the menu was swallowed in the builder, so nothing was ever
                    // shown and there is nothing on screen to dismiss.
                    int npc = dropMenuNpc;
                    int menu = dropMenuId;
                    dropMenuNpc = Integer.MIN_VALUE;
                    // Quoted from the menu rather than hardcoded: the pair identifies the submenu the
                    // selection belongs to, and a server that renumbered it would otherwise be sent
                    // an index against the wrong menu.
                    q.a().b((short) npc, (byte) menu, (byte) dropSlot);
                    dropPhase = 2;
                    dropWait = 80;              // 4 s for the confirmation to arrive
                    trace("DROP sent npc=" + npc + " menu=" + menu + " slot=" + dropSlot
                            + " want=" + (dropWanted[dropSlot] ? "closed" : "open"));
                    return;
                }
                default: {
                    readDropDialog();
                }
            }
        } catch (Throwable t) {
            dropPhase = 0;
        }
    }

    /**
     * Takes the material menu the server sent in answer to packet 1, and says whether it took it.
     *
     * Verified by shape and by name: six entries, each naming the material this module expects at
     * that position. A menu failing either check is not answered and not hidden — positions that are
     * not what was measured would close a drop the operator never chose.
     *
     * The labels carry the current state as well as the names, so one menu plans the whole pass:
     * every slot already where the operator wants it is marked done without a packet, and the slot
     * this menu answers for is chosen HERE, from what the server just said, rather than from the
     * guess phase 0 made before the menu existed.
     */
    private static boolean dropMenu(et items, int idMenu, int idNPC) {
        try {
            int count = items == null ? 0 : items.c();
            if (count != MATERIAL_SLOTS) {
                trace("DROP menu has " + count + " entries, expected " + MATERIAL_SLOTS
                        + " — stopping");
                dropsManaged = false;
                dropPhase = 0;
                return false;
            }
            // The labels state the CURRENT state, not just the six names: an entry reading "Đóng rớt
            // X" offers to close X, so X is open right now, and "Mở rớt X" means X is already closed.
            // Measured 2026-09-04 — §12 called this toggle unreadable, and it is not. Reading it is
            // what stops the walk from flipping a material that was already where it was wanted:
            // every slot used to cost two packets, one wrong way and one back.
            for (int i = 0; i < MATERIAL_SLOTS; i++) {
                Object entry = items.a(i);
                String label = entry instanceof bt ? norm(((bt) entry).a) : "";
                if (label.indexOf(DROP_NAMES[i]) < 0) {
                    trace("DROP menu entry " + i + " reads \"" + clean(label)
                            + "\", expected " + DROP_NAMES[i] + " — stopping");
                    dropsManaged = false;
                    dropPhase = 0;
                    return false;
                }
                boolean isClosed;
                if (label.indexOf("dong rot") >= 0) {
                    isClosed = false;       // it offers to close, so it is open
                } else if (label.indexOf("mo rot") >= 0) {
                    isClosed = true;        // it offers to open, so it is closed
                } else {
                    trace("DROP menu entry " + i + " states no direction — stopping");
                    dropsManaged = false;
                    dropPhase = 0;
                    return false;
                }
                dropState[i] = isClosed ? 2 : 1;
                // Already where the operator wants it: nothing to send, and sending would undo it.
                if (isClosed == dropWanted[i]) {
                    dropDone[i] = true;
                }
            }
            // This submenu is open on the server side right now, so it answers for whichever slot
            // still needs a flip. Re-asking with a second packet 1 — which is what picking the slot
            // before the menu arrived used to cost — buys nothing.
            dropSlot = nextDrop();
            if (dropSlot < 0) {
                dropPhase = 0;
                trace("DROP menu read, every material already as asked");
                return true;                // this module's own menu: taken, so never shown
            }
            dropMenuNpc = idNPC;
            dropMenuId = idMenu;
            return true;
        } catch (Throwable t) {
            dropMenuNpc = Integer.MIN_VALUE;
            return false;
        }
    }

    /** The next material whose state is not yet confirmed to be the wanted one. */
    private static int nextDrop() {
        for (int i = 0; i < MATERIAL_SLOTS; i++) {
            if (!dropDone[i]) {
                return i;
            }
        }
        return -1;
    }

    /**
     * Reads the server's confirmation and decides whether the flip landed.
     *
     * The dialog names the material and states the resulting direction, so this checks both: a
     * dialog about a different material means the menu positions are not what was measured, and
     * carrying on would close drops the operator never chose. That stops the whole feature instead.
     */
    private static void readDropDialog() {
        if (fu.s == null) {
            if (dropWait <= 0) {
                if (++dropTries >= 2) {
                    trace("DROP slot=" + dropSlot + " no confirmation, giving up on it");
                    dropDone[dropSlot] = true;      // unknown stays unknown in the snapshot
                    dropPhase = 0;
                    return;
                }
                dropPhase = 0;                      // try this slot once more
            }
            return;
        }
        String text = norm(dialogText(fu.s));
        if (text.indexOf("chuc nang rot") < 0) {
            return;                                 // some other dialog; wait for ours
        }
        boolean closed = text.indexOf("da duoc dong") >= 0;
        boolean opened = text.indexOf("da duoc mo") >= 0;
        int named = -1;
        for (int i = 0; i < MATERIAL_SLOTS; i++) {
            if (text.indexOf(DROP_NAMES[i]) >= 0) {
                named = i;
                break;
            }
        }
        pressOk(fu.s);
        if (named != dropSlot || (!closed && !opened)) {
            trace("DROP slot=" + dropSlot + " confirmation named " + named
                    + " — stopping, the menu order is not what was measured");
            dropsManaged = false;
            dropPhase = 0;
            return;
        }
        dropState[named] = closed ? 2 : 1;
        boolean want = dropWanted[named];
        if (closed == want) {
            dropDone[named] = true;
            dropPhase = 0;
            // Cleared, not left at the confirmation deadline's remainder: that remainder is what made
            // the pass crawl — every settled slot idled out the ~4 s it had not needed.
            dropWait = 0;
            trace("DROP slot=" + named + " now " + (closed ? "closed" : "open"));
            return;
        }
        // Landed the wrong way: it is a toggle, so one more flip is the fix. Once only.
        if (++dropTries >= 2) {
            dropDone[named] = true;
            dropPhase = 0;
            dropWait = 0;
            trace("DROP slot=" + named + " will not settle, leaving it "
                    + (closed ? "closed" : "open"));
            return;
        }
        dropPhase = 0;
        dropWait = 20;
    }

    /** The six known states, as the snapshot publishes them: `-` unknown, `0` open, `1` closed. */
    private static String dropStates() {
        StringBuffer out = new StringBuffer(MATERIAL_SLOTS);
        for (int i = 0; i < MATERIAL_SLOTS; i++) {
            out.append(dropState[i] == 0 ? '-' : (dropState[i] == 2 ? '1' : '0'));
        }
        return out.toString();
    }

    // ---- TRACE ----------------------------------------------------------------
    //
    // A diagnostic recorder, off unless the operator asks for it. Two questions cannot be
    // answered by reading the decompiled client — what packet a native menu actually sends,
    // and what a clickable board on the map really is — so this records the real flow while
    // the operator drives the game by hand.
    //
    // Enabled by the presence of a marker file beside the snapshot, NOT by a property or a
    // control key: no pinned launch argument changes, no control-format bump, and nothing in
    // the product surface. Absent marker means every hook below returns on its first line.

    /** Hard cap on the log. A trace session is minutes long; this is far more than it needs. */
    private static final int TRACE_MAX_BYTES = 2 * 1024 * 1024;


    /** Whether the operator asked for the range overlay. Local only: it draws, it never sends. */
    private static boolean ringOn = false;
    /** When the ring last wrote a trace line, so a 15Hz paint cannot flood the log. */
    private static long lastRingTrace = 0L;
    /** Whether reaching the spot arms the fight, or only parks the character on it. */
    private static boolean atkFarmOnArrival = false;
    private static boolean traceOn = false;
    private static int traceBytes = 0;
    private static long traceCheckedAt = 0L;
    /** Buffered lines, flushed once per tick so a client crash still leaves what it recorded. */
    private static StringBuffer tracePending = new StringBuffer(4096);

    /** Re-reads the marker, so tracing can be turned on and off without restarting the client. */
    private static void traceCheck(long now) {
        if (traceMarkerPath == null || now - traceCheckedAt < CTL_EVERY_MS) {
            return;
        }
        traceCheckedAt = now;
        boolean present;
        try {
            present = new java.io.File(traceMarkerPath).exists();
        } catch (Throwable t) {
            present = false;
        }
        if (present != traceOn) {
            traceOn = present;
            if (traceOn) {
                traceBytes = 0;
                trace("== trace on ==");
            }
        }
        // The probe rides the same 500 ms check — but outside the branch above, because the trace
        // state usually has not changed and an early return there would mean the probe never ran.
        // It fires once per appearance of its marker: it sends a packet, so repeating it every tick
        // would be a flood the operator did not ask for.
        boolean probe;
        try {
            probe = probeMarkerPath != null && new java.io.File(probeMarkerPath).exists();
        } catch (Throwable t) {
            probe = false;
        }
        if (probe && !probeSeen) {
            probeArmed = true;
        }
        probeSeen = probe;
        boolean spot;
        try {
            spot = spotMarkerPath != null && new java.io.File(spotMarkerPath).exists();
        } catch (Throwable t) {
            spot = false;
        }
        if (spot && !spotSeen) {
            spotArmed = true;
        }
        spotSeen = spot;
    }

    private static boolean probeSeen = false;
    private static boolean probeArmed = false;
    private static boolean spotSeen = false;
    private static boolean spotArmed = false;
    /** Which stone the next firing targets, so flipping the marker twice covers a map with two. */
    private static int probeRound = 0;

    /**
     * Sends one teleport-stone interaction from wherever the character happens to be standing.
     *
     * Two questions, one packet. The distance: it logs every `cv == 2` entity with its distance, then
     * sends opcode 23 with the stone's own `cu` — byte for byte what `ez.k()` sends when the operator
     * taps it. A `SMENU` line following in the log means the server does not care how far away the
     * character was, exactly as the zone board turned out (§10.1 of docs/core/12-control-transport.md),
     * and TRAVEL needs no walk to the stone. The destinations: `SMENU` carries the server's own labels
     * plus the idNPC/idMenu pair a selection has to quote, which decides whether the borrowed
     * `MOD03.f322` table of stone-reachable maps is needed at all.
     *
     * A map can carry more than one stone — map 33 bridges two regions — so each firing takes the next
     * one in `cu` order rather than assuming there is only ever one.
     *
     * Nothing here is destructive: the worst case is a menu opening, which is what a tap does. Nothing
     * is selected, so no travel happens; press Back to dismiss it.
     */
    private static void probeStone() {
        try {
            if (!inGame() || cn.g == null || cn.j == null) {
                trace("PROBE skipped: not in the world");
                return;
            }
            fa[] stones = new fa[8];
            int found = 0;
            int listed = 0;
            for (int i = 0; i < cn.j.c(); i++) {
                Object entry = cn.j.a(i);
                if (!(entry instanceof fa)) {
                    continue;
                }
                fa candidate = (fa) entry;
                if (candidate.cv != 2) {
                    continue;
                }
                int distance = abs(cn.g.aZ - candidate.aZ) + abs(cn.g.ba - candidate.ba);
                // Listed in full, not capped at a dozen: the previous run truncated before reaching
                // the entities further down the map, which is how a second stone would be missed.
                if (listed < 40) {
                    ++listed;
                    trace("PROBE near cv=2 cu=" + candidate.cu + " x=" + candidate.aZ + " y="
                            + candidate.ba + " dist=" + distance + " name=" + clean(candidate.cC));
                }
                // Matched by name, not by cu: the four zone boards on one map already proved cu is
                // per-instance, and the stone's cu is what tells its region apart (10/33/55).
                if (found < stones.length && norm(candidate.cC).indexOf("dich chuyen") >= 0) {
                    stones[found++] = candidate;
                }
            }
            trace("PROBE cv=2 total listed=" + listed + " stones=" + found
                    + " map=" + (fu.q == null ? -1 : fu.q.d) + " me=" + cn.g.aZ + "," + cn.g.ba);
            if (found == 0) {
                trace("PROBE no teleport stone on this map");
                return;
            }
            fa stone = stones[probeRound % found];
            ++probeRound;
            int distance = abs(cn.g.aZ - stone.aZ) + abs(cn.g.ba - stone.ba);
            trace("PROBE stone cu=" + stone.cu + " x=" + stone.aZ + " y=" + stone.ba
                    + " dist=" + distance);
            q.a().a((byte) stone.cu);
            trace("PROBE sent op=23 cu=" + stone.cu);
        } catch (Throwable t) {
            trace("PROBE failed");
        }
    }

    /**
     * Dumps every monster in the scene with its SPAWN ANCHOR, and clusters those anchors.
     *
     * The anchor is the fact worth having. `cc`'s constructor sets `F`/`G` from the spawn coordinates
     * the catalogue packet carried and never moves them, while `aZ`/`ba` drift as the monster wanders;
     * `au.java:253` pulls it back to `(F, G)` once it passes 1.5 × `C` (= 60). So a spot's centre is
     * the mean of a cluster of ANCHORS, not of current positions — the anchors stand still.
     *
     * Clustered here rather than in the tool because the raw list is the thing that would be lost: the
     * scene only holds what the server streamed, so the cluster has to be formed while it is in hand.
     * Single-link at 96 px, one and a half wander radii: two monsters whose wander circles overlap
     * belong to the same spot.
     */
    static final class SpotCandidate {
        final int x;
        final int y;
        final int mobCount;
        final int spreadRadius;
        final String mobName;
        final int mobLevel;

        SpotCandidate(int x, int y, int mobCount, int spreadRadius, String mobName, int mobLevel) {
            this.x = x;
            this.y = y;
            this.mobCount = mobCount;
            this.spreadRadius = spreadRadius;
            this.mobName = mobName != null ? mobName : "";
            this.mobLevel = mobLevel;
        }
    }

    private static void probeSpots() {
        try {
            computeSpotCandidates(true);
        } catch (Throwable t) {
            trace("SPOT failed: " + t);
        }
    }

    private static SpotCandidate[] computeSpotCandidates(boolean doTrace) {
        if (!inGame() || cn.g == null || cn.j == null || fu.q == null) {
            if (doTrace) {
                trace("SPOT skipped: not in the world");
            }
            return null;
        }
        int cap = 64;
        int[] ax = new int[cap];
        int[] ay = new int[cap];
        int[] group = new int[cap];
        String[] mobName = new String[cap];
        int[] mobLevel = new int[cap];
        int found = 0;
        for (int i = 0; i < cn.j.c() && found < cap; i++) {
            Object entry = cn.j.a(i);
            if (!(entry instanceof au)) {
                continue;
            }
            au mob = (au) entry;
            if (mob.cv != 1) {
                continue;
            }
            ax[found] = mob.F;
            ay[found] = mob.G;
            group[found] = -1;
            mobName[found] = mob.cC;
            mobLevel[found] = mob.bz;
            if (doTrace) {
                trace("SPOT mob cu=" + mob.cu + " lv=" + mob.bz + " hp=" + mob.bt
                        + " anchor=" + mob.F + "," + mob.G
                        + " at=" + mob.aZ + "," + mob.ba
                        + " drift=" + (abs(mob.aZ - mob.F) + abs(mob.ba - mob.G))
                        + " name=" + clean(mob.cC));
            }
            ++found;
        }
        if (doTrace) {
            trace("SPOT map=" + fu.q.d + " khu=" + (cs.u + 1) + " me=" + cn.g.aZ + "," + cn.g.ba
                    + " monsters=" + found);
        }
        if (found == 0) {
            return new SpotCandidate[0];
        }
        int groups = 0;
        for (int i = 0; i < found; i++) {
            if (group[i] < 0) {
                group[i] = groups++;
            }
            for (int j = i + 1; j < found; j++) {
                if (abs(ax[i] - ax[j]) + abs(ay[i] - ay[j]) > 96) {
                    continue;
                }
                if (group[j] < 0) {
                    group[j] = group[i];
                    continue;
                }
                if (group[j] != group[i]) {
                    int from = group[j];
                    int into = group[i];
                    for (int k = 0; k < found; k++) {
                        if (group[k] == from) {
                            group[k] = into;
                        }
                    }
                }
            }
        }
        int validCount = 0;
        for (int g = 0; g < groups; g++) {
            for (int i = 0; i < found; i++) {
                if (group[i] == g) {
                    ++validCount;
                    break;
                }
            }
        }
        SpotCandidate[] result = new SpotCandidate[validCount];
        int outIdx = 0;
        for (int g = 0; g < groups; g++) {
            int count = 0;
            int sumX = 0;
            int sumY = 0;
            int spread = 0;
            for (int i = 0; i < found; i++) {
                if (group[i] != g) {
                    continue;
                }
                ++count;
                sumX += ax[i];
                sumY += ay[i];
            }
            if (count == 0) {
                continue;
            }
            int cx = sumX / count;
            int cy = sumY / count;
            for (int i = 0; i < found; i++) {
                if (group[i] != g) {
                    continue;
                }
                int reach = abs(ax[i] - cx) + abs(ay[i] - cy);
                if (reach > spread) {
                    spread = reach;
                }
            }
            String name = null;
            int best = 0;
            int level = 0;
            for (int i = 0; i < found; i++) {
                if (group[i] != g || mobName[i] == null) {
                    continue;
                }
                int same = 0;
                for (int j = 0; j < found; j++) {
                    if (group[j] == g && mobName[i].equals(mobName[j])) {
                        ++same;
                    }
                }
                if (same > best) {
                    best = same;
                    name = mobName[i];
                    level = mobLevel[i];
                }
            }
            if (doTrace) {
                trace("SPOT cluster mobs=" + count + " centre=" + cx + "," + cy
                        + " spread=" + spread
                        + " fromMe=" + (abs(cn.g.aZ - cx) + abs(cn.g.ba - cy))
                        + " name=" + (name == null ? "?" : clean(name))
                        + " lv=" + level
                        + " same=" + best);
            }
            result[outIdx++] = new SpotCandidate(cx, cy, count, spread, name, level);
        }
        return result;
    }

    private static String parseScanId(String text) {
        if (text == null) {
            return null;
        }
        int idx = text.indexOf("\"scan_id\"");
        if (idx < 0) {
            return null;
        }
        int colon = text.indexOf(':', idx + 9);
        if (colon < 0) {
            return null;
        }
        int startQuote = text.indexOf('"', colon + 1);
        if (startQuote < 0) {
            return null;
        }
        int endQuote = text.indexOf('"', startQuote + 1);
        if (endQuote < 0) {
            return null;
        }
        String scanId = text.substring(startQuote + 1, endQuote).trim();
        return scanId.length() > 0 ? scanId : null;
    }

    private static void escapeJsonString(String value, StringBuffer out) {
        if (value == null) {
            out.append("\"\"");
            return;
        }
        out.append('"');
        int len = value.length();
        for (int i = 0; i < len; i++) {
            char c = value.charAt(i);
            switch (c) {
                case '"':
                    out.append("\\\"");
                    break;
                case '\\':
                    out.append("\\\\");
                    break;
                case '\b':
                    out.append("\\b");
                    break;
                case '\f':
                    out.append("\\f");
                    break;
                case '\n':
                    out.append("\\n");
                    break;
                case '\r':
                    out.append("\\r");
                    break;
                case '\t':
                    out.append("\\t");
                    break;
                default:
                    if (c < 0x20) {
                        out.append("\\u");
                        String hex = Integer.toHexString(c);
                        for (int k = hex.length(); k < 4; k++) {
                            out.append('0');
                        }
                        out.append(hex);
                    } else {
                        out.append(c);
                    }
                    break;
            }
        }
        out.append('"');
    }

    private static String formatSpotResultJson(String scanId, int mapId, int capturedZone, SpotCandidate[] candidates) {
        StringBuffer json = new StringBuffer(256 + (candidates != null ? candidates.length * 128 : 0));
        json.append("{\n");
        json.append("  \"scan_id\": ");
        escapeJsonString(scanId, json);
        json.append(",\n");
        json.append("  \"map_id\": ").append(mapId).append(",\n");
        json.append("  \"captured_zone\": ").append(capturedZone).append(",\n");
        json.append("  \"candidates\": [");
        if (candidates != null && candidates.length > 0) {
            for (int i = 0; i < candidates.length; i++) {
                if (i > 0) {
                    json.append(",");
                }
                json.append("\n    {\n");
                json.append("      \"x\": ").append(candidates[i].x).append(",\n");
                json.append("      \"y\": ").append(candidates[i].y).append(",\n");
                json.append("      \"mob_count\": ").append(candidates[i].mobCount).append(",\n");
                json.append("      \"spread_radius\": ").append(candidates[i].spreadRadius).append(",\n");
                json.append("      \"mob_name\": ");
                escapeJsonString(candidates[i].mobName, json);
                json.append(",\n");
                json.append("      \"mob_level\": ").append(candidates[i].mobLevel).append("\n");
                json.append("    }");
            }
            json.append("\n  ");
        }
        json.append("]\n");
        json.append("}\n");
        return json.toString();
    }

    private static String readSmallFile(String path) {
        if (path == null) {
            return null;
        }
        java.io.File f = new java.io.File(path);
        if (!f.exists() || f.length() == 0) {
            return null;
        }
        int len = (int) f.length();
        if (len > 4096) {
            len = 4096;
        }
        java.io.FileInputStream fis = null;
        try {
            fis = new java.io.FileInputStream(f);
            byte[] buf = new byte[len];
            int read = 0;
            while (read < len) {
                int r = fis.read(buf, read, len - read);
                if (r < 0) {
                    break;
                }
                read += r;
            }
            return new String(buf, 0, read, "UTF-8");
        } catch (Throwable t) {
            return null;
        } finally {
            if (fis != null) {
                try {
                    fis.close();
                } catch (Throwable ignored) {
                }
            }
        }
    }

    private static String lastSpotScanId = null;
    private static long spotReqCheckedAt = 0L;

    private static void spotSidecarTick(long now) {
        if (spotReqPath == null) {
            return;
        }
        if (now - spotReqCheckedAt < 200L) {
            return;
        }
        spotReqCheckedAt = now;

        java.io.File reqFile = new java.io.File(spotReqPath);
        if (!reqFile.exists()) {
            return;
        }

        String reqText = readSmallFile(spotReqPath);
        if (reqText == null) {
            return;
        }

        String scanId = parseScanId(reqText);
        if (scanId == null) {
            trace("SPOT sidecar req unparsable");
            return;
        }

        if (scanId.equals(lastSpotScanId)) {
            return;
        }

        java.io.File readyFile = new java.io.File(spotReadyPath);
        java.io.File tmpFile = new java.io.File(spotPayloadPath);

        if (readyFile.exists() && tmpFile.exists()) {
            String existingPayload = readSmallFile(spotPayloadPath);
            if (existingPayload != null && scanId.equals(parseScanId(existingPayload))) {
                lastSpotScanId = scanId;
                return;
            }
        }

        if (!inGame() || cn.g == null || cn.j == null || fu.q == null) {
            return;
        }

        try {
            if (readyFile.exists()) {
                readyFile.delete();
            }
            if (tmpFile.exists()) {
                tmpFile.delete();
            }
        } catch (Throwable ignored) {
        }

        SpotCandidate[] candidates = computeSpotCandidates(traceOn);
        if (candidates == null) {
            return;
        }

        int mapId = fu.q.d & 0xFF;
        int capturedZone = cs.u >= 0 ? (cs.u + 1) : 0;
        String jsonPayload = formatSpotResultJson(scanId, mapId, capturedZone, candidates);

        java.io.FileOutputStream fos = null;
        boolean writeOk = false;
        try {
            fos = new java.io.FileOutputStream(tmpFile);
            fos.write(jsonPayload.getBytes("UTF-8"));
            fos.flush();
            fos.close();
            fos = null;
            writeOk = true;
        } catch (Throwable t) {
            trace("SPOT sidecar payload write failed: " + t);
            try {
                if (tmpFile.exists()) {
                    tmpFile.delete();
                }
            } catch (Throwable ignored) {
            }
        } finally {
            if (fos != null) {
                try {
                    fos.close();
                } catch (Throwable ignored) {
                }
            }
        }

        if (!writeOk) {
            return;
        }

        java.io.FileOutputStream readyFos = null;
        try {
            readyFos = new java.io.FileOutputStream(readyFile);
            readyFos.write('\n');
            readyFos.flush();
            readyFos.close();
            readyFos = null;
            lastSpotScanId = scanId;
            trace("SPOT sidecar published scan=" + scanId + " map=" + mapId + " zone=" + capturedZone + " candidates=" + candidates.length);
        } catch (Throwable t) {
            trace("SPOT sidecar ready marker failed: " + t);
            try {
                if (readyFile.exists()) {
                    readyFile.delete();
                }
            } catch (Throwable ignored) {
            }
        } finally {
            if (readyFos != null) {
                try {
                    readyFos.close();
                } catch (Throwable ignored) {
                }
            }
        }
    }

    private static int abs(int value) {
        return value < 0 ? -value : value;
    }

    /** Appends one line, dropping everything once the cap is reached. */
    private static void trace(String line) {
        if (!traceOn || tracePath == null || traceBytes >= TRACE_MAX_BYTES || line == null) {
            return;
        }
        traceBytes += line.length() + 1;
        tracePending.append(line).append('\n');
    }

    /** Writes whatever the tick recorded. Append, not replace: the log is a history. */
    private static void traceFlush() {
        if (tracePending.length() == 0) {
            return;
        }
        String body = tracePending.toString();
        tracePending.setLength(0);
        java.io.OutputStream stream = null;
        try {
            stream = new java.io.FileOutputStream(tracePath, true);
            stream.write(body.getBytes("UTF-8"));
            stream.flush();
        } catch (Throwable t) {
            // An unwritable log must never disturb the client.
        } finally {
            if (stream != null) {
                try {
                    stream.close();
                } catch (Throwable t) {
                    // nothing useful to do on a failed close
                }
            }
        }
    }

    /**
     * Draws the attack-range ring around the character, when the operator asked for it.
     *
     * Called from every RETURN of `cn.a(bx)`, the world paint, not from its first line. Two reasons, and
     * the first cost two rounds of "nothing appears": a prologue runs before `bx2.a(-p.d.a, -p.d.b)`,
     * so the context is still in SCREEN space and world coordinates land hundreds of pixels off a
     * 240x320 display; and it also runs before `fu.q.a(bx2)` paints the map, which would then cover
     * whatever did land. At the epilogue the translation is in force and the map is already down.
     *
     * Why a ring at all: every threshold this tool works in is a distance in world pixels, and there
     * is nothing on screen to judge one against. `atk.radius` is the number the operator sets and
     * cannot see; STAND_DRIFT and MOVE_DRIFT are the ones that decide when the walker gives up and
     * goes back. Drawn, they are obvious; as numbers they are guesses.
     *
     * Traced, not guessed at: `paint` runs 15+ times a second, so it records one line per second at
     * most, and only while tracing is on. Three rounds of "nothing appears" were spent reasoning about
     * a transform nobody had measured; the numbers cost one line and end the argument.
     */
    public static void paint(bx canvas) {
        if (!ringOn || canvas == null) {
            return;
        }
        try {
            if (cn.g == null || fu.a != fu.c) {
                return;             // not on the world screen; nothing to anchor to
            }
            if (p.d == null) {
                return;             // no camera yet; there is no screen position to draw at
            }
            // Where a world coordinate has to be DRAWN so that it LANDS where the client's own
            // entities land. Two separate facts, and conflating them is what put the ring off screen
            // for three rounds:
            //
            //   1. The client draws an entity at its raw world coordinate (`cn.java:896`,
            //      `bx2.a(fe2.a, ..., this.aZ, this.ba, 33)`), because `cn.a(bx)` translates by
            //      `-p.d` first (`cn.java:548`). So "on screen at the character" means the pixel
            //      `world - camera`.
            //   2. At the RETURN the translation is no longer just `-p.d`. `bx2.a(bu, bv)` added the
            //      screen shake and the HUD block added `bx2.a(fu.X - fu.r.a * ey.c - 3, ...)` and
            //      never undid it.
            //
            // Drawing at `C` puts ink at `C + T`, where `T` is whatever is in force. Wanting
            // `C + T == world - camera` gives `C = world - camera - T`. The old code drew at
            // `world - T`, which is that answer plus the camera: correct only while the camera sat at
            // the origin, which is the top-left corner of the map and nowhere a character ever farms.
            int shiftX = -canvas.a() - p.d.a;
            int shiftY = -canvas.b() - p.d.b;
            int x = cn.g.aZ + shiftX;
            int y = cn.g.ba + shiftY;
            // A real circle, not the Manhattan diamond this drew first. The diamond was honest about
            // the client's own reach test and useless for the job: judging which monster is inside the
            // reach, and picking one to aim at, needs a shape the eye reads as a distance.
            // What the transform actually resolved to, once a second: `t` is the translation in force,
            // `cam` the camera, `world` the character, `at` where the ring is being drawn, and `ink`
            // where that lands on the 240x320 screen. `ink` inside the screen and on the character is
            // the whole claim this feature makes.
            long now = System.currentTimeMillis();
            if (traceOn && now - lastRingTrace >= 1000L) {
                lastRingTrace = now;
                trace("RING t=" + canvas.a() + ',' + canvas.b()
                        + " cam=" + p.d.a + ',' + p.d.b
                        + " world=" + cn.g.aZ + ',' + cn.g.ba
                        + " at=" + x + ',' + y
                        + " ink=" + (x + canvas.a()) + ',' + (y + canvas.b())
                        + " screen=" + fu.X + 'x' + fu.Y
                        + " r=" + atkRadius);
            }
            canvas.a(0x33FF33);
            circle(canvas, x, y, atkRadius);
            // Who the client is aiming at. `cn.i` is its own focus, so this marks the same entity the
            // next attack will reach for rather than the tool's guess at one.
            fa aim = cn.i;
            if (aim != null) {
                int aimX = aim.aZ + shiftX;
                int aimY = aim.ba + shiftY;
                // Measured in world coordinates, drawn in translated ones: the reach test is the
                // client's own and must not be affected by where the HUD left the origin.
                boolean reachable = abs(aim.aZ - cn.g.aZ) + abs(aim.ba - cn.g.ba) <= atkRadius;
                // Colour carries the one fact worth knowing about the aim: whether it is close enough
                // to be hit. Naming it in a trace would mean reading a log while fighting.
                canvas.a(reachable ? 0xFFFF33 : 0xFF3333);
                circle(canvas, aimX, aimY, 14);
                canvas.a(aimX - 8, aimY, aimX + 8, aimY);
                canvas.a(aimX, aimY - 8, aimX, aimY + 8);
                // And the line to it, which is what makes a target legible in a crowd.
                canvas.a(x, y, aimX, aimY);
            }
            if (atkMode != 0 && atkX >= 0 && atkY >= 0) {
                // The leash, drawn on the spot rather than the character: this is the distance the
                // walker measures from where it was told to stand, and seeing it on the character
                // would put it in the wrong place entirely.
                int spotX = atkX + shiftX;
                int spotY = atkY + shiftY;
                canvas.a(0xFFAA00);
                circle(canvas, spotX, spotY, atkMode == 1 ? STAND_DRIFT : MOVE_DRIFT);
                // And the spot itself, because a leash with no centre is hard to read.
                canvas.a(0xFFFFFF);
                canvas.a(spotX - 4, spotY, spotX + 4, spotY);
                canvas.a(spotX, spotY - 4, spotX, spotY + 4);
            }
        } catch (Throwable t) {
            // An overlay is never worth breaking the client's paint over: one bad frame beats none.
        }
    }

    /**
     * Draws a circle of `reach` world pixels around a point.
     *
     * Eight-way symmetry from a midpoint walk: one octant is computed and the other seven are its
     * mirrors, so a circle costs `reach / 1.4` iterations and no trigonometry. J2ME has no float worth
     * using here and `drawArc` would need a bounding box in screen space, which is not what this has.
     */
    private static void circle(bx canvas, int cx, int cy, int reach) {
        if (reach <= 0) {
            return;
        }
        int dx = reach;
        int dy = 0;
        int error = 1 - reach;
        while (dx >= dy) {
            plot8(canvas, cx, cy, dx, dy);
            ++dy;
            if (error < 0) {
                error += 2 * dy + 1;
            } else {
                --dx;
                error += 2 * (dy - dx) + 1;
            }
        }
    }

    /** Plots one octant's point into all eight, as single-pixel lines. */
    private static void plot8(bx canvas, int cx, int cy, int dx, int dy) {
        dot(canvas, cx + dx, cy + dy);
        dot(canvas, cx + dy, cy + dx);
        dot(canvas, cx - dy, cy + dx);
        dot(canvas, cx - dx, cy + dy);
        dot(canvas, cx - dx, cy - dy);
        dot(canvas, cx - dy, cy - dx);
        dot(canvas, cx + dy, cy - dx);
        dot(canvas, cx + dx, cy - dy);
    }

    /** One pixel, as the shortest line `bx` can draw: it exposes no point primitive. */
    private static void dot(bx canvas, int x, int y) {
        canvas.a(x, y, x, y);
    }
    // ---- POTATO --------------------------------------------------------------
    //
    // Paint, decoupled from the tick. Merged from `mod/potato/src/POTATO.java`, which is
    // where the measurement and the per-layer safety arguments were made; the entry points
    // keep their names so the bytecode half of A3.1 stays a one-word owner change per site
    // (`PatchCanvas` folds com/silverknight/a.run()'s repaint+serviceRepaints pair into one
    // `Zeus.doRepaint(Canvas)`, `PatchLayers` injects `Zeus.skipLayer(bit)` into ey.a(bx)
    // and br.a(bx), and the recompiled bx calls `Zeus.countDraw()` on every primitive that
    // reaches Graphics).
    //
    // Vanilla `com.silverknight.a.run()` paints on every 40 ms iteration, so the only way to
    // cut render cost was to lengthen the period — which also slowed game logic, and logic is
    // what faces the server. Here the 25 Hz tick is untouched and only painting is skipped.
    // Sends no packet, reads no packet, draws no RNG, and is unreachable from any network
    // handler.
    //
    // This is the THIRD transport and it is deliberately not part of control v13: the paint
    // mode is per-tab and set by the launcher, so it rides on `-D` properties and its own
    // polled file instead of on CTL_KEYS, and the key count stays 35. `zeus-control.txt` is
    // fail-closed (an unparsable file turns every module off, because a wrong farm spot is
    // real damage); `potato.ctl` is fail-safe (an unparsable file keeps the mode already in
    // force, because a wrong paint rate is only pixels). Merging the two files would mean
    // bumping CTL_VERSION for something that is not game logic — WIRE-CONTRACT.md §7.

    /** paint 1 of every N ticks; 1 = vanilla, 0 = never paint. -Dpotato.paintEvery. */
    public static int paintEvery = 1;

    /**
     * Keep painting in states whose logic lives in the draw path (see paintMustRun).
     *
     * A measurement-only escape hatch: with no login there is no game scene, so the guard
     * would force paint on every tick and hide the effect of paintEvery. Never ship a tab
     * with the guard off — RUNTIME-SPEC A3.3, and without it a hidden tab stalls a cutscene.
     */
    public static boolean paintGuard = true;

    /** ticks observed (one per canvas loop iteration). */
    public static long paintTicks = 0L;
    /** ticks that actually painted. */
    public static long paintFrames = 0L;
    /** bx draw calls, summed across every primitive. */
    public static long paintDraws = 0L;

    /** report interval in ms; 0 disables reporting. -Dpotato.reportMs. */
    public static long paintReportMs = 0L;
    private static long paintDrawsAtReport = 0L;
    private static long paintTicksAtReport = 0L;
    private static long paintFramesAtReport = 0L;
    private static long paintReportAt = 0L;

    /*
     * Runtime control. The system property alone fixes the mode at launch, but "hide this
     * tab" is a checkbox: it has to change while the tab runs. A polled file is used rather
     * than a listening socket because a socket inside the client would be an unauthenticated
     * endpoint any local process could drive, and file permissions already express "who may
     * switch this tab".
     */
    /** Paint control file, or null to leave the mode fixed at its launch value. -Dpotato.ctl. */
    private static String potatoCtlPath = null;
    /** How often the paint control file is consulted, in ms. -Dpotato.ctlEveryMs. */
    private static long potatoCtlEveryMs = 1000L;
    private static long potatoCtlPolledAt = 0L;
    private static long potatoCtlStamp = -1L;
    /** Times the paint control file changed the mode; surfaced in the report. */
    public static long potatoCtlApplied = 0L;

    /*
     * Layer gating. Independent of paintEvery: paintEvery decides how often the whole frame
     * is drawn, layerMask decides what a drawn frame contains. Both are needed — a visible
     * tab still has to be watchable, so it cannot use a high paintEvery, and dropping the
     * minimap and effects is the part of the frame a player does not miss.
     *
     * A set bit SKIPS that layer, so 0 is vanilla. Bits are assigned in
     * mod/potato/tools/PatchLayers.java, which injects the call site:
     *   1 = ey.a(bx)  minimap
     *   2 = br.a(bx)  effects
     */
    public static int layerMask = 0;
    /** Layer draws skipped; surfaced in the report so gating is visible. */
    public static long layerSkips = 0L;
    private static long layerSkipsAtReport = 0L;

    /*
     * Its own initializer rather than a few lines added to the block above, for the reason
     * recorded at the trace paths: static field initializers run at their own position in the
     * file, so a `= null` declared here would execute AFTER an earlier block had filled it in
     * and silently undo the read. Everything this module owns is declared above this point.
     */
    static {
        paintEvery = intProp("potato.paintEvery", 1);
        paintReportMs = (long) intProp("potato.reportMs", 0);
        paintGuard = intProp("potato.guard", 1) != 0;
        if (paintEvery < 0) {
            paintEvery = 0;
        }
        potatoCtlPath = System.getProperty("potato.ctl");
        if (potatoCtlPath != null && potatoCtlPath.length() == 0) {
            potatoCtlPath = null;
        }
        potatoCtlEveryMs = (long) intProp("potato.ctlEveryMs", 1000);
        if (potatoCtlEveryMs < 100L) {
            potatoCtlEveryMs = 100L;
        }
        layerMask = intProp("potato.layerMask", 0);
        if (layerMask < 0) {
            layerMask = 0;
        }
    }

    /**
     * Replaces `this.repaint(); this.serviceRepaints();` in the canvas loop. Called once per
     * tick whether or not it paints, so it is also the tick counter — and so it is the only
     * hook that still runs at paintEvery 0, which is what lets a hidden tab be shown again.
     */
    public static void doRepaint(javax.microedition.lcdui.Canvas canvas) {
        pollPotatoControl(dx.a());
        boolean painted = shouldPaint();
        if (painted) {
            canvas.repaint();
            canvas.serviceRepaints();
        }
        ++paintTicks;
        if (painted) {
            ++paintFrames;
        }
        if (paintReportMs > 0L) {
            paintReport();
        }
    }

    /**
     * Whether the caller's draw layer is gated off. Called from the prologue PatchLayers
     * injects into each layer method; returning true makes that method return before drawing
     * anything.
     *
     * Only layers whose bodies were read and found free of gameplay side effects get a bit —
     * see mod/potato/tools/PatchLayers.java for the per-layer argument.
     */
    public static boolean skipLayer(int bit) {
        if ((layerMask & bit) == 0) {
            return false;
        }
        ++layerSkips;
        return true;
    }

    /** Called by bx on every primitive that reaches Graphics. */
    public static void countDraw() {
        ++paintDraws;
    }

    /**
     * Whether this tick paints.
     *
     * Guard: some vanilla logic lives inside the draw path, so paint cannot be dropped
     * unconditionally — not even at paintEvery 0, which is why the guard is checked before
     * the never-paint case. cf.i(bx) advances the map-19/67 cutscene and ends it through
     * n.b().c(); eq.a(bx) advances character-select animation. Both run only from a draw
     * call, so those states keep painting regardless of paintEvery. Verified in
     * mod/src_decomp/cf.java:1858 and mod/src_decomp/eq.java:158.
     */
    public static boolean shouldPaint() {
        if (paintEvery == 1) {
            return true;                    // vanilla behaviour
        }
        if (paintGuard && paintMustRun()) {
            return true;
        }
        if (paintEvery == 0) {
            return false;                   // hidden tab: paint nothing
        }
        return paintTicks % (long) paintEvery == 0L;
    }

    /** The states that must paint: anything but the game scene, plus the two cutscene maps. */
    private static boolean paintMustRun() {
        try {
            if (!inGame()) {
                return true;    // login, character select, or no screen at all yet
            }
            if (fu.q != null && (fu.q.d == 19 || fu.q.d == 67)) {
                return true;    // cutscene advanced from cf.i(bx)
            }
        } catch (Throwable t) {
            return true;        // never trade a stall for a saved frame
        }
        return false;
    }

    /**
     * Consults the paint control file when due. Content is one or two integers:
     * "<paintEvery> [layerMask]" — e.g. "10 3" to hide the tab with both layers dropped, or
     * "1" to return to vanilla and keep the mask.
     *
     * lastModified gates the read, so a steady state costs one stat per second and no parse.
     * Every failure keeps the current mode: a launcher that truncates the file mid-write must
     * not be able to stall the client. This is the fail-safe half of WIRE-CONTRACT §7 —
     * `read()` returns null on an absent, empty or short file, and null here means "change
     * nothing", never "reset to a default" the way control() means it.
     */
    private static void pollPotatoControl(long now) {
        if (potatoCtlPath == null || now - potatoCtlPolledAt < potatoCtlEveryMs) {
            return;
        }
        potatoCtlPolledAt = now;
        try {
            java.io.File file = new java.io.File(potatoCtlPath);
            long stamp = file.lastModified();
            if (stamp == 0L || stamp == potatoCtlStamp) {
                return;         // absent, or unchanged since the last read
            }
            potatoCtlStamp = stamp;
            int[] want = new int[2];
            int found = ctlInts(read(potatoCtlPath), want);
            if (found >= 1 && want[0] != paintEvery) {
                paintEvery = want[0];
                ++potatoCtlApplied;
            }
            if (found >= 2 && want[1] != layerMask) {
                layerMask = want[1];
                ++potatoCtlApplied;
            }
        } catch (Throwable t) {
            // unreadable or absent: keep the mode we already have
        }
    }

    /**
     * Parses up to out.length non-negative integers from a control body, in order, and
     * returns how many were found. A partially written file yields fewer values and the
     * caller applies only those, so a racing write can never install a half-read number.
     *
     * Digits only, by construction: a '-' is skipped like any other separator, so the file
     * cannot express a negative paintEvery and the launch-time clamp stays the only one
     * needed.
     */
    private static int ctlInts(String body, int[] out) {
        if (body == null) {
            return 0;
        }
        int found = 0;
        int at = 0;
        int length = body.length();
        while (at < length && found < out.length) {
            char c = body.charAt(at);
            if (c < '0' || c > '9') {
                ++at;
                continue;
            }
            int value = 0;
            int digits = 0;
            while (at < length) {
                c = body.charAt(at);
                if (c < '0' || c > '9') {
                    break;
                }
                value = value * 10 + (c - '0');
                ++at;
                if (++digits > 6) {
                    return found;       // implausible, keep what parsed
                }
            }
            out[found++] = value;
        }
        return found;
    }

    /**
     * One line per paintReportMs, on stdout rather than through trace(): the numbers exist to
     * compare configurations with no display and no login, and mod/potato/bench.sh greps the
     * JVM log for exactly this prefix. The tokens and their order are that bench contract, so
     * they stay spelled as they were — RUNTIME-SPEC §4.7 reads this line, not a snapshot key,
     * which is also why nothing here touches the 48-key snapshot.
     */
    private static void paintReport() {
        long now = dx.a();
        if (paintReportAt == 0L) {
            paintReportAt = now;
            return;
        }
        long elapsed = now - paintReportAt;
        if (elapsed < paintReportMs) {
            return;
        }
        System.out.println("POTATO tps=" + ((paintTicks - paintTicksAtReport) * 1000L / elapsed)
                + " fps=" + ((paintFrames - paintFramesAtReport) * 1000L / elapsed)
                + " draws/s=" + ((paintDraws - paintDrawsAtReport) * 1000L / elapsed)
                + " paintEvery=" + paintEvery
                + " guard=" + (paintGuard ? 1 : 0)
                + " ctl=" + potatoCtlApplied
                + " mask=" + layerMask
                + " skips/s=" + ((layerSkips - layerSkipsAtReport) * 1000L / elapsed)
                + " screen=" + paintScreen());
        paintReportAt = now;
        paintTicksAtReport = paintTicks;
        paintFramesAtReport = paintFrames;
        paintDrawsAtReport = paintDraws;
        layerSkipsAtReport = layerSkips;
    }

    /** Coarse screen label, so the report is readable with no display attached. */
    private static String paintScreen() {
        try {
            if (inGame()) {
                return "game";
            }
            if (fu.a == fu.b) {
                return "login";
            }
            return "other";
        } catch (Throwable t) {
            return "?";
        }
    }
    // ---- end POTATO ----------------------------------------------------------

    /**
     * Records one outbound packet: opcode and payload.
     *
     * Called from a prologue injected into `ef.b()`, which is the single choke point every
     * sender funnels through (`q` extends `ef`, and every `q` method ends in `this.b()`).
     * At that moment the payload is complete, because every write happens before the send.
     */
    public static void sent(ep packet) {
        if (!traceOn || packet == null) {
            return;
        }
        try {
            byte[] payload = packet.a();
            StringBuffer line = new StringBuffer(64);
            line.append("SEND op=").append(packet.a).append(" len=")
                    .append(payload == null ? 0 : payload.length).append(' ');
            if (payload != null) {
                // Bounded: a long packet is a template dump, and its tail says nothing useful.
                int limit = payload.length < 48 ? payload.length : 48;
                for (int i = 0; i < limit; i++) {
                    int value = payload[i] & 0xff;
                    line.append(HEX.charAt(value >> 4)).append(HEX.charAt(value & 0xf));
                }
                if (payload.length > limit) {
                    line.append("..");
                }
            }
            trace(line.toString());
        } catch (Throwable t) {
            // A trace must never break a send.
        }
    }

    private static final String HEX = "0123456789abcdef";

    /**
     * Records one menu the client is about to show, and says whether a module took it.
     *
     * Called from a prologue injected into `fr.a(et,int,String,boolean,et)`, the menu builder. The
     * button's own command id travels with its caption, which is what lets a module send the same
     * selection the operator's tap would.
     *
     * Returning true makes the builder return before it sets `this.a = true`, so the menu is never
     * drawn. A module that answers the menu itself has no use for it on screen — and dismissing it
     * after the fact was not enough, because the frame in between still showed it.
     */
    public static boolean menu(et items, String title) {
        // Captured before the trace gate, because ZONE needs the roster whether or not the operator
        // asked for a log. Only while a board is being waited on: reading every local menu into
        // module state would make a shop or an inventory menu look like a board reply.
        boolean taken = false;
        if (zonePhase == 2) {
            zoneRoster(items);
            // Only a roster this module could actually read is worth hiding: a menu it rejected has
            // to stay visible, or a board that is not what was measured would vanish silently.
            //
            // FORCED OFF. The swallow returns from `fr.a` before its own prologue runs, so the menu
            // object keeps the previous menu's state: `this.a` stays true from the last one and
            // `this.g`/`this.aa` are stale or null. cn's paint gate (cn.java:704) reads exactly those,
            // and the frame came out blank white with the tick still running. Hiding the menu has to
            // let the builder finish and clear `a` afterwards instead; until then it stays visible.
            taken = false;
        }
        if (!traceOn) {
            return taken;
        }
        try {
            trace("MENU title=" + clean(title) + " count=" + (items == null ? 0 : items.c())
                    + (taken ? " (taken)" : ""));
            if (items == null) {
                return taken;
            }
            for (int i = 0; i < items.c(); i++) {
                Object entry = items.a(i);
                if (!(entry instanceof bt)) {
                    trace("  [" + i + "] (not a button)");
                    continue;
                }
                bt button = (bt) entry;
                // `e` is the command id the client dispatches on and `f` the sub-index, both set by
                // the bt constructors (bt.java:38-54). Together they are what a later round would
                // have to reproduce to make the same selection the operator's tap makes.
                trace("  [" + i + "] cmd=" + button.e + " sub=" + button.f + " text="
                        + clean(button.a));
            }
        } catch (Throwable t) {
            // As above: never break the client's own menu.
        }
        return taken;
    }

    /**
     * Records a server-driven menu, the kind a teleport stone or a shop NPC arrives as, and says
     * whether a module took it.
     *
     * Separate from {@link #menu} because the two overloads of `fr.a` carry different facts. A local
     * menu's buttons hold their own command and sub-index, so logging those is enough to reproduce a
     * selection. A server menu's buttons hold neither: `fr.a(2, _)` sends `q.a().b(fr.C, fr.B, fr.h)`,
     * so the reproducible part is the idNPC/idMenu pair from the builder call plus the entry's
     * position — and pressing a button selects whatever is highlighted, not what was pressed.
     *
     * Returning true hides it, for the same reason the local overload does: a module that answers the
     * menu itself never needs it drawn.
     */
    public static boolean serverMenu(et items, int idMenu, int idNPC, String title) {
        boolean taken = false;
        // Captured before the trace gate, because TRAVEL needs the labels whether or not the operator
        // asked for a log. Only while a stone is being waited on: hooking every server menu into
        // module state would make a shop visit look like a travel reply.
        if (travelState == TV_STONE_WAIT && travelMenuNpc == Integer.MIN_VALUE) {
            try {
                int count = items == null ? 0 : items.c();
                String[] labels = new String[count];
                for (int i = 0; i < count; i++) {
                    Object entry = items.a(i);
                    labels[i] = entry instanceof bt ? ((bt) entry).a : null;
                }
                travelMenu = labels;
                travelMenuNpc = idNPC;
                travelMenuId = idMenu;
            } catch (Throwable t) {
                travelMenu = null;
            }
        }
        // Same reason, for the material submenu: DROPS asked for it with packet 1 and the answer is
        // the only place the idNPC/idMenu pair its selection has to quote is stated.
        if (dropPhase == 1 && dropMenuNpc == Integer.MIN_VALUE && dropSlot >= 0) {
            taken = dropMenu(items, idMenu, idNPC);
            if (taken) {
                // HIDDEN at the source, not dismissed after the fact: returning true makes the
                // builder return before `this.a = true`, so this menu never becomes the open panel —
                // there is no frame that draws it and no later tick that has to close it.
                //
                // Nothing is written to `fu.p` here on purpose. Phase 0 only starts behind
                // `noDialog()`, so `fu.p.a` is already false when the reply lands, and the swallowed
                // build leaves every field of the singleton exactly as it was. Clearing `a` would be
                // a no-op in that case and would silently close a menu the OPERATOR opened during
                // the wait in the other — the swallow must not take a panel it did not create.
                trace("DROP menu taken, not shown");
            }
            // A menu this module rejected — wrong entry count, or an entry naming a different
            // material — stays on screen instead of vanishing silently.
        }
        // ---- DUNGEON ----------------------------------------------------------
        // Same reason, for the dungeon NPC's two menus: this module asked with opcode 23 and the
        // answer is the only place the idNPC/idMenu pair its selection has to quote is stated.
        // Gated on its own wait state so an operator's own shop visit is never read as a reply.
        if (dungeonState == DN_INTERACT && dungeonMenuNpc == Integer.MIN_VALUE) {
            try {
                int count = items == null ? 0 : items.c();
                String[] labels = new String[count];
                for (int i = 0; i < count; i++) {
                    Object entry = items.a(i);
                    labels[i] = entry instanceof bt ? ((bt) entry).a : null;
                }
                dungeonMenu = labels;
                dungeonMenuNpc = idNPC;
                dungeonMenuId = idMenu;
            } catch (Throwable t) {
                dungeonMenu = null;
                dungeonMenuNpc = Integer.MIN_VALUE;
            }
            // NOT swallowed, and that is deliberate rather than an oversight. Returning true makes
            // the builder return before `this.a = true`, which leaves the singleton with `a` false
            // and a stale `g`/`aa`; cn's paint gate (cn.java:704) then draws a blank white frame.
            // The menu stays visible and this module closes it itself, the way travelCloseMenu()
            // does, once it has taken what it needed.
        }
        // ---- end DUNGEON ------------------------------------------------------
        if (!traceOn) {
            return taken;
        }
        try {
            trace("SMENU npc=" + idNPC + " menu=" + idMenu + " title=" + clean(title)
                    + " count=" + (items == null ? 0 : items.c()) + (taken ? " (taken)" : ""));
            if (items == null) {
                return taken;
            }
            for (int i = 0; i < items.c(); i++) {
                Object entry = items.a(i);
                if (!(entry instanceof bt)) {
                    trace("  <" + i + "> (not a button)");
                    continue;
                }
                trace("  <" + i + "> text=" + clean(((bt) entry).a));
            }
        } catch (Throwable t) {
            // As above: never break the client's own menu.
        }
        return taken;
    }

    /** Last values the tick-side trace reported, so it logs changes rather than every tick. */
    private static String traceDialog = null;
    private static int traceTargetId = Integer.MIN_VALUE;
    private static int traceZone = Integer.MIN_VALUE;

    /**
     * Records what the operator selected and what the server said, once per change.
     *
     * The target is how a clickable board on the map identifies itself: its kind, its template
     * and its name are exactly what a zone-change implementation has to match on.
     */
    private static void traceTick() {
        if (!traceOn) {
            return;
        }
        try {
            if (cn.i != null) {
                int id = cn.i.cu * 1000 + cn.i.cv;
                if (id != traceTargetId) {
                    traceTargetId = id;
                    trace("TARGET cv=" + cn.i.cv + " cu=" + cn.i.cu + " x=" + cn.i.aZ
                            + " y=" + cn.i.ba + " name=" + clean(cn.i.cC));
                }
            } else if (traceTargetId != Integer.MIN_VALUE) {
                traceTargetId = Integer.MIN_VALUE;
                trace("TARGET none");
            }
            String dialog = fu.s == null ? null : dialogText(fu.s);
            if (dialog != null && !dialog.equals(traceDialog)) {
                traceDialog = dialog;
                trace("DIALOG " + clean(dialog));
            } else if (dialog == null && traceDialog != null) {
                traceDialog = null;
                trace("DIALOG closed");
            }
            if (cs.u != traceZone) {
                traceZone = cs.u;
                trace("ZONE now=" + cs.u + " count=" + cs.v);
            }
        } catch (Throwable t) {
            // A trace must never stall the tick.
        }
    }

    private static void player() {
        if (outPath == null) {
            return;
        }
        try {
            // dx.a() is System.currentTimeMillis(). fu.aj is a tick counter that wraps
            // at 10000, so it cannot measure a five-minute window (docs/core/10 §4.3).
            long now = dx.a();
            if (cn.g == null) {
                return;             // not in a character yet: nothing to report
            }
            // cn.g is non-null well before the character exists: the client installs a
            // level-0 placeholder called "unname" (cn.java:94) and only fills the real
            // stats in when opcode 3 arrives. Sampling that placeholder put a level-0
            // reading at the start of the window, so the first real reading looked like a
            // gain of 80 levels and the rate came out in the millions.
            boolean statsArrived = cn.g.bz > 0;
            if (statsArrived && now - sampledAt >= sampleEveryMs) {
                sampledAt = now;
                sample(now, cn.g.bz, cn.g.bA);
            }
            if (now - wroteAt >= writeEveryMs) {
                wroteAt = now;
                publish(now);
            }
        } catch (Throwable t) {
            // A sensor must never stall the client tick.
        }
    }

    private static void sample(long now, int level, int permille) {
        sampleAt[sampleHead] = now;
        sampleLevel[sampleHead] = level;
        samplePermille[sampleHead] = permille;
        sampleHead = (sampleHead + 1) % SAMPLE_MAX;
        if (sampleCount < SAMPLE_MAX) {
            ++sampleCount;
        }
    }

    /**
     * XP gain in permille per hour, or -1 when it cannot be measured yet.
     *
     * Levelling mid-window would make the raw permille delta negative, so each level
     * crossed contributes a full 1000 (docs/core/10 §4.1). The rate is deliberately
     * reported instead of an ETA: an ETA implies the client knows the XP each level
     * needs, and it does not — only the percentage through the current one.
     */
    private static int xpPermillePerHour() {
        if (sampleCount < 2) {
            return -1;
        }
        long newestAt = 0L;
        int newestLevel = 0;
        int newestPermille = 0;
        long oldestAt = 0L;
        int oldestLevel = 0;
        int oldestPermille = 0;
        boolean haveOldest = false;
        for (int i = 0; i < sampleCount; i++) {
            int index = (sampleHead - 1 - i + SAMPLE_MAX * 2) % SAMPLE_MAX;
            long at = sampleAt[index];
            if (i == 0) {
                newestAt = at;
                newestLevel = sampleLevel[index];
                newestPermille = samplePermille[index];
                continue;
            }
            if (newestAt - at > xpWindowMs) {
                break;              // older than the window: stop walking back
            }
            oldestAt = at;
            oldestLevel = sampleLevel[index];
            oldestPermille = samplePermille[index];
            haveOldest = true;
        }
        if (!haveOldest) {
            return -1;
        }
        long elapsed = newestAt - oldestAt;
        if (elapsed <= 0L) {
            return -1;
        }
        long gained = (long) (newestLevel - oldestLevel) * 1000L
                + (long) (newestPermille - oldestPermille);
        if (gained <= 0L) {
            return 0;               // measured, and genuinely not gaining
        }
        return (int) (gained * 3600000L / elapsed);
    }

    /** Writes the snapshot atomically: temp file, then replace. */
    private static void publish(long now) {
        StringBuffer out = new StringBuffer(320);
        // 5: travelgoal now reports the destination in force rather than the configured one, so a
        // tool reading v4 semantics would mislabel a nav.target route as an atk.travel one.
        // 6: `mounts` added. The tool cannot name a mount without it — no id-to-name table exists
        // anywhere in the client, so the bag is the only honest source.
        out.append("v=6\n");
        out.append("t=").append(now).append('\n');
        out.append("name=").append(clean(cn.g.cC)).append('\n');
        out.append("lv=").append(cn.g.bz).append('\n');
        out.append("xp=").append(cn.g.bA).append('\n');
        out.append("hp=").append(cn.g.bs).append('\n');
        out.append("hpmax=").append(cn.g.bt).append('\n');
        out.append("mp=").append(cn.g.bu).append('\n');
        out.append("mpmax=").append(cn.g.bv).append('\n');
        // bD is gold and bC is gem; both only arrive with opcode 16.
        if (cn.g.bD != 0L || cn.g.bC != 0L) {
            walletKnown = true;
        }
        out.append("wallet=").append(walletKnown ? 1 : 0).append('\n');
        out.append("gold=").append(cn.g.bD).append('\n');
        out.append("gem=").append(cn.g.bC).append('\n');
        out.append("map=").append(fu.q != null ? fu.q.d : -1).append('\n');
        out.append("zone=").append(cs.u).append('\n');
        out.append("px=").append(cn.g.aZ).append('\n');
        out.append("py=").append(cn.g.ba).append('\n');
        // bq.e is the attack quota. At <= 0 the client silently drops auto from 1 to 0
        // (bq.java:577), which is the top cause of "auto stopped for no reason".
        out.append("quota=").append(bq.e).append('\n');
        out.append("bag=").append(bw.V != null ? bw.V.c() : -1).append('\n');
        out.append("bagmax=").append(bq.x).append('\n');
        out.append("state=").append(cn.g.cG).append('\n');
        out.append("mount=").append(cn.g.ef).append('\n');
        // The mounts actually in the bag, `id:name` pairs joined by `|`, so the tool can offer them
        // by their server-given names instead of by a number.
        out.append("mounts=").append(mountList()).append('\n');
        out.append("guild=").append(cn.g.cP != null ? clean(cn.g.cP.c) : "").append('\n');
        out.append("xprate=").append(xpPermillePerHour()).append('\n');
        // What the two active modules are actually doing. Without these there is no way to
        // tell a working module from a silent one without opening the game and watching.
        out.append("atkphase=").append(bq.o).append('\n');
        out.append("ctl=").append(ctlState).append('\n');
        out.append("atkstate=").append(combatOwned ? atkState : -1).append('\n');
        out.append("target=").append(cn.i != null && cn.i.cv == 1 && cn.i.bs > 0 ? 1 : 0)
                .append('\n');
        out.append("stuck=").append(stuckKind).append('\n');
        // Travel's own state and, when it stopped, why. A route that fails has to say so: "walking"
        // that never arrives and "gave up two maps ago" look identical from outside.
        out.append("travel=").append(travelState).append('\n');
        out.append("travelwhy=").append(travelWhy).append('\n');
        // The destination in force, not the configured one: with nav.target set they differ, and the
        // panel has to name where the character is actually headed.
        out.append("travelgoal=").append(goal()).append('\n');
        out.append("travelhops=").append(travelHops).append('\n');
        out.append("potions=").append(potionCount).append('\n');
        out.append("revives=").append(reviveCount).append('\n');
        // The pickup record actually in force, read back from the client rather than echoed from
        // the settings file. co.b() and its read-back disagree about which byte carries which
        // value, so this is the only honest way to show the operator what took effect.
        out.append("pkrank=").append(bq.q == null ? -1 : bq.q.a).append('\n');
        out.append("pkmphp=").append(bq.q == null ? -1 : bq.q.c).append('\n');
        out.append("pkgold=").append(bq.q == null ? -1 : bq.q.b).append('\n');
        // Which buff slots the client will actually cast, after the learned check.
        out.append("buffs=").append(buffState()).append('\n');
        // Six characters, one per material: `-` never confirmed, `0` open, `1` closed. Read back
        // from the server's own confirmations, so a setting that never took effect shows as unknown
        // instead of as applied.
        out.append("drops=").append(dropStates()).append('\n');
        // eh.h false or a loading map means the values are last-known, not current.
        out.append("stale=").append(sceneReady() ? 0 : 1).append('\n');
        // ---- ENHANCE ----------------------------------------------------------
        // Enhance's own state and, when it stopped, why. Appended after every key that was
        // already published, so a tool built against the previous snapshot still reads all of
        // them and simply does not know these three yet.
        out.append("enhancephase=").append(enhancePhase).append('\n');
        out.append("enhancewhy=").append(enhanceWhy).append('\n');
        out.append("enhancedone=").append(enhanceDone).append('\n');
        // ---- end ENHANCE ------------------------------------------------------
        // ---- DUNGEON ----------------------------------------------------------
        // The dungeon module's own state and, when it stopped, why. Appended after every key that
        // was already published, so a tool built against the previous snapshot still reads all of
        // them and simply does not know these four yet.
        out.append("dungeonstate=").append(dungeonState).append('\n');
        out.append("dungeonwhy=").append(dungeonWhy).append('\n');
        out.append("dungeonruns=").append(dungeonRuns).append('\n');
        // -1 when the module is off rather than 48, for travelgoal's reason: the panel has to tell
        // "nowhere" apart from a real map id, and 48 is a real map id. Reporting the dungeon as
        // the destination of a trip that is not running would be a destination nobody armed.
        out.append("dungeongoal=").append(dungeonEnabled ? DUNGEON_MAP : -1).append('\n');
        // ---- end DUNGEON ------------------------------------------------------
        write(out.toString());
    }

    /**
     * Whether the scene is settled enough for the readings to be current.
     *
     * Player still reports while loading — a stale number is useful to a human — but it
     * marks the snapshot so nothing downstream treats it as live.
     */
    private static boolean sceneReady() {
        try {
            return fu.a == fu.c && eh.h && fu.q != null && cs.i != cs.j;
        } catch (Throwable t) {
            return false;
        }
    }

    /** Strips the delimiters the format reserves, and bounds the length. */
    private static String clean(String value) {
        if (value == null) {
            return "";
        }
        String text = value;
        if (text.length() > 64) {
            text = text.substring(0, 64);
        }
        StringBuffer safe = new StringBuffer(text.length());
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c == '=' || c == '\n' || c == '\r') {
                continue;
            }
            safe.append(c);
        }
        return safe.toString();
    }

    /**
     * Replaces the snapshot, never leaving a half-written one readable.
     *
     * A failed rename keeps the previous snapshot rather than deleting it: slightly
     * stale data beats an empty file, and the next tick tries again.
     */
    private static void write(String body) {
        java.io.OutputStream stream = null;
        try {
            java.io.File target = new java.io.File(outPath);
            java.io.File temp = new java.io.File(outPath + ".tmp");
            stream = new java.io.FileOutputStream(temp);
            stream.write(body.getBytes("UTF-8"));
            stream.close();
            stream = null;
            if (target.exists() && !target.delete()) {
                return;             // keep the previous snapshot
            }
            temp.renameTo(target);
        } catch (Throwable t) {
            // Unwritable path: stay silent rather than spamming the client's log.
        } finally {
            if (stream != null) {
                try {
                    stream.close();
                } catch (Throwable t) {
                    // nothing useful to do on a failed close
                }
            }
        }
    }

    // ---- GUARDS ---------------------------------------------------------------
    //
    // docs/core/08-module-attack.md §10. Two of these — eh.h and cs.i != cs.j — are the
    // ones KnightMod never reads, which is how it ends up acting on the previous map's
    // coordinates while a new map loads.

    /** In the world screen, not a menu. */
    private static boolean inGame() {
        return fu.a != null && fu.a == fu.c;
    }

    /** No dialog is up. Acting behind one sends input the operator cannot see. */
    private static boolean noDialog() {
        return fu.s == null && fu.t == null && (fu.p == null || !fu.p.a);
    }

    private static boolean alive() {
        return cn.g != null && cn.g.cG != 4;
    }

    /** Free to move: neither our own path nor the client's movement lock is held. */
    private static boolean canMove() {
        return !bq.m && cn.g != null && cn.g.cO == null;
    }

    /** The captcha monster ("Con Ma") outranks everything; never fight through it. */
    private static boolean captcha() {
        return cn.i != null && cn.i.cy == 2;
    }

    /** Everything Attack and Item both need before touching a field. */
    private static boolean ready() {
        return inGame() && sceneReady() && alive() && noDialog() && !captcha();
    }

    // ---- GAME READY & SESSION LIFECYCLE ---------------------------------------

    private static final int DIALOG_DEBOUNCE_TICKS = 3;
    private static final int DIALOG_MAX_TRIES = 3;

    private static String dialogLastFingerprint = "";
    private static int dialogStableTicks = 0;
    private static int dialogTries = 0;
    private static String dialogLastTracedFingerprint = "";

    /** Settle ticks required in-world before gameReady() becomes true (10 ticks = 400ms at 25t/s). */
    private static final int GAME_READY_SETTLE_TICKS = 10;
    private static int readySettleTicks = 0;
    private static int lastScreenId = -1;

    /** Map stability gate for travel/routing (10 ticks = 400ms at 25t/s). */
    private static final int MAP_STABLE_TICKS = 10;
    private static int stableMapId = -1;
    private static int mapStableTicks = 0;

    /** True when the client has observed one stable authoritative map for the consecutive threshold. */
    public static boolean mapStable() {
        return inGame() && sceneReady() && fu.q != null && fu.q.d >= 0
                && fu.q.d == stableMapId && mapStableTicks >= MAP_STABLE_TICKS;
    }

    public static void mapStableReset() {
        stableMapId = -1;
        mapStableTicks = 0;
    }

    /**
     * Internal game-readiness gate.
     * Prevents automation from resuming during login, loading, or dialog settling.
     */
    public static boolean gameReady() {
        if (!inGame() || !sceneReady() || !alive() || captcha() || !noDialog()
                || cn.g == null || cn.g.cx < 0 || cn.g.cy < 0 || fu.q == null || fu.q.d < 0) {
            return false;
        }
        return readySettleTicks >= GAME_READY_SETTLE_TICKS;
    }

    /**
     * Resets session-local settling and dialog recovery state.
     * Called on character-select / world entry transitions.
     */
    public static void sessionReset() {
        readySettleTicks = 0;
        dialogLastFingerprint = "";
        dialogStableTicks = 0;
        dialogTries = 0;
        dialogLastTracedFingerprint = "";
        authReset();
        mapStableReset();
        navSessionReset();
    }

    private static void sessionTick() {
        int screenId = (fu.a == null) ? -1 : ((fu.a == fu.i) ? 1 : ((fu.a == fu.c) ? 2 : 0));
        if (screenId != lastScreenId) {
            if (screenId == 1 || screenId == 2) {
                sessionReset();
            }
            lastScreenId = screenId;
        }
        if (inGame() && sceneReady()) {
            if (readySettleTicks < GAME_READY_SETTLE_TICKS) {
                ++readySettleTicks;
            }
            if (fu.q != null && fu.q.d >= 0) {
                int curMap = fu.q.d;
                if (curMap == stableMapId) {
                    if (mapStableTicks < MAP_STABLE_TICKS) {
                        ++mapStableTicks;
                    }
                } else {
                    stableMapId = curMap;
                    mapStableTicks = 1;
                }
            } else {
                mapStableReset();
            }
        } else {
            readySettleTicks = 0;
            mapStableReset();
        }
    }

    /**
     * Strict allowlist for harmless informational server notices and announcements.
     * Transport/disconnect dialogs and specialized material drop dialogs are excluded.
     */
    private static boolean isAllowlistedInformational(String text) {
        if (text == null || text.length() == 0) {
            return false;
        }
        if (text.indexOf("mat ket noi") >= 0 || text.indexOf("ket noi that bai") >= 0
                || text.indexOf("vui long dang nhap lai") >= 0) {
            return false;
        }
        if (text.indexOf("chuc nang rot") >= 0 || text.indexOf("nguyen lieu me day") >= 0) {
            return false;
        }
        if (text.indexOf("thong bao") >= 0
                || text.indexOf("chao mung") >= 0
                || text.indexOf("su kien") >= 0
                || text.indexOf("chuc cac hiep si") >= 0
                || text.indexOf("tips:") >= 0
                || text.indexOf("huong dan") >= 0) {
            return true;
        }
        return false;
    }

    /**
     * Validates that the dialog has exactly one button and that it is an acknowledge/close button.
     * Never confirms single buttons with "Đồng ý" (dong y) or multi-button choice dialogs.
     */
    private static bt findDismissButton(et buttons) {
        if (buttons == null || buttons.c() != 1) {
            return null;
        }
        Object entry = buttons.a(0);
        if (!(entry instanceof bt)) {
            return null;
        }
        bt btn = (bt) entry;
        String cap = norm(btn.a).trim();
        if (cap.equals("dong") || cap.equals("ok") || cap.equals("dong tab nay")
                || cap.equals("tro ve") || cap.equals("da hieu")) {
            return btn;
        }
        return null;
    }

    /**
     * Dedicated safe dialog recovery path. Runs ahead of normal automation readiness checks.
     * Safe informational dialogs are debounced and dismissed via their close button.
     * Unknown or dangerous dialogs fail closed, remain untouched, and emit deduplicated trace evidence.
     */
    private static void dialogRecovery() {
        if (fu.s == null) {
            if (dialogStableTicks > 0 || dialogLastFingerprint.length() > 0) {
                dialogLastFingerprint = "";
                dialogStableTicks = 0;
                dialogTries = 0;
            }
            return;
        }
        if (!(fu.s instanceof ah)) {
            return;
        }
        ah dialog = (ah) fu.s;
        String rawText = dialogText(dialog);
        String text = norm(rawText);
        et buttons = dialog.C;
        int btnCount = (buttons != null) ? buttons.c() : 0;

        String fingerprint = text + "|" + btnCount;
        if (fingerprint.equals(dialogLastFingerprint)) {
            ++dialogStableTicks;
        } else {
            dialogLastFingerprint = fingerprint;
            dialogStableTicks = 1;
            dialogTries = 0;
        }

        // Bounded debounce: require dialog identity/state stability before acting
        if (dialogStableTicks < DIALOG_DEBOUNCE_TICKS) {
            return;
        }

        boolean allowlisted = isAllowlistedInformational(text);
        bt dismissBtn = allowlisted ? findDismissButton(buttons) : null;

        if (dismissBtn != null) {
            if (dialogTries < DIALOG_MAX_TRIES) {
                ++dialogTries;
                trace("DIALOG dismissed try=" + dialogTries + " text=" + clean(text));
                dismissBtn.a();
            }
        } else {
            // Unknown or non-dismissible dialog: fail closed and emit deduplicated diagnostic trace
            if (!fingerprint.equals(dialogLastTracedFingerprint)) {
                dialogLastTracedFingerprint = fingerprint;
                StringBuffer caps = new StringBuffer();
                if (buttons != null) {
                    for (int i = 0; i < buttons.c(); i++) {
                        Object b = buttons.a(i);
                        if (b instanceof bt) {
                            if (caps.length() > 0) caps.append(',');
                            caps.append(clean(((bt) b).a));
                        }
                    }
                }
                trace("DIALOG unresolved class=" + dialog.getClass().getName()
                        + " buttons=" + btnCount
                        + " captions=[" + caps.toString() + "]"
                        + " text=" + clean(text));
            }
        }
    }

    // ---- TRAVEL ---------------------------------------------------------------
    //
    // Gets the character to the anchor's map. Two moves only, and the probe of 2026-09-03
    // (docs/core/13-module-travel.md §4.3) is what made the first one cheap:
    //
    //   1. A teleport stone. Server does not distance-check opcode 23 — 618 px and 882 px both
    //      returned a menu — so this never walks to the stone. It sends the same opcode the
    //      client's own ez.k() sends, reads the destinations the server itself names, and picks
    //      the one that leaves the least walking. KnightMod's approach loop and its lag counters
    //      are not ported: they solved a problem that turned out not to exist.
    //   2. Walking out through a map exit. The client is handed every exit of the current map,
    //      with the destination map's NAME, in the map-load packet (cs.a). So the only surveyed
    //      data this needs is which maps border which — the client has no such table.
    //
    // Everything else is refused rather than guessed: an unroutable destination stops and says so
    // instead of walking the character somewhere hopeful.

    /** Travel states. Deliberately few: anything not one of these is a reason to stop. */
    private static final int TV_OFF = 0, TV_IDLE = 1, TV_STONE_WAIT = 2, TV_WALK = 3,
            TV_ARRIVED = 4, TV_BLOCKED = 5;

    private static int travelState = TV_OFF;
    private static int travelWait = 0;
    private static int travelMapSeen = Integer.MIN_VALUE;
    /** Stones already asked on this map, so a stone that cannot help is not asked forever. */
    private static int travelStoneTried = 0;
    /** True once the stone budget on this map ran out, so the fallback is announced exactly once. */
    private static boolean travelStoneGaveUp = false;
    private static int travelStoneAsked = -1;
    private static int travelHops = 0;
    /** Why travel stopped, published in the snapshot so a stall is legible from the tool. */
    private static int travelWhy = 0;
    /**
     * The map to walk to, or -1 for nowhere. The only destination there is.
     *
     * There were two, and that was the bug: `atk.travel` walked to wherever the spot was saved while
     * this walked to a named map, so with both set the walker arrived at one and was dragged toward the
     * other. Arriving here turns the walk off rather than starting to fight; whether to fight is
     * {@link #atkFarmOnArrival}.
     */
    private static int navTarget = -1;
    /** True once the walker reached {@link #navTarget}, so the arrival is announced exactly once. */
    private static boolean navDone = false;

    private static void navReset() {
        navDone = false;
        travelReset();
    }

    /** True when Auto Farm is armed with a valid attack spot. */
    private static boolean autoFarmActive() {
        return (atkMode == 1 || atkMode == 2) && atkX >= 0 && atkY >= 0 && atkMap >= 0;
    }

    /** The destination in force, or -1 when the walker has nowhere to be. */
    private static int goal() {
        if (atkMode != 0) {
            return autoFarmActive() ? atkMap : -1;
        }
        return navDone ? -1 : navTarget;
    }

    /**
     * Resets session-local travel, movement lock, and path buffers on a new world session.
     * Preserves durable control settings (atkMap/X/Y, atkMode, atkFarmOnArrival, navTarget, navDone).
     */
    public static void navSessionReset() {
        travelMapSeen = Integer.MIN_VALUE;
        travelHops = 0;
        travelStoneTried = 0;
        travelStoneGaveUp = false;
        travelStoneAsked = -1;
        travelMenu = null;
        travelMenuNpc = Integer.MIN_VALUE;
        travelMazeKnown = false;
        travelMazeAt = 0;
        travelStallTicks = 0;
        travelLastX = Integer.MIN_VALUE;
        travelLastY = Integer.MIN_VALUE;
        travelWait = 0;
        travelWhy = 0;
        travelState = (goal() >= 0) ? TV_IDLE : TV_OFF;
        bq.m = false;
        if (cn.g != null) {
            cn.g.cO = null;
        }
    }

    private static void travelReset() {
        travelState = goal() >= 0 ? TV_IDLE : TV_OFF;
        travelWait = 0;
        travelStoneTried = 0;
        travelStoneGaveUp = false;
        travelStoneAsked = -1;
        travelHops = 0;
        travelWhy = 0;
        travelMenuNpc = Integer.MIN_VALUE;
        travelMenu = null;
        travelMazeKnown = false;
        travelMazeAt = 0;
        travelStallTicks = 0;
        travelLastX = Integer.MIN_VALUE;
        travelLastY = Integer.MIN_VALUE;
    }

    /**
     * One step toward the anchor's map, or nothing at all.
     *
     * Runs before attack() so that arriving and fighting can happen on the same tick the map
     * changes. Off unless the operator asked for it: a spot saved on another map used to mean
     * "do nothing", and turning that into "walk across the world" without being asked would be
     * a surprise, not a feature.
     */
    private static void travel() {
        try {
            int want = goal();
            // No attack mode needed: walking somewhere the operator named is the whole request, and
            // whether to fight on arrival is a separate decision.
            if (want < 0) {
                travelState = TV_OFF;
                return;
            }
            if (!inGame() || !sceneReady() || !alive() || captcha()
                    || cn.g == null || fu.q == null) {
                return;             // keep the intent; a load screen is not a failure
            }
            // Deliberately NOT ready(): that requires no dialog, and an open menu is exactly the
            // state a stone reply arrives in. Gating on it here would deadlock — the menu blocks
            // travel, and only travel closes the menu. Route decisions additionally require mapStable().
            if (travelState != TV_STONE_WAIT && (!gameReady() || !mapStable())) {
                return;
            }
            int here = fu.q.d;
            if (here != travelMapSeen) {
                // A new map resets the per-map attempts, and counts a hop. The hop cap is what
                // stops a two-map loop from running until the operator notices.
                boolean isInitialSessionMap = (travelMapSeen == Integer.MIN_VALUE);
                travelMapSeen = here;
                travelStoneTried = 0;
                travelStoneGaveUp = false;
                travelStoneAsked = -1;
                travelMenu = null;
                travelMenuNpc = Integer.MIN_VALUE;
                travelMazeKnown = false;
                travelMazeAt = 0;
                travelStallTicks = 0;
                travelLastX = Integer.MIN_VALUE;
                travelLastY = Integer.MIN_VALUE;
                // Any route from the previous map is void, and the movement lock outlives it. Left
                // set, canMove() stays false forever and both travel and attack silently stop.
                bq.m = false;
                if (cn.g != null) {
                    cn.g.cO = null;
                }
                if (!isInitialSessionMap && travelState != TV_OFF && travelState != TV_ARRIVED) {
                    ++travelHops;
                }
                travelWait = 12;    // let the scene settle before reading it
                if (travelState == TV_BLOCKED) {
                    travelState = TV_IDLE;   // a new map is a new chance
                }
            }
            if (here == want) {
                if (travelState != TV_ARRIVED) {
                    travelState = TV_ARRIVED;
                    travelWhy = 0;
                    trace("TRAVEL arrived at map " + here + " in " + travelHops + " hops");
                    // A named destination is a one-shot errand: latch it, or the walker keeps dragging
                    // the character back every time the operator moves on.
                    if (atkMode == 0 && navTarget == here) {
                        navDone = true;
                        trace("NAV arrived at map " + here + ", switching itself off");
                    }
                    if (autoFarmActive()) {
                        atkState = TO_SPOT;
                    }
                }
                return;
            }
            if (travelState == TV_ARRIVED) {
                travelState = TV_IDLE;      // pushed off the destination; go back
                travelHops = 0;
            }
            if (travelState == TV_BLOCKED) {
                return;
            }
            if (travelHops > 24) {
                travelStop(4, "hop cap reached");
                return;
            }
            // Waiting on a stone is polled before the settle timer, not after it: the menu can land
            // on any tick and the timer is that wait's deadline, not a delay before looking.
            if (travelState == TV_STONE_WAIT) {
                travelStoneMenu();
                return;
            }
            if (travelWait > 0) {
                --travelWait;
                return;
            }
            if (travelState == TV_WALK && !canMove()) {
                return;             // a route is running; let the client finish it
            }
            // A character that stops making progress is a stall, not a route. Without this the mod
            // would sit in TV_WALK forever on a map whose exit the pathfinder cannot reach, and the
            // tool would show "travelling" with nothing happening.
            if (travelState == TV_WALK) {
                if (cn.g.aZ == travelLastX && cn.g.ba == travelLastY) {
                    ++travelStallTicks;
                } else {
                    travelStallTicks = 0;
                    travelLastX = cn.g.aZ;
                    travelLastY = cn.g.ba;
                }
                if (travelStallTicks > 150) {
                    travelStop(2, "stalled walking on map " + here);
                    return;
                }
            }
            travelState = TV_IDLE;
            // Stone first, walk when it declines. travelStone() returns false both when this map has
            // no stone and when its budget ran out, so the fallback needs no separate branch.
            if (travelStone(here)) {
                return;
            }
            travelWalk(here);
        } catch (Throwable t) {
            travelStop(5, "travel threw");
        }
    }

    private static void travelStop(int why, String reason) {
        travelState = TV_BLOCKED;
        travelWhy = why;
        trace("TRAVEL blocked: " + reason);
    }

    /** Labels of the server menu currently open, captured by {@link #serverMenu}. */
    private static String[] travelMenu = null;
    private static int travelMenuNpc = Integer.MIN_VALUE;
    private static int travelMenuId = 0;

    /**
     * Asks a teleport stone on this map, if one of its destinations beats walking.
     *
     * Returns true when a stone was asked, so the caller stops for this tick. The reply arrives as a
     * server menu, which lands in {@link #serverMenu} and is acted on by {@link #travelStoneMenu}.
     *
     * No approach: the stone is asked from wherever the character stands, because the server does
     * not check. That is measured, not assumed — 618 px and 882 px both answered.
     */
    private static boolean travelStone(int here) {
        // Three failures is the whole stone budget for this map, and running out means WALK, not stop.
        // A stone that will not answer is a slow route, not a dead end — the map graph still has one.
        if (travelStoneTried >= 3) {
            if (!travelStoneGaveUp) {
                travelStoneGaveUp = true;
                trace("TRAVEL stone unresponsive after 3 tries on map " + here + ", walking instead");
            }
            return false;
        }
        if (cn.j == null) {
            return false;
        }
        fa stone = null;
        for (int i = 0; i < cn.j.c(); i++) {
            Object entry = cn.j.a(i);
            if (!(entry instanceof fa)) {
                continue;
            }
            fa candidate = (fa) entry;
            // Matched by name: a map can carry more than one stone and their cu differs per region,
            // so cu identifies which stone this is rather than what it is.
            if (candidate.cv != 2 || norm(candidate.cC).indexOf("dich chuyen") < 0) {
                continue;
            }
            if (candidate.cu == travelStoneAsked) {
                continue;           // already asked this one on this map; try the other
            }
            stone = candidate;
            break;
        }
        if (stone == null) {
            return false;
        }
        travelStoneAsked = stone.cu;
        ++travelStoneTried;
        travelMenu = null;
        travelMenuNpc = Integer.MIN_VALUE;
        try {
            q.a().a((byte) stone.cu);
        } catch (Throwable t) {
            return false;
        }
        travelState = TV_STONE_WAIT;
        travelWait = 20;            // ~20 ticks for the reply; the menu usually lands well inside
        trace("TRAVEL asked stone cu=" + stone.cu + " on map " + here);
        return true;
    }

    /**
     * Picks the destination that leaves the least walking, or dismisses the menu.
     *
     * The server names its own destinations, so the reachable set is read rather than tabulated:
     * MOD03.f322 turned out to promise three maps this server does not offer (§4.3). Each label is
     * resolved to a map id and scored by how many map borders still separate it from the goal; the
     * goal itself scores zero. A destination only wins if it beats standing here, so a stone that
     * cannot help is dismissed instead of used.
     */
    private static void travelStoneMenu() {
        if (travelMenu == null) {
            if (travelWait > 0) {
                --travelWait;
                return;
            }
            trace("TRAVEL stone gave no menu");
            travelState = TV_IDLE;
            travelWait = 6;
            return;
        }
        String[] labels = travelMenu;
        int npc = travelMenuNpc;
        int menuId = travelMenuId;
        travelMenu = null;
        travelMenuNpc = Integer.MIN_VALUE;
        int dest = goal();
        int hereScore = mapDistance(fu.q.d, dest);
        int best = -1;
        int bestScore = Integer.MAX_VALUE;
        for (int i = 0; i < labels.length; i++) {
            int id = mapByName(labels[i]);
            if (id < 0) {
                continue;           // a destination this client has no name for; not routable
            }
            int score = mapDistance(id, dest);
            if (score < 0) {
                continue;
            }
            if (score < bestScore) {
                bestScore = score;
                best = i;
            }
        }
        if (best < 0 || (hereScore >= 0 && bestScore >= hereScore)) {
            trace("TRAVEL stone useless: best=" + bestScore + " here=" + hereScore);
            travelCloseMenu();
            travelState = TV_IDLE;
            travelWait = 6;
            return;
        }
        trace("TRAVEL stone -> [" + best + "] " + clean(labels[best]) + " score " + hereScore
                + " -> " + bestScore);
        try {
            // The idNPC/idMenu pair the server itself sent, not an assumed zero: this is the same
            // packet fr.a(2, _) builds when the operator taps the row.
            q.a().b((short) npc, (byte) menuId, (byte) best);
        } catch (Throwable t) {
            travelCloseMenu();
            travelStop(3, "stone select failed");
            return;
        }
        // The client dismisses its own menu when the operator taps a row; sending the packet
        // ourselves does not, and an open menu fails noDialog() for every other module.
        travelCloseMenu();
        travelState = TV_IDLE;
        travelWait = 30;            // the map change lands on its own; do not act meanwhile
    }

    /** Dismisses a menu the way its own Back button does. */
    private static void travelCloseMenu() {
        try {
            if (fu.p != null && fu.p.a) {
                fu.p.f();
            }
        } catch (Throwable t) {
            // A menu that will not close is not worth stalling travel over.
        }
    }

    private static int travelStallTicks = 0;
    private static int travelLastX = Integer.MIN_VALUE;
    private static int travelLastY = Integer.MIN_VALUE;

    /**
     * Walks out through the exit that leads to the next map on the route.
     *
     * The exit list and the destination names in it are the server's, delivered with the map
     * (cs.a, "LoadMap vecPointChange"), so no exit coordinates are surveyed here. The one surveyed
     * thing is which maps border which, because the client carries no such table.
     */
    private static void travelWalk(int here) {
        int dest = goal();
        int hop = mapNextHop(here, dest);
        if (hop < 0) {
            travelStop(1, "no route from " + here + " to " + dest);
            return;
        }
        String want = mapName(hop);
        if (want == null) {
            travelStop(1, "no name for map " + hop);
            return;
        }
        String needle = norm(want);
        if (cs.a == null) {
            return;
        }
        for (int i = 0; i < cs.a.c(); i++) {
            Object entry = cs.a.a(i);
            if (!(entry instanceof eo)) {
                continue;
            }
            eo gate = (eo) entry;
            if (gate.t == null || norm(gate.t).indexOf(needle) < 0) {
                continue;
            }
            if (travelState != TV_WALK) {
                trace("TRAVEL walk to map " + hop + " via " + clean(gate.t)
                        + " at " + gate.a + "," + gate.b);
            }
            travelState = TV_WALK;
            travelMove(here, gate.a, gate.b);
            return;
        }
        travelStop(2, "map " + here + " has no exit named " + clean(want));
    }

    /**
     * Four maps the pathfinder cannot cross in one route, and the waypoint chains that cross them.
     *
     * Borrowed verbatim from KnightMod (`MOD03.f095/f096/f116/f298`), including which direction each
     * chain runs. It is survey data about map geometry, not something derivable from the client, and
     * it is the one part of TRAVEL this round could not verify by walking it — the destinations the
     * stone offers do not pass through any of these four. Marked so the next reader knows.
     */
    private static int[][] mazeChain(int map) {
        if (map == 21) {
            return new int[][] {{480, 888}, {648, 696}, {144, 528}, {432, 168}};
        }
        if (map == 38) {
            return new int[][] {{696, 1104}, {624, 792}, {120, 528}, {504, 192}};
        }
        if (map == 41) {
            return new int[][] {{312, 96}, {816, 144}, {1056, 528}, {1440, 408}};
        }
        if (map == 96) {
            return new int[][] {{216, 192}, {216, 576}, {624, 288}, {168, 648}, {72, 192}};
        }
        return null;
    }

    /** Which way along the chain, by the same rule KnightMod used for each of the four maps. */
    private static boolean mazeForward(int map, int hop, int exitX) {
        if (map == 21) {
            return hop == 37;
        }
        if (map == 38) {
            return hop == 39;
        }
        if (map == 41) {
            return hop == 51;
        }
        return exitX < 600;         // map 96
    }

    private static int travelMazeAt = 0;
    private static boolean travelMazeFwd = false;
    private static boolean travelMazeKnown = false;

    /**
     * Paths to a point, going through the map's waypoint chain first if it has one.
     *
     * Pathfinder argument order is destination first, current position second — the documented
     * mistake that sends the character the opposite way (docs/core/08 §7.2).
     */
    private static void travelMove(int here, int exitX, int exitY) {
        int goX = exitX;
        int goY = exitY;
        int[][] chain = mazeChain(here);
        if (chain != null) {
            if (!travelMazeKnown) {
                travelMazeFwd = mazeForward(here, mapNextHop(here, goal()), exitX);
                travelMazeAt = 0;
                travelMazeKnown = true;
            }
            if (travelMazeAt < chain.length) {
                int at = travelMazeFwd ? travelMazeAt : chain.length - 1 - travelMazeAt;
                goX = chain[at][0];
                goY = chain[at][1];
                // Reaching a waypoint is an arrival like any other: the leftover route and pixel
                // offset have to go, or the next link is pathed while the client still thinks it walks.
                if (travelArrive(goX, goY, 54)) {
                    ++travelMazeAt;
                    return;         // next tick heads for the next link
                }
            }
        }
        if (!canMove()) {
            return;
        }
        // Standing on it already: nothing to path, and the map change is the server's to make. Cleared
        // through travelArrive so a pixel offset left over from the approach cannot wedge the client.
        if (travelArrive(goX, goY, 24)) {
            travelStallTicks += 4;  // on the gate and not moving through it counts toward a stall
            return;
        }
        try {
            short[] path = fu.c.a(goX / 24, goY / 24, cn.g.aZ / 24, cn.g.ba / 24, 500);
            if (path == null || path.length > 500) {
                travelStallTicks += 4;
                return;
            }
            cn.g.cO = path;
            cn.g.cJ = 0;
            cn.g.cm = 0;
            cn.g.cn = 0;
            cn.g.bg = cn.g.aZ;
            cn.g.bh = cn.g.ba;
            bq.m = true;
        } catch (Throwable t) {
            bq.m = false;
            travelStallTicks += 4;
        }
    }

    /**
     * Declares the walk over and clears every field that means "still moving".
     *
     * `cO` alone is not enough. `au.java:187` only drops `cG` back to 0 once **both** `bc` and `bd`
     * are zero, so a leftover pixel offset keeps the client in its walking state and every later step
     * is refused in silence — no error, nothing in a trace, just a character that never moves again.
     * `bg`/`bh` are pinned to where the character actually is for the same reason: a stale step target
     * is a walk the client believes it still owes.
     *
     * Manhattan, not Euclid: the client's own reach checks are Manhattan and a mixed metric would call
     * "arrived" at a distance the client still considers travelling.
     */
    private static boolean travelArrive(int x, int y, int tolerance) {
        if (cn.g == null) {
            return false;
        }
        if (abs(cn.g.aZ - x) + abs(cn.g.ba - y) > tolerance) {
            return false;
        }
        bq.m = false;
        cn.g.cO = null;
        cn.g.bg = cn.g.aZ;
        cn.g.bh = cn.g.ba;
        cn.g.bc = 0;
        cn.g.bd = 0;
        return true;
    }

    // ---- MAP GRAPH ------------------------------------------------------------
    //
    // Which maps border which. The client has no such table — it is handed the exits of the map it
    // is standing on and nothing more — so this is survey data, taken from KnightMod's MOD03.f325
    // and transcribed by script rather than by hand. 78 nodes, 81 undirected edges, 76 of them in
    // one connected component; 127 and 135 sit outside it.

    private static final String MAP_ADJ =
            "0:1|1:0,2,3,7,50|2:1|3:1,4|4:3,5|5:4,6|6:5|7:1,8,11,15,18|8:7,9|9:8,10|10:9|11:7,12|"
            + "12:11,13|13:12,14|14:13|15:7,16|16:15,17|17:16|18:7,25|19:24,47,67|20:23,34|21:24,37|"
            + "22:24,42|23:20,24|24:19,21,22,23|25:18,26,33|26:25,27|27:26,28|28:27|29:30,35|30:29,31|"
            + "31:30,32|32:31|33:25,34,35,36,46|34:20,33|35:29,33|36:33|37:21,38|38:37,39|39:38,40|"
            + "40:39,41|41:40,51|42:22,43|43:42,44|44:43,45|45:44,52|46:33|47:19|50:1|51:41|52:45,62|"
            + "62:52|63:64,65,70,72|64:63,66,70,71|65:63,68|66:64,69|67:19,68,69,70|68:65,67|69:66,67|"
            + "70:63,64,67|71:64,73|72:63,73|73:71,72,74|74:73,75|75:74,76|76:75,77|77:76,78|78:77,79|"
            + "79:78,92|92:79,93|93:92,94|94:93,95|95:94,96|96:95,97|97:96,98|98:97|127:|135:1,127";

    private static final int MAP_MAX = 136;
    private static int[][] mapAdj = null;

    /** Parses {@link #MAP_ADJ} once. Null rows are maps the survey never reached. */
    private static int[][] adjacency() {
        if (mapAdj != null) {
            return mapAdj;
        }
        int[][] table = new int[MAP_MAX][];
        int from2 = 0;
        while (from2 < MAP_ADJ.length()) {
            int end = MAP_ADJ.indexOf('|', from2);
            if (end < 0) {
                end = MAP_ADJ.length();
            }
            String row = MAP_ADJ.substring(from2, end);
            from2 = end + 1;
            int colon = row.indexOf(':');
            if (colon <= 0) {
                continue;
            }
            int id = num(row.substring(0, colon));
            String list = row.substring(colon + 1);
            if (id < 0 || id >= MAP_MAX) {
                continue;
            }
            int count = list.length() == 0 ? 0 : 1;
            for (int i = 0; i < list.length(); i++) {
                if (list.charAt(i) == ',') {
                    ++count;
                }
            }
            int[] row2 = new int[count];
            int at = 0;
            int cut = 0;
            while (at < count) {
                int comma = list.indexOf(',', cut);
                if (comma < 0) {
                    comma = list.length();
                }
                row2[at++] = num(list.substring(cut, comma));
                cut = comma + 1;
            }
            table[id] = row2;
        }
        mapAdj = table;
        return mapAdj;
    }

    /** The first map to walk into on the way to the goal, or −1 when there is no walking route. */
    private static int mapNextHop(int from, int to) {
        if (from == to || from < 0 || to < 0 || from >= MAP_MAX || to >= MAP_MAX) {
            return -1;
        }
        int[][] adj = adjacency();
        if (adj[from] == null) {
            return -1;
        }
        // One traced BFS, not one BFS per neighbour. The previous shape asked mapDistance() for every
        // neighbour, so a degree-5 map ran five full searches to learn one step; walking back through
        // `prev` gives the same answer from a single pass.
        int[] prev = new int[MAP_MAX];
        for (int i = 0; i < MAP_MAX; i++) {
            prev[i] = -2;               // -2 unseen, -1 is the source's own marker
        }
        int[] queue = new int[MAP_MAX];
        int head = 0;
        int tail = 0;
        queue[tail++] = from;
        prev[from] = -1;
        while (head < tail) {
            int at = queue[head++];
            int[] next = adj[at];
            if (next == null) {
                continue;
            }
            for (int i = 0; i < next.length; i++) {
                int step = next[i];
                if (step < 0 || step >= MAP_MAX || prev[step] != -2) {
                    continue;
                }
                prev[step] = at;
                if (step == to) {
                    // Walk back to the map that borders `from`: that is the hop to take now.
                    int cursor = step;
                    while (prev[cursor] != from) {
                        cursor = prev[cursor];
                    }
                    return cursor;
                }
                queue[tail++] = step;
            }
        }
        return -1;
    }

    /**
     * Breadth-first search, returning the number of borders between two maps, or −1 if unreachable.
     *
     * Used to score a stone destination: fewer borders left to walk is better, and the goal scores
     * zero. Equal-cost is not a tie worth breaking — the first is as good as the last.
     */
    private static int mapDistance(int from, int to) {
        if (from == to) {
            return 0;
        }
        if (from < 0 || to < 0 || from >= MAP_MAX || to >= MAP_MAX) {
            return -1;
        }
        int[][] adj = adjacency();
        int[] depth = new int[MAP_MAX];
        for (int i = 0; i < MAP_MAX; i++) {
            depth[i] = -1;
        }
        int[] queue = new int[MAP_MAX];
        int head = 0;
        int tail = 0;
        queue[tail++] = from;
        depth[from] = 0;
        while (head < tail) {
            int at = queue[head++];
            int[] next = adj[at];
            if (next == null) {
                continue;
            }
            for (int i = 0; i < next.length; i++) {
                int to2 = next[i];
                if (to2 < 0 || to2 >= MAP_MAX || depth[to2] >= 0) {
                    continue;
                }
                depth[to2] = depth[at] + 1;
                if (to2 == to) {
                    return depth[to2];
                }
                queue[tail++] = to2;
            }
        }
        return -1;
    }

    /** The client's own map-name table, plus the eight ids it does not cover. */
    private static String mapName(int id) {
        try {
            if (id >= 0 && df.gE != null && id < df.gE.length) {
                String name = df.gE[id];
                return name != null && name.length() > 0 ? name : null;
            }
        } catch (Throwable t) {
            return null;
        }
        switch (id) {
            case 92: return "Cổng trắng";
            case 93: return "Thị trấn mùa đông";
            case 94: return "Thung lũng băng giá";
            case 95: return "Chân núi tuyết";
            case 96: return "Đèo băng giá";
            case 97: return "Vực thẳm sương mù";
            case 98: return "Trạm núi tuyết";
            case 135: return "Làng Phủ Sương";
            default: return null;
        }
    }

    /**
     * Resolves a menu label to a map id, or −1.
     *
     * Exact-after-normalising first, then containment, because the stone's labels and `df.gE` differ
     * in case and diacritics but not in wording. Two labels the stone offers — "Chợ nguyên liệu" and
     * "Khu mua bán đặc biệt" — are in no name table this client ships, so −1 is a real answer here
     * and not a bug: those destinations simply cannot be scored, and are skipped.
     */
    private static int mapByName(String label) {
        if (label == null) {
            return -1;
        }
        String want = norm(label.trim());
        if (want.length() == 0) {
            return -1;
        }
        for (int pass = 0; pass < 2; pass++) {
            for (int id = 0; id < MAP_MAX; id++) {
                String name = mapName(id);
                if (name == null) {
                    continue;
                }
                String have = norm(name.trim());
                if (have.length() == 0) {
                    continue;
                }
                if (pass == 0 ? have.equals(want) : have.indexOf(want) >= 0) {
                    return id;
                }
            }
        }
        return -1;
    }

    // ---- ENHANCE --------------------------------------------------------------
    //
    // Auto equipment upgrade ("Cường Hóa"). Its own module rather than a branch of ATTACK, for
    // the reason REVIVE already paid for: it has to run on a character that is not armed to
    // fight, and a module nested inside `attack()` never runs there.
    //
    // The switch, the settings and the published state are all wired. The walk is not, and that
    // is deliberate rather than unfinished-by-accident: docs/18-cuong-hoa-vs-ban-rac.md §2.4
    // found no enhance screen anywhere in the client, so the dialog is built from server-sent
    // text, and §3.2 lays out two ways to drive it that both begin with recording the real NPC
    // menu labels by hand. Guessing those labels would walk the character into a menu nothing
    // here can read, and a dialog left open then blocks every module that gates on ready().

    private static void enhance() {
        if (!enhanceOn) {
            return;
        }
        try {
            // The NPC menu this module has to drive is not measured yet: docs/18-cuong-hoa-vs-ban-rac.md
            // §3.2 records that both candidate paths start from recording the real label sequence by
            // hand, and guessing it would walk the character into a dialog nothing here can read.
            // Until then the switch is inert and reports itself as idle, so the tool shows "off"
            // rather than a phase that is not moving.
            enhancePhase = 0;
        } catch (Throwable t) {
            // A module must never stall the client tick.
        }
    }

    /** Forgets one trip. Called when the switch turns off; `allOff()` clears the same fields inline. */
    private static void enhanceReset() {
        enhancePhase = 0;
        enhanceWhy = 0;
        enhanceWait = 0;
    }

    // ---- end ENHANCE ----------------------------------------------------------

    // ---- NPC ENGINE -----------------------------------------------------------
    //
    // What every module that talks to an NPC needs and none of them should reinvent: find the
    // entity, send the client's own click, capture the reply, pick a row out of it. The shapes are
    // not new — `zoneBoard()` is the finder, `travelStone()` is the click-and-wait, and
    // `travelStoneMenu()` is the selection — so each helper below is that shape with the needle
    // changed rather than a second convention beside it.
    //
    // Two facts about server menus drive the whole engine, both recorded above at
    // {@link #serverMenu}: the selection is `q.a().b(idNPC, idMenu, index)` quoting the pair the
    // SERVER sent, because a server menu's buttons carry no command of their own; and pressing a
    // button selects whatever is highlighted (`fr.h`), which is private in this build. The second
    // is why nothing here sets a cursor: the index travels in the packet instead, which is the same
    // thing the client's own `fr.a(2, _)` sends.

    /**
     * The nearest NPC whose name matches, or the one carrying the fallback template id.
     *
     * The name is the match and the id is the fallback, not the other way round: the id this server
     * gives the dungeon guide is -37 today, but template ids differ per region the way the teleport
     * stones' do, so the name is what survives a client update. Nearest by Manhattan, because the
     * client's own reach checks are Manhattan and a mixed metric would call a farther NPC nearer.
     */
    private static fa dungeonNpc() {
        if (cn.j == null || cn.g == null) {
            return null;
        }
        fa best = null;
        fa fallback = null;
        int bestDistance = Integer.MAX_VALUE;
        for (int i = 0; i < cn.j.c(); i++) {
            Object entry = cn.j.a(i);
            if (!(entry instanceof fa)) {
                continue;
            }
            fa candidate = (fa) entry;
            // cv == 2 is an NPC; 0 is a player and 1 a monster.
            if (candidate.cv != 2) {
                continue;
            }
            if (candidate.cC != null && norm(candidate.cC).indexOf(DUNGEON_NPC_NAME) >= 0) {
                int distance = abs(cn.g.aZ - candidate.aZ) + abs(cn.g.ba - candidate.ba);
                if (distance < bestDistance) {
                    bestDistance = distance;
                    best = candidate;
                }
                continue;
            }
            if (fallback == null && candidate.cu == DUNGEON_NPC_CU) {
                fallback = candidate;
            }
        }
        return best != null ? best : fallback;
    }

    /**
     * Asks an NPC and arms the wait, exactly as {@link #travelStone} asks a teleport stone.
     *
     * Returns false only when the send itself failed. The reply is a packet and lands on a later
     * tick, so this never polls for it — it records the NPC's id, so the ask can be repeated
     * without the entity still being in the scene stream, and hands the tick back.
     */
    private static boolean dungeonClickNpc(fa npc) {
        dungeonNpcCu = npc.cu;
        return dungeonAskNpc();
    }

    /** Re-asks the NPC last clicked, by id. The bounded retry is a second question, not a tighter loop. */
    private static boolean dungeonAskNpc() {
        if (dungeonNpcCu == -1) {
            return false;
        }
        dungeonMenu = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        try {
            q.a().a((byte) dungeonNpcCu);
        } catch (Throwable t) {
            return false;
        }
        dungeonWait = 20;           // travel's measured budget; the menu usually lands well inside
        return true;
    }

    /**
     * The row of the captured menu whose label contains `needle`, or -1.
     *
     * `reject` is tested on the whole needle and never on a prefix, because the two labels this
     * module has to tell apart differ in their last letter after normalising: "Giao tiếp" becomes
     * "giao tiep" and "Giao dịch" becomes "giao dich". Excluding on "giao" would drop the dialogue
     * button along with the shop, and opening the shop instead of the dialogue is the documented
     * failure mode of this trip.
     */
    private static int dungeonMenuPick(String needle, String reject) {
        if (dungeonMenu == null) {
            return -1;
        }
        for (int i = 0; i < dungeonMenu.length; i++) {
            String label = norm(dungeonMenu[i]);
            if (label.indexOf(needle) < 0) {
                continue;
            }
            if (reject != null && label.indexOf(reject) >= 0) {
                continue;
            }
            return i;
        }
        return -1;
    }

    /**
     * Picks a row of the captured menu and forgets the capture.
     *
     * The menu is left on screen rather than dismissed: the server replaces the panel with the next
     * one, and closing it here would race that replacement. Only a menu this module has decided it
     * will not use is closed, by {@link #dungeonCloseMenu}.
     */
    private static boolean dungeonSelect(int index) {
        int npc = dungeonMenuNpc;
        int menuId = dungeonMenuId;
        dungeonMenu = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        try {
            // The idNPC/idMenu pair the server itself sent, not an assumed zero: this is the packet
            // fr.a(2, _) builds when the operator taps the row.
            q.a().b((short) npc, (byte) menuId, (byte) index);
        } catch (Throwable t) {
            return false;
        }
        return true;
    }

    /** Drops the panel this module opened. A menu left up blocks every module that gates on ready(). */
    private static void dungeonCloseMenu() {
        try {
            if (fu.p != null && fu.p.a) {
                fu.p.f();
            }
        } catch (Throwable t) {
            // A menu that will not close is not worth stalling the trip over.
        }
    }

    /**
     * Whether the character is inside the dungeon.
     *
     * The map id first and the client's own name as confirmation, because the id is survey data
     * about this server while the name comes from `df.gE` — index 48 is "Ngã tư tử thần", which
     * {@link #norm} reduces to "nga tu tu than". No other map in that table normalises to contain
     * "nga tu", so the name cannot false-positive onto a different map.
     */
    private static boolean dungeonInDungeon() {
        if (fu.q == null) {
            return false;
        }
        return fu.q.d == DUNGEON_MAP
                || norm(mapName(fu.q.d)).indexOf(DUNGEON_NAME) >= 0;
    }

    /**
     * The half-hour slot of day the wall clock is in, 0..47, or -1 when the clock is unreadable.
     *
     * `dx.a()` is the client's MONOTONIC clock and is useless here: a schedule is a time of day, so
     * this needs a wall clock. Calendar is in the MIDP profile and this file already reaches into
     * `java.io` by qualified name rather than by import, so the same style keeps the header alone.
     */
    private static int dungeonSlotNow() {
        try {
            java.util.Calendar clock = java.util.Calendar.getInstance();
            return clock.get(java.util.Calendar.HOUR_OF_DAY) * 2
                    + (clock.get(java.util.Calendar.MINUTE) >= 30 ? 1 : 0);
        } catch (Throwable t) {
            return -1;
        }
    }

    /** The day of year, or -1 when the clock is unreadable. What a fired schedule is stamped with. */
    private static int dungeonDayNow() {
        try {
            return java.util.Calendar.getInstance().get(java.util.Calendar.DAY_OF_YEAR);
        } catch (Throwable t) {
            return -1;
        }
    }

    /**
     * Whether a scheduled trip may start now.
     *
     * `slot >= dungeonSchedule` is the same test as comparing minutes against `schedule * 30`, and
     * the day stamp is what makes it fire once: a slot is half an hour WIDE, so without the stamp a
     * trip armed at 20:00 for slot 40 would start again on every tick until 20:30.
     *
     * An unreadable clock fails OPEN, deliberately. A character that never runs because the clock
     * threw is worse than one that runs early — the schedule is a convenience, not a safety limit,
     * and the alternative is a module that silently stops working with nothing in the trace to say
     * why.
     */
    private static boolean dungeonScheduleDue() {
        int slot = dungeonSlotNow();
        if (slot < 0) {
            return true;
        }
        if (dungeonDayNow() == dungeonScheduleDay) {
            return false;           // today's trip already started
        }
        return slot >= dungeonSchedule;
    }

    // ---- end NPC ENGINE -------------------------------------------------------

    // ---- DUNGEON --------------------------------------------------------------
    //
    // Auto Phó Bản "Ngã tư tử thần": walk to the dungeon guide on map 1, take the dialogue row and
    // then the dungeon row out of its two server menus, let the server teleport the character in,
    // and count a run when the character comes back. Its own module rather than a branch of ATTACK
    // for the reason REVIVE and ENHANCE already paid for: a trip is not a fight, and it has to run
    // on a character that is not armed to hold a spot.
    //
    // Two things this module deliberately does NOT do, so the gaps are stated rather than implied:
    //
    //   - It does not fight inside the dungeon and does not widen the scan radius. `attack()`
    //     releases the combat fields whenever `fu.q.d != atkMap`, and `combatOn()` writes
    //     `cn.g.bi = atkRadius` on every tick while combat is armed — so a radius set here would be
    //     overwritten later in the same tick, and making ATTACK fight on map 48 would mean editing
    //     another module's map gate. Fighting in the dungeon is therefore the operator arming a
    //     spot on map 48 with `atk.radius` set wide, which already works and needs no new code.
    //   - It does not exclude the "thien thach" monsters. Zeus never selects a target: `combatOn()`
    //     sets `bq.W`, a one-shot "catch the nearest target" flag the CLIENT consumes, and `cn.i`
    //     is client-owned — the only place this file writes it is `combatOff()`, nulling it to
    //     release. Honouring the exclusion would mean intercepting the client's own pick, which
    //     would fight `bq.W` rather than serve it. Recorded here as a known gap.

    /** Dungeon states. The same discipline as {@link #TV_OFF}: anything else is a reason to stop. */
    private static final int DN_OFF = 0, DN_IDLE = 1, DN_GOTO_NPC = 2, DN_INTERACT = 3,
            DN_IN_DUNGEON = 4, DN_DONE = 5;

    /** The dungeon's map id, and the two ways it is recognised. See {@link #dungeonInDungeon}. */
    private static final int DUNGEON_MAP = 48;
    private static final String DUNGEON_NAME = "nga tu";

    /** The guide's map, name and fallback template id. */
    private static final int DUNGEON_NPC_MAP = 1;
    private static final String DUNGEON_NPC_NAME = "pho chi huy";
    private static final int DUNGEON_NPC_CU = -37;
    /**
     * Where to walk when the guide is not in the scene stream at all.
     *
     * A degraded path rather than the normal one: {@link #dungeonNpc} finds the guide by name in
     * almost every tick, and this is only reached when the entity list has no NPC matching either
     * the name or the fallback id.
     */
    private static final int DUNGEON_NPC_X = 552;
    private static final int DUNGEON_NPC_Y = 504;

    /** Asks before giving up on anything. Three is the budget KnightMod's own module used. */
    private static final int DN_MAX_TRIES = 3;
    /**
     * Ticks with no movement at all before a walk is called a stall: 2.4 s at the loop's 25 ticks/s,
     * the same order {@link #travel} uses. A path that never completes is a stall, not a walk.
     */
    private static final int DN_STALL_TICKS = 60;
    /**
     * Ticks to stand still after a run before asking for the next, and the ceiling the snapshot
     * contract puts on the tally: `dungeonruns` is rejected outright past 1000 rather than clamped,
     * so counting past it would break the whole snapshot instead of one field.
     */
    private static final int DN_BETWEEN_RUNS = 100;
    private static final int DUNGEON_RUNS_MAX = 1000;

    /**
     * DUNGEON's own settings and state.
     *
     * `dungeonMaxRuns` and `dungeonSchedule` both take -1 as off, because 0 is a real value inside
     * each range. `dungeonWait` is a tick budget in the shape of {@link #travelWait}: a menu reply
     * is a packet, so the module arms a deadline and hands the tick back instead of polling.
     * `dungeonTried` bounds every ask, in the shape of {@link #travelStoneTried}. `dungeonStep`
     * says which of the NPC's two menus is expected next: 0 the dialogue, 1 the dungeon row.
     * `dungeonWasIn` is what makes a run countable — leaving map 48 means nothing on its own, since
     * a death town-port leaves it too.
     */
    private static boolean dungeonEnabled = false;
    private static int dungeonMaxRuns = -1;
    private static int dungeonSchedule = -1;
    private static int dungeonState = DN_OFF;
    private static int dungeonWhy = 0;
    private static int dungeonRuns = 0;
    private static int dungeonWait = 0;
    private static int dungeonTried = 0;
    private static int dungeonStep = 0;
    private static boolean dungeonWasIn = false;
    private static int dungeonNpcCu = -1;
    private static int dungeonScheduleDay = -1;
    /** True between the first ask of a trip and its run limit, so the schedule gates the trip and not each run. */
    private static boolean dungeonTripActive = false;
    private static int dungeonStallTicks = 0;
    private static int dungeonLastX = Integer.MIN_VALUE;
    private static int dungeonLastY = Integer.MIN_VALUE;
    private static int dungeonMapSeen = Integer.MIN_VALUE;

    /** Labels of the server menu currently open, captured by {@link #serverMenu}. Its own trio: never TRAVEL's. */
    private static String[] dungeonMenu = null;
    private static int dungeonMenuNpc = Integer.MIN_VALUE;
    private static int dungeonMenuId = 0;

    /**
     * Forgets one trip.
     *
     * `dungeonRuns` survives, because a tally of what already happened is not a piece of in-flight
     * state — `enhanceDone`'s rule. The switch turning back on clears it separately, since a limit
     * of three runs left over from the last trip would be a limit of three runs already spent.
     */
    private static void dungeonReset() {
        dungeonState = DN_OFF;
        dungeonWhy = 0;
        dungeonWait = 0;
        dungeonTried = 0;
        dungeonStep = 0;
        dungeonWasIn = false;
        dungeonTripActive = false;
        dungeonNpcCu = -1;
        dungeonScheduleDay = -1;
        dungeonStallTicks = 0;
        dungeonLastX = Integer.MIN_VALUE;
        dungeonLastY = Integer.MIN_VALUE;
        dungeonMapSeen = Integer.MIN_VALUE;
        dungeonMenu = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
    }

    /** Stops and says why, in the shape of {@link #travelStop}. `dungeonWhy != 0` is what holds DN_OFF. */
    private static void dungeonStop(int why, String reason) {
        dungeonState = DN_OFF;
        dungeonWhy = why;
        dungeonTripActive = false;
        // A stop mid-conversation leaves a panel up that nothing will read, and a panel left up
        // blocks every module that gates on ready().
        dungeonCloseMenu();
        dungeonMenu = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        trace("DUNGEON stopped (" + why + "): " + reason);
    }

    /**
     * One step of the trip, or nothing at all.
     *
     * Runs outside `items()`, which gates on ready(): ready() requires no dialog, and an NPC menu
     * is exactly the state this module has to act in. Gated there it would wait for the operator to
     * dismiss a menu it had opened itself — the bug that moved `drops()` and `zone()` out.
     */
    private static void dungeon() {
        try {
            if (!dungeonEnabled) {
                // Off is a state rather than a no-op: a menu captured on the way down has to be
                // dropped with it, or it stays up blocking everything that gates on ready().
                if (dungeonState != DN_OFF || dungeonMenuNpc != Integer.MIN_VALUE) {
                    dungeonCloseMenu();
                    dungeonReset();
                }
                return;
            }
            if (!inGame() || !sceneReady() || !alive() || captcha()
                    || cn.g == null || fu.q == null) {
                return;             // keep the intent; a load screen is not a failure
            }
            // Deliberately NOT ready(): that requires no dialog, and this module lives inside NPC
            // menus. Gating on it would deadlock on the first reply.
            int here = fu.q.d;
            if (here != dungeonMapSeen) {
                dungeonMapSeen = here;
                dungeonWait = 12;   // let the scene settle before reading it
                dungeonStallTicks = 0;
                dungeonLastX = Integer.MIN_VALUE;
                dungeonLastY = Integer.MIN_VALUE;
                // Any walk from the previous map is void, and the movement lock outlives it: left
                // set, canMove() stays false forever and every module that walks silently stops.
                // Released only while THIS module is the one walking — a lock TRAVEL holds belongs
                // to TRAVEL, and taking it would stop a route the operator armed.
                if (dungeonState == DN_GOTO_NPC) {
                    bq.m = false;
                    cn.g.cO = null;
                }
            }
            // Entry and exit are read before the wait budget, not after it: the teleport sets that
            // budget on the same tick it changes the map, and a run counted twelve ticks late is a
            // run counted after the next ask has already gone out.
            if (dungeonInDungeon()) {
                dungeonWasIn = true;
                dungeonTried = 0;
                if (dungeonState != DN_IN_DUNGEON) {
                    dungeonState = DN_IN_DUNGEON;
                    dungeonWhy = 0;
                    trace("DUNGEON entered map " + here);
                }
                // ATTACK fights here when a spot is armed on this map; this module has nothing to
                // add and does not duplicate a target selection it does not own.
                return;
            }
            if (here == DUNGEON_NPC_MAP && dungeonWasIn) {
                // Back on the guide's map having been inside: that is a finished run. Any other map
                // would count a death town-port or a manual walk as one.
                dungeonWasIn = false;
                dungeonState = DN_DONE;
                return;
            }
            if (dungeonWait > 0) {
                --dungeonWait;
                return;
            }
            switch (dungeonState) {
                case DN_OFF:
                    // A non-zero why is a deliberate stop, not a module that never started. Holding
                    // here is what "never loop" means; re-arming is the operator's switch.
                    if (dungeonWhy == 0) {
                        dungeonState = DN_IDLE;
                    }
                    return;
                case DN_IDLE:
                    dungeonIdle();
                    return;
                case DN_GOTO_NPC:
                    dungeonGotoNpc(here);
                    return;
                case DN_INTERACT:
                    dungeonInteract();
                    return;
                case DN_DONE:
                    dungeonDone();
                    return;
                default:
                    dungeonStop(3, "unknown state " + dungeonState);
            }
        } catch (Throwable t) {
            // A module must never stall the client tick.
        }
    }

    /** Leaves IDLE when the run limit allows it and the schedule, if any, has come round. */
    private static void dungeonIdle() {
        if (dungeonMaxRuns != -1 && dungeonRuns >= dungeonMaxRuns) {
            dungeonStop(4, "run limit of " + dungeonMaxRuns + " already reached");
            return;
        }
        // The schedule gates the TRIP, not each run: once a trip is under way it runs to its limit,
        // which is what makes "ten runs at 20:00" mean ten runs and not one.
        if (!dungeonTripActive) {
            if (dungeonSchedule >= 0) {
                if (!dungeonScheduleDue()) {
                    return;         // still before the slot; IDLE is the honest state to publish
                }
                // Stamped when the trip starts, so it cannot re-fire today. -1 on an unreadable
                // clock, which simply leaves the gate open — the fail-open choice above.
                dungeonScheduleDay = dungeonDayNow();
            }
            dungeonTripActive = true;
        }
        dungeonState = DN_GOTO_NPC;
        dungeonTried = 0;
        dungeonStep = 0;
        dungeonStallTicks = 0;
        dungeonNpcCu = -1;
        dungeonWhy = 0;
        trace("DUNGEON starting a run (done=" + dungeonRuns + " max=" + dungeonMaxRuns + ")");
    }

    /** Walks to the guide and asks it. Holds rather than routing when the character is elsewhere. */
    private static void dungeonGotoNpc(int here) {
        if (here != DUNGEON_NPC_MAP) {
            // Not the guide's map, and no route is invented here. TRAVEL owns the movement lock and
            // the operator arms `nav.target` separately; two modules pathing at once is how a
            // character ends up walked somewhere neither of them asked for. Hold, and say why.
            if (dungeonWhy != 1) {
                dungeonWhy = 1;
                trace("DUNGEON needs map " + DUNGEON_NPC_MAP + ", standing on " + here
                        + "; waiting for TRAVEL");
            }
            return;
        }
        if (dungeonWhy != 0) {
            dungeonWhy = 0;
        }
        fa npc = dungeonNpc();
        int x = DUNGEON_NPC_X;
        int y = DUNGEON_NPC_Y;
        if (npc != null) {
            x = npc.aZ;
            y = npc.ba;
        }
        if (!travelArrive(x, y, 80)) {
            // The client's own pathfinder, through the helper TRAVEL uses. No movement at all for a
            // whole budget is a stall: the pathfinder can hand back a route that never completes,
            // and without this the state would never change and the tool would show a walk that is
            // not walking.
            if (cn.g.aZ == dungeonLastX && cn.g.ba == dungeonLastY) {
                if (++dungeonStallTicks > DN_STALL_TICKS) {
                    dungeonStop(3, "stalled walking to the dungeon NPC on map " + here);
                    return;
                }
            } else {
                dungeonStallTicks = 0;
                dungeonLastX = cn.g.aZ;
                dungeonLastY = cn.g.ba;
            }
            travelMove(here, x, y);
            return;
        }
        if (npc == null) {
            // At the surveyed spot with nothing to talk to. Bounded, then a reason: waiting forever
            // on an NPC that is not in the scene stream is the failure this file records for TRAVEL.
            if (++dungeonTried >= DN_MAX_TRIES) {
                dungeonStop(1, "no dungeon NPC on map " + here);
                return;
            }
            dungeonWait = 20;
            return;
        }
        if (dungeonClickNpc(npc)) {
            dungeonState = DN_INTERACT;
            dungeonStep = 0;
            dungeonTried = 0;
            dungeonStallTicks = 0;
            trace("DUNGEON asked NPC cu=" + npc.cu + " at " + npc.aZ + "," + npc.ba);
        } else if (++dungeonTried >= DN_MAX_TRIES) {
            dungeonStop(2, "the dungeon NPC would not take a click");
        }
    }

    /**
     * Drives the NPC's two menus: the dialogue row, then the dungeon row.
     *
     * Both are read out of the labels {@link #serverMenu} captured rather than out of a hard-coded
     * index, because the index is whatever position the server put the row in and the server is the
     * only thing that knows. Each ask arms a deadline and hands the tick back; the reply lands in
     * the capture branch and is acted on here on a later tick.
     */
    private static void dungeonInteract() {
        if (dungeonMenu == null) {
            if (dungeonWait > 0) {
                --dungeonWait;
                return;
            }
            if (++dungeonTried >= DN_MAX_TRIES) {
                dungeonStop(2, dungeonStep == 0 ? "the dungeon NPC gave no menu"
                        : "no submenu after the dialogue pick");
                return;
            }
            if (dungeonStep != 0) {
                // The submenu never came. Start the conversation over from the NPC rather than
                // poking a panel this module can no longer address: re-selecting the open menu is
                // what KnightMod did by setting its cursor, and that field (`fr.h`) is private in
                // this build. Asking the NPC again is the same conversation over a packet that
                // exists here.
                dungeonStep = 0;
            }
            dungeonAskNpc();
            return;
        }
        // The dialogue row first and the shop never: "giao dich" is the exclusion, tested on the
        // whole word. Opening the shop instead is the documented failure mode of this trip.
        String needle = dungeonStep == 0 ? "giao tiep" : DUNGEON_NAME;
        String reject = dungeonStep == 0 ? "giao dich" : null;
        int pick = dungeonMenuPick(needle, reject);
        if (pick < 0) {
            // Not the menu that was asked for — the operator opened something, or the server sent a
            // page this trip has no use for. Closed rather than left up, and bounded.
            dungeonCloseMenu();
            dungeonMenu = null;
            dungeonMenuNpc = Integer.MIN_VALUE;
            if (++dungeonTried >= DN_MAX_TRIES) {
                dungeonStop(2, "no \"" + needle + "\" row in the dungeon NPC's menu");
                return;
            }
            dungeonWait = 10;
            return;
        }
        if (!dungeonSelect(pick)) {
            dungeonStop(2, "selecting \"" + needle + "\" failed");
            return;
        }
        if (dungeonStep == 0) {
            dungeonStep = 1;
            dungeonTried = 0;
            dungeonWait = 20;
            trace("DUNGEON picked the dialogue, waiting on the submenu");
            return;
        }
        // The teleport is the server's to make. Nothing to poll: the map change ends this state, and
        // the budget is only how long to wait before calling that a failure.
        dungeonTried = 0;
        dungeonWait = 40;
        trace("DUNGEON asked for the dungeon, waiting on the teleport");
    }

    /** Counts the run, then either stops at the limit or goes round again. */
    private static void dungeonDone() {
        if (dungeonRuns < DUNGEON_RUNS_MAX) {
            ++dungeonRuns;
        }
        trace("DUNGEON run " + dungeonRuns + " complete");
        if (dungeonMaxRuns != -1 && dungeonRuns >= dungeonMaxRuns) {
            // Stops, and stays stopped. Looping back for one more run past a limit the operator set
            // is the one failure this module must not have.
            dungeonStop(4, "run limit of " + dungeonMaxRuns + " reached");
            return;
        }
        dungeonState = DN_IDLE;
        dungeonWait = DN_BETWEEN_RUNS;
        dungeonTried = 0;
        dungeonStep = 0;
        dungeonNpcCu = -1;
    }

    // ---- end DUNGEON ----------------------------------------------------------

    // ---- ATTACK ---------------------------------------------------------------
    //
    // Zeus does not pick targets or swing. It sets the client's own auto fields and lets
    // bq.q() choose and bq's loop attack, which is why this module is small. What it adds
    // is the anchor: the spot the operator captured, written into bq.R/S so the client's
    // three uses of the anchor — pull back, retarget, repath — all serve that spot.

    /** State machine of docs/core/08 §7, reduced to the three states that stay on one map. */
    private static final int FIGHTING = 0;
    private static final int TO_SPOT = 1;
    private static final int SETTLE = 2;
    private static int atkState = FIGHTING;
    /** Ticks left in SETTLE. KnightMod's number, kept. */
    private static int settleTicks = 0;

    /** Drift that sends a standing character back, in pixels. KnightMod's number. */
    private static final int STAND_DRIFT = 30;
    /**
     * How close stand mode has to get before it counts as arrived.
     *
     * Tighter than {@link #STAND_DRIFT}, which is the leash that decides when to walk BACK: arriving
     * within the leash would leave the character standing wherever it stopped, up to 30 px from the
     * place the operator recorded, and pinning there publishes that spot to the server instead.
     */
    private static final int ARRIVE_DRIFT = 8;
    /** Drift that sends a moving character back. KnightMod's number. */
    private static final int MOVE_DRIFT = 280;
    /** Client's own scan radius, restored when auto goes off. */
    private static final int NATIVE_RADIUS = 140;

    /** True while this module owns the combat fields, so off() runs exactly once. */
    private static boolean combatOwned = false;
    /** bq.l as it was before this module turned the native potion pump off. */
    private static boolean nativePotionWas = false;

    /** 0 fine, 1 no monsters in range, 2 monsters but cannot reach or hit them. */
    private static int stuckKind = 0;
    private static int noTargetTicks = 0;
    private static int noFightTicks = 0;
    private static int pickedCount = 0;
    private static int potionCount = 0;

    /**
     * One character per buff slot for what the client will really cast.
     *
     * Not an echo of the setting: a slot the character has not learned reads 0 here even when the
     * operator asked for it, which is the difference between "buff is off" and "you do not have
     * that buff yet".
     */
    private static String buffState() {
        StringBuffer out = new StringBuffer(BUFF_SLOTS);
        for (int i = 0; i < BUFF_SLOTS; i++) {
            boolean on = ah.f != null
                    && i < ah.f.length
                    && ah.f[i] != null
                    && ah.f[i].length >= 2
                    && ah.f[i][1] == 1;
            out.append(on ? '1' : '0');
        }
        return out.toString();
    }

    private static void attack() {
        try {
            if (atkMode == 0 || atkX < 0 || atkY < 0) {
                combatOff();
                return;
            }
            // Death releases the combat fields and nothing else: revive() runs from tick(), before
            // this, so it works whether or not a spot is armed. Bailing on ready() instead left
            // `combatOff()` unreached and the module went on claiming it owned those fields while
            // the character lay on the ground.
            if (cn.g != null && cn.g.cG == 4) {
                combatOff();
                return;
            }
            if (!gameReady() || cn.g == null) {
                return;             // keep the intent; just do nothing this tick
            }
            // A different map is out of scope this round: the character stays put rather
            // than being walked somewhere by a travel table nobody has driven yet.
            if (fu.q == null || fu.q.d != atkMap) {
                combatOff();
                return;
            }
            diagnose();
            switch (atkState) {
                case TO_SPOT:
                    toSpot();
                    return;
                case SETTLE:
                    if (--settleTicks <= 0) {
                        atkState = FIGHTING;
                    }
                    return;
                default:
                    // Arriving parks the character; fighting is a second decision. Without it the
                    // walker still delivers the character to the spot and then stands there, which is
                    // what "go there, do not farm" has to mean.
                    if (!atkFarmOnArrival) {
                        combatOff();
                        return;
                    }
                    fighting();
            }
        } catch (Throwable t) {
            // A mod failure must never stall the client tick.
        }
    }

    /**
     * Ticks the frame loop runs in a second.
     *
     * `com.silverknight.a.run()` calls the tick and then sleeps to a 40 ms frame, so this is a
     * ceiling rather than a guarantee: a client that cannot keep up runs fewer. Every "seconds"
     * setting is therefore a floor on the wait, which is the safe direction for a delay.
     */
    private static final int TICKS_PER_SECOND = 25;

    /** Ticks between attempts. The frame loop sleeps to 40 ms, so 60 ticks is 2.4 s. */
    private static final int REVIVE_EVERY = 60;
    /** Ticket attempts per death before falling through to town. KnightMod's own ceiling. */
    private static final int REVIVE_TRIES = 3;
    /** Ticks dead before the blocking UI is cleared, once. 1.6 s at the loop's 25 ticks/s. */
    private static final int UI_CLEAR_TICKS = 40;
    /** Seconds to lie there before the first attempt. The operator's own pause; 0 is immediate. */
    private static int reviveDelay = 0;

    private static int reviveWait = 0;
    private static int reviveCount = 0;
    /** Ticks spent dead in the current death. */
    private static int reviveDead = 0;
    /** Ticket attempts already sent in the current death. */
    private static int reviveTries = 0;
    /** True once this death found no ticket, so the operator is told exactly once. */
    private static boolean reviveNoTicket = false;
    /** True once this death cleared the UI, so it is cleared once instead of every tick. */
    private static boolean reviveCleared = false;

    /** Forgets one death. Called when the character is up again, and when settings are dropped. */
    private static void reviveReset() {
        reviveWait = 0;
        reviveDead = 0;
        reviveTries = 0;
        reviveNoTicket = false;
        reviveCleared = false;
    }

    /**
     * Its own module: sees the character die, waits the operator's pause, gets it back up.
     *
     * Runs from `tick()` and depends on nothing else — not `atk.mode`, not a saved spot, not
     * `ready()`. It used to live inside `attack()`, which returns at its first line when auto is
     * off, so a character configured only to revive stayed on the ground. Reviving is not part of
     * holding a spot; it is what makes every other module able to resume.
     *
     * Two paths, both the client's own senders. A ticket puts the character back where it fell,
     * which is the only one that lets fighting resume without travelling; town works but leaves it
     * off the spot. Running out of tickets falls through to town rather than lying there forever,
     * and says so once.
     *
     * It deliberately does NOT rewrite its own settings: `control()` re-reads the file every
     * CTL_EVERY_MS, so anything changed from in here is overwritten within half a second and the
     * operator's tool would be showing something the jar is not doing.
     */
    private static void revive() {
        // Alive: forget the death, so the next one starts from zero rather than from wherever the
        // last one left the counters. Held here rather than in a caller for the same reason the
        // module moved: a reset that only runs when auto is armed is a reset that usually does not.
        if (cn.g == null || cn.g.cG != 4) {
            reviveReset();
            return;
        }
        if (!reviveOn) {
            return;
        }
        ++reviveDead;
        // A menu or dialog that was open when the character fell does not stop the revive: this
        // module does not gate on ready(), so the packet goes out either way. What it stops is
        // everything afterwards: ready() stays false once the character is up, so attack() and
        // items() both do nothing, with no error anywhere to say why. Cleared on its own schedule
        // rather than the delay's: a menu frozen over the screen is worth closing whether or not
        // the operator asked to lie there a while first.
        if (reviveDead >= UI_CLEAR_TICKS && !reviveCleared) {
            reviveCleared = true;
            clearBlockingUi();
            trace("REVIVE cleared UI dead=" + reviveDead);
        }
        // The operator's own pause before the first attempt. Zero sends on the first tick dead.
        if (reviveDead < reviveDelay * TICKS_PER_SECOND) {
            return;
        }
        if (reviveWait > 0) {
            --reviveWait;
            return;
        }
        reviveWait = REVIVE_EVERY;
        try {
            if (reviveMode == 1 && !reviveNoTicket && reviveTries < REVIVE_TRIES) {
                bw ticket = reviveTicket();
                if (ticket != null) {
                    // Opcode -30 with the client's own virtual NPC id for reviving on the spot.
                    q.a().b((short) -51, (byte) 0, (byte) 0);
                    ++reviveTries;
                    ++reviveCount;
                    trace("REVIVE ticket id=" + ticket.O + " try=" + reviveTries
                            + " dead=" + reviveDead);
                    return;
                }
                reviveNoTicket = true;
                note("Zeus: hết vé hồi sinh, tự về làng.");
                trace("REVIVE no ticket dead=" + reviveDead);
            }
            // Opcode 31: give up the corpse and wake in town.
            q.a().b((byte) 0);
            ++reviveCount;
            trace("REVIVE town dead=" + reviveDead + " tries=" + reviveTries);
        } catch (Throwable t) {
            // A dead socket must not stall the tick; the next period tries again.
        }
    }

    /**
     * One line in the client's own message ticker.
     *
     * `fu.b(String)` queues into `cn.k`, and `cf.c()` (cf.java:902) draws from that queue — which
     * `fu.b()` pumps every tick, whatever screen is up. `fu.c(String)` would have been the wrong
     * call: it writes the single `cn.r.G` slot, so the next message overwrites this one before it
     * has been read. It has one side effect: `fu.b` stamps `fu.au`, which the client's ten-minute
     * tip timer measures from (fu.java:296), so a notice postpones one tip. Cheap at this rate.
     */
    private static void note(String text) {
        try {
            fu.b(text);
        } catch (Throwable t) {
            // A message nobody can see is not worth failing a revive over.
        }
    }

    /**
     * Closes whatever UI is blocking, the way the client's own Back does.
     *
     * NOT `fu.p = null`, which is what the mod this was compared against does: `fu.p` is assigned
     * exactly once (fu.java:87) and `fu.b()` dereferences it bare every frame (fu.java:245, :279),
     * as does the rest of the client — nulling it is an NPE in the paint loop. `fu.s` and `fu.t`
     * are different: the client nulls those itself (ah.java:364, dz.java:36, fu.j()).
     *
     * The dialog is dropped, not answered. `medalDialog()` presses a button only after matching the
     * text; a dialog that happened to be up when the character fell could be anything, and pressing
     * its first button answers a question nobody read.
     */
    private static void clearBlockingUi() {
        try {
            if (fu.p != null && fu.p.a) {
                fu.p.f();
                fu.m();
            }
            fu.s = null;
            fu.t = null;
        } catch (Throwable t) {
            // UI that will not close is not worth failing the revive over.
        }
    }

    /**
     * The revive ticket in the bag, or null.
     *
     * Matched by name because a dropped-in ticket has no id this jar can know in advance. That is
     * fragile by nature — the server rewording it silently disables revival — so the name is the
     * one place this module accepts a string match, and the id it finds is traced when it is used.
     */
    private static bw reviveTicket() {
        if (bw.V == null) {
            return null;
        }
        for (int index = 0; index < bw.V.c(); index++) {
            Object entry = bw.V.a(index);
            if (!(entry instanceof bw)) {
                continue;
            }
            bw item = (bw) entry;
            if (item.g != null && norm(item.g).indexOf("hoi sinh tai cho") >= 0) {
                return item;
            }
        }
        return null;
    }

    /** Manhattan drift from the spot, which is what both KnightMod thresholds measure. */
    private static int drift() {
        return Math.abs(cn.g.aZ - atkX) + Math.abs(cn.g.ba - atkY);
    }

    private static void fighting() {
        int limit = atkMode == 1 ? STAND_DRIFT : MOVE_DRIFT;
        if (drift() > limit) {
            combatOff();
            cn.i = null;
            atkState = TO_SPOT;
            return;
        }
        if (atkMode == 1) {
            // Pin all six position fields. This makes bc/bd zero, which turns the client's
            // periodic resync into the only way the server learns the position — a
            // deliberate consequence, docs/core/08 §4.1.
            cn.g.aZ = atkX;
            cn.g.ba = atkY;
            cn.g.bg = atkX;
            cn.g.bh = atkY;
            cn.g.bc = 0;
            cn.g.bd = 0;
        }
        combatOn();
        skills();
    }

    /** Sets the client's own auto fields and anchors them on the captured spot. */
    private static void combatOn() {
        bq.o = (byte) 1;
        bq.Y = true;
        // Not a counter: a one-shot "catch the nearest target" flag the client consumes.
        // Forcing it true every tick is how KnightMod keeps asking, and it is correct.
        bq.W = true;
        bq.R = atkX;
        bq.S = atkY;
        cn.g.bi = atkRadius;
        if (!combatOwned) {
            combatOwned = true;
            // One pump, not two. The client's own gate is bq.l; with both running, which
            // mechanism drank is unknowable. docs/core/08 §5.3 option (c).
            nativePotionWas = bq.l;
            bq.l = false;
            syncNativeSettings();
        }
    }

    /** Releases every field this module took, and only if it took them. */
    private static void combatOff() {
        if (!combatOwned) {
            return;
        }
        combatOwned = false;
        bq.o = (byte) -1;
        bq.Y = false;
        bq.W = false;
        cn.i = null;
        if (cn.g != null) {
            cn.g.bi = NATIVE_RADIUS;
        }
        bq.l = nativePotionWas;
        syncNativeSettings();
        atkState = FIGHTING;
        stuckKind = 0;
        noTargetTicks = 0;
        noFightTicks = 0;
    }

    /**
     * Tells the client and the server which settings are in force.
     *
     * Everything here is a field the client already owns and already syncs: thresholds, the potion
     * gate, the pickup record and the buff slots. Zeus writes them and calls the client's own
     * `co.b()` — it does not keep a parallel copy, because two copies of a setting is how nobody can
     * say which one collected an item or drank a potion.
     *
     * The thresholds are server state: change `ah.e[]` without this call and the next server push
     * overwrites it (docs/core/08 §5.2). The native menu only moves in tens, so the free threshold
     * is rounded for display while the mod itself uses the exact one.
     */
    private static void syncNativeSettings() {
        try {
            if (ah.e != null && ah.e.length >= 2) {
                ah.e[0] = round10(atkHpPct);
                ah.e[1] = round10(atkMpPct);
            }
            applyPickup();
            applyBuffs();
            co.b();
        } catch (Throwable t) {
            // co.b() swallows its own failures; this guards the array accesses around it.
        }
    }

    /**
     * Writes the pickup record exactly the way the game's own menu writes it.
     *
     * `ah.java:213` is the model: `bq.q = new be((byte) rank, R[1], R[2])`, where option index 5 of
     * the client's own equipment list means "don't pick equipment" and is stored as −1. Reproducing
     * that call rather than assembling the bytes by hand matters, because `be`'s constructor swaps
     * its last two arguments (`be.java:12-16`: `a=by2; b=by4; c=by3`). So the menu's layout is
     * a = rank, c = MP/HP mode, b = gold mode — and that is the layout the collector's own filter
     * reads: `bq.java:541` gates equipment on `q.a`, and `bq.java:546-552` gates the two potion
     * kinds on `q.c` against `be.e`/`be.f`.
     *
     * The client is inconsistent about this and Zeus deliberately does not try to be smarter. Going
     * out, `co.b()` serialises `q.a, q.b, q.c` (co.java:118-124). Coming back, `co.java:52` rebuilds
     * `new be(o[4], o[5], o[6])`, which lands byte 5 in `c` and byte 6 in `b` — the reverse. So a
     * server echo of the settings packet swaps MP/HP with gold, and the client's own summary text
     * (co.java:59-61) is written for the echoed layout while its filter is written for the menu's.
     * One of the two is wrong in vanilla.
     *
     * Zeus therefore writes the menu's layout, which is the one the filter honours, and publishes
     * `bq.q` read back live in the snapshot. If an echo ever does swap them, the panel shows MP/HP
     * and gold exchanged relative to what was configured, rather than the tool quietly claiming a
     * setting the client is not using.
     */
    private static void applyPickup() {
        // The client treats a null record as "collector off", so an all-off configuration is
        // expressed the same way rather than as three separate "don't" values.
        if (itemRank >= 5 && itemMpHp >= 3 && itemGold >= 1) {
            bq.q = null;
            return;
        }
        int rank = itemRank < 5 ? itemRank : -1;
        bq.q = new be((byte) rank, (byte) itemMpHp, (byte) itemGold);
    }

    /**
     * Turns the client's own buff slots on or off.
     *
     * The cast loop already exists at `bq.java:615-626`; it runs whenever `bq.p == 1` and uses the
     * same `bq.j` predicate skill rotation uses. So there is nothing to write here beyond the
     * flags. A slot the character has not learned is left alone: `ah.b` bounds the array and
     * `bq.I[skill] > 0` is what the native menu itself checks before enabling one.
     */
    private static void applyBuffs() {
        if (ah.f == null) {
            return;
        }
        int slots = Math.min(BUFF_SLOTS, Math.min(ah.b, ah.f.length));
        boolean any = false;
        for (int i = 0; i < slots; i++) {
            if (ah.f[i] == null || ah.f[i].length < 2) {
                continue;
            }
            boolean learned = bq.I != null
                    && ah.f[i][0] >= 0
                    && ah.f[i][0] < bq.I.length
                    && bq.I[ah.f[i][0]] > 0;
            boolean on = atkBuff[i] && learned;
            ah.f[i][1] = on ? 1 : 0;
            any |= on;
        }
        bq.p = (byte) (any ? 1 : 0);
    }

    private static int round10(int percent) {
        int rounded = (percent + 5) / 10 * 10;
        if (rounded < 10) {
            rounded = 10;
        }
        if (rounded > 90) {
            rounded = 90;
        }
        return rounded;
    }

    /**
     * Walks back to the spot, then waits for the scene to settle.
     *
     * Pathfinder argument order is destination first, current position second — the one
     * documented mistake that sends the character the opposite way (docs/core/08 §7.2).
     * A path longer than the cap is a failure the caller must reject, not a route.
     */
    private static void toSpot() {
        // Stand mode has to reach the recorded place, not merely its neighbourhood: the operator chose
        // those exact coordinates, and `fighting()` pins the character there once it arrives. Move mode
        // only needs to be inside the bãi, because roaming from it is the point.
        int reach = atkMode == 1 ? ARRIVE_DRIFT : STAND_DRIFT;
        if (drift() <= reach) {
            arrived();
            return;
        }
        if (!canMove()) {
            return;             // a path is already running; let it finish
        }
        try {
            short[] path = fu.c.a(atkX / 24, atkY / 24, cn.g.aZ / 24, cn.g.ba / 24, 500);
            if (path == null) {
                arrived();      // already in the destination cell
                return;
            }
            if (path.length > 500) {
                return;         // pathfinding failed; do not walk a rubbish route
            }
            cn.g.cO = path;
            cn.g.cJ = 0;
            cn.g.cm = 0;
            cn.g.cn = 0;
            cn.g.bg = cn.g.aZ;
            cn.g.bh = cn.g.ba;
            bq.m = true;
        } catch (Throwable t) {
            bq.m = false;
        }
    }

    /** Clears the movement lock. Leaving it set is how the operator loses manual control. */
    private static void arrived() {
        bq.m = false;
        cn.g.cO = null;
        cn.g.bg = cn.g.aZ;
        cn.g.bh = cn.g.ba;
        cn.g.bc = 0;
        cn.g.bd = 0;
        settleTicks = 15;
        atkState = SETTLE;
    }

    /**
     * Separates "no monsters here" from "monsters I cannot reach".
     *
     * cf.K is the client's own answer to "did the last scan find a target": false means the
     * radius is empty, true with a character that never enters combat means terrain. The two
     * call for different fixes, and KnightMod conflates them into one town-charm reflex.
     * This round reports the diagnosis and acts on neither, because acting means travelling.
     */
    private static void diagnose() {
        if (!combatOwned) {
            stuckKind = 0;
            return;
        }
        if (cf.K) {
            noTargetTicks = 0;
            noFightTicks = cn.g.cG == 2 ? 0 : noFightTicks + 1;
        } else {
            noFightTicks = 0;
            ++noTargetTicks;
        }
        if (noTargetTicks > 200) {
            stuckKind = 1;
        } else if (noFightTicks > 400) {
            stuckKind = 2;
        } else {
            stuckKind = 0;
        }
    }

    /**
     * Presses the first hotkey skill the client says can fire.
     *
     * The usability test is the client's own bq.j(skillId, -1): learned, off cooldown, MP
     * paid, not already casting. Reimplementing it would mean re-deriving the per-level MP
     * table, so PatchZeus widens that one method instead. The -1 matters: a real slot index
     * makes bq.j clear the hotkey as a side effect.
     */
    private static void skills() {
        try {
            if (cn.i == null || cn.i.cv != 1 || cn.i.bs <= 0 || cn.i.cG == 4) {
                return;             // no live monster held: never swing at a corpse
            }
            // bq.j prints a message when ef == 0, which would spam the client's log every
            // tick, so that state is filtered before asking.
            if (cn.g.ef == 0) {
                return;
            }
            ao[] page = bq.w == null ? null : bq.w[bq.d];
            if (page == null) {
                return;
            }
            for (int slot = 0; slot < page.length; slot++) {
                ao entry = page[slot];
                if (entry == null || entry.b != 0) {
                    continue;       // b != 0 means the slot holds an item, not a skill
                }
                if (!cn.g.j(entry.a, -1)) {
                    continue;
                }
                cn.g.a(slot, false);
                return;             // one press per tick, like the client's own loop
            }
        } catch (Throwable t) {
            // A missing hotkey page must not stall the tick.
        }
    }
    /**
     * Drinks from the bag, not from a hotkey.
     *
     * The client's own pump reads two fixed hotkey slots and cannot refill them once the id
     * they hold runs out, which is why it silently stops forever (docs/core/08 §5.4). Scanning
     * the bag by function code instead keeps working. bq.s[L] is the shared item cooldown:
     * this module arms it after each send, exactly as the client's own three drink paths do
     * (bq.java:355-357, bq.java:1052-1055, fo.java:499-502). Each function code cools on
     * its own slot, so HP and MP never block each other.
     */
    private static void potions() {
        if ((!atkHpOn && !atkMpOn) || cn.g == null || cn.g.cG == 4 || bw.V == null) {
            return;
        }
        try {
            if (atkHpOn && cn.g.bt > 0 && cn.g.bs * 100 / cn.g.bt < atkHpPct && drink(0)) {
                return;
            }
            if (atkMpOn && cn.g.bv > 0 && cn.g.bu * 100 / cn.g.bv < atkMpPct) {
                drink(1);
            }
        } catch (Throwable t) {
            // An inventory being rebuilt mid-scan must not stall the tick.
        }
    }
    /** Sends the first bag potion of one function code, if its cooldown has expired. */
    private static boolean drink(int function) {
        if (bq.s == null || function >= bq.s.length || bq.s[function] == null
                || bq.s[function].b > 0) {
            return false;
        }
        for (int index = 0; index < bw.V.c(); index++) {
            Object entry = bw.V.a(index);
            if (!(entry instanceof bw)) {
                continue;
            }
            bw item = (bw) entry;
            if (item.u != 4 || item.L != function || item.K <= 0) {
                continue;
            }
            q.a().e((short) item.O);
            // Same arm as the client's own paths: 2000 ms real time, clocked by dx.a().
            bq.s[function].b = 2000;
            bq.s[function].c = 2000;
            bq.s[function].a = dx.a();
            ++potionCount;
            return true;
        }
        return false;
    }

    // ---- ITEM -----------------------------------------------------------------
    //
    // The client already collects drops itself, gated on the `bq.q` record and filtered by
    // `fa.ct` at `bq.java:530-558`. So this module does not pick anything up: it writes that
    // record (see `applyPickup`) and lets the collector work. An earlier revision had its own
    // loop, which meant two mechanisms racing — set "không nhặt" in the game's own menu and
    // Zeus kept collecting, with no way to tell which one had taken an item.
    //
    // What is left here is what the client has no automation for: riding, and pressing OK on
    // the server-worded material dialog.
    //
    // `fa.dH[]` is the material-box table, not a mount list. Mounts are the five template ids
    // {62..66} the client's own menu matches (`fr.java:591-614`).

    private static void items() {
        try {
            if (!gameReady()) {
                return;
            }
            medalDialog();
            mount();
        } catch (Throwable t) {
            // A mod failure must never stall the client tick.
        }
    }

    /** Ticks between rides once one has been sent. 900 at the loop's 25 ticks/s is 36 s. */
    private static final int MOUNT_EVERY = 900;
    /**
     * Seconds before looking again when the bag held no mount.
     *
     * Nothing was sent, so the cost of looking is a scan of the bag — but it is charged every time,
     * and a scan every tick with an empty bag is exactly the spam this avoids. Five seconds is the
     * operator's own number.
     */
    private static final int MOUNT_RETRY = 5 * TICKS_PER_SECOND;
    /** The five mount template ids, as the client's own menu matches them. */
    private static final int MOUNT_ID_MIN = 62, MOUNT_ID_MAX = 66;
    /** `mount.id` value that means "whichever mount is in the bag". */
    private static final int MOUNT_ANY = 0;

    private static int mountWait = 0;

    /**
     * Rides a mount from the bag: the chosen template id, or any of them.
     *
     * By id, not by name: the client's own menu matches `u == 4 && O in {62..66}`
     * (`fr.java:591-614`, `bg.java:80-88`), while the mod it shipped matched seven display strings —
     * two of which correspond to no id at all, and all of which break when the server rewords one.
     * The names still reach the operator, but as data published from the bag rather than as a table
     * this jar invents: see `mountList()`.
     *
     * `mount.id = 0` rides whatever is there. Any other value rides only that one, and says nothing
     * when it is absent — that is the operator asking for one specific mount.
     *
     * The long period is charged only for a ride that was actually sent. Charging it for a failed
     * scan is the bug the compared mod has: pick a mount up a second after the scan and the
     * character walks for another 36 seconds.
     */
    private static void mount() {
        if (mountWait > 0) {
            --mountWait;
            return;
        }
        if (!mountOn || cn.g == null || bw.V == null) {
            return;
        }
        if (cn.g.ef != -1) {
            return;                 // already riding
        }
        bw pick = null;
        for (int index = 0; index < bw.V.c(); index++) {
            Object entry = bw.V.a(index);
            if (!(entry instanceof bw)) {
                continue;
            }
            bw item = (bw) entry;
            if (item.u != 4 || item.O < MOUNT_ID_MIN || item.O > MOUNT_ID_MAX) {
                continue;
            }
            if (mountId == MOUNT_ANY) {
                pick = item;
                break;              // any will do: the first one found
            }
            if (item.O == mountId) {
                pick = item;
                break;
            }
        }
        if (pick == null) {
            mountWait = MOUNT_RETRY;
            return;
        }
        mountWait = MOUNT_EVERY;
        // Opcode 32, the same sender the client's own mount menu uses.
        q.a().e((short) pick.O);
        trace("MOUNT sent id=" + pick.O + " want=" + mountId);
    }

    /**
     * The mounts in the bag, as `id:name` pairs, so the tool can offer them by name.
     *
     * The names come from the server, per item (`bw.g`, assigned in `j`'s constructors), and there
     * is no id-to-name table anywhere in the client to read instead. So the honest list is the one
     * the bag actually holds: an operator carrying nothing sees nothing to choose, which is true,
     * rather than five invented labels.
     *
     * `:` separates the pair and `|` the entries, so both are stripped from a name — a server that
     * ships one in a display string would otherwise split the field.
     */
    private static String mountList() {
        if (bw.V == null) {
            return "";
        }
        StringBuffer out = new StringBuffer(64);
        for (int index = 0; index < bw.V.c(); index++) {
            Object entry = bw.V.a(index);
            if (!(entry instanceof bw)) {
                continue;
            }
            bw item = (bw) entry;
            if (item.u != 4 || item.O < MOUNT_ID_MIN || item.O > MOUNT_ID_MAX) {
                continue;
            }
            if (out.length() > 0) {
                out.append('|');
            }
            out.append(item.O).append(':').append(field(item.g));
        }
        return out.toString();
    }

    /** `clean()` plus the two characters this field's own syntax reserves. */
    private static String field(String value) {
        String text = clean(value);
        StringBuffer safe = new StringBuffer(text.length());
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c == '|' || c == ':') {
                continue;
            }
            safe.append(c);
        }
        return safe.toString();
    }
    /** Sends the client's own dismount command. Needed on maps that forbid riding. */
    public static void dismount() {
        try {
            if (cn.g != null && cn.g.ef != -1) {
                q.a().h((byte) -1);
            }
        } catch (Throwable t) {
            // nothing useful to do if the socket is gone
        }
    }

    private static int medalWait = 0;

    /**
     * Presses OK on the material dialog instead of swallowing it.
     *
     * The wording comes from the server — there is no matching constant anywhere in the
     * client — so this is matched on accent-stripped text and is inherently fragile: one
     * reworded sentence and it goes quiet with no error. It is also why the dialog is
     * pressed rather than swallowed: swallowing would deny the server its acknowledgement.
     *
     * The text is read from da.q, the wrapped body lines, and NOT from toString(): neither
     * da nor its parent cg overrides toString(), so it returns "ah@1a2b3c" and no wording
     * could ever match. The title lives in ah.r, which is private; q is package-private and
     * this class is in the same (default) package, so it needs no bytecode patch. Every ah
     * constructor fills q from the body text (ah.java:384,412,434,465,499,531,676,712,811),
     * so a dialog with a body always has it; the one path that can leave it null (ah.java:770)
     * is guarded below.
     *
     * Both the field and the wording are what KnightMod's own reconnect module reads
     * (modsrc3/MOD06.java:94, matching "nguyen lieu me day" or "me day" + "da duoc dong"),
     * which is what confirmed that the material close-drop notice is a server dialog and not
     * a client setting.
     */
    private static void medalDialog() {
        if (medalWait > 0) {
            --medalWait;
            return;
        }
        if (!itemMedal || fu.s == null) {
            return;
        }
        // The material walk is waiting for exactly this kind of dialog and needs to read it first.
        if (dropPhase == 2) {
            return;
        }
        try {
            String text = norm(dialogText(fu.s));
            // "Chức năng rớt ..." is the shared prefix of all six materials in both directions;
            // matching the ĐÓNG wording alone left the MỞ confirmation on screen, blocking.
            // Deliberately not a bare "nguyen lieu" match: df.u and df.gd are the crafting NPC's
            // prompts, which the operator opens on purpose.
            if (text.indexOf("chuc nang rot") < 0 && text.indexOf("nguyen lieu me day") < 0) {
                return;
            }
            medalWait = 15;         // KnightMod's delay before pressing
            pressOk(fu.s);
        } catch (Throwable t) {
            // A dialog whose text cannot be read is left alone.
        }
    }

    /**
     * Invokes the dialog's own OK button, the same call a tap on it makes.
     *
     * `ah.C` is the dialog's button list and the patcher widens it to public for exactly this.
     * The earlier revision called `fu.s.b(0, 0)` instead — a pointer press at the top-left
     * corner, which lands on the button only by luck. Falls back to the first button when no
     * caption matches, because a dialog with buttons always has one that dismisses it.
     */
    private static void pressOk(da dialog) {
        if (!(dialog instanceof ah)) {
            return;
        }
        et buttons = ((ah) dialog).C;
        if (buttons == null || buttons.c() <= 0) {
            return;
        }
        for (int i = 0; i < buttons.c(); i++) {
            Object entry = buttons.a(i);
            if (!(entry instanceof bt)) {
                continue;
            }
            bt button = (bt) entry;
            String caption = norm(button.a);
            if (caption.indexOf("ok") >= 0 || caption.indexOf("dong") >= 0) {
                button.a();
                return;
            }
        }
        Object first = buttons.a(0);
        if (first instanceof bt) {
            ((bt) first).a();
        }
    }

    /** Joins one dialog's wrapped body lines, or "" when it has none. */
    private static String dialogText(da dialog) {
        String[] lines = dialog.q;
        if (lines == null) {
            return "";
        }
        StringBuffer out = new StringBuffer(64);
        for (int i = 0; i < lines.length; i++) {
            if (lines[i] != null) {
                out.append(lines[i]).append(' ');
            }
        }
        return out.toString();
    }

    /** Lowercases and strips Vietnamese accents, so server wording matches either way. */
    private static String norm(String value) {
        if (value == null) {
            return "";
        }
        String source = "àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡ"
                + "ùúụủũưừứựửữỳýỵỷỹđ";
        String target = "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooo"
                + "uuuuuuuuuuuyyyyyd";
        String lower = value.toLowerCase();
        StringBuffer out = new StringBuffer(lower.length());
        for (int i = 0; i < lower.length(); i++) {
            char c = lower.charAt(i);
            int at = source.indexOf(c);
            out.append(at < 0 ? c : target.charAt(at));
        }
        return out.toString();
    }

    public static boolean action(int cmd, int arg) {
        return false;
    }

    /** Dialog hook (reserved; unused in round 1). */
    public static boolean dialog(String text) {
        return false;
    }

    private static int intProp(String key, int fallback) {
        try {
            String v = System.getProperty(key);
            if (v != null && v.length() > 0) {
                return Integer.parseInt(v.trim());
            }
        } catch (Throwable t) {
            // absent or unparseable: keep the default
        }
        return fallback;
    }
}
