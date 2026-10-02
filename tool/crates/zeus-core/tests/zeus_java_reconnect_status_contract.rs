use std::fs;
use std::path::Path;

#[test]
fn verify_zeus_java_reconnect_status_sidecar_contract() {
    let zeus_java_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path)
        .expect("Zeus.java must exist and be readable");

    // 1. Verify reconnectStatusSidecarTick is invoked before the fu.a == null branch in tick()
    let tick_pos = content.find("public static void tick()").expect("tick() must exist");
    let status_call_pos = content[tick_pos..].find("reconnectStatusSidecarTick();")
        .expect("reconnectStatusSidecarTick() must be called in tick()");
    let null_branch_pos = content[tick_pos..].find("if (fu.a == null)")
        .expect("if (fu.a == null) must exist in tick()");
    assert!(
        status_call_pos < null_branch_pos,
        "reconnectStatusSidecarTick() must be called before if (fu.a == null) branch"
    );

    // 2. Extract reconnectStatusSidecarTick method body
    let method_header = "private static void reconnectStatusSidecarTick()";
    let method_start = content.find(method_header)
        .expect("reconnectStatusSidecarTick() definition must exist");
    let method_body = &content[method_start..method_start + 3000];

    // 3. Verify publisher has no cn.g prerequisite
    assert!(
        !method_body.contains("cn.g"),
        "reconnectStatusSidecarTick() must have no cn.g prerequisite"
    );

    // 4. Verify publisher contains no network send primitive
    let forbidden_network = ["send(", "opcode", "Socket", "Http", "writePacket", "MIDlet"];
    for forbidden in forbidden_network {
        assert!(
            !method_body.contains(forbidden),
            "reconnectStatusSidecarTick() must contain no network primitive: {forbidden}"
        );
    }

    // 5. Verify emitted key set is exactly v, t, seq, episode, active, state, transitions, world_before
    let expected_keys = [
        "v=1\\n",
        "t=",
        "seq=",
        "episode=",
        "active=",
        "state=",
        "transitions=",
        "world_before=",
    ];
    for key in expected_keys {
        assert!(
            method_body.contains(key),
            "reconnectStatusSidecarTick() must emit key: {key}"
        );
    }

    // Ensure exactly 8 keys are appended
    let appends: Vec<&str> = method_body
        .lines()
        .filter(|l| l.contains("sb.append(\""))
        .collect();
    assert_eq!(
        appends.len(),
        8,
        "Exactly 8 keys must be appended in reconnectStatusSidecarTick: {:?}",
        appends
    );

    // 6. Verify state mappings exist
    assert!(method_body.contains("\"idle\""), "must contain idle mapping");
    assert!(method_body.contains("\"native_wait\""), "must contain native_wait mapping");
    assert!(method_body.contains("\"login\""), "must contain login mapping");
    assert!(method_body.contains("\"server\""), "must contain server mapping");
    assert!(method_body.contains("\"character\""), "must contain character mapping");
    assert!(method_body.contains("\"loading\""), "must contain loading mapping");
    assert!(method_body.contains("\"world_settle\""), "must contain world_settle mapping");
    assert!(method_body.contains("\"other\""), "must contain other mapping");

    // 7. Verify file constant and initialization
    assert!(
        content.contains("RECONNECT_STATUS_FILE = \"zeus-reconnect.txt\";"),
        "RECONNECT_STATUS_FILE must be defined as zeus-reconnect.txt"
    );
    assert!(
        content.contains("reconnectStatusPath = home + RECONNECT_STATUS_FILE;"),
        "reconnectStatusPath must be initialized from home + RECONNECT_STATUS_FILE"
    );
}
