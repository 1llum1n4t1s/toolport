//! Removal fixtures run in child processes with isolated HOME, data and bus state.
use std::process::Command;

#[cfg(target_os = "linux")]
#[test]
fn purge_headless_without_keyring_and_with_file_backend_removes_data() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_headless");
    for file_backend in [false, true] {
        let data = env.dir.join(if file_backend { "file" } else { "no-bus" });
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("registry.json"), serde_json::to_vec(&conduit_lib::registry::Registry::default()).unwrap()).unwrap();
        std::fs::write(data.join("secrets.enc"), "encrypted fixture").unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_toolport-gateway"));
        command.args(["--remove-data", "--confirm"])
            .env("TOOLPORT_DATA_DIR", &data).env("HOME", env.dir.join("home"))
            .env_remove("DBUS_SESSION_BUS_ADDRESS").env_remove("CONDUIT_SECRET_KEY")
            .env_remove("TOOLPORT_SECRET_KEY");
        if file_backend {
            command.env("TOOLPORT_SECRET_KEY", "purge-fixture-key")
                .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/purge-fixture-bus");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["leftovers"].as_array().unwrap().is_empty());
        assert!(report["notes"].as_array().unwrap().iter().any(|note| note.as_str().unwrap().contains("No keyring in use")));
        assert!(!data.join("secrets.enc").exists());
        assert!(!data.join("registry.json").exists());
    }
}

#[test]
fn purge_named_override_without_registry_never_deletes_user_files() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_named_override");
    let data = env.dir.join("Toolport");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("personal.txt"), "keep").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_toolport-gateway"))
        .args(["--remove-data", "--confirm"]).env("TOOLPORT_DATA_DIR", &data)
        .env("HOME", env.dir.join("home")).env_remove("DBUS_SESSION_BUS_ADDRESS")
        .output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("ownership"));
    assert_eq!(std::fs::read_to_string(data.join("personal.txt")).unwrap(), "keep");
}
