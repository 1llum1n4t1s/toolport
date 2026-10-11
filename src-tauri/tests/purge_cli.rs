//! Removal fixtures run in child processes with isolated HOME, data and bus state.
use std::process::Command;

fn isolated_command(root: &std::path::Path, data: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_toolport-gateway"));
    // Client-path overrides and other ambient config must not escape the fixture.
    command.env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default());
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) { command.env(key, value); }
    }
    let home = root.join("home");
    command.env("TOOLPORT_DATA_DIR", data).env("HOME", &home).env("USERPROFILE", &home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_RUNTIME_DIR", root.join("absent-runtime"))
        .env_remove("DBUS_SESSION_BUS_ADDRESS").env_remove("CONDUIT_SECRET_KEY")
        .env_remove("TOOLPORT_SECRET_KEY");
    command
}

#[cfg(target_os = "linux")]
#[test]
fn purge_headless_without_keyring_and_with_file_backend_removes_data() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_headless");
    for file_backend in [false, true] {
        let data = env.dir.join(if file_backend { "file" } else { "no-bus" });
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("registry.json"), serde_json::to_vec(&conduit_lib::registry::Registry::default()).unwrap()).unwrap();
        std::fs::write(data.join("secrets.enc"), "encrypted fixture").unwrap();
        let mut command = isolated_command(&env.dir, &data);
        command.args(["--remove-data", "--confirm"]);
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
    let output = isolated_command(&env.dir, &data)
        .args(["--remove-data", "--confirm"])        .output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("ownership"));
    assert_eq!(std::fs::read_to_string(data.join("personal.txt")).unwrap(), "keep");
}

