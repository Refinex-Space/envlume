mod support;

use envlume_application::{
    ErrorCode, Session,
    input::{TOOL_NAMES, ToolCall},
    result::{Cell, QueryResult},
};
use envlume_domain::*;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeSet};
use support::*;

fn catalog() -> Vec<Value> {
    serde_json::from_str(include_str!("tools.json")).unwrap()
}
fn examples() -> Value {
    serde_json::from_str(include_str!("examples.json")).unwrap()
}

#[test]
fn rust_decoder_enforces_all_eight_published_input_boundaries() {
    let catalog = catalog();
    let fixtures = examples();
    let names = catalog
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(names, TOOL_NAMES.into_iter().collect());
    assert_eq!(names.len(), catalog.len());
    for tool in catalog {
        let name = tool["name"].as_str().unwrap();
        let schema = &tool["inputSchema"];
        let valid = &fixtures[name]["arguments"];
        let check = |args: Value, accepted| {
            assert_eq!(
                ToolCall::parse(name, args.clone()).is_ok(),
                accepted,
                "Rust {name}: {args}"
            );
        };
        assert_eq!(schema["additionalProperties"], false);
        check(valid.clone(), true);
        for bad in [Value::Null, json!([]), json!("text"), json!(0)] {
            check(bad, false);
        }
        for key in [
            "projectId",
            "bindingId",
            "cwd",
            "path",
            "host",
            "port",
            "connectionString",
            "headers",
            "capability",
            "limit",
        ] {
            let mut args = valid.clone();
            args[key] = json!("untrusted");
            check(args, false);
        }
        for field in schema["required"].as_array().unwrap() {
            let mut args = valid.clone();
            args.as_object_mut()
                .unwrap()
                .remove(field.as_str().unwrap());
            check(args, false);
        }
        for (field, rule) in schema["properties"].as_object().unwrap() {
            let with = |value: Value| {
                let mut args = valid.clone();
                args[field] = value;
                args
            };
            let max = rule["maxLength"].as_u64().unwrap() as usize;
            check(with(json!("a".repeat(max))), true);
            check(with(json!("a".repeat(max + 1))), false);
            for value in [Value::Null, json!(42), json!("")] {
                check(with(value), false);
            }
            if field == "sql" {
                check(with(json!(" \n ")), false);
                check(with(json!("列".repeat(max))), true);
            } else {
                for invalid in [
                    "/tmp/orders",
                    "C:\\work",
                    "../other",
                    "mysql://db:3306",
                    "10.0.0.1",
                    "db.example",
                    "bad\n",
                    "a b",
                ] {
                    check(with(json!(invalid)), false);
                }
            }
        }
    }
    assert!(ToolCall::parse("get_secret", json!({})).is_err());
    assert!(
        ToolCall::parse(
            "database_schema",
            json!({"target":"dev","resource":"primary-db"})
        )
        .is_ok()
    );
}

#[test]
fn each_tool_requires_its_exact_grant_and_returns_the_frozen_projection() {
    let fixtures = examples();
    for tool in catalog() {
        let name = tool["name"].as_str().unwrap();
        let repo = Memory(RefCell::new(state()));
        let binding = repo.0.borrow().binding.id;
        let cases = FixtureCases::default();
        let session =
            Session::from_registration(&repo, binding, Actor::Mcp(ClientKind::Codex)).unwrap();
        let args = fixtures[name]["arguments"].clone();
        let expected = tool["capability"]
            .as_str()
            .unwrap()
            .replace("<id>", if name == "task_run" { TASK } else { RESOURCE });
        // A registry with every *other* permission must still refuse this operation.
        let grants = repo.0.borrow().grants.clone();
        repo.0
            .borrow_mut()
            .grants
            .retain(|g| g.capability.wire_name() != expected);
        let denied = session.invoke(&cases, name, args.clone());
        assert_eq!(denied.error_code, Some(ErrorCode::PolicyDenied), "{name}");
        assert_eq!(cases.calls.get(), 0);
        repo.0.borrow_mut().grants = grants;
        let report = session.invoke(&cases, name, args);
        assert_ne!(report.request_id, denied.request_id);
        assert_eq!(report.exit_code(), 0, "{name}");
        assert_eq!(report.capability.as_deref(), Some(expected.as_str()));
        assert_eq!(
            report.evidence_id.is_some(),
            matches!(name, "task_run" | "database_query" | "connection_test")
        );
        let wire = serde_json::to_value(report).unwrap();
        let result = &wire["result"];
        assert_eq!(result, &fixtures[name]["result"]);
        for private in ["private.invalid", "fixture-reference", "canonicalPath"] {
            assert!(!wire.to_string().contains(private));
        }
        assert_eq!(cases.calls.get(), 1);
    }
}

