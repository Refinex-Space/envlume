CREATE TABLE credential_cleanup (
    reference TEXT PRIMARY KEY CHECK(length(reference) BETWEEN 1 AND 256),
    project_id BLOB NOT NULL REFERENCES projects(id)
) STRICT;
CREATE INDEX resource_credential_refs ON resource_bindings(credential_ref) WHERE credential_ref IS NOT NULL;

-- Covers rotation, clear, removal, and the T09 endpoint-change trigger.
CREATE TRIGGER queue_detached_credential AFTER UPDATE OF credential_ref ON resource_bindings
WHEN OLD.credential_ref GLOB 'envlume:v1:*' AND OLD.credential_ref IS NOT NEW.credential_ref
BEGIN
    INSERT OR IGNORE INTO credential_cleanup(reference,project_id)
        SELECT OLD.credential_ref,project_id FROM environment_targets WHERE id=OLD.target_id;
END;
CREATE TRIGGER queue_removed_credential AFTER DELETE ON resource_bindings
WHEN OLD.credential_ref GLOB 'envlume:v1:*'
BEGIN
    INSERT OR IGNORE INTO credential_cleanup(reference,project_id)
        SELECT OLD.credential_ref,project_id FROM environment_targets WHERE id=OLD.target_id;
END;
