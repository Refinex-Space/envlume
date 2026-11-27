//! Information schema identifiers are data. SQL text is fixed; all selector values are bound.
use envlume_application::{
    remote::RemoteFault,
    result::{Column, Index, Table},
};
use envlume_domain::MySqlTable;
use std::collections::BTreeMap;

pub(super) const COLUMNS: &str =
    "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE FROM information_schema.COLUMNS
    WHERE BINARY TABLE_SCHEMA = BINARY ? AND BINARY TABLE_NAME = BINARY ?
    AND FIND_IN_SET(BINARY COLUMN_NAME,?)>0
    ORDER BY ORDINAL_POSITION LIMIT 1025";
pub(super) const INDEXES: &str =
    "SELECT INDEX_NAME, COLUMN_NAME, SEQ_IN_INDEX FROM information_schema.STATISTICS
    WHERE BINARY TABLE_SCHEMA = BINARY ? AND BINARY TABLE_NAME = BINARY ?
    AND FIND_IN_SET(BINARY INDEX_NAME,?)>0
    ORDER BY BINARY INDEX_NAME, SEQ_IN_INDEX LIMIT 1025";
pub(super) type ColRow = (String, String, String);
pub(super) type IndexRow = (String, Option<String>, u64);

pub(super) fn project(
    name: &str,
    allowed: &MySqlTable,
    cols: Vec<ColRow>,
    indexes: Vec<IndexRow>,
) -> Result<Option<Table>, RemoteFault> {
    if cols.len() > 1024 || indexes.len() > 1024 {
        return Err(RemoteFault::Limit);
    }
    let columns: Vec<_> = cols
        .into_iter()
        .filter(|(n, _, _)| allowed.columns.contains(n))
        .map(|(name, sql_type, nullable)| {
            if sql_type.len() > 64
                || !sql_type
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b' ')
            {
                return Err(RemoteFault::Service);
            }
            Ok(Column {
                name,
                sql_type,
                nullable: match nullable.as_str() {
                    "YES" => true,
                    "NO" => false,
                    _ => return Err(RemoteFault::Service),
                },
            })
        })
        .collect::<Result<_, _>>()?;
    if columns.is_empty() {
        return Ok(None);
    }
    let mut groups = BTreeMap::<String, Vec<(Option<String>, u64)>>::new();
    for (index, col, position) in indexes {
        groups.entry(index).or_default().push((col, position));
    }
    let indexes = groups
        .into_iter()
        .filter_map(|(name, parts)| {
            // Never advertise a partial composite index, or expressions involving invisible columns.
            if !allowed.indexes.contains(&name) {
                return None;
            }
            let keys = parts
                .into_iter()
                .enumerate()
                .map(|(i, (col, pos))| {
                    col.filter(|n| pos == i as u64 + 1 && columns.iter().any(|c| &c.name == n))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Index {
                name,
                columns: keys,
            })
        })
        .collect();
    Ok(Some(Table {
        table: name.to_owned(),
        columns,
        indexes,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composite_and_expression_indexes_cannot_disclose_hidden_columns() {
        let allow = MySqlTable {
            columns: ["id".into()].into(),
            indexes: ["PRIMARY".into(), "mixed".into(), "expr".into()].into(),
        };
        let cols = vec![
            ("id".into(), "bigint".into(), "NO".into()),
            ("private".into(), "varchar".into(), "YES".into()),
        ];
        let idx = vec![
            ("PRIMARY".into(), Some("id".into()), 1),
            ("mixed".into(), Some("id".into()), 1),
            ("mixed".into(), Some("private".into()), 2),
            ("expr".into(), None, 1),
        ];
        let table = project("example", &allow, cols, idx).unwrap().unwrap();
        assert_eq!(table.columns.len(), 1);
        assert_eq!(table.indexes.len(), 1);
        assert_eq!(table.indexes[0].name, "PRIMARY");
    }
}