#[test]
fn target_switch_uses_distinct_registered_resources_and_keeps_the_workspace_profile() {
    let repo = Memory(RefCell::new(state()));
    let binding = repo.0.borrow().binding.clone();
    let profile = repo.0.borrow().profiles[0].clone();
    let session = Session::from_registration(&repo, binding.id, Actor::Desktop).unwrap();
    let cases = FixtureCases::default();
    let args = |target| json!({"target":target,"resource":"primary-db"});
    assert_eq!(
        session
            .invoke(&cases, "database_schema", args("dev"))
            .exit_code(),
        0
    );
    assert_eq!(
        session
            .invoke(&cases, "database_schema", args("test"))
            .exit_code(),
        3
    );
    let mut state = repo.0.borrow_mut();
    let scope = ResourceScope {
        target: state.targets[1].id,
        resource: state.resources[1].id,
    };
    let mut grant = state.grants[0].clone();
    grant.id = GrantId::generate();
    grant.capability = Capability::DatabaseSchema(scope);
    state.grants.push(grant);
    drop(state);
    let switched = session.invoke(&cases, "database_schema", args("test"));
    assert_eq!(switched.exit_code(), 0);
    assert_eq!(
        switched.capability.as_deref(),
        Some(Capability::DatabaseSchema(scope).wire_name().as_str())
    );
    assert_eq!(repo.0.borrow().binding.profile_id, binding.profile_id);
    assert_eq!(repo.0.borrow().profiles[0], profile);
    assert_eq!(
        session
            .invoke(&cases, "toolchain_doctor", json!({"profile":"default"}))
            .exit_code(),
        0
    );
}

#[test]
fn registration_and_each_invocation_fail_closed() {
    let repo = Memory(RefCell::new(state()));
    let original = repo.0.borrow().clone();
    let binding = original.binding.id;
    let cases = FixtureCases::default();
    assert!(matches!(
        Session::from_registration(&repo, BindingId::generate(), Actor::Cli),
        Err(ErrorCode::ProjectNotTrusted)
    ));
    let session = Session::from_registration(&repo, binding, Actor::Cli).unwrap();
    let now = UtcDateTime::now();
    let check = |mutate: &dyn Fn(&mut envlume_application::ports::BindingSnapshot), error| {
        let mut changed = original.clone();
        mutate(&mut changed);
        repo.0.replace(changed);
        let report = session.invoke(&cases, "project_context", json!({}));
        assert_eq!(report.error_code, Some(error));
        assert_eq!(report.exit_code(), 3);
    };
    check(
        &|s| s.binding.state = BindingState::Untrusted,
        ErrorCode::ProjectNotTrusted,
    );
    check(
        &|s| s.binding.state = BindingState::Stale,
        ErrorCode::ProjectNotTrusted,
    );
    check(
        &|s| s.binding.state = BindingState::Revoked(now),
        ErrorCode::BindingRevoked,
    );
    check(
        &|s| s.project.deleted_at = Some(now),
        ErrorCode::ProjectNotTrusted,
    );
    check(
        &|s| s.grants[0].revoked_at = Some(now),
        ErrorCode::PolicyDenied,
    );
    check(
        &|s| s.grants[0].expires_at = Some(now),
        ErrorCode::PolicyDenied,
    );
    check(
        &|s| s.grants[0].binding_id = BindingId::generate(),
        ErrorCode::PolicyDenied,
    );
    check(
        &|s| {
            s.project.id = ProjectId::generate();
            s.binding.project_id = s.project.id;
        },
        ErrorCode::ProjectNotTrusted,
    );
    repo.0.replace(original);
    let args = json!({"target":"dev","resource":"primary-db"});
    repo.0.borrow_mut().targets[0].project_id = ProjectId::generate();
    assert_eq!(
        session.invoke(&cases, "connection_test", args).error_code,
        Some(ErrorCode::TargetNotAllowed)
    );
    assert_eq!(cases.calls.get(), 0);
}

