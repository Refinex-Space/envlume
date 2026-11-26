use envlume_application::{
    ports::Repository,
    workspaces::{ManagementError, WorkspaceRepository, add_project},
};
use envlume_domain::{Alias, Capability, ToolRule, UtcDateTime};
use envlume_storage::{Store, StorePaths};
use std::fs;

#[test]
fn attaching_and_editing_shared_profiles_preserves_other_bindings_and_grants() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(StorePaths::at(tmp.path().join("store")).unwrap()).unwrap();
    let one = tmp.path().join("one");
    let two = tmp.path().join("two");
    fs::create_dir(&one).unwrap();
    fs::create_dir(&two).unwrap();
    let a = add_project(&store, &one, None, |_, _| true).unwrap();
    let b = add_project(&store, &two, Some(a.project_id), |_, _| true).unwrap();
    let name = Alias::try_from("default".to_owned()).unwrap();
    let rule = |v: &str| ToolRule {
        tool: Alias::try_from("jdk".to_owned()).unwrap(),
        version_requirement: v.into(),
        selected_path: None,
        priority: 0,
    };
    let id = store
        .configure_toolchain(a.binding_id, &one, &name, &[rule("8")])
        .unwrap();
    let snapshot = |binding| store.load_binding(binding).unwrap().unwrap();
    assert!(!snapshot(b.binding_id).has_grant(Capability::ToolchainInspect, UtcDateTime::now()));
    assert_eq!(
        store
            .configure_toolchain(b.binding_id, &two, &name, &[rule("8")])
            .unwrap(),
        id
    );
    assert!(matches!(
        store.configure_toolchain(a.binding_id, &one, &name, &[rule("17")]),
        Err(ManagementError::SharedProfile)
    ));
    let second = Alias::try_from("modern".to_owned()).unwrap();
    let other = store
        .configure_toolchain(b.binding_id, &two, &second, &[rule("17")])
        .unwrap();
    assert_ne!(other, id);
    assert_eq!(snapshot(a.binding_id).binding.profile_id, Some(id));
    assert_eq!(
        snapshot(a.binding_id)
            .profiles
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .rules,
        [rule("8")]
    );
    store.revoke(b.binding_id).unwrap();
    assert!(
        store
            .configure_toolchain(b.binding_id, &two, &second, &[rule("21")])
            .is_err()
    );
    assert!(!snapshot(b.binding_id).has_grant(Capability::ToolchainInspect, UtcDateTime::now()));
}
