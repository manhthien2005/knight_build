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
import GameObjects.Other_Players;
import InterfaceComponents.MsgDialog;
import InterfaceComponents.MainDialog;
import InterfaceComponents.InputDialog;
import InterfaceComponents.ChatTextField;
import InterfaceComponents.iCommand;
import InterfaceComponents.TabRebuildItem;
import InterfaceComponents.DataRebuildItem;
import InterfaceComponents.TabShopNew;
import InterfaceComponents.MainTabNew;
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
import CLib.Session_ME;
import net.Message;
import netcommand.Cmd_Message;
import netcommand.global.GlobalService;
import netcommand.global.GlobalLogicHandler;
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
import GameObjects.Other_Players;
import InterfaceComponents.MsgDialog;
import InterfaceComponents.MainDialog;
import InterfaceComponents.InputDialog;
import InterfaceComponents.ChatTextField;
import InterfaceComponents.iCommand;
import InterfaceComponents.TabRebuildItem;
import InterfaceComponents.DataRebuildItem;
import InterfaceComponents.TabShopNew;
import InterfaceComponents.MainTabNew;
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
import CLib.Session_ME;
import net.Message;
import netcommand.Cmd_Message;
import netcommand.global.GlobalService;
import netcommand.global.GlobalLogicHandler;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.Vector;

/**
 * Focused test suite for Safe Single-Item Enhancement Runtime Engine (ENHANCE-04).
 * Tests all locked contracts:
 *  - Target safety (wire identity, count, fingerprint, level)
 *  - Auto charm truth table (levels 0..14)
 *  - Payment preflight (Gold vs Gem mode)
 *  - Execute semantics (sent at most once, bounded attempts)
 *  - Result classification (success, protected failure, degraded failure, destroyed, ambiguous)
 *  - Authoritative before/after accounting
 *  - Highlighting logic
 */
public class EnhancementEngineTest {

    static Field f(Class<?> owner, String name) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    static void set(String name, Object value) throws Exception {
        f(Class.forName("Zeus"), name).set(null, value);
    }

    static Object get(String name) throws Exception {
        return f(Class.forName("Zeus"), name).get(null);
    }