#[test]
fn cli_json_human_text_and_all_seven_exit_codes_match_the_snapshots() {
    use time::format_description::well_known::Rfc3339;
    let fixtures: Vec<Value> = serde_json::from_str(include_str!("cli.json")).unwrap();
    let examples = examples();
    let codes = [
        None,
        Some(ErrorCode::InvalidInput),
        Some(ErrorCode::PolicyDenied),
        Some(ErrorCode::ToolchainMissing),
        Some(ErrorCode::NetworkUnreachable),
        Some(ErrorCode::Timeout),
        Some(ErrorCode::InternalError),
    ];
    assert_eq!(fixtures.len(), codes.len());
    for (fixture, code) in fixtures.iter().zip(codes) {
        let expected = &fixture["json"];
        let name = expected["operation"].as_str().unwrap();
        let repo = Memory(RefCell::new(state()));
        let binding = repo.0.borrow().binding.id;
        let session = Session::from_registration(&repo, binding, Actor::Cli).unwrap();
        let cases = FixtureCases::default();
        if code == Some(ErrorCode::PolicyDenied) {
            repo.0.borrow_mut().grants.clear();
        } else {
            cases.failure.set(code);
        }
        let report = session.invoke(&cases, name, examples[name]["arguments"].clone());
        assert_eq!(
            report.exit_code(),
            fixture["exitCode"].as_u64().unwrap() as u8
        );
        let mut actual = serde_json::to_value(&report).unwrap();
        let timestamp = actual["timestampUtc"].as_str().unwrap();
        assert!(timestamp.ends_with('Z'));
        assert!(UtcDateTime::parse(timestamp, &Rfc3339).is_ok());
        assert!(actual["durationMs"].is_u64());
        let human = report.human_text().replace(
            actual["requestId"].as_str().unwrap(),
            expected["requestId"].as_str().unwrap(),
        );
        assert_eq!(human, fixture["human"].as_str().unwrap());
        for dynamic in ["requestId", "durationMs", "timestampUtc"] {
            actual[dynamic] = expected[dynamic].clone();
        }
        assert_eq!(&actual, expected);
    }
}

#[test]
fn query_projection_rejects_excess_rows_and_mismatched_columns() {
    assert!(QueryResult::new(vec![], (0..201).map(|_| vec![]).collect(), true).is_err());
    assert!(QueryResult::new(vec![], vec![vec![Cell::Null]], false).is_err());
}

#[test]
fn direct_calls_cannot_override_identity_or_pass_a_raw_endpoint_to_a_use_case() {
    let repo = Memory(RefCell::new(state()));
    let binding = repo.0.borrow().binding.id;
    let cases = FixtureCases::default();
    for actor in [Actor::Desktop, Actor::Cli, Actor::Mcp(ClientKind::Cursor)] {
        let session = Session::from_registration(&repo, binding, actor).unwrap();
        for (name, arguments) in [
            ("project_context", json!({"bindingId":binding})),
            ("task_run", json!({"taskId":"../mvn"})),
            (
                "connection_test",
                json!({"target":"dev","resource":"mysql://private.invalid:3306"}),
            ),
            (
                "database_query",
                json!({"target":"dev","resource":"primary-db","sql":"SELECT 1","cwd":"/tmp/foreign"}),
            ),
        ] {
            let report = session.invoke(&cases, name, arguments);
            assert_eq!(report.error_code, Some(ErrorCode::InvalidInput));
            assert_eq!(report.operation, name);
            assert!(report.binding_alias.is_none());
            assert!(
                !serde_json::to_string(&report)
                    .unwrap()
                    .contains("private.invalid")
            );
        }
    }
    assert_eq!(cases.calls.get(), 0);
}

