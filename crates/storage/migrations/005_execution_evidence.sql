ALTER TABLE execution_records RENAME TO previous_execution_records;
CREATE TABLE execution_records (
    id BLOB PRIMARY KEY CHECK(length(id) = 16),
    request_id BLOB NOT NULL CHECK(length(request_id) = 16),
    evidence_id BLOB UNIQUE CHECK(length(evidence_id) = 16),
    project_id BLOB NOT NULL, binding_id BLOB NOT NULL, target_id BLOB, resource_id BLOB,
    capability TEXT NOT NULL CHECK(length(capability) BETWEEN 1 AND 96),
    actor TEXT NOT NULL CHECK(actor IN ('DESKTOP','CLI','LOCAL_STDIO','CLAUDE_CODE','CODEX','CURSOR')),
    decision TEXT NOT NULL CHECK(decision IN ('SUCCESS','FAILED','DENIED','SKIPPED')),
    sanitized_summary TEXT NOT NULL CHECK(length(sanitized_summary) <= 2048),
    exit_code INTEGER,
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    details_json TEXT CHECK(length(details_json)<=8192 AND json_valid(details_json)),
    FOREIGN KEY(project_id, binding_id) REFERENCES workspace_bindings(project_id, id),
    FOREIGN KEY(project_id, target_id) REFERENCES environment_targets(project_id, id),
    FOREIGN KEY(target_id, resource_id) REFERENCES resource_bindings(target_id, id),
    CHECK(resource_id IS NULL OR target_id IS NOT NULL)
) STRICT;
INSERT INTO execution_records SELECT old.*,NULL FROM previous_execution_records AS old;
DROP TABLE previous_execution_records;
CREATE INDEX execution_by_binding_time ON execution_records(binding_id, created_at);
CREATE INDEX execution_by_request ON execution_records(request_id);