    static void call(String name) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod(name);
        m.setAccessible(true);
        m.invoke(null);
    }

    @SuppressWarnings("unchecked")
    static Vector<Object> queue() throws Exception {
        Object link = Session_ME.gI();
        Field o = f(Session_ME.class, "sender");
        Object sender = o.get(link);
        Field a = f(sender.getClass(), "sendingMessage");
        return (Vector<Object>) a.get(sender);
    }

    static void clearQueue() throws Exception {
        queue().removeAllElements();
    }

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static MainItem makeItem(int id, int category, String name, String baseName, int level, int tier) {
        MainItem it = new MainItem();
        it.Id = id;
        it.ItemCatagory = category;
        it.itemName = name;
        it.itemNameExcludeLv = baseName;
        it.tier = (byte) level;
        it.colorNameItem = tier;
        it.numPotion = 1;
        it.IdTem = (short) 100;
        it.isLock = 1;
        it.imageId = 10;
        return it;
    }

    static MainItem makeCharm(int id, String name, String baseName, int charmType) {
        MainItem it = new MainItem();
        it.Id = id;
        it.ItemCatagory = 7; // category 7
        it.typeMaterial = 11; // charm family
        it.itemName = name;
        it.itemNameExcludeLv = baseName;
        it.tier = 0;
        it.colorNameItem = 0;
        it.numPotion = 10;
        return it;
    }

    static mVector bag(Item... items) {
        mVector v = new mVector("bag");
        for (int i = 0; i < items.length; i++) {
            v.addElement(items[i]);
        }
        return v;
    }

    public static void main(String[] args) throws Exception {
        System.out.println("=== EnhancementEngineTest ===");

        // Precondition setup
        clearQueue();
        if (GameScreen.player == null) {
            GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
        }

        // ---------------------------------------------------------------------
        // Test 1: Auto Charm Truth Table
        // ---------------------------------------------------------------------
        System.out.println("--- Test 1: Auto charm truth table ---");
        Method autoCharmMethod = Class.forName("Zeus").getDeclaredMethod("resolveAutoCharm", int.class);
        autoCharmMethod.setAccessible(true);

        for (int lv = 0; lv <= 5; lv++) {
            int charm = ((Integer) autoCharmMethod.invoke(null, lv)).intValue();
            check("level " + lv + " resolves no charm (0)", charm == 0);
        }
        for (int lv = 6; lv <= 10; lv++) {
            int charm = ((Integer) autoCharmMethod.invoke(null, lv)).intValue();
            check("level " + lv + " resolves Co 3 la (1)", charm == 1);
        }
        for (int lv = 11; lv <= 14; lv++) {
            int charm = ((Integer) autoCharmMethod.invoke(null, lv)).intValue();
            check("level " + lv + " resolves Co 4 la (2)", charm == 2);
        }

        // ---------------------------------------------------------------------
        // Test 2: Target Validation & Wire Uniqueness Rule
        // ---------------------------------------------------------------------
        System.out.println("--- Test 2: Target safety & wire uniqueness ---");
        Method validateTargetMethod = Class.forName("Zeus").getDeclaredMethod("validateEnhancementTarget");
        validateTargetMethod.setAccessible(true);

        // Subtest 2.1: 0 matching items -> ITEM_MISSING_OR_CHANGED
        Item.VecInvetoryPlayer = bag(makeItem(102, 3, "Kiem khac", "Kiem khac", 0, 1));
        set("enhTemplateId", 101);
        set("enhCategory", 3);
        set("enhBaseName", "Kiem ngan");
        set("enhTier", 2);
        set("enhExpectedLevel", 5);
        set("enhState", 3); // VALIDATING_TARGET

        validateTargetMethod.invoke(null);
        int state = ((Integer) get("enhState")).intValue();
        check("0 matching O+u items enters ITEM_MISSING_OR_CHANGED (20)", state == 20);

        // Subtest 2.2: 2 matching items with same O+u -> AMBIGUOUS_WIRE_TARGET
        MainItem swordA = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        MainItem swordB = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        Item.VecInvetoryPlayer = bag(swordA, swordB);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("2 matching O+u items enters AMBIGUOUS_WIRE_TARGET (21)", state == 21);

        // Subtest 2.3: Fingerprint mismatch (different base_name) -> ITEM_MISSING_OR_CHANGED
        MainItem swordWrongName = makeItem(101, 3, "Kiem dai +5", "Kiem dai", 5, 2);
        Item.VecInvetoryPlayer = bag(swordWrongName);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Fingerprint name mismatch enters ITEM_MISSING_OR_CHANGED", state == 20);

        // Subtest 2.4: Fingerprint mismatch (different tier) -> ITEM_MISSING_OR_CHANGED
        MainItem swordWrongTier = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 1);
        Item.VecInvetoryPlayer = bag(swordWrongTier);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Fingerprint tier mismatch enters ITEM_MISSING_OR_CHANGED", state == 20);

        // Subtest 2.5: Level mismatch -> ITEM_MISSING_OR_CHANGED
        MainItem swordWrongLevel = makeItem(101, 3, "Kiem ngan +4", "Kiem ngan", 4, 2);
        Item.VecInvetoryPlayer = bag(swordWrongLevel);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Current level mismatch enters ITEM_MISSING_OR_CHANGED", state == 20);

        // Subtest 2.6: Exactly 1 valid match -> proceeds to LOCATING_BLACKSMITH (4)
        MainItem swordValid = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        Item.VecInvetoryPlayer = bag(swordValid);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("1 exact match proceeds to LOCATING_BLACKSMITH (4)", state == 4);
        int activeSlot = ((Integer) get("enhActiveTargetSlot")).intValue();
        check("Active target slot recorded for highlight", activeSlot == 0);

        // ---------------------------------------------------------------------
        // Test 3: Charm Resolution
        // ---------------------------------------------------------------------
        System.out.println("--- Test 3: Charm resolution & safety ---");
        Method resolveCharmMethod = Class.forName("Zeus").getDeclaredMethod("resolveEnhancementCharm");
        resolveCharmMethod.setAccessible(true);

        // Subtest 3.1: Mode 0 requires no charm
        set("enhConfiguredCharmMode", 0);
        set("enhState", 8); // RESOLVING_CHARM
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        int resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Mode 0 proceeds to VERIFYING_RESOURCES with resolved_charm 0", state == 10 && resolvedCharm == 0);

        // Subtest 3.2: Mode 1 missing charm -> CHARM_MISSING (24)
        Item.VecInvetoryPlayer = bag(swordValid);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Mode 1 with missing charm enters CHARM_MISSING (24)", state == 24);

        // Subtest 3.3: Mode 1 with valid charm -> proceeds to INSERTING_CHARM (9)
        MainItem charm3 = makeCharm(501, "Cỏ 3 lá", "Cỏ 3 lá", 1);
        Item.VecInvetoryPlayer = bag(swordValid, charm3);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Mode 1 with valid charm proceeds to INSERTING_CHARM (9)", state == 9 && resolvedCharm == 1);

        // Subtest 3.4: Ambiguous charm candidates (multiple templates for same semantic type) -> AMBIGUOUS_CHARM (22)
        MainItem charm3_dup = makeCharm(502, "Cỏ 3 lá", "Cỏ 3 lá", 1);
        Item.VecInvetoryPlayer = bag(swordValid, charm3, charm3_dup);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Ambiguous charm templates enters AMBIGUOUS_CHARM (22)", state == 22);

        // Subtest 3.5: No silent fallback (Mode 2 when only Co 3 la is present) -> CHARM_MISSING
        Item.VecInvetoryPlayer = bag(swordValid, charm3);
        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Mode 2 cannot fall back to Co 3 la -> CHARM_MISSING (24)", state == 24);

        // Subtest 3.6: Explicit Cỏ bốn lá resolves when bag text uses Vietnamese word form
        MainItem charm4Word = makeCharm(777, "Cỏ bốn lá", "Cỏ bốn lá", 2);
        Item.VecInvetoryPlayer = bag(swordValid, charm4Word);
        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Explicit Cỏ bốn lá resolves when bag text uses Vietnamese word form", state == 9 && resolvedCharm == 2);

        // Subtest 3.7: Explicit Cỏ bốn lá resolves after normalized diacritics
        MainItem charm4NoDiacritics = makeCharm(778, "Co bon la", "Co bon la", 2);
        Item.VecInvetoryPlayer = bag(swordValid, charm4NoDiacritics);
        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Explicit Cỏ bốn lá resolves after normalized diacritics", state == 9 && resolvedCharm == 2);

        // Subtest 3.8: Explicit Cỏ 4 lá remains compatible
        MainItem charm4Digit = makeCharm(779, "Cỏ 4 lá", "Cỏ 4 lá", 2);
        Item.VecInvetoryPlayer = bag(swordValid, charm4Digit);
        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Explicit Cỏ 4 lá remains compatible", state == 9 && resolvedCharm == 2);

        // Subtest 3.9: Explicit Cỏ ba lá resolves
        MainItem charm3Word = makeCharm(780, "Cỏ ba lá", "Cỏ ba lá", 1);
        Item.VecInvetoryPlayer = bag(swordValid, charm3Word);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Explicit Cỏ ba lá resolves", state == 9 && resolvedCharm == 1);

        // Subtest 3.10: Unrelated similarly named items do not match
        MainItem unrelatedA = makeCharm(801, "Bánh bao", "Bánh bao", 0);
        MainItem unrelatedB = makeCharm(802, "Bình máu 4", "Bình máu 4", 0);
        MainItem unrelatedC = makeCharm(803, "Cỏ bốn góc", "Cỏ bốn góc", 0);
        Item.VecInvetoryPlayer = bag(swordValid, unrelatedA, unrelatedB, unrelatedC);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Unrelated items with ba do not match Mode 1 -> CHARM_MISSING (24)", state == 24);

        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Unrelated items with 4/bon do not match Mode 2 -> CHARM_MISSING (24)", state == 24);

        // Subtest 3.11: No hardcoded charm template ID is required for runtime matching
        MainItem customTemplateCharm = makeCharm(9999, "Cỏ bốn lá", "Cỏ bốn lá", 2);
        Item.VecInvetoryPlayer = bag(swordValid, customTemplateCharm);
        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        int selectedTemplateId = ((Integer) get("snapSelectedCharmTemplateId")).intValue();
        check("No hardcoded charm template ID is required (template 9999 resolved)", state == 9 && selectedTemplateId == 9999);

        // Subtest 3.12: Missing requested charm produces CHARM_MISSING before Opcode67
        clearQueue();
        Item.VecInvetoryPlayer = bag(swordValid);
        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Missing requested charm produces CHARM_MISSING (24) before Opcode67", state == 24 && queue().isEmpty());

        // Subtest 3.13: AUTO_POLICY remains unchanged
        check("Auto policy level 4 resolves Mode 0", ((Integer) autoCharmMethod.invoke(null, 4)).intValue() == 0);
        check("Auto policy level 7 resolves Mode 1", ((Integer) autoCharmMethod.invoke(null, 7)).intValue() == 1);
        check("Auto policy level 12 resolves Mode 2", ((Integer) autoCharmMethod.invoke(null, 12)).intValue() == 2);

        // Subtest 3.14: Java strict post-dispatch slot validation (no roaming remap)
        set("enhTemplateId", 101);
        set("enhCategory", 3);
        set("enhBaseName", "Kiem ngan");
        set("enhTier", 2);
        set("enhExpectedLevel", 5);
        set("enhCapturedSlot", 3); // Expected at slot 3
        Item.VecInvetoryPlayer = bag(swordValid); // Placed at slot 0
        set("enhState", 3);
        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Post-fence item slot mismatch strictly rejected with ITEM_MISSING_OR_CHANGED (20)", state == 20);
        set("enhCapturedSlot", -1); // Reset slot constraint


        // ---------------------------------------------------------------------
        // Test 4: Payment Preflight
        // ---------------------------------------------------------------------
        System.out.println("--- Test 4: Payment preflight ---");
        Method verifyResMethod = Class.forName("Zeus").getDeclaredMethod("verifyEnhancementResources");
        verifyResMethod.setAccessible(true);

        // Setup mock forge cost structures
        TabRebuildItem.dataRebuild = new DataRebuildItem[16];
        for (int i = 0; i < 16; i++) {
            TabRebuildItem.dataRebuild[i] = new DataRebuildItem();
            TabRebuildItem.dataRebuild[i].priceCoin = 50000; // quoted gold
            TabRebuildItem.dataRebuild[i].priceGold = 20;    // quoted gems
            TabRebuildItem.dataRebuild[i].mValue = new byte[]{2, 1, 0, 0}; // mandatory materials
        }
        TabRebuildItem.idMaterial = new short[]{301, 302, 303, 304}; // material template IDs
        TabRebuildItem.numMaterialInven = new int[]{5, 5, 5, 5}; // bag counts
        TabRebuildItem.mNameMaterial = new String[]{"Da", "Sat", "Dong", "Vang"};
        TabRebuildItem.itemRe = swordValid;

        // Subtest 4.1: Gold mode with sufficient gold but 0 gems -> SUCCESS (READY_FOR_ATTEMPT)
        GameScreen.player.coin = 100000; // gold
        GameScreen.player.gold = 0;      // gems
        set("enhPaymentType", 0);
        set("enhResolvedCharmMode", 0);
        set("enhState", 10);
        Item.VecInvetoryPlayer = bag(swordValid, makeItem(301, 7, "Da", "Da", 0, 0), makeItem(302, 7, "Sat", "Sat", 0, 0));

        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 0 succeeds with 0 gems if gold is sufficient", state == 11);

        // Subtest 4.2: Gold mode with insufficient gold -> INSUFFICIENT_GOLD (25)
        GameScreen.player.coin = 1000;
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 0 with low gold enters INSUFFICIENT_GOLD (25)", state == 25);

        // Subtest 4.3: Gem mode with sufficient gems but 0 gold -> SUCCESS (READY_FOR_ATTEMPT)
        GameScreen.player.coin = 0;
        GameScreen.player.gold = 50;
        set("enhPaymentType", 1);
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 1 succeeds with 0 gold if gems are sufficient", state == 11);

        // Subtest 4.4: Gem mode with insufficient gems -> INSUFFICIENT_GEMS (26)
        GameScreen.player.gold = 5;
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 1 with low gems enters INSUFFICIENT_GEMS (26)", state == 26);

        // Subtest 4.5: Missing mandatory material -> INSUFFICIENT_MATERIALS (27)
        GameScreen.player.gold = 50;
        TabRebuildItem.numMaterialInven[0] = 0; // missing material 0
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Missing mandatory material enters INSUFFICIENT_MATERIALS (27)", state == 27);
        TabRebuildItem.numMaterialInven[0] = 5; // restore

        // ---------------------------------------------------------------------
        // Test 5: Execution Semantics & Attempt Limits
        // ---------------------------------------------------------------------
        System.out.println("--- Test 5: Execution semantics & attempt limits ---");
        Method executeAttemptMethod = Class.forName("Zeus").getDeclaredMethod("executeEnhancementAttempt");
        executeAttemptMethod.setAccessible(true);

        clearQueue();
        set("enhState", 11); // READY_FOR_ATTEMPT
        set("enhAttemptCount", 0);
        set("enhMaxAttempts", 2);
        set("enhPaymentType", 0);

        // Attempt 1: should send Opcode 67 sub-action 2 exactly once
        executeAttemptMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        int attempts = ((Integer) get("enhAttemptCount")).intValue();
        Vector<Object> q = queue();
        check("Execute packet increments attempt count to 1", attempts == 1);
        check("State becomes ATTEMPTING / WAITING_RESULT", state == 12 || state == 13);
        check("Opcode 67 emitted exactly once", q.size() == 1);
        Message pkt = (Message) q.elementAt(0);
        check("Emitted packet opcode is 67", pkt.command == 67);

        // Subtest 5.2: Exceeding max_attempts enters ATTEMPT_LIMIT_REACHED (18)
        set("enhState", 11);
        set("enhAttemptCount", 2);
        set("enhMaxAttempts", 2);
        executeAttemptMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Reaching max_attempts blocks execution and enters ATTEMPT_LIMIT_REACHED (18)", state == 18);

        // ---------------------------------------------------------------------
        // Test 6: Result Classification & Authoritative Accounting
        // ---------------------------------------------------------------------
        System.out.println("--- Test 6: Result classification & accounting ---");
        Method settleMethod = Class.forName("Zeus").getDeclaredMethod("settleEnhancementResult");
        settleMethod.setAccessible(true);

        // Pre-execute snapshot values
        set("snapGoldBefore", 100000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{5L, 5L, 0L, 0L});
        set("snapCharmBefore", 2L);
        set("snapTargetLevelBefore", 5);
        set("snapSelectedCharmTemplateId", 501);
        set("enhResolvedCharmMode", 1);
        set("enhTargetLevel", 7);
        set("enhAttemptCount", 1);
        set("enhMaxAttempts", 10);
        set("enhActualGoldSpent", 0L);
        set("enhActualGemSpent", 0L);
        set("enhActualMaterialsSpent", new long[]{0L, 0L, 0L, 0L});
        set("enhActualCharmsSpent", 0L);

        // Simulate after-state: gold dropped by 50000, 1 charm consumed, level upgraded to 6
        GameScreen.player.coin = 50000;
        GameScreen.player.gold = 50;
        MainItem swordPlus6 = makeItem(101, 3, "Kiem ngan +6", "Kiem ngan", 6, 2);
        Item.VecInvetoryPlayer = bag(swordPlus6, makeItem(501, 7, "Cỏ 3 lá", "Cỏ 3 lá", 0, 0)); // 1 charm left
        TabRebuildItem.isNextRebuild = 3; // Success

        settleMethod.invoke(null);
        long actualGold = ((Long) get("enhActualGoldSpent")).longValue();
        long actualCharms = ((Long) get("enhActualCharmsSpent")).longValue();
        int currentLv = ((Integer) get("enhCurrentLevel")).intValue();
        check("Actual gold spent measured from live delta (50000)", actualGold == 50000);
        check("Actual charm spent measured from live delta (1)", actualCharms == 1);
        check("Authoritative level updated to 6", currentLv == 6);

        // Subtest 6.2: Protected failure (TabRebuildItem.isNextRebuild == 4, same level) -> FAILURE_PROTECTED
        TabRebuildItem.isNextRebuild = 4;
        set("snapTargetLevelBefore", 6);
        MainItem swordStill6 = makeItem(101, 3, "Kiem ngan +6", "Kiem ngan", 6, 2);
        Item.VecInvetoryPlayer = bag(swordStill6);
        settleMethod.invoke(null);
        String lastRes = (String) get("enhLastResult");
        check("TabRebuildItem.isNextRebuild == 4 with unchanged level classifies as FAILURE_PROTECTED", "FAILURE_PROTECTED".equals(lastRes));

        // Subtest 6.3: Degraded failure (TabRebuildItem.isNextRebuild == 4, dropped to level 5) -> FAILURE_DEGRADED
        set("snapTargetLevelBefore", 6);
        MainItem swordDegraded5 = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        Item.VecInvetoryPlayer = bag(swordDegraded5);
        settleMethod.invoke(null);
        lastRes = (String) get("enhLastResult");
        currentLv = ((Integer) get("enhCurrentLevel")).intValue();
        check("TabRebuildItem.isNextRebuild == 4 with lower level classifies as FAILURE_DEGRADED", "FAILURE_DEGRADED".equals(lastRes));
        check("Adopted authoritative lower level 5", currentLv == 5);

        // Subtest 6.4: Destruction (item missing) -> ITEM_DESTROYED (19)
        Item.VecInvetoryPlayer = bag(); // empty
        settleMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Missing item after attempt enters ITEM_DESTROYED (19)", state == 19);

        // Subtest 6.5: Target Reached (upgraded to target level 7)
        TabRebuildItem.isNextRebuild = 3;
        set("snapTargetLevelBefore", 6);
        MainItem swordTarget7 = makeItem(101, 3, "Kiem ngan +7", "Kiem ngan", 7, 2);
        Item.VecInvetoryPlayer = bag(swordTarget7);
        settleMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Reaching target level enters TARGET_REACHED (17)", state == 17);

        // ---------------------------------------------------------------------
        // Test 7: Restart / Manual Review Protection
        // ---------------------------------------------------------------------
        System.out.println("--- Test 7: Restart & manual review protection ---");
        Method recoverMethod = Class.forName("Zeus").getDeclaredMethod("recoverEnhancementSession");
        recoverMethod.setAccessible(true);

        // If in-flight execute was true when session reset/recovered:
        set("enhInFlightExecute", true);
        set("enhState", 12); // ATTEMPTING
        recoverMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Unsettled in-flight execute recovers safely into MANUAL_REVIEW_REQUIRED (33)", state == 33);
        boolean inFlight = ((Boolean) get("enhInFlightExecute")).booleanValue();
        check("In-flight flag cleared to prevent replay", !inFlight);

        // ---------------------------------------------------------------------
        // Test 8: Visual Highlighting
        // ---------------------------------------------------------------------
        System.out.println("--- Test 8: Target highlight ---");
        Method highlightMethod = Class.forName("Zeus").getDeclaredMethod("isEnhancementHighlightActive");
        highlightMethod.setAccessible(true);

        set("enhState", 4); // active state
        set("enhActiveTargetSlot", 2);
        boolean active = ((Boolean) highlightMethod.invoke(null)).booleanValue();
        check("Highlight is active for in-progress enhancement", active);

        set("enhState", 17); // TARGET_REACHED (terminal)
        active = ((Boolean) highlightMethod.invoke(null)).booleanValue();
        check("Highlight clears on terminal TARGET_REACHED", !active);

        // ---------------------------------------------------------------------
        // Test 9: Blacksmith Cross-Map Routing, Ownership & Error Taxonomy
        // ---------------------------------------------------------------------
        System.out.println("--- Test 9: Blacksmith routing & error taxonomy ---");
        Method enhanceMethod = Class.forName("Zeus").getDeclaredMethod("enhance");
        enhanceMethod.setAccessible(true);
        Method goalMethod = Class.forName("Zeus").getDeclaredMethod("goal");
        goalMethod.setAccessible(true);
        Method formatStatusMethod = Class.forName("Zeus").getDeclaredMethod("formatEnhancementStatusJson");
        formatStatusMethod.setAccessible(true);

        setupWorldState(44);
        clearQueue();

        // Subtest 9.1: Auto Farm conflict -> ENHANCEMENT_TRAVEL_CONFLICT (34), preserves atk.map/x/y
        set("atkMode", 1);
        set("atkMap", 44);
        set("atkX", 100);
        set("atkY", 200);
        set("navTarget", -1);
        set("navDone", true);
        set("enhState", 4); // LOCATING_BLACKSMITH
        set("enhAttemptCount", 0);
        set("enhLastResult", null);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Auto Farm conflict transitions to ENHANCEMENT_TRAVEL_CONFLICT (34)", state == 34);
        check("Durable atkMap preserved", ((Integer) get("atkMap")).intValue() == 44);
        check("Durable atkX preserved", ((Integer) get("atkX")).intValue() == 100);
        check("Durable atkY preserved", ((Integer) get("atkY")).intValue() == 200);
        check("No packet sent during conflict", queue().size() == 0);

        // Subtest 9.2: Manual Travel conflict -> ENHANCEMENT_TRAVEL_CONFLICT (34), preserves navTarget/navDone
        set("atkMode", 0);
        set("atkMap", -1);
        set("navTarget", 8);
        set("navDone", false);
        set("enhState", 4);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Manual Travel conflict transitions to ENHANCEMENT_TRAVEL_CONFLICT (34)", state == 34);
        check("Durable navTarget preserved", ((Integer) get("navTarget")).intValue() == 8);
        check("Durable navDone preserved", !((Boolean) get("navDone")).booleanValue());

        // Subtest 9.3: Unavailable BFS route -> BLACKSMITH_ROUTE_UNAVAILABLE (35)
        set("atkMode", 0);
        set("navTarget", -1);
        set("navDone", true);
        setupWorldState(9999); // completely disconnected / unknown map
        set("enhState", 4);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Unavailable BFS route transitions to BLACKSMITH_ROUTE_UNAVAILABLE (35)", state == 35);
        check("Temporary routing cleaned on terminal", !((Boolean) get("enhNavigating")).booleanValue());

        // Subtest 9.4: Map 44 -> Map 1 routing state progression without live enhancement
        setupWorldState(44); // Cây cầu ma ám
        set("enhState", 4); // LOCATING_BLACKSMITH
        set("enhAttemptCount", 0);
        set("enhLastResult", null);
        clearQueue();

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        boolean navigating = ((Boolean) get("enhNavigating")).booleanValue();
        int activeGoal = ((Integer) goalMethod.invoke(null)).intValue();
        check("On Map 44, enhancement activates temporary navigation", navigating);
        check("goal() directs navigation to Map 1", activeGoal == 1);
        check("Remains in LOCATING_BLACKSMITH (4) while cross-map routing", state == 4);
        check("Zero execute packets sent while routing", queue().size() == 0);

        // Character arrives on Map 1, but far from anchor
        setupWorldState(1);
        GameScreen.player.x = 100;
        GameScreen.player.y = 100;
        // Map 1 Pháp sư NPC present at anchor 324, 624
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Pháp sư", -36, 2, 324, 624));
        // Settle map
        Field mapStableField = Class.forName("Zeus").getDeclaredField("mapStableTicks");
        mapStableField.setAccessible(true);
        mapStableField.set(null, 15);
        Field stableMapField = Class.forName("Zeus").getDeclaredField("stableMapId");
        stableMapField.setAccessible(true);
        stableMapField.set(null, 1);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("On Map 1 with dist > 45, transitions to APPROACHING_BLACKSMITH (5)", state == 5);

        // Character approaches anchor (dist <= 45)
        GameScreen.player.x = 320;
        GameScreen.player.y = 624;
        clearQueue();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("When near Pháp sư, transitions to OPENING_FORGE (6)", state == 6);
        check("Emitted NPC interaction packet (not opcode 67)", queue().size() == 1);
        Message talkPkt = (Message) queue().elementAt(0);
        check("Interaction packet is not opcode 67", talkPkt.command != 67);

        // Subtest 9.5: Current Map 1 near-anchor fast path
        clearQueue();
        setupWorldState(1);
        GameScreen.player.x = 324;
        GameScreen.player.y = 624;
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Pháp sư", -36, 2, 324, 624));
        mapStableField.set(null, 15);
        stableMapField.set(null, 1);
        set("enhState", 4);
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Near-anchor fast path transitions directly from 4 to OPENING_FORGE (6)", state == 6);
        check("Fast path sends NPC interaction without cross-map routing", queue().size() == 1);

        // Subtest 9.6: Missing live Pháp sư after bounded scans
        setupWorldState(1);
        GameScreen.player.x = 324;
        GameScreen.player.y = 624;
        // cn.MainItem has NPC with cu=-36 but WRONG name / not Pháp sư
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Dan lang", -36, 2, 324, 624));
        set("enhState", 4);
        set("enhBlacksmithScanTicks", 0);
        mapStableField.set(null, 15);
        stableMapField.set(null, 1);

        // Scan past bound
        int maxScans = ((Integer) get("MAX_BLACKSMITH_SCANS")).intValue();
        for (int s = 0; s <= maxScans + 2; s++) {
            enhanceMethod.invoke(null);
        }
        state = ((Integer) get("enhState")).intValue();
        check("Missing live Pháp sư after bounded scans enters BLACKSMITH_NOT_FOUND (36)", state == 36);
        check("cu=-36 with wrong name was rejected (not accepted as sole identity)", state == 36);

        // Subtest 9.7: Forge open failure timeout
        set("enhState", 6); // OPENING_FORGE
        set("enhWait", 0);
        set("enhForgeOpenTries", 0);
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Pháp sư", -36, 2, 324, 624));
        // Retry until forge open fails
        for (int i = 0; i < 5; i++) {
            set("enhWait", 0);
            enhanceMethod.invoke(null);
        }
        state = ((Integer) get("enhState")).intValue();
        check("Forge open timeout enters FORGE_OPEN_FAILED (38)", state == 38);

        // Subtest 9.8: Local pre-execute failures leave structured server result unset
        int[] localErrorStates = new int[] { 34, 35, 36, 37, 38, 20, 21, 23, 24, 25, 26, 27 };
        for (int s : localErrorStates) {
            set("enhState", s);
            set("enhLastResult", null);
            set("enhAttemptCount", 0);
            String json = (String) formatStatusMethod.invoke(null);
            check("State " + s + " status JSON has last_result null", json.contains("\"last_result\": null"));
            check("State " + s + " status JSON has attempt_count 0", json.contains("\"attempt_count\": 0"));
        }

        // ---------------------------------------------------------------------
        // Test 10: Strict Blacksmith Identity & Menu Contract
        // ---------------------------------------------------------------------
        System.out.println("--- Test 10: Strict Blacksmith Identity & Menu Contract ---");
        Method findBsMethod = Class.forName("Zeus").getDeclaredMethod("findBlacksmithNpc");
        findBsMethod.setAccessible(true);
        setupWorldState(1);
        GameScreen.player.x = 300;
        GameScreen.player.y = 600;

        // 10.1: cv=2, name "Pháp sư" => eligible
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Pháp sư", -36, 2, 324, 624));
        MainObject res = (MainObject) findBsMethod.invoke(null);
        check("cv=2 with name 'Pháp sư' is eligible", res != null && "Pháp sư".equals(res.name));

        // 10.2: cv!=2, name "Pháp sư" => not eligible
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Pháp sư", -36, 1, 324, 624)); // cv = 1
        res = (MainObject) findBsMethod.invoke(null);
        check("cv!=2 with name 'Pháp sư' is NOT eligible", res == null);

        // 10.3: cv=2, name containing 'Cường hóa' but NOT 'Pháp sư' => NOT eligible
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Cường hóa", -36, 2, 324, 624));
        res = (MainObject) findBsMethod.invoke(null);
        check("cv=2 with name 'Cường hóa' but not 'Pháp sư' is NOT eligible", res == null);

        // 10.4: cu=-36 with non-Pháp-sư name => not eligible
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Thợ rèn", -36, 2, 324, 624));
        res = (MainObject) findBsMethod.invoke(null);
        check("cu=-36 with non-Pháp-sư name is NOT eligible", res == null);

        // 10.5: Pháp sư candidate with cu=-36 receives priority only after eligibility
        GameScreen.Vecplayers = new mVector("npcs");
        GameScreen.Vecplayers.addElement(makeNpc("Pháp sư tập sự", -10, 2, 305, 605)); // dist = 10
        GameScreen.Vecplayers.addElement(makeNpc("Pháp sư", -36, 2, 350, 650));         // dist = 100, but -10000 bonus
        res = (MainObject) findBsMethod.invoke(null);
        check("cu=-36 provides distance priority between valid Pháp sư candidates", res != null && res.ID == -36);

        // 10.6: Menu option 'Cường hóa' is deferred: not taken synchronously, allows native builder to finish
        Method serverMenuMethod = Class.forName("Zeus").getDeclaredMethod("serverMenu", mVector.class, int.class, int.class, String.class);
        serverMenuMethod.setAccessible(true);
        set("enhState", 6); // OPENING_FORGE
        clearQueue();
        mVector menuItems = new mVector("menu");
        iCommand opt1 = new iCommand("Nhiệm vụ", 0);
        iCommand opt2 = new iCommand("Cường hoá", 1);
        iCommand opt3 = new iCommand("Thoát", 2);
        menuItems.addElement(opt1);
        menuItems.addElement(opt2);
        menuItems.addElement(opt3);
        boolean menuTaken = ((Boolean) serverMenuMethod.invoke(null, menuItems, 10, -36, "Pháp sư")).booleanValue();
        check("serverMenu does NOT swallow menu synchronously (allows native builder to finish)", !menuTaken);
        check("serverMenu sends zero packets synchronously", queue().size() == 0);
        check("serverMenu captures pending option 1", ((Integer) get("enhPendingMenuOption")).intValue() == 1);
        check("serverMenu captures pending NPC -36", ((Integer) get("enhPendingMenuNpc")).intValue() == -36);
        check("serverMenu captures pending menu 10", ((Integer) get("enhPendingMenuId")).intValue() == 10);
        // Deferred dispatch occurs on subsequent game tick
        enhanceMethod.invoke(null);
        check("deferred selection dispatches exactly 1 packet on subsequent tick", queue().size() == 1);
        Message pkt10 = (Message) queue().elementAt(0);
        check("deferred selection opcode is -30 (not 67)", pkt10.command == (byte) -30);
        check("deferred selection clears pending menu NPC", ((Integer) get("enhPendingMenuNpc")).intValue() == Integer.MIN_VALUE);

        // 10.7: Focused wire test proving NPC=-36, menu=0, option=0 selects short-byte-byte overload via deferred dispatch
        clearQueue();
        set("enhState", 6);
        call("clearPendingForgeMenu");
        mVector liveMenuItems = new mVector("menu");
        liveMenuItems.addElement(new iCommand("Cường hóa", 0)); // option index 0
        boolean liveMenuTaken = ((Boolean) serverMenuMethod.invoke(null, liveMenuItems, 0, -36, "Pháp sư")).booleanValue();
        check("live-shape menu selection not swallowed synchronously", !liveMenuTaken);
        check("zero packets queued synchronously inside serverMenu", queue().size() == 0);
        enhanceMethod.invoke(null);
        check("exactly 1 packet queued for deferred menu selection", queue().size() == 1);
        Message wirePkt = (Message) queue().elementAt(0);
        check("menu-selection wire opcode is -30", wirePkt.command == (byte) -30);
        check("menu-selection produces zero Opcode 67 packets", wirePkt.command != (byte) 67);
        byte[] payload = wirePkt.getData();
        java.io.DataInputStream dis = new java.io.DataInputStream(new java.io.ByteArrayInputStream(payload));
        short wireNpc = dis.readShort();
        byte wireMenu = dis.readByte();
        byte wireOption = dis.readByte();
        check("payload field order preserves NPC short -36", wireNpc == (short) -36);
        check("payload field order preserves menu byte 0", wireMenu == (byte) 0);
        check("payload field order preserves option byte 0", wireOption == (byte) 0);

        // 10.8: Fail-closed & live shape verification for deferred serverMenu
        // Case A: Unrelated menu without "cuong hoa" must NOT be captured, and must send 0 packets
        clearQueue();
        set("enhState", 6);
        call("clearPendingForgeMenu");
        mVector unrelatedMenu = new mVector("menu");
        unrelatedMenu.addElement(new iCommand("Nhiệm vụ", 0));
        unrelatedMenu.addElement(new iCommand("Thoát", 1));
        boolean unrelatedTaken = ((Boolean) serverMenuMethod.invoke(null, unrelatedMenu, 0, -36, "Pháp sư")).booleanValue();
        check("unrelated menu is NOT taken (fail-closed, native-visible)", !unrelatedTaken);
        check("unrelated menu produces zero packets synchronously", queue().size() == 0);
        check("unrelated menu does not capture pending selection", ((Integer) get("enhPendingMenuNpc")).intValue() == Integer.MIN_VALUE);
        enhanceMethod.invoke(null);
        check("unrelated menu produces zero packets on subsequent tick", queue().size() == 0);

        // Case B: Full live-shape 18-item Pháp sư menu
        clearQueue();
        set("enhState", 6);
        call("clearPendingForgeMenu");
        mVector fullPhapSuMenu = new mVector("menu");
        fullPhapSuMenu.addElement(new iCommand("Cường hóa", 0));
        fullPhapSuMenu.addElement(new iCommand("Chuyển hóa trang bị", 1));
        fullPhapSuMenu.addElement(new iCommand("Shop nguyên liệu", 2));
        fullPhapSuMenu.addElement(new iCommand("Hướng dẫn Cường hóa", 3));
        fullPhapSuMenu.addElement(new iCommand("Hướng dẫn Chuyển hóa trang bị", 4));
        fullPhapSuMenu.addElement(new iCommand("Hợp thành", 5));
        fullPhapSuMenu.addElement(new iCommand("Hướng dẫn Hợp thành", 6));
        fullPhapSuMenu.addElement(new iCommand("Khảm ngọc", 7));
        fullPhapSuMenu.addElement(new iCommand("Hướng dẫn khảm ngọc", 8));
        fullPhapSuMenu.addElement(new iCommand("Hợp ngọc", 9));
        fullPhapSuMenu.addElement(new iCommand("Hướng dẫn hợp ngọc", 10));
        fullPhapSuMenu.addElement(new iCommand("Đục lỗ", 11));
        fullPhapSuMenu.addElement(new iCommand("Hợp nguyên liệu mề đay", 12));
        fullPhapSuMenu.addElement(new iCommand("Mề đay chiến binh", 13));
        fullPhapSuMenu.addElement(new iCommand("Mề đay pháp sư", 14));
        fullPhapSuMenu.addElement(new iCommand("Mề đay sát thủ", 15));
        fullPhapSuMenu.addElement(new iCommand("Mề đay xạ thủ", 16));
        fullPhapSuMenu.addElement(new iCommand("Nâng cấp mề đay", 17));
        boolean fullMenuTaken = ((Boolean) serverMenuMethod.invoke(null, fullPhapSuMenu, 0, -36, "Pháp sư")).booleanValue();
        check("live-shape 18-item menu not swallowed synchronously (native builder preserved)", !fullMenuTaken);
        check("live-shape 18-item menu sends 0 packets synchronously", queue().size() == 0);
        check("live-shape resolves Cường hóa index 0 from label, not fallback", ((Integer) get("enhPendingMenuOption")).intValue() == 0);

        // Duplicate delivery guard: second delivery of identical menu produces no duplicate selection
        boolean duplicateTaken = ((Boolean) serverMenuMethod.invoke(null, fullPhapSuMenu, 0, -36, "Pháp sư")).booleanValue();
        check("duplicate menu delivery not swallowed", !duplicateTaken);
        check("duplicate menu delivery sends zero packets", queue().size() == 0);

        // Later game tick dispatches verified selection exactly once
        enhanceMethod.invoke(null);
        check("live-shape 18-item menu sends exactly 1 packet on subsequent tick", queue().size() == 1);
        Message fullPkt = (Message) queue().elementAt(0);
        check("live-shape menu wire opcode is -30", fullPkt.command == (byte) -30);
        check("live-shape menu produces zero Opcode 67 packets", fullPkt.command != (byte) 67);
        byte[] fullPayload = fullPkt.getData();
        java.io.DataInputStream disFull = new java.io.DataInputStream(new java.io.ByteArrayInputStream(fullPayload));
        short fullNpc = disFull.readShort();
        byte fullMenuId = disFull.readByte();
        byte fullOption = disFull.readByte();
        check("live-shape menu packet NPC is -36", fullNpc == (short) -36);
        check("live-shape menu packet menu is 0", fullMenuId == (byte) 0);
        check("live-shape menu packet option is 0", fullOption == (byte) 0);

        // Subsequent game tick produces NO duplicate dispatch
        enhanceMethod.invoke(null);
        check("subsequent tick produces no duplicate packets", queue().size() == 1);

        // Simulated forge open progresses to DRY_RUN_COMPLETE under validation_only
        set("enhValidationOnly", true);
        GameCanvas.currentScreen = makeForgePopup();
        enhanceMethod.invoke(null);
        check("after simulated forge open, state progresses to DRY_RUN_COMPLETE (39)", ((Integer) get("enhState")).intValue() == 39);
        check("zero Opcode 67 packets emitted on forge open", queue().size() == 1); // still only the single -30 packet

        // Cancellation / reset clears pending menu state
        set("enhPendingMenuNpc", -36);
        set("enhPendingMenuId", 0);
        set("enhPendingMenuOption", 0);
        call("clearPendingForgeMenu");
        check("clearPendingForgeMenu clears pending NPC", ((Integer) get("enhPendingMenuNpc")).intValue() == Integer.MIN_VALUE);


        // ---------------------------------------------------------------------
        // Test 11: Deterministic Pre-Opcode-67 Validation Interlock & Guard
        // ---------------------------------------------------------------------
        System.out.println("--- Test 11: Deterministic Dry-Run Interlock & Structural Opcode 67 Guard ---");
        // Subtest 11.1: Forge-ready + validationOnly=true transitions directly to DRY_RUN_COMPLETE (39)
        setupWorldState(1);
        set("enhState", 6); // OPENING_FORGE
        set("enhValidationOnly", true);
        clearQueue();
        GameCanvas.currentScreen = makeForgePopup();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Forge-ready with validationOnly=true transitions to DRY_RUN_COMPLETE (39)", state == 39);
        check("State 39 is classified as terminal", ((Boolean) Class.forName("Zeus").getDeclaredMethod("isEnhancementStateTerminal", int.class).invoke(null, 39)).booleanValue());
        check("State 39 name is DRY_RUN_COMPLETE", "DRY_RUN_COMPLETE".equals(Class.forName("Zeus").getDeclaredMethod("getEnhancementStateName", int.class).invoke(null, 39)));
        check("Validation-only at forge-ready sends ZERO Opcode 67 packets", queue().size() == 0);

        // Subtest 11.2: Forge-ready + validationOnly=false still transitions to INSERTING_TARGET (7)
        setupWorldState(1);
        set("enhState", 6); // OPENING_FORGE
        set("enhValidationOnly", false);
        clearQueue();
        GameCanvas.currentScreen = makeForgePopup();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Forge-ready with validationOnly=false transitions to INSERTING_TARGET (7)", state == 7);

        // Subtest 11.3: Telemetry contract for DRY_RUN_COMPLETE
        set("enhState", 39);
        set("enhValidationOnly", true);
        set("enhErrorCode", null);
        set("enhErrorMessage", null);
        set("enhAttemptCount", 0);
        set("enhLastResult", null);
        String dryRunJson = (String) formatStatusMethod.invoke(null);
        check("Telemetry contains state DRY_RUN_COMPLETE", dryRunJson.indexOf("\"state\": \"DRY_RUN_COMPLETE\"") >= 0);
        check("Telemetry contains attempt_count 0", dryRunJson.indexOf("\"attempt_count\": 0") >= 0);
        check("Telemetry contains last_result null", dryRunJson.indexOf("\"last_result\": null") >= 0);
        check("Telemetry contains validation_only true", dryRunJson.indexOf("\"validation_only\": true") >= 0);
        check("Telemetry has no error_code for DRY_RUN_COMPLETE", dryRunJson.indexOf("\"error_code\"") < 0);

        // Subtest 11.4: Structural Guard at State 7 (INSERTING_TARGET)
        setupWorldState(1);
        set("enhState", 7);
        set("enhValidationOnly", true);
        clearQueue();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("State 7 structural guard intercepts and enters DRY_RUN_COMPLETE (39)", state == 39);
        check("State 7 structural guard sends zero packets", queue().size() == 0);

        // Subtest 11.5: Structural Guard at State 9 (INSERTING_CHARM)
        setupWorldState(1);
        set("enhState", 9);
        set("enhValidationOnly", true);
        clearQueue();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("State 9 structural guard intercepts and enters DRY_RUN_COMPLETE (39)", state == 39);
        check("State 9 structural guard sends zero packets", queue().size() == 0);

        // Subtest 11.6: Structural Guard at executeEnhancementAttempt
        setupWorldState(1);
        set("enhState", 11);
        set("enhValidationOnly", true);
        clearQueue();
        executeAttemptMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Execute structural guard intercepts and enters DRY_RUN_COMPLETE (39)", state == 39);
        check("Execute structural guard sends zero packets", queue().size() == 0);

        // Subtest 11.7: Request scoping and enhanceReset
        Method resetMethod = Class.forName("Zeus").getDeclaredMethod("enhanceReset");
        resetMethod.setAccessible(true);
        set("enhValidationOnly", true);
        resetMethod.invoke(null);
        boolean valOnlyAfterReset = ((Boolean) get("enhValidationOnly")).booleanValue();
        check("enhanceReset clears validationOnly flag", !valOnlyAfterReset);

        // ---------------------------------------------------------------------
        // Test 12: Interstitial Dialog Recovery & Native Dismissal
        // ---------------------------------------------------------------------
        System.out.println("--- Test 12: Interstitial dialog recovery & native dismissal ---");
        Method dialogRecoveryMethod = Class.forName("Zeus").getDeclaredMethod("dialogRecovery");
        dialogRecoveryMethod.setAccessible(true);

        if (GameCanvas.menu2 == null) {
            GameCanvas.menu2 = new Menu2();
        }

        // 12.1: Unrelated GameCanvas.menu2 notification (Menu2.isNPCMenu=1) while unowned is NOT dismissed
        GameCanvas.currentDialog = null;
        GameCanvas.menu2.isShowMenu = true;
        Menu2.isNPCMenu = 1; // notification mode
        setupWorldState(1);
        GameCanvas.currentScreen = GameCanvas.game;
        dialogRecoveryMethod.invoke(null);
        check("Unrelated GameCanvas.menu2 notification (Menu2.isNPCMenu=1) is NOT dismissed when unowned", GameCanvas.menu2.isShowMenu);
        GameCanvas.menu2.isShowMenu = false; // reset

        // 12.2: GameCanvas.menu2 menu mode (Menu2.isNPCMenu=0) is NOT auto-dismissed (fail-closed)
        GameCanvas.currentDialog = null;
        GameCanvas.menu2.isShowMenu = true;
        Menu2.isNPCMenu = 0; // menu mode
        dialogRecoveryMethod.invoke(null);
        check("GameCanvas.menu2 menu mode (Menu2.isNPCMenu=0) fails closed and remains open", GameCanvas.menu2.isShowMenu);
        GameCanvas.menu2.isShowMenu = false; // reset

        // 12.3: Unrelated ev tab-screen (e.g. inventory) is NOT force-closed by cleanEnhancementRouting
        TabScreenNew invScreen = makeInventoryPopup();
        GameCanvas.currentScreen = invScreen;
        call("cleanEnhancementRouting");
        check("Unrelated ev screen is NOT force-closed to GameCanvas.game", GameCanvas.currentScreen == invScreen);
        check("GameCanvas.currentScreen remains invScreen and not GameCanvas.game", GameCanvas.currentScreen != GameCanvas.game);

        // 12.4: Unowned forge screen is NOT closed by cleanEnhancementRouting
        TabScreenNew unownedForge = makeForgePopup();
        GameCanvas.currentScreen = unownedForge;
        set("enhOwnsForgeScreen", false);
        call("cleanEnhancementRouting");
        check("Unowned forge screen is NOT closed by cleanEnhancementRouting", GameCanvas.currentScreen == unownedForge);

        // 12.5: Enhancement-owned GameCanvas.menu2 is dismissed via native fr.f() when owned
        TabScreenNew ownedForge = makeForgePopup();
        GameCanvas.currentScreen = ownedForge;
        GameCanvas.menu2.isShowMenu = true;
        Menu2.isNPCMenu = 1;
        set("enhOwnsForgeScreen", true);
        set("enhOwnsResultDialog", true);
        dialogRecoveryMethod.invoke(null);
        check("Enhancement-owned GameCanvas.menu2 is dismissed by dialogRecovery", !GameCanvas.menu2.isShowMenu);
        check("enhOwnsResultDialog is cleared after dismissal", !((Boolean) get("enhOwnsResultDialog")).booleanValue());

        // 12.6: Zero Opcode 67 packets emitted on dialog dismissal
        clearQueue();
        set("enhOwnsForgeScreen", true);
        set("enhOwnsResultDialog", true);
        GameCanvas.menu2.isShowMenu = true;
        Menu2.isNPCMenu = 1;
        dialogRecoveryMethod.invoke(null);
        check("Dialog dismissal emits zero packets", queue().size() == 0);

        // 12.7: Enhancement-owned forge screen cleanup restores GameCanvas.currentScreen to GameCanvas.game
        GameCanvas.currentScreen = ownedForge;
        set("enhOwnsForgeScreen", true);
        call("cleanEnhancementRouting");
        check("Enhancement-owned forge screen is restored to GameCanvas.game", GameCanvas.currentScreen == GameCanvas.game);
        check("enhOwnsForgeScreen cleared after cleanup", !((Boolean) get("enhOwnsForgeScreen")).booleanValue());

        // 12.8: Dry-run validation never sets enhOwnsResultDialog
        call("enhanceReset");
        setupWorldState(1);
        set("enhValidationOnly", true);
        set("enhState", 6); // OPENING_FORGE
        GameCanvas.currentScreen = makeForgePopup();
        enhanceMethod.invoke(null);
        check("Dry run terminates in DRY_RUN_COMPLETE (39)", ((Integer) get("enhState")).intValue() == 39);
        check("Dry run does NOT set enhOwnsResultDialog", !((Boolean) get("enhOwnsResultDialog")).booleanValue());
        check("Dry run clears enhOwnsForgeScreen", !((Boolean) get("enhOwnsForgeScreen")).booleanValue());

        // 12.9: Real execute lifecycle sets enhOwnsResultDialog and clears on reset
        call("enhanceReset");
        setupWorldState(1);
        set("enhValidationOnly", false);
        set("enhState", 13); // WAITING_RESULT
        set("enhInFlightExecute", true);
        set("enhOwnsForgeScreen", true);
        TabRebuildItem.isNextRebuild = 3; // server SUCCESS result
        enhanceMethod.invoke(null);
        check("Real result lifecycle sets enhOwnsResultDialog", ((Boolean) get("enhOwnsResultDialog")).booleanValue());
        call("enhanceReset");
        check("enhanceReset clears enhOwnsResultDialog", !((Boolean) get("enhOwnsResultDialog")).booleanValue());
        check("enhanceReset clears enhOwnsForgeScreen", !((Boolean) get("enhOwnsForgeScreen")).booleanValue());

        // 12.10: Between attempts clean state
        check("Target slot cleared (-1)", ((Integer) get("enhActiveTargetSlot")).intValue() == -1);
        check("Engine idle (0)", ((Integer) get("enhState")).intValue() == 0);
        check("Dialog ownership false", !((Boolean) get("enhOwnsResultDialog")).booleanValue());
        check("Forge ownership false", !((Boolean) get("enhOwnsForgeScreen")).booleanValue());

        // ---------------------------------------------------------------------
        // Test 13: Post-Execute Result Wait & Non-Replay State Reconciliation
        // ---------------------------------------------------------------------
        System.out.println("--- Test 13: Result wait window & conservative state reconciliation ---");

        // 13.1: Normal TabRebuildItem.isNextRebuild == 3 arriving after >1.5s within wait window is captured as RESULT_CODE_SUCCESS
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhState", 13); // WAITING_RESULT
        set("enhInFlightExecute", true);
        set("enhWait", 100); // 100 ticks remaining (>1.5s)
        set("enhResultDeadline", System.currentTimeMillis() + 4000L); // deadline in future
        TabRebuildItem.isNextRebuild = 3; // server result arrives
        enhanceMethod.invoke(null);
        check("13.1: Normal TabRebuildItem.isNextRebuild == 3 captures SUCCESS", "SUCCESS".equals(get("enhLastResult")));
        check("13.1: Settlement provenance is RESULT_CODE_SUCCESS", "RESULT_CODE_SUCCESS".equals(get("enhSettlementProvenance")));
        check("13.1: Settlement source is RESULT_CODE", "RESULT_CODE".equals(get("enhSettlementSource")));
        check("13.1: Result code is 3", ((Integer) get("enhResultCode")).intValue() == 3);

        // 13.2: Native result delay around 3.7 seconds does not cause premature RESULT_AMBIGUOUS
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 150); // still waiting
        set("enhResultDeadline", System.currentTimeMillis() + 5000L); // well beyond 3.7s
        enhanceMethod.invoke(null);
        check("13.2: Engine remains in WAITING_RESULT (13) during 3.7s native delay", ((Integer) get("enhState")).intValue() == 13);
        check("13.2: No premature RESULT_AMBIGUOUS", get("enhErrorCode") == null);

        // 13.3 & 13.4: Exactly one Opcode 67 execute is sent, zero retries
        call("enhanceReset");
        setupWorldState(1);
        clearQueue();
        set("enhState", 11); // READY_FOR_ATTEMPT
        set("enhAttemptCount", 0);
        set("enhMaxAttempts", 1);
        set("enhExpectedLevel", 0);
        set("enhTargetLevel", 1);
        set("enhTemplateId", 67);
        set("enhCategory", 3);
        set("enhBaseName", "Kiem tap");
        set("enhTier", 1);
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap", "Kiem tap", 0, 1));
        if (TabRebuildItem.dataRebuild == null) TabRebuildItem.dataRebuild = new DataRebuildItem[16];
        TabRebuildItem.dataRebuild[0] = new DataRebuildItem();
        TabRebuildItem.dataRebuild[0].priceCoin = 3000;
        TabRebuildItem.dataRebuild[0].priceGold = 0;
        TabRebuildItem.dataRebuild[0].mValue = new byte[]{1, 1, 0, 0};
        executeAttemptMethod.invoke(null);
        int queuedPackets = queue().size();
        check("13.3: Exactly one Opcode 67 packet sent on execute", queuedPackets == 1);
        set("enhWait", 50);
        set("enhResultDeadline", System.currentTimeMillis() + 2000L);
        enhanceMethod.invoke(null);
        check("13.4: Zero execute retries during result wait", queue().size() == 1);

        // 13.5: Normal TabRebuildItem.isNextRebuild == 4 (failure) remains correctly classified
        call("enhanceReset");
        setupWorldState(1);
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap", "Kiem tap", 0, 1));
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("snapTargetLevelBefore", 0);
        TabRebuildItem.isNextRebuild = 4;
        enhanceMethod.invoke(null);
        check("13.5: Normal TabRebuildItem.isNextRebuild == 4 classified as FAILURE_PROTECTED", "FAILURE_PROTECTED".equals(get("enhLastResult")));
        check("13.5: Settlement source is RESULT_CODE", "RESULT_CODE".equals(get("enhSettlementSource")));
        check("13.5: Result code is 4", ((Integer) get("enhResultCode")).intValue() == 4);

        // 13.6, 13.7, 13.8: Exact state fallback reconciliation produces STATE_RECONCILED_SUCCESS with result_code null
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0; // No TabRebuildItem.isNextRebuild captured!
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 0); // timeout expired
        set("enhResultDeadline", System.currentTimeMillis() - 100L); // past deadline
        set("snapRequestId", "req-13-uuid");
        set("enhRequestId", "req-13-uuid");
        set("snapTemplateId", 67);
        set("enhTemplateId", 67);
        set("snapCategory", 3);
        set("enhCategory", 3);
        set("snapBaseName", "Kiem tap");
        set("enhBaseName", "Kiem tap");
        set("snapTier", 1);
        set("enhTier", 1);
        set("snapExpectedLevel", 0);
        set("enhExpectedLevel", 0);
        set("snapTargetLevel", 1);
        set("enhTargetLevel", 1);
        set("snapPaymentType", 0);
        set("enhPaymentType", 0);
        set("snapRecipeGoldCost", 3000L);
        set("snapRecipeGemCost", 0L);
        set("snapRecipeMaterials", new long[]{1L, 1L, 0L, 0L});
        set("snapResolvedCharmMode", 0);
        set("snapCharmBefore", 0L);
        set("snapGoldBefore", 10000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{10L, 10L, 0L, 0L});
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap +1", "Kiem tap", 1, 1));
        GameScreen.player.coin = 7000;
        GameScreen.player.gold = 50;
        TabRebuildItem.numMaterialInven = new int[]{9, 9, 0, 0};
        enhanceMethod.invoke(null);
        check("13.7: Exact state reconciliation produces STATE_RECONCILED_SUCCESS", "STATE_RECONCILED_SUCCESS".equals(get("enhSettlementProvenance")));
        check("13.7: State transitions to TARGET_REACHED (17)", ((Integer) get("enhState")).intValue() == 17);
        check("13.7: Settlement source is STATE_RECONCILED", "STATE_RECONCILED".equals(get("enhSettlementSource")));
        check("13.8: Reconciled result code is -1 (NULL)", ((Integer) get("enhResultCode")).intValue() == -1);
        String reconciledJson = (String) formatStatusMethod.invoke(null);
        check("13.8: Status JSON has result_code: null", reconciledJson.indexOf("\"result_code\": null") >= 0);
        check("13.8: Status JSON has last_result: STATE_RECONCILED_SUCCESS", reconciledJson.indexOf("\"last_result\": \"STATE_RECONCILED_SUCCESS\"") >= 0);
        check("13.8: Status JSON has settlement_source: STATE_RECONCILED", reconciledJson.indexOf("\"settlement_source\": \"STATE_RECONCILED\"") >= 0);

        // 13.9: Level advancement with mismatched resources remains ambiguous
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 0);
        set("enhResultDeadline", System.currentTimeMillis() - 100L);
        set("snapRequestId", "req-13-uuid");
        set("enhRequestId", "req-13-uuid");
        set("snapTemplateId", 67);
        set("enhTemplateId", 67);
        set("snapCategory", 3);
        set("enhCategory", 3);
        set("snapBaseName", "Kiem tap");
        set("enhBaseName", "Kiem tap");
        set("snapTier", 1);
        set("enhTier", 1);
        set("snapExpectedLevel", 0);
        set("enhExpectedLevel", 0);
        set("snapTargetLevel", 1);
        set("enhTargetLevel", 1);
        set("snapPaymentType", 0);
        set("snapRecipeGoldCost", 3000L);
        set("snapRecipeGemCost", 0L);
        set("snapRecipeMaterials", new long[]{1L, 1L, 0L, 0L});
        set("snapResolvedCharmMode", 0);
        set("snapCharmBefore", 0L);
        set("snapGoldBefore", 10000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{10L, 10L, 0L, 0L});
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap +1", "Kiem tap", 1, 1));
        GameScreen.player.coin = 8000; // mismatch
        GameScreen.player.gold = 50;
        TabRebuildItem.numMaterialInven = new int[]{9, 9, 0, 0};
        enhanceMethod.invoke(null);
        check("13.9: Mismatched resources enters RESULT_AMBIGUOUS (29)", ((Integer) get("enhState")).intValue() == 29);

        // 13.10: Resource delta match without exact target level advancement remains ambiguous
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 0);
        set("enhResultDeadline", System.currentTimeMillis() - 100L);
        set("snapRequestId", "req-13-uuid");
        set("enhRequestId", "req-13-uuid");
        set("snapTemplateId", 67);
        set("enhTemplateId", 67);
        set("snapCategory", 3);
        set("enhCategory", 3);
        set("snapBaseName", "Kiem tap");
        set("enhBaseName", "Kiem tap");
        set("snapTier", 1);
        set("enhTier", 1);
        set("snapExpectedLevel", 0);
        set("snapTargetLevel", 1);
        set("snapPaymentType", 0);
        set("snapRecipeGoldCost", 3000L);
        set("snapRecipeGemCost", 0L);
        set("snapRecipeMaterials", new long[]{1L, 1L, 0L, 0L});
        set("snapResolvedCharmMode", 0);
        set("snapCharmBefore", 0L);
        set("snapGoldBefore", 10000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{10L, 10L, 0L, 0L});
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap", "Kiem tap", 0, 1)); // unadvanced level 0
        GameScreen.player.coin = 7000;
        GameScreen.player.gold = 50;
        TabRebuildItem.numMaterialInven = new int[]{9, 9, 0, 0};
        enhanceMethod.invoke(null);
        check("13.10: Unadvanced level enters RESULT_AMBIGUOUS (29)", ((Integer) get("enhState")).intValue() == 29);

        // 13.11: Wrong fingerprint remains ambiguous
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 0);
        set("enhResultDeadline", System.currentTimeMillis() - 100L);
        set("snapRequestId", "req-13-uuid");
        set("enhRequestId", "req-13-uuid");
        set("snapTemplateId", 67);
        set("enhTemplateId", 67);
        set("snapCategory", 3);
        set("enhCategory", 3);
        set("snapBaseName", "Kiem tap");
        set("enhBaseName", "Kiem tap");
        set("snapTier", 1);
        set("enhTier", 1);
        set("snapExpectedLevel", 0);
        set("snapTargetLevel", 1);
        set("snapPaymentType", 0);
        set("snapRecipeGoldCost", 3000L);
        set("snapRecipeGemCost", 0L);
        set("snapRecipeMaterials", new long[]{1L, 1L, 0L, 0L});
        set("snapResolvedCharmMode", 0);
        set("snapCharmBefore", 0L);
        set("snapGoldBefore", 10000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{10L, 10L, 0L, 0L});
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap +1", "Kiem tap", 1, 2)); // tier 2 != tier 1
        GameScreen.player.coin = 7000;
        GameScreen.player.gold = 50;
        TabRebuildItem.numMaterialInven = new int[]{9, 9, 0, 0};
        enhanceMethod.invoke(null);
        check("13.11: Wrong fingerprint enters RESULT_AMBIGUOUS (29)", ((Integer) get("enhState")).intValue() == 29);

        // 13.12: Duplicate O+u remains ambiguous
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 0);
        set("enhResultDeadline", System.currentTimeMillis() - 100L);
        set("snapRequestId", "req-13-uuid");
        set("enhRequestId", "req-13-uuid");
        set("snapTemplateId", 67);
        set("enhTemplateId", 67);
        set("snapCategory", 3);
        set("enhCategory", 3);
        set("snapBaseName", "Kiem tap");
        set("enhBaseName", "Kiem tap");
        set("snapTier", 1);
        set("enhTier", 1);
        set("snapExpectedLevel", 0);
        set("snapTargetLevel", 1);
        set("snapPaymentType", 0);
        set("snapRecipeGoldCost", 3000L);
        set("snapRecipeGemCost", 0L);
        set("snapRecipeMaterials", new long[]{1L, 1L, 0L, 0L});
        set("snapResolvedCharmMode", 0);
        set("snapCharmBefore", 0L);
        set("snapGoldBefore", 10000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{10L, 10L, 0L, 0L});
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap +1", "Kiem tap", 1, 1), makeItem(67, 3, "Kiem tap +1", "Kiem tap", 1, 1));
        GameScreen.player.coin = 7000;
        GameScreen.player.gold = 50;
        TabRebuildItem.numMaterialInven = new int[]{9, 9, 0, 0};
        enhanceMethod.invoke(null);
        check("13.12: Duplicate O+u enters RESULT_AMBIGUOUS (29)", ((Integer) get("enhState")).intValue() == 29);

        // 13.13: Wrong request UUID cannot reconcile
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 0);
        set("enhResultDeadline", System.currentTimeMillis() - 100L);
        set("snapRequestId", "req-different-uuid");
        set("enhRequestId", "req-13-uuid");
        set("snapTemplateId", 67);
        set("enhTemplateId", 67);
        set("snapCategory", 3);
        set("enhCategory", 3);
        set("snapBaseName", "Kiem tap");
        set("enhBaseName", "Kiem tap");
        set("snapTier", 1);
        set("enhTier", 1);
        set("snapExpectedLevel", 0);
        set("snapTargetLevel", 1);
        set("snapPaymentType", 0);
        set("snapRecipeGoldCost", 3000L);
        set("snapRecipeGemCost", 0L);
        set("snapRecipeMaterials", new long[]{1L, 1L, 0L, 0L});
        set("snapResolvedCharmMode", 0);
        set("snapCharmBefore", 0L);
        set("snapGoldBefore", 10000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{10L, 10L, 0L, 0L});
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap +1", "Kiem tap", 1, 1));
        GameScreen.player.coin = 7000;
        GameScreen.player.gold = 50;
        TabRebuildItem.numMaterialInven = new int[]{9, 9, 0, 0};
        enhanceMethod.invoke(null);
        check("13.13: Request UUID mismatch enters RESULT_AMBIGUOUS (29)", ((Integer) get("enhState")).intValue() == 29);

        // 13.14: Unchanged target does not become fake failure or fake success
        call("enhanceReset");
        setupWorldState(1);
        TabRebuildItem.isNextRebuild = 0;
        set("enhLastResult", null);
        set("enhState", 13);
        set("enhInFlightExecute", true);
        set("enhWait", 0);
        set("enhResultDeadline", System.currentTimeMillis() - 100L);
        set("snapRequestId", "req-13-uuid");
        set("enhRequestId", "req-13-uuid");
        set("snapTemplateId", 67);
        set("enhTemplateId", 67);
        set("snapCategory", 3);
        set("enhCategory", 3);
        set("snapBaseName", "Kiem tap");
        set("enhBaseName", "Kiem tap");
        set("snapTier", 1);
        set("enhTier", 1);
        set("snapExpectedLevel", 0);
        set("snapTargetLevel", 1);
        Item.VecInvetoryPlayer = bag(makeItem(67, 3, "Kiem tap", "Kiem tap", 0, 1));
        GameScreen.player.coin = 10000;
        GameScreen.player.gold = 50;
        enhanceMethod.invoke(null);
        check("13.14: Unchanged target enters RESULT_AMBIGUOUS (29)", ((Integer) get("enhState")).intValue() == 29);
        check("13.14: Last result is null", get("enhLastResult") == null);
        check("13.14: Result code is null (-1)", ((Integer) get("enhResultCode")).intValue() == -1);

        // ---------------------------------------------------------------------
        // Test 14: Same-UUID Exactly-Once & Terminal Monotonicity (ENHANCE-05Q)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 14: Same-UUID exactly-once & terminal monotonicity ---");
        Method enhSidecarTickMethod = Class.forName("Zeus").getDeclaredMethod("enhSidecarTick", long.class);
        enhSidecarTickMethod.setAccessible(true);

        java.io.File tempDir = new java.io.File(System.getProperty("java.io.tmpdir"), "zeus_test_" + System.currentTimeMillis());
        tempDir.mkdirs();
        java.io.File reqFile = new java.io.File(tempDir, "zeus-enhance.req");
        java.io.File statusFile = new java.io.File(tempDir, "zeus-enhance-status.json");
        java.io.File cancelFile = new java.io.File(tempDir, "zeus-enhance.cancel");

        set("enhReqPath", reqFile.getAbsolutePath());
        set("enhStatusPath", statusFile.getAbsolutePath());
        set("enhCancelPath", cancelFile.getAbsolutePath());
        set("enhReqCheckedAt", 0L);
        call("enhanceReset");

        // 14.1: Same UUID while non-terminal is not reinitialized
        writeReqFile(reqFile, "uuid-test-14-same", 67, 1, 1);
        long now = 1000L;
        enhSidecarTickMethod.invoke(null, now);
        check("14.1: New request accepted into VALIDATING_REQUEST (1)", ((Integer) get("enhState")).intValue() == 1);
        set("enhState", 11); // Advance to READY_FOR_ATTEMPT
        set("enhAttemptCount", 1);
        now += 300L;
        enhSidecarTickMethod.invoke(null, now);
        check("14.1: Same UUID while in-flight is NOT reinitialized (state 11)", ((Integer) get("enhState")).intValue() == 11);
        check("14.1: attempt_count preserved at 1", ((Integer) get("enhAttemptCount")).intValue() == 1);

        // 14.2: Same UUID after TARGET_REACHED is not reinitialized
        set("enhState", 17); // TARGET_REACHED
        set("enhAttemptCount", 1);
        set("enhActualGoldSpent", 3000L);
        set("enhActualGemSpent", 0L);
        set("enhActualCharmsSpent", 0L);
        set("enhActualMaterialsSpent", new long[]{1L, 1L, 0L, 0L});
        set("enhAccountingStatus", "SETTLED");
        set("enhResultCode", 3);
        set("enhSettlementProvenance", "RESULT_CODE_SUCCESS");
        set("enhSettlementSource", "RESULT_CODE");
        set("enhLastResult", "SUCCESS");
        set("enhCurrentLevel", 1);
        call("publishEnhancementStatus");

        // Request file STILL exists on disk with same UUID "uuid-test-14-same"
        now += 300L;
        enhSidecarTickMethod.invoke(null, now);
        check("14.2: Same UUID after TARGET_REACHED does NOT reinitialize to VALIDATING_REQUEST", ((Integer) get("enhState")).intValue() == 17);
        check("14.2: attempt_count remains 1 after tick", ((Integer) get("enhAttemptCount")).intValue() == 1);
        check("14.2: actual gold spent remains 3000", ((Long) get("enhActualGoldSpent")).longValue() == 3000L);
        check("14.2: result_code remains 3", ((Integer) get("enhResultCode")).intValue() == 3);
        check("14.2: settlement provenance remains RESULT_CODE_SUCCESS", "RESULT_CODE_SUCCESS".equals(get("enhSettlementProvenance")));
        check("14.2: current_level remains 1", ((Integer) get("enhCurrentLevel")).intValue() == 1);
        // 14.3: Terminal TARGET_REACHED remains stable over multiple sidecar ticks
        for (int t = 0; t < 5; t++) {
            now += 300L;
            enhSidecarTickMethod.invoke(null, now);
        }
        check("14.3: Terminal TARGET_REACHED stable over multiple ticks", ((Integer) get("enhState")).intValue() == 17);
        check("14.3: attempt_count stable over multiple ticks", ((Integer) get("enhAttemptCount")).intValue() == 1);
        String sJson = readStatusJson(statusFile);
        check("14.3: Status file on disk has TARGET_REACHED", sJson != null && sJson.contains("\"TARGET_REACHED\""));
        check("14.3: Status file on disk has result_code 3", sJson != null && sJson.contains("\"result_code\": 3"));
        check("14.3: Status file on disk has attempt_count 1", sJson != null && sJson.contains("\"attempt_count\": 1"));

        // 14.4: Same UUID after terminal failure is not reinitialized
        set("enhState", 18); // ATTEMPT_LIMIT_REACHED
        set("enhLastResult", "ATTEMPT_LIMIT_REACHED");
        call("publishEnhancementStatus");
        now += 300L;
        enhSidecarTickMethod.invoke(null, now);
        check("14.4: Same UUID after ATTEMPT_LIMIT_REACHED is NOT reinitialized (18)", ((Integer) get("enhState")).intValue() == 18);

        // 14.5: Distinct new UUID can start after previous lifecycle is complete
        writeReqFile(reqFile, "uuid-test-14-brand-new", 67, 1, 1);
        now += 300L;
        enhSidecarTickMethod.invoke(null, now);
        check("14.5: Genuinely new UUID transitions to VALIDATING_REQUEST (1)", ((Integer) get("enhState")).intValue() == 1);
        check("14.5: New request ID adopted", "uuid-test-14-brand-new".equals(get("enhRequestId")));

        // 14.6: Forge cleanup after terminal does not overwrite status
        set("enhState", 17); // TARGET_REACHED
        set("enhOwnsForgeScreen", true);
        set("enhOwnsResultDialog", false);
        call("cleanEnhancementRouting");
        check("14.6: Forge ownership cleared by cleanup", !((Boolean) get("enhOwnsForgeScreen")).booleanValue());
        check("14.6: Status remains TARGET_REACHED (17)", ((Integer) get("enhState")).intValue() == 17);

        // 14.7: Owned result-dialog cleanup sends zero Opcode 67
        clearQueue();
        set("enhOwnsForgeScreen", true);
        set("enhOwnsResultDialog", true);
        GameCanvas.menu2.isShowMenu = true;
        Menu2.isNPCMenu = 1;
        dialogRecoveryMethod.invoke(null);
        check("14.7: Dialog dismissal emits zero packets", queue().size() == 0);

        // ---------------------------------------------------------------------
        // Test 15: Cross-Restart Stale Replay Protection (ENHANCE-05Q)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 15: Cross-restart stale replay protection ---");
        // 15.1: Restart with terminal status for same UUID prevents stale re-execution
        call("enhanceReset");
        set("lastEnhRequestId", null);
        set("enhState", 0);
        call("resetEnhancementDeduplication");

        writeReqFile(reqFile, "uuid-test-15-terminal", 67, 1, 1);
        String termStatusJson = "{\n"
            + "  \"version\": 1,\n"
            + "  \"request_id\": \"uuid-test-15-terminal\",\n"
            + "  \"state\": \"TARGET_REACHED\",\n"
            + "  \"captured_slot\": 0,\n"
            + "  \"template_id\": 67,\n"
            + "  \"category\": 3,\n"
            + "  \"base_name\": \"Kiem tap\",\n"
            + "  \"start_level\": 0,\n"
            + "  \"current_level\": 1,\n"
            + "  \"target_level\": 1,\n"
            + "  \"attempt_count\": 1,\n"
            + "  \"quoted_gold_cost\": 3000,\n"
            + "  \"quoted_gem_cost\": 0,\n"
            + "  \"quoted_material_requirements\": [1, 1, 0, 0],\n"
            + "  \"actual_gold_spent\": 3000,\n"
            + "  \"actual_gem_spent\": 0,\n"
            + "  \"actual_materials_spent\": [1, 1, 0, 0],\n"
            + "  \"actual_charms_spent\": 0,\n"
            + "  \"accounting_status\": \"SETTLED\",\n"
            + "  \"result_code\": 3,\n"
            + "  \"settlement_source\": \"RESULT_CODE\",\n"
            + "  \"settlement_provenance\": \"RESULT_CODE_SUCCESS\",\n"
            + "  \"updated_at\": \"2026-09-28T05:00:00Z\"\n"
            + "}\n";
        java.io.FileOutputStream fos = new java.io.FileOutputStream(statusFile);
        fos.write(termStatusJson.getBytes("UTF-8"));
        fos.close();

        now += 300L;
        enhSidecarTickMethod.invoke(null, now);
        check("15.1: Restart with terminal status does NOT re-enter VALIDATING_REQUEST", ((Integer) get("enhState")).intValue() != 1);
        check("15.1: Terminal state TARGET_REACHED preserved across restart", ((Integer) get("enhState")).intValue() == 17);
        check("15.1: Terminal result_code preserved (3)", ((Integer) get("enhResultCode")).intValue() == 3);

        // 15.2: Restart with post-fence non-terminal status enters MANUAL_REVIEW_REQUIRED (33)
        call("enhanceReset");
        set("lastEnhRequestId", null);
        set("enhState", 0);
        call("resetEnhancementDeduplication");

        writeReqFile(reqFile, "uuid-test-15-postfence", 67, 1, 1);
        String postFenceStatusJson = "{\n"
            + "  \"version\": 1,\n"
            + "  \"request_id\": \"uuid-test-15-postfence\",\n"
            + "  \"state\": \"WAITING_RESULT\",\n"
            + "  \"captured_slot\": 0,\n"
            + "  \"template_id\": 67,\n"
            + "  \"category\": 3,\n"
            + "  \"base_name\": \"Kiem tap\",\n"
            + "  \"start_level\": 0,\n"
            + "  \"current_level\": 0,\n"
            + "  \"target_level\": 1,\n"
            + "  \"attempt_count\": 1,\n"
            + "  \"quoted_gold_cost\": 3000,\n"
            + "  \"quoted_gem_cost\": 0,\n"
            + "  \"quoted_material_requirements\": [1, 1, 0, 0],\n"
            + "  \"actual_gold_spent\": 0,\n"
            + "  \"actual_gem_spent\": 0,\n"
            + "  \"actual_materials_spent\": [0, 0, 0, 0],\n"
            + "  \"actual_charms_spent\": 0,\n"
            + "  \"accounting_status\": \"PENDING\",\n"
            + "  \"updated_at\": \"2026-09-28T05:00:00Z\"\n"
            + "}\n";
        fos = new java.io.FileOutputStream(statusFile);
        fos.write(postFenceStatusJson.getBytes("UTF-8"));
        fos.close();

        clearQueue();
        now += 300L;
        enhSidecarTickMethod.invoke(null, now);
        check("15.2: Post-fence restart transitions to MANUAL_REVIEW_REQUIRED (33)", ((Integer) get("enhState")).intValue() == 33);
        check("15.2: Post-fence restart emits ZERO Opcode 67 packets", queue().size() == 0);

        // 15.3: Genuinely new request UUID after restart is accepted
        writeReqFile(reqFile, "uuid-test-15-brand-new", 67, 1, 1);
        now += 300L;
        enhSidecarTickMethod.invoke(null, now);
        check("15.3: Genuinely new UUID after restart accepted into VALIDATING_REQUEST (1)", ((Integer) get("enhState")).intValue() == 1);
        check("15.3: New UUID adopted", "uuid-test-15-brand-new".equals(get("enhRequestId")));

        System.out.println("=== EnhancementEngineTest Total Failures: " + failures + " ===");
        if (failures > 0) {
            System.exit(1);
        }
    }

    static void writeReqFile(java.io.File file, String reqId, int templateId, int targetLevel, int maxAttempts) throws Exception {
        String json = "{\n"
            + "  \"request_id\": \"" + reqId + "\",\n"
            + "  \"captured_slot\": 0,\n"
            + "  \"template_id\": " + templateId + ",\n"
            + "  \"category\": 3,\n"
            + "  \"base_name\": \"Kiem tap\",\n"
            + "  \"tier\": 1,\n"
            + "  \"expected_level\": 0,\n"
            + "  \"target_level\": " + targetLevel + ",\n"
            + "  \"charm_mode\": 0,\n"
            + "  \"payment_type\": 0,\n"
            + "  \"max_attempts\": " + maxAttempts + ",\n"
            + "  \"validation_only\": false\n"
            + "}\n";
        java.io.FileOutputStream fos = new java.io.FileOutputStream(file);
        fos.write(json.getBytes("UTF-8"));
        fos.close();
    }

    static String readStatusJson(java.io.File file) throws Exception {
        if (!file.exists()) return null;
        java.io.FileInputStream fis = new java.io.FileInputStream(file);
        byte[] b = new byte[(int) file.length()];
        int read = fis.read(b);
        fis.close();
        return new String(b, 0, read, "UTF-8");
    }

    static TabScreenNew makeForgePopup() {
        TabScreenNew popup = new TabScreenNew();
        popup.VecTabScreen = new mVector("tabs");
        TabRebuildItem forgeTab = new TabRebuildItem("Cuong hoa", (byte) 0);
        popup.VecTabScreen.addElement(forgeTab);
        popup.selectTab = 0;
        return popup;
    }

    static TabScreenNew makeInventoryPopup() {
        TabScreenNew popup = new TabScreenNew();
        popup.VecTabScreen = new mVector("tabs");
        TabShopNew invTab = null;
        try {
            Field uf = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
            uf.setAccessible(true);
            sun.misc.Unsafe unsafe = (sun.misc.Unsafe) uf.get(null);
            invTab = (TabShopNew) unsafe.allocateInstance(TabShopNew.class);
        } catch (Throwable t) {
        }
        popup.VecTabScreen.addElement(invTab);
        popup.selectTab = 0;
        return popup;
    }

    static void setupWorldState(int mapId) {
        if (GameCanvas.game == null) {
            GameCanvas.game = new GameScreen();
        }
        GameCanvas.currentScreen = GameCanvas.game;
        LoadMapScreen.isNextMap = true;
        GameCanvas.currentDialog = null;
        if (GameCanvas.loadmap == null) {
            try {
                Field uf = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
                uf.setAccessible(true);
                sun.misc.Unsafe unsafe = (sun.misc.Unsafe) uf.get(null);
                GameCanvas.loadmap = (LoadMap) unsafe.allocateInstance(LoadMap.class);
            } catch (Throwable t) {
            }
        }
        if (GameCanvas.loadmap != null) {
            GameCanvas.loadmap.idMap = mapId;
        }
        if (GameScreen.player == null) {
            GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
        }
        GameScreen.player.Action = 0; // alive (4 is dead)
        GameScreen.player.typePk = -1; // realistic peaceful v4.0.3 player
        GameScreen.player.typeBoss = 0;
        GameScreen.player.x = 100;
        GameScreen.player.y = 100;
        try {
            Field rst = Class.forName("Zeus").getDeclaredField("readySettleTicks");
            rst.setAccessible(true);
            rst.set(null, 15);
            Field mst = Class.forName("Zeus").getDeclaredField("mapStableTicks");
            mst.setAccessible(true);
            mst.set(null, 15);
            Field sm = Class.forName("Zeus").getDeclaredField("stableMapId");
            sm.setAccessible(true);
            sm.set(null, mapId);
        } catch (Throwable t) {
        }
    }

    static MainObject makeNpc(String name, int cu, int cv, int x, int y) {
        MainObject npc = new MainObject();
        npc.name = name;
        npc.ID = cu;
        npc.typeObject = (byte) cv;
        npc.x = x;
        npc.y = y;
        return npc;
    }
}
