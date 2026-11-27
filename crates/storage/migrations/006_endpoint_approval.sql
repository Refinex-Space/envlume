ALTER TABLE resource_bindings ADD COLUMN database_name TEXT CHECK(length(database_name) BETWEEN 1 AND 64);
ALTER TABLE resource_bindings ADD COLUMN confirmed_at_ms INTEGER CHECK(confirmed_at_ms>=0);

-- Earlier versions did not record local endpoint approval.
UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
WHERE resource_id IS NOT NULL AND revoked_at IS NULL;

CREATE TRIGGER reconsider_resource AFTER UPDATE OF host,port,database_name,tls_mode,target_id ON resource_bindings
WHEN OLD.host IS NOT NEW.host OR OLD.port IS NOT NEW.port OR OLD.database_name IS NOT NEW.database_name
  OR OLD.tls_mode IS NOT NEW.tls_mode OR OLD.target_id IS NOT NEW.target_id
BEGIN
    UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
    WHERE resource_id=OLD.id AND revoked_at IS NULL;
    UPDATE resource_bindings SET confirmed_at_ms=NULL,credential_ref=NULL WHERE id=NEW.id;
END;

CREATE TRIGGER reconsider_target AFTER UPDATE OF kind,risk_level,status ON environment_targets
WHEN OLD.kind IS NOT NEW.kind OR OLD.risk_level IS NOT NEW.risk_level OR OLD.status IS NOT NEW.status
BEGIN
    UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
    WHERE target_id=NEW.id AND revoked_at IS NULL;
END;

CREATE TRIGGER retire_disabled_resource AFTER UPDATE OF status ON resource_bindings
WHEN OLD.status IS NOT NEW.status
BEGIN
    UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
    WHERE resource_id=NEW.id AND revoked_at IS NULL;
END;
