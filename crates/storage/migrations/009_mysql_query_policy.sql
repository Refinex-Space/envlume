ALTER TABLE resource_bindings ADD COLUMN mysql_query_policy TEXT
    CHECK(mysql_query_policy IS NULL OR (length(mysql_query_policy)<=65536 AND json_valid(mysql_query_policy)));

CREATE TRIGGER reconsider_mysql_read AFTER UPDATE OF mysql_query_policy ON resource_bindings
WHEN OLD.mysql_query_policy IS NOT NEW.mysql_query_policy
BEGIN
    UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
    WHERE resource_id=NEW.id AND capability='database.read' AND revoked_at IS NULL;
END;

CREATE TRIGGER forget_mysql_read AFTER UPDATE OF mysql_route,mysql_tables ON resource_bindings
WHEN OLD.mysql_route IS NOT NEW.mysql_route OR OLD.mysql_tables IS NOT NEW.mysql_tables
BEGIN
    UPDATE resource_bindings SET mysql_query_policy=NULL WHERE id=NEW.id;
END;
