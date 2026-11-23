-- UUIDs use their 16-byte representation. Times are UTC Unix milliseconds.
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK(version > 0),
    applied_at INTEGER NOT NULL CHECK(applied_at >= 0)
) STRICT;

CREATE TABLE projects (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    name TEXT NOT NULL CHECK(length(trim(name)) BETWEEN 1 AND 128),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    updated_at INTEGER NOT NULL CHECK(updated_at >= created_at),
    deleted_at INTEGER CHECK(deleted_at >= created_at)
) STRICT;
CREATE INDEX projects_visible ON projects(name, id) WHERE deleted_at IS NULL;

CREATE TABLE toolchain_profiles (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    project_id BLOB NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64 AND name GLOB '[a-z]*' AND name NOT GLOB '*[^a-z0-9_-]*'),
    rules_json TEXT NOT NULL CHECK(length(rules_json) <= 65536 AND json_type(rules_json) = 'array'),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    UNIQUE(project_id, name), UNIQUE(project_id, id)
) STRICT;

CREATE TABLE workspace_bindings (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    project_id BLOB NOT NULL REFERENCES projects(id),
    alias TEXT NOT NULL CHECK(length(alias) BETWEEN 1 AND 64 AND alias GLOB '[a-z]*' AND alias NOT GLOB '*[^a-z0-9_-]*'),
    canonical_path TEXT NOT NULL CHECK(length(canonical_path) BETWEEN 1 AND 32767),
    git_fingerprint TEXT CHECK(length(git_fingerprint) BETWEEN 1 AND 256),
    profile_id BLOB,
    trust_state TEXT NOT NULL CHECK(trust_state IN ('UNTRUSTED','TRUSTED','STALE','REVOKED')),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    revoked_at INTEGER CHECK(revoked_at >= created_at),
    CHECK((trust_state = 'REVOKED') = (revoked_at IS NOT NULL)),
    FOREIGN KEY(project_id, profile_id) REFERENCES toolchain_profiles(project_id, id),
    UNIQUE(project_id, id), UNIQUE(project_id, alias)
) STRICT;
CREATE UNIQUE INDEX active_workspace_path ON workspace_bindings(canonical_path) WHERE revoked_at IS NULL;

CREATE TABLE environment_targets (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    project_id BLOB NOT NULL REFERENCES projects(id),
    alias TEXT NOT NULL CHECK(length(alias) BETWEEN 1 AND 64 AND alias GLOB '[a-z]*' AND alias NOT GLOB '*[^a-z0-9_-]*'),
    kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 64),
    risk_level TEXT NOT NULL CHECK(risk_level IN ('LOW','MEDIUM','HIGH','CRITICAL')),
    is_default INTEGER NOT NULL CHECK(is_default IN (0,1) AND (is_default = 0 OR (kind <> 'PROD' AND risk_level <> 'CRITICAL'))),
    status TEXT NOT NULL CHECK(status IN ('ACTIVE','DISABLED')),
    UNIQUE(project_id, alias), UNIQUE(project_id, id)
) STRICT;
CREATE UNIQUE INDEX default_target ON environment_targets(project_id) WHERE is_default = 1;

CREATE TABLE resource_bindings (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    target_id BLOB NOT NULL REFERENCES environment_targets(id),
    alias TEXT NOT NULL CHECK(length(alias) BETWEEN 1 AND 64 AND alias GLOB '[a-z]*' AND alias NOT GLOB '*[^a-z0-9_-]*'),
    kind TEXT NOT NULL CHECK(kind = 'MYSQL'),
    host TEXT NOT NULL CHECK(length(host) BETWEEN 1 AND 253 AND host NOT GLOB '*[^a-zA-Z0-9.:-]*'),
    port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
    tls_mode TEXT NOT NULL CHECK(tls_mode IN ('VERIFY_IDENTITY','DISABLED')),
    credential_ref TEXT CHECK(length(credential_ref) BETWEEN 1 AND 256),
    policy_labels TEXT NOT NULL CHECK(length(policy_labels) <= 4096 AND json_type(policy_labels) = 'array'),
    status TEXT NOT NULL CHECK(status IN ('ACTIVE','DISABLED')),
    UNIQUE(target_id, alias), UNIQUE(target_id, id)
) STRICT;

