use assert_cmd::Command;
use atlas_llm::AtlasConfig;
use tempfile::TempDir;

#[test]
fn init_creates_three_files() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success();

    assert!(dir.path().join(".atlas/config.yaml").exists());
    assert!(dir.path().join(".atlas/components.overrides.yaml").exists());
    assert!(dir.path().join(".atlas/subsystems.overrides.yaml").exists());
}

#[test]
fn init_skips_existing_files() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    std::fs::write(dir.path().join(".atlas/config.yaml"), "existing content").unwrap();

    let output = Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("skipped") && stdout.contains("config.yaml"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".atlas/config.yaml")).unwrap(),
        "existing content"
    );
}

#[test]
fn init_prints_written_paths() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("config.yaml"));
    assert!(stdout.contains("components.overrides.yaml"));
    assert!(stdout.contains("subsystems.overrides.yaml"));
}

#[test]
fn freshly_init_config_loads_without_setting_env_vars() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success();

    // The scaffolded config.yaml ships ${ANTHROPIC_API_KEY} etc. inside
    // commented-out documentation lines. Loading it should succeed without
    // any provider env vars being set, because the active config selects
    // claude-code/* which needs no providers entry.
    let config = AtlasConfig::load(&dir.path().join(".atlas/config.yaml"))
        .expect("freshly init'd config must load without env vars set");
    assert_eq!(config.defaults.model, "claude-code/claude-sonnet-4-6");
}

#[test]
fn freshly_init_config_loads_when_flipped_to_http_provider() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success();

    // Simulate the smallest realistic edit a user makes to enable an HTTP
    // provider: swap the default model and add a concrete api_key.
    // params.max_tokens must already be in the scaffold — otherwise
    // AtlasConfig::load returns ConfigError::MissingMaxTokens.
    let config_path = dir.path().join(".atlas/config.yaml");
    let original = std::fs::read_to_string(&config_path).unwrap();
    let edited = original.replace(
        "defaults:\n  model: claude-code/claude-sonnet-4-6",
        "providers:\n  anthropic:\n    api_key: \"test-key\"\ndefaults:\n  model: anthropic/claude-sonnet-4-6",
    );
    assert_ne!(edited, original, "model line not found in scaffold");
    std::fs::write(&config_path, edited).unwrap();

    AtlasConfig::load(&config_path)
        .expect("flipped-to-anthropic config must load — params.max_tokens must ship in scaffold");
}

#[test]
fn freshly_init_overrides_pass_validate_overrides() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("atlas")
        .unwrap()
        .args(["init", dir.path().to_str().unwrap()])
        .assert()
        .success();

    // `atlas init` followed immediately by `atlas validate-overrides`
    // must succeed: the scaffolded override files have to parse against
    // the loader's schema_version contract.
    Command::cargo_bin("atlas")
        .unwrap()
        .args([
            "validate-overrides",
            dir.path()
                .join(".atlas/components.overrides.yaml")
                .to_str()
                .unwrap(),
        ])
        .assert()
        .success();
}
