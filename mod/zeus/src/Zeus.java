import Main.GameCanvas;
import GameScreen.GameScreen;
import GameScreen.SelectCharScreen;
import GameScreen.LoginScreen;
import GameScreen.LoadMapScreen;
import GameScreen.MainScreen;
import GameScreen.PaintInfoGameScreen;
import GameScreen.TabScreenNew;
import GameObjects.Player;
import GameObjects.MainObject;
import GameObjects.MainMonster;
import GameObjects.Item;
import GameObjects.MainItem;
import GameObjects.AutoGetItem;
import GameObjects.MainClan;
import GameObjects.MainRMS;
import GameObjects.DelaySkill;
import InterfaceComponents.MsgDialog;
import InterfaceComponents.MainDialog;
import InterfaceComponents.InputDialog;
import InterfaceComponents.ChatTextField;
import InterfaceComponents.iCommand;
import InterfaceComponents.TabRebuildItem;
import InterfaceComponents.DataRebuildItem;
import CLib.TField;
import Model.Menu2;
import Model.Point;
import Model.T;
import Model.mCamera;
import Model.AvMain;
import Thread_More.LoadMap;
import Skill.HotKey;
import CLib.mGraphics;
import CLib.mVector;
import CLib.mSystem;
import net.Message;
import netcommand.global.GlobalService;
import netcommand.global.GlobalLogicHandler;
/*
 * Zeus_Knight — local-only assistant for KnightOnline_402 (J2ME MIDlet).
 *
 * Modules: AUTH (enter the game), PLAYER (publish a read-only snapshot),
 * ATTACK (hold a spot and fight there) and ITEM (pick up drops, ride, dismiss a dialog).
 *
 * PLAYER sends no packet and writes no native field. It reads GameScreen.player every tick and
 * writes a small key=value file the tool polls; see docs/core/11-player-transport.md
 * for why a file and not a socket. Field semantics are verified in
 * docs/core/10-module-player.md — the two that are easy to get wrong:
 *
 *   - GameScreen.player.phantramLv is XP as PERMILLE of the current level (0..1000), not absolute XP.
 *     Proof: the client renders bA/10 + "," + bA%10 + "%" (cf.java:795) and sizes
 *     the bar as bA/10*77/100 (cf.java:798).
 *   - GameScreen.player.coin is gold (readLong) and GameScreen.player.gold is gem (readInt), and BOTH arrive
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
 * Player.autoItem() pick targets and bq's own loop swing; ITEM sends the same opcode 20 the
 * client sends when the operator taps a drop. Every action travels through code the
 * client already ships.
 *
 * AUTH, verified against orig_decomp/:
 *
 *   - bs.c() (login-screen constructor) reads RMS user_pass and, when present,
 *     calls bs.i() + fu.o() + bs.a(i,j) — i.e. the CLIENT logs itself in.
 *     (bs.java:127-139)
 *   - After login the client shows the character-select screen (GameCanvas.selectChar, class x).
 *   - SelectCharScreen.VecSelectChar() ticks that screen; when MsgDialog.isAutologin == true it selects the slot SelectCharScreen.selectChar and
 *     enters the game. (x.java:132-146)
 *   - The vanilla reconnect loop (GlobalLogicHandler.isDisConect + MsgDialog.a, 30 s) is untouched — Auth does
 *     not swallow the disconnect dialog in round 1, so the native loop keeps
 *     running on its own. (bv.java, MsgDialog.java:1289-1302)
 *
 * So the ONLY thing AUTH does: when the character-select screen appears, set
 * the slot cursor and raise MsgDialog.isAutologin. Everything else is the client's own code.
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

    /** Called at the end of GameCanvas.login() every tick. */
    public static void tick() {
        healthSidecarTick();
        reconnectStatusSidecarTick();
        if (GameCanvas.currentScreen == null) {
            sessionReset();
            return;
        }
        sessionTick();
        reconnectSupervisorTick();
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
        traceCheck(mSystem.currentTimeMillis());
        traceTick();
        if (probeArmed) {
            probeArmed = false;
            probeStone();
        }
        if (spotArmed) {
            spotArmed = false;
            probeSpots();
        }
        spotSidecarTick(mSystem.currentTimeMillis());
        traceFlush();
    }

    // ---- AUTH ----------------------------------------------------------------

    private static void auth() {
        try {
            if (GameCanvas.currentScreen == GameCanvas.selectChar) {
                // Character-select screen. Strict positional validation — CHAR-SLOT-02.
                int targetSlot = slot();
                if (targetSlot < 0 || targetSlot > 2) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot invalid internal=" + targetSlot + " reason=MALFORMED_OR_OUT_OF_RANGE");
                    }
                    return;
                }

                if (SelectCharScreen.VecSelectChar == null) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot refused internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=0 reason=LIST_NULL");
                    }
                    return;
                }

                int count = SelectCharScreen.VecSelectChar.size();
                if (targetSlot >= count) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot refused internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=" + count + " reason=INDEX_OUT_OF_BOUNDS");
                    }
                    return;
                }

                if (SelectCharScreen.VecSelectChar.elementAt(targetSlot) == null) {
                    if (!authRefusalTraced) {
                        authRefusalTraced = true;
                        trace("AUTH character slot refused internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=" + count + " reason=NULL_CHARACTER_OBJECT");
                    }
                    return;
                }

                // GameCanvas.currentScreen == GameCanvas.selectChar && SelectCharScreen.VecSelectChar != null && targetSlot >= 0 && targetSlot < SelectCharScreen.VecSelectChar.size() && SelectCharScreen.VecSelectChar.elementAt(targetSlot) != null
                if (authAttempts == 0) {
                    // SelectCharScreen.selectChar is private in vanilla; PatchZeus widens it to public.
                    // Access through the live screen instance GameCanvas.selectChar.
                    GameCanvas.selectChar.selectChar = targetSlot;
                    // MsgDialog.isAutologin makes SelectCharScreen.VecSelectChar() select the slot and enter the game.
                    MsgDialog.isAutologin = true;
                    armed = true;
                    authAttempts = 1;
                    authWaitTicks = 0;
                    trace("AUTH select character slot internal=" + targetSlot + " visual=" + (targetSlot + 1) + " count=" + count + " attempt=1");
                } else if (authAttempts < AUTH_MAX_ATTEMPTS) {
                    if (++authWaitTicks >= AUTH_RETRY_INTERVAL_TICKS) {
                        authWaitTicks = 0;
                        ++authAttempts;
                        GameCanvas.selectChar.selectChar = targetSlot;
                        MsgDialog.isAutologin = true;
                        trace("AUTH retry character select slot=" + GameCanvas.selectChar.selectChar + " attempt=" + authAttempts);
                    }
                } else {
                    if (!authExhaustedTraced) {
                        authExhaustedTraced = true;
                        trace("AUTH character select retry exhausted attempts=" + authAttempts);
                    }
                }
            } else if (GameCanvas.currentScreen != GameCanvas.selectChar) {
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
     * `i.GiaoTiep()` for any `cv != 0` target without the hostility test it applies to players, and
     * `ez.GiaoTiep()` then sends opcode 23 with the board's `cu` (`ez.java:239-243`). So only a live send
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
    private static final String INVENTORY_FILE = "zeus-inventory.json";
    private static final String ENH_REQ_FILE = "zeus-enhance.req";
    private static final String ENH_STATUS_FILE = "zeus-enhance-status.json";
    private static final String ENH_CANCEL_FILE = "zeus-enhance.cancel";
    private static final String HEALTH_FILE = "zeus-health.txt";
    private static final String RECONNECT_STATUS_FILE = "zeus-reconnect.txt";

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
    private static String inventoryPath;
    private static String enhReqPath;
    private static String enhStatusPath;
    private static String enhCancelPath;
    private static String healthPath;
    private static long lastHealthPublishedAt = 0L;
    private static long healthSeq = 0L;
    private static String reconnectStatusPath;
    private static long lastReconnectStatusPublishedAt = 0L;
    private static long reconnectStatusSeq = 0L;
    private static long lastInventoryHash = Long.MIN_VALUE;
    private static boolean inventoryWritten = false;

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
     * `MainObject.ct < Player.autoItem.a`, so 1 means "blue and better". Index 5 in the client's own option list is
     * "don't pick equipment", which it stores as −1 (`MsgDialog.java:208-212`). `itemMpHp` and `itemGold`
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

    /** Buff slots the operator can address. The client's own count is `MsgDialog.MaxSkillBuff`. */
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
        cleanEnhancementRouting();
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
        // ---- VISUAL QOL (v14) -------------------------------------------------
        desiredEffects = 1;
        desiredHidePlayers = 0;
        reconcileVisualQoL();
        // ---- end VISUAL QOL ---------------------------------------------------
    }

    private static void control() {
        if (ctlPath == null) {
            return;
        }
        try {
            long now = mSystem.currentTimeMillis();
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
            // `MainRMS.setSaveAuto()` is a packet, so it is sent when a setting actually changed rather than every
            // half second. The signature covers exactly the values that live in native fields.
            int signature = nativeSignature();
            if (signature != syncedSignature && inGame() && GameScreen.player != null) {
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

    /** Signature last pushed with `MainRMS.setSaveAuto()`. Deliberately impossible to match on the first read. */
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
        "dungeon.on", "dungeon.max", "dungeon.startMin", "dungeon.endMin",
        // ---- end DUNGEON ------------------------------------------------------
        // ---- VISUAL QOL (v15) -------------------------------------------------
        "ui.effects", "ui.hidePlayers",
        // ---- end VISUAL QOL ---------------------------------------------------
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
    // a fresh profile because a ring nobody asked for over a game they are watching is an imposition.
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
    // `dungeon.on` is the switch, `dungeon.max` is how many runs to make before stopping,
    // `dungeon.startMin` is the start minute of the daily UTC+7 window (inclusive, 0..1439),
    // `dungeon.endMin` is the end minute of the daily UTC+7 window (exclusive, 0..1439).
    // -1 for both startMin and endMin means unscheduled / immediate behavior.
    private static final int K_DUNGEON_ON = 32;
    private static final int K_DUNGEON_MAX = 33;
    private static final int K_DUNGEON_START_MIN = 34;
    private static final int K_DUNGEON_END_MIN = 35;
    // ---- end DUNGEON ----------------------------------------------------------
    // ---- VISUAL QOL (v15) -----------------------------------------------------
    private static final int K_UI_EFFECTS = 36;
    private static final int K_UI_HIDE_PLAYERS = 37;
    // ---- end VISUAL QOL -------------------------------------------------------
    /** Format version this jar accepts. Bumped when the key set changed shape. */
    private static final int CTL_VERSION = 15;

    /** Desired state for ui.effects: 1 (enabled, default), 0 (disabled). */
    private static int desiredEffects = 1;
    /** Desired state for ui.hidePlayers: 0 (show all, default), 1 (hide others), 2 (hide all). */
    private static int desiredHidePlayers = 0;

    /**
     * Reconciles desired visual QoL settings directly into native client static fields.
     * Change-only application prevents redundant writes.
     * Guaranteed never to produce GameScreen.isHideOderPlayer=true && GameScreen.isHideFullOderPlayer=true simultaneously.
     */
    private static void reconcileVisualQoL() {
        try {
            // ui.effects: 1 -> MainObject.hideEff = 0, 0 -> MainObject.hideEff = 1
            byte targetCh = (byte) (desiredEffects == 1 ? 0 : 1);
            if (MainObject.hideEff != targetCh) {
                MainObject.hideEff = targetCh;
            }

            // ui.hidePlayers:
            // 0 -> GameScreen.isHideOderPlayer=false, GameScreen.isHideFullOderPlayer=false
            // 1 -> GameScreen.isHideOderPlayer=true,  GameScreen.isHideFullOderPlayer=false
            // 2 -> GameScreen.isHideOderPlayer=false, GameScreen.isHideFullOderPlayer=true
            boolean targetN = (desiredHidePlayers == 1);
            boolean targetO = (desiredHidePlayers == 2);
            if (GameScreen.isHideOderPlayer != targetN || GameScreen.isHideFullOderPlayer != targetO) {
                // Clear the opposite flag first so both are never true simultaneously
                if (!targetN) {
                    GameScreen.isHideOderPlayer = false;
                }
                if (!targetO) {
                    GameScreen.isHideFullOderPlayer = false;
                }
                GameScreen.isHideOderPlayer = targetN;
                GameScreen.isHideFullOderPlayer = targetO;
            }
        } catch (Throwable t) {
            // Visual QoL reconciliation must never throw or interrupt tick execution
        }
    }

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
        if (value[K_DUNGEON_MAX] != -1 && (value[K_DUNGEON_MAX] < 1 || value[K_DUNGEON_MAX] > 10)) {
            return false;
        }
        int startMin = value[K_DUNGEON_START_MIN];
        int endMin = value[K_DUNGEON_END_MIN];
        boolean unscheduled = (startMin == -1 && endMin == -1);
        boolean validWindow = (startMin >= 0 && startMin <= 1439
                && endMin >= 0 && endMin <= 1439
                && startMin < endMin);
        if (!unscheduled && !validWindow) {
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
        if (value[K_UI_EFFECTS] < 0 || value[K_UI_EFFECTS] > 1) {
            return false;
        }
        if (value[K_UI_HIDE_PLAYERS] < 0 || value[K_UI_HIDE_PLAYERS] > 2) {
            return false;
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
        dungeonStartMin = value[K_DUNGEON_START_MIN];
        dungeonEndMin = value[K_DUNGEON_END_MIN];
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
        desiredEffects = value[K_UI_EFFECTS];
        desiredHidePlayers = value[K_UI_HIDE_PLAYERS];
        reconcileVisualQoL();
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
            inventoryPath = home + INVENTORY_FILE;
            enhReqPath = home + ENH_REQ_FILE;
            enhStatusPath = home + ENH_STATUS_FILE;
            enhCancelPath = home + ENH_CANCEL_FILE;
            healthPath = home + HEALTH_FILE;
            reconnectStatusPath = home + RECONNECT_STATUS_FILE;
        }
        String explicitHealth = System.getProperty("zeus.health.out");
        if (explicitHealth != null && explicitHealth.trim().length() > 0) {
            healthPath = explicitHealth.trim();
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
    // The switch is SENT, not tapped. Every board button is `new iCommand(caption, 13, index, cn.b())`
    // (er.java:4102-4104), and `iCommand.a()` reaches `cn.a(13, index)`, whose entire body is
    // `GlobalService.gI().Change_Area((byte) index)` — opcode 51, one byte. So a switch is one packet and the menu is not
    // part of it.
    //
    // What the menu is still needed for: the captions. `LoadMap.MaxArea` is the entry count and `cs.o[]` a
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
                    || GameScreen.player == null || GameCanvas.loadmap == null) {
                return;
            }
            if (zonePhase != 2 && !noDialog()) {
                return;
            }
            int here = GameCanvas.loadmap.idMap;
            // A zone belongs to one map, so landing on a different one re-arms the switch.
            if (here != zoneMapDone && zonePhase == 0) {
                zonePhase = 1;
                zoneTries = 0;
            }
            // Naming a zone is checkable without opening anything: selecting button `sub` lands on
            // `LoadMap.Area == sub`, and the caption for that button reads "khu sub+1".
            if (atkZoneMode == 2 && LoadMap.Area == atkZonePick - 1) {
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
                MainObject board = zoneBoard();
                if (board == null) {
                    zonePhase = 0;              // no board here; nothing to do on this map
                    zoneMapDone = here;
                    trace("ZONE no board on map " + here);
                    return;
                }
                zoneRosterMap = Integer.MIN_VALUE;
                GlobalService.gI().chat_npc((byte) board.ID);
                zonePhase = 2;
                zoneWait = 100;                 // 5 s for the server to answer
                trace("ZONE opened board cu=" + board.ID + " on map " + here);
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
            if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                GameCanvas.menu2.doCloseMenu();
                GameCanvas.clearKeyHold();
            }
        } catch (Throwable t) {
            // A menu that will not close is not worth failing the switch over.
        }
    }

    /**
     * Reads the board roster out of a menu this module asked for.
     *
     * Called from the `Menu2.a` prologue, where the button list is already assembled. Every fact a
     * switch needs is here and nowhere else: `iCommand.f` is the byte opcode 51 carries, and the caption
     * carries the zone number and the population.
     */
    private static void zoneRoster(mVector items) {
        try {
            int count = items == null ? 0 : items.size();
            byte[] sub = new byte[count];
            int[] num = new int[count];
            int[] players = new int[count];
            boolean[] skip = new boolean[count];
            for (int i = 0; i < count; i++) {
                num[i] = -1;
                players[i] = -1;
                Object entry = items.elementAt(i);
                if (!(entry instanceof iCommand)) {
                    skip[i] = true;
                    continue;
                }
                iCommand button = (iCommand) entry;
                String text = norm(button.caption);
                sub[i] = button.subIndex;
                num[i] = captionZone(text);
                players[i] = captionCount(text);
                // `e != 13` is not a zone button at all. The rest are zones this module will not
                // enter: the trade zone, the two-hour zone, and the ticketed one.
                skip[i] = button.indexMenu != 13
                        || text.indexOf("khu") < 0
                        || text.indexOf("buon") >= 0
                        || text.indexOf("2h") >= 0
                        || num[i] == ZONE_TICKETED;
            }
            zoneRosterSub = sub;
            zoneRosterNum = num;
            zoneRosterCount = players;
            zoneRosterSkip = skip;
            zoneRosterMap = GameCanvas.loadmap == null ? Integer.MIN_VALUE : GameCanvas.loadmap.idMap;
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
     * `GlobalService.gI().Change_Area((byte) sub)` is the whole of `cn.a(13, sub)`, which is where `iCommand.a()` arrives when the
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
        if (zoneRosterSub[at] == LoadMap.Area) {
            trace("ZONE already in khu " + zoneRosterNum[at]);
            return;
        }
        GlobalService.gI().Change_Area(zoneRosterSub[at]);
        trace("ZONE sent op=51 sub=" + zoneRosterSub[at] + " khu=" + zoneRosterNum[at]
                + " players=" + zoneRosterCount[at] + " from khu " + (LoadMap.Area + 1));
    }

    /** The nearest zone board on this map, or null when there is none. */
    private static MainObject zoneBoard() {
        if (GameScreen.Vecplayers == null || GameScreen.player == null) {
            return null;
        }
        MainObject best = null;
        int bestDistance = Integer.MAX_VALUE;
        for (int i = 0; i < GameScreen.Vecplayers.size(); i++) {
            Object entry = GameScreen.Vecplayers.elementAt(i);
            if (!(entry instanceof MainObject)) {
                continue;
            }
            MainObject candidate = (MainObject) entry;
            // The name, not the template id: a map carries several boards and their ids differ.
            if (candidate.typeObject != 2 || candidate.name == null
                    || norm(candidate.name).indexOf("khu") < 0) {
                continue;
            }
            int distance = abs(GameScreen.player.x - candidate.x) + abs(GameScreen.player.y - candidate.y);
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
    // `GameCanvas.currentScreen = this`. So the panel BECOMES the active screen, `inGame()` goes false, and every module
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
            if (!inGame() || !sceneReady() || !alive() || captcha() || GameScreen.player == null) {
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
                    GlobalService.gI().Dynamic_Menu((short) 125, (byte) 125, (byte) 0);
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
                    GlobalService.gI().Dynamic_Menu((short) npc, (byte) menu, (byte) dropSlot);
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
    private static boolean dropMenu(mVector items, int idMenu, int idNPC) {
        try {
            int count = items == null ? 0 : items.size();
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
                Object entry = items.elementAt(i);
                String label = entry instanceof iCommand ? norm(((iCommand) entry).caption) : "";
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
        if (GameCanvas.currentDialog == null) {
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
        String text = norm(dialogText(GameCanvas.currentDialog));
        if (text.indexOf("chuc nang rot") < 0) {
            return;                                 // some other dialog; wait for ours
        }
        boolean closed = text.indexOf("MainDialog duoc dong") >= 0;
        boolean opened = text.indexOf("MainDialog duoc mo") >= 0;
        int named = -1;
        for (int i = 0; i < MATERIAL_SLOTS; i++) {
            if (text.indexOf(DROP_NAMES[i]) >= 0) {
                named = i;
                break;
            }
        }
        pressOk(GameCanvas.currentDialog);
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
     * sends opcode 23 with the stone's own `cu` — byte for byte what `ez.GiaoTiep()` sends when the operator
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
            if (!inGame() || GameScreen.player == null || GameScreen.Vecplayers == null) {
                trace("PROBE skipped: not in the world");
                return;
            }
            MainObject[] stones = new MainObject[8];
            int found = 0;
            int listed = 0;
            for (int i = 0; i < GameScreen.Vecplayers.size(); i++) {
                Object entry = GameScreen.Vecplayers.elementAt(i);
                if (!(entry instanceof MainObject)) {
                    continue;
                }
                MainObject candidate = (MainObject) entry;
                if (candidate.typeObject != 2) {
                    continue;
                }
                int distance = abs(GameScreen.player.x - candidate.x) + abs(GameScreen.player.y - candidate.y);
                // Listed in full, not capped at a dozen: the previous run truncated before reaching
                // the entities further down the map, which is how a second stone would be missed.
                if (listed < 40) {
                    ++listed;
                    trace("PROBE near cv=2 cu=" + candidate.ID + " x=" + candidate.x + " y="
                            + candidate.y + " dist=" + distance + " name=" + clean(candidate.name));
                }
                // Matched by name, not by cu: the four zone boards on one map already proved cu is
                // per-instance, and the stone's cu is what tells its region apart (10/33/55).
                if (found < stones.length && norm(candidate.name).indexOf("dich chuyen") >= 0) {
                    stones[found++] = candidate;
                }
            }
            trace("PROBE cv=2 total listed=" + listed + " stones=" + found
                    + " map=" + (GameCanvas.loadmap == null ? -1 : GameCanvas.loadmap.idMap) + " me=" + GameScreen.player.x + "," + GameScreen.player.y);
            if (found == 0) {
                trace("PROBE no teleport stone on this map");
                return;
            }
            MainObject stone = stones[probeRound % found];
            ++probeRound;
            int distance = abs(GameScreen.player.x - stone.x) + abs(GameScreen.player.y - stone.y);
            trace("PROBE stone cu=" + stone.ID + " x=" + stone.x + " y=" + stone.y
                    + " dist=" + distance);
            GlobalService.gI().chat_npc((byte) stone.ID);
            trace("PROBE sent op=23 cu=" + stone.ID);
        } catch (Throwable t) {
            trace("PROBE failed");
        }
    }

    /**
     * Dumps every monster in the scene with its SPAWN ANCHOR, and clusters those anchors.
     *
     * The anchor is the fact worth having. `cc`'s constructor sets `F`/`G` from the spawn coordinates
     * the catalogue packet carried and never moves them, while `aZ`/`ba` drift as the monster wanders;
     * `MainMonster.java:253` pulls it back to `(F, G)` once it passes 1.5 × `C` (= 60). So a spot's centre is
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
        if (!inGame() || GameScreen.player == null || GameScreen.Vecplayers == null || GameCanvas.loadmap == null) {
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
        for (int i = 0; i < GameScreen.Vecplayers.size() && found < cap; i++) {
            Object entry = GameScreen.Vecplayers.elementAt(i);
            if (!(entry instanceof MainMonster)) {
                continue;
            }
            MainMonster mob = (MainMonster) entry;
            if (mob.typeObject != 1) {
                continue;
            }
            ax[found] = mob.xAnchor;
            ay[found] = mob.yAnchor;
            group[found] = -1;
            mobName[found] = mob.name;
            mobLevel[found] = mob.Lv;
            if (doTrace) {
                trace("SPOT mob cu=" + mob.ID + " lv=" + mob.Lv + " hp=" + mob.maxHp
                        + " anchor=" + mob.xAnchor + "," + mob.yAnchor
                        + " at=" + mob.x + "," + mob.y
                        + " drift=" + (abs(mob.x - mob.xAnchor) + abs(mob.y - mob.yAnchor))
                        + " name=" + clean(mob.name));
            }
            ++found;
        }
        if (doTrace) {
            trace("SPOT map=" + GameCanvas.loadmap.idMap + " khu=" + (LoadMap.Area + 1) + " me=" + GameScreen.player.x + "," + GameScreen.player.y
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
                        + " fromMe=" + (abs(GameScreen.player.x - cx) + abs(GameScreen.player.y - cy))
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

        if (!inGame() || GameScreen.player == null || GameScreen.Vecplayers == null || GameCanvas.loadmap == null) {
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

        int mapId = GameCanvas.loadmap.idMap & 0xFF;
        int capturedZone = LoadMap.Area >= 0 ? (LoadMap.Area + 1) : 0;
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
     * Called from every RETURN of `cn.a(mGraphics)`, the world paint, not from its first line. Two reasons, and
     * the first cost two rounds of "nothing appears": a prologue runs before `bx2.a(-MainScreen.cameraMain.xCam, -MainScreen.cameraMain.yCam)`,
     * so the context is still in SCREEN space and world coordinates land hundreds of pixels off a
     * 240x320 display; and it also runs before `GameCanvas.loadmap.a(bx2)` paints the map, which would then cover
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
    public static void paint(mGraphics canvas) {
        if (canvas == null) {
            return;
        }
        try {
            if (isEnhancementHighlightActive()) {
                paintEnhancementHighlight(canvas);
            }
        } catch (Throwable ignored) {
        }
        if (!ringOn) {
            return;
        }
        try {
            if (GameScreen.player == null || GameCanvas.currentScreen != GameCanvas.game) {
                return;             // not on the world screen; nothing to anchor to
            }
            if (MainScreen.cameraMain == null) {
                return;             // no camera yet; there is no screen position to draw at
            }
            // Where a world coordinate has to be DRAWN so that it LANDS where the client's own
            // entities land. Two separate facts, and conflating them is what put the ring off screen
            // for three rounds:
            //
            //   1. The client draws an entity at its raw world coordinate (`cn.java:896`,
            //      `bx2.a(fe2.a, ..., this.x, this.y, 33)`), because `cn.a(mGraphics)` translates by
            //      `-MainScreen.cameraMain` first (`cn.java:548`). So "on screen at the character" means the pixel
            //      `world - camera`.
            //   2. At the RETURN the translation is no longer just `-MainScreen.cameraMain`. `bx2.a(bu, bv)` added the
            //      screen shake and the HUD block added `bx2.a(GameCanvas.w - fu.r.a * ey.c - 3, ...)` and
            //      never undid it.
            //
            // Drawing at `C` puts ink at `C + T`, where `T` is whatever is in force. Wanting
            // `C + T == world - camera` gives `C = world - camera - T`. The old code drew at
            // `world - T`, which is that answer plus the camera: correct only while the camera sat at
            // the origin, which is the top-left corner of the map and nowhere a character ever farms.
            int shiftX = -canvas.getTranslateX() - MainScreen.cameraMain.xCam;
            int shiftY = -canvas.getTranslateY() - MainScreen.cameraMain.yCam;
            int x = GameScreen.player.x + shiftX;
            int y = GameScreen.player.y + shiftY;
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
                trace("RING t=" + canvas.getTranslateX() + ',' + canvas.getTranslateY()
                        + " cam=" + MainScreen.cameraMain.xCam + ',' + MainScreen.cameraMain.yCam
                        + " world=" + GameScreen.player.x + ',' + GameScreen.player.y
                        + " at=" + x + ',' + y
                        + " ink=" + (x + canvas.getTranslateX()) + ',' + (y + canvas.getTranslateY())
                        + " screen=" + GameCanvas.w + 'x' + GameCanvas.h
                        + " r=" + atkRadius);
            }
            canvas.setColor(0x33FF33);
            circle(canvas, x, y, atkRadius);
            // Who the client is aiming at. `GameScreen.ObjFocus` is its own focus, so this marks the same entity the
            // next attack will reach for rather than the tool's guess at one.
            MainObject aim = GameScreen.ObjFocus;
            if (aim != null) {
                int aimX = aim.x + shiftX;
                int aimY = aim.y + shiftY;
                // Measured in world coordinates, drawn in translated ones: the reach test is the
                // client's own and must not be affected by where the HUD left the origin.
                boolean reachable = abs(aim.x - GameScreen.player.x) + abs(aim.y - GameScreen.player.y) <= atkRadius;
                // Colour carries the one fact worth knowing about the aim: whether it is close enough
                // to be hit. Naming it in a trace would mean reading a log while fighting.
                canvas.setColor(reachable ? 0xFFFF33 : 0xFF3333);
                circle(canvas, aimX, aimY, 14);
                canvas.drawLine(aimX - 8, aimY, aimX + 8, aimY, false);
                canvas.drawLine(aimX, aimY - 8, aimX, aimY + 8, false);
                // And the line to it, which is what makes a target legible in a crowd.
                canvas.drawLine(x, y, aimX, aimY, false);
            }
            if (atkMode != 0 && atkX >= 0 && atkY >= 0) {
                // The leash, drawn on the spot rather than the character: this is the distance the
                // walker measures from where it was told to stand, and seeing it on the character
                // would put it in the wrong place entirely.
                int spotX = atkX + shiftX;
                int spotY = atkY + shiftY;
                canvas.setColor(0xFFAA00);
                circle(canvas, spotX, spotY, atkMode == 1 ? STAND_DRIFT : MOVE_DRIFT);
                // And the spot itself, because a leash with no centre is hard to read.
                canvas.setColor(0xFFFFFF);
                canvas.drawLine(spotX - 4, spotY, spotX + 4, spotY, false);
                canvas.drawLine(spotX, spotY - 4, spotX, spotY + 4, false);
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
    private static void circle(mGraphics canvas, int cx, int cy, int reach) {
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
    private static void plot8(mGraphics canvas, int cx, int cy, int dx, int dy) {
        dot(canvas, cx + dx, cy + dy);
        dot(canvas, cx + dy, cy + dx);
        dot(canvas, cx - dy, cy + dx);
        dot(canvas, cx - dx, cy + dy);
        dot(canvas, cx - dx, cy - dy);
        dot(canvas, cx - dy, cy - dx);
        dot(canvas, cx + dy, cy - dx);
        dot(canvas, cx + dx, cy - dy);
    }

    /** One pixel, as the shortest line `mGraphics` can draw: it exposes no point primitive. */
    private static void dot(mGraphics canvas, int x, int y) {
        canvas.drawLine(x, y, x, y, false);
    }
    // ---- POTATO --------------------------------------------------------------
    //
    // Paint, decoupled from the tick. Merged from `mod/potato/src/POTATO.java`, which is
    // where the measurement and the per-layer safety arguments were made; the entry points
    // keep their names so the bytecode half of A3.1 stays a one-word owner change per site
    // (`PatchCanvas` folds com/silverknight/a.run()'s repaint+serviceRepaints pair into one
    // `Zeus.doRepaint(Canvas)`, `PatchLayers` injects `Zeus.skipLayer(bit)` into ey.a(mGraphics)
    // and br.a(mGraphics), and the recompiled mGraphics calls `Zeus.countDraw()` on every primitive that
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
    /** mGraphics draw calls, summed across every primitive. */
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
     *   1 = ey.a(mGraphics)  minimap
     *   2 = br.a(mGraphics)  effects
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
        pollPotatoControl(mSystem.currentTimeMillis());
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

    /** Called by mGraphics on every primitive that reaches Graphics. */
    public static void countDraw() {
        ++paintDraws;
    }

    /**
     * Whether this tick paints.
     *
     * Guard: some vanilla logic lives inside the draw path, so paint cannot be dropped
     * unconditionally — not even at paintEvery 0, which is why the guard is checked before
     * the never-paint case. cf.i(mGraphics) advances the map-19/67 cutscene and ends it through
     * n.b().c(); eq.a(mGraphics) advances character-select animation. Both run only from a draw
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
            if (GameCanvas.loadmap != null && (GameCanvas.loadmap.idMap == 19 || GameCanvas.loadmap.idMap == 67)) {
                return true;    // cutscene advanced from cf.i(mGraphics)
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
        long now = mSystem.currentTimeMillis();
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
            if (GameCanvas.currentScreen == GameCanvas.login) {
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
    public static void sent(Message packet) {
        if (!traceOn || packet == null) {
            return;
        }
        try {
            byte[] payload = packet.getData();
            StringBuffer line = new StringBuffer(64);
            line.append("SEND op=").append(packet.command).append(" len=")
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
     * Called from a prologue injected into `Menu2.a(mVector,int,String,boolean,mVector)`, the menu builder. The
     * button's own command id travels with its caption, which is what lets a module send the same
     * selection the operator's tap would.
     *
     * Returning true makes the builder return before it sets `this.a = true`, so the menu is never
     * drawn. A module that answers the menu itself has no use for it on screen — and dismissing it
     * after the fact was not enough, because the frame in between still showed it.
     */
    public static boolean menu(mVector items, String title) {
        // Captured before the trace gate, because ZONE needs the roster whether or not the operator
        // asked for a log. Only while a board is being waited on: reading every local menu into
        // module state would make a shop or an inventory menu look like a board reply.
        boolean taken = false;
        if (zonePhase == 2) {
            zoneRoster(items);
            // Only a roster this module could actually read is worth hiding: a menu it rejected has
            // to stay visible, or a board that is not what was measured would vanish silently.
            //
            // FORCED OFF. The swallow returns from `Menu2.a` before its own prologue runs, so the menu
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
            trace("MENU title=" + clean(title) + " count=" + (items == null ? 0 : items.size())
                    + (taken ? " (taken)" : ""));
            if (items == null) {
                return taken;
            }
            for (int i = 0; i < items.size(); i++) {
                Object entry = items.elementAt(i);
                if (!(entry instanceof iCommand)) {
                    trace("  [" + i + "] (not a button)");
                    continue;
                }
                iCommand button = (iCommand) entry;
                // `e` is the command id the client dispatches on and `f` the sub-index, both set by
                // the iCommand constructors (iCommand.java:38-54). Together they are what a later round would
                // have to reproduce to make the same selection the operator's tap makes.
                trace("  [" + i + "] cmd=" + button.indexMenu + " sub=" + button.subIndex + " text="
                        + clean(button.caption));
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
     * Separate from {@link #menu} because the two overloads of `Menu2.a` carry different facts. A local
     * menu's buttons hold their own command and sub-index, so logging those is enough to reproduce a
     * selection. A server menu's buttons hold neither: `Menu2.a(2, _)` sends `GlobalService.gI().b(Menu2.C, Menu2.B, Menu2.h)`,
     * so the reproducible part is the idNPC/idMenu pair from the builder call plus the entry's
     * position — and pressing a button selects whatever is highlighted, not what was pressed.
     *
     * Returning true hides it, for the same reason the local overload does: a module that answers the
     * menu itself never needs it drawn.
     */
    public static boolean serverMenu(mVector items, int idMenu, int idNPC, String title) {
        boolean taken = false;
        // Captured before the trace gate, because TRAVEL needs the labels whether or not the operator
        // asked for a log. Only while a stone is being waited on: hooking every server menu into
        // module state would make a shop visit look like a travel reply.
        if (travelState == TV_STONE_WAIT && travelMenuNpc == Integer.MIN_VALUE) {
            try {
                int count = items == null ? 0 : items.size();
                String[] labels = new String[count];
                for (int i = 0; i < count; i++) {
                    Object entry = items.elementAt(i);
                    labels[i] = entry instanceof iCommand ? ((iCommand) entry).caption : null;
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
                // Nothing is written to `GameCanvas.menu2` here on purpose. Phase 0 only starts behind
                // `noDialog()`, so `GameCanvas.menu2.isShowMenu` is already false when the reply lands, and the swallowed
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
                int count = items == null ? 0 : items.size();
                String[] labels = new String[count];
                for (int i = 0; i < count; i++) {
                    Object entry = items.elementAt(i);
                    labels[i] = entry instanceof iCommand ? ((iCommand) entry).caption : null;
                }
                dungeonMenu = labels;
                dungeonMenuItems = items;
                dungeonMenuNpc = idNPC;
                dungeonMenuId = idMenu;
            } catch (Throwable t) {
                dungeonMenu = null;
                dungeonMenuItems = null;
                dungeonMenuNpc = Integer.MIN_VALUE;
            }
            // NOT swallowed, and that is deliberate rather than an oversight. Returning true makes
            // the builder return before `this.a = true`, which leaves the singleton with `a` false
            // and a stale `g`/`aa`; cn's paint gate (cn.java:704) then draws a blank white frame.
            // The menu stays visible and this module closes it itself, the way travelCloseMenu()
            // does, once it has taken what it needed.
        }
        // ---- end DUNGEON ------------------------------------------------------
        // ---- ENHANCE-04 -------------------------------------------------------
        if (enhState == 6 && items != null) {
            try {
                int pickIndex = -1;
                String matchedLabel = null;
                for (int i = 0; i < items.size(); i++) {
                    Object entry = items.elementAt(i);
                    if (entry instanceof iCommand) {
                        String label = norm(((iCommand) entry).caption);
                        if ("cuong hoa".equals(label)) {
                            pickIndex = i;
                            matchedLabel = label;
                            break;
                        }
                    }
                }
                if (pickIndex >= 0) {
                    long fingerprint = (((long) idNPC) << 32) ^ (((long) idMenu) << 16) ^ (long) items.size() ^ (((long) pickIndex) << 8);
                    synchronized (ENH_MENU_LOCK) {
                        if (fingerprint != enhLastDispatchedMenuFingerprint && enhPendingMenuRecord == null) {
                            PendingForgeMenu rec = new PendingForgeMenu(idNPC, idMenu, pickIndex, items.size(), matchedLabel, fingerprint);
                            setPendingForgeMenu(rec);
                            trace("ENHANCE captured forge menu npc=" + idNPC + " menu=" + idMenu + " option=" + pickIndex + " fingerprint=" + fingerprint);
                        }
                    }
                    // NEVER swallow the menu synchronously. Return false to allow native Menu2.setinfoDynamic to complete normally.
                    taken = false;
                }
            } catch (Throwable t) {
            }
        }
        // ---- end ENHANCE-04 ---------------------------------------------------
        if (!traceOn) {
            return taken;
        }
        try {
            trace("SMENU npc=" + idNPC + " menu=" + idMenu + " title=" + clean(title)
                    + " count=" + (items == null ? 0 : items.size()) + (taken ? " (taken)" : ""));
            if (items == null) {
                return taken;
            }
            for (int i = 0; i < items.size(); i++) {
                Object entry = items.elementAt(i);
                if (!(entry instanceof iCommand)) {
                    trace("  <" + i + "> (not a button)");
                    continue;
                }
                trace("  <" + i + "> text=" + clean(((iCommand) entry).caption));
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
            if (GameScreen.ObjFocus != null) {
                int id = GameScreen.ObjFocus.ID * 1000 + GameScreen.ObjFocus.typeObject;
                if (id != traceTargetId) {
                    traceTargetId = id;
                    trace("TARGET cv=" + GameScreen.ObjFocus.typeObject + " cu=" + GameScreen.ObjFocus.ID + " x=" + GameScreen.ObjFocus.x
                            + " y=" + GameScreen.ObjFocus.y + " name=" + clean(GameScreen.ObjFocus.name));
                }
            } else if (traceTargetId != Integer.MIN_VALUE) {
                traceTargetId = Integer.MIN_VALUE;
                trace("TARGET none");
            }
            MainDialog activeDialog = GameCanvas.currentDialog != null ? GameCanvas.currentDialog : (GameCanvas.subDialog != null ? GameCanvas.subDialog : null);
            String dialog = activeDialog == null ? null : dialogText(activeDialog);
            if (dialog != null && !dialog.equals(traceDialog)) {
                traceDialog = dialog;
                trace("DIALOG " + clean(dialog));
            } else if (dialog == null && traceDialog != null) {
                traceDialog = null;
                trace("DIALOG closed");
            }
            if (LoadMap.Area != traceZone) {
                traceZone = LoadMap.Area;
                trace("ZONE now=" + LoadMap.Area + " count=" + LoadMap.MaxArea);
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
            // mSystem.currentTimeMillis() is System.currentTimeMillis(). fu.aj is a tick counter that wraps
            // at 10000, so it cannot measure a five-minute window (docs/core/10 §4.3).
            long now = mSystem.currentTimeMillis();
            if (GameScreen.player == null) {
                return;             // not in a character yet: nothing to report
            }
            // GameScreen.player is non-null well before the character exists: the client installs a
            // level-0 placeholder called "unname" (cn.java:94) and only fills the real
            // stats in when opcode 3 arrives. Sampling that placeholder put a level-0
            // reading at the start of the window, so the first real reading looked like a
            // gain of 80 levels and the rate came out in the millions.
            boolean statsArrived = GameScreen.player.Lv > 0;
            if (statsArrived && now - sampledAt >= sampleEveryMs) {
                sampledAt = now;
                sample(now, GameScreen.player.Lv, GameScreen.player.phantramLv);
            }
            if (now - wroteAt >= writeEveryMs) {
                wroteAt = now;
                publish(now);
                publishInventory(now);
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
    private static int getAutoItemRank() {
        try {
            if (Player.autoItem == null) return -1;
            java.lang.reflect.Field f = AutoGetItem.class.getDeclaredField("valueColorItem");
            f.setAccessible(true);
            return f.getByte(Player.autoItem);
        } catch (Throwable t) { return -1; }
    }
    private static int getAutoItemPotion() {
        try {
            if (Player.autoItem == null) return -1;
            java.lang.reflect.Field f = AutoGetItem.class.getDeclaredField("isGetPotion");
            f.setAccessible(true);
            return f.getByte(Player.autoItem);
        } catch (Throwable t) { return -1; }
    }
    private static int getAutoItemMoney() {
        try {
            if (Player.autoItem == null) return -1;
            java.lang.reflect.Field f = AutoGetItem.class.getDeclaredField("isGetMoney");
            f.setAccessible(true);
            return f.getByte(Player.autoItem);
        } catch (Throwable t) { return -1; }
    }

    private static void publish(long now) {
        StringBuffer out = new StringBuffer(320);
        // 5: travelgoal now reports the destination in force rather than the configured one, so a
        // tool reading v4 semantics would mislabel a nav.target route as an atk.travel one.
        // 6: `mounts` added. The tool cannot name a mount without it — no id-to-name table exists
        // anywhere in the client, so the bag is the only honest source.
        out.append("v=6\n");
        out.append("t=").append(now).append('\n');
        out.append("name=").append(clean(GameScreen.player.name)).append('\n');
        out.append("lv=").append(GameScreen.player.Lv).append('\n');
        out.append("xp=").append(GameScreen.player.phantramLv).append('\n');
        out.append("hp=").append(GameScreen.player.hp).append('\n');
        out.append("hpmax=").append(GameScreen.player.maxHp).append('\n');
        out.append("mp=").append(GameScreen.player.mp).append('\n');
        out.append("mpmax=").append(GameScreen.player.maxMp).append('\n');
        // bD is gold and bC is gem; both only arrive with opcode 16.
        if (GameScreen.player.coin != 0L || GameScreen.player.gold != 0L) {
            walletKnown = true;
        }
        out.append("wallet=").append(walletKnown ? 1 : 0).append('\n');
        out.append("gold=").append(GameScreen.player.coin).append('\n');
        out.append("gem=").append(GameScreen.player.gold).append('\n');
        out.append("map=").append(GameCanvas.loadmap != null ? GameCanvas.loadmap.idMap : -1).append('\n');
        out.append("zone=").append(LoadMap.Area).append('\n');
        out.append("px=").append(GameScreen.player.x).append('\n');
        out.append("py=").append(GameScreen.player.y).append('\n');
        // Player.demUnFire is the attack quota. At <= 0 the client silently drops auto from 1 to 0
        // (bq.java:577), which is the top cause of "auto stopped for no reason".
        out.append("quota=").append(Player.demUnFire).append('\n');
        out.append("bag=").append(Item.VecInvetoryPlayer != null ? Item.VecInvetoryPlayer.size() : -1).append('\n');
        out.append("bagmax=").append(Player.maxInven).append('\n');
        out.append("state=").append(GameScreen.player.Action).append('\n');
        out.append("mount=").append(GameScreen.player.typeMount).append('\n');
        // The mounts actually in the bag, `id:name` pairs joined by `|`, so the tool can offer them
        // by their server-given names instead of by a number.
        out.append("mounts=").append(mountList()).append('\n');
        out.append("guild=").append(GameScreen.player.myClan != null ? clean(GameScreen.player.myClan.name) : "").append('\n');
        out.append("xprate=").append(xpPermillePerHour()).append('\n');
        // What the two active modules are actually doing. Without these there is no way to
        // tell a working module from a silent one without opening the game and watching.
        out.append("atkphase=").append(Player.isAutoFire).append('\n');
        out.append("ctl=").append(ctlState).append('\n');
        out.append("atkstate=").append(combatOwned ? atkState : -1).append('\n');
        out.append("target=").append(GameScreen.ObjFocus != null && GameScreen.ObjFocus.typeObject == 1 && GameScreen.ObjFocus.hp > 0 ? 1 : 0)
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
        // the settings file. MainRMS.setSaveAuto() and its read-back disagree about which byte carries which
        // value, so this is the only honest way to show the operator what took effect.
        out.append("pkrank=").append(getAutoItemRank()).append('\n');
        out.append("pkmphp=").append(getAutoItemPotion()).append('\n');
        out.append("pkgold=").append(getAutoItemMoney()).append('\n');
        // Which buff slots the client will actually cast, after the learned check.
        out.append("buffs=").append(buffState()).append('\n');
        // Six characters, one per material: `-` never confirmed, `0` open, `1` closed. Read back
        // from the server's own confirmations, so a setting that never took effect shows as unknown
        // instead of as applied.
        out.append("drops=").append(dropStates()).append('\n');
        // LoadMapScreen.isNextMap false or a loading map means the values are last-known, not current.
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
        out.append("dungeonfails=").append(dungeonFails).append('\n');
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
            return GameCanvas.currentScreen == GameCanvas.game && LoadMapScreen.isNextMap && GameCanvas.loadmap != null && LoadMap.isShowEffAuto != LoadMap.EFF_PHOBANG_END;
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

    // ---- HEALTH SIDECAR (R1A) ------------------------------------------------
    /**
     * Observational runtime health sidecar (zeus-health.txt, contract v1).
     * Proves game-loop progress across login/reconnect states even when GameCanvas.currentScreen == null or GameScreen.player == null.
     * Publishes ~1000ms cadence against mSystem.currentTimeMillis() to zeus-health.txt via temp file and atomic replace.
     */
    private static void healthSidecarTick() {
        try {
            if (healthPath == null) {
                return;
            }
            long now = mSystem.currentTimeMillis();
            if (now < lastHealthPublishedAt) {
                // Defensive wall-clock rollback handling
                lastHealthPublishedAt = now;
            }
            if (now - lastHealthPublishedAt < 1000L) {
                return;
            }
            lastHealthPublishedAt = now;
            healthSeq++;

            String screen;
            if (GameCanvas.currentScreen == null) {
                screen = "none";
            } else if (GameCanvas.currentScreen == GameCanvas.login && GameCanvas.subDialog == GameCanvas.msgchat) {
                screen = "server";
            } else if (GameCanvas.currentScreen == GameCanvas.login) {
                screen = "login";
            } else if (GameCanvas.currentScreen == GameCanvas.selectChar) {
                screen = "character";
            } else if (GameCanvas.currentScreen == GameCanvas.game) {
                screen = "world";
            } else {
                screen = "other";
            }

            int dialog = GameCanvas.currentDialog != null ? 1 : 0;
            int disconnect = GlobalLogicHandler.isDisConect ? 1 : 0;

            StringBuffer sb = new StringBuffer(128);
            sb.append("v=1\n");
            sb.append("t=").append(now).append('\n');
            sb.append("seq=").append(healthSeq).append('\n');
            sb.append("screen=").append(screen).append('\n');
            sb.append("dialog=").append(dialog).append('\n');
            sb.append("disconnect=").append(disconnect).append('\n');

            writeHealth(sb.toString());
        } catch (Throwable t) {
            // Fail silent: health publishing must never stall the client tick.
        }
    }

    private static void writeHealth(String body) {
        if (healthPath == null) {
            return;
        }
        java.io.OutputStream stream = null;
        try {
            java.io.File target = new java.io.File(healthPath);
            java.io.File temp = new java.io.File(healthPath + ".tmp");
            stream = new java.io.FileOutputStream(temp);
            stream.write(body.getBytes("UTF-8"));
            stream.close();
            stream = null;
            if (target.exists() && !target.delete()) {
                return;
            }
            temp.renameTo(target);
        } catch (Throwable t) {
            // Fail silent
        } finally {
            if (stream != null) {
                try {
                    stream.close();
                } catch (Throwable ignored) {
                }
            }
        }
    }

    /**
     * Observational reconnect status sidecar (zeus-reconnect.txt, contract v1).
     * Publishes ~1000ms cadence against mSystem.currentTimeMillis() to zeus-reconnect.txt via temp file and atomic replace.
     * Strictly observe-only: zero mutations to reconnect state machine or game loop.
     */
    private static void reconnectStatusSidecarTick() {
        try {
            if (reconnectStatusPath == null) {
                return;
            }
            long now = mSystem.currentTimeMillis();
            if (now < lastReconnectStatusPublishedAt) {
                // Defensive wall-clock rollback handling
                lastReconnectStatusPublishedAt = now;
            }
            if (now - lastReconnectStatusPublishedAt < 1000L) {
                return;
            }
            lastReconnectStatusPublishedAt = now;
            reconnectStatusSeq++;

            int active;
            String stateStr;
            int worldBefore;
            if (!reconnectEpisodeActive) {
                active = 0;
                stateStr = "idle";
                worldBefore = 0;
            } else {
                active = 1;
                switch (reconnectState) {
                    case RC_NATIVE_WAIT: stateStr = "native_wait"; break;
                    case RC_LOGIN: stateStr = "login"; break;
                    case RC_SERVER: stateStr = "server"; break;
                    case RC_CHARACTER: stateStr = "character"; break;
                    case RC_LOADING: stateStr = "loading"; break;
                    case RC_WORLD_SETTLE: stateStr = "world_settle"; break;
                    case RC_OTHER: stateStr = "other"; break;
                    default: stateStr = "other"; break;
                }
                worldBefore = reconnectWorldSeenBeforeEpisode ? 1 : 0;
            }

            StringBuffer sb = new StringBuffer(128);
            sb.append("v=1\n");
            sb.append("t=").append(now).append('\n');
            sb.append("seq=").append(reconnectStatusSeq).append('\n');
            sb.append("episode=").append(reconnectEpisodeId).append('\n');
            sb.append("active=").append(active).append('\n');
            sb.append("state=").append(stateStr).append('\n');
            sb.append("transitions=").append(reconnectTransitions).append('\n');
            sb.append("world_before=").append(worldBefore).append('\n');

            writeReconnectStatus(sb.toString());
        } catch (Throwable t) {
            // Fail silent: status publishing must never stall the client tick.
        }
    }

    private static void writeReconnectStatus(String body) {
        if (reconnectStatusPath == null) {
            return;
        }
        java.io.OutputStream stream = null;
        try {
            java.io.File target = new java.io.File(reconnectStatusPath);
            java.io.File temp = new java.io.File(reconnectStatusPath + ".tmp");
            stream = new java.io.FileOutputStream(temp);
            stream.write(body.getBytes("UTF-8"));
            stream.close();
            stream = null;
            if (target.exists() && !target.delete()) {
                return;
            }
            temp.renameTo(target);
        } catch (Throwable t) {
            // Fail silent
        } finally {
            if (stream != null) {
                try {
                    stream.close();
                } catch (Throwable ignored) {
                }
            }
        }
    }

    // ---- INVENTORY TELEMETRY (ENHANCE-01) -------------------------------------
    /**
     * Serializes the current bag inventory (Item.VecInvetoryPlayer) in stable slot order.
     * Zero-mutation: reads fields, sends no packets, never mutates Item.VecInvetoryPlayer items.
     */
    public static String formatInventoryCatalogJson() {
        StringBuffer out = new StringBuffer(512);
        int capacity = Player.maxInven > 0 ? Player.maxInven : 0;
        out.append("{\n");
        out.append("  \"version\": 1,\n");
        out.append("  \"bag_capacity\": ").append(capacity).append(",\n");
        out.append("  \"items\": [");
        if (Item.VecInvetoryPlayer != null && Item.VecInvetoryPlayer.size() > 0) {
            boolean first = true;
            for (int slot = 0; slot < Item.VecInvetoryPlayer.size(); slot++) {
                Object entry = Item.VecInvetoryPlayer.elementAt(slot);
                if (entry == null || !(entry instanceof Item)) {
                    continue;
                }
                Item it = (Item) entry;
                if (!first) {
                    out.append(",");
                }
                first = false;
                out.append("\n    {\n");
                out.append("      \"slot\": ").append(slot).append(",\n");
                out.append("      \"template_id\": ").append(it.Id).append(",\n");
                out.append("      \"category\": ").append(it.ItemCatagory).append(",\n");
                out.append("      \"base_name\": ");
                String baseName = (it.itemNameExcludeLv != null && it.itemNameExcludeLv.length() > 0) ? it.itemNameExcludeLv : (it.itemName != null ? it.itemName : "");
                escapeJsonString(baseName, out);
                out.append(",\n");
                out.append("      \"display_name\": ");
                escapeJsonString(it.itemName != null ? it.itemName : "", out);
                out.append(",\n");
                out.append("      \"level\": ").append((int) it.tier).append(",\n");
                out.append("      \"tier\": ").append(it.colorNameItem).append(",\n");
                out.append("      \"count\": ").append(it.numPotion > 0 ? it.numPotion : 1).append(",\n");
                out.append("      \"durability\": ");
                if (it.IdTem >= 0) {
                    out.append(it.IdTem);
                } else {
                    out.append("null");
                }
                out.append(",\n");
                out.append("      \"bind\": ");
                if (it.isLock >= 0) {
                    out.append((int) it.isLock);
                } else {
                    out.append("null");
                }
                out.append(",\n");
                out.append("      \"icon\": ");
                if (it.imageId >= 0) {
                    out.append(it.imageId);
                } else {
                    out.append("null");
                }
                out.append(",\n");
                out.append("      \"candidate_for_enhancement\": ").append(it.ItemCatagory == 3 ? "true" : "false").append("\n");
                out.append("    }");
            }
            if (!first) {
                out.append("\n  ");
            }
        }
        out.append("]\n");
        out.append("}\n");
        return out.toString();
    }

    /** Fast deterministic hash for change suppression. */
    public static long computeInventoryHash() {
        long hash = 17L;
        hash = 31L * hash + (long) (Player.maxInven > 0 ? Player.maxInven : 0);
        int count = Item.VecInvetoryPlayer != null ? Item.VecInvetoryPlayer.size() : 0;
        hash = 31L * hash + (long) count;
        if (Item.VecInvetoryPlayer != null) {
            for (int i = 0; i < count; i++) {
                Object entry = Item.VecInvetoryPlayer.elementAt(i);
                if (entry instanceof Item) {
                    Item it = (Item) entry;
                    hash = 31L * hash + (long) i;
                    hash = 31L * hash + (long) it.Id;
                    hash = 31L * hash + (long) it.ItemCatagory;
                    hash = 31L * hash + (long) it.tier;
                    hash = 31L * hash + (long) it.colorNameItem;
                    hash = 31L * hash + (long) it.numPotion;
                    hash = 31L * hash + (long) it.IdTem;
                    hash = 31L * hash + (long) it.isLock;
                    hash = 31L * hash + (long) it.imageId;
                    if (it.itemName != null) {
                        hash = 31L * hash + (long) it.itemName.hashCode();
                    }
                }
            }
        }
        return hash;
    }

    /** Publishes inventory sidecar with change suppression. */
    private static void publishInventory(long now) {
        if (inventoryPath == null || Item.VecInvetoryPlayer == null) {
            return;
        }
        try {
            long currentHash = computeInventoryHash();
            if (inventoryWritten && currentHash == lastInventoryHash) {
                return;
            }
            String body = formatInventoryCatalogJson();
            writeInventory(body);
            lastInventoryHash = currentHash;
            inventoryWritten = true;
        } catch (Throwable t) {
            // A sensor must never stall the client tick.
        }
    }

    /** Atomically writes the inventory sidecar file. */
    private static void writeInventory(String body) {
        java.io.OutputStream stream = null;
        try {
            java.io.File target = new java.io.File(inventoryPath);
            java.io.File temp = new java.io.File(inventoryPath + ".tmp");
            stream = new java.io.FileOutputStream(temp);
            stream.write(body.getBytes("UTF-8"));
            stream.close();
            stream = null;
            if (target.exists() && !target.delete()) {
                return;
            }
            temp.renameTo(target);
        } catch (Throwable t) {
            // Unwritable path: stay silent rather than spamming.
        } finally {
            if (stream != null) {
                try {
                    stream.close();
                } catch (Throwable ignored) {
                }
            }
        }
    }
    // ---- end INVENTORY TELEMETRY ----------------------------------------------

    // ---- GUARDS ---------------------------------------------------------------
    //
    // docs/core/08-module-attack.md §10. Two of these — LoadMapScreen.isNextMap and LoadMap.isShowEffAuto != LoadMap.EFF_PHOBANG_END — are the
    // ones KnightMod never reads, which is how it ends up acting on the previous map's
    // coordinates while a new map loads.

    /** In the world screen, not a menu. */
    private static boolean inGame() {
        return GameCanvas.currentScreen != null && GameCanvas.currentScreen == GameCanvas.game;
    }

    /** No dialog is up. Acting behind one sends input the operator cannot see. */
    private static boolean noDialog() {
        return GameCanvas.currentDialog == null && GameCanvas.subDialog == null && (GameCanvas.menu2 == null || !GameCanvas.menu2.isShowMenu);
    }

    private static boolean alive() {
        return GameScreen.player != null && GameScreen.player.Action != 4;
    }

    /** Free to move: neither our own path nor the client's movement lock is held. */
    private static boolean canMove() {
        return !Player.isLockKey && GameScreen.player != null && GameScreen.player.posTransRoad == null;
    }

    /** The captcha monster ("Con Ma") outranks everything; never fight through it. */
    private static boolean captcha() {
        return GameScreen.ObjFocus != null && GameScreen.ObjFocus.typeBoss == 2;
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
        return inGame() && sceneReady() && GameCanvas.loadmap != null && GameCanvas.loadmap.idMap >= 0
                && GameCanvas.loadmap.idMap == stableMapId && mapStableTicks >= MAP_STABLE_TICKS;
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
                || GameScreen.player == null || GameScreen.player.x < 0 || GameScreen.player.y < 0 || GameCanvas.loadmap == null || GameCanvas.loadmap.idMap < 0) {
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
        cleanEnhancementRouting();
        recoverEnhancementSession();
        dungeonSessionReset();
    }

    private static void sessionTick() {
        int screenId = (GameCanvas.currentScreen == null) ? -1 : ((GameCanvas.currentScreen == GameCanvas.selectChar) ? 1 : ((GameCanvas.currentScreen == GameCanvas.game) ? 2 : 0));
        if (screenId != lastScreenId) {
            if (screenId == 1 || screenId == 2) {
                sessionReset();
            }
            lastScreenId = screenId;
        }
        reconcileVisualQoL();
        if (inGame() && sceneReady()) {
            if (readySettleTicks < GAME_READY_SETTLE_TICKS) {
                ++readySettleTicks;
            }
            if (GameCanvas.loadmap != null && GameCanvas.loadmap.idMap >= 0) {
                int curMap = GameCanvas.loadmap.idMap;
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

    // ---- RECONNECT (R2A OBSERVE-ONLY SUPERVISOR) ------------------------------
    // Models reconnect episodes and native lifecycle transitions before recovery actions are authorized.
    // R2A strictly observes and traces: no dialog dismissal, no login packets, no field mutations.

    public static final int RC_IDLE = 0;
    public static final int RC_NATIVE_WAIT = 1;
    public static final int RC_LOGIN = 2;
    public static final int RC_SERVER = 3;
    public static final int RC_CHARACTER = 4;
    public static final int RC_WORLD_SETTLE = 5;
    public static final int RC_OTHER = 6;
    public static final int RC_LOADING = 7;

    private static boolean reconnectEpisodeActive = false;
    private static int reconnectState = RC_IDLE;
    private static long reconnectStartedAt = 0L;
    private static long reconnectStateSince = 0L;
    private static int reconnectEpisodeId = 0;
    private static int reconnectTransitions = 0;
    private static boolean reconnectEverStableWorldSeen = false;
    private static boolean reconnectWorldSeenBeforeEpisode = false;
    private static String reconnectLastReason = "";

    // R2B1: Bounded Native Dialog Fallback Recovery Policy
    private static final long RC_RECOVERY_COOLDOWN_MS = 5000L;
    private static final int RC_RECOVERY_MAX_ACTIONS = 4;
    private static final long RC_RECOVERY_BACKOFF_MS = 600000L;
    private static final long RC_NATIVE_GRACE_MS = 5000L;
    private static final long RC_STATE_DWELL_MS = 5000L;

    private static int reconnectRecoveryAttempts = 0;
    private static long reconnectRecoveryLastActionAt = 0L;
    private static long reconnectRecoveryBackoffUntil = 0L;
    private static String reconnectRecoveryLastFingerprint = "";

    // R2B2: Bounded Native Login Recovery Policy
    public static final long RC_LOGIN_DWELL_MS = 5000L;
    public static final long RC_LOGIN_RETRY_INTERVAL_MS = 30000L;
    public static final int RC_LOGIN_MAX_ACTIONS = 3;
    public static final long RC_LOGIN_BACKOFF_MS = 600000L;

    private static int reconnectLoginAttempts = 0;
    private static long reconnectLoginLastActionAt = 0L;
    private static long reconnectLoginBackoffUntil = 0L;
    private static String reconnectLoginLastFingerprint = "";

    public static boolean isReconnectEpisodeActive() {
        return reconnectEpisodeActive;
    }

    public static int getReconnectState() {
        return reconnectState;
    }

    public static int getReconnectEpisodeId() {
        return reconnectEpisodeId;
    }

    public static int getReconnectTransitions() {
        return reconnectTransitions;
    }

    public static boolean isReconnectEverStableWorldSeen() {
        return reconnectEverStableWorldSeen;
    }

    public static boolean isReconnectWorldSeenBeforeEpisode() {
        return reconnectWorldSeenBeforeEpisode;
    }

    public static String getReconnectLastReason() {
        return reconnectLastReason;
    }

    public static int getReconnectRecoveryAttempts() {
        return reconnectRecoveryAttempts;
    }

    public static long getReconnectRecoveryLastActionAt() {
        return reconnectRecoveryLastActionAt;
    }

    public static long getReconnectRecoveryBackoffUntil() {
        return reconnectRecoveryBackoffUntil;
    }

    public static String getReconnectRecoveryLastFingerprint() {
        return reconnectRecoveryLastFingerprint;
    }

    public static int getReconnectLoginAttempts() {
        return reconnectLoginAttempts;
    }

    public static long getReconnectLoginLastActionAt() {
        return reconnectLoginLastActionAt;
    }

    public static long getReconnectLoginBackoffUntil() {
        return reconnectLoginBackoffUntil;
    }

    public static String getReconnectLoginLastFingerprint() {
        return reconnectLoginLastFingerprint;
    }

    private static String stateName(int state) {
        switch (state) {
            case RC_IDLE: return "RC_IDLE";
            case RC_NATIVE_WAIT: return "RC_NATIVE_WAIT";
            case RC_LOGIN: return "RC_LOGIN";
            case RC_SERVER: return "RC_SERVER";
            case RC_CHARACTER: return "RC_CHARACTER";
            case RC_WORLD_SETTLE: return "RC_WORLD_SETTLE";
            case RC_OTHER: return "RC_OTHER";
            case RC_LOADING: return "RC_LOADING";
            default: return "RC_UNKNOWN(" + state + ")";
        }
    }

    /**
     * Conservative observer-only classifier for strong transport/disconnect evidence.
     * Evaluates GlobalLogicHandler.isDisConect and strongly recognized disconnect phrases in modal dialogs.
     */
    private static String detectStrongDisconnectReason() {
        if (GlobalLogicHandler.isDisConect) {
            return "NATIVE_BV_A";
        }
        if (GameCanvas.currentDialog != null && (GameCanvas.currentDialog instanceof MsgDialog)) {
            String text = norm(dialogText(GameCanvas.currentDialog));
            if (text.indexOf("mat ket noi") >= 0) {
                return "MODAL_DISCONNECT_MAT_KET_NOI";
            }
            if (text.indexOf("ket noi that bai") >= 0) {
                return "MODAL_DISCONNECT_KET_NOI_THAT_BAI";
            }
            if (text.indexOf("vui long dang nhap lai") >= 0) {
                return "MODAL_DISCONNECT_VUI_LONG_DANG_NHAP_LAI";
            }
        }
        return null;
    }

    /**
     * Inspects a live MsgDialog modal dialog and finds the exact reconnect OK button.
     * Candidate must be a iCommand object, candidate command id iCommand.e must equal 0,
     * and candidate caption must normalize to an explicitly OK-like caption ("ok" or "o k").
     * Returns null if no exact command can be proven.
     */
    private static iCommand findReconnectOkButton(MsgDialog dialog) {
        if (dialog == null) {
            return null;
        }
        mVector buttons = dialog.cmdList;
        if (buttons == null) {
            return null;
        }
        for (int i = 0; i < buttons.size(); i++) {
            Object obj = buttons.elementAt(i);
            if (obj instanceof iCommand) {
                iCommand btn = (iCommand) obj;
                if (btn.indexMenu == 0) {
                    String cap = norm(btn.caption).trim();
                    if (cap.equals("ok") || cap.equals("o k")) {
                        return btn;
                    }
                }
            }
        }
        return null;
    }

    /**
     * Evaluates and executes the bounded native reconnect fallback action.
     * Guaranteed to only be called when reconnectEpisodeActive == true,
     * reconnectState == RC_NATIVE_WAIT, and reconnectWorldSeenBeforeEpisode == true.
     */
    private static void evaluateReconnectRecoveryAction(long now) {
        if (GameCanvas.currentDialog == null || !(GameCanvas.currentDialog instanceof MsgDialog)) {
            return;
        }
        MsgDialog dialog = (MsgDialog) GameCanvas.currentDialog;
        String rawText = dialogText(dialog);
        String text = norm(rawText);

        // Fail closed unless proven disconnect modal
        boolean isProvenDisconnect = (text.indexOf("mat ket noi") >= 0
                || text.indexOf("ket noi that bai") >= 0
                || text.indexOf("vui long dang nhap lai") >= 0);
        if (!isProvenDisconnect) {
            return;
        }
        // In-progress connecting or wait dialogs must fail closed
        if (text.indexOf("dang ket noi") >= 0 || text.indexOf("vui long cho") >= 0
                || text.indexOf("cho ket noi") >= 0) {
            return;
        }

        iCommand okBtn = findReconnectOkButton(dialog);
        if (okBtn == null) {
            return;
        }

        // Native deadline & grace policy: wait for native GlobalLogicHandler.timeReconnect deadline + 5000 ms grace
        if (GlobalLogicHandler.isDisConect && GlobalLogicHandler.timeReconnect > 0L) {
            if (now < GlobalLogicHandler.timeReconnect + RC_NATIVE_GRACE_MS) {
                return;
            }
        }

        // Require current RC_NATIVE_WAIT state to have dwelled for at least RC_STATE_DWELL_MS (5000 ms)
        long dwell = (now >= reconnectStateSince) ? (now - reconnectStateSince) : 0L;
        if (dwell < RC_STATE_DWELL_MS) {
            return;
        }

        // Cooldown check (5000 ms)
        if (reconnectRecoveryLastActionAt > 0L) {
            long elapsed = (now >= reconnectRecoveryLastActionAt) ? (now - reconnectRecoveryLastActionAt) : -1L;
            if (elapsed < RC_RECOVERY_COOLDOWN_MS) {
                return;
            }
        }

        // Backoff check (600000 ms after 4 actions)
        if (reconnectRecoveryBackoffUntil > 0L) {
            if (now < reconnectRecoveryBackoffUntil) {
                return;
            } else {
                // Backoff expired; reset attempts for another bounded cycle
                reconnectRecoveryBackoffUntil = 0L;
                reconnectRecoveryAttempts = 0;
            }
        }

        // Final live dialog and button revalidation before accounting and dispatch
        if (GameCanvas.currentDialog != dialog || !(GameCanvas.currentDialog instanceof MsgDialog)) {
            return;
        }
        MsgDialog liveDialog = (MsgDialog) GameCanvas.currentDialog;
        String liveRawText = dialogText(liveDialog);
        String liveText = norm(liveRawText);
        boolean liveProvenDisconnect = (liveText.indexOf("mat ket noi") >= 0
                || liveText.indexOf("ket noi that bai") >= 0
                || liveText.indexOf("vui long dang nhap lai") >= 0);
        if (!liveProvenDisconnect) {
            return;
        }
        if (liveText.indexOf("dang ket noi") >= 0 || liveText.indexOf("vui long cho") >= 0
                || liveText.indexOf("cho ket noi") >= 0) {
            return;
        }
        iCommand liveOkBtn = findReconnectOkButton(liveDialog);
        if (liveOkBtn == null || liveOkBtn != okBtn) {
            return;
        }

        // Diagnostics fingerprint (deduplicated tracing)
        mVector btns = liveDialog.cmdList;
        int btnCount = (btns != null) ? btns.size() : 0;
        reconnectRecoveryLastFingerprint = liveText + "|" + btnCount + "|" + okBtn.indexMenu + ":" + norm(okBtn.caption).trim();

        // Increment and arm action accounting BEFORE dispatch (mandatory ordering)
        reconnectRecoveryAttempts++;
        reconnectRecoveryLastActionAt = now;
        if (reconnectRecoveryAttempts >= RC_RECOVERY_MAX_ACTIONS) {
            reconnectRecoveryBackoffUntil = now + RC_RECOVERY_BACKOFF_MS;
        }

        trace("RECONNECT recovery action try=" + reconnectRecoveryAttempts
                + " max=" + RC_RECOVERY_MAX_ACTIONS
                + " btn=" + clean(okBtn.caption)
                + " cmd=" + okBtn.indexMenu
                + (reconnectRecoveryBackoffUntil > 0L ? " backoffMs=" + RC_RECOVERY_BACKOFF_MS : ""));

        // Dispatch exact live native OK command
        okBtn.perform();
    }

    /**
     * Evaluates and executes the bounded native LoginScreen recovery action.
     * Guaranteed to only be called when reconnectEpisodeActive == true,
     * reconnectWorldSeenBeforeEpisode == true, and reconnectState == RC_LOGIN.
     */
    private static void evaluateReconnectLoginAction(long now) {
        if (!reconnectEpisodeActive || !reconnectWorldSeenBeforeEpisode || reconnectState != RC_LOGIN) {
            return;
        }
        if (detectStrongDisconnectReason() != null) {
            return;
        }

        // Clean UI routing checks
        if (GameCanvas.currentScreen == null || GameCanvas.currentScreen != GameCanvas.login || GameCanvas.login == null) {
            return;
        }
        if (GameCanvas.currentDialog != null || GameCanvas.subDialog != null) {
            return;
        }
        if (GameCanvas.menu2 == null || GameCanvas.menu2.isShowMenu) {
            return;
        }
        if (ChatTextField.isShow) {
            return;
        }

        // Live center command slot check
        iCommand loginBtn = GameCanvas.login.right;
        if (loginBtn == null || loginBtn.indexMenu != 0 || loginBtn.caption == null) {
            return;
        }
        String caption = norm(loginBtn.caption).trim();
        if (!caption.equals("choi tiep")) {
            return;
        }

        // Normal textbox credentials check (LoginScreen.tfusername is username, LoginScreen.tfpassword is password)
        if (LoginScreen.tfusername == null || LoginScreen.tfpassword == null) {
            return;
        }
        String user = LoginScreen.tfusername.getText();
        String pass = LoginScreen.tfpassword.getText();
        if (user == null || user.trim().length() == 0 || pass == null || pass.trim().length() == 0) {
            // Special credential mode or empty normal fields: FAIL_CLOSED
            return;
        }

        // Dwell time: RC_LOGIN must have dwelled for at least RC_LOGIN_DWELL_MS (5000 ms)
        long dwell = (now >= reconnectStateSince) ? (now - reconnectStateSince) : 0L;
        if (dwell < RC_LOGIN_DWELL_MS) {
            return;
        }

        // Backoff check: 10 minutes (600000 ms) after 3 actions
        if (reconnectLoginBackoffUntil > 0L) {
            if (now < reconnectLoginBackoffUntil) {
                return;
            } else {
                // Backoff expired; reset attempts for another bounded cycle
                reconnectLoginBackoffUntil = 0L;
                reconnectLoginAttempts = 0;
            }
        }

        // Cooldown / retry interval check: at least 30000 ms between actions
        if (reconnectLoginLastActionAt > 0L) {
            long elapsed = (now >= reconnectLoginLastActionAt) ? (now - reconnectLoginLastActionAt) : -1L;
            if (elapsed < RC_LOGIN_RETRY_INTERVAL_MS) {
                return;
            }
        }

        // Final complete live LoginScreen routing and credential revalidation before accounting and dispatch
        if (GameCanvas.currentScreen != GameCanvas.login || GameCanvas.login == null || GameCanvas.currentDialog != null || GameCanvas.subDialog != null
                || GameCanvas.menu2 == null || GameCanvas.menu2.isShowMenu || ChatTextField.isShow
                || GameCanvas.login.right != loginBtn || loginBtn.indexMenu != 0 || loginBtn.caption == null
                || !norm(loginBtn.caption).trim().equals("choi tiep")
                || LoginScreen.tfusername == null || LoginScreen.tfpassword == null) {
            return;
        }
        String liveUser = LoginScreen.tfusername.getText();
        String livePass = LoginScreen.tfpassword.getText();
        if (liveUser == null || liveUser.trim().length() == 0
                || livePass == null || livePass.trim().length() == 0) {
            return;
        }

        // Deduplicated diagnostics fingerprint (non-sensitive action identity only: no username/password)
        reconnectLoginLastFingerprint = caption + "|" + loginBtn.indexMenu;

        // Action accounting MUST be armed before loginBtn.perform() dispatch
        reconnectLoginAttempts++;
        reconnectLoginLastActionAt = now;
        if (reconnectLoginAttempts >= RC_LOGIN_MAX_ACTIONS) {
            reconnectLoginBackoffUntil = now + RC_LOGIN_BACKOFF_MS;
        }

        trace("RECONNECT login action try=" + reconnectLoginAttempts
                + " max=" + RC_LOGIN_MAX_ACTIONS
                + " btn=" + clean(loginBtn.caption)
                + " cmd=" + loginBtn.indexMenu
                + (reconnectLoginBackoffUntil > 0L ? " backoffMs=" + RC_LOGIN_BACKOFF_MS : ""));

        loginBtn.perform();
    }

    /**
     * Authoritative world readiness predicate for reconnect lifecycle.
     * Evaluates map stability and existence of authoritative player object.
     * Does NOT require alive(), absence of captcha, or absence of non-disconnect dialog.
     */
    private static boolean reconnectWorldReady() {
        return GameCanvas.currentScreen == GameCanvas.game && GameScreen.player != null && mapStable();
    }

    /**
     * Ticked from tick() after sessionTick() and before auth().
     * A transient GameCanvas.currentScreen == null pauses observation without resetting the active episode.
     */
    private static void reconnectSupervisorTick() {
        try {
            long now = mSystem.currentTimeMillis();

            // Clock rollback defensive re-anchor
            if (now < reconnectStartedAt) {
                reconnectStartedAt = now;
            }
            if (now < reconnectStateSince) {
                reconnectStateSince = now;
            }
            if (now < reconnectRecoveryLastActionAt) {
                if (reconnectRecoveryBackoffUntil > reconnectRecoveryLastActionAt) {
                    reconnectRecoveryBackoffUntil = now + RC_RECOVERY_BACKOFF_MS;
                }
                reconnectRecoveryLastActionAt = now;
            }
            if (now < reconnectLoginLastActionAt) {
                if (reconnectLoginBackoffUntil > reconnectLoginLastActionAt) {
                    reconnectLoginBackoffUntil = now + RC_LOGIN_BACKOFF_MS;
                }
                reconnectLoginLastActionAt = now;
            }

            // Normal prior stable gameplay sets reconnectEverStableWorldSeen when no episode is active
            if (!reconnectEpisodeActive && reconnectWorldReady()) {
                reconnectEverStableWorldSeen = true;
            }

            String reason = detectStrongDisconnectReason();
            boolean strongDisconnect = (reason != null);

            // EPISODE OPENING RULE
            if (!reconnectEpisodeActive) {
                if (strongDisconnect) {
                    reconnectEpisodeActive = true;
                    reconnectEpisodeId++;
                    reconnectStartedAt = now;
                    reconnectStateSince = now;
                    reconnectState = RC_NATIVE_WAIT;
                    reconnectTransitions = 0;
                    // R2A-S2: snapshot immutable copy from reconnectEverStableWorldSeen exactly once at open
                    reconnectWorldSeenBeforeEpisode = reconnectEverStableWorldSeen;
                    reconnectLastReason = reason;

                    // R2B1: reset per-episode recovery state
                    reconnectRecoveryAttempts = 0;
                    reconnectRecoveryBackoffUntil = 0L;
                    reconnectRecoveryLastFingerprint = "";

                    // R2B2: reset per-episode login state
                    reconnectLoginAttempts = 0;
                    reconnectLoginLastActionAt = 0L;
                    reconnectLoginBackoffUntil = 0L;
                    reconnectLoginLastFingerprint = "";

                    trace("RECONNECT episode open id=" + reconnectEpisodeId
                            + " reason=" + reason
                            + " worldSeenBefore=" + reconnectWorldSeenBeforeEpisode);
                }
                return;
            }

            // ACTIVE EPISODE HANDLING
            if (GameCanvas.currentScreen == null) {
                // Transient null-screen pauses observation; does NOT reset episode
                return;
            }

            // SUCCESSFUL CLOSE RULE (R2A-S1 & R2A-S3 & R2C):
            // An active episode is successful only when strongDisconnect is false AND reconnectWorldReady().
            // strongDisconnect strictly outranks screen readiness and prevents successful close.
            if (!strongDisconnect && reconnectWorldReady()) {
                reconnectEverStableWorldSeen = true;
                if (reconnectState != RC_IDLE) {
                    reconnectTransitions++;
                }
                long duration = (now >= reconnectStartedAt) ? (now - reconnectStartedAt) : 0L;
                trace("RECONNECT episode close id=" + reconnectEpisodeId
                        + " status=SUCCESS durationMs=" + duration
                        + " transitions=" + reconnectTransitions);
                reconnectEpisodeActive = false;
                reconnectState = RC_IDLE;
                reconnectStateSince = now;
                reconnectLastReason = "";

                // R2B1: reset per-episode recovery state on successful close
                reconnectRecoveryAttempts = 0;
                reconnectRecoveryBackoffUntil = 0L;
                reconnectRecoveryLastFingerprint = "";

                // R2B2: reset per-episode login state on successful close
                reconnectLoginAttempts = 0;
                reconnectLoginLastActionAt = 0L;
                reconnectLoginBackoffUntil = 0L;
                reconnectLoginLastFingerprint = "";
                return;
            }

            // DERIVE STATE FOR ACTIVE EPISODE
            int targetState;
            if (strongDisconnect) {
                targetState = RC_NATIVE_WAIT;
            } else if (GameCanvas.currentScreen == GameCanvas.load) {
                targetState = RC_LOADING;
            } else if (GameCanvas.currentScreen == GameCanvas.login && GameCanvas.subDialog == GameCanvas.msgchat) {
                targetState = RC_SERVER;
            } else if (GameCanvas.currentScreen == GameCanvas.login) {
                targetState = RC_LOGIN;
            } else if (GameCanvas.currentScreen == GameCanvas.selectChar) {
                targetState = RC_CHARACTER;
            } else if (GameCanvas.currentScreen == GameCanvas.game) {
                targetState = RC_WORLD_SETTLE;
            } else {
                targetState = RC_OTHER;
            }

            if (targetState != reconnectState) {
                int oldState = reconnectState;
                reconnectState = targetState;
                reconnectStateSince = now;
                reconnectTransitions++;
                trace("RECONNECT transition id=" + reconnectEpisodeId
                        + " from=" + stateName(oldState)
                        + " to=" + stateName(targetState)
                        + " count=" + reconnectTransitions);
            }

            // R2B1: BOUNDED NATIVE RECOVERY ACTION
            // Action is only authorized when reconnectState == RC_NATIVE_WAIT and reconnectWorldSeenBeforeEpisode == true
            if (reconnectState == RC_NATIVE_WAIT && reconnectWorldSeenBeforeEpisode) {
                evaluateReconnectRecoveryAction(now);
            }

            // R2B2: BOUNDED NATIVE LOGIN RECOVERY ACTION
            // Action is only authorized when reconnectState == RC_LOGIN and reconnectWorldSeenBeforeEpisode == true
            if (reconnectState == RC_LOGIN && reconnectWorldSeenBeforeEpisode) {
                evaluateReconnectLoginAction(now);
            }
        } catch (Throwable t) {
            // Fail silent: supervisor observation must never stall client tick
        }
    }
    // ---- end RECONNECT --------------------------------------------------------

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
                || text.indexOf("huong dan") >= 0
                || text.indexOf("vui long cho") >= 0) {
            return true;
        }
        return false;
    }

    /**
     * Validates that the dialog has exactly one button and that it is an acknowledge/close button.
     * Never confirms single buttons with "Đồng ý" (dong y) or multi-button choice dialogs.
     */
    private static iCommand findDismissButton(mVector buttons) {
        if (buttons == null || buttons.size() != 1) {
            return null;
        }
        Object entry = buttons.elementAt(0);
        if (!(entry instanceof iCommand)) {
            return null;
        }
        iCommand btn = (iCommand) entry;
        String cap = norm(btn.caption).trim();
        if (cap.equals("dong") || cap.equals("ok") || cap.equals("dong tab nay")
                || cap.equals("tro ve") || cap.equals("MainDialog hieu")) {
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
        if (GameCanvas.currentDialog == null) {
            if (dialogStableTicks > 0 || dialogLastFingerprint.length() > 0) {
                dialogLastFingerprint = "";
                dialogStableTicks = 0;
                dialogTries = 0;
            }
            if (enhOwnsResultDialog && enhOwnsForgeScreen && isForgeScreenOpen()
                    && GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu && Menu2.isNPCMenu == 1) {
                if (dialogTries < DIALOG_MAX_TRIES) {
                    ++dialogTries;
                    trace("DIALOG owned enhancement GameCanvas.menu2 notification dismissed try=" + dialogTries);
                    GameCanvas.menu2.doCloseMenu();
                    enhOwnsResultDialog = false;
                    GameCanvas.clearKeyHold();
                    if (isEnhancementStateTerminal(enhState)) {
                        cleanEnhancementRouting();
                    }
                }
            }
            return;
        }
        if (!(GameCanvas.currentDialog instanceof MsgDialog)) {
            return;
        }
        MsgDialog dialog = (MsgDialog) GameCanvas.currentDialog;
        String rawText = dialogText(dialog);
        String text = norm(rawText);
        mVector buttons = dialog.cmdList;
        int btnCount = (buttons != null) ? buttons.size() : 0;

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
        iCommand dismissBtn = allowlisted ? findDismissButton(buttons) : null;

        if (dismissBtn != null) {
            if (dialogTries < DIALOG_MAX_TRIES) {
                ++dialogTries;
                trace("DIALOG dismissed try=" + dialogTries + " text=" + clean(text));
                dismissBtn.perform();
            }
        } else {
            // Unknown or non-dismissible dialog: fail closed and emit deduplicated diagnostic trace
            if (!fingerprint.equals(dialogLastTracedFingerprint)) {
                dialogLastTracedFingerprint = fingerprint;
                StringBuffer caps = new StringBuffer();
                if (buttons != null) {
                    for (int i = 0; i < buttons.size(); i++) {
                        Object b = buttons.elementAt(i);
                        if (b instanceof iCommand) {
                            if (caps.length() > 0) caps.append(',');
                            caps.append(clean(((iCommand) b).caption));
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
    //      client's own ez.GiaoTiep() sends, reads the destinations the server itself names, and picks
    //      the one that leaves the least walking. KnightMod's approach loop and its lag counters
    //      are not ported: they solved a problem that turned out not to exist.
    //   2. Walking out through a map exit. The client is handed every exit of the current map,
    //      with the destination map's NAME, in the map-load packet (LoadMap.vecPointChange). So the only surveyed
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
        if (navTarget >= 0 && !navDone) {
            return navTarget;
        }
        if (enhNavigating) {
            return BLACKSMITH_MAP;
        }
        if (dungeonNavigating) {
            return DUNGEON_NPC_MAP;
        }
        return -1;
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
        Player.isLockKey = false;
        if (GameScreen.player != null) {
            GameScreen.player.posTransRoad = null;
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
                    || GameScreen.player == null || GameCanvas.loadmap == null) {
                return;             // keep the intent; a load screen is not a failure
            }
            // Deliberately NOT ready(): that requires no dialog, and an open menu is exactly the
            // state a stone reply arrives in. Gating on it here would deadlock — the menu blocks
            // travel, and only travel closes the menu. Route decisions additionally require mapStable().
            if (travelState != TV_STONE_WAIT && (!gameReady() || !mapStable())) {
                return;
            }
            int here = GameCanvas.loadmap.idMap;
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
                Player.isLockKey = false;
                if (GameScreen.player != null) {
                    GameScreen.player.posTransRoad = null;
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
                if (GameScreen.player.x == travelLastX && GameScreen.player.y == travelLastY) {
                    ++travelStallTicks;
                } else {
                    travelStallTicks = 0;
                    travelLastX = GameScreen.player.x;
                    travelLastY = GameScreen.player.y;
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
        if (GameScreen.Vecplayers == null) {
            return false;
        }
        MainObject stone = null;
        for (int i = 0; i < GameScreen.Vecplayers.size(); i++) {
            Object entry = GameScreen.Vecplayers.elementAt(i);
            if (!(entry instanceof MainObject)) {
                continue;
            }
            MainObject candidate = (MainObject) entry;
            // Matched by name: a map can carry more than one stone and their cu differs per region,
            // so cu identifies which stone this is rather than what it is.
            if (candidate.typeObject != 2 || norm(candidate.name).indexOf("dich chuyen") < 0) {
                continue;
            }
            if (candidate.ID == travelStoneAsked) {
                continue;           // already asked this one on this map; try the other
            }
            stone = candidate;
            break;
        }
        if (stone == null) {
            return false;
        }
        travelStoneAsked = stone.ID;
        ++travelStoneTried;
        travelMenu = null;
        travelMenuNpc = Integer.MIN_VALUE;
        try {
            GlobalService.gI().chat_npc((byte) stone.ID);
        } catch (Throwable t) {
            return false;
        }
        travelState = TV_STONE_WAIT;
        travelWait = 20;            // ~20 ticks for the reply; the menu usually lands well inside
        trace("TRAVEL asked stone cu=" + stone.ID + " on map " + here);
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
        int hereScore = mapDistance(GameCanvas.loadmap.idMap, dest);
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
            // packet Menu2.a(2, _) builds when the operator taps the row.
            GlobalService.gI().Dynamic_Menu((short) npc, (byte) menuId, (byte) best);
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
            if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                GameCanvas.menu2.doCloseMenu();
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
     * (LoadMap.vecPointChange, "LoadMap vecPointChange"), so no exit coordinates are surveyed here. The one surveyed
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
        if (LoadMap.vecPointChange == null) {
            return;
        }
        for (int i = 0; i < LoadMap.vecPointChange.size(); i++) {
            Object entry = LoadMap.vecPointChange.elementAt(i);
            if (!(entry instanceof Point)) {
                continue;
            }
            Point gate = (Point) entry;
            if (gate.name == null || norm(gate.name).indexOf(needle) < 0) {
                continue;
            }
            if (travelState != TV_WALK) {
                trace("TRAVEL walk to map " + hop + " via " + clean(gate.name)
                        + " at " + gate.x + "," + gate.y);
            }
            travelState = TV_WALK;
            travelMove(here, gate.x, gate.y);
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
            short[] path = GameCanvas.game.updateFindRoad(goX / 24, goY / 24, GameScreen.player.x / 24, GameScreen.player.y / 24, 500);
            if (path == null || path.length > 500) {
                travelStallTicks += 4;
                return;
            }
            GameScreen.player.posTransRoad = path;
            GameScreen.player.countAutoMove = 0;
            GameScreen.player.xStopMove = 0;
            GameScreen.player.yStopMove = 0;
            GameScreen.player.toX = GameScreen.player.x;
            GameScreen.player.toY = GameScreen.player.y;
            Player.isLockKey = true;
        } catch (Throwable t) {
            Player.isLockKey = false;
            travelStallTicks += 4;
        }
    }

    /**
     * Declares the walk over and clears every field that means "still moving".
     *
     * `cO` alone is not enough. `MainMonster.java:187` only drops `cG` back to 0 once **both** `bc` and `bd`
     * are zero, so a leftover pixel offset keeps the client in its walking state and every later step
     * is refused in silence — no error, nothing in a trace, just a character that never moves again.
     * `bg`/`bh` are pinned to where the character actually is for the same reason: a stale step target
     * is a walk the client believes it still owes.
     *
     * Manhattan, not Euclid: the client's own reach checks are Manhattan and a mixed metric would call
     * "arrived" at a distance the client still considers travelling.
     */
    private static boolean travelArrive(int x, int y, int tolerance) {
        if (GameScreen.player == null) {
            return false;
        }
        if (abs(GameScreen.player.x - x) + abs(GameScreen.player.y - y) > tolerance) {
            return false;
        }
        Player.isLockKey = false;
        GameScreen.player.posTransRoad = null;
        GameScreen.player.toX = GameScreen.player.x;
        GameScreen.player.toY = GameScreen.player.y;
        GameScreen.player.vx = 0;
        GameScreen.player.vy = 0;
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
            if (id >= 0 && T.mapName != null && id < T.mapName.length) {
                String name = T.mapName[id];
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
     * Exact-after-normalising first, then containment, because the stone's labels and `T.mapName` differ
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

    // ---- ENHANCE-04 RUNTIME CORE ----------------------------------------------
    //
    // Safe single-item enhancement runtime engine without multi-item queue.
    // Driven by sidecar requests (zeus-enhance.req) and reporting telemetry via
    // zeus-enhance-status.json.

    // State machine fields (public static for testability and runtime visibility)
    public static int enhState = 0; // 0 = IDLE
    public static String enhRequestId = "";
    public static int enhCapturedSlot = -1;
    public static int enhTemplateId = 0;
    public static int enhCategory = 3;
    public static String enhBaseName = "";
    public static int enhTier = 0;
    public static int enhExpectedLevel = 0;
    public static int enhStartLevel = 0;
    public static int enhCurrentLevel = 0;
    public static int enhTargetLevel = 0;
    public static int enhConfiguredCharmMode = 3;
    public static int enhResolvedCharmMode = 0;
    public static int enhPaymentType = 0;
    public static int enhAttemptCount = 0;
    public static int enhMaxAttempts = 1;
    public static String enhLastResult = null;
    public static int enhActiveTargetSlot = -1;
    public static boolean enhInFlightExecute = false;
    public static boolean enhValidationOnly = false;
    public static boolean enhValidationOnlyMalformed = false;

    public static long enhQuotedGoldCost = 0L;
    public static long enhQuotedGemCost = 0L;
    public static long[] enhQuotedMaterialRequirements = new long[4];
    public static long enhActualGoldSpent = 0L;
    public static long enhActualGemSpent = 0L;
    public static long[] enhActualMaterialsSpent = new long[4];
    public static long enhActualCharmsSpent = 0L;
    public static String enhAccountingStatus = "PENDING";
    public static String enhErrorCode = null;
    public static String enhErrorMessage = null;

    // Pre-attempt snapshot values for delta accounting
    public static long snapGoldBefore = 0L;
    public static long snapGemBefore = 0L;
    public static long[] snapMaterialsBefore = new long[4];
    public static long snapCharmBefore = 0L;
    public static int snapTargetLevelBefore = 0;
    public static int snapSelectedCharmTemplateId = 0;

    // Pre-execute evidence snapshot values for safe state reconciliation
    public static String snapRequestId = null;
    public static int snapTemplateId = 0;
    public static int snapCategory = 3;
    public static String snapBaseName = null;
    public static int snapTier = 0;
    public static int snapExpectedLevel = 0;
    public static int snapTargetLevel = 0;
    public static int snapPaymentType = 0;
    public static int snapResolvedCharmMode = 0;
    public static long snapRecipeGoldCost = 0L;
    public static long snapRecipeGemCost = 0L;
    public static long[] snapRecipeMaterials = new long[4];
    public static long enhExecuteStartedAt = 0L;
    public static long enhResultDeadline = 0L;
    public static int enhResultCode = -1;
    public static String enhSettlementSource = null;
    public static String enhSettlementProvenance = null;

    // Post-execute result wait window: 8.0s (200 ticks at 25 t/s)
    // Exceeds 3.7s native animation delay with 4.3s network/server RTT margin
    public static final long ENH_RESULT_WAIT_MS = 8000L;
    public static final int ENH_RESULT_WAIT_TICKS = 200;

    public static final int BLACKSMITH_MAP = 1;
    public static final int BLACKSMITH_ANCHOR_X = 324;
    public static final int BLACKSMITH_ANCHOR_Y = 624;
    public static final int MAX_BLACKSMITH_SCANS = 50;

    public static boolean enhNavigating = false;
    public static int enhBlacksmithScanTicks = 0;
    public static int enhForgeOpenTries = 0;
    public static boolean enhOwnsForgeScreen = false;
    public static boolean enhOwnsResultDialog = false;
    public static final class PendingForgeMenu {
        public final int npc;
        public final int menuId;
        public final int option;
        public final int itemCount;
        public final String optionLabel;
        public final long fingerprint;

        public PendingForgeMenu(int npc, int menuId, int option, int itemCount, String optionLabel, long fingerprint) {
            this.npc = npc;
            this.menuId = menuId;
            this.option = option;
            this.itemCount = itemCount;
            this.optionLabel = optionLabel;
            this.fingerprint = fingerprint;
        }
    }

    private static final Object ENH_MENU_LOCK = new Object();
    public static volatile PendingForgeMenu enhPendingMenuRecord = null;
    public static int enhPendingMenuNpc = Integer.MIN_VALUE;
    public static int enhPendingMenuId = -1;
    public static int enhPendingMenuOption = -1;
    public static long enhPendingMenuFingerprint = 0L;
    public static volatile long enhLastDispatchedMenuFingerprint = 0L;
    public static int enhPendingMenuWaitTicks = 0;
    public static final int MAX_PENDING_MENU_WAIT_TICKS = 20;

    public static int enhArmedBlacksmithNpcId = Integer.MIN_VALUE;
    public static long enhArmedBlacksmithGeneration = 0L;
    public static long enhBlacksmithInteractionGenerationSeq = 0L;
    public static long enhLastDispatchedIntroDialogGen = 0L;
    public static boolean enhIntroDialogHandled = false;
    public static long enhLastDispatchedIntroDialogFingerprint = 0L;
    public static int enhIntroDialogWaitTicks = 0;
    public static final int MAX_INTRO_DIALOG_WAIT_TICKS = 40;

    public static String getDialogNameShow(MainDialog dialog) {
        if (dialog instanceof MsgDialog) {
            try {
                java.lang.reflect.Field fr = MsgDialog.class.getDeclaredField("nameShow");
                fr.setAccessible(true);
                Object vr = fr.get(dialog);
                if (vr instanceof String && ((String) vr).trim().length() > 0) {
                    return (String) vr;
                }
            } catch (Throwable t) {
            }
        }
        return "";
    }

    public static String getDialogBody(MainDialog dialog) {
        if (dialog == null) return "";
        StringBuffer sb = new StringBuffer(64);
        if (dialog instanceof MsgDialog) {
            try {
                java.lang.reflect.Field fs = MsgDialog.class.getDeclaredField("status");
                fs.setAccessible(true);
                Object vs = fs.get(dialog);
                if (vs instanceof String && ((String) vs).length() > 0) {
                    sb.append((String) vs).append(' ');
                }
            } catch (Throwable t) {
            }
        }
        try {
            java.lang.reflect.Field fStr = MainDialog.class.getDeclaredField("strinfo");
            fStr.setAccessible(true);
            String[] lines = (String[]) fStr.get(dialog);
            if (lines != null) {
                for (int i = 0; i < lines.length; i++) {
                    if (lines[i] != null) {
                        sb.append(lines[i]).append(' ');
                    }
                }
            }
        } catch (Throwable t) {
        }
        return sb.toString().trim();
    }

    public static mVector getDialogButtons(MainDialog dialog) {
        if (dialog == null) return null;
        mVector res = new mVector("dialogButtons");
        try {
            if (dialog instanceof MsgDialog) {
                mVector cmdList = ((MsgDialog) dialog).cmdList;
                if (cmdList != null && cmdList.size() > 0) {
                    for (int i = 0; i < cmdList.size(); i++) {
                        Object b = cmdList.elementAt(i);
                        if (b != null) res.addElement(b);
                    }
                    return res;
                }
            }
            if (dialog.left != null) res.addElement(dialog.left);
            if (dialog.center != null) res.addElement(dialog.center);
            if (dialog.right != null) res.addElement(dialog.right);
        } catch (Throwable t) {
        }
        return res;
    }

    public static long computeDialogFingerprint(MainDialog dialog) {
        if (dialog == null) return 0L;
        StringBuffer sb = new StringBuffer(128);
        sb.append(dialog.getClass().getName()).append(';');
        String name = getDialogNameShow(dialog);
        sb.append(normSemantic(name)).append(';');
        String body = getDialogBody(dialog);
        sb.append(normSemantic(body)).append(';');
        mVector btns = getDialogButtons(dialog);
        int count = btns != null ? btns.size() : 0;
        sb.append(count).append(';');
        if (btns != null) {
            for (int i = 0; i < btns.size(); i++) {
                Object b = btns.elementAt(i);
                if (b instanceof iCommand && ((iCommand) b).caption != null) {
                    sb.append(normSemantic(((iCommand) b).caption)).append(',');
                }
            }
        }
        String s = sb.toString();
        long hash = 0xcbf29ce484222325L;
        for (int i = 0; i < s.length(); i++) {
            hash ^= (long) s.charAt(i);
            hash *= 0x100000001b3L;
        }
        return hash;
    }

    public static iCommand findBlacksmithIntroDialogButton(MainDialog dialog) {
        if (dialog == null) {
            return null;
        }
        try {
            mVector buttons = getDialogButtons(dialog);
            if (buttons != null) {
                for (int i = 0; i < buttons.size(); i++) {
                    Object entry = buttons.elementAt(i);
                    if (entry instanceof iCommand) {
                        iCommand btn = (iCommand) entry;
                        if (btn.caption != null) {
                            String s = normSemantic(btn.caption);
                            if ("cuong hoa".equals(s)) {
                                return btn;
                            }
                        }
                    }
                }
            }
        } catch (Throwable t) {
        }
        return null;
    }

    public static boolean isPhapSuDialogContext(MainDialog dialog) {
        if (dialog == null) {
            return false;
        }
        if (!(dialog instanceof MsgDialog)) {
            return false;
        }
        if (enhState != 6) {
            return false;
        }
        if (GameCanvas.currentDialog != dialog) {
            return false;
        }
        if (GameCanvas.subDialog != null) {
            return false;
        }
        if (enhArmedBlacksmithNpcId == Integer.MIN_VALUE || enhArmedBlacksmithGeneration <= 0L) {
            return false;
        }
        MainObject bs = findBlacksmithNpc();
        if (bs == null || bs.ID != enhArmedBlacksmithNpcId) {
            return false;
        }
        try {
            String fullText = dialogText(dialog);
            if (fullText == null || fullText.length() == 0) {
                return false;
            }
            String n = norm(fullText);

            if (n.indexOf("giao dich") >= 0) {
                return false;
            }
            if (n.indexOf("thong bao") >= 0 || n.indexOf("chuc mung") >= 0 || n.indexOf("bao tri") >= 0 || n.indexOf("he thong") >= 0) {
                return false;
            }
            if (n.indexOf("xac nhan") >= 0 || n.indexOf("co muon") >= 0 || n.indexOf("ban co muon") >= 0 || n.indexOf("dong y") >= 0 || n.indexOf("nga tu") >= 0) {
                return false;
            }

            String nameShow = getDialogNameShow(dialog);
            String normName = normSemantic(nameShow);
            if ("phap su".equals(normName)) {
                return true;
            }
            try {
                java.lang.reflect.Field fStr = MainDialog.class.getDeclaredField("strinfo");
                fStr.setAccessible(true);
                String[] lines = (String[]) fStr.get(dialog);
                if (lines != null && lines.length > 0 && lines[0] != null) {
                    String firstLine = normSemantic(lines[0]);
                    if (firstLine.startsWith("phap su:") || firstLine.startsWith("phap su :") || firstLine.equals("phap su")) {
                        return true;
                    }
                }
            } catch (Throwable ignored) {
            }
        } catch (Throwable t) {
        }
        return false;
    }

    public static boolean isEnhancementIntroDialog(MainDialog dialog) {
        return isPhapSuDialogContext(dialog) && findBlacksmithIntroDialogButton(dialog) != null;
    }

    public static void clearPendingForgeMenu() {
        synchronized (ENH_MENU_LOCK) {
            enhPendingMenuRecord = null;
            enhPendingMenuNpc = Integer.MIN_VALUE;
            enhPendingMenuId = -1;
            enhPendingMenuOption = -1;
            enhPendingMenuFingerprint = 0L;
            enhLastDispatchedMenuFingerprint = 0L;
            enhPendingMenuWaitTicks = 0;
        }
    }

    public static void setPendingForgeMenu(PendingForgeMenu rec) {
        synchronized (ENH_MENU_LOCK) {
            enhPendingMenuRecord = rec;
            if (rec != null) {
                enhPendingMenuNpc = rec.npc;
                enhPendingMenuId = rec.menuId;
                enhPendingMenuOption = rec.option;
                enhPendingMenuFingerprint = rec.fingerprint;
            } else {
                enhPendingMenuNpc = Integer.MIN_VALUE;
                enhPendingMenuId = -1;
                enhPendingMenuOption = -1;
                enhPendingMenuFingerprint = 0L;
            }
            enhPendingMenuWaitTicks = 0;
        }
    }

    public static PendingForgeMenu getPendingForgeMenu() {
        synchronized (ENH_MENU_LOCK) {
            return enhPendingMenuRecord;
        }
    }

    public static void cleanEnhancementRouting() {
        enhNavigating = false;
        enhBlacksmithScanTicks = 0;
        enhForgeOpenTries = 0;
        clearPendingForgeMenu();
        enhIntroDialogHandled = false;
        enhLastDispatchedIntroDialogFingerprint = 0L;
        enhLastDispatchedIntroDialogGen = 0L;
        enhArmedBlacksmithNpcId = Integer.MIN_VALUE;
        enhArmedBlacksmithGeneration = 0L;
        enhIntroDialogWaitTicks = 0;
        try {
            if (enhOwnsResultDialog && enhOwnsForgeScreen && isForgeScreenOpen() && GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                GameCanvas.menu2.doCloseMenu();
            }
            if (enhOwnsForgeScreen && isForgeScreenOpen()) {
                GameCanvas.game.Show();
            }
        } catch (Throwable ignored) {
        }
        enhOwnsResultDialog = false;
        enhOwnsForgeScreen = false;
    }

    public static boolean isEnhancementTravelConflict() {
        if (autoFarmActive() || atkMode != 0) {
            return true;
        }
        if (navTarget >= 0 && !navDone) {
            return true;
        }
        return false;
    }

    private static final java.util.Vector processedRequestIds = new java.util.Vector();
    private static String lastEnhRequestId = null;
    private static long enhReqCheckedAt = 0L;
    private static int enhWait = 0;
    private static int enhTerminalCleanupTicks = 0;

    public static void resetEnhancementDeduplication() {
        processedRequestIds.removeAllElements();
        lastEnhRequestId = null;
        cleanEnhancementRouting();
    }

    public static int parseEnhancementState(String name) {
        if (name == null) return -1;
        for (int i = 0; i <= 39; i++) {
            if (name.equals(getEnhancementStateName(i))) {
                return i;
            }
        }
        return -1;
    }

    public static String getEnhancementStateName(int state) {
        switch (state) {
            case 0: return "IDLE";
            case 1: return "VALIDATING_REQUEST";
            case 2: return "WAITING_GAME_READY";
            case 3: return "VALIDATING_TARGET";
            case 4: return "LOCATING_BLACKSMITH";
            case 5: return "APPROACHING_BLACKSMITH";
            case 6: return "OPENING_FORGE";
            case 7: return "INSERTING_TARGET";
            case 8: return "RESOLVING_CHARM";
            case 9: return "INSERTING_CHARM";
            case 10: return "VERIFYING_RESOURCES";
            case 11: return "READY_FOR_ATTEMPT";
            case 12: return "ATTEMPTING";
            case 13: return "WAITING_RESULT";
            case 14: return "WAITING_SETTLEMENT";
            case 15: return "FAILURE_PROTECTED";
            case 16: return "FAILURE_DEGRADED";
            case 17: return "TARGET_REACHED";
            case 18: return "ATTEMPT_LIMIT_REACHED";
            case 19: return "ITEM_DESTROYED";
            case 20: return "ITEM_MISSING_OR_CHANGED";
            case 21: return "AMBIGUOUS_WIRE_TARGET";
            case 22: return "AMBIGUOUS_CHARM";
            case 23: return "INELIGIBLE_ITEM";
            case 24: return "CHARM_MISSING";
            case 25: return "INSUFFICIENT_GOLD";
            case 26: return "INSUFFICIENT_GEMS";
            case 27: return "INSUFFICIENT_MATERIALS";
            case 28: return "SERVER_REJECTED";
            case 29: return "RESULT_AMBIGUOUS";
            case 30: return "ACCOUNTING_UNSETTLED";
            case 31: return "TIMEOUT";
            case 32: return "CANCELLED";
            case 33: return "MANUAL_REVIEW_REQUIRED";
            case 34: return "ENHANCEMENT_TRAVEL_CONFLICT";
            case 35: return "BLACKSMITH_ROUTE_UNAVAILABLE";
            case 36: return "BLACKSMITH_NOT_FOUND";
            case 37: return "BLACKSMITH_INTERACTION_FAILED";
            case 38: return "FORGE_OPEN_FAILED";
            case 39: return "DRY_RUN_COMPLETE";
            default: return "UNKNOWN";
        }
    }

    public static boolean isEnhancementStateTerminal(int state) {
        return state >= 17 && state <= 39;
    }

    public static int resolveAutoCharm(int currentLevel) {
        if (currentLevel <= 5) return 0;
        if (currentLevel <= 10) return 1;
        if (currentLevel <= 14) return 2;
        return 0;
    }

    public static void validateEnhancementTarget() {
        if (Item.VecInvetoryPlayer == null) {
            enhState = 20; // ITEM_MISSING_OR_CHANGED
            enhErrorMessage = "Bag is empty or null";
            return;
        }
        int matchCount = 0;
        MainItem matchedItem = null;
        int matchedSlot = -1;
        int bagCount = Item.VecInvetoryPlayer.size();
        for (int i = 0; i < bagCount; i++) {
            Object obj = Item.VecInvetoryPlayer.elementAt(i);
            if (!(obj instanceof MainItem)) {
                continue;
            }
            MainItem item = (MainItem) obj;
            if (item.Id == enhTemplateId && item.ItemCatagory == enhCategory) {
                matchCount++;
                if (matchedItem == null) {
                    matchedItem = item;
                    matchedSlot = i;
                }
            }
        }
        if (matchCount == 0) {
            enhState = 20; // ITEM_MISSING_OR_CHANGED
            enhErrorMessage = "Target item not found in bag";
            enhActiveTargetSlot = -1;
            return;
        }
        if (matchCount > 1) {
            enhState = 21; // AMBIGUOUS_WIRE_TARGET
            enhErrorMessage = "Ambiguous wire target: multiple items match template " + enhTemplateId;
            enhActiveTargetSlot = -1;
            return;
        }
        // Exactly 1 match: validate fingerprint and level
        if (enhBaseName != null && enhBaseName.trim().length() > 0 && matchedItem.itemNameExcludeLv != null) {
            if (!enhBaseName.equals(matchedItem.itemNameExcludeLv)) {
                enhState = 20; // ITEM_MISSING_OR_CHANGED
                enhErrorMessage = "Item base name changed: expected " + enhBaseName + ", got " + matchedItem.itemNameExcludeLv;
                enhActiveTargetSlot = -1;
                return;
            }
        }
        if (matchedItem.colorNameItem != enhTier) {
            enhState = 20; // ITEM_MISSING_OR_CHANGED
            enhErrorMessage = "Item tier changed: expected " + enhTier + ", got " + matchedItem.colorNameItem;
            enhActiveTargetSlot = -1;
            return;
        }
        if (matchedItem.tier != enhExpectedLevel) {
            enhState = 20; // ITEM_MISSING_OR_CHANGED
            enhErrorMessage = "Item level changed: expected " + enhExpectedLevel + ", got " + matchedItem.tier;
            enhActiveTargetSlot = -1;
            return;
        }
        if (enhCapturedSlot >= 0 && matchedSlot != enhCapturedSlot) {
            enhState = 20; // ITEM_MISSING_OR_CHANGED
            enhErrorMessage = "Target item slot mismatch after dispatch: expected slot " + enhCapturedSlot + ", found at " + matchedSlot;
            enhActiveTargetSlot = -1;
            return;
        }
        enhCurrentLevel = matchedItem.tier;
        enhStartLevel = matchedItem.tier;
        enhActiveTargetSlot = matchedSlot;
        enhState = 4; // LOCATING_BLACKSMITH
    }

    public static void resolveEnhancementCharm() {
        int desiredMode = enhConfiguredCharmMode;
        if (desiredMode == 3) {
            desiredMode = resolveAutoCharm(enhCurrentLevel);
        }
        if (desiredMode == 0) {
            enhResolvedCharmMode = 0;
            snapSelectedCharmTemplateId = 0;
            enhState = 10; // VERIFYING_RESOURCES
            return;
        }
        if (Item.VecInvetoryPlayer == null) {
            enhState = 24; // CHARM_MISSING
            enhErrorMessage = "Bag is empty; charm missing";
            return;
        }
        int bagCount = Item.VecInvetoryPlayer.size();
        java.util.Vector charmTemplates = new java.util.Vector();
        MainItem chosenCharm = null;
        for (int i = 0; i < bagCount; i++) {
            Object obj = Item.VecInvetoryPlayer.elementAt(i);
            if (!(obj instanceof MainItem)) {
                continue;
            }
            MainItem item = (MainItem) obj;
            if (item.ItemCatagory == 7 && item.typeMaterial == 11) {
                boolean matchesMode = isSemanticCharm(item.itemName, item.itemNameExcludeLv, desiredMode);
                if (matchesMode) {
                    Integer idObj = new Integer(item.Id);
                    if (!charmTemplates.contains(idObj)) {
                        charmTemplates.addElement(idObj);
                    }
                    if (chosenCharm == null) {
                        chosenCharm = item;
                    }
                }
            }
        }
        if (charmTemplates.isEmpty()) {
            enhState = 24; // CHARM_MISSING
            enhErrorMessage = "Required charm mode " + desiredMode + " not found in bag";
            return;
        }
        if (charmTemplates.size() > 1) {
            enhState = 22; // AMBIGUOUS_CHARM
            enhErrorMessage = "Multiple distinct charm templates match mode " + desiredMode;
            return;
        }
        enhResolvedCharmMode = desiredMode;
        snapSelectedCharmTemplateId = chosenCharm.Id;
        enhState = 9; // INSERTING_CHARM
    }

    public static void verifyEnhancementResources() {
        int targetLv = (TabRebuildItem.itemRe != null) ? TabRebuildItem.itemRe.tier : enhCurrentLevel;
        long quotedGold = 0L;
        long quotedGems = 0L;
        byte[] reqMats = null;
        if (TabRebuildItem.dataRebuild != null && targetLv >= 0 && targetLv < TabRebuildItem.dataRebuild.length && TabRebuildItem.dataRebuild[targetLv] != null) {
            quotedGold = TabRebuildItem.dataRebuild[targetLv].priceCoin;
            quotedGems = TabRebuildItem.dataRebuild[targetLv].priceGold;
            reqMats = TabRebuildItem.dataRebuild[targetLv].mValue;
        }
        enhQuotedGoldCost = quotedGold;
        enhQuotedGemCost = quotedGems;
        if (enhQuotedMaterialRequirements == null) {
            enhQuotedMaterialRequirements = new long[4];
        }
        if (reqMats != null) {
            for (int i = 0; i < reqMats.length && i < enhQuotedMaterialRequirements.length; i++) {
                enhQuotedMaterialRequirements[i] = reqMats[i];
            }
        }
        if (GameScreen.player == null) {
            enhState = 25;
            enhErrorMessage = "Player hero is null";
            return;
        }
        if (enhPaymentType == 0) { // Gold mode
            if (GameScreen.player.coin < quotedGold) {
                enhState = 25; // INSUFFICIENT_GOLD
                enhErrorMessage = "Insufficient gold: have " + GameScreen.player.coin + ", need " + quotedGold;
                return;
            }
        } else if (enhPaymentType == 1) { // Gem mode
            if (GameScreen.player.gold < quotedGems) {
                enhState = 26; // INSUFFICIENT_GEMS
                enhErrorMessage = "Insufficient gems: have " + GameScreen.player.gold + ", need " + quotedGems;
                return;
            }
        }
        if (reqMats != null) {
            for (int i = 0; i < reqMats.length; i++) {
                int required = reqMats[i] & 0xFF;
                if (required > 0) {
                    int available = (TabRebuildItem.numMaterialInven != null && i < TabRebuildItem.numMaterialInven.length) ? TabRebuildItem.numMaterialInven[i] : 0;
                    if (available < required) {
                        enhState = 27; // INSUFFICIENT_MATERIALS
                        enhErrorMessage = "Missing required material index " + i + ": have " + available + ", need " + required;
                        return;
                    }
                }
            }
        }
        enhState = 11; // READY_FOR_ATTEMPT
    }

    public static void executeEnhancementAttempt() {
        if (enhAttemptCount >= enhMaxAttempts) {
            enhState = 18; // ATTEMPT_LIMIT_REACHED
            enhErrorMessage = "Attempt limit " + enhMaxAttempts + " reached";
            return;
        }
        if (GameScreen.player != null) {
            snapGoldBefore = GameScreen.player.coin;
            snapGemBefore = GameScreen.player.gold;
        }
        snapTargetLevelBefore = (TabRebuildItem.itemRe != null) ? TabRebuildItem.itemRe.tier : enhCurrentLevel;
        if (snapSelectedCharmTemplateId > 0 && Item.VecInvetoryPlayer != null) {
            snapCharmBefore = countItemInBag(snapSelectedCharmTemplateId);
        } else {
            snapCharmBefore = 0L;
        }
        if (TabRebuildItem.numMaterialInven != null) {
            if (snapMaterialsBefore == null) snapMaterialsBefore = new long[4];
            for (int i = 0; i < TabRebuildItem.numMaterialInven.length && i < snapMaterialsBefore.length; i++) {
                snapMaterialsBefore[i] = TabRebuildItem.numMaterialInven[i];
            }
        }

        if (enhValidationOnly) {
            enhState = 39; // DRY_RUN_COMPLETE
            enhErrorCode = null;
            enhErrorMessage = null;
            cleanEnhancementRouting();
            return;
        }

        // Preserve pre-execute evidence snapshot immediately before Opcode 67
        snapRequestId = enhRequestId;
        snapTemplateId = enhTemplateId;
        snapCategory = enhCategory;
        snapBaseName = enhBaseName;
        snapTier = enhTier;
        snapExpectedLevel = enhExpectedLevel;
        snapTargetLevel = enhTargetLevel;
        snapPaymentType = enhPaymentType;
        snapResolvedCharmMode = enhResolvedCharmMode;
        snapRecipeGoldCost = enhQuotedGoldCost;
        snapRecipeGemCost = enhQuotedGemCost;
        if (snapRecipeMaterials == null) snapRecipeMaterials = new long[4];
        if (enhQuotedMaterialRequirements != null) {
            for (int i = 0; i < enhQuotedMaterialRequirements.length && i < snapRecipeMaterials.length; i++) {
                snapRecipeMaterials[i] = enhQuotedMaterialRequirements[i];
            }
        }
        enhResultCode = -1;
        enhSettlementSource = null;
        enhSettlementProvenance = null;

        enhAttemptCount++;
        enhState = 12; // ATTEMPTING
        enhInFlightExecute = true;

        try {
            GlobalService.gI().Rebuild_Item((byte) 2, (short) 0, (byte) enhPaymentType);
            enhState = 13; // WAITING_RESULT
            long now = System.currentTimeMillis();
            enhExecuteStartedAt = now;
            enhResultDeadline = now + ENH_RESULT_WAIT_MS;
            enhWait = ENH_RESULT_WAIT_TICKS;
        } catch (Throwable t) {
            enhState = 29; // RESULT_AMBIGUOUS
            enhErrorMessage = "Failed to send Opcode 67 sub-action 2: " + t;
        }
    }

    public static void settleEnhancementResult() {
        if (GameScreen.player != null) {
            long goldDelta = Math.max(0L, snapGoldBefore - GameScreen.player.coin);
            enhActualGoldSpent += goldDelta;
            snapGoldBefore = GameScreen.player.coin;

            long gemDelta = Math.max(0L, snapGemBefore - GameScreen.player.gold);
            enhActualGemSpent += gemDelta;
            snapGemBefore = GameScreen.player.gold;
        }
        if (snapSelectedCharmTemplateId > 0 && Item.VecInvetoryPlayer != null) {
            long charmsLeft = countItemInBag(snapSelectedCharmTemplateId);
            long charmDelta = Math.max(0L, snapCharmBefore - charmsLeft);
            enhActualCharmsSpent += charmDelta;
            snapCharmBefore = charmsLeft;
        }
        if (TabRebuildItem.numMaterialInven != null && snapMaterialsBefore != null) {
            if (enhActualMaterialsSpent == null) enhActualMaterialsSpent = new long[4];
            for (int i = 0; i < TabRebuildItem.numMaterialInven.length && i < snapMaterialsBefore.length; i++) {
                long matDelta = Math.max(0L, snapMaterialsBefore[i] - TabRebuildItem.numMaterialInven[i]);
                enhActualMaterialsSpent[i] += matDelta;
                snapMaterialsBefore[i] = TabRebuildItem.numMaterialInven[i];
            }
        }
        enhAccountingStatus = "SETTLED";
        enhInFlightExecute = false;

        MainItem currentTarget = null;
        if (Item.VecInvetoryPlayer != null) {
            int bagCount = Item.VecInvetoryPlayer.size();
            for (int i = 0; i < bagCount; i++) {
                Object obj = Item.VecInvetoryPlayer.elementAt(i);
                if (obj instanceof MainItem) {
                    MainItem item = (MainItem) obj;
                    if (item.Id == enhTemplateId && item.ItemCatagory == enhCategory) {
                        currentTarget = item;
                        enhActiveTargetSlot = i;
                        break;
                    }
                }
            }
        }

        if (currentTarget == null) {
            enhState = 19; // ITEM_DESTROYED
            enhLastResult = "DESTROYED";
            enhActiveTargetSlot = -1;
            return;
        }

        int newLevel = currentTarget.tier;
        enhCurrentLevel = newLevel;

        if ("STATE_RECONCILED_SUCCESS".equals(enhSettlementProvenance)) {
            enhLastResult = "STATE_RECONCILED_SUCCESS";
            if (newLevel >= enhTargetLevel) {
                enhState = 17; // TARGET_REACHED
            } else if (enhAttemptCount >= enhMaxAttempts) {
                enhState = 18; // ATTEMPT_LIMIT_REACHED
            } else {
                enhState = 10;
            }
            return;
        }

        if (newLevel >= enhTargetLevel) {
            enhLastResult = "SUCCESS";
            enhState = 17; // TARGET_REACHED
            return;
        }

        if (TabRebuildItem.isNextRebuild == 3) {
            enhLastResult = "SUCCESS";
            if (enhAttemptCount >= enhMaxAttempts) {
                enhState = 18; // ATTEMPT_LIMIT_REACHED
            } else {
                enhState = 10; // Ready for next cycle
            }
            return;
        }

        if (TabRebuildItem.isNextRebuild == 4) {
            if (newLevel == snapTargetLevelBefore) {
                enhLastResult = "FAILURE_PROTECTED";
                enhSettlementProvenance = "FAILURE_PROTECTED";
                enhState = 15; // FAILURE_PROTECTED
            } else if (newLevel < snapTargetLevelBefore) {
                enhLastResult = "FAILURE_DEGRADED";
                enhSettlementProvenance = "FAILURE_DEGRADED";
                enhState = 16; // FAILURE_DEGRADED
            } else {
                enhLastResult = "FAILURE_PROTECTED";
                enhSettlementProvenance = "FAILURE_PROTECTED";
                enhState = 15;
            }
            if (enhAttemptCount >= enhMaxAttempts) {
                enhState = 18; // ATTEMPT_LIMIT_REACHED
            }
            return;
        }

        if (enhAttemptCount >= enhMaxAttempts) {
            enhState = 18;
        }
    }

    public static boolean canReconcileStateSuccess() {
        if (!enhInFlightExecute) {
            return false;
        }
        if (TabRebuildItem.isNextRebuild == 3 || TabRebuildItem.isNextRebuild == 4 || TabRebuildItem.isNextRebuild == 1 || TabRebuildItem.isNextRebuild == 2 || TabRebuildItem.isNextRebuild >= 5) {
            return false;
        }
        if (enhRequestId == null || enhRequestId.length() == 0 || !enhRequestId.equals(snapRequestId)) {
            return false;
        }
        if (Item.VecInvetoryPlayer == null) {
            return false;
        }
        int bagCount = Item.VecInvetoryPlayer.size();
        int matchCount = 0;
        MainItem matchedItem = null;
        for (int i = 0; i < bagCount; i++) {
            Object obj = Item.VecInvetoryPlayer.elementAt(i);
            if (obj instanceof MainItem) {
                MainItem it = (MainItem) obj;
                if (it.Id == snapTemplateId && it.ItemCatagory == snapCategory) {
                    matchCount++;
                    matchedItem = it;
                }
            }
        }
        if (matchCount != 1 || matchedItem == null) {
            return false;
        }
        if (matchedItem.colorNameItem != snapTier) {
            return false;
        }
        if (snapBaseName != null && snapBaseName.trim().length() > 0 && matchedItem.itemNameExcludeLv != null) {
            if (!snapBaseName.equals(matchedItem.itemNameExcludeLv)) {
                return false;
            }
        }
        if (matchedItem.tier != snapTargetLevel) {
            return false;
        }
        if (snapTargetLevelBefore != snapExpectedLevel) {
            return false;
        }
        if (matchedItem.tier != snapExpectedLevel + 1) {
            return false;
        }
        if (GameScreen.player == null) {
            return false;
        }
        long liveGoldDelta = Math.max(0L, snapGoldBefore - GameScreen.player.coin);
        long liveGemDelta = Math.max(0L, snapGemBefore - GameScreen.player.gold);

        if (snapPaymentType == 0) {
            if (liveGoldDelta != snapRecipeGoldCost) {
                return false;
            }
            if (liveGemDelta != 0L) {
                return false;
            }
        } else if (snapPaymentType == 1) {
            if (liveGemDelta != snapRecipeGemCost) {
                return false;
            }
            if (liveGoldDelta != 0L) {
                return false;
            }
        } else {
            return false;
        }

        if (snapRecipeMaterials != null && TabRebuildItem.numMaterialInven != null && snapMaterialsBefore != null) {
            for (int i = 0; i < snapRecipeMaterials.length && i < 4; i++) {
                long required = snapRecipeMaterials[i];
                long actualAvailable = (i < TabRebuildItem.numMaterialInven.length) ? TabRebuildItem.numMaterialInven[i] : 0L;
                long actualBefore = (i < snapMaterialsBefore.length) ? snapMaterialsBefore[i] : 0L;
                long actualMatDelta = Math.max(0L, actualBefore - actualAvailable);
                if (actualMatDelta != required) {
                    return false;
                }
            }
        }

        if (snapResolvedCharmMode == 0) {
            if (snapSelectedCharmTemplateId > 0) {
                long currentCharmCount = countItemInBag(snapSelectedCharmTemplateId);
                long charmDelta = Math.max(0L, snapCharmBefore - currentCharmCount);
                if (charmDelta != 0L) {
                    return false;
                }
            }
        } else {
            if (snapSelectedCharmTemplateId <= 0) {
                return false;
            }
            long currentCharmCount = countItemInBag(snapSelectedCharmTemplateId);
            long charmDelta = Math.max(0L, snapCharmBefore - currentCharmCount);
            if (charmDelta != 1L) {
                return false;
            }
        }

        return true;
    }

    public static void recoverEnhancementSession() {
        if (enhInFlightExecute) {
            enhState = 33; // MANUAL_REVIEW_REQUIRED
            enhInFlightExecute = false;
            enhOwnsResultDialog = false;
            enhOwnsForgeScreen = false;
            enhErrorMessage = "Interrupted during in-flight enhancement attempt; manual review required";
            cleanEnhancementRouting();
            publishEnhancementStatus();
        }
    }

    public static boolean isEnhancementHighlightActive() {
        return enhState > 0 && enhState < 17 && enhActiveTargetSlot >= 0;
    }

    private static void paintEnhancementHighlight(mGraphics canvas) {
        if (canvas == null || !isEnhancementHighlightActive()) return;
        try {
            canvas.setColor(0xFFCC00); // Amber highlight
        } catch (Throwable ignored) {
        }
    }

    private static long countItemInBag(int templateId) {
        if (Item.VecInvetoryPlayer == null) return 0L;
        long total = 0L;
        for (int i = 0; i < Item.VecInvetoryPlayer.size(); i++) {
            Object obj = Item.VecInvetoryPlayer.elementAt(i);
            if (obj instanceof MainItem) {
                MainItem it = (MainItem) obj;
                if (it.Id == templateId) {
                    total += it.numPotion > 0 ? it.numPotion : 1;
                }
            }
        }
        return total;
    }

    private static MainItem findBagItem(int templateId, int category) {
        if (Item.VecInvetoryPlayer == null) return null;
        for (int i = 0; i < Item.VecInvetoryPlayer.size(); i++) {
            Object obj = Item.VecInvetoryPlayer.elementAt(i);
            if (obj instanceof MainItem) {
                MainItem it = (MainItem) obj;
                if (it.Id == templateId && it.ItemCatagory == category) {
                    return it;
                }
            }
        }
        return null;
    }

    private static MainObject findBlacksmithNpc() {
        if (GameScreen.Vecplayers == null || GameScreen.player == null) {
            return null;
        }
        MainObject best = null;
        int bestDistance = Integer.MAX_VALUE;
        for (int i = 0; i < GameScreen.Vecplayers.size(); i++) {
            Object entry = GameScreen.Vecplayers.elementAt(i);
            if (!(entry instanceof MainObject)) {
                continue;
            }
            MainObject candidate = (MainObject) entry;
            if (candidate.typeObject != 2) {
                continue;
            }
            if (candidate.name != null) {
                String n = norm(candidate.name);
                if (n.indexOf("phap su") >= 0) {
                    int distance = abs(GameScreen.player.x - candidate.x) + abs(GameScreen.player.y - candidate.y);
                    if (candidate.ID == -36) {
                        distance -= 10000;
                    }
                    if (distance < bestDistance) {
                        bestDistance = distance;
                        best = candidate;
                    }
                }
            }
        }
        return best;
    }

    private static boolean isForgeScreenOpen() {
        if (GameCanvas.currentScreen instanceof TabScreenNew) {
            TabScreenNew pop = (TabScreenNew) GameCanvas.currentScreen;
            if (pop.VecTabScreen != null) {
                for (int i = 0; i < pop.VecTabScreen.size(); i++) {
                    Object tab = pop.VecTabScreen.elementAt(i);
                    if (tab instanceof TabRebuildItem) {
                        return pop.selectTab == i;
                    }
                }
            }
        }
        return false;
    }

    private static void checkRestartRecovery(long now) {
        if (enhStatusPath == null) return;
        java.io.File sFile = new java.io.File(enhStatusPath);
        if (!sFile.exists()) return;
        String sText = readSmallFile(enhStatusPath);
        if (sText == null) return;
        String pReqId = parseJsonString(sText, "request_id");
        if (pReqId == null || pReqId.length() == 0) return;
        String pStateStr = parseJsonString(sText, "state");
        int pState = parseEnhancementState(pStateStr);
        if (pState < 0) return;

        if (!processedRequestIds.contains(pReqId)) {
            processedRequestIds.addElement(pReqId);
        }
        lastEnhRequestId = pReqId;

        if (isEnhancementStateTerminal(pState)) {
            enhRequestId = pReqId;
            enhState = pState;
            enhCapturedSlot = parseJsonInt(sText, "captured_slot", enhCapturedSlot);
            enhTemplateId = parseJsonInt(sText, "template_id", enhTemplateId);
            enhCategory = parseJsonInt(sText, "category", enhCategory);
            enhBaseName = parseJsonString(sText, "base_name");
            enhStartLevel = parseJsonInt(sText, "start_level", enhStartLevel);
            enhCurrentLevel = parseJsonInt(sText, "current_level", enhCurrentLevel);
            enhTargetLevel = parseJsonInt(sText, "target_level", enhTargetLevel);
            enhAttemptCount = parseJsonInt(sText, "attempt_count", enhAttemptCount);
            enhActualGoldSpent = parseJsonLong(sText, "actual_gold_spent", enhActualGoldSpent);
            enhActualGemSpent = parseJsonLong(sText, "actual_gem_spent", enhActualGemSpent);
            enhActualCharmsSpent = parseJsonLong(sText, "actual_charms_spent", enhActualCharmsSpent);
            enhAccountingStatus = parseJsonString(sText, "accounting_status");
            enhResultCode = parseJsonInt(sText, "result_code", enhResultCode);
            enhSettlementSource = parseJsonString(sText, "settlement_source");
            enhSettlementProvenance = parseJsonString(sText, "settlement_provenance");
            enhLastResult = parseJsonString(sText, "last_result");
            enhErrorCode = parseJsonString(sText, "error_code");
            enhErrorMessage = parseJsonString(sText, "error_message");
            return;
        }

        if (pState == 12 || pState == 13 || pState == 14) {
            enhRequestId = pReqId;
            enhState = 33; // MANUAL_REVIEW_REQUIRED
            enhErrorCode = "MANUAL_REVIEW_REQUIRED";
            enhErrorMessage = "Interrupted during in-flight enhancement attempt; manual review required";
            cleanEnhancementRouting();
            publishEnhancementStatus();
            return;
        }
    }

    private static void enhSidecarTick(long now) {
        if (enhReqPath == null) {
            return;
        }
        if (now - enhReqCheckedAt < 200L) {
            return;
        }
        enhReqCheckedAt = now;

        // Check restart recovery from persisted status if uninitialized
        if (lastEnhRequestId == null) {
            checkRestartRecovery(now);
        }

        // Check cancellation
        if (enhCancelPath != null) {
            java.io.File cancelFile = new java.io.File(enhCancelPath);
            if (cancelFile.exists()) {
                try {
                    cancelFile.delete();
                } catch (Throwable ignored) {
                }
                if (enhState > 0 && !isEnhancementStateTerminal(enhState)) {
                    if (enhInFlightExecute) {
                        // Let attempt finish or enter manual review
                    } else {
                        enhState = 32; // CANCELLED
                        enhErrorMessage = "Enhancement cancelled via sidecar";
                        cleanEnhancementRouting();
                        publishEnhancementStatus();
                        return;
                    }
                }
            }
        }

        // Check request file
        java.io.File reqFile = new java.io.File(enhReqPath);
        if (!reqFile.exists()) {
            if (isEnhancementStateTerminal(enhState) && enhOwnsForgeScreen) {
                if (!enhOwnsResultDialog) {
                    cleanEnhancementRouting();
                } else {
                    enhTerminalCleanupTicks++;
                    if (enhTerminalCleanupTicks >= 5) {
                        cleanEnhancementRouting();
                    }
                }
            }
            return;
        }

        String reqText = readSmallFile(enhReqPath);
        if (reqText == null) {
            return;
        }

        String reqId = parseJsonString(reqText, "request_id");
        if (reqId == null || reqId.length() == 0) {
            return;
        }

        // EXACTLY-ONCE CONTRACT:
        // Once a request UUID has been accepted, the same UUID must NEVER initialize
        // a new enhancement attempt in the same JVM lifecycle.
        // Terminal versus non-terminal state must not weaken this rule.
        // Repeated reads of the same request file must become no-ops.
        if (processedRequestIds.contains(reqId) || reqId.equals(lastEnhRequestId)) {
            if (isEnhancementStateTerminal(enhState)) {
                if (enhOwnsForgeScreen) {
                    if (!enhOwnsResultDialog) {
                        cleanEnhancementRouting();
                    } else {
                        enhTerminalCleanupTicks++;
                        if (enhTerminalCleanupTicks >= 5) {
                            cleanEnhancementRouting();
                        }
                    }
                }
            }
            return;
        }

        // A new request UUID cannot interrupt an in-flight non-terminal attempt
        if (enhState != 0 && !isEnhancementStateTerminal(enhState)) {
            return;
        }

        processedRequestIds.addElement(reqId);
        lastEnhRequestId = reqId;
        enhRequestId = reqId;
        enhTerminalCleanupTicks = 0;
        enhCapturedSlot = parseJsonInt(reqText, "captured_slot", -1);
        enhTemplateId = parseJsonInt(reqText, "template_id", 0);
        enhCategory = parseJsonInt(reqText, "category", 3);
        enhBaseName = parseJsonString(reqText, "base_name");
        enhTier = parseJsonInt(reqText, "tier", 0);
        enhExpectedLevel = parseJsonInt(reqText, "expected_level", 0);
        enhTargetLevel = parseJsonInt(reqText, "target_level", 0);
        enhConfiguredCharmMode = parseJsonInt(reqText, "charm_mode", 3);
        enhPaymentType = parseJsonInt(reqText, "payment_type", 0);
        enhMaxAttempts = parseJsonInt(reqText, "max_attempts", 1);
        enhValidationOnlyMalformed = false;
        enhValidationOnly = parseJsonBooleanStrict(reqText, "validation_only", false);

        enhAttemptCount = 0;
        enhActualGoldSpent = 0L;
        enhActualGemSpent = 0L;
        enhActualCharmsSpent = 0L;
        enhActualMaterialsSpent = new long[4];
        enhAccountingStatus = "PENDING";
        enhLastResult = null;
        enhErrorCode = null;
        enhErrorMessage = null;
        enhInFlightExecute = false;
        enhActiveTargetSlot = -1;
        enhArmedBlacksmithNpcId = Integer.MIN_VALUE;
        enhArmedBlacksmithGeneration = 0L;
        enhLastDispatchedIntroDialogGen = 0L;
        enhLastDispatchedIntroDialogFingerprint = 0L;
        enhIntroDialogWaitTicks = 0;
        enhIntroDialogHandled = false;

        enhState = 1; // VALIDATING_REQUEST
        publishEnhancementStatus();
    }

    private static String parseJsonString(String text, String key) {
        if (text == null || key == null) return null;
        String pattern = "\"" + key + "\"";
        int idx = text.indexOf(pattern);
        if (idx < 0) return null;
        int colon = text.indexOf(':', idx + pattern.length());
        if (colon < 0) return null;
        int startQuote = text.indexOf('"', colon + 1);
        if (startQuote < 0) return null;
        int endQuote = text.indexOf('"', startQuote + 1);
        if (endQuote < 0) return null;
        return text.substring(startQuote + 1, endQuote).trim();
    }

    private static int parseJsonInt(String text, String key, int def) {
        if (text == null || key == null) return def;
        String pattern = "\"" + key + "\"";
        int idx = text.indexOf(pattern);
        if (idx < 0) return def;
        int colon = text.indexOf(':', idx + pattern.length());
        if (colon < 0) return def;
        int start = colon + 1;
        while (start < text.length() && Character.isWhitespace(text.charAt(start))) {
            start++;
        }
        int end = start;
        while (end < text.length() && (Character.isDigit(text.charAt(end)) || text.charAt(end) == '-')) {
            end++;
        }
        if (end > start) {
            try {
                return Integer.parseInt(text.substring(start, end));
            } catch (Throwable ignored) {
            }
        }
        return def;
    }

    private static long parseJsonLong(String text, String key, long def) {
        if (text == null || key == null) return def;
        String pattern = "\"" + key + "\"";
        int idx = text.indexOf(pattern);
        if (idx < 0) return def;
        int colon = text.indexOf(':', idx + pattern.length());
        if (colon < 0) return def;
        int start = colon + 1;
        while (start < text.length() && Character.isWhitespace(text.charAt(start))) {
            start++;
        }
        int end = start;
        while (end < text.length() && (Character.isDigit(text.charAt(end)) || text.charAt(end) == '-')) {
            end++;
        }
        if (end > start) {
            try {
                return Long.parseLong(text.substring(start, end));
            } catch (Throwable ignored) {
            }
        }
        return def;
    }

    private static boolean parseJsonBooleanStrict(String text, String key, boolean def) {
        if (text == null || key == null) return def;
        String pattern = "\"" + key + "\"";
        int idx = text.indexOf(pattern);
        if (idx < 0) return def;
        int colon = text.indexOf(':', idx + pattern.length());
        if (colon < 0) {
            enhValidationOnlyMalformed = true;
            return def;
        }
        int start = colon + 1;
        while (start < text.length() && Character.isWhitespace(text.charAt(start))) {
            start++;
        }
        if (text.startsWith("true", start)) {
            return true;
        }
        if (text.startsWith("false", start)) {
            return false;
        }
        enhValidationOnlyMalformed = true;
        return def;
    }

    private static String formatRfc3339(long timeMillis) {
        java.util.Calendar cal = java.util.Calendar.getInstance(java.util.TimeZone.getTimeZone("UTC"));
        cal.setTime(new java.util.Date(timeMillis));
        int year = cal.get(java.util.Calendar.YEAR);
        int month = cal.get(java.util.Calendar.MONTH) + 1;
        int day = cal.get(java.util.Calendar.DAY_OF_MONTH);
        int hour = cal.get(java.util.Calendar.HOUR_OF_DAY);
        int minute = cal.get(java.util.Calendar.MINUTE);
        int second = cal.get(java.util.Calendar.SECOND);
        StringBuffer sb = new StringBuffer(24);
        sb.append(year).append('-');
        if (month < 10) sb.append('0');
        sb.append(month).append('-');
        if (day < 10) sb.append('0');
        sb.append(day).append('T');
        if (hour < 10) sb.append('0');
        sb.append(hour).append(':');
        if (minute < 10) sb.append('0');
        sb.append(minute).append(':');
        if (second < 10) sb.append('0');
        sb.append(second).append('Z');
        return sb.toString();
    }

    public static String formatEnhancementStatusJson() {
        StringBuffer sb = new StringBuffer(512);
        sb.append("{\n");
        sb.append("  \"version\": 1,\n");
        sb.append("  \"request_id\": \"").append(enhRequestId != null ? enhRequestId : "").append("\",\n");
        sb.append("  \"state\": \"").append(getEnhancementStateName(enhState)).append("\",\n");
        sb.append("  \"captured_slot\": ").append(enhCapturedSlot).append(",\n");
        sb.append("  \"template_id\": ").append(enhTemplateId).append(",\n");
        sb.append("  \"category\": ").append(enhCategory).append(",\n");
        sb.append("  \"base_name\": \"").append(enhBaseName != null ? enhBaseName : "").append("\",\n");
        sb.append("  \"start_level\": ").append(enhStartLevel).append(",\n");
        sb.append("  \"current_level\": ").append(enhCurrentLevel).append(",\n");
        sb.append("  \"target_level\": ").append(enhTargetLevel).append(",\n");
        sb.append("  \"configured_charm_mode\": ").append(enhConfiguredCharmMode).append(",\n");
        sb.append("  \"resolved_charm_mode\": ").append(enhResolvedCharmMode).append(",\n");
        sb.append("  \"payment_type\": ").append(enhPaymentType).append(",\n");
        sb.append("  \"attempt_count\": ").append(enhAttemptCount).append(",\n");
        sb.append("  \"max_attempts\": ").append(enhMaxAttempts).append(",\n");
        if (enhValidationOnly) {
            sb.append("  \"validation_only\": true,\n");
        }
        if ("STATE_RECONCILED_SUCCESS".equals(enhSettlementProvenance)) {
            sb.append("  \"last_result\": \"STATE_RECONCILED_SUCCESS\",\n");
            sb.append("  \"result_code\": null,\n");
            sb.append("  \"settlement_source\": \"STATE_RECONCILED\",\n");
            sb.append("  \"settlement_provenance\": \"STATE_RECONCILED_SUCCESS\",\n");
        } else if ("SUCCESS".equals(enhLastResult) || "RESULT_CODE_SUCCESS".equals(enhSettlementProvenance)) {
            sb.append("  \"last_result\": \"SUCCESS\",\n");
            sb.append("  \"result_code\": 3,\n");
            sb.append("  \"settlement_source\": \"RESULT_CODE\",\n");
            sb.append("  \"settlement_provenance\": \"RESULT_CODE_SUCCESS\",\n");
        } else if (enhLastResult != null) {
            sb.append("  \"last_result\": \"").append(enhLastResult).append("\",\n");
            if (enhResultCode > 0) {
                sb.append("  \"result_code\": ").append(enhResultCode).append(",\n");
            } else {
                sb.append("  \"result_code\": null,\n");
            }
            sb.append("  \"settlement_source\": \"RESULT_CODE\",\n");
            sb.append("  \"settlement_provenance\": \"").append(enhLastResult).append("\",\n");
        } else {
            sb.append("  \"last_result\": null,\n");
            sb.append("  \"result_code\": null,\n");
            sb.append("  \"settlement_source\": null,\n");
            sb.append("  \"settlement_provenance\": null,\n");
        }
        sb.append("  \"quoted_gold_cost\": ").append(enhQuotedGoldCost).append(",\n");
        sb.append("  \"quoted_gem_cost\": ").append(enhQuotedGemCost).append(",\n");
        sb.append("  \"quoted_material_requirements\": [");
        if (enhQuotedMaterialRequirements != null) {
            for (int i = 0; i < enhQuotedMaterialRequirements.length; i++) {
                if (i > 0) sb.append(", ");
                sb.append(enhQuotedMaterialRequirements[i]);
            }
        }
        sb.append("],\n");
        sb.append("  \"actual_gold_spent\": ").append(enhActualGoldSpent).append(",\n");
        sb.append("  \"actual_gem_spent\": ").append(enhActualGemSpent).append(",\n");
        sb.append("  \"actual_materials_spent\": [");
        if (enhActualMaterialsSpent != null) {
            for (int i = 0; i < enhActualMaterialsSpent.length; i++) {
                if (i > 0) sb.append(", ");
                sb.append(enhActualMaterialsSpent[i]);
            }
        }
        sb.append("],\n");
        sb.append("  \"actual_charms_spent\": ").append(enhActualCharmsSpent).append(",\n");
        sb.append("  \"accounting_status\": \"").append(enhAccountingStatus != null ? enhAccountingStatus : "PENDING").append("\",\n");
        String errCode = enhErrorCode;
        if (errCode == null && enhState >= 18 && enhState != 17 && enhState != 39) {
            errCode = getEnhancementStateName(enhState);
        }
        if (errCode != null) {
            sb.append("  \"error_code\": \"").append(errCode).append("\",\n");
        }
        if (enhErrorMessage != null) {
            sb.append("  \"error_message\": \"").append(clean(enhErrorMessage)).append("\",\n");
        }
        sb.append("  \"updated_at\": \"").append(formatRfc3339(System.currentTimeMillis())).append("\"\n");
        sb.append("}");
        return sb.toString();
    }

    public static void publishEnhancementStatus() {
        if (isEnhancementStateTerminal(enhState) && !enhOwnsResultDialog) {
            cleanEnhancementRouting();
        }
        if (enhStatusPath == null) {
            return;
        }
        try {
            String json = formatEnhancementStatusJson();
            java.io.File target = new java.io.File(enhStatusPath);
            java.io.File temp = new java.io.File(enhStatusPath + ".tmp");
            java.io.FileOutputStream fos = new java.io.FileOutputStream(temp);
            fos.write(json.getBytes("UTF-8"));
            fos.flush();
            fos.close();
            if (target.exists()) {
                target.delete();
            }
            temp.renameTo(target);
        } catch (Throwable ignored) {
        }
    }

    private static void enhance() {
        long now = System.currentTimeMillis();
        enhSidecarTick(now);

        if (enhState == 0 || isEnhancementStateTerminal(enhState)) {
            if (!enhanceOn) {
                return;
            }
            enhancePhase = 0;
            return;
        }

        try {
            if (enhState >= 1 && enhState <= 6) {
                if (isEnhancementTravelConflict()) {
                    enhState = 34; // ENHANCEMENT_TRAVEL_CONFLICT
                    enhErrorCode = "ENHANCEMENT_TRAVEL_CONFLICT";
                    enhErrorMessage = "Enhancement travel conflict: navigation owned by Auto Farm or Manual Travel";
                    cleanEnhancementRouting();
                    publishEnhancementStatus();
                    return;
                }
            }
            switch (enhState) {
                case 1: // VALIDATING_REQUEST
                    if (enhCategory != 3 || enhExpectedLevel < 0 || enhExpectedLevel > 14
                            || enhTargetLevel <= enhExpectedLevel || enhTargetLevel > 15
                            || enhConfiguredCharmMode > 3 || enhPaymentType > 1 || enhMaxAttempts < 1
                            || enhValidationOnlyMalformed) {
                        enhState = 23; // INELIGIBLE_ITEM
                        enhErrorMessage = "Invalid enhancement request parameters";
                        publishEnhancementStatus();
                        return;
                    }
                    enhState = 2; // WAITING_GAME_READY
                    publishEnhancementStatus();
                    break;

                case 2: // WAITING_GAME_READY
                    if (!inGame() || GameScreen.player == null || GameScreen.Vecplayers == null) {
                        return;
                    }
                    enhState = 3; // VALIDATING_TARGET
                    publishEnhancementStatus();
                    break;

                case 3: // VALIDATING_TARGET
                    validateEnhancementTarget();
                    publishEnhancementStatus();
                    break;

                case 4: // LOCATING_BLACKSMITH
                    int here = GameCanvas.loadmap != null ? GameCanvas.loadmap.idMap : -1;
                    if (here != BLACKSMITH_MAP) {
                        int hop = mapNextHop(here, BLACKSMITH_MAP);
                        if (hop < 0 || mapDistance(here, BLACKSMITH_MAP) < 0 || travelState == TV_BLOCKED) {
                            enhState = 35; // BLACKSMITH_ROUTE_UNAVAILABLE
                            enhErrorCode = "BLACKSMITH_ROUTE_UNAVAILABLE";
                            enhErrorMessage = "No route from map " + here + " to Blacksmith map " + BLACKSMITH_MAP;
                            cleanEnhancementRouting();
                            publishEnhancementStatus();
                            return;
                        }
                        enhNavigating = true;
                        publishEnhancementStatus();
                        return;
                    }

                    // Authoritative arrival on Map 1
                    enhNavigating = false;
                    if (!mapStable() || !gameReady()) {
                        return;
                    }
                    MainObject blacksmith = findBlacksmithNpc();
                    if (blacksmith == null) {
                        enhBlacksmithScanTicks++;
                        if (enhBlacksmithScanTicks > MAX_BLACKSMITH_SCANS) {
                            enhState = 36; // BLACKSMITH_NOT_FOUND
                            enhErrorCode = "BLACKSMITH_NOT_FOUND";
                            enhErrorMessage = "Blacksmith NPC not found on Map 1";
                            cleanEnhancementRouting();
                            publishEnhancementStatus();
                            return;
                        }
                        travelMove(here, BLACKSMITH_ANCHOR_X, BLACKSMITH_ANCHOR_Y);
                        return;
                    }
                    enhBlacksmithScanTicks = 0;
                    int dist = abs(GameScreen.player.x - blacksmith.x) + abs(GameScreen.player.y - blacksmith.y);
                    if (dist > 45) {
                        travelMove(here, blacksmith.x, blacksmith.y);
                        enhState = 5; // APPROACHING_BLACKSMITH
                    } else {
                        enhState = 6; // OPENING_FORGE
                        enhWait = 20;
                        enhForgeOpenTries = 1;
                        enhArmedBlacksmithNpcId = blacksmith.ID;
                        enhArmedBlacksmithGeneration = ++enhBlacksmithInteractionGenerationSeq;
                        try {
                            GlobalService.gI().chat_npc((byte) blacksmith.ID);
                        } catch (Throwable t) {
                            enhArmedBlacksmithNpcId = Integer.MIN_VALUE;
                            enhArmedBlacksmithGeneration = 0L;
                            enhState = 37; // BLACKSMITH_INTERACTION_FAILED
                            enhErrorCode = "BLACKSMITH_INTERACTION_FAILED";
                            enhErrorMessage = "Failed sending interaction packet to blacksmith";
                            cleanEnhancementRouting();
                        }
                    }
                    publishEnhancementStatus();
                    break;

                case 5: // APPROACHING_BLACKSMITH
                    MainObject bs = findBlacksmithNpc();
                    if (bs == null) {
                        enhState = 37; // BLACKSMITH_INTERACTION_FAILED
                        enhErrorCode = "BLACKSMITH_INTERACTION_FAILED";
                        enhErrorMessage = "Blacksmith NPC lost during approach";
                        cleanEnhancementRouting();
                        publishEnhancementStatus();
                        return;
                    }
                    int d = abs(GameScreen.player.x - bs.x) + abs(GameScreen.player.y - bs.y);
                    if (d <= 45) {
                        enhState = 6; // OPENING_FORGE
                        enhWait = 20;
                        enhForgeOpenTries = 1;
                        enhArmedBlacksmithNpcId = bs.ID;
                        enhArmedBlacksmithGeneration = ++enhBlacksmithInteractionGenerationSeq;
                        try {
                            GlobalService.gI().chat_npc((byte) bs.ID);
                        } catch (Throwable t) {
                            enhArmedBlacksmithNpcId = Integer.MIN_VALUE;
                            enhArmedBlacksmithGeneration = 0L;
                            enhState = 37; // BLACKSMITH_INTERACTION_FAILED
                            enhErrorCode = "BLACKSMITH_INTERACTION_FAILED";
                            enhErrorMessage = "Failed sending interaction packet to blacksmith";
                            cleanEnhancementRouting();
                        }
                        publishEnhancementStatus();
                    } else {
                        int curMap = GameCanvas.loadmap != null ? GameCanvas.loadmap.idMap : 0;
                        travelMove(curMap, bs.x, bs.y);
                    }
                    break;

                case 6: // OPENING_FORGE
                    if (isForgeScreenOpen()) {
                        enhOwnsForgeScreen = true;
                        clearPendingForgeMenu();
                        enhIntroDialogHandled = false;
                        enhLastDispatchedIntroDialogFingerprint = 0L;
                        enhLastDispatchedIntroDialogGen = 0L;
                        enhArmedBlacksmithNpcId = Integer.MIN_VALUE;
                        enhArmedBlacksmithGeneration = 0L;
                        enhIntroDialogWaitTicks = 0;
                        if (enhValidationOnly) {
                            enhState = 39; // DRY_RUN_COMPLETE
                            enhForgeOpenTries = 0;
                            enhErrorCode = null;
                            enhErrorMessage = null;
                            cleanEnhancementRouting();
                            publishEnhancementStatus();
                            return;
                        }
                        enhState = 7; // INSERTING_TARGET
                        enhForgeOpenTries = 0;
                        publishEnhancementStatus();
                        return;
                    }
                    if (GameCanvas.currentDialog != null) {
                        MainDialog curDlg = GameCanvas.currentDialog;
                        if (isPhapSuDialogContext(curDlg)) {
                            iCommand introBtn = findBlacksmithIntroDialogButton(curDlg);
                            if (introBtn == null) {
                                enhState = 38; // FORGE_OPEN_FAILED
                                enhErrorCode = "FORGE_INTRO_DIALOG_NO_ACTION";
                                enhErrorMessage = "Intro dialog has no approved action button";
                                cleanEnhancementRouting();
                                publishEnhancementStatus();
                                return;
                            }
                            long dlgFp = computeDialogFingerprint(curDlg);
                            if (dlgFp == enhLastDispatchedIntroDialogFingerprint && enhLastDispatchedIntroDialogGen == enhArmedBlacksmithGeneration) {
                                enhIntroDialogWaitTicks++;
                                if (enhIntroDialogWaitTicks > MAX_INTRO_DIALOG_WAIT_TICKS) {
                                    trace("ENHANCE intro dialog timeout after " + enhIntroDialogWaitTicks + " ticks");
                                    enhState = 38; // FORGE_OPEN_FAILED
                                    enhErrorCode = "FORGE_INTRO_DIALOG_TIMEOUT";
                                    enhErrorMessage = "Intro dialog failed to advance within budget";
                                    cleanEnhancementRouting();
                                    publishEnhancementStatus();
                                    return;
                                }
                                return;
                            }
                            if (GameCanvas.currentDialog != curDlg) {
                                return;
                            }
                            enhLastDispatchedIntroDialogFingerprint = dlgFp;
                            enhLastDispatchedIntroDialogGen = enhArmedBlacksmithGeneration;
                            enhIntroDialogHandled = true;
                            enhIntroDialogWaitTicks = 0;
                            trace("ENHANCE advancing intro dialog via native perform() caption=" + introBtn.caption);
                            try {
                                introBtn.perform();
                            } catch (Throwable t) {
                                trace("ENHANCE intro dialog perform threw exception: " + t);
                                enhState = 38;
                                enhErrorCode = "FORGE_INTRO_DIALOG_EXCEPTION";
                                enhErrorMessage = "Native intro dialog perform threw exception: " + t.getMessage();
                                cleanEnhancementRouting();
                                publishEnhancementStatus();
                                return;
                            }
                            enhWait = 20;
                            return;
                        } else {
                            return;
                        }
                    }
                    PendingForgeMenu pending = getPendingForgeMenu();
                    if (pending != null) {
                        // Phase 1 & Phase 6: Active menu identity guard & strictly native dispatch
                        boolean nativeReady = false;
                        if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                            mVector nativeItems = GameCanvas.menu2.menuItems;
                            if (nativeItems != null && nativeItems.size() == pending.itemCount) {
                                int opt = pending.option;
                                if (opt >= 0 && opt < nativeItems.size()) {
                                    Object itemAtOpt = nativeItems.elementAt(opt);
                                    if (itemAtOpt instanceof iCommand) {
                                        String normCaption = norm(((iCommand) itemAtOpt).caption);
                                        if ("cuong hoa".equals(normCaption)) {
                                            nativeReady = true;
                                        }
                                    }
                                }
                            }
                        }

                        if (!nativeReady) {
                            // Phase 3: Bounded native ready wait
                            enhPendingMenuWaitTicks++;
                            if (enhPendingMenuWaitTicks > MAX_PENDING_MENU_WAIT_TICKS) {
                                trace("ENHANCE native menu readiness timed out after " + enhPendingMenuWaitTicks + " ticks");
                                enhState = 38; // FORGE_OPEN_FAILED
                                enhErrorCode = "FORGE_MENU_READY_TIMEOUT";
                                enhErrorMessage = "Native Menu2 failed to become ready within budget";
                                cleanEnhancementRouting();
                                publishEnhancementStatus();
                                return;
                            }
                            // While native Menu2 has not completed construction, keep pending state and wait
                            return;
                        }

                        // Native Menu2 is fully ready and positively verified: invoke native selection
                        boolean dispatchSuccess = false;
                        try {
                            setFrIndex(GameCanvas.menu2, pending.option);
                            GameCanvas.menu2.commandPointer(2, 0);
                            dispatchSuccess = true;
                        } catch (Throwable t) {
                            trace("ENHANCE native commandPointer threw exception: " + t);
                            enhState = 38; // FORGE_OPEN_FAILED
                            enhErrorCode = "FORGE_DISPATCH_EXCEPTION";
                            enhErrorMessage = "Native commandPointer threw exception: " + t.getMessage();
                            cleanEnhancementRouting();
                            publishEnhancementStatus();
                            return;
                        }

                        if (dispatchSuccess) {
                            synchronized (ENH_MENU_LOCK) {
                                enhLastDispatchedMenuFingerprint = pending.fingerprint;
                                enhPendingMenuRecord = null;
                                enhPendingMenuNpc = Integer.MIN_VALUE;
                                enhPendingMenuId = -1;
                                enhPendingMenuOption = -1;
                                enhPendingMenuFingerprint = 0L;
                                enhPendingMenuWaitTicks = 0;
                            }
                            enhWait = 20;
                            return;
                        }
                    }
                    if (enhWait > 0) {
                        enhWait--;
                        return;
                    }
                    if (GameCanvas.currentDialog != null || GameCanvas.subDialog != null || (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu)) {
                        return;
                    }
                    if (enhForgeOpenTries >= 3) {
                        enhState = 38; // FORGE_OPEN_FAILED
                        enhErrorCode = "FORGE_OPEN_FAILED";
                        enhErrorMessage = "Forge dialog failed to open";
                        cleanEnhancementRouting();
                        publishEnhancementStatus();
                        return;
                    }
                    MainObject bsRetry = findBlacksmithNpc();
                    if (bsRetry != null) {
                        enhForgeOpenTries++;
                        enhWait = 20;
                        enhLastDispatchedMenuFingerprint = 0L;
                        enhArmedBlacksmithNpcId = bsRetry.ID;
                        enhArmedBlacksmithGeneration = ++enhBlacksmithInteractionGenerationSeq;
                        try {
                            GlobalService.gI().chat_npc((byte) bsRetry.ID);
                        } catch (Throwable t) {
                            enhArmedBlacksmithNpcId = Integer.MIN_VALUE;
                            enhArmedBlacksmithGeneration = 0L;
                            enhState = 37; // BLACKSMITH_INTERACTION_FAILED
                            enhErrorCode = "BLACKSMITH_INTERACTION_FAILED";
                            enhErrorMessage = "Failed retry interaction packet to blacksmith";
                            cleanEnhancementRouting();
                            publishEnhancementStatus();
                            return;
                        }
                    } else {
                        enhState = 37; // BLACKSMITH_INTERACTION_FAILED
                        enhErrorCode = "BLACKSMITH_INTERACTION_FAILED";
                        enhErrorMessage = "Blacksmith NPC lost during retry";
                        cleanEnhancementRouting();
                        publishEnhancementStatus();
                        return;
                    }
                    break;

                case 7: // INSERTING_TARGET
                    if (enhValidationOnly) {
                        enhState = 39; // DRY_RUN_COMPLETE
                        enhErrorCode = null;
                        enhErrorMessage = null;
                        cleanEnhancementRouting();
                        publishEnhancementStatus();
                        return;
                    }
                    if (TabRebuildItem.itemRe != null && TabRebuildItem.itemRe.Id == enhTemplateId) {
                        enhState = 8; // RESOLVING_CHARM
                        publishEnhancementStatus();
                        return;
                    }
                    MainItem targetItem = findBagItem(enhTemplateId, enhCategory);
                    if (targetItem == null) {
                        enhState = 20;
                        publishEnhancementStatus();
                        return;
                    }
                    GlobalService.gI().Rebuild_Item((byte) 0, (short) targetItem.Id, (byte) targetItem.ItemCatagory);
                    enhState = 8; // RESOLVING_CHARM
                    publishEnhancementStatus();
                    break;

                case 8: // RESOLVING_CHARM
                    resolveEnhancementCharm();
                    publishEnhancementStatus();
                    break;

                case 9: // INSERTING_CHARM
                    if (enhValidationOnly) {
                        enhState = 39; // DRY_RUN_COMPLETE
                        enhErrorCode = null;
                        enhErrorMessage = null;
                        cleanEnhancementRouting();
                        publishEnhancementStatus();
                        return;
                    }
                    if (enhResolvedCharmMode == 0) {
                        enhState = 10;
                        publishEnhancementStatus();
                        return;
                    }
                    if (TabRebuildItem.itemPlus != null && TabRebuildItem.itemPlus.Id == snapSelectedCharmTemplateId) {
                        enhState = 10; // VERIFYING_RESOURCES
                        publishEnhancementStatus();
                        return;
                    }
                    MainItem charmItem = findBagItem(snapSelectedCharmTemplateId, 7);
                    if (charmItem == null) {
                        enhState = 24;
                        publishEnhancementStatus();
                        return;
                    }
                    GlobalService.gI().Rebuild_Item((byte) 0, (short) charmItem.Id, (byte) 7);
                    enhState = 10; // VERIFYING_RESOURCES
                    publishEnhancementStatus();
                    break;

                case 10: // VERIFYING_RESOURCES
                    verifyEnhancementResources();
                    publishEnhancementStatus();
                    break;

                case 11: // READY_FOR_ATTEMPT
                    executeEnhancementAttempt();
                    publishEnhancementStatus();
                    break;

                case 12: // ATTEMPTING
                    enhState = 13;
                    break;

                case 13: // WAITING_RESULT
                    if (TabRebuildItem.isNextRebuild == 3) {
                        enhOwnsResultDialog = true;
                        enhResultCode = 3;
                        enhSettlementSource = "RESULT_CODE";
                        enhSettlementProvenance = "RESULT_CODE_SUCCESS";
                        enhLastResult = "SUCCESS";
                        enhState = 14;
                        settleEnhancementResult();
                        publishEnhancementStatus();
                        return;
                    }
                    if (TabRebuildItem.isNextRebuild == 4) {
                        enhOwnsResultDialog = true;
                        enhResultCode = 4;
                        enhSettlementSource = "RESULT_CODE";
                        enhState = 14;
                        settleEnhancementResult();
                        publishEnhancementStatus();
                        return;
                    }
                    if (TabRebuildItem.isNextRebuild == 1 || TabRebuildItem.isNextRebuild == 2 || TabRebuildItem.isNextRebuild >= 5) {
                        enhState = 28; // SERVER_REJECTED
                        enhErrorCode = "SERVER_REJECTED";
                        enhErrorMessage = "Server rejected enhancement attempt (code=" + TabRebuildItem.isNextRebuild + ")";
                        enhResultCode = TabRebuildItem.isNextRebuild;
                        enhSettlementSource = "RESULT_CODE";
                        enhInFlightExecute = false;
                        cleanEnhancementRouting();
                        publishEnhancementStatus();
                        return;
                    }
                    long nowWait = System.currentTimeMillis();
                    boolean waitTimeout = (nowWait >= enhResultDeadline);
                    if (enhWait > 0 && !waitTimeout) {
                        enhWait--;
                        return;
                    }

                    // Fallback conservative state reconciliation
                    if (canReconcileStateSuccess()) {
                        enhOwnsResultDialog = false;
                        enhResultCode = -1; // Keep result_code NULL for state-reconciled success
                        enhSettlementSource = "STATE_RECONCILED";
                        enhSettlementProvenance = "STATE_RECONCILED_SUCCESS";
                        enhLastResult = "STATE_RECONCILED_SUCCESS";
                        settleEnhancementResult();
                        publishEnhancementStatus();
                        return;
                    }

                    enhState = 29; // RESULT_AMBIGUOUS
                    enhErrorCode = "RESULT_AMBIGUOUS";
                    enhErrorMessage = "Timed out waiting for server enhancement result and exact state reconciliation criteria not met";
                    cleanEnhancementRouting();
                    publishEnhancementStatus();
                    break;

                case 14: // WAITING_SETTLEMENT
                    settleEnhancementResult();
                    publishEnhancementStatus();
                    break;

                case 15: // FAILURE_PROTECTED
                case 16: // FAILURE_DEGRADED
                    if (enhAttemptCount < enhMaxAttempts) {
                        enhState = 7;
                    } else {
                        enhState = 18; // ATTEMPT_LIMIT_REACHED
                    }
                    publishEnhancementStatus();
                    break;
            }
        } catch (Throwable t) {
            // A module must never stall the client tick.
        }
    }

    /** Forgets one trip. Called when the switch turns off; `allOff()` clears the same fields inline. */
    private static void enhanceReset() {
        enhancePhase = 0;
        enhanceWhy = 0;
        enhanceWait = 0;
        enhState = 0;
        enhActiveTargetSlot = -1;
        enhInFlightExecute = false;
        enhValidationOnly = false;
        enhValidationOnlyMalformed = false;
        enhErrorCode = null;
        enhErrorMessage = null;
        enhTerminalCleanupTicks = 0;
        snapRequestId = null;
        snapTemplateId = 0;
        snapCategory = 3;
        snapBaseName = null;
        snapTier = 0;
        snapExpectedLevel = 0;
        snapTargetLevel = 0;
        snapPaymentType = 0;
        snapResolvedCharmMode = 0;
        snapRecipeGoldCost = 0L;
        snapRecipeGemCost = 0L;
        snapRecipeMaterials = new long[4];
        enhExecuteStartedAt = 0L;
        enhResultDeadline = 0L;
        enhResultCode = -1;
        enhSettlementSource = null;
        enhSettlementProvenance = null;
        cleanEnhancementRouting();
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
    // {@link #serverMenu}: the selection is `GlobalService.gI().b(idNPC, idMenu, index)` quoting the pair the
    // SERVER sent, because a server menu's buttons carry no command of their own; and pressing a
    // button selects whatever is highlighted (`Menu2.h`), which is private in this build. The second
    // is why nothing here sets a cursor: the index travels in the packet instead, which is the same
    // thing the client's own `Menu2.a(2, _)` sends.

    /**
     * The nearest NPC whose name matches, or the one carrying the fallback template id.
     *
     * The name is the match and the id is the fallback, not the other way round: the id this server
     * gives the dungeon guide is -37 today, but template ids differ per region the way the teleport
     * stones' do, so the name is what survives a client update. Nearest by Manhattan, because the
     * client's own reach checks are Manhattan and a mixed metric would call a farther NPC nearer.
     */
    private static MainObject dungeonNpc() {
        if (GameScreen.Vecplayers == null || GameScreen.player == null) {
            return null;
        }
        MainObject best = null;
        MainObject fallback = null;
        int bestDistance = Integer.MAX_VALUE;
        for (int i = 0; i < GameScreen.Vecplayers.size(); i++) {
            Object entry = GameScreen.Vecplayers.elementAt(i);
            if (!(entry instanceof MainObject)) {
                continue;
            }
            MainObject candidate = (MainObject) entry;
            // cv == 2 is an NPC; 0 is a player and 1 a monster.
            if (candidate.typeObject != 2) {
                continue;
            }
            if (candidate.name != null && norm(candidate.name).indexOf(DUNGEON_NPC_NAME) >= 0) {
                int distance = abs(GameScreen.player.x - candidate.x) + abs(GameScreen.player.y - candidate.y);
                if (distance < bestDistance) {
                    bestDistance = distance;
                    best = candidate;
                }
                continue;
            }
            if (fallback == null && candidate.ID == DUNGEON_NPC_CU) {
                fallback = candidate;
            }
        }
        return best != null ? best : fallback;
    }

    /**
    /**
     * Source-proven native interaction eligibility from cf.g:2028-2037:
     * MainObject.getDistance(npc.x, npc.y, GameScreen.player.x, GameScreen.player.y) <= GameScreen.player.wFocus
     * Uses Euclidean distance and player's reach/scan radius bi (default 140px).
     */
    public static boolean dungeonNpcEligible(MainObject npc) {
        if (npc == null || GameScreen.player == null) {
            return false;
        }
        int reach = GameScreen.player.wFocus > 0 ? GameScreen.player.wFocus : 140;
        return MainObject.getDistance(npc.x, npc.y, GameScreen.player.x, GameScreen.player.y) <= reach;
    }

    /**
     * Asks an NPC and arms the wait, exactly as {@link #travelStone} asks a teleport stone.
     *
     * Returns false only when the send itself failed. The reply is a packet and lands on a later
     * tick, so this never polls for it — it records the NPC's id, so the ask can be repeated
     * without the entity still being in the scene stream, and hands the tick back.
     */
    private static boolean dungeonClickNpc(MainObject npc) {
        dungeonNpcCu = npc.ID;
        dungeonState = DN_PREPARATION;
        dungeonStep = 0;
        dungeonMenuId = Integer.MIN_VALUE;
        dungeonMenu = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        GameScreen.ObjFocus = npc;
        try {
            MainObject.resetDirection(GameScreen.player, npc);
        } catch (Throwable t) {
        }
        try {
            GameScreen.player.resetAction();
        } catch (Throwable t) {
        }
        try {
            npc.GiaoTiep();
        } catch (Throwable t) {
            dungeonState = DN_ROUTING;
            return false;
        }
        dungeonTried = 0;
        dungeonStallTicks = 0;
        dungeonWait = 40;
        trace("DUNGEON asked NPC cu=" + npc.ID + " at " + npc.x + "," + npc.y);
        return true;
    }

    /** Re-asks the NPC last clicked, by id. The bounded retry is a second question, not a tighter loop. */
    private static boolean dungeonAskNpc() {
        if (dungeonNpcCu == -1) {
            return false;
        }
        MainObject npc = dungeonNpc();
        if (npc != null && GameScreen.player != null) {
            GameScreen.ObjFocus = npc;
            try {
                MainObject.resetDirection(GameScreen.player, npc);
                GameScreen.player.resetAction();
            } catch (Throwable t) {
            }
            try {
                npc.GiaoTiep();
                dungeonMenu = null;
                dungeonMenuNpc = Integer.MIN_VALUE;
                dungeonWait = 40;
                return true;
            } catch (Throwable t) {
                return false;
            }
        }
        return false;
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

    static int getFrIndex(Menu2 menu) {
        if (menu == null) {
            return -1;
        }
        return menu.menuSelectedItem;
    }

    static void setFrIndex(Menu2 menu, int index) {
        if (menu == null) {
            return;
        }
        menu.menuSelectedItem = index;
    }

    static mVector getFrItems(Menu2 menu) {
        if (menu == null) {
            return null;
        }
        return menu.menuItems;
    }

    static iCommand findGiaoTiepInDialog(MainDialog dialog) {
        if (dialog == null) {
            return null;
        }
        try {
            if (dialog.right != null && dialog.right.caption != null) {
                String s = norm(dialog.right.caption);
                if (s.indexOf("giao tiep") >= 0 && s.indexOf("giao dich") < 0) {
                    return dialog.right;
                }
            }
            if (dialog.left != null && dialog.left.caption != null) {
                String s = norm(dialog.left.caption);
                if (s.indexOf("giao tiep") >= 0 && s.indexOf("giao dich") < 0) {
                    return dialog.left;
                }
            }
            if (dialog instanceof MsgDialog) {
                mVector buttons = ((MsgDialog) dialog).cmdList;
                if (buttons != null) {
                    for (int i = 0; i < buttons.size(); i++) {
                        Object entry = buttons.elementAt(i);
                        if (entry instanceof iCommand) {
                            iCommand btn = (iCommand) entry;
                            if (btn.caption != null) {
                                String s = norm(btn.caption);
                                if (s.indexOf("giao tiep") >= 0 && s.indexOf("giao dich") < 0) {
                                    return btn;
                                }
                            }
                        }
                    }
                }
            }
        } catch (Throwable t) {
        }
        return null;
    }

    /**
     * Identifies whether a dialog in GameCanvas.currentDialog is an active NPC conversation/story dialog for Pho Chi Huy,
     * matching the KnightMod V2 bytecode contract (v2_dungeon.java:369).
     */
    static boolean isNpcSpeechDialog(MainDialog dialog) {
        if (dialog == null) {
            return false;
        }
        try {
            if (findGiaoTiepInDialog(dialog) != null) {
                return true;
            }
            String text = dialogText(dialog);
            if (text != null && text.length() > 0) {
                String n = norm(text);
                if (n.indexOf("co muon") >= 0 || n.indexOf("ban co muon") >= 0) {
                    return false;
                }
                if (n.indexOf("pho chi huy") >= 0 || n.indexOf("nhiem vu") >= 0 || (n.indexOf("nga tu") >= 0 && n.indexOf("muon") < 0)) {
                    return true;
                }
            }
        } catch (Throwable t) {
        }
        return false;
    }

    /**
     * Picks a row of the captured menu and forgets the capture.
     *
     * Invokes native server menu action handler Menu2.a(2, 0) when the active client menu is open,
     * allowing the native client to manage network dispatch and panel closing side-effects.
     * Falls back to raw network dispatch if Menu2 is not active (mocked test harness).
     */
    private static boolean dungeonSelect(int index) {
        int npc = dungeonMenuNpc;
        int menuId = dungeonMenuId;
        dungeonMenu = null;
        dungeonMenuItems = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        try {
            if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                setFrIndex(GameCanvas.menu2, index);
                GameCanvas.menu2.commandPointer(2, 0);
                return true;
            }
            // Fallback for mocked test harness
            GlobalService.gI().Dynamic_Menu((short) npc, (byte) menuId, (byte) index);
        } catch (Throwable t) {
            return false;
        }
        return true;
    }

    /** Drops the panel this module opened. A menu left up blocks every module that gates on ready(). */
    private static void dungeonCloseMenu() {
        try {
            if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                GameCanvas.menu2.doCloseMenu();
            }
        } catch (Throwable t) {
            // A menu that will not close is not worth stalling the trip over.
        }
        dungeonMenu = null;
        dungeonMenuItems = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
    }

    /**
     * Whether the character is inside the dungeon.
     *
     * The map id first and the client's own name as confirmation, because the id is survey data
     * about this server while the name comes from `T.mapName` — index 48 is "Ngã tư tử thần", which
     * {@link #norm} reduces to "nga tu tu than". No other map in that table normalises to contain
     * "nga tu", so the name cannot false-positive onto a different map.
     */
    private static boolean dungeonInDungeon() {
        if (GameCanvas.loadmap == null) {
            return false;
        }
        return GameCanvas.loadmap.idMap == DUNGEON_MAP
                || norm(mapName(GameCanvas.loadmap.idMap)).indexOf(DUNGEON_NAME) >= 0;
    }

    /** Time zone identifier explicitly used for all dungeon scheduling calculations. */
    public static final String DUNGEON_TZ_ID = "GMT+07:00";

    /**
     * Minute of day in UTC+7 (0..1439), or -1 if the clock is unreadable.
     * Computes hour * 60 + minute explicitly in GMT+07:00.
     */
    public static int dungeonMinuteOfDayUtc7(long epochMillis) {
        try {
            java.util.Calendar cal = java.util.Calendar.getInstance(java.util.TimeZone.getTimeZone(DUNGEON_TZ_ID));
            cal.setTime(new java.util.Date(epochMillis));
            return cal.get(java.util.Calendar.HOUR_OF_DAY) * 60 + cal.get(java.util.Calendar.MINUTE);
        } catch (Throwable t) {
            return -1;
        }
    }

    public static int dungeonMinuteOfDayUtc7() {
        return dungeonMinuteOfDayUtc7(System.currentTimeMillis());
    }

    /**
     * Date key in UTC+7 (year * 1000 + dayOfYear), or -1 if the clock is unreadable.
     * Stamped on the active scheduled day to track daily quotas across calendar transitions.
     */
    public static int dungeonDateKeyUtc7(long epochMillis) {
        try {
            java.util.Calendar cal = java.util.Calendar.getInstance(java.util.TimeZone.getTimeZone(DUNGEON_TZ_ID));
            cal.setTime(new java.util.Date(epochMillis));
            int year = cal.get(java.util.Calendar.YEAR);
            int dayOfYear = cal.get(java.util.Calendar.DAY_OF_YEAR);
            return year * 1000 + dayOfYear;
        } catch (Throwable t) {
            return -1;
        }
    }

    public static int dungeonDateKeyUtc7() {
        return dungeonDateKeyUtc7(System.currentTimeMillis());
    }

    /** True if exact unscheduled sentinel is configured (-1/-1). */
    public static boolean isDungeonUnscheduled() {
        return dungeonStartMin == -1 && dungeonEndMin == -1;
    }

    /** True if a valid scheduled window is configured (0 <= startMin < endMin <= 1439). */
    public static boolean isDungeonScheduled() {
        return dungeonStartMin >= 0 && dungeonEndMin <= 1439 && dungeonStartMin < dungeonEndMin;
    }

    /**
     * Whether a scheduled trip may start given converted UTC+7 minute and date key.
     * Exact unscheduled (-1/-1) is immediate (true).
     * Valid scheduled window checks range [startMin, endMin) and daily quota.
     * Any other state (invalid schedule, clock/conversion failure) FAILS CLOSED (false).
     */
    public static boolean dungeonScheduleDue(int min, int dateKey) {
        if (isDungeonUnscheduled()) {
            return true;
        }
        if (!isDungeonScheduled()) {
            return false; // Fail closed: invalid schedule window
        }
        if (min < 0 || dateKey < 0) {
            return false; // Fail closed: clock/conversion failure
        }
        if (min < dungeonStartMin || min >= dungeonEndMin) {
            return false; // Outside window
        }
        if (dateKey == dungeonScheduleDateKey && dungeonMaxRuns != -1 && dungeonRuns >= dungeonMaxRuns) {
            return false; // Daily quota exhausted
        }
        return true;
    }

    /**
     * Whether a scheduled trip may start at the given epoch timestamp.
     * Start minute is inclusive, end minute is exclusive (startMin <= min < endMin).
     * Unreadable clock or invalid conversion FAILS CLOSED (returns false).
     * Does not catch up late if past endMin.
     */
    public static boolean dungeonScheduleDue(long epochMillis) {
        if (isDungeonUnscheduled()) {
            return true;
        }
        if (!isDungeonScheduled()) {
            return false;
        }
        int min = dungeonMinuteOfDayUtc7(epochMillis);
        int dateKey = dungeonDateKeyUtc7(epochMillis);
        return dungeonScheduleDue(min, dateKey);
    }

    public static boolean dungeonScheduleDue() {
        return dungeonScheduleDue(System.currentTimeMillis());
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
    //     releases the combat fields whenever `GameCanvas.loadmap.idMap != atkMap`, and `combatOn()` writes
    //     `GameScreen.player.wFocus = atkRadius` on every tick while combat is armed — so a radius set here would be
    //     overwritten later in the same tick, and making ATTACK fight on map 48 would mean editing
    //     another module's map gate. Fighting in the dungeon is therefore the operator arming a
    //     spot on map 48 with `atk.radius` set wide, which already works and needs no new code.
    //   - It does not exclude the "thien thach" monsters. Zeus never selects a target: `combatOn()`
    //     sets `Player.isCurAutoFire`, a one-shot "catch the nearest target" flag the CLIENT consumes, and `GameScreen.ObjFocus`
    //     is client-owned — the only place this file writes it is `combatOff()`, nulling it to
    //     release. Honouring the exclusion would mean intercepting the client's own pick, which
    //     would fight `Player.isCurAutoFire` rather than serve it. Recorded here as a known gap.

    /** Dungeon states. The same discipline as {@link #TV_OFF}: anything else is a reason to stop. */
    public static final int DN_OFF = 0;
    public static final int DN_IDLE = 1;
    public static final int DN_ROUTING = 2;
    public static final int DN_PREPARATION = 3;
    public static final int DN_COMBAT = 4;
    public static final int DN_COMPLETION_WAIT = 5;
    public static final int DN_DEATH = 6;
    public static final int DN_FAILURE = 7;
    public static final int DN_MANUAL_REVIEW = 8;

    // Backward-compatible aliases:
    public static final int DN_GOTO_NPC = DN_ROUTING;
    public static final int DN_INTERACT = DN_PREPARATION;
    public static final int DN_IN_DUNGEON = DN_COMBAT;
    public static final int DN_DONE = DN_COMPLETION_WAIT;

    /** The dungeon's map id, and the two ways it is recognised. See {@link #dungeonInDungeon}. */
    public static final int DUNGEON_MAP = 48;
    public static final String DUNGEON_NAME = "nga tu";

    /** The guide's map, name and fallback template id. */
    public static final int DUNGEON_NPC_MAP = 1;
    public static final String DUNGEON_NPC_NAME = "pho chi huy";
    public static final int DUNGEON_NPC_CU = -37;
    public static final int DUNGEON_NPC_X = 552;
    public static final int DUNGEON_NPC_Y = 504;

    /** Dungeon combat anchor coordinates and scan radius, source-proven from V2 reference. */
    public static final int DUNGEON_COMBAT_X = 672;
    public static final int DUNGEON_COMBAT_Y = 600;
    public static final int DUNGEON_SCAN_RADIUS = 600;
    public static final int DUNGEON_LEASH_RADIUS = 48;
    public static final int DUNGEON_IDLE_LEASH_TICKS = 20;

    /** Asks before giving up on anything. Three is the budget KnightMod's own module used. */
    public static final int DN_MAX_TRIES = 3;
    /**
     * Ticks with no movement at all before a walk is called a stall: 2.4 s at the loop's 25 ticks/s,
     * the same order {@link #travel} uses. A path that never completes is a stall, not a walk.
     */
    public static final int DN_STALL_TICKS = 60;
    /** Ticks to stand still after a run before asking for the next. */
    public static final int DN_BETWEEN_RUNS = 100;
    public static final int DUNGEON_RUNS_MAX = 1000;

    /** Max runtime in ticks (300 seconds at 25 ticks/s = 7500 ticks). */
    public static final int DN_MAX_RUN_TICKS = 7500;

    /** Consecutive failure cap before hard-stopping to manual review. */
    public static final int DN_CONSECUTIVE_FAIL_CAP = 2;

    /**
     * DUNGEON's own settings and state.
     */
    static boolean dungeonEnabled = false;
    static int dungeonMaxRuns = -1;
    static int dungeonStartMin = -1;
    static int dungeonEndMin = -1;
    static int dungeonScheduleDateKey = -1;
    static int dungeonState = DN_OFF;
    static int dungeonWhy = 0;
    static int dungeonRuns = 0;
    static int dungeonFails = 0;
    static int dungeonConsecutiveFails = 0;
    static int dungeonWait = 0;
    static int dungeonTried = 0;
    static int dungeonStep = 0;
    static boolean dungeonWasIn = false;
    static int dungeonNpcCu = -1;
    static boolean dungeonTripActive = false;
    static int dungeonStallTicks = 0;
    static int dungeonLastX = Integer.MIN_VALUE;
    static int dungeonLastY = Integer.MIN_VALUE;
    static int dungeonMapSeen = Integer.MIN_VALUE;

    /** Internal Dungeon navigation ownership flag. */
    static boolean dungeonNavigating = false;
    /** True once live combat with a valid monster was engaged inside Map 48. */
    static boolean dungeonCombatEngaged = false;
    /** True if death occurred while inside Map 48. */
    static boolean dungeonDiedInRun = false;
    /** True if manual travel or escape was triggered during the run. */
    static boolean dungeonManualEscaped = false;
    /** True if a completion dialog or completion candidate signal was observed. */
    static boolean dungeonClearCandidate = false;
    /** True if 0 monsters were sustained for >= 50 ticks after engaging combat. */
    static boolean dungeonMonstersCleared = false;
    static int dungeonMonstersZeroTicks = 0;
    static int dungeonNoTargetTicks = 0;
    static int dungeonRunTicks = 0;

    /** Latch: true once native Ngã Tư submenu dispatch occurs, awaiting entry result. */
    static boolean dungeonAwaitingEntry = false;

    /** Labels and item collection of the server menu currently open, captured by {@link #serverMenu}. Its own trio: never TRAVEL's. */
    static String[] dungeonMenu = null;
    static mVector dungeonMenuItems = null;
    static int dungeonMenuNpc = Integer.MIN_VALUE;
    static int dungeonMenuId = 0;

    /** Backup fields for user Auto Farm / combat configuration. */
    static boolean dungeonCombatBackedUp = false;
    static int dungeonUserAtkMode = 0;
    static int dungeonUserAtkMap = 0;
    static int dungeonUserAtkX = -1;
    static int dungeonUserAtkY = -1;
    static int dungeonUserAtkRadius = 120;
    static boolean dungeonUserAtkFarmOnArrival = true;

    /** Backs up the operator's Auto Farm combat configuration before Dungeon takes over combat. */
    public static void dungeonBackupCombat() {
        if (!dungeonCombatBackedUp) {
            dungeonUserAtkMode = atkMode;
            dungeonUserAtkMap = atkMap;
            dungeonUserAtkX = atkX;
            dungeonUserAtkY = atkY;
            dungeonUserAtkRadius = atkRadius;
            dungeonUserAtkFarmOnArrival = atkFarmOnArrival;
            dungeonCombatBackedUp = true;
        }
    }

    /** Restores the operator's Auto Farm combat configuration on exit or termination. */
    public static void dungeonRestoreCombat() {
        if (dungeonCombatBackedUp) {
            atkMode = dungeonUserAtkMode;
            atkMap = dungeonUserAtkMap;
            atkX = dungeonUserAtkX;
            atkY = dungeonUserAtkY;
            atkRadius = dungeonUserAtkRadius;
            atkFarmOnArrival = dungeonUserAtkFarmOnArrival;
            dungeonCombatBackedUp = false;
        }
        if (GameScreen.player != null) {
            GameScreen.player.wFocus = NATIVE_RADIUS;
        }
        Player.isAutoFire = (byte) -1;
        Player.isAutoHPMP = false;
        Player.isCurAutoFire = false;
        GameScreen.ObjFocus = null;
    }

    /**
     * Normalizes entity name and checks whether it contains 'thien thach'.
     */
    public static boolean isMeteorTarget(MainObject target) {
        if (target == null || target.name == null) {
            return false;
        }
        String name = normSemantic(target.name);
        return name.indexOf("thien thach") >= 0;
    }

    /**
     * Validates whether an entity is a live, valid non-meteor monster target.
     */
    public static boolean isValidDungeonTarget(MainObject target) {
        if (target == null || target.typeObject != 1 || target.hp <= 0 || target.Action == 4) {
            return false;
        }
        return !isMeteorTarget(target);
    }

    /**
     * Finds the nearest valid live non-meteor monster entity relative to the dungeon center anchor.
     */
    public static MainObject findBestDungeonTarget() {
        if (GameScreen.Vecplayers == null) {
            return null;
        }
        MainObject best = null;
        int minDistance = Integer.MAX_VALUE;
        for (int i = 0; i < GameScreen.Vecplayers.size(); i++) {
            Object entry = GameScreen.Vecplayers.elementAt(i);
            if (!(entry instanceof MainObject)) {
                continue;
            }
            MainObject candidate = (MainObject) entry;
            if (!isValidDungeonTarget(candidate)) {
                continue;
            }
            int dist = Math.abs(DUNGEON_COMBAT_X - candidate.x) + Math.abs(DUNGEON_COMBAT_Y - candidate.y);
            if (dist < minDistance) {
                minDistance = dist;
                best = candidate;
            }
        }
        return best;
    }

    /**
     * Counts live, valid non-meteor monster entities currently in the scene.
     */
    public static int countLiveDungeonMonsters() {
        if (GameScreen.Vecplayers == null) {
            return 0;
        }
        int count = 0;
        for (int i = 0; i < GameScreen.Vecplayers.size(); i++) {
            Object entry = GameScreen.Vecplayers.elementAt(i);
            if (!(entry instanceof MainObject)) {
                continue;
            }
            MainObject candidate = (MainObject) entry;
            if (isValidDungeonTarget(candidate)) {
                count++;
            }
        }
        return count;
    }

    /**
     * Inspects active dialogs for clear candidate keywords while inside Map 48.
     */
    public static void checkDungeonCompletionDialog() {
        try {
            if (GameCanvas.currentDialog instanceof MsgDialog) {
                MsgDialog dialog = (MsgDialog) GameCanvas.currentDialog;
                String text = normSemantic(dialogText(dialog));
                if (text.indexOf("hoan thanh") >= 0
                        || text.indexOf("chien thang") >= 0
                        || text.indexOf("thanh cong") >= 0
                        || text.indexOf("vuot qua") >= 0
                        || text.indexOf("ket qua") >= 0
                        || text.indexOf("phan thuong") >= 0) {
                    dungeonClearCandidate = true;
                }
            }
        } catch (Throwable t) {
            // Guard against UI reflection/dialog failures
        }
    }

    /**
     * Autonomous combat loop inside Map 48.
     */
    public static void dungeonCombat() {
        dungeonBackupCombat();
        if (GameScreen.player != null) {
            GameScreen.player.wFocus = DUNGEON_SCAN_RADIUS;
        }
        if (++dungeonRunTicks > DN_MAX_RUN_TICKS) {
            trace("DUNGEON run timed out after " + dungeonRunTicks + " ticks");
            dungeonFailRun(6, "dungeon run timed out (>300s)");
            return;
        }

        checkDungeonCompletionDialog();

        // Release meteor target immediately if held
        if (GameScreen.ObjFocus != null && (isMeteorTarget(GameScreen.ObjFocus) || GameScreen.ObjFocus.typeObject != 1 || GameScreen.ObjFocus.hp <= 0 || GameScreen.ObjFocus.Action == 4)) {
            GameScreen.ObjFocus = null;
        }
        if (GameScreen.ObjFocus == null) {
            GameScreen.ObjFocus = findBestDungeonTarget();
        }

        if (GameScreen.ObjFocus != null) {
            dungeonCombatEngaged = true;
            dungeonNoTargetTicks = 0;
            dungeonMonstersZeroTicks = 0;
            Player.xBeginAutoFire = GameScreen.ObjFocus.x;
            Player.yBeginAutofire = GameScreen.ObjFocus.y;
            Player.isAutoFire = (byte) 1;
            Player.isAutoHPMP = true;
            Player.isCurAutoFire = true;
            skills();
            potions();
        } else {
            Player.isAutoFire = (byte) -1;
            Player.isAutoHPMP = false;
            Player.isCurAutoFire = false;
            ++dungeonNoTargetTicks;
            if (dungeonNoTargetTicks >= DUNGEON_IDLE_LEASH_TICKS) {
                if (GameScreen.player != null) {
                    int drift = Math.abs(GameScreen.player.x - DUNGEON_COMBAT_X) + Math.abs(GameScreen.player.y - DUNGEON_COMBAT_Y);
                    if (drift > DUNGEON_LEASH_RADIUS && canMove()) {
                        travelMove(DUNGEON_MAP, DUNGEON_COMBAT_X, DUNGEON_COMBAT_Y);
                    }
                }
            }
            potions();
            int liveCount = countLiveDungeonMonsters();
            if (liveCount == 0) {
                if (++dungeonMonstersZeroTicks >= 50) {
                    dungeonMonstersCleared = true;
                }
            } else {
                dungeonMonstersZeroTicks = 0;
            }
        }
    }

    /**
     * Forgets one trip.
     */
    public static void dungeonReset() {
        dungeonState = DN_OFF;
        dungeonWhy = 0;
        dungeonWait = 0;
        dungeonTried = 0;
        dungeonStep = 0;
        dungeonWasIn = false;
        dungeonTripActive = false;
        dungeonNpcCu = -1;
        dungeonScheduleDateKey = -1;
        dungeonStallTicks = 0;
        dungeonLastX = Integer.MIN_VALUE;
        dungeonLastY = Integer.MIN_VALUE;
        dungeonMapSeen = Integer.MIN_VALUE;
        dungeonMenu = null;
        dungeonMenuItems = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        dungeonMenuId = Integer.MIN_VALUE;
        dungeonNavigating = false;
        dungeonCombatEngaged = false;
        dungeonDiedInRun = false;
        dungeonManualEscaped = false;
        dungeonClearCandidate = false;
        dungeonMonstersCleared = false;
        dungeonMonstersZeroTicks = 0;
        dungeonNoTargetTicks = 0;
        dungeonRunTicks = 0;
        dungeonConsecutiveFails = 0;
        dungeonAwaitingEntry = false;
        if (GameCanvas.currentDialog != null && isDungeonConfirmDialog(GameCanvas.currentDialog)) {
            dismissDungeonDialog(GameCanvas.currentDialog);
        }
        if (GameCanvas.subDialog != null && isDungeonConfirmDialog(GameCanvas.subDialog)) {
            dismissDungeonDialog(GameCanvas.subDialog);
        }
        dungeonRestoreCombat();
    }

    /** Stops and says why. If why == 5, transitions to DN_MANUAL_REVIEW. */
    public static void dungeonStop(int why, String reason) {
        dungeonState = (why == 5) ? DN_MANUAL_REVIEW : DN_OFF;
        dungeonWhy = why;
        dungeonTripActive = false;
        dungeonNavigating = false;
        dungeonCloseMenu();
        dungeonMenu = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        dungeonMenuId = Integer.MIN_VALUE;
        dungeonStep = 0;
        dungeonWait = 0;
        dungeonTried = 0;
        dungeonAwaitingEntry = false;
        if (GameCanvas.currentDialog != null && isDungeonConfirmDialog(GameCanvas.currentDialog)) {
            dismissDungeonDialog(GameCanvas.currentDialog);
        }
        if (GameCanvas.subDialog != null && isDungeonConfirmDialog(GameCanvas.subDialog)) {
            dismissDungeonDialog(GameCanvas.subDialog);
        }
        dungeonRestoreCombat();
        trace("DUNGEON stopped (" + why + "): " + reason);
    }

    /** Transitions to DN_MANUAL_REVIEW while publishing dungeonWhy=0 (state-only manual review). */
    public static void dungeonManualReview(String reason) {
        dungeonStop(0, reason);
        dungeonState = DN_MANUAL_REVIEW;
        trace("DUNGEON manual review: " + reason);
    }

    /** Handles run failure, bounds consecutive failures, and triggers manual review when cap reached. */
    public static void dungeonFailRun(int why, String reason) {
        if (dungeonFails < DUNGEON_RUNS_MAX) {
            ++dungeonFails;
        }
        ++dungeonConsecutiveFails;
        dungeonAwaitingEntry = false;
        dungeonRestoreCombat();
        trace("DUNGEON run failed (" + why + "): " + reason + " (fails=" + dungeonFails
                + " consec=" + dungeonConsecutiveFails + ")");
        if (dungeonConsecutiveFails >= DN_CONSECUTIVE_FAIL_CAP) {
            dungeonStop(5, "consecutive failure cap of " + DN_CONSECUTIVE_FAIL_CAP + " reached");
            dungeonState = DN_MANUAL_REVIEW;
            return;
        }
        dungeonState = DN_FAILURE;
        dungeonWhy = why;
        dungeonWait = DN_BETWEEN_RUNS;
        dungeonTried = 0;
        dungeonStep = 0;
        dungeonNpcCu = -1;
        dungeonClearCandidate = false;
        dungeonMonstersCleared = false;
        dungeonMonstersZeroTicks = 0;
        dungeonNoTargetTicks = 0;
        dungeonRunTicks = 0;
        dungeonCombatEngaged = false;
        dungeonDiedInRun = false;
        dungeonManualEscaped = false;
    }

    /** Resets session-local transient menu and navigation state on world entry/disconnect. */
    public static void dungeonSessionReset() {
        dungeonCloseMenu();
        dungeonMenu = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        dungeonWait = 0;
        dungeonStep = 0;
        dungeonTried = 0;
        dungeonNavigating = false;
        if (dungeonState == DN_PREPARATION || dungeonState == DN_ROUTING) {
            dungeonState = DN_IDLE;
        }
        if (dungeonWasIn && (GameCanvas.loadmap == null || GameCanvas.loadmap.idMap != DUNGEON_MAP)) {
            dungeonWasIn = false;
            dungeonFailRun(8, "reconnect outside dungeon during active run");
        }
    }

    /**
     * One step of the trip, or nothing at all.
     */
    private static void dungeon() {
        try {
            if (!dungeonEnabled) {
                if (dungeonState != DN_OFF || dungeonMenuNpc != Integer.MIN_VALUE) {
                    dungeonCloseMenu();
                    dungeonReset();
                }
                return;
            }
            if (!inGame() || !sceneReady() || captcha()
                    || GameScreen.player == null || GameCanvas.loadmap == null) {
                return; // keep intent; loading screen is not a failure
            }

            int here = GameCanvas.loadmap.idMap;
            if (here != dungeonMapSeen) {
                dungeonMapSeen = here;
                dungeonWait = 12; // let the scene settle
                dungeonStallTicks = 0;
                dungeonLastX = Integer.MIN_VALUE;
                dungeonLastY = Integer.MIN_VALUE;
                if (dungeonState == DN_ROUTING || dungeonState == DN_GOTO_NPC) {
                    Player.isLockKey = false;
                    GameScreen.player.posTransRoad = null;
                }
            }

            if (dungeonInDungeon()) {
                dungeonWasIn = true;
                dungeonTried = 0;
                dungeonNavigating = false;
                dungeonAwaitingEntry = false;
                if (dungeonState != DN_COMBAT) {
                    dungeonState = DN_COMBAT;
                    dungeonWhy = 0;
                    dungeonRunTicks = 0;
                    dungeonClearCandidate = false;
                    dungeonMonstersCleared = false;
                    dungeonMonstersZeroTicks = 0;
                    dungeonNoTargetTicks = 0;
                    trace("DUNGEON entered map " + here);
                }

                // Check death state inside dungeon
                if (GameScreen.player != null && GameScreen.player.Action == 4) {
                    dungeonDiedInRun = true;
                    dungeonState = DN_DEATH;
                    return;
                }

                dungeonCombat();
                return;
            }

            // Map 48 exit evaluation
            if (dungeonWasIn) {
                dungeonWasIn = false;
                dungeonNavigating = false;

                boolean success = (here == DUNGEON_NPC_MAP)
                        && dungeonCombatEngaged
                        && !dungeonDiedInRun
                        && !dungeonManualEscaped
                        && alive()
                        && (dungeonClearCandidate || dungeonMonstersCleared);

                if (success) {
                    dungeonState = DN_COMPLETION_WAIT;
                    dungeonRestoreCombat();
                    dungeonDone();
                    return;
                } else {
                    int failWhy = 10;
                    String reason = "ambiguous exit from dungeon";
                    if (dungeonDiedInRun) {
                        failWhy = 7;
                        reason = "died in dungeon and returned to town";
                    } else if (dungeonManualEscaped) {
                        failWhy = 9;
                        reason = "manual escape or travel conflict during dungeon";
                    } else if (dungeonRunTicks > DN_MAX_RUN_TICKS) {
                        failWhy = 6;
                        reason = "dungeon run timed out";
                    } else if (!dungeonCombatEngaged) {
                        failWhy = 10;
                        reason = "exited dungeon without combat engagement";
                    } else if (!dungeonClearCandidate && !dungeonMonstersCleared) {
                        failWhy = 10;
                        reason = "exited dungeon without completion signals";
                    }
                    dungeonFailRun(failWhy, reason);
                    return;
                }
            }

            if (dungeonWait > 0 && dungeonState != DN_PREPARATION) {
                --dungeonWait;
                return;
            }

            switch (dungeonState) {
                case DN_OFF:
                    if (dungeonWhy == 0) {
                        dungeonState = DN_IDLE;
                    }
                    return;
                case DN_IDLE:
                    dungeonIdle();
                    return;
                case DN_ROUTING: // DN_GOTO_NPC
                    dungeonGotoNpc(here);
                    return;
                case DN_PREPARATION: // DN_INTERACT
                    dungeonInteract();
                    return;
                case DN_COMPLETION_WAIT: // DN_DONE
                    dungeonDone();
                    return;
                case DN_FAILURE:
                    if (dungeonMaxRuns != -1 && dungeonRuns >= dungeonMaxRuns) {
                        if (isDungeonUnscheduled()) {
                            dungeonStop(4, "run limit reached");
                            return;
                        }
                    }
                    dungeonState = DN_IDLE;
                    return;
                case DN_DEATH:
                    if (dungeonInDungeon() && alive()) {
                        dungeonState = DN_COMBAT;
                    }
                    return;
                case DN_MANUAL_REVIEW:
                    return;
                default:
                    dungeonStop(3, "unknown state " + dungeonState);
            }
        } catch (Throwable t) {
            trace("DUNGEON tick exception: " + t);
        }
    }

    /** Leaves IDLE when the run limit allows it and the schedule, if any, has come round. */
    private static void dungeonIdle() {
        dungeonIdle(System.currentTimeMillis());
    }

    public static void dungeonIdle(long epochMillis) {
        int dateKey = dungeonDateKeyUtc7(epochMillis);
        int min = dungeonMinuteOfDayUtc7(epochMillis);
        dungeonIdle(min, dateKey);
    }

    public static void dungeonIdle(int min, int dateKey) {
        if (isDungeonUnscheduled()) {
            if (dungeonMaxRuns != -1 && dungeonRuns >= dungeonMaxRuns) {
                dungeonStop(4, "run limit of " + dungeonMaxRuns + " already reached");
                return;
            }
        } else if (isDungeonScheduled()) {
            if (dateKey < 0 || min < 0) {
                return; // clock failure fails closed
            }
            if (dateKey != dungeonScheduleDateKey) {
                if (dungeonStartMin <= min && min < dungeonEndMin) {
                    dungeonScheduleDateKey = dateKey;
                    dungeonRuns = 0;
                    dungeonConsecutiveFails = 0;
                    trace("DUNGEON entered new scheduled UTC+7 day " + dateKey + "; reset daily quota");
                } else {
                    return; // outside window on new/un-stamped day; wait for window
                }
            } else {
                if (min < dungeonStartMin || min >= dungeonEndMin) {
                    return; // outside window today
                }
                if (dungeonMaxRuns != -1 && dungeonRuns >= dungeonMaxRuns) {
                    return; // quota for today reached, remain idle waiting for next day
                }
            }
        } else {
            // Neither exact unscheduled nor valid scheduled window: FAIL CLOSED
            return;
        }
        dungeonTripActive = true;
        dungeonState = DN_ROUTING;
        dungeonTried = 0;
        dungeonStep = 0;
        dungeonStallTicks = 0;
        dungeonNpcCu = -1;
        dungeonWhy = 0;
        dungeonAwaitingEntry = false;
        trace("DUNGEON starting a run (done=" + dungeonRuns + " max=" + dungeonMaxRuns + ")");
    }

    /** Walks to the guide and asks it. Autonomously routes to Map 1 when standing elsewhere. */
    private static void dungeonGotoNpc(int here) {
        if (navTarget >= 0 && !navDone) {
            if (dungeonNavigating) {
                dungeonNavigating = false;
            }
            if (dungeonWhy != 9) {
                dungeonWhy = 9;
                trace("DUNGEON yielding to manual travel (navTarget=" + navTarget + ")");
            }
            return;
        }

        if (here != DUNGEON_NPC_MAP) {
            int hop = mapNextHop(here, DUNGEON_NPC_MAP);
            if (hop < 0 || mapDistance(here, DUNGEON_NPC_MAP) < 0 || travelState == TV_BLOCKED) {
                dungeonStop(1, "no route from map " + here + " to dungeon NPC map " + DUNGEON_NPC_MAP);
                return;
            }
            dungeonNavigating = true;
            if (dungeonWhy != 0) {
                dungeonWhy = 0;
            }
            return;
        }

        // Authoritative arrival on Map 1
        dungeonNavigating = false;
        if (dungeonWhy != 0) {
            dungeonWhy = 0;
        }

        MainObject npc = dungeonNpc();
        int x = DUNGEON_NPC_X;
        int y = DUNGEON_NPC_Y;
        if (npc != null) {
            x = npc.x;
            y = npc.y;
        }
        if (npc == null || !dungeonNpcEligible(npc)) {
            if (GameScreen.player.x == dungeonLastX && GameScreen.player.y == dungeonLastY) {
                if (++dungeonStallTicks > DN_STALL_TICKS) {
                    dungeonStop(3, "stalled walking to the dungeon NPC on map " + here);
                    return;
                }
            } else {
                dungeonStallTicks = 0;
                dungeonLastX = GameScreen.player.x;
                dungeonLastY = GameScreen.player.y;
            }
            travelMove(here, x, y);
            return;
        }

        // Native interaction condition met: halt movement velocity before interaction
        Player.isLockKey = false;
        GameScreen.player.posTransRoad = null;
        GameScreen.player.toX = GameScreen.player.x;
        GameScreen.player.toY = GameScreen.player.y;
        GameScreen.player.vx = 0;
        GameScreen.player.vy = 0;
        try {
            GameScreen.player.resetAction();
        } catch (Throwable t) {
        }

        if (GameCanvas.currentDialog != null) {
            if (isDungeonConfirmDialog(GameCanvas.currentDialog)) {
                dismissDungeonDialog(GameCanvas.currentDialog);
                return;
            }
            if (isBlockingDialog(GameCanvas.currentDialog)) {
                if (++dungeonTried >= DN_MAX_TRIES) {
                    dungeonStop(5, "unrelated dialog blocking dungeon NPC interaction: " + clean(dialogText(GameCanvas.currentDialog)));
                    return;
                }
                dungeonWait = 10;
                return;
            }
        }
        if (dungeonClickNpc(npc)) {
            return;
        }
        if (++dungeonTried >= DN_MAX_TRIES) {
            dungeonStop(2, "the dungeon NPC would not take a click");
        }
    }

    /**
     * Drives the NPC's two menus and the confirmation dialog reactively:
     * 1. Check if already inside Map 48 -> DN_COMBAT.
     * 2. Unrelated dialog fail-closed safety check.
     * 3. Submenu check: If active menu in GameCanvas.menu2 contains "Ngã Tư", dispatch immediately via GameCanvas.menu2.commandPointer(2, 0).
     * 4. Giao tiếp check: If actionable "Giao tiếp" command in GameCanvas.menu2/GameCanvas.currentDialog, invoke native iCommand.a().
     * 5. Speech dialog check: If GameCanvas.currentDialog is active NPC story/speech dialog, advance it.
     * 6. Confirmation check: If valid confirmation dialog in GameCanvas.currentDialog, confirm it (optional; direct teleport also succeeds).
     * 7. Bounded retry and wait countdown: decrement dungeonWait, retry ask NPC if wait expires up to DN_MAX_TRIES.
     */
    private static void dungeonInteract() {
        if (dungeonInDungeon()) {
            dungeonAwaitingEntry = false;
            dungeonState = DN_COMBAT;
            return;
        }

        // 1. Reactive check: Valid confirmation dialog in GameCanvas.currentDialog or GameCanvas.subDialog (optional; direct teleport also succeeds)
        MainDialog confirmDlg = dungeonConfirmDialogTarget();
        if (confirmDlg != null) {
            trace("DUNGEON observed confirmation dialog: \"" + clean(dialogText(confirmDlg)) + "\"");
            if (!dungeonConfirmDialog(confirmDlg)) {
                if (++dungeonTried >= DN_MAX_TRIES) {
                    dungeonStop(2, "could not confirm dungeon entry dialog");
                    return;
                }
                dungeonWait = 10;
                return;
            }
            dungeonStep = 3;
            dungeonTried = 0;
            dungeonWait = 80;
            trace("DUNGEON confirmed entry dialog, waiting on teleport to Map 48");
            return;
        }

        // Unrelated modal safety check: fail closed on genuine blocking modals
        MainDialog blockingDlg = (GameCanvas.currentDialog != null && isBlockingDialog(GameCanvas.currentDialog)) ? GameCanvas.currentDialog : ((GameCanvas.subDialog != null && isBlockingDialog(GameCanvas.subDialog)) ? GameCanvas.subDialog : null);
        if (blockingDlg != null) {
            if (++dungeonTried >= DN_MAX_TRIES) {
                dungeonStop(5, "unrelated dialog blocking dungeon: " + clean(dialogText(blockingDlg)));
                return;
            }
            dungeonWait = 10;
            return;
        }

        // 2. Reactive check: Submenu ("Vào Ngã Tư Tử Thần")
        mVector activeItems = (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) ? getFrItems(GameCanvas.menu2) : dungeonMenuItems;
        if (activeItems == null && dungeonMenuItems != null) {
            activeItems = dungeonMenuItems;
        }
        int ngaTuRow = -1;
        if (activeItems != null) {
            for (int i = 0; i < activeItems.size(); i++) {
                Object entry = activeItems.elementAt(i);
                if (entry instanceof iCommand) {
                    iCommand btn = (iCommand) entry;
                    if (btn.caption != null) {
                        String label = norm(btn.caption);
                        if (label.indexOf("nga tu") >= 0 || label.indexOf("tu than") >= 0 || label.indexOf("vao nga tu") >= 0) {
                            ngaTuRow = i;
                            break;
                        }
                    }
                }
            }
        }
        if (ngaTuRow < 0 && dungeonMenu != null) {
            ngaTuRow = dungeonMenuPick("nga tu", null);
            if (ngaTuRow < 0) {
                ngaTuRow = dungeonMenuPick("tu than", null);
            }
            if (ngaTuRow < 0) {
                ngaTuRow = dungeonMenuPick("vao nga tu", null);
            }
        }

        if (ngaTuRow >= 0) {
            if (dungeonAwaitingEntry) {
                dungeonManualReview("server re-presented Ngã Tư submenu without entering dungeon (entry rejected or unmet requirement)");
                return;
            }
            int npc = dungeonMenuNpc;
            int menuId = dungeonMenuId;
            dungeonMenu = null;
            dungeonMenuItems = null;
            dungeonMenuNpc = Integer.MIN_VALUE;
            dungeonStep = 2;
            dungeonTried = 0;
            dungeonWait = 60;
            dungeonAwaitingEntry = true;
            trace("DUNGEON invoked native second-menu action for row " + ngaTuRow + ", waiting on confirmation dialog or teleport");

            if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                setFrIndex(GameCanvas.menu2, ngaTuRow);
                try {
                    GameCanvas.menu2.commandPointer(2, 0);
                } catch (Throwable t) {
                    dungeonStop(2, "native server menu action failed: " + t);
                }
            } else {
                try {
                    GlobalService.gI().Dynamic_Menu((short) npc, (byte) menuId, (byte) ngaTuRow);
                } catch (Throwable t) {
                    dungeonStop(2, "selecting dungeon row " + ngaTuRow + " failed");
                }
            }
            return;
        }

        // 3. Reactive check: Actionable "Giao tiếp" command
        iCommand giaoTiepCmd = null;
        int giaoTiepIndex = -1;
        if (activeItems != null) {
            for (int i = 0; i < activeItems.size(); i++) {
                Object entry = activeItems.elementAt(i);
                if (entry instanceof iCommand) {
                    iCommand btn = (iCommand) entry;
                    if (btn.caption != null) {
                        String label = norm(btn.caption);
                        if (label.indexOf("giao tiep") >= 0 && label.indexOf("giao dich") < 0) {
                            giaoTiepCmd = btn;
                            giaoTiepIndex = i;
                            break;
                        }
                    }
                }
            }
        }
        if (giaoTiepCmd == null && GameCanvas.currentDialog != null) {
            giaoTiepCmd = findGiaoTiepInDialog(GameCanvas.currentDialog);
        }
        int legacyPick = -1;
        if (giaoTiepCmd == null && dungeonMenu != null) {
            legacyPick = dungeonMenuPick("giao tiep", "giao dich");
        }

        if (dungeonStep == 0 && (giaoTiepCmd != null || legacyPick >= 0)) {
            if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu && giaoTiepIndex >= 0) {
                setFrIndex(GameCanvas.menu2, giaoTiepIndex);
            }
            dungeonStep = 1;
            dungeonTried = 0;
            dungeonWait = 60;
            dungeonMenu = null;
            dungeonMenuItems = null;
            dungeonMenuNpc = Integer.MIN_VALUE;
            trace("DUNGEON invoked native 'Giao tiếp' command, waiting on second menu");

            if (giaoTiepCmd != null) {
                giaoTiepCmd.perform();
            } else {
                if (!dungeonSelect(legacyPick)) {
                    dungeonStop(2, "selecting \"giao tiếp\" failed");
                }
            }
            return;
        }

        // 4. Reactive check: Relevant NPC / story speech dialog in GameCanvas.currentDialog
        if (GameCanvas.currentDialog != null && isNpcSpeechDialog(GameCanvas.currentDialog)) {
            dungeonStep = (dungeonStep < 1) ? 1 : dungeonStep;
            dungeonTried = 0;
            dungeonWait = 60;
            dungeonMenu = null;
            dungeonMenuItems = null;
            dungeonMenuNpc = Integer.MIN_VALUE;
            trace("DUNGEON advancing Pho Chi Huy speech dialog in GameCanvas.currentDialog");
            if (GameCanvas.currentDialog.right != null) {
                GameCanvas.currentDialog.right.perform();
            } else if (GameCanvas.currentDialog.left != null) {
                GameCanvas.currentDialog.left.perform();
            } else {
                GameCanvas.keyMyPressed[5] = true;
                GameCanvas.keyMyHold[5] = true;
            }
            return;
        }

        // 5. Cooldown / Wait Budget
        if (dungeonWait > 0) {
            --dungeonWait;
            return;
        }

        // 6. Bounded Retry
        if (dungeonAwaitingEntry) {
            if (++dungeonTried >= DN_MAX_TRIES) {
                dungeonStop(2, "dungeon interaction timed out waiting for confirmation or teleport after " + DN_MAX_TRIES + " tries");
                return;
            }
            dungeonWait = 60;
            return;
        }

        if (++dungeonTried >= DN_MAX_TRIES) {
            dungeonStop(2, "dungeon interaction timed out after " + DN_MAX_TRIES + " tries");
            return;
        }
        dungeonStep = 0;
        dungeonCloseMenu();
        dungeonMenu = null;
        dungeonMenuItems = null;
        dungeonMenuNpc = Integer.MIN_VALUE;
        dungeonAskNpc();
    }

    /**
     * Confirms the dungeon entry confirmation dialog ("Bạn có muốn vào Ngã tư tử thần không?").
     * Finds and activates the affirmative button ("Có", "Đồng ý", "OK", "Vào", "Chấp nhận", "Chọn").
     * Never activates negative buttons ("Không", "Hủy", "Bỏ qua").
     */
    private static boolean dungeonConfirmDialog(MainDialog dialog) {
        if (dialog == null) {
            return false;
        }
        try {
            // 1. Search buttons in MsgDialog.cmdList (command list)
            if (dialog instanceof MsgDialog) {
                mVector buttons = ((MsgDialog) dialog).cmdList;
                if (buttons != null && buttons.size() > 0) {
                    // Pass 1: explicit affirmative caption
                    for (int i = 0; i < buttons.size(); i++) {
                        Object entry = buttons.elementAt(i);
                        if (!(entry instanceof iCommand)) {
                            continue;
                        }
                        iCommand btn = (iCommand) entry;
                        String cap = norm(btn.caption).trim();
                        if (isAffirmativeCaption(cap)) {
                            trace("DUNGEON dialog confirming via MsgDialog.cmdList button[" + i + "]=\"" + clean(btn.caption) + "\"");
                            btn.perform();
                            return true;
                        }
                    }
                    // Pass 2: if 1 or 2 buttons, first button if not negative
                    if (buttons.size() <= 2) {
                        Object first = buttons.elementAt(0);
                        if (first instanceof iCommand) {
                            iCommand btn = (iCommand) first;
                            String cap = norm(btn.caption).trim();
                            if (!isNegativeCaption(cap)) {
                                trace("DUNGEON dialog confirming via MsgDialog.cmdList button[0]=\"" + clean(btn.caption) + "\"");
                                btn.perform();
                                return true;
                            }
                        }
                    }
                }
            } else if (dialog instanceof InputDialog) {
                try {
                    java.lang.reflect.Field fb = InputDialog.class.getDeclaredField("b");
                    fb.setAccessible(true);
                    Object btn = fb.get(dialog);
                    if (btn instanceof iCommand) {
                        iCommand b = (iCommand) btn;
                        String cap = (b.caption == null) ? "" : norm(b.caption).trim();
                        if (cap.length() > 0 && isAffirmativeCaption(cap) && !isNegativeCaption(cap)) {
                            trace("DUNGEON dialog confirming via InputDialog.b=\"" + clean(b.caption) + "\"");
                            b.perform();
                            return true;
                        } else {
                            trace("DUNGEON InputDialog.b caption blank, negative, or ambiguous: \"" + clean(b.caption) + "\"");
                            return false;
                        }
                    }
                } catch (Throwable t) {
                }
                return false;
            }

            // 2. Check softkeys Z (left) and ab (right)
            if (dialog.left != null) {
                String cap = norm(dialog.left.caption).trim();
                if (isAffirmativeCaption(cap) || (!isNegativeCaption(cap) && dialog.left.caption != null)) {
                    trace("DUNGEON dialog confirming via dialog.left=\"" + clean(dialog.left.caption) + "\"");
                    dialog.left.perform();
                    return true;
                }
            }
            if (dialog.right != null) {
                String cap = norm(dialog.right.caption).trim();
                if (isAffirmativeCaption(cap)) {
                    trace("DUNGEON dialog confirming via dialog.right=\"" + clean(dialog.right.caption) + "\"");
                    dialog.right.perform();
                    return true;
                }
            }

            // 3. Fallback: if dialog.left exists and no other option, press Z
            if (dialog.left != null) {
                trace("DUNGEON dialog confirming via fallback dialog.left");
                dialog.left.perform();
                return true;
            }
        } catch (Throwable t) {
            return false;
        }
        return false;
    }

    private static boolean isAffirmativeCaption(String cap) {
        if (cap == null || cap.length() == 0) {
            return false;
        }
        return cap.equals("co")
                || cap.equals("dong y")
                || cap.equals("ok")
                || cap.equals("vao")
                || cap.equals("chap nhan")
                || cap.equals("chon")
                || cap.indexOf("co") >= 0
                || cap.indexOf("dong y") >= 0
                || cap.indexOf("ok") >= 0
                || cap.indexOf("vao") >= 0;
    }

    private static boolean isNegativeCaption(String cap) {
        if (cap == null || cap.length() == 0) {
            return false;
        }
        return cap.equals("khong")
                || cap.equals("huy")
                || cap.equals("bo qua")
                || cap.equals("dong")
                || cap.indexOf("khong") >= 0
                || cap.indexOf("huy") >= 0
                || cap.indexOf("bo qua") >= 0;
    }

    private static boolean isDungeonConfirmDialog(MainDialog dialog) {
        if (dialog == null) {
            return false;
        }
        try {
            // A dialog with "Giao tiếp" is Pho Chi Huy's first dialog, never a confirmation dialog
            if (findGiaoTiepInDialog(dialog) != null) {
                return false;
            }
            // An active speech dialog for Pho Chi Huy is dialogue/story, not an entry confirmation
            if (isNpcSpeechDialog(dialog)) {
                return false;
            }
            String text = norm(dialogText(dialog));
            boolean hasDungeonId = text.indexOf("nga tu") >= 0 || text.indexOf("tu than") >= 0;
            boolean hasEntryIntent = text.indexOf("vao") >= 0;
            return hasDungeonId && hasEntryIntent;
        } catch (Throwable t) {
            return false;
        }
    }

    private static MainDialog dungeonConfirmDialogTarget() {
        if (dungeonStep != 2) {
            return null;
        }
        if (isDungeonConfirmDialog(GameCanvas.currentDialog)) {
            return GameCanvas.currentDialog;
        }
        if (isDungeonConfirmDialog(GameCanvas.subDialog)) {
            return GameCanvas.subDialog;
        }
        return null;
    }

    private static boolean isDangerousAffirmativeCaption(String cap) {
        if (cap == null || cap.length() == 0) {
            return false;
        }
        return cap.equals("co")
                || cap.equals("dong y")
                || cap.equals("vao")
                || cap.equals("chap nhan")
                || cap.equals("chon")
                || cap.indexOf("dong y") >= 0
                || cap.indexOf("chap nhan") >= 0;
    }

    /**
     * Determines whether an active dialog is a genuine blocking modal that prevents safe
     * automation interaction.
     *
     * Non-blocking broadcast announcements (created via GameCanvas.currentScreen(String) with single dismiss button
     * f == -1, d == null, or dismiss caption) do not prevent native NPC menu interaction.
     *
     * Choice modals (>= 2 buttons), text inputs (j[]), active callbacks (d != null / f >= 0),
     * affirmative action buttons, and unknown dialog frames return true (fail closed).
     *
     * The expected Dungeon confirmation dialog returns false so it is handled by the confirmation
     * step rather than treated as an unrelated blocker.
     */
    public static boolean isBlockingDialog(MainDialog dialog) {
        if (dialog == null) {
            return false;
        }
        if (dungeonStep == 2 && isDungeonConfirmDialog(dialog)) {
            return false;
        }
        if (isNpcSpeechDialog(dialog)) {
            return false;
        }
        if (!(dialog instanceof MsgDialog)) {
            return true;
        }
        MsgDialog ahDialog = (MsgDialog) dialog;
        mVector buttons = ahDialog.cmdList;
        if (buttons == null || buttons.size() == 0) {
            return false;
        }
        if (buttons.size() > 1) {
            return true;
        }
        Object entry = buttons.elementAt(0);
        if (!(entry instanceof iCommand)) {
            return true;
        }
        iCommand btn = (iCommand) entry;
        String cap = norm(btn.caption).trim();
        if (isDangerousAffirmativeCaption(cap)) {
            return true;
        }
        if (btn.action != null) {
            return true;
        }
        if (btn.Pointer != null) {
            return true;
        }
        if (btn.indexMenu != -1 || btn.subIndex != -1) {
            return true;
        }
        return false;
    }

    private static void dismissDungeonDialog(MainDialog dialog) {
        if (dialog == null) {
            return;
        }
        try {
            if (dialog instanceof MsgDialog) {
                mVector buttons = ((MsgDialog) dialog).cmdList;
                if (buttons != null) {
                    for (int i = 0; i < buttons.size(); i++) {
                        Object entry = buttons.elementAt(i);
                        if (entry instanceof iCommand) {
                            iCommand btn = (iCommand) entry;
                            String cap = norm(btn.caption).trim();
                            if (isNegativeCaption(cap)) {
                                trace("DUNGEON dismissing dialog via button \"" + clean(btn.caption) + "\"");
                                btn.perform();
                                return;
                            }
                        }
                    }
                }
            }
            if (dialog.right != null) {
                String cap = norm(dialog.right.caption).trim();
                if (isNegativeCaption(cap)) {
                    trace("DUNGEON dismissing dialog via dialog.right \"" + clean(dialog.right.caption) + "\"");
                    dialog.right.perform();
                    return;
                }
            }
            if (GameCanvas.currentDialog == dialog) {
                GameCanvas.currentDialog = null;
            } else if (GameCanvas.subDialog == dialog) {
                GameCanvas.subDialog = null;
            }
        } catch (Throwable t) {
            if (GameCanvas.currentDialog == dialog) {
                GameCanvas.currentDialog = null;
            } else if (GameCanvas.subDialog == dialog) {
                GameCanvas.subDialog = null;
            }
        }
    }

    /** Counts the run, resets failure streak, then either stops at the limit or goes round again. */
    public static void dungeonDone() {
        if (dungeonRuns < DUNGEON_RUNS_MAX) {
            ++dungeonRuns;
        }
        dungeonConsecutiveFails = 0;
        dungeonRestoreCombat();
        dungeonCombatEngaged = false;
        dungeonClearCandidate = false;
        dungeonMonstersCleared = false;
        dungeonMonstersZeroTicks = 0;
        dungeonNoTargetTicks = 0;
        dungeonRunTicks = 0;
        dungeonDiedInRun = false;
        dungeonManualEscaped = false;
        trace("DUNGEON run " + dungeonRuns + " complete");
        if (dungeonMaxRuns != -1 && dungeonRuns >= dungeonMaxRuns) {
            if (isDungeonUnscheduled()) {
                dungeonStop(4, "run limit of " + dungeonMaxRuns + " reached");
                return;
            }
            dungeonState = DN_IDLE;
            dungeonTripActive = false;
            dungeonWait = DN_BETWEEN_RUNS;
            trace("DUNGEON reached daily max runs of " + dungeonMaxRuns + " for UTC+7 day; waiting for next window");
            return;
        }
        dungeonState = DN_IDLE;
        dungeonTripActive = false;
        dungeonWait = DN_BETWEEN_RUNS;
        dungeonTried = 0;
        dungeonStep = 0;
        dungeonNpcCu = -1;
    }

    // ---- end DUNGEON ----------------------------------------------------------

    // ---- ATTACK ---------------------------------------------------------------
    //
    // Zeus does not pick targets or swing. It sets the client's own auto fields and lets
    // Player.autoItem() choose and bq's loop attack, which is why this module is small. What it adds
    // is the anchor: the spot the operator captured, written into Player.xBeginAutoFire/S so the client's
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
    /** Player.isAutoHPMP as it was before this module turned the native potion pump off. */
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
            boolean on = MsgDialog.Autobuff != null
                    && i < MsgDialog.Autobuff.length
                    && MsgDialog.Autobuff[i] != null
                    && MsgDialog.Autobuff[i].length >= 2
                    && MsgDialog.Autobuff[i][1] == 1;
            out.append(on ? '1' : '0');
        }
        return out.toString();
    }

    private static void attack() {
        try {
            if (dungeonEnabled && GameCanvas.loadmap != null && GameCanvas.loadmap.idMap == DUNGEON_MAP) {
                return;
            }
            if (atkMode == 0 || atkX < 0 || atkY < 0) {
                combatOff();
                return;
            }
            // Death releases the combat fields and nothing else: revive() runs from tick(), before
            // this, so it works whether or not a spot is armed. Bailing on ready() instead left
            // `combatOff()` unreached and the module went on claiming it owned those fields while
            // the character lay on the ground.
            if (GameScreen.player != null && GameScreen.player.Action == 4) {
                combatOff();
                return;
            }
            if (!gameReady() || GameScreen.player == null) {
                return;             // keep the intent; just do nothing this tick
            }
            // A different map is out of scope this round: the character stays put rather
            // than being walked somewhere by a travel table nobody has driven yet.
            if (GameCanvas.loadmap == null || GameCanvas.loadmap.idMap != atkMap) {
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
        if (GameScreen.player == null || GameScreen.player.Action != 4) {
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
                Item ticket = reviveTicket();
                if (ticket != null) {
                    // Opcode -30 with the client's own virtual NPC id for reviving on the spot.
                    GlobalService.gI().Dynamic_Menu((short) -51, (byte) 0, (byte) 0);
                    ++reviveTries;
                    ++reviveCount;
                    trace("REVIVE ticket id=" + ticket.Id + " try=" + reviveTries
                            + " dead=" + reviveDead);
                    return;
                }
                reviveNoTicket = true;
                note("Zeus: hết vé hồi sinh, tự về làng.");
                trace("REVIVE no ticket dead=" + reviveDead);
            }
            // Opcode 31: give up the corpse and wake in town.
            GlobalService.gI().gohome((byte) 0);
            ++reviveCount;
            trace("REVIVE town dead=" + reviveDead + " tries=" + reviveTries);
        } catch (Throwable t) {
            // A dead socket must not stall the tick; the next period tries again.
        }
    }

    /**
     * One line in the client's own message ticker.
     *
     * `GameCanvas.addInfoChar(String)` queues into `cn.k`, and `cf.c()` (cf.java:902) draws from that queue — which
     * `GameCanvas.login()` pumps every tick, whatever screen is up. `GameCanvas.game(String)` would have been the wrong
     * call: it writes the single `cn.r.G` slot, so the next message overwrites this one before it
     * has been read. It has one side effect: `GameCanvas.login` stamps `fu.MainMonster`, which the client's ten-minute
     * tip timer measures from (fu.java:296), so a notice postpones one tip. Cheap at this rate.
     */
    private static void note(String text) {
        try {
            GameCanvas.addInfoCharServer(text);
        } catch (Throwable t) {
            // A message nobody can see is not worth failing a revive over.
        }
    }

    /**
     * Closes whatever UI is blocking, the way the client's own Back does.
     *
     * NOT `GameCanvas.menu2 = null`, which is what the mod this was compared against does: `GameCanvas.menu2` is assigned
     * exactly once (fu.java:87) and `GameCanvas.login()` dereferences it bare every frame (fu.java:245, :279),
     * as does the rest of the client — nulling it is an NPE in the paint loop. `GameCanvas.currentDialog` and `GameCanvas.subDialog`
     * are different: the client nulls those itself (MsgDialog.java:364, InputDialog.java:36, fu.j()).
     *
     * The dialog is dropped, not answered. `medalDialog()` presses a button only after matching the
     * text; a dialog that happened to be up when the character fell could be anything, and pressing
     * its first button answers a question nobody read.
     */
    private static void clearBlockingUi() {
        try {
            if (GameCanvas.menu2 != null && GameCanvas.menu2.isShowMenu) {
                GameCanvas.menu2.doCloseMenu();
                GameCanvas.clearKeyHold();
            }
            GameCanvas.currentDialog = null;
            GameCanvas.subDialog = null;
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
    private static Item reviveTicket() {
        if (Item.VecInvetoryPlayer == null) {
            return null;
        }
        for (int index = 0; index < Item.VecInvetoryPlayer.size(); index++) {
            Object entry = Item.VecInvetoryPlayer.elementAt(index);
            if (!(entry instanceof Item)) {
                continue;
            }
            Item item = (Item) entry;
            if (item.itemName != null && norm(item.itemName).indexOf("hoi sinh tai cho") >= 0) {
                return item;
            }
        }
        return null;
    }

    /** Manhattan drift from the spot, which is what both KnightMod thresholds measure. */
    private static int drift() {
        return Math.abs(GameScreen.player.x - atkX) + Math.abs(GameScreen.player.y - atkY);
    }

    private static void fighting() {
        int limit = atkMode == 1 ? STAND_DRIFT : MOVE_DRIFT;
        if (drift() > limit) {
            combatOff();
            GameScreen.ObjFocus = null;
            atkState = TO_SPOT;
            return;
        }
        if (atkMode == 1) {
            // Pin all six position fields. This makes bc/bd zero, which turns the client's
            // periodic resync into the only way the server learns the position — a
            // deliberate consequence, docs/core/08 §4.1.
            GameScreen.player.x = atkX;
            GameScreen.player.y = atkY;
            GameScreen.player.toX = atkX;
            GameScreen.player.toY = atkY;
            GameScreen.player.vx = 0;
            GameScreen.player.vy = 0;
        }
        combatOn();
        skills();
    }

    /** Sets the client's own auto fields and anchors them on the captured spot. */
    private static void combatOn() {
        Player.isAutoFire = (byte) 1;
        Player.isAutoHPMP = true;
        // Not a counter: a one-shot "catch the nearest target" flag the client consumes.
        // Forcing it true every tick is how KnightMod keeps asking, and it is correct.
        Player.isCurAutoFire = true;
        Player.xBeginAutoFire = atkX;
        Player.yBeginAutofire = atkY;
        GameScreen.player.wFocus = atkRadius;
        if (!combatOwned) {
            combatOwned = true;
            // One pump, not two. The client's own gate is Player.isAutoHPMP; with both running, which
            // mechanism drank is unknowable. docs/core/08 §5.3 option (c).
            nativePotionWas = Player.isAutoHPMP;
            Player.isAutoHPMP = false;
            syncNativeSettings();
        }
    }

    /** Releases every field this module took, and only if it took them. */
    private static void combatOff() {
        if (!combatOwned) {
            return;
        }
        combatOwned = false;
        Player.isAutoFire = (byte) -1;
        Player.isAutoHPMP = false;
        Player.isCurAutoFire = false;
        GameScreen.ObjFocus = null;
        if (GameScreen.player != null) {
            GameScreen.player.wFocus = NATIVE_RADIUS;
        }
        Player.isAutoHPMP = nativePotionWas;
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
     * `MainRMS.setSaveAuto()` — it does not keep a parallel copy, because two copies of a setting is how nobody can
     * say which one collected an item or drank a potion.
     *
     * The thresholds are server state: change `MsgDialog.mHPMP[]` without this call and the next server push
     * overwrites it (docs/core/08 §5.2). The native menu only moves in tens, so the free threshold
     * is rounded for display while the mod itself uses the exact one.
     */
    private static void syncNativeSettings() {
        try {
            if (MsgDialog.mHPMP != null && MsgDialog.mHPMP.length >= 2) {
                MsgDialog.mHPMP[0] = round10(atkHpPct);
                MsgDialog.mHPMP[1] = round10(atkMpPct);
            }
            applyPickup();
            applyBuffs();
            MainRMS.setSaveAuto();
        } catch (Throwable t) {
            // MainRMS.setSaveAuto() swallows its own failures; this guards the array accesses around it.
        }
    }

    /**
     * Writes the pickup record exactly the way the game's own menu writes it.
     *
     * `MsgDialog.java:213` is the model: `Player.autoItem = new AutoGetItem((byte) rank, R[1], R[2])`, where option index 5 of
     * the client's own equipment list means "don't pick equipment" and is stored as −1. Reproducing
     * that call rather than assembling the bytes by hand matters, because `be`'s constructor swaps
     * its last two arguments (`be.java:12-16`: `a=by2; b=by4; c=by3`). So the menu's layout is
     * a = rank, c = MP/HP mode, b = gold mode — and that is the layout the collector's own filter
     * reads: `bq.java:541` gates equipment on `q.a`, and `bq.java:546-552` gates the two potion
     * kinds on `q.c` against `be.e`/`be.f`.
     *
     * The client is inconsistent about this and Zeus deliberately does not try to be smarter. Going
     * out, `MainRMS.setSaveAuto()` serialises `q.a, q.b, q.c` (co.java:118-124). Coming back, `co.java:52` rebuilds
     * `new AutoGetItem(o[4], o[5], o[6])`, which lands byte 5 in `c` and byte 6 in `b` — the reverse. So a
     * server echo of the settings packet swaps MP/HP with gold, and the client's own summary text
     * (co.java:59-61) is written for the echoed layout while its filter is written for the menu's.
     * One of the two is wrong in vanilla.
     *
     * Zeus therefore writes the menu's layout, which is the one the filter honours, and publishes
     * `Player.autoItem` read back live in the snapshot. If an echo ever does swap them, the panel shows MP/HP
     * and gold exchanged relative to what was configured, rather than the tool quietly claiming a
     * setting the client is not using.
     */
    private static void applyPickup() {
        // The client treats a null record as "collector off", so an all-off configuration is
        // expressed the same way rather than as three separate "don't" values.
        if (itemRank >= 5 && itemMpHp >= 3 && itemGold >= 1) {
            Player.autoItem = null;
            return;
        }
        int rank = itemRank < 5 ? itemRank : -1;
        Player.autoItem = new AutoGetItem((byte) rank, (byte) itemMpHp, (byte) itemGold);
    }

    /**
     * Turns the client's own buff slots on or off.
     *
     * The cast loop already exists at `bq.java:615-626`; it runs whenever `Player.IndexFire == 1` and uses the
     * same `bq.j` predicate skill rotation uses. So there is nothing to write here beyond the
     * flags. A slot the character has not learned is left alone: `MsgDialog.MaxSkillBuff` bounds the array and
     * `Player.mCurentLvSkill[skill] > 0` is what the native menu itself checks before enabling one.
     */
    private static void applyBuffs() {
        if (MsgDialog.Autobuff == null) {
            return;
        }
        int slots = Math.min(BUFF_SLOTS, Math.min(MsgDialog.MaxSkillBuff, MsgDialog.Autobuff.length));
        boolean any = false;
        for (int i = 0; i < slots; i++) {
            if (MsgDialog.Autobuff[i] == null || MsgDialog.Autobuff[i].length < 2) {
                continue;
            }
            boolean learned = Player.mCurentLvSkill != null
                    && MsgDialog.Autobuff[i][0] >= 0
                    && MsgDialog.Autobuff[i][0] < Player.mCurentLvSkill.length
                    && Player.mCurentLvSkill[MsgDialog.Autobuff[i][0]] > 0;
            boolean on = atkBuff[i] && learned;
            MsgDialog.Autobuff[i][1] = on ? 1 : 0;
            any |= on;
        }
        Player.IndexFire = (byte) (any ? 1 : 0);
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
            short[] path = GameCanvas.game.updateFindRoad(atkX / 24, atkY / 24, GameScreen.player.x / 24, GameScreen.player.y / 24, 500);
            if (path == null) {
                arrived();      // already in the destination cell
                return;
            }
            if (path.length > 500) {
                return;         // pathfinding failed; do not walk a rubbish route
            }
            GameScreen.player.posTransRoad = path;
            GameScreen.player.countAutoMove = 0;
            GameScreen.player.xStopMove = 0;
            GameScreen.player.yStopMove = 0;
            GameScreen.player.toX = GameScreen.player.x;
            GameScreen.player.toY = GameScreen.player.y;
            Player.isLockKey = true;
        } catch (Throwable t) {
            Player.isLockKey = false;
        }
    }

    /** Clears the movement lock. Leaving it set is how the operator loses manual control. */
    private static void arrived() {
        Player.isLockKey = false;
        GameScreen.player.posTransRoad = null;
        GameScreen.player.toX = GameScreen.player.x;
        GameScreen.player.toY = GameScreen.player.y;
        GameScreen.player.vx = 0;
        GameScreen.player.vy = 0;
        settleTicks = 15;
        atkState = SETTLE;
    }

    /**
     * Separates "no monsters here" from "monsters I cannot reach".
     *
     * PaintInfoGameScreen.isPaintInfoFocus is the client's own answer to "did the last scan find a target": false means the
     * radius is empty, true with a character that never enters combat means terrain. The two
     * call for different fixes, and KnightMod conflates them into one town-charm reflex.
     * This round reports the diagnosis and acts on neither, because acting means travelling.
     */
    private static void diagnose() {
        if (!combatOwned) {
            stuckKind = 0;
            return;
        }
        if (PaintInfoGameScreen.isPaintInfoFocus) {
            noTargetTicks = 0;
            noFightTicks = GameScreen.player.Action == 2 ? 0 : noFightTicks + 1;
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
            if (GameScreen.ObjFocus == null || GameScreen.ObjFocus.typeObject != 1 || GameScreen.ObjFocus.hp <= 0 || GameScreen.ObjFocus.Action == 4) {
                return;             // no live monster held: never swing at a corpse
            }
            // bq.j prints a message when ef == 0, which would spam the client's log every
            // tick, so that state is filtered before asking.
            if (GameScreen.player.typeMount == 0) {
                return;
            }
            HotKey[] page = Player.mhotkey == null ? null : Player.mhotkey[Player.levelTab];
            if (page == null) {
                return;
            }
            for (int slot = 0; slot < page.length; slot++) {
                HotKey entry = page[slot];
                if (entry == null || entry.type != 0) {
                    continue;       // b != 0 means the slot holds an item, not a skill
                }
                if (!GameScreen.player.setDelaySkill(entry.id, -1)) {
                    continue;
                }
                GameScreen.player.setActionHotKey(slot, false);
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
     * the bag by function code instead keeps working. Player.timeDelayPotion[L] is the shared item cooldown:
     * this module arms it after each send, exactly as the client's own three drink paths do
     * (bq.java:355-357, bq.java:1052-1055, fo.java:499-502). Each function code cools on
     * its own slot, so HP and MP never block each other.
     */
    private static void potions() {
        if ((!atkHpOn && !atkMpOn) || GameScreen.player == null || GameScreen.player.Action == 4 || Item.VecInvetoryPlayer == null) {
            return;
        }
        try {
            if (atkHpOn && GameScreen.player.maxHp > 0 && GameScreen.player.hp * 100 / GameScreen.player.maxHp < atkHpPct && drink(0)) {
                return;
            }
            if (atkMpOn && GameScreen.player.maxMp > 0 && GameScreen.player.mp * 100 / GameScreen.player.maxMp < atkMpPct) {
                drink(1);
            }
        } catch (Throwable t) {
            // An inventory being rebuilt mid-scan must not stall the tick.
        }
    }
    /** Sends the first bag potion of one function code, if its cooldown has expired. */
    private static boolean drink(int function) {
        if (Player.timeDelayPotion == null || function >= Player.timeDelayPotion.length || Player.timeDelayPotion[function] == null
                || Player.timeDelayPotion[function].value > 0) {
            return false;
        }
        for (int index = 0; index < Item.VecInvetoryPlayer.size(); index++) {
            Object entry = Item.VecInvetoryPlayer.elementAt(index);
            if (!(entry instanceof Item)) {
                continue;
            }
            Item item = (Item) entry;
            if (item.ItemCatagory != 4 || item.numPotion != function || item.numPotion <= 0) {
                continue;
            }
            GlobalService.gI().Use_Potion((short) item.Id);
            // Same arm as the client's own paths: 2000 ms real time, clocked by mSystem.currentTimeMillis().
            Player.timeDelayPotion[function].value = 2000;
            Player.timeDelayPotion[function].limit = 2000;
            Player.timeDelayPotion[function].timebegin = mSystem.currentTimeMillis();
            ++potionCount;
            return true;
        }
        return false;
    }

    // ---- ITEM -----------------------------------------------------------------
    //
    // The client already collects drops itself, gated on the `Player.autoItem` record and filtered by
    // `MainObject.ct` at `bq.java:530-558`. So this module does not pick anything up: it writes that
    // record (see `applyPickup`) and lets the collector work. An earlier revision had its own
    // loop, which meant two mechanisms racing — set "không nhặt" in the game's own menu and
    // Zeus kept collecting, with no way to tell which one had taken an item.
    //
    // What is left here is what the client has no automation for: riding, and pressing OK on
    // the server-worded material dialog.
    //
    // `MainObject.dH[]` is the material-box table, not a mount list. Mounts are the five template ids
    // {62..66} the client's own menu matches (`Menu2.java:591-614`).

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
     * (`Menu2.java:591-614`, `bg.java:80-88`), while the mod it shipped matched seven display strings —
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
        if (!mountOn || GameScreen.player == null || Item.VecInvetoryPlayer == null) {
            return;
        }
        if (GameScreen.player.typeMount != -1) {
            return;                 // already riding
        }
        Item pick = null;
        for (int index = 0; index < Item.VecInvetoryPlayer.size(); index++) {
            Object entry = Item.VecInvetoryPlayer.elementAt(index);
            if (!(entry instanceof Item)) {
                continue;
            }
            Item item = (Item) entry;
            if (item.ItemCatagory != 4 || item.Id < MOUNT_ID_MIN || item.Id > MOUNT_ID_MAX) {
                continue;
            }
            if (mountId == MOUNT_ANY) {
                pick = item;
                break;              // any will do: the first one found
            }
            if (item.Id == mountId) {
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
        GlobalService.gI().Use_Potion((short) pick.Id);
        trace("MOUNT sent id=" + pick.Id + " want=" + mountId);
    }

    /**
     * The mounts in the bag, as `id:name` pairs, so the tool can offer them by name.
     *
     * The names come from the server, per item (`Item.g`, assigned in `j`'s constructors), and there
     * is no id-to-name table anywhere in the client to read instead. So the honest list is the one
     * the bag actually holds: an operator carrying nothing sees nothing to choose, which is true,
     * rather than five invented labels.
     *
     * `:` separates the pair and `|` the entries, so both are stripped from a name — a server that
     * ships one in a display string would otherwise split the field.
     */
    private static String mountList() {
        if (Item.VecInvetoryPlayer == null) {
            return "";
        }
        StringBuffer out = new StringBuffer(64);
        for (int index = 0; index < Item.VecInvetoryPlayer.size(); index++) {
            Object entry = Item.VecInvetoryPlayer.elementAt(index);
            if (!(entry instanceof Item)) {
                continue;
            }
            Item item = (Item) entry;
            if (item.ItemCatagory != 4 || item.Id < MOUNT_ID_MIN || item.Id > MOUNT_ID_MAX) {
                continue;
            }
            if (out.length() > 0) {
                out.append('|');
            }
            out.append(item.Id).append(':').append(field(item.itemName));
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
            if (GameScreen.player != null && GameScreen.player.typeMount != -1) {
                GlobalService.gI().useMount((byte) -1);
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
     * The text is read from MainDialog.q, the wrapped body lines, and NOT from toString(): neither
     * MainDialog nor its parent cg overrides toString(), so it returns "MsgDialog@1a2b3c" and no wording
     * could ever match. The title lives in MsgDialog.r, which is private; q is package-private and
     * this class is in the same (default) package, so it needs no bytecode patch. Every MsgDialog
     * constructor fills q from the body text (MsgDialog.java:384,412,434,465,499,531,676,712,811),
     * so a dialog with a body always has it; the one path that can leave it null (MsgDialog.java:770)
     * is guarded below.
     *
     * Both the field and the wording are what KnightMod's own reconnect module reads
     * (modsrc3/MOD06.java:94, matching "nguyen lieu me day" or "me day" + "MainDialog duoc dong"),
     * which is what confirmed that the material close-drop notice is a server dialog and not
     * a client setting.
     */
    private static void medalDialog() {
        if (medalWait > 0) {
            --medalWait;
            return;
        }
        if (!itemMedal || GameCanvas.currentDialog == null) {
            return;
        }
        // The material walk is waiting for exactly this kind of dialog and needs to read it first.
        if (dropPhase == 2) {
            return;
        }
        try {
            String text = norm(dialogText(GameCanvas.currentDialog));
            // "Chức năng rớt ..." is the shared prefix of all six materials in both directions;
            // matching the ĐÓNG wording alone left the MỞ confirmation on screen, blocking.
            // Deliberately not a bare "nguyen lieu" match: df.u and df.gd are the crafting NPC's
            // prompts, which the operator opens on purpose.
            if (text.indexOf("chuc nang rot") < 0 && text.indexOf("nguyen lieu me day") < 0) {
                return;
            }
            medalWait = 15;         // KnightMod's delay before pressing
            pressOk(GameCanvas.currentDialog);
        } catch (Throwable t) {
            // A dialog whose text cannot be read is left alone.
        }
    }

    /**
     * Invokes the dialog's own OK button, the same call a tap on it makes.
     *
     * `MsgDialog.cmdList` is the dialog's button list and the patcher widens it to public for exactly this.
     * The earlier revision called `GameCanvas.currentDialog.b(0, 0)` instead — a pointer press at the top-left
     * corner, which lands on the button only by luck. Falls back to the first button when no
     * caption matches, because a dialog with buttons always has one that dismisses it.
     */
    private static void pressOk(MainDialog dialog) {
        if (!(dialog instanceof MsgDialog)) {
            return;
        }
        mVector buttons = ((MsgDialog) dialog).cmdList;
        if (buttons == null || buttons.size() <= 0) {
            return;
        }
        for (int i = 0; i < buttons.size(); i++) {
            Object entry = buttons.elementAt(i);
            if (!(entry instanceof iCommand)) {
                continue;
            }
            iCommand button = (iCommand) entry;
            String caption = norm(button.caption);
            if (caption.indexOf("ok") >= 0 || caption.indexOf("dong") >= 0) {
                button.perform();
                return;
            }
        }
        Object first = buttons.elementAt(0);
        if (first instanceof iCommand) {
            ((iCommand) first).perform();
        }
    }

    /** Joins one dialog's wrapped body lines and title/caption fields, or "" when it has none. */
    private static String dialogText(MainDialog dialog) {
        if (dialog == null) {
            return "";
        }
        StringBuffer out = new StringBuffer(64);
        if (dialog instanceof MsgDialog) {
            try {
                java.lang.reflect.Field fr = MsgDialog.class.getDeclaredField("nameShow");
                fr.setAccessible(true);
                Object vr = fr.get(dialog);
                if (vr instanceof String && ((String) vr).length() > 0) {
                    out.append((String) vr).append(' ');
                }
            } catch (Throwable t) {
            }
            try {
                java.lang.reflect.Field fs = MsgDialog.class.getDeclaredField("status");
                fs.setAccessible(true);
                Object vs = fs.get(dialog);
                if (vs instanceof String && ((String) vs).length() > 0) {
                    out.append((String) vs).append(' ');
                }
            } catch (Throwable t) {
            }
        } else if (dialog instanceof InputDialog) {
            try {
                java.lang.reflect.Field fn = InputDialog.class.getDeclaredField("name");
                fn.setAccessible(true);
                Object vn = fn.get(dialog);
                if (vn instanceof String && ((String) vn).length() > 0) {
                    out.append((String) vn).append(' ');
                }
            } catch (Throwable t) {
            }
        }
        try {
            java.lang.reflect.Field fStr = MainDialog.class.getDeclaredField("strinfo");
            fStr.setAccessible(true);
            String[] lines = (String[]) fStr.get(dialog);
            if (lines != null) {
                for (int i = 0; i < lines.length; i++) {
                    if (lines[i] != null) {
                        out.append(lines[i]).append(' ');
                    }
                }
            }
        } catch (Throwable t) {
        }
        return out.toString();
    }

    /** Lowercases and strips Vietnamese accents, so server wording matches either way. */
    private static String norm(String value) {
        if (value == null) {
            return "";
        }
        String source = "àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡ"
                + "ùúụủũưừứựửữỳýỵỷỹđ"
                + "ÀÁẠẢÃÂẦẤẬẨẪĂẰẮẶẲẴÈÉẸẺẼÊỀẾỆỂỄÌÍỊỈĨÒÓỌỎÕÔỒỐỘỔỖƠỜỚỢỞỠ"
                + "ÙÚỤỦŨƯỪỨỰỬỮỲÝỴỶỸĐ";
        String target = "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooo"
                + "uuuuuuuuuuuyyyyyd"
                + "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooo"
                + "uuuuuuuuuuuyyyyyd";
        String lower = value.toLowerCase();
        StringBuffer out = new StringBuffer(lower.length());
        for (int i = 0; i < lower.length(); i++) {
            char c = lower.charAt(i);
            int at = source.indexOf(c);
            if (at >= 0 && at < target.length()) {
                out.append(target.charAt(at));
            } else {
                out.append(c);
            }
        }
        return out.toString();
    }

    /** Centralized semantic normalization helper: lowercases, strips accents, collapses whitespace. */
    public static String normSemantic(String value) {
        if (value == null) {
            return "";
        }
        String stripped = norm(value).trim();
        StringBuffer out = new StringBuffer(stripped.length());
        boolean lastSpace = false;
        for (int i = 0; i < stripped.length(); i++) {
            char c = stripped.charAt(i);
            if (c <= ' ') {
                if (!lastSpace && out.length() > 0) {
                    out.append(' ');
                    lastSpace = true;
                }
            } else {
                out.append(c);
                lastSpace = false;
            }
        }
        return out.toString();
    }

    public static boolean isSemanticCharm(String name, String base, int desiredMode) {
        return isSemanticCharmText(normSemantic(name), desiredMode)
                || isSemanticCharmText(normSemantic(base), desiredMode);
    }

    public static boolean isSemanticCharmText(String s, int desiredMode) {
        if (s == null || s.length() == 0) {
            return false;
        }
        String spaced = " " + s + " ";
        if (desiredMode == 1) { // 3-leaf charm
            boolean has3Leaf = s.indexOf("co ba la") >= 0
                    || s.indexOf("co 3 la") >= 0
                    || s.indexOf("co 3la") >= 0
                    || spaced.indexOf(" 3 la ") >= 0
                    || spaced.indexOf(" ba la ") >= 0
                    || spaced.indexOf(" 3la ") >= 0;
            boolean has4Conflict = s.indexOf("bon la") >= 0
                    || s.indexOf("4 la") >= 0
                    || s.indexOf("4la") >= 0;
            return has3Leaf && !has4Conflict;
        } else if (desiredMode == 2) { // 4-leaf charm
            boolean has4Leaf = s.indexOf("co bon la") >= 0
                    || s.indexOf("co 4 la") >= 0
                    || s.indexOf("co 4la") >= 0
                    || spaced.indexOf(" 4 la ") >= 0
                    || spaced.indexOf(" bon la ") >= 0
                    || spaced.indexOf(" 4la ") >= 0;
            boolean has3Conflict = s.indexOf("ba la") >= 0
                    || s.indexOf("3 la") >= 0
                    || s.indexOf("3la") >= 0;
            return has4Leaf && !has3Conflict;
        }
        return false;
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

    /**
     * Canonical host guard for server connection and credential transmission.
     *
     * In managed mode (System.getProperty("zeus.server.host") is set), socket connection
     * and opcode-1 credential submission are allowed ONLY when:
     *   expected property == RMS selectedServerHost == mSystem.listServer[GameCanvas.IndexServer][1]
     *
     * In unmanaged/manual mode (property is null), official behavior is preserved (returns true).
     */
    public static boolean serverTargetSafe() {
        try {
            String expectedHost = System.getProperty("zeus.server.host");
            if (expectedHost == null) {
                // Unmanaged official / manual mode: preserve official behavior
                return true;
            }
            expectedHost = expectedHost.trim();
            if (expectedHost.length() == 0) {
                return false;
            }

            byte[] bytes = Model.CRes.loadRMS("selectedServerHost");
            if (bytes == null || bytes.length == 0) {
                return false;
            }
            String savedHost = new String(bytes, "UTF-8").trim();
            if (savedHost.length() == 0 || !expectedHost.equalsIgnoreCase(savedHost)) {
                return false;
            }

            int index = Main.GameCanvas.IndexServer;
            if (index < 0) {
                return false;
            }
            String[][] list = CLib.mSystem.listServer;
            if (list == null || index >= list.length) {
                return false;
            }
            String[] row = list[index];
            if (row == null || row.length < 2) {
                return false;
            }
            String currentHost = row[1];
            if (currentHost == null) {
                return false;
            }
            String trimmedCurrent = currentHost.trim();
            if (trimmedCurrent.length() == 0) {
                return false;
            }

            return expectedHost.equalsIgnoreCase(trimmedCurrent);
        } catch (Throwable t) {
            return false;
        }
    }
}
