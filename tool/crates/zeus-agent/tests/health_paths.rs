mod spot_scan {
    pub const SPOT_REQUEST_FILE_NAME: &str = "zeus-spot-req.json";
    pub const SPOT_RESULT_PAYLOAD_FILE_NAME: &str = "zeus-spot-payload.json";
    pub const SPOT_RESULT_READY_FILE_NAME: &str = "zeus-spot-ready.txt";
}
mod inventory {
    pub const INVENTORY_FILE_NAME: &str = "zeus-inventory.json";
}
mod enhancement {
    pub const ENHANCE_REQUEST_FILE_NAME: &str = "zeus-enhance-req.json";
    pub const ENHANCE_STATUS_FILE_NAME: &str = "zeus-enhance-status.json";
    pub const ENHANCE_CANCEL_FILE_NAME: &str = "zeus-enhance-cancel.txt";
}

#[path = "../src/launch.rs"]
#[allow(dead_code)]
mod launch;

#[test]
fn account_paths_health_file_agrees_with_zeus_core_health_file_name() {
    let paths = launch::AccountPaths::for_slot(1);
    assert_eq!(
        paths.health_file(),
        paths.home.join(zeus_core::wire::HEALTH_FILE_NAME)
    );
    assert_eq!(
        paths.health_file().file_name().unwrap(),
        zeus_core::wire::HEALTH_FILE_NAME
    );
    assert_eq!(zeus_core::wire::HEALTH_FILE_NAME, "zeus-health.txt");
}

#[test]
fn account_paths_reconnect_status_file_agrees_with_zeus_core_constant() {
    let paths = launch::AccountPaths::for_slot(1);
    assert_eq!(
        paths.reconnect_status_file(),
        paths.home.join(zeus_core::wire::RECONNECT_STATUS_FILE_NAME)
    );
    assert_eq!(
        paths.reconnect_status_file().file_name().unwrap(),
        zeus_core::wire::RECONNECT_STATUS_FILE_NAME
    );
    assert_eq!(zeus_core::wire::RECONNECT_STATUS_FILE_NAME, "zeus-reconnect.txt");
}