CREATE TABLE task_definitions (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    project_id BLOB NOT NULL REFERENCES projects(id),
    binding_id BLOB,
    task_key TEXT NOT NULL CHECK(length(task_key) BETWEEN 1 AND 64 AND task_key GLOB '[a-z]*' AND task_key NOT GLOB '*[^a-z0-9_-]*'),
    executable TEXT NOT NULL CHECK(length(executable) BETWEEN 1 AND 32767),
    arguments TEXT NOT NULL CHECK(length(arguments) <= 65536 AND json_type(arguments) = 'array'),
    workspace_relative_dir TEXT NOT NULL CHECK(length(workspace_relative_dir) BETWEEN 1 AND 32767),
    timeout_seconds INTEGER NOT NULL CHECK(timeout_seconds > 0),
    input TEXT NOT NULL CHECK(input = 'CLOSED'),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    FOREIGN KEY(project_id, binding_id) REFERENCES workspace_bindings(project_id, id),
    UNIQUE(project_id, task_key), UNIQUE(project_id, id)
) STRICT;

CREATE TABLE capability_grants (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    project_id BLOB NOT NULL,
    binding_id BLOB NOT NULL,
    target_id BLOB, resource_id BLOB, task_id BLOB,
    capability TEXT NOT NULL,
    expires_at INTEGER CHECK(expires_at >= 0),
    revoked_at INTEGER CHECK(revoked_at >= 0),
    FOREIGN KEY(project_id, binding_id) REFERENCES workspace_bindings(project_id, id),
    FOREIGN KEY(project_id, target_id) REFERENCES environment_targets(project_id, id),
    FOREIGN KEY(target_id, resource_id) REFERENCES resource_bindings(target_id, id),
    FOREIGN KEY(project_id, task_id) REFERENCES task_definitions(project_id, id),
    CHECK(
        (capability IN ('project.inspect','environment.inspect','toolchain.inspect','task.inspect')
            AND target_id IS NULL AND resource_id IS NULL AND task_id IS NULL) OR
        (capability = 'task.run' AND task_id IS NOT NULL AND target_id IS NULL AND resource_id IS NULL) OR
        (capability IN ('resource.connect','database.schema','database.read')
            AND target_id IS NOT NULL AND resource_id IS NOT NULL AND task_id IS NULL)
    )
) STRICT;
CREATE UNIQUE INDEX live_grant ON capability_grants(binding_id, capability, ifnull(resource_id,X''), ifnull(task_id,X'')) WHERE revoked_at IS NULL;

CREATE TABLE client_integrations (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    binding_id BLOB NOT NULL REFERENCES workspace_bindings(id),
    client_kind TEXT NOT NULL CHECK(client_kind IN ('CLAUDE_CODE','CODEX','CURSOR')),
    config_location TEXT NOT NULL CHECK(length(config_location) BETWEEN 1 AND 32767),
    config_fingerprint TEXT NOT NULL CHECK(length(config_fingerprint) BETWEEN 1 AND 256),
    status TEXT NOT NULL CHECK(status IN ('INSTALLED','STALE','REMOVED')),
    UNIQUE(binding_id, client_kind)
) STRICT;

CREATE TABLE execution_records (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    request_id BLOB NOT NULL CHECK(length(request_id) = 16),
    evidence_id BLOB UNIQUE CHECK(length(evidence_id) = 16),
    project_id BLOB NOT NULL, binding_id BLOB NOT NULL, target_id BLOB, resource_id BLOB,
    capability TEXT NOT NULL CHECK(length(capability) BETWEEN 1 AND 96),
    actor TEXT NOT NULL CHECK(actor IN ('DESKTOP','CLI','CLAUDE_CODE','CODEX','CURSOR')),
    decision TEXT NOT NULL CHECK(decision IN ('SUCCESS','FAILED','DENIED','SKIPPED')),
    sanitized_summary TEXT NOT NULL CHECK(length(sanitized_summary) <= 2048),
    exit_code INTEGER,
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    FOREIGN KEY(project_id, binding_id) REFERENCES workspace_bindings(project_id, id),
    FOREIGN KEY(project_id, target_id) REFERENCES environment_targets(project_id, id),
    FOREIGN KEY(target_id, resource_id) REFERENCES resource_bindings(target_id, id),
    CHECK(resource_id IS NULL OR target_id IS NOT NULL)
) STRICT;
CREATE INDEX execution_by_binding_time ON execution_records(binding_id, created_at);
CREATE INDEX execution_by_request ON execution_records(request_id);
