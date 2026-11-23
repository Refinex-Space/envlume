use envlume_domain::*;

#[test]
fn soft_delete_revokes_all_and_only_the_projects_bindings_and_grants() {
    let at = UtcDateTime::now();
    let mut project = Project {
        id: ProjectId::generate(),
        name: "orders".into(),
        description: String::new(),
        created_at: at,
        updated_at: at,
        deleted_at: None,
    };
    let profile_id = ProfileId::generate();
    let binding = |project_id, name: &str| WorkspaceBinding {
        alias: name.to_owned().try_into().unwrap(),
        id: BindingId::generate(),
        project_id,
        canonical_path: std::env::temp_dir().join(name),
        git_fingerprint: Some("shared-origin".into()),
        profile_id: Some(profile_id),
        state: BindingState::Trusted,
    };
    let mut bindings = [
        binding(project.id, "main"),
        binding(project.id, "worktree"),
        binding(ProjectId::generate(), "other"),
    ];
    let mut grants = bindings.each_ref().map(|b| CapabilityGrant {
        id: GrantId::generate(),
        binding_id: b.id,
        capability: Capability::TaskInspect,
        expires_at: None,
        revoked_at: None,
    });
    let other = bindings[2].clone();
    project.soft_delete(&mut bindings, &mut grants, at);
    project.soft_delete(&mut bindings, &mut grants, at + time::Duration::seconds(10));
    assert_eq!(project.deleted_at, Some(at));
    assert!(
        bindings[..2]
            .iter()
            .all(|b| b.state == BindingState::Revoked(at))
    );
    assert!(grants[..2].iter().all(|g| g.revoked_at == Some(at)));
    assert_eq!(bindings[2], other);
    assert_eq!(grants[2].revoked_at, None);
}
