//! Compile the supported MySQL SELECT grammar into a fresh parameterized statement.
use envlume_domain::QueryPolicy;
use sqlparser::{
    ast::*,
    dialect::MySqlDialect,
    parser::Parser,
    tokenizer::{Token, Tokenizer, Whitespace},
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Denied;

pub enum Parameter {
    Integer(i64),
    Text(String),
}
pub struct ReadQuery {
    statement: String,
    parameters: Vec<Parameter>,
    columns: Vec<String>,
    masked: usize,
}
impl ReadQuery {
    pub fn statement(&self) -> &str {
        &self.statement
    }
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }
    pub fn columns(&self) -> &[String] {
        &self.columns
    }
    pub fn masked(&self) -> usize {
        self.masked
    }
}

fn id(i: &Ident) -> Result<&str, Denied> {
    let s = &i.value;
    if !matches!(i.quote_style, None | Some('`'))
        || s.is_empty()
        || s.len() > 64
        || !s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || s.as_bytes()[0].is_ascii_digit()
    {
        return Err(Denied);
    }
    Ok(s)
}
fn quoted(s: &str) -> String {
    format!("`{s}`")
}
fn parse(sql: &str) -> Result<Box<Query>, Denied> {
    if sql.len() > 32768 {
        return Err(Denied);
    }
    let dialect = MySqlDialect {};
    let mut tokens = Tokenizer::new(&dialect, sql)
        .tokenize()
        .map_err(|_| Denied)?;
    if tokens.iter().any(|t| {
        matches!(
            t,
            Token::Whitespace(
                Whitespace::MultiLineComment(_) | Whitespace::SingleLineComment { .. }
            )
        )
    }) {
        return Err(Denied);
    }
    tokens.retain(|t| !matches!(t, Token::Whitespace(_)));
    if matches!(tokens.last(), Some(Token::SemiColon)) {
        tokens.pop();
    }
    if tokens.len() > 192 || tokens.contains(&Token::SemiColon) {
        return Err(Denied);
    }
    let mut stmts = Parser::new(&dialect)
        .with_recursion_limit(24)
        .with_tokens(tokens)
        .parse_statements()
        .map_err(|_| Denied)?;
    match stmts.pop() {
        Some(Statement::Query(q)) if stmts.is_empty() => Ok(q),
        _ => Err(Denied),
    }
}
struct Source {
    sql: String,
    alias: String,
    columns: BTreeMap<String, bool>,
}
impl Source {
    fn column<'a>(&self, e: &'a Expr) -> Result<(&'a str, bool), Denied> {
        let name = match e {
            Expr::Identifier(i) => id(i)?,
            Expr::CompoundIdentifier(parts) => match parts.as_slice() {
                [a, i] if id(a)? == self.alias => id(i)?,
                _ => return Err(Denied),
            },
            _ => return Err(Denied),
        };
        Ok((name, *self.columns.get(name).ok_or(Denied)?))
    }
    fn reference(&self, name: &str) -> String {
        format!("{}.{}", quoted(&self.alias), quoted(name))
    }
}
struct Compiler<'a> {
    policy: &'a QueryPolicy,
    db: &'a str,
    baseline: Box<Query>,
    params: Vec<Parameter>,
}
struct Projection {
    sql: String,
    columns: Vec<(String, bool)>,
}
fn alias_name(a: &TableAlias) -> Result<&str, Denied> {
    if !a.columns.is_empty() || a.at.is_some() {
        return Err(Denied);
    }
    id(&a.name)
}
impl Compiler<'_> {
    fn source(&mut self, f: &TableFactor) -> Result<Source, Denied> {
        match f {
            TableFactor::Table {
                name,
                alias,
                args: None,
                with_hints,
                version: None,
                with_ordinality: false,
                partitions,
                json_path: None,
                sample: None,
                index_directives,
                inheritance: None,
            } if with_hints.is_empty() && partitions.is_empty() && index_directives.is_empty() => {
                let parts = name
                    .0
                    .iter()
                    .map(|p| id(p.as_ident().ok_or(Denied)?))
                    .collect::<Result<Vec<_>, _>>()?;
                let t = match parts.as_slice() {
                    [t] => *t,
                    [d, t] if *d == self.db => *t,
                    _ => return Err(Denied),
                };
                let policy = self.policy.tables.get(t).ok_or(Denied)?;
                let a = alias.as_ref().map(alias_name).transpose()?.unwrap_or(t);
                let columns = policy
                    .readable
                    .iter()
                    .map(|s| (s.clone(), false))
                    .chain(policy.masked.iter().map(|s| (s.clone(), true)))
                    .collect();
                Ok(Source {
                    sql: format!("{}.{} AS {}", quoted(self.db), quoted(t), quoted(a)),
                    alias: a.into(),
                    columns,
                })
            }
            TableFactor::Derived {
                lateral: false,
                subquery,
                alias: Some(a),
                sample: None,
            } => {
                let a = alias_name(a)?;
                let p = self.select(subquery, false)?;
                Ok(Source {
                    sql: format!("({}) AS {}", p.sql, quoted(a)),
                    alias: a.into(),
                    columns: p.columns.into_iter().collect(),
                })
            }
            _ => Err(Denied),
        }
    }
    fn predicate(&mut self, e: &Expr, source: &Source) -> Result<String, Denied> {
        match e {
            Expr::BinaryOp { left, op, right } => {
                let op = match op {
                    BinaryOperator::Eq => "=",
                    BinaryOperator::NotEq => "<>",
                    BinaryOperator::Gt => ">",
                    BinaryOperator::GtEq => ">=",
                    BinaryOperator::Lt => "<",
                    BinaryOperator::LtEq => "<=",
                    BinaryOperator::And => "AND",
                    BinaryOperator::Or => "OR",
                    _ => return Err(Denied),
                };
                Ok(format!(
                    "({} {} {})",
                    self.predicate(left, source)?,
                    op,
                    self.predicate(right, source)?
                ))
            }
            Expr::Nested(v) => self.predicate(v, source),
            Expr::Value(v) => {
                self.params.push(match &v.value {
                    Value::SingleQuotedString(s) => Parameter::Text(s.clone()),
                    Value::Number(n, false) => Parameter::Integer(n.parse().map_err(|_| Denied)?),
                    _ => return Err(Denied),
                });
                Ok("?".into())
            }
            _ => {
                let (n, masked) = source.column(e)?;
                if masked {
                    return Err(Denied);
                }
                Ok(source.reference(n))
            }
        }
    }
    fn select(&mut self, q: &Query, outer: bool) -> Result<Projection, Denied> {
        // Compare all remaining AST fields against a plain SELECT, so new dialect clauses fail closed.
        let mut plain = (*self.baseline).clone();
        plain.body = q.body.clone();
        plain.order_by = q.order_by.clone();
        plain.limit_clause = q.limit_clause.clone();
        if plain != *q {
            return Err(Denied);
        }
        let (SetExpr::Select(select), SetExpr::Select(base)) =
            (q.body.as_ref(), self.baseline.body.as_ref())
        else {
            return Err(Denied);
        };
        let mut acceptable = (**base).clone();
        acceptable.projection = select.projection.clone();
        acceptable.from = select.from.clone();
        acceptable.selection = select.selection.clone();
        if acceptable != **select {
            return Err(Denied);
        }
        let [from] = select.from.as_slice() else {
            return Err(Denied);
        };
        if !from.joins.is_empty() {
            return Err(Denied);
        }
        let source = self.source(&from.relation)?;
        let mut columns = Vec::new();
        let mut exprs = Vec::new();
        let mut add = |value: String, label: &str, masked: bool| -> Result<(), Denied> {
            if columns.len() >= 32
                || columns
                    .iter()
                    .any(|(n, _): &(String, bool)| n.eq_ignore_ascii_case(label))
            {
                return Err(Denied);
            }
            columns.push((label.to_owned(), masked));
            exprs.push(if masked { "'[REDACTED]'".into() } else { value });
            Ok(())
        };
        for item in &select.projection {
            match item {
                SelectItem::Wildcard(opts) | SelectItem::QualifiedWildcard(_, opts)
                    if *opts == WildcardAdditionalOptions::default() =>
                {
                    if let SelectItem::QualifiedWildcard(kind, _) = item {
                        let SelectItemQualifiedWildcardKind::ObjectName(n) = kind else {
                            return Err(Denied);
                        };
                        let [p] = n.0.as_slice() else {
                            return Err(Denied);
                        };
                        if id(p.as_ident().ok_or(Denied)?)? != source.alias {
                            return Err(Denied);
                        }
                    }
                    for (n, m) in &source.columns {
                        add(source.reference(n), n, *m)?;
                    }
                }
                SelectItem::UnnamedExpr(e) => {
                    let (n, m) = source.column(e)?;
                    add(source.reference(n), n, m)?;
                }
                SelectItem::ExprWithAlias {
                    expr: Expr::Function(f),
                    alias,
                } if select.projection.len() == 1
                    && f.to_string().eq_ignore_ascii_case("COUNT(*)") =>
                {
                    add("COUNT(*)".into(), id(alias)?, false)?;
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let (n, m) = source.column(expr)?;
                    add(source.reference(n), id(alias)?, m)?;
                }
                _ => return Err(Denied),
            }
        }
        let select_fields = if outer {
            // Guard the entire row on the server. The extra scalar distinguishes a real NULL
            // from an oversized row. Keep ORDER BY and LIMIT on this same SELECT.
            let size = exprs
                .iter()
                .map(|v| format!("COALESCE(OCTET_LENGTH({v}),0)"))
                .collect::<Vec<_>>()
                .join("+");
            let oversized = format!("({size}>{})", self.policy.max_bytes / 2);
            let mut fields = exprs
                .iter()
                .map(|v| format!("IF({oversized},NULL,{v})"))
                .collect::<Vec<_>>();
            fields.push(oversized);
            format!(
                "/*+ MAX_EXECUTION_TIME({}) */ {}",
                self.policy.timeout_ms,
                fields.join(",")
            )
        } else {
            exprs
                .iter()
                .zip(&columns)
                .map(|(v, (name, _))| format!("{v} AS {}", quoted(name)))
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut sql = format!("SELECT {select_fields} FROM {}", source.sql);
        if let Some(e) = &select.selection {
            sql.push_str(&format!(" WHERE {}", self.predicate(e, &source)?));
        }
        if let Some(sort) = &q.order_by {
            let OrderByKind::Expressions(xs) = &sort.kind else {
                return Err(Denied);
            };
            if sort.interpolate.is_some() {
                return Err(Denied);
            }
            let mut order = Vec::new();
            for x in xs {
                let (n, m) = source.column(&x.expr)?;
                if m || x.options.nulls_first.is_some() || x.with_fill.is_some() {
                    return Err(Denied);
                }
                let direction = match x.options.sort {
                    None | Some(OrderBySort::Asc) => "ASC",
                    Some(OrderBySort::Desc) => "DESC",
                    _ => return Err(Denied),
                };
                order.push(format!("{} {direction}", source.reference(n)));
            }
            sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
        let mut limit = None;
        if let Some(clause) = &q.limit_clause {
            let LimitClause::LimitOffset {
                limit: Some(Expr::Value(v)),
                offset: None,
                limit_by,
            } = clause
            else {
                return Err(Denied);
            };
            let Value::Number(n, false) = &v.value else {
                return Err(Denied);
            };
            if !limit_by.is_empty() {
                return Err(Denied);
            }
            limit = Some(n.parse::<u64>().map_err(|_| Denied)?);
        }
        if outer {
            limit = Some(
                limit
                    .unwrap_or(u64::MAX)
                    .min(self.policy.max_rows as u64 + 1),
            );
        }
        if let Some(n) = limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }
        Ok(Projection { sql, columns })
    }
}
pub fn compile(sql: &str, db: &str, policy: &QueryPolicy) -> Result<ReadQuery, Denied> {
    // The database name comes from local registration, but is still an SQL identifier.
    id(&Ident::new(db))?;
    let q = parse(sql)?;
    let mut c = Compiler {
        policy,
        db,
        baseline: parse("SELECT x FROM t")?,
        params: vec![],
    };
    let p = c.select(&q, true)?;
    let masked = p.columns.iter().filter(|(_, m)| *m).count();
    let columns: Vec<_> = p.columns.into_iter().map(|(n, _)| n).collect();
    Ok(ReadQuery {
        statement: p.sql,
        parameters: c.params,
        columns,
        masked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use envlume_domain::QueryColumns;
    fn policy() -> QueryPolicy {
        QueryPolicy {
            tables: BTreeMap::from([(
                "orders".into(),
                QueryColumns {
                    readable: ["id", "note"].map(str::to_owned).into(),
                    masked: ["name"].map(str::to_owned).into(),
                },
            )]),
            max_rows: 200,
            max_bytes: 262144,
            timeout_ms: 5000,
        }
    }
    #[test]
    fn regenerate_only_scoped_identifiers_and_bind_literal_values() {
        let p = policy();
        for s in [
            "SELECT * FROM orders",
            "SELECT d.* FROM demo.orders AS d ORDER BY d.id DESC",
            "SELECT z.person FROM (SELECT name AS person FROM orders) AS z",
            "SELECT COUNT(*) AS total FROM orders",
        ] {
            assert!(compile(s, "demo", &p).is_ok(), "{s}");
        }
        let q = compile(
            "SELECT o.name AS person FROM orders o WHERE note = 'fixture note' LIMIT 900",
            "demo",
            &p,
        )
        .unwrap();
        assert!(!q.statement().contains("fixture note") && !q.statement().contains("`name`"));
        assert!(q.statement().ends_with("LIMIT 201"));
        assert_eq!(q.masked(), 1);
        assert_eq!(q.parameters().len(), 1);
    }
    #[test]
    fn unsupported_or_ambiguous_references_are_denied() {
        for s in [
            "SELECT * FROM elsewhere.orders",
            "SELECT missing FROM orders",
            "SELECT id FROM orders WHERE name = 'fixture'",
            "SELECT id FROM orders ORDER BY name",
            "SELECT id AS a, note AS A FROM orders",
            "SELECT other.* FROM orders",
            "SELECT id FROM (SELECT missing FROM orders) AS x",
            "SELECT x.id FROM (SELECT name AS id FROM orders) AS x WHERE x.id = 1",
            "SELECT id FROM orders UNION SELECT id FROM orders",
            "SELECT id FROM orders FOR UPDATE",
            "SELECT id FROM orders; SELECT id FROM orders",
            "SHOW TABLES",
            "SELECT RAND() FROM orders",
            "SELECT id FROM orders /* fixture */",
            "SELECT DISTINCT id FROM orders",
            "SELECT id FROM orders JOIN orders o ON o.id = orders.id",
        ] {
            assert!(compile(s, "demo", &policy()).is_err(), "{s}");
        }
    }
}
