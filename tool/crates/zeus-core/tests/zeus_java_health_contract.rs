use std::fs;
use std::path::Path;

#[test]
fn verify_zeus_java_health_sidecar_contract() {
    let zeus_java_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../mod/zeus/src/Zeus.java");
    let content = fs::read_to_string(&zeus_java_path)
        .expect("Zeus.java must exist and be readable");

    // 1. Verify healthSidecarTick is invoked before the fu.a == null branch in tick()
    let tick_pos = content.find("public static void tick()").expect("tick() must exist");
    let health_call_pos = content[tick_pos..].find("healthSidecarTick();")
        .expect("healthSidecarTick() must be called in tick()");
    let null_branch_pos = content[tick_pos..].find("if (fu.a == null)")
        .expect("if (fu.a == null) must exist in tick()");
    assert!(
        health_call_pos < null_branch_pos,
        "healthSidecarTick() must be called before if (fu.a == null) branch"
    );

    // 2. Extract healthSidecarTick method body
    let method_header = "private static void healthSidecarTick()";
    let method_start = content.find(method_header)
        .expect("healthSidecarTick() definition must exist");
    let method_body = &content[method_start..method_start + 2500];

    // 3. Verify publisher has no cn.g prerequisite
    assert!(
        !method_body.contains("cn.g"),
        "healthSidecarTick() must have no cn.g prerequisite"
    );

    // 4. Verify health publisher contains no network send primitive
    let forbidden_network = ["send(", "opcode", "Socket", "Http", "writePacket", "MIDlet"];
    for forbidden in forbidden_network {
        assert!(
            !method_body.contains(forbidden),
            "healthSidecarTick() must contain no network primitive: {forbidden}"
        );
    }

    // 5. Verify emitted key set is exactly v, t, seq, screen, dialog, disconnect
    let expected_keys = ["v=1\\n", "t=", "seq=", "screen=", "dialog=", "disconnect="];
    for key in expected_keys {
        assert!(
            method_body.contains(key),
            "healthSidecarTick() must emit key: {key}"
        );
    }

    // Ensure no extra unexpected keys are appended
    let appends: Vec<&str> = method_body
        .lines()
        .filter(|l| l.contains("sb.append(\""))
        .collect();
    assert_eq!(
        appends.len(),
        6,
        "Exactly 6 keys must be appended in healthSidecarTick: {:?}",
        appends
    );
}
