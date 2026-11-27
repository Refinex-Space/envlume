pub(crate) const STEPS: &[&str] = &[
    include_str!("../migrations/001_baseline.sql"),
    include_str!("../migrations/002_project_description.sql"),
    include_str!("../migrations/003_workspace_trust.sql"),
    include_str!("../migrations/004_task_approval.sql"),
    include_str!("../migrations/005_execution_evidence.sql"),
    include_str!("../migrations/006_endpoint_approval.sql"),
    include_str!("../migrations/007_credential_cleanup.sql"),
    include_str!("../migrations/008_mysql_route.sql"),
    include_str!("../migrations/009_mysql_query_policy.sql"),
    include_str!("../migrations/010_integration_transactions.sql"),
];