#[cfg(target_os = "linux")]
#[test]
fn purge_override_with_registry_preserves_user_files_for_both_brand_overrides() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_user_files");
    for legacy in [false, true] {
        let root = env.dir.join(if legacy { "legacy" } else { "current" });
        let data = root.join(if legacy { "Conduit" } else { "Toolport" });
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("registry.json"), serde_json::to_vec(&conduit_lib::registry::Registry::default()).unwrap()).unwrap();
        for name in ["backups/tax.zip", "bin/personal-script", "logs/other-app.log", "daemon-notes.json"] {
            let path = data.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "user bytes").unwrap();
        }
        std::fs::write(data.join("personal.txt"), "keep").unwrap();
        std::fs::write(data.join("secrets.enc"), "fixture").unwrap();
        let mut command = isolated_command(&root, &data);
        if legacy { command.env_remove("TOOLPORT_DATA_DIR").env("CONDUIT_DATA_DIR", &data); }
        let output = command.args(["--remove-data", "--confirm"]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["preserved"].as_array().unwrap().iter().any(|entry| entry.as_str().unwrap().contains("personal.txt")));
        assert_eq!(std::fs::read_to_string(data.join("personal.txt")).unwrap(), "keep");
        for name in ["backups/tax.zip", "bin/personal-script", "logs/other-app.log", "daemon-notes.json"] {
            assert_eq!(std::fs::read_to_string(data.join(name)).unwrap(), "user bytes", "{name}");
            assert!(report["preserved"].as_array().unwrap().iter().any(|entry| entry.as_str().unwrap().contains(name)), "{name}");
        }
        assert!(!data.join("registry.json").exists() && !data.join("secrets.enc").exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn purge_global_option_proceeds_with_corrupt_registry() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_corrupt");
    let data = env.dir.join("Toolport");
    std::fs::create_dir(&data).unwrap();
    let path = data.join("registry.json");
    std::fs::write(&path, "broken").unwrap();
    std::fs::write(data.join("personal.txt"), "keep").unwrap();
    let output = isolated_command(&env.dir, &data).args(["--remove-data", "--confirm"]).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(path.to_str().unwrap()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
    let output = isolated_command(&env.dir, &data).args(["--remove-data", "--all-installs", "--confirm"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert!(!path.exists());
    assert_eq!(std::fs::read_to_string(data.join("personal.txt")).unwrap(), "keep");
}

#[cfg(target_os = "linux")]
#[test]
fn purge_missing_desktop_bus_keeps_registry_for_credential_retry() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_bus_retry");
    let data = env.dir.join("Toolport");
    let runtime = env.dir.join("runtime");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&runtime).unwrap();
    let registry = serde_json::to_vec(&conduit_lib::registry::Registry::default()).unwrap();
    std::fs::write(data.join("registry.json"), &registry).unwrap();
    std::fs::write(data.join("secrets.enc"), "fixture").unwrap();
    for key in ["", "selected-file-backend"] {
        let output = isolated_command(&env.dir, &data).env("XDG_RUNTIME_DIR", &runtime)
            .env("TOOLPORT_SECRET_KEY", key).args(["--remove-data", "--confirm"]).output().unwrap();
        assert!(!output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["leftovers"][0]["path"].as_str().unwrap().contains("credential cleanup not done"));
        assert_eq!(std::fs::read(data.join("registry.json")).unwrap(), registry);
        assert!(data.join("secrets.enc").exists());
    }
    std::fs::remove_dir(runtime).unwrap();
    let output = isolated_command(&env.dir, &data).args(["--remove-data", "--confirm"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert!(!data.join("registry.json").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn purge_recovers_runtime_bus_but_keeps_registry_when_secret_service_is_closed() {
    use std::io::BufRead;
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_runtime_bus");
    let data = env.dir.join("Toolport");
    let runtime = env.dir.join("runtime");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&runtime).unwrap();
    let config = env.dir.join("bus.conf");
    // A private bus with no activation directories cannot touch a desktop keyring.
    std::fs::write(&config, format!(r#"<busconfig><type>session</type><listen>unix:path={}</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>"#, runtime.join("bus").display())).unwrap();
    struct Bus(std::process::Child);
    impl Drop for Bus {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }
    let mut bus = Bus(Command::new("dbus-daemon").arg(format!("--config-file={}", config.display()))
        .args(["--nofork", "--nopidfile", "--print-address=1"]).stdout(std::process::Stdio::piped()).spawn().unwrap());
    let mut address = String::new();
    std::io::BufReader::new(bus.0.stdout.take().unwrap()).read_line(&mut address).unwrap();
    assert!(address.starts_with("unix:path="));
    let registry = serde_json::to_vec(&conduit_lib::registry::Registry::default()).unwrap();
    std::fs::write(data.join("registry.json"), &registry).unwrap();
    for key in ["", "selected-file-backend"] {
        let output = isolated_command(&env.dir, &data).env("XDG_RUNTIME_DIR", &runtime)
            .env("TOOLPORT_SECRET_KEY", key).args(["--remove-data", "--confirm"]).output().unwrap();
        assert!(!output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["leftovers"][0]["error"].as_str().unwrap().contains("Secret Service is unavailable"), "{report}");
        assert_eq!(std::fs::read(data.join("registry.json")).unwrap(), registry);
        assert!(report["removed"].as_array().unwrap().iter().all(|p| !p.as_str().unwrap().contains("registry.json")));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn purge_other_registry_failure_exits_nonzero_and_retry_converges() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_other_retry");
    let data = env.dir.join("Toolport");
    let other = env.dir.join("Toolport-dev");
    for dir in [&data, &other] { std::fs::create_dir(dir).unwrap(); }
    let registry = serde_json::to_vec(&conduit_lib::registry::Registry::default()).unwrap();
    std::fs::write(data.join("registry.json"), &registry).unwrap();
    std::fs::write(data.join("secrets.enc"), "selected encrypted tokens").unwrap();
    let other_registry = other.join("registry.json");
    std::fs::write(&other_registry, "broken").unwrap();
    let output = isolated_command(&env.dir, &data).args(["--remove-data", "--confirm"]).output().unwrap();
    assert!(!output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains(other_registry.to_str().unwrap()) && text.contains("Credentials were not removed"), "{text}");
    assert_eq!(std::fs::read(data.join("registry.json")).unwrap(), registry);
    assert!(data.join("secrets.enc").exists());
    std::fs::write(&other_registry, &registry).unwrap();
    let output = isolated_command(&env.dir, &data).args(["--remove-data", "--confirm"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert!(!data.join("registry.json").exists() && !data.join("secrets.enc").exists());
    assert_eq!(std::fs::read(other_registry).unwrap(), registry);
}


#[cfg(target_os = "linux")]
#[test]
fn purge_r4_dev_leftovers_do_not_block_later_main_install_removal() {
    let env = conduit_lib::registry::DataDirTestEnv::new("purge_cli_r4_leftovers");
    let dev = env.dir.join("Toolport-dev");
    let main = env.dir.join("Toolport");
    let registry = serde_json::to_vec(&conduit_lib::registry::Registry::default()).unwrap();
    for dir in [&dev, &main] {
        std::fs::create_dir(dir).unwrap();
        std::fs::write(dir.join("registry.json"), &registry).unwrap();
    }
    std::fs::create_dir(dev.join("logs")).unwrap();
    std::fs::write(dev.join("logs/other-app.log"), "user bytes").unwrap();
    let output = isolated_command(&env.dir, &dev).args(["--remove-data", "--confirm"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert!(!dev.join("registry.json").exists());
    let output = isolated_command(&env.dir, &main).args(["--remove-data", "--confirm"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert!(!main.join("registry.json").exists());
    assert_eq!(std::fs::read_to_string(dev.join("logs/other-app.log")).unwrap(), "user bytes");
}