#[test]
fn changing_resource_risk_or_disabling_a_task_overrides_a_live_grant() {
    let repo = Memory(RefCell::new(state()));
    let binding = repo.0.borrow().binding.id;
    let session = Session::from_registration(&repo, binding, Actor::Cli).unwrap();
    let cases = FixtureCases::default();
    let fixtures = examples();
    let query = || {
        session
            .invoke(
                &cases,
                "database_query",
                fixtures["database_query"]["arguments"].clone(),
            )
            .error_code
    };
    repo.0.borrow_mut().targets[0].risk = RiskLevel::Critical;
    assert_eq!(query(), Some(ErrorCode::TargetNotAllowed));
    {
        let mut state = repo.0.borrow_mut();
        state.targets[0].risk = RiskLevel::Low;
        state.targets[0].kind = EnvironmentKind::Prod;
    }
    assert_eq!(query(), Some(ErrorCode::TargetNotAllowed));
    repo.0.borrow_mut().tasks[0].enabled = false;
    assert_eq!(
        session
            .invoke(&cases, "task_run", json!({"taskId":"maven-test"}))
            .error_code,
        Some(ErrorCode::PolicyDenied)
    );
    assert_eq!(cases.calls.get(), 0);
}

#[test]
fn listing_metadata_does_not_grant_access_to_the_listed_objects() {
    let repo = Memory(RefCell::new(state()));
    let binding = repo.0.borrow().binding.id;
    let session = Session::from_registration(&repo, binding, Actor::Cli).unwrap();
    repo.0.borrow_mut().grants.retain(|g| {
        matches!(
            g.capability,
            Capability::TaskInspect | Capability::EnvironmentInspect
        )
    });
    let cases = FixtureCases::default();
    for (name, field) in [("task_list", "tasks"), ("environment_list", "environments")] {
        let report = session.invoke(&cases, name, json!({}));
        assert_eq!(report.exit_code(), 0);
        assert_eq!(
            serde_json::to_value(report).unwrap()["result"][field],
            json!([])
        );
    }
    assert_eq!(
        session
            .invoke(&cases, "task_run", json!({"taskId":"maven-test"}))
            .error_code,
        Some(ErrorCode::PolicyDenied)
    );
}

#[test]
fn a_staging_read_grant_does_not_make_the_endpoint_visible() {
    let mut data = state();
    data.targets[0].kind = EnvironmentKind::Staging;
    data.grants.retain(|g| {
        matches!(
            g.capability,
            Capability::EnvironmentInspect | Capability::DatabaseRead(_)
        )
    });
    let binding = data.binding.id;
    let repo = Memory(RefCell::new(data));
    let session = Session::from_registration(&repo, binding, Actor::Cli).unwrap();
    let report = session.invoke(&FixtureCases::default(), "environment_list", json!({}));
    assert_eq!(
        serde_json::to_value(report).unwrap()["result"]["environments"],
        json!([])
    );
}

#[test]
fn remote_matrix_checks_both_policy_and_scope_before_any_io() {
    let cases = FixtureCases::default();
    for kind in [
        EnvironmentKind::Dev,
        EnvironmentKind::Test,
        EnvironmentKind::Staging,
        EnvironmentKind::Prod,
        EnvironmentKind::Custom("legacy".into()),
    ] {
        for risk in [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ] {
            let mut state = state();
            state.targets.truncate(1);
            state.targets[0].kind = kind.clone();
            state.targets[0].risk = risk;
            state.targets[0].alias = alias("innocent");
            let id = state.binding.id;
            let repo = Memory(RefCell::new(state));
            let session = Session::from_registration(&repo, id, Actor::LocalStdio).unwrap();
            for name in ["connection_test", "database_schema", "database_query"] {
                let mut args = json!({"target":"innocent","resource":"primary-db"});
                if name == "database_query" {
                    args["sql"] = json!("SELECT id FROM orders");
                }
                let expected = risk != RiskLevel::Critical
                    && match kind {
                        EnvironmentKind::Dev | EnvironmentKind::Test => {
                            name != "database_query" || risk != RiskLevel::High
                        }
                        EnvironmentKind::Staging => name != "database_query",
                        _ => false,
                    };
                let before = (cases.secret_checks.get(), cases.connections.get());
                let r = session.invoke(&cases, name, args);
                assert_eq!(r.ok, expected, "{name} {kind:?} {risk:?}");
                if !expected {
                    assert_eq!(r.error_code, Some(ErrorCode::TargetNotAllowed));
                }
                let increment = usize::from(expected);
                assert_eq!(
                    (cases.secret_checks.get(), cases.connections.get()),
                    (before.0 + increment, before.1 + increment)
                );
            }
            for name in ["database_write", "redis_write", "es_write", "ssh_exec"] {
                assert_eq!(
                    session.invoke(&cases, name, json!({})).error_code,
                    Some(ErrorCode::InvalidInput)
                );
            }
            assert!(
                !session
                    .invoke(&cases, "connection_test", json!({"resource":"primary-db"}))
                    .ok
            );
            assert!(!session.invoke(&cases,"database_query",json!({"target":"innocent","resource":"primary-db","sql":"SELECT 1","allowProd":true})).ok);
        }
    }
}

