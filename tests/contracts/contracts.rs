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
