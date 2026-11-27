ALTER TABLE resource_bindings ADD COLUMN mysql_route TEXT CHECK(mysql_route IS NULL OR (length(mysql_route)<=24576 AND json_valid(mysql_route)));
ALTER TABLE resource_bindings ADD COLUMN mysql_tables TEXT NOT NULL DEFAULT '{}' CHECK(length(mysql_tables)<=65536 AND json_valid(mysql_tables));

CREATE TRIGGER reconsider_mysql_policy AFTER UPDATE OF mysql_route,mysql_tables ON resource_bindings
WHEN OLD.mysql_route IS NOT NEW.mysql_route OR OLD.mysql_tables IS NOT NEW.mysql_tables
BEGIN
    UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
    WHERE resource_id=NEW.id AND revoked_at IS NULL;
    UPDATE resource_bindings SET credential_ref=NULL WHERE id=NEW.id AND OLD.mysql_route IS NOT NEW.mysql_route;
END;

CREATE TRIGGER forget_mysql_policy AFTER UPDATE OF host,port,database_name,tls_mode,target_id ON resource_bindings
WHEN OLD.host IS NOT NEW.host OR OLD.port IS NOT NEW.port OR OLD.database_name IS NOT NEW.database_name
  OR OLD.tls_mode IS NOT NEW.tls_mode OR OLD.target_id IS NOT NEW.target_id
BEGIN
    UPDATE resource_bindings SET mysql_route=NULL,mysql_tables='{}' WHERE id=NEW.id;
END;