#[test]
fn rejected_resource_requests_do_not_reach_secret_store_or_connector() {
    let original = state();
    let binding = original.binding.id;
    let repo = Memory(RefCell::new(original.clone()));
    let s = Session::from_registration(&repo, binding, Actor::Desktop).unwrap();
    let cases = FixtureCases::default();
    let args = json!({"target":"dev","resource":"primary-db"});
    for mutate in 0..8 {
        let mut snapshot = original.clone();
        let grant = snapshot
            .grants
            .iter_mut()
            .find(|g| matches!(g.capability, Capability::ResourceConnect(_)))
            .unwrap();
        match mutate {
            0 => snapshot.targets[0].project_id = ProjectId::generate(),
            1 => grant.binding_id = BindingId::generate(),
            2 => {
                grant.capability = Capability::ResourceConnect(ResourceScope {
                    target: snapshot.targets[1].id,
                    resource: snapshot.resources[1].id,
                })
            }
            3 => {
                grant.capability = Capability::DatabaseSchema(ResourceScope {
                    target: snapshot.targets[0].id,
                    resource: snapshot.resources[0].id,
                })
            }
            4 => grant.expires_at = Some(UtcDateTime::now()),
            5 => grant.revoked_at = Some(UtcDateTime::now()),
            6 => snapshot.resources[0].endpoint_confirmed = false,
            _ => snapshot.resources[0].status = Status::Disabled,
        }
        *repo.0.borrow_mut() = snapshot;
        assert!(
            !s.invoke(&cases, "connection_test", args.clone()).ok,
            "case {mutate}"
        );
    }
    assert_eq!((cases.secret_checks.get(), cases.connections.get()), (0, 0));
    *repo.0.borrow_mut() = original;
    assert!(s.invoke(&cases, "connection_test", args.clone()).ok);
    repo.0.borrow_mut().grants.clear();
    assert!(!s.invoke(&cases, "connection_test", args).ok);
    assert_eq!((cases.secret_checks.get(), cases.connections.get()), (1, 1));
}

#[test]
fn revocation_after_dispatch_before_provider_access_blocks_io() {
    use envlume_application::ports::{BindingSnapshot, Repository};
    struct RevokeBeforeUse {
        state: BindingSnapshot,
        revoke_at: usize,
        reads: std::cell::Cell<usize>,
    }
    impl Repository for RevokeBeforeUse {
        fn load_binding(&self, id: BindingId) -> Result<Option<BindingSnapshot>, ErrorCode> {
            assert_eq!(self.state.binding.id, id);
            self.reads.set(self.reads.get() + 1);
            let mut snapshot = self.state.clone();
            if self.reads.get() >= self.revoke_at {
                snapshot.grants.clear();
            }
            Ok(Some(snapshot))
        }
    }
    // Registration, dispatch and the endpoint identity check precede the two provider boundaries.
    for (revoke_at, secret_checks) in [(4, 0), (5, 1)] {
        let repo = RevokeBeforeUse {
            state: state(),
            reads: Default::default(),
            revoke_at,
        };
        let session = Session::from_registration(&repo, repo.state.binding.id, Actor::Cli).unwrap();
        let cases = FixtureCases::default();
        let report = session.invoke(
            &cases,
            "connection_test",
            json!({"target":"dev","resource":"primary-db"}),
        );
        assert_eq!(report.error_code, Some(ErrorCode::PolicyDenied));
        assert_eq!(
            (cases.secret_checks.get(), cases.connections.get()),
            (secret_checks, 0)
        );
    }
}
