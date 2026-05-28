use super::*;

#[test]
fn missing_file_loads_empty_settings() {
    let store = GatewayStore::new(
        std::env::temp_dir().join(format!("pamagotchi-missing-{}.yml", nanoid::nanoid!())),
    );

    let settings = store.load().unwrap();

    assert!(settings.gateway.is_empty());
}

#[test]
fn load_or_create_writes_empty_settings_when_missing() {
    let path = std::env::temp_dir().join(format!(
        "pamagotchi-create-gateways-{}.yml",
        nanoid::nanoid!()
    ));
    let store = GatewayStore::new(&path);

    let settings = store.load_or_create().unwrap();

    assert!(settings.gateway.is_empty());
    assert!(path.exists());

    let loaded = store.load().unwrap();
    assert!(loaded.gateway.is_empty());
}

#[test]
fn add_generates_unique_ids_and_persists_gateway_list() {
    let path = std::env::temp_dir().join(format!("pamagotchi-gateways-{}.yml", nanoid::nanoid!()));
    let store = GatewayStore::new(path);

    let first = store.add("whatsapp", BTreeMap::new()).unwrap();
    let second = store.add("whatsapp", BTreeMap::new()).unwrap();
    let settings = store.load().unwrap();

    assert_ne!(first.id, second.id);
    assert_eq!(settings.gateway.len(), 2);
    assert_eq!(settings.gateway[0].kind, "whatsapp");
}

#[test]
fn validates_duplicate_ids() {
    let settings = GatewaySettings {
        gateway: vec![
            GatewayEntry {
                id: "same".into(),
                kind: "whatsapp".into(),
                vars: BTreeMap::new(),
            },
            GatewayEntry {
                id: "same".into(),
                kind: "discord".into(),
                vars: BTreeMap::new(),
            },
        ],
    };

    assert!(settings.validate().is_err());
}

#[test]
fn updates_gateway_vars() {
    let path =
        std::env::temp_dir().join(format!("pamagotchi-update-vars-{}.yml", nanoid::nanoid!()));
    let store = GatewayStore::new(path);
    let entry = store.add("discord", BTreeMap::new()).unwrap();
    let vars = BTreeMap::from([("bot_token".into(), Value::String("secret".into()))]);

    let updated = store.update_vars(&entry.id, vars.clone()).unwrap().unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(updated.vars, vars);
    assert_eq!(loaded.gateway[0].vars, vars);
}

#[test]
fn computes_storage_paths_relative_to_data_dir() {
    let data_dir = PathBuf::from("/tmp/pamagotchi/data");

    assert_eq!(
        settings_path(&data_dir),
        PathBuf::from("/tmp/pamagotchi/data/gateway.yml")
    );
    assert_eq!(
        gateway_data_dir(&data_dir, "abc123"),
        PathBuf::from("/tmp/pamagotchi/data/gateways/abc123")
    );
}
